//! Separate native front end. Opening this window never starts a process.

use std::cell::RefCell;
use std::path::PathBuf;

use crate::accessibility::set_accessible_name;
use crate::agent_protocol::{
    Approval, Decision, Phase, MAX_PROMPT_BYTES, MAX_TRANSCRIPT_BYTES, TRANSCRIPT_TRUNCATION_MARKER,
};
use crate::agent_transport::{Transport, UserAction};
use crate::announcements::{self, Announcer};
use crate::dpi;
use crate::native_text;
use crate::profile_editor::{
    activate, apply_theme, child, control_color, create_dialog, dispatch_owned, erase_background,
    handle_select_all, handle_tab, owns_message, release_theme, restore_focus, set_text, wide,
};
use crate::theme::{draw_mode_button, Theme};
use windows::core::{w, Error, Result};
use windows::Win32::Foundation::{
    E_OUTOFMEMORY, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_GETFIRSTVISIBLELINE, EM_GETSEL, EM_LINEFROMCHAR, EM_LINEINDEX,
    EM_LINESCROLL, EM_REPLACESEL, EM_SETLIMITTEXT, EM_SETSEL, ODS_HOTLIGHT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetClientRect, GetForegroundWindow, GetWindowTextLengthW,
    GetWindowTextW, KillTimer, MoveWindow, SendMessageW, SetTimer, SetWindowPos, BN_CLICKED,
    BS_OWNERDRAW, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, LBN_SELCHANGE,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LB_ADDSTRING, LB_GETCURSEL, LB_SETCURSEL, MINMAXINFO, MSG,
    SWP_NOACTIVATE, SWP_NOZORDER, WINDOW_STYLE, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER,
    WS_BORDER, WS_TABSTOP, WS_VSCROLL,
};

pub const ID_TRANSCRIPT: usize = 4500;
pub const ID_PROMPT: usize = 4501;
pub const ID_START: usize = 4502;
pub const ID_SEND: usize = 4503;
pub const ID_INTERRUPT: usize = 4504;
pub const ID_CONTEXT: usize = 4505;
pub const ID_OPTIONS: usize = 4506;
pub const ID_APPLY: usize = 4507;
pub const ID_FALLBACK: usize = 4508;
pub const ID_CLOSE: usize = 4509;
pub const ID_STATUS: usize = 4510;
const POLL_TIMER: usize = 4511;

const START: &str = "Запустить App Server";
const SEND: &str = "Отправить";
const INTERRUPT: &str = "Прервать запрос";
const APPLY: &str = "Передать выбранное решение";
const FALLBACK: &str = "Открыть обычный терминал";
const CLOSE: &str = "Закрыть App Server";
const OPTIONS: [&str; 3] = [
    "Разрешить только это действие (accept)",
    "Отклонить действие, продолжить диалог (decline)",
    "Отклонить и прервать запрос (cancel)",
];
const INTRO: &str = "Отдельный клиент Codex CLI 0.164.0. Кнопка «Запустить App Server» запускает локальный процесс с существующим входом Codex. «Отправить» передаёт введённое сообщение модели. Новый диалог: read-only sandbox, untrusted approvals, подтверждает пользователь. Поддерживается текст и разовые подтверждения обычных команд. Вход, файлы, сетевые разрешения, постоянные правила и прочие запросы — через обычный терминал. Закрытие останавливает только этот App Server. История хранится в памяти окна и ограничена 192 КиБ.";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Fallback(PathBuf),
    Closed,
}

#[derive(Clone, Copy)]
struct View {
    hwnd: HWND,
    owner: HWND,
    previous_focus: HWND,
    transcript: HWND,
    prompt: HWND,
    start: HWND,
    send: HWND,
    interrupt: HWND,
    context: HWND,
    options: HWND,
    apply: HWND,
    fallback: HWND,
    close: HWND,
    status: HWND,
    announcement: Announcer,
    dpi: u32,
    theme: Theme,
}

struct State {
    view: View,
    cli_path: Option<PathBuf>,
    cwd: PathBuf,
    transport: Option<Transport>,
    transport_revision: u64,
    phase: Option<Phase>,
    shown_approval: Option<Approval>,
    transcript: String,
    transcript_start: u64,
    status: String,
    send_waiting: bool,
    decision_waiting: bool,
    ready: bool,
}

thread_local! {
    static WINDOW: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Outcome>> = const { RefCell::new(None) };
}

fn current() -> Option<View> {
    WINDOW.with(|state| state.borrow().as_ref().map(|state| state.view))
}

pub fn take_result() -> Option<Outcome> {
    OUTCOME.with(|outcome| outcome.borrow_mut().take())
}

/// Merely display controls. The process is started by ID_START only.
pub fn show(owner: HWND, cli_path: Option<PathBuf>, cwd: PathBuf) -> Result<()> {
    if let Some(view) = current() {
        activate(view.hwnd, view.transcript);
        return Ok(());
    }
    let (hwnd, instance, dpi) = create_dialog(
        owner,
        w!("BksTermAppServerClient"),
        w!("Codex App Server — BKS Term"),
        Some(window_proc),
        960,
        860,
    )?;
    let mut view = View {
        hwnd,
        owner,
        previous_focus: unsafe { GetFocus() },
        transcript: HWND::default(),
        prompt: HWND::default(),
        start: HWND::default(),
        send: HWND::default(),
        interrupt: HWND::default(),
        context: HWND::default(),
        options: HWND::default(),
        apply: HWND::default(),
        fallback: HWND::default(),
        close: HWND::default(),
        status: HWND::default(),
        announcement: Announcer::default(),
        dpi,
        theme: Theme::new(dpi),
    };
    OUTCOME.with(|outcome| *outcome.borrow_mut() = None);
    WINDOW.with(|state| {
        *state.borrow_mut() = Some(State {
            view,
            cli_path,
            cwd,
            transport: None,
            transport_revision: 0,
            phase: None,
            shown_approval: None,
            transcript: String::new(),
            transcript_start: 0,
            status: String::new(),
            send_waiting: false,
            decision_waiting: false,
            ready: false,
        })
    });
    let result = create_controls(&mut view, instance);
    WINDOW.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.view = view;
            state.ready = result.is_ok();
        }
    });
    if let Err(error) = result {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    apply_fonts(view);
    layout(view);
    let (available, cwd) = WINDOW.with(|state| {
        let state = state.borrow();
        let state = state.as_ref().expect("App Server view initialized");
        (state.cli_path.is_some(), state.cwd.display().to_string())
    });
    let initial_transcript =
        format!("Codex App Server пока не запущен.\r\nРабочая папка: {cwd}\r\n\r\n{INTRO}");
    WINDOW.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.transcript = initial_transcript.clone();
        }
    });
    set_text(view.transcript, &initial_transcript);
    set_text(
        view.context,
        "Ожидающих подтверждений нет. Ни одно решение не выбрано.",
    );
    set_status(
        view,
        if available {
            "Процесс не запущен. Прочитайте описание и нажмите «Запустить App Server»."
        } else {
            "Поддерживаемый native Codex CLI 0.164.0 не найден. Доступен обычный терминал."
        },
    );
    enable(view.start, available);
    enable(view.send, false);
    enable(view.interrupt, false);
    enable(view.options, false);
    enable(view.apply, false);
    unsafe {
        SetTimer(Some(hwnd), POLL_TIMER, 50, None);
    }
    activate(hwnd, view.transcript);
    Ok(())
}

fn create_controls(view: &mut View, instance: HINSTANCE) -> Result<()> {
    let read_style = WS_TABSTOP
        | WS_BORDER
        | WS_VSCROLL
        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32);
    view.transcript = child(
        view.hwnd,
        instance,
        w!("EDIT"),
        "",
        read_style,
        ID_TRANSCRIPT,
    )?;
    view.prompt = child(
        view.hwnd,
        instance,
        w!("EDIT"),
        "",
        WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | WINDOW_STYLE((ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL) as u32),
        ID_PROMPT,
    )?;
    view.context = child(view.hwnd, instance, w!("EDIT"), "", read_style, ID_CONTEXT)?;
    view.status = child(view.hwnd, instance, w!("EDIT"), "", read_style, ID_STATUS)?;
    view.options = child(
        view.hwnd,
        instance,
        w!("LISTBOX"),
        "",
        WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
        ID_OPTIONS,
    )?;
    let buttons = [
        (START, ID_START),
        (SEND, ID_SEND),
        (INTERRUPT, ID_INTERRUPT),
        (APPLY, ID_APPLY),
        (FALLBACK, ID_FALLBACK),
        (CLOSE, ID_CLOSE),
    ];
    let mut handles = Vec::new();
    for (label, id) in buttons {
        let control = child(
            view.hwnd,
            instance,
            w!("BUTTON"),
            label,
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            id,
        )?;
        crate::control_state::install(control);
        handles.push(control);
    }
    [
        view.start,
        view.send,
        view.interrupt,
        view.apply,
        view.fallback,
        view.close,
    ] = handles.try_into().expect("six buttons");
    view.announcement = announcements::create(view.hwnd, instance)?;
    for (control, label) in [
        (view.transcript, "История диалога Codex, только чтение"),
        (
            view.prompt,
            "Сообщение для Codex. Enter — новая строка, отправка кнопкой",
        ),
        (
            view.context,
            "Полный контекст текущего подтверждения App Server, только чтение",
        ),
        (
            view.options,
            "Решение по текущему запросу. Вариант по умолчанию не выбран",
        ),
        (
            view.status,
            "Состояние и возможности App Server, только чтение",
        ),
        (view.start, START),
        (view.send, SEND),
        (view.interrupt, INTERRUPT),
        (view.apply, APPLY),
        (view.fallback, FALLBACK),
        (view.close, CLOSE),
    ] {
        set_accessible_name(control, label)?;
    }
    unsafe {
        SendMessageW(
            view.prompt,
            EM_SETLIMITTEXT,
            Some(WPARAM(MAX_PROMPT_BYTES)),
            None,
        );
        SendMessageW(
            view.transcript,
            EM_SETLIMITTEXT,
            Some(WPARAM(2 * MAX_TRANSCRIPT_BYTES)),
            None,
        );
        SendMessageW(view.context, EM_SETLIMITTEXT, Some(WPARAM(64 * 1024)), None);
        SendMessageW(view.status, EM_SETLIMITTEXT, Some(WPARAM(8192)), None);
        for (index, option) in OPTIONS.into_iter().enumerate() {
            let option = wide(option);
            if SendMessageW(
                view.options,
                LB_ADDSTRING,
                None,
                Some(LPARAM(option.as_ptr() as isize)),
            )
            .0 != index as isize
            {
                // A missing native item must never shift decline onto the
                // index mapped to accept. Abort the whole view on insertion
                // failure, before any choice can become available.
                return Err(Error::from_hresult(E_OUTOFMEMORY));
            }
        }
        SendMessageW(view.options, LB_SETCURSEL, Some(WPARAM(usize::MAX)), None);
    }
    Ok(())
}

pub fn handle_message(message: &MSG) -> bool {
    let Some(view) = current().filter(|view| owns_message(view.hwnd, message)) else {
        return false;
    };
    unsafe {
        let focus = GetFocus();
        if message.message == WM_KEYDOWN {
            let stops = [
                view.transcript,
                view.prompt,
                view.start,
                view.send,
                view.interrupt,
                view.context,
                view.options,
                view.apply,
                view.fallback,
                view.close,
                view.status,
            ]
            .into_iter()
            .filter(|control| IsWindowEnabled(*control).as_bool())
            .collect::<Vec<_>>();
            if handle_tab(message.wParam.0, &stops) {
                return true;
            }
            if message.wParam.0 == VK_ESCAPE.0 as usize {
                close();
                return true;
            }
            if [view.transcript, view.prompt, view.context, view.status].contains(&focus)
                && handle_select_all(message.wParam.0, focus)
            {
                return true;
            }
            if message.wParam.0 == VK_RETURN.0 as usize {
                if focus == view.prompt {
                    // IsDialogMessage can turn Enter into a default button.
                    // Dispatch directly so the multiline prompt receives it.
                    let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(message);
                    windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(message);
                } else {
                    let id = [
                        (view.start, ID_START),
                        (view.send, ID_SEND),
                        (view.interrupt, ID_INTERRUPT),
                        (view.apply, ID_APPLY),
                        (view.fallback, ID_FALLBACK),
                        (view.close, ID_CLOSE),
                    ]
                    .into_iter()
                    .find(|(control, _)| *control == focus)
                    .map(|(_, id)| id);
                    if let Some(id) = id {
                        command(view, id);
                    }
                }
                // Reading and selecting a list item never confirms it.
                return true;
            }
        }
        if message.message == WM_CHAR
            && focus != view.prompt
            && matches!(message.wParam.0, 10 | 13 | 27)
        {
            return true;
        }
        dispatch_owned(view.hwnd, message);
    }
    true
}

pub fn poll() {
    let update = WINDOW.with(|state| {
        let mut state = state.borrow_mut();
        let state = state.as_mut()?;
        let snapshot = state
            .transport
            .as_ref()?
            .snapshot_since(state.transport_revision)?;
        state.transport_revision = snapshot.revision;
        let phase_changed = state.phase != Some(snapshot.phase);
        state.phase = Some(snapshot.phase);
        state.send_waiting = false;
        state.decision_waiting = false;
        let transcript = native_lines(&snapshot.transcript);
        let patch = native_text::difference(&state.transcript, &transcript);
        let trim = TranscriptTrim::between(state.transcript_start, snapshot.transcript_start);
        state.transcript = transcript;
        state.transcript_start = snapshot.transcript_start;
        let changed_approval = state.shown_approval != snapshot.approval;
        state.shown_approval = snapshot.approval.clone();
        Some((
            state.view,
            snapshot,
            patch,
            changed_approval,
            phase_changed,
            trim,
        ))
    });
    let Some((view, snapshot, patch, changed_approval, phase_changed, trim)) = update else {
        return;
    };
    if phase_changed {
        let phase = match snapshot.phase {
            Phase::Initializing => "подключение",
            Phase::StartingThread => "новый диалог",
            Phase::Ready => "готов",
            Phase::Busy => "ответ",
            Phase::Failed => "ошибка",
            Phase::Stopped => "остановлен",
        };
        set_text(view.hwnd, &format!("Codex App Server — {phase} — BKS Term"));
    }
    if let Some(patch) = patch {
        apply_text_patch(view.transcript, patch, trim);
    }
    if changed_approval {
        if let Some(approval) = &snapshot.approval {
            set_text(
                view.context,
                &native_lines(&format!(
                    "Ожидают решения: {}. Ревизия: {}.\n{}",
                    snapshot.pending_count, approval.revision, approval.context
                )),
            );
        } else {
            set_text(
                view.context,
                "Ожидающих подтверждений нет. Предыдущий выбор недействителен.",
            );
        }
        unsafe {
            SendMessageW(view.options, LB_SETCURSEL, Some(WPARAM(usize::MAX)), None);
        }
    }
    enable(view.start, false);
    enable(view.send, snapshot.can_send);
    enable(view.interrupt, snapshot.can_interrupt);
    enable(view.options, snapshot.approval.is_some());
    enable(
        view.apply,
        snapshot.approval.is_some() && selected_decision(view).is_some(),
    );
    set_status(view, &snapshot.status);
}

pub fn close() {
    if let Some(view) = current() {
        finish(view, Outcome::Closed);
    }
}

fn command(view: View, id: usize) {
    match id {
        ID_START if unsafe { IsWindowEnabled(view.start).as_bool() } => {
            let settings = WINDOW.with(|state| {
                let state = state.borrow();
                let state = state.as_ref()?;
                if state.transport.is_some() {
                    return None;
                }
                Some((state.cli_path.clone()?, state.cwd.clone()))
            });
            let Some((path, cwd)) = settings else {
                return;
            };
            match Transport::start(path, cwd) {
                Ok(transport) => {
                    WINDOW.with(|state| {
                        if let Some(state) = state.borrow_mut().as_mut() {
                            state.transport = Some(transport);
                        }
                    });
                    enable(view.start, false);
                    set_status(view, "Проверка native Codex CLI 0.164.0 и подключение…");
                    poll();
                }
                Err(error) => set_status(view, &error),
            }
        }
        ID_SEND if unsafe { IsWindowEnabled(view.send).as_bool() } => {
            let text = read_prompt(view.prompt);
            let result = WINDOW.with(|state| {
                let mut state = state.borrow_mut();
                let state = state.as_mut().ok_or("Окно закрыто.")?;
                if state.send_waiting {
                    return Err("Сообщение уже ожидает отправки.".to_owned());
                }
                state
                    .transport
                    .as_ref()
                    .ok_or("App Server не запущен.")?
                    .send(UserAction::Prompt(text))?;
                state.send_waiting = true;
                Ok(())
            });
            match result {
                Ok(()) => enable(view.send, false),
                Err(error) => set_status(view, &error),
            }
        }
        ID_INTERRUPT if unsafe { IsWindowEnabled(view.interrupt).as_bool() } => {
            let result = send_action(UserAction::Interrupt);
            match result {
                Ok(()) => {
                    enable(view.interrupt, false);
                    enable(view.apply, false);
                }
                Err(error) => set_status(view, &error),
            }
        }
        ID_APPLY if unsafe { IsWindowEnabled(view.apply).as_bool() } => {
            let Some(decision) = selected_decision(view) else {
                return;
            };
            let approval = WINDOW.with(|state| {
                let state = state.borrow();
                let state = state.as_ref()?;
                (!state.decision_waiting)
                    .then(|| state.shown_approval.clone())
                    .flatten()
            });
            let Some(approval) = approval else {
                return;
            };
            match send_action(UserAction::Decide(approval, decision)) {
                Ok(()) => {
                    WINDOW.with(|state| {
                        if let Some(state) = state.borrow_mut().as_mut() {
                            state.decision_waiting = true;
                        }
                    });
                    enable(view.apply, false);
                }
                Err(error) => set_status(view, &error),
            }
        }
        ID_FALLBACK => {
            let cwd = WINDOW.with(|state| state.borrow().as_ref().map(|state| state.cwd.clone()));
            if let Some(cwd) = cwd {
                finish(view, Outcome::Fallback(cwd));
            }
        }
        ID_CLOSE => finish(view, Outcome::Closed),
        _ => {}
    }
}

fn send_action(action: UserAction) -> std::result::Result<(), String> {
    WINDOW.with(|state| {
        state
            .borrow()
            .as_ref()
            .and_then(|state| state.transport.as_ref())
            .ok_or_else(|| "App Server не запущен.".to_owned())?
            .send(action)
    })
}

fn selected_decision(view: View) -> Option<Decision> {
    match unsafe { SendMessageW(view.options, LB_GETCURSEL, None, None).0 } {
        0 => Some(Decision::AcceptOnce),
        1 => Some(Decision::Decline),
        2 => Some(Decision::CancelTurn),
        _ => None,
    }
}

fn read_prompt(hwnd: HWND) -> String {
    unsafe {
        let count = (GetWindowTextLengthW(hwnd).max(0) as usize).min(MAX_PROMPT_BYTES);
        let mut text = vec![0; count + 1];
        let count = GetWindowTextW(hwnd, &mut text).max(0) as usize;
        String::from_utf16_lossy(&text[..count])
    }
}

fn finish(view: View, outcome: Outcome) {
    let restore = unsafe { GetForegroundWindow() == view.hwnd };
    OUTCOME.with(|result| *result.borrow_mut() = Some(outcome));
    if unsafe { DestroyWindow(view.hwnd) }.is_err() {
        OUTCOME.with(|result| *result.borrow_mut() = None);
        return;
    }
    if restore {
        restore_focus(view.owner, view.previous_focus);
    }
}

fn enable(hwnd: HWND, enabled: bool) {
    unsafe {
        let _ = EnableWindow(hwnd, enabled);
    }
}

fn set_status(view: View, text: &str) {
    let changed = WINDOW.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return false;
        };
        if state.status == text {
            return false;
        }
        state.status = text.to_owned();
        true
    });
    if changed {
        set_text(view.status, &native_lines(&format!("{text}\n\n{INTRO}")));
        set_text(view.announcement.hwnd, text);
        let _ = view.announcement.announce(text);
    }
}

fn native_lines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

#[derive(Clone, Copy)]
struct TranscriptTrim {
    removed_units: u64,
    old_marker_units: u32,
    new_marker_units: u32,
}

impl TranscriptTrim {
    fn between(old_start: u64, new_start: u64) -> Option<Self> {
        if new_start <= old_start {
            return None;
        }
        let marker = native_lines(TRANSCRIPT_TRUNCATION_MARKER)
            .encode_utf16()
            .count() as u32;
        Some(Self {
            removed_units: new_start - old_start,
            old_marker_units: if old_start == 0 { 0 } else { marker },
            new_marker_units: marker,
        })
    }

    fn rebase(self, offset: u32) -> u32 {
        if offset < self.old_marker_units {
            return offset.min(self.new_marker_units);
        }
        let body = u64::from(offset - self.old_marker_units);
        if body < self.removed_units {
            return 0;
        }
        (u64::from(self.new_marker_units) + body - self.removed_units).min(u64::from(u32::MAX))
            as u32
    }
}

fn apply_text_patch(hwnd: HWND, patch: native_text::TextPatch, trim: Option<TranscriptTrim>) {
    unsafe {
        let mut start = 0u32;
        let mut end = 0u32;
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
        let first_line = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
        let view_offset = SendMessageW(hwnd, EM_LINEINDEX, Some(WPARAM(first_line as usize)), None)
            .0
            .max(0) as u32;
        SendMessageW(
            hwnd,
            EM_SETSEL,
            Some(WPARAM(patch.start as usize)),
            Some(LPARAM(patch.end as isize)),
        );
        let text = wide(&patch.replacement);
        SendMessageW(
            hwnd,
            EM_REPLACESEL,
            Some(WPARAM(0)),
            Some(LPARAM(text.as_ptr() as isize)),
        );
        let length = GetWindowTextLengthW(hwnd).max(0) as u32;
        let rebase = |offset| trim.map_or(offset, |trim| trim.rebase(offset)).min(length);
        SendMessageW(
            hwnd,
            EM_SETSEL,
            Some(WPARAM(rebase(start) as usize)),
            Some(LPARAM(rebase(end) as isize)),
        );
        let target_line = SendMessageW(
            hwnd,
            EM_LINEFROMCHAR,
            Some(WPARAM(rebase(view_offset) as usize)),
            None,
        )
        .0;
        let current_line = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
        SendMessageW(
            hwnd,
            EM_LINESCROLL,
            Some(WPARAM(0)),
            Some(LPARAM(target_line - current_line)),
        );
    }
}

fn apply_fonts(view: View) {
    apply_theme(
        view.hwnd,
        &view.theme,
        &[
            view.transcript,
            view.prompt,
            view.start,
            view.send,
            view.interrupt,
            view.context,
            view.options,
            view.apply,
            view.fallback,
            view.close,
            view.status,
            view.announcement.hwnd,
        ],
        &[
            view.transcript,
            view.prompt,
            view.context,
            view.options,
            view.status,
        ],
    );
}

fn layout(view: View) {
    unsafe {
        let mut rect = RECT::default();
        if GetClientRect(view.hwnd, &mut rect).is_err() {
            return;
        }
        let margin = dpi::scale(12, view.dpi);
        let gap = dpi::scale(6, view.dpi);
        let width = (rect.right - 2 * margin).max(1);
        let height = (rect.bottom - 2 * margin).max(1);
        let button_h = dpi::scale(34, view.dpi);
        let line_h = dpi::scale(24, view.dpi);
        let status_h = dpi::scale(64, view.dpi).min(height / 8).max(1);
        let options_h = dpi::scale(70, view.dpi).min(height / 8).max(1);
        let prompt_h = dpi::scale(72, view.dpi).min(height / 7).max(1);
        let remaining =
            (height - 2 * button_h - line_h - status_h - options_h - prompt_h - 7 * gap).max(2);
        let transcript_h = (remaining * 3 / 5).max(1);
        let context_h = (remaining - transcript_h).max(1);
        let mut y = margin;
        for (control, h) in [(view.transcript, transcript_h), (view.prompt, prompt_h)] {
            let _ = MoveWindow(control, margin, y, width, h, true);
            y += h + gap;
        }
        let third = (width - 2 * gap) / 3;
        for (index, control) in [view.start, view.send, view.interrupt]
            .into_iter()
            .enumerate()
        {
            let _ = MoveWindow(
                control,
                margin + index as i32 * (third + gap),
                y,
                third.max(1),
                button_h,
                true,
            );
        }
        y += button_h + gap;
        for (control, h) in [
            (view.context, context_h),
            (view.options, options_h),
            (view.status, status_h),
        ] {
            let _ = MoveWindow(control, margin, y, width, h, true);
            y += h + gap;
        }
        let _ = MoveWindow(view.announcement.hwnd, margin, y, width, line_h, true);
        y += line_h + gap;
        for (index, control) in [view.apply, view.fallback, view.close]
            .into_iter()
            .enumerate()
        {
            let _ = MoveWindow(
                control,
                margin + index as i32 * (third + gap),
                y,
                third.max(1),
                button_h,
                true,
            );
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(view) = current().filter(|view| view.hwnd == hwnd) else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_SETFOCUS => {
            if !view.transcript.0.is_null() {
                let _ = SetFocus(Some(view.transcript));
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            finish(view, Outcome::Closed);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == POLL_TIMER => {
            poll();
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            let code = (wparam.0 >> 16) as u32;
            if id == ID_OPTIONS && code == LBN_SELCHANGE {
                let ready = WINDOW.with(|state| {
                    state.borrow().as_ref().is_some_and(|state| {
                        state.shown_approval.is_some() && !state.decision_waiting
                    })
                });
                enable(view.apply, ready && selected_decision(view).is_some());
            } else if code == BN_CLICKED {
                command(view, id);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            layout(view);
            LRESULT(0)
        }
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            let dpi = if message == WM_DPICHANGED {
                ((wparam.0 & 0xffff) as u32).max(dpi::BASE_DPI)
            } else {
                view.dpi
            };
            let next = View {
                dpi,
                theme: Theme::new(dpi),
                ..view
            };
            WINDOW.with(|state| {
                if let Some(state) = state.borrow_mut().as_mut() {
                    state.view = next;
                }
            });
            apply_fonts(next);
            release_theme(view.theme);
            if message == WM_DPICHANGED && lparam.0 != 0 {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            layout(next);
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                (*(lparam.0 as *mut MINMAXINFO)).ptMinTrackSize = POINT {
                    x: dpi::scale(760, view.dpi),
                    y: dpi::scale(650, view.dpi),
                };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => erase_background(hwnd, wparam, &view.theme),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => control_color(wparam, &view.theme, true),
        WM_CTLCOLORSTATIC => control_color(
            wparam,
            &view.theme,
            [view.transcript, view.context, view.status]
                .iter()
                .any(|control| control.0 as isize == lparam.0),
        ),
        WM_DRAWITEM if lparam.0 != 0 => {
            let mut item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if crate::control_state::is_hot(item.hwndItem) {
                item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            let label = match wparam.0 {
                ID_START => START,
                ID_SEND => SEND,
                ID_INTERRUPT => INTERRUPT,
                ID_APPLY => APPLY,
                ID_FALLBACK => FALLBACK,
                _ => CLOSE,
            };
            draw_mode_button(&item, &view.theme, label);
            LRESULT(1)
        }
        WM_NCDESTROY => {
            let _ = KillTimer(Some(hwnd), POLL_TIMER);
            let state = WINDOW.with(|state| state.borrow_mut().take());
            if let Some(state) = state {
                if state.ready {
                    OUTCOME.with(|outcome| {
                        if outcome.borrow().is_none() {
                            *outcome.borrow_mut() = Some(Outcome::Closed);
                        }
                    });
                }
                drop(state); // Transport::drop only sets its cancellation flag.
            }
            release_theme(view.theme);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WINDOW_EX_STYLE, WS_CHILD};

    struct HiddenEdit {
        parent: HWND,
        edit: HWND,
    }
    impl HiddenEdit {
        fn new() -> Self {
            unsafe {
                let parent = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    480,
                    200,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD
                        | WS_VSCROLL
                        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                    0,
                    400,
                    100,
                    Some(parent),
                    None,
                    None,
                    None,
                )
                .unwrap();
                Self { parent, edit }
            }
        }
        fn text(&self) -> Vec<u16> {
            unsafe {
                let mut text = vec![0; GetWindowTextLengthW(self.edit) as usize + 1];
                let length = GetWindowTextW(self.edit, &mut text) as usize;
                text.truncate(length);
                text
            }
        }
    }
    impl Drop for HiddenEdit {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.parent);
            }
        }
    }

    #[test]
    fn native_lines_preserve_unicode_and_mixed_newlines() {
        assert_eq!(native_lines("a\r\nb\nc\rd 😀"), "a\r\nb\r\nc\r\nd 😀");
    }

    #[test]
    fn streamed_append_preserves_native_edit_selection_and_scroll() {
        // Both windows are private fixtures; the parent is never shown or
        // activated. This verifies native EDIT behavior, not NVDA acceptance.
        unsafe {
            let fixture = HiddenEdit::new();
            let edit = fixture.edit;
            let old = (0..60)
                .map(|i| format!("Строка {i:02} 😀\r\n"))
                .collect::<String>();
            set_text(edit, &old);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(10)), Some(LPARAM(18)));
            SendMessageW(edit, EM_LINESCROLL, Some(WPARAM(0)), Some(LPARAM(12)));
            let before_line = SendMessageW(edit, EM_GETFIRSTVISIBLELINE, None, None).0;
            assert!(before_line > 0);
            let new = format!("{old}Новый фрагмент 😀\r\n");
            let patch = native_text::difference(&old, &new).unwrap();
            assert_eq!(patch.start as usize, old.encode_utf16().count());
            apply_text_patch(edit, patch, None);
            let mut start = 0u32;
            let mut end = 0u32;
            SendMessageW(
                edit,
                EM_GETSEL,
                Some(WPARAM(&mut start as *mut u32 as usize)),
                Some(LPARAM(&mut end as *mut u32 as isize)),
            );
            assert_eq!((start, end), (10, 18));
            assert_eq!(
                SendMessageW(edit, EM_GETFIRSTVISIBLELINE, None, None).0,
                before_line
            );
            let mut text = vec![0; GetWindowTextLengthW(edit) as usize + 1];
            let length = GetWindowTextW(edit, &mut text) as usize;
            assert_eq!(String::from_utf16(&text[..length]).unwrap(), new);
        }
    }

    #[test]
    fn prefix_trim_rebases_surviving_unicode_selection_and_first_visible_text() {
        for old_start in [0, 500] {
            let fixture = HiddenEdit::new();
            let edit = fixture.edit;
            let marker = native_lines(TRANSCRIPT_TRUNCATION_MARKER);
            let lines = (0..80)
                .map(|i| format!("Строка {i:02} Ж😀e\u{301}\r\n"))
                .collect::<Vec<_>>();
            let old = format!(
                "{}{}",
                if old_start == 0 { "" } else { &marker },
                lines.concat()
            );
            let new = format!(
                "{}{}Новая строка Ж😀e\u{301}\r\n",
                marker,
                lines[20..].concat()
            );
            let removed = lines[..20].concat().encode_utf16().count() as u64;
            set_text(edit, &old);
            let selected = "Ж😀e\u{301}";
            let row_start = old.find("Строка 35").unwrap();
            let selected_byte = row_start + old[row_start..].find(selected).unwrap();
            let selected_start = old[..selected_byte].encode_utf16().count();
            let selected_end = selected_start + selected.encode_utf16().count();
            unsafe {
                SendMessageW(
                    edit,
                    EM_SETSEL,
                    Some(WPARAM(selected_start)),
                    Some(LPARAM(selected_end as isize)),
                );
                SendMessageW(edit, EM_LINESCROLL, Some(WPARAM(0)), Some(LPARAM(28)));
                let first = SendMessageW(edit, EM_GETFIRSTVISIBLELINE, None, None).0;
                let old_view_offset =
                    SendMessageW(edit, EM_LINEINDEX, Some(WPARAM(first as usize)), None).0 as usize;
                let old_units = fixture.text();
                let before_view = String::from_utf16(&old_units[old_view_offset..])
                    .unwrap()
                    .split("\r\n")
                    .next()
                    .unwrap()
                    .to_owned();
                assert!(before_view.starts_with("Строка"));
                let trim = TranscriptTrim::between(old_start, old_start + removed).unwrap();
                apply_text_patch(
                    edit,
                    native_text::difference(&old, &new).unwrap(),
                    Some(trim),
                );
                let mut start = 0u32;
                let mut end = 0u32;
                SendMessageW(
                    edit,
                    EM_GETSEL,
                    Some(WPARAM(&mut start as *mut u32 as usize)),
                    Some(LPARAM(&mut end as *mut u32 as isize)),
                );
                let units = fixture.text();
                assert_eq!(
                    String::from_utf16(&units[start as usize..end as usize]).unwrap(),
                    selected
                );
                assert_eq!(start, trim.rebase(selected_start as u32));
                assert_eq!(end, trim.rebase(selected_end as u32));
                let first = SendMessageW(edit, EM_GETFIRSTVISIBLELINE, None, None).0;
                let offset =
                    SendMessageW(edit, EM_LINEINDEX, Some(WPARAM(first as usize)), None).0 as usize;
                let after_view = String::from_utf16(&units[offset..])
                    .unwrap()
                    .split("\r\n")
                    .next()
                    .unwrap()
                    .to_owned();
                assert_eq!(after_view, before_view);
                assert_eq!(String::from_utf16(&units).unwrap(), new);
            }
        }
    }
}
