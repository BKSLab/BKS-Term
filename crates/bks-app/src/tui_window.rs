//! Explicit, native view of one recognized CLI dialog snapshot.
//! This module never reads or writes a terminal session. The caller binds the
//! snapshot to a source and revalidates that source before applying an outcome.

use std::cell::RefCell;

use crate::accessibility::set_accessible_name;
use crate::announcements::{self, Announcer};
use crate::dpi;
use crate::profile_editor::{
    activate, apply_theme, child, control_color, create_dialog, dispatch_owned, erase_background,
    handle_select_all, handle_tab, owns_message, release_theme, restore_focus, set_text, wide,
};
use crate::theme::{draw_mode_button, Theme};
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{
    E_INVALIDARG, E_OUTOFMEMORY, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{InvalidateRect, SetBkColor, SetTextColor, HDC};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_GETFIRSTVISIBLELINE, EM_GETSEL, EM_LINESCROLL, EM_SETSEL, ODS_HOTLIGHT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetAncestor, GetClientRect, GetForegroundWindow, MoveWindow,
    SendMessageW, SetWindowLongPtrW, SetWindowLongW, SetWindowPos, BN_CLICKED, BS_OWNERDRAW,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GA_ROOT, GWLP_ID, GWL_STYLE, LBN_SELCHANGE,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LB_ADDSTRING, LB_DELETESTRING, LB_GETCURSEL, LB_GETITEMRECT,
    LB_GETTOPINDEX, LB_INSERTSTRING, LB_SETCURSEL, LB_SETTOPINDEX, MINMAXINFO, MSG, SWP_NOACTIVATE,
    SWP_NOZORDER, WINDOW_STYLE, WM_ACTIVATE, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS, WM_SETFONT, WM_SETREDRAW,
    WM_SETTINGCHANGE, WM_SIZE, WS_BORDER, WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

pub const ID_CONTEXT: usize = 4400;
pub const ID_OPTIONS: usize = 4401;
pub const ID_APPLY: usize = 4402;
pub const ID_CLOSE: usize = 4403;
pub const ID_STATUS: usize = 4404;
pub const ID_CONTROL: usize = 4405;

const APPLY_LABEL: &str = "Передать выбранный вариант в CLI";
const CLOSE_LABEL: &str = "Закрыть без отправки";
const DIRECT_LABEL: &str = "Вернуться к управлению TUI";
const CONTROL_LABEL: &str = "Управлять клавишами TUI";
const SNAPSHOT_LABEL: &str = "Читать снимок";

/// The complete matched prompt, including its path, options and footer, belongs
/// in `context`. In particular, sandbox disclosures must remain in that text.
pub struct ViewData {
    pub title: String,
    pub context: String,
    pub options: Vec<String>,
    pub selected: usize,
    pub cli_label: String,
    /// True permits only validated choices. Literal navigation keys in an
    /// unmapped view require BeginControl and the source owner's acknowledgment.
    pub mapped: bool,
}

impl ViewData {
    fn validate(&self) -> Result<()> {
        let invalid = self.title.trim().is_empty()
            || self.context.trim().is_empty()
            || self.cli_label.trim().is_empty()
            || self.options.is_empty()
            || self.options.len() > i32::MAX as usize
            || self.selected >= self.options.len()
            || [&self.title, &self.context, &self.cli_label]
                .into_iter()
                .any(|text| text.contains('\0'))
            || self
                .options
                .iter()
                .any(|option| option.trim().is_empty() || option.contains(['\0', '\r', '\n']));
        if invalid {
            return Err(Error::new(
                E_INVALIDARG,
                "Неполные данные диалога CLI или недопустимый выбранный вариант.",
            ));
        }
        Ok(())
    }

    fn caption(&self) -> String {
        format!("{} — {} — BKS Term", self.title, self.cli_label)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Apply(usize),
    DirectInput,
    /// Non-closing request; the owner must bind a current source and call update_live.
    BeginControl,
    /// Non-closing literal user input. No key is derived from screen contents.
    Key(TuiKey),
    /// Non-closing return to independent snapshot reading.
    EndControl,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    ShiftTab,
    Escape,
    Enter,
}

impl TuiKey {
    fn from_virtual_key(key: usize, shift: bool, control: bool, alt: bool) -> Option<Self> {
        if control || alt || (shift && key != VK_TAB.0 as usize) {
            return None;
        }
        Some(match key {
            key if key == VK_UP.0 as usize => Self::Up,
            key if key == VK_DOWN.0 as usize => Self::Down,
            key if key == VK_LEFT.0 as usize => Self::Left,
            key if key == VK_RIGHT.0 as usize => Self::Right,
            key if key == VK_HOME.0 as usize => Self::Home,
            key if key == VK_END.0 as usize => Self::End,
            key if key == VK_PRIOR.0 as usize => Self::PageUp,
            key if key == VK_NEXT.0 as usize => Self::PageDown,
            key if key == VK_TAB.0 as usize && shift => Self::ShiftTab,
            key if key == VK_TAB.0 as usize => Self::Tab,
            key if key == VK_ESCAPE.0 as usize => Self::Escape,
            key if key == VK_RETURN.0 as usize => Self::Enter,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Up => "стрелка вверх",
            Self::Down => "стрелка вниз",
            Self::Left => "стрелка влево",
            Self::Right => "стрелка вправо",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "Page Up",
            Self::PageDown => "Page Down",
            Self::Tab => "Tab",
            Self::ShiftTab => "Shift+Tab",
            Self::Escape => "Esc",
            Self::Enter => "Enter",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ControlPhase {
    #[default]
    Snapshot,
    Starting,
    Live,
    Pending(TuiKey),
}

impl ControlPhase {
    fn active(self) -> bool {
        self != Self::Snapshot
    }

    fn begin(&mut self, mapped: bool) -> bool {
        if mapped || self.active() {
            return false;
        }
        *self = Self::Starting;
        true
    }

    fn accept_update(&mut self, mapped: bool) -> bool {
        if mapped || !self.active() {
            return false;
        }
        *self = Self::Live;
        true
    }

    fn request_key(&mut self, key: TuiKey, repeated: bool) -> bool {
        if *self != Self::Live || repeated {
            return false;
        }
        *self = Self::Pending(key);
        true
    }
}

#[derive(Clone, Copy)]
struct View {
    hwnd: HWND,
    owner: HWND,
    previous_focus: HWND,
    context: HWND,
    options: HWND,
    apply: HWND,
    control: HWND,
    close: HWND,
    status: Announcer,
    dpi: u32,
    theme: Theme,
    mapped: bool,
}

struct State {
    view: View,
    context: String,
    options: Vec<String>,
    actual_selected: usize,
    caption: String,
    cli_label: String,
    stale_reason: Option<String>,
    status_text: String,
    ready: bool,
    control: ControlPhase,
    control_notice: Option<String>,
    chrome_live: bool,
    control_button_live: bool,
}

thread_local! {
    static DIALOG: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<Outcome>> = const { RefCell::new(None) };
}

fn current() -> Option<View> {
    DIALOG.with(|dialog| dialog.borrow().as_ref().map(|state| state.view))
}

pub fn is_open() -> bool {
    current().is_some()
}

pub fn is_foreground() -> bool {
    current()
        .is_some_and(|view| unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) == view.hwnd })
}

pub fn take_result() -> Option<Outcome> {
    OUTCOME.with(|outcome| outcome.borrow_mut().take())
}

/// Call only for an explicit user action. Reopening an existing view activates
/// its original snapshot; it never silently replaces it with a different one.
pub fn show(owner: HWND, data: ViewData) -> Result<()> {
    if let Some(view) = current() {
        activate(view.hwnd, view.context);
        return Ok(());
    }
    data.validate()?;
    let caption = data.caption();
    let title = wide(&caption);
    let (hwnd, instance, dpi) = create_dialog(
        owner,
        w!("BksTermTuiDialog"),
        PCWSTR(title.as_ptr()),
        Some(window_proc),
        780,
        680,
    )?;
    let mut view = View {
        hwnd,
        owner,
        previous_focus: unsafe { GetFocus() },
        context: HWND::default(),
        options: HWND::default(),
        apply: HWND::default(),
        control: HWND::default(),
        close: HWND::default(),
        status: Announcer::default(),
        dpi,
        theme: Theme::new(dpi),
        mapped: data.mapped,
    };
    OUTCOME.with(|outcome| *outcome.borrow_mut() = None);
    DIALOG.with(|dialog| {
        *dialog.borrow_mut() = Some(State {
            view,
            context: data.context.clone(),
            options: data.options.clone(),
            actual_selected: data.selected,
            caption,
            cli_label: data.cli_label.clone(),
            stale_reason: None,
            status_text: String::new(),
            ready: false,
            control: ControlPhase::Snapshot,
            control_notice: None,
            chrome_live: false,
            control_button_live: false,
        });
    });
    let result = create_controls(&mut view, instance, &data);
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
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
    update_status(view);
    // Opening a snapshot starts with its complete context. Live control always
    // requires the user's separate action after the snapshot is available.
    activate(hwnd, view.context);
    announce_status(view);
    Ok(())
}

/// Acknowledge an explicit BeginControl or Key with a freshly bound projection.
/// Snapshot and mapped views cannot be silently upgraded by this function.
/// The owner decides when a new frame/deadline is ready; this UI never retries.
pub fn update_live(data: ViewData) -> Result<()> {
    data.validate()?;
    if data.mapped {
        return Err(Error::new(
            E_INVALIDARG,
            "В этом диалоге используются только проверенные варианты.",
        ));
    }
    let caption = data.caption();
    let pending = DIALOG.with(|dialog| {
        let dialog = dialog.borrow();
        let state = dialog.as_ref()?;
        (state.ready && !state.view.mapped && state.control.active()).then(|| {
            (
                state.view,
                state.control == ControlPhase::Starting,
                state.context != data.context,
                option_update(&state.options, &data.options),
                !state.chrome_live || state.caption != caption,
                !state.chrome_live,
            )
        })
    });
    let Some((view, first, changed_context, options_update, changed_caption, changed_names)) =
        pending
    else {
        return Err(Error::new(
            E_INVALIDARG,
            "Пользователь не включил управление TUI.",
        ));
    };
    if changed_context {
        update_context(view.context, &data.context);
    }
    let result = update_options(view, &data.options, data.selected, &options_update);
    if let Err(error) = result {
        freeze_control("Не удалось обновить экран. Управление остановлено.");
        return Err(error);
    }
    DIALOG.with(|dialog| {
        if let Some(state) = dialog.borrow_mut().as_mut() {
            state.control.accept_update(false);
            state.context = data.context;
            state.options = data.options;
            state.actual_selected = data.selected;
            state.caption = caption.clone();
            state.cli_label = data.cli_label;
            state.stale_reason = None;
            state.control_notice = None;
            state.chrome_live = true;
        }
    });
    if changed_caption {
        set_text(view.hwnd, &format!("Управление TUI — {caption}"));
    }
    if changed_names {
        let _ = set_accessible_name(
            view.context,
            "Полный исходный текст TUI. Только чтение; клавиши передаются приложению",
        );
        let _ = set_accessible_name(view.options, options_name(view, true));
    }
    sync_control_button(view);
    update_status(view);
    if first && is_foreground() {
        unsafe {
            let _ = SetFocus(Some(view.options));
        }
    }
    Ok(())
}

/// Stop forwarding without replacing text, selection, focus or the native HWND.
/// The caller may use this after source loss, stale Enter, or failed refresh.
pub fn freeze_control(reason: &str) {
    let next = DIALOG.with(|dialog| {
        let mut dialog = dialog.borrow_mut();
        let state = dialog.as_mut()?;
        if state.view.mapped || !state.ready {
            return None;
        }
        state.control = ControlPhase::Snapshot;
        state.control_notice = Some(if reason.trim().is_empty() {
            "Управление остановлено. Снимок сохранён для чтения.".to_owned()
        } else {
            reason.replace('\0', "�")
        });
        let changed_chrome = std::mem::replace(&mut state.chrome_live, false);
        Some((
            state.view,
            state.caption.clone(),
            state.cli_label.clone(),
            changed_chrome,
        ))
    });
    let Some((view, caption, label, changed_chrome)) = next else {
        return;
    };
    OUTCOME.with(|outcome| {
        let mut outcome = outcome.borrow_mut();
        if matches!(*outcome, Some(Outcome::BeginControl | Outcome::Key(_))) {
            *outcome = None;
        }
    });
    if changed_chrome {
        set_text(view.hwnd, &caption);
        let _ = set_accessible_name(
            view.context,
            &format!("Полный исходный текст диалога {label}. Только чтение"),
        );
        let _ = set_accessible_name(view.options, options_name(view, false));
    }
    sync_control_button(view);
    update_status(view);
}

fn toggle_control(view: View) {
    let active = DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .is_some_and(|state| state.control.active())
    });
    if active {
        end_control(view);
        return;
    }
    let begin = DIALOG.with(|dialog| {
        let mut dialog = dialog.borrow_mut();
        let Some(state) = dialog.as_mut() else {
            return false;
        };
        if !state.ready || !state.control.begin(view.mapped) {
            return false;
        }
        state.control_notice = None;
        true
    });
    if begin {
        OUTCOME.with(|outcome| *outcome.borrow_mut() = Some(Outcome::BeginControl));
        sync_control_button(view);
        update_status(view);
    }
}

fn end_control(view: View) {
    freeze_control("Управление отключено. Снимок сохранён для чтения.");
    OUTCOME.with(|outcome| *outcome.borrow_mut() = Some(Outcome::EndControl));
    if is_foreground() {
        unsafe {
            let _ = SetFocus(Some(view.context));
        }
    }
}

fn request_control_key(view: View, key: TuiKey, repeated: bool) {
    let accepted = DIALOG.with(|dialog| {
        dialog.borrow_mut().as_mut().is_some_and(|state| {
            state.ready && !view.mapped && state.control.request_key(key, repeated)
        })
    });
    if accepted {
        let selected =
            DIALOG.with(|dialog| dialog.borrow().as_ref().map(|state| state.actual_selected));
        if let Some(selected) = selected {
            let _ = select_option(view, selected);
        }
        OUTCOME.with(|outcome| *outcome.borrow_mut() = Some(Outcome::Key(key)));
        update_status(view);
    }
}

fn control_label() -> &'static str {
    if DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .is_some_and(|state| state.control.active())
    }) {
        SNAPSHOT_LABEL
    } else {
        CONTROL_LABEL
    }
}

fn sync_control_button(view: View) {
    let changed = DIALOG.with(|dialog| {
        let mut dialog = dialog.borrow_mut();
        let Some(state) = dialog.as_mut() else {
            return false;
        };
        let live = state.control.active();
        if state.control_button_live == live {
            return false;
        }
        state.control_button_live = live;
        true
    });
    if changed && !view.control.0.is_null() {
        let label = control_label();
        set_text(view.control, label);
        let _ = set_accessible_name(view.control, label);
        unsafe {
            let _ = InvalidateRect(Some(view.control), None, false);
        }
    }
}

fn add_option(hwnd: HWND, message: u32, index: usize, option: &str) -> Result<()> {
    unsafe {
        let option = wide(option);
        if SendMessageW(
            hwnd,
            message,
            Some(WPARAM(index)),
            Some(LPARAM(option.as_ptr() as isize)),
        )
        .0 < 0
        {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
    }
    Ok(())
}

fn populate_options(view: View, options: &[String], selected: usize) -> Result<()> {
    for option in options {
        add_option(view.options, LB_ADDSTRING, 0, option)?;
    }
    select_option(view, selected)
}

fn select_option(view: View, selected: usize) -> Result<()> {
    if unsafe { SendMessageW(view.options, LB_GETCURSEL, None, None).0 } == selected as isize {
        return Ok(());
    }
    if unsafe { SendMessageW(view.options, LB_SETCURSEL, Some(WPARAM(selected)), None).0 } < 0 {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum OptionUpdate {
    Unchanged,
    Rows(Vec<usize>),
    Splice {
        start: usize,
        removed: usize,
        added: usize,
    },
}

/// A selection-marker edit must not recreate every accessible list item.
fn option_update(previous: &[String], next: &[String]) -> OptionUpdate {
    if previous.len() == next.len() {
        let changed: Vec<usize> = previous
            .iter()
            .zip(next)
            .enumerate()
            .filter_map(|(index, (previous, next))| (previous != next).then_some(index))
            .collect();
        return if changed.is_empty() {
            OptionUpdate::Unchanged
        } else {
            OptionUpdate::Rows(changed)
        };
    }
    let start = previous
        .iter()
        .zip(next)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = previous[start..]
        .iter()
        .rev()
        .zip(next[start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    OptionUpdate::Splice {
        start,
        removed: previous.len() - start - suffix,
        added: next.len() - start - suffix,
    }
}

fn update_options(
    view: View,
    options: &[String],
    selected: usize,
    update: &OptionUpdate,
) -> Result<()> {
    if *update == OptionUpdate::Unchanged {
        // Native single-selection updates invalidate the old and new rows and
        // scroll only when the newly selected option is outside the viewport.
        return select_option(view, selected);
    }
    let previous_selected = unsafe { SendMessageW(view.options, LB_GETCURSEL, None, None).0 };
    let top = unsafe { SendMessageW(view.options, LB_GETTOPINDEX, None, None).0 }.max(0);
    unsafe {
        SendMessageW(view.options, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    let result = (|| {
        match update {
            OptionUpdate::Unchanged => {}
            OptionUpdate::Rows(rows) => {
                for index in rows {
                    unsafe {
                        SendMessageW(view.options, LB_DELETESTRING, Some(WPARAM(*index)), None);
                    }
                    add_option(view.options, LB_INSERTSTRING, *index, &options[*index])?;
                }
            }
            OptionUpdate::Splice {
                start,
                removed,
                added,
            } => {
                for _ in 0..*removed {
                    unsafe {
                        SendMessageW(view.options, LB_DELETESTRING, Some(WPARAM(*start)), None);
                    }
                }
                for (index, option) in options.iter().enumerate().skip(*start).take(*added) {
                    add_option(view.options, LB_INSERTSTRING, index, option)?;
                }
            }
        }
        select_option(view, selected)
    })();
    unsafe {
        // An unchanged selection keeps its reading position even when rows
        // before it are patched. A moved selection may scroll into view.
        if previous_selected == selected as isize {
            SendMessageW(
                view.options,
                LB_SETTOPINDEX,
                Some(WPARAM((top as usize).min(options.len() - 1))),
                None,
            );
        }
        SendMessageW(view.options, WM_SETREDRAW, Some(WPARAM(1)), None);
        if let OptionUpdate::Rows(rows) = update {
            if previous_selected >= 0 {
                invalidate_row(view.options, previous_selected as usize);
            }
            invalidate_row(view.options, selected);
            for index in rows {
                invalidate_row(view.options, *index);
            }
        } else {
            let _ = InvalidateRect(Some(view.options), None, false);
        }
    }
    result
}

fn update_context(hwnd: HWND, text: &str) {
    let text = native_lines(text);
    unsafe {
        let mut start = 0_u32;
        let mut end = 0_u32;
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
        let first_line = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
        SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(0)), None);
        set_text(hwnd, &text);
        let length = text.encode_utf16().count();
        SendMessageW(
            hwnd,
            EM_SETSEL,
            Some(WPARAM((start as usize).min(length))),
            Some(LPARAM((end as usize).min(length) as isize)),
        );
        let changed_line = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
        SendMessageW(
            hwnd,
            EM_LINESCROLL,
            None,
            Some(LPARAM(first_line - changed_line)),
        );
        SendMessageW(hwnd, WM_SETREDRAW, Some(WPARAM(1)), None);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn invalidate_row(hwnd: HWND, index: usize) {
    let mut rect = RECT::default();
    unsafe {
        if SendMessageW(
            hwnd,
            LB_GETITEMRECT,
            Some(WPARAM(index)),
            Some(LPARAM(&mut rect as *mut RECT as isize)),
        )
        .0 >= 0
        {
            let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
        }
    }
}

fn options_name(view: View, live: bool) -> &'static str {
    if view.mapped {
        "Варианты CLI. Выбор пока не отправляется"
    } else if live {
        "Экран TUI. Клавиши передаются приложению"
    } else {
        "Видимые строки TUI. Стрелки перемещают только чтение"
    }
}

fn create_controls(view: &mut View, instance: HINSTANCE, data: &ViewData) -> Result<()> {
    view.context = child(
        view.hwnd,
        instance,
        w!("EDIT"),
        &native_lines(&data.context),
        WS_TABSTOP
            | WS_VSCROLL
            | WS_BORDER
            | WINDOW_STYLE((ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL) as u32),
        ID_CONTEXT,
    )?;
    view.options = child(
        view.hwnd,
        instance,
        w!("LISTBOX"),
        "",
        WS_TABSTOP
            | WS_VSCROLL
            | WS_BORDER
            | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
        ID_OPTIONS,
    )?;
    view.apply = child(
        view.hwnd,
        instance,
        w!("BUTTON"),
        apply_label(*view),
        WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        ID_APPLY,
    )?;
    view.close = child(
        view.hwnd,
        instance,
        w!("BUTTON"),
        CLOSE_LABEL,
        WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
        ID_CLOSE,
    )?;
    if !view.mapped {
        view.control = child(
            view.hwnd,
            instance,
            w!("BUTTON"),
            CONTROL_LABEL,
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            ID_CONTROL,
        )?;
        set_accessible_name(view.control, CONTROL_LABEL)?;
        crate::control_state::install(view.control);
    }
    view.status = announcements::create(view.hwnd, instance)?;
    unsafe {
        // Reuse the existing notification provider; allow its native STATIC
        // text to wrap and retain this dialog's stable control ID.
        SetWindowLongPtrW(view.status.hwnd, GWLP_ID, ID_STATUS as isize);
        SetWindowLongW(
            view.status.hwnd,
            GWL_STYLE,
            (WS_CHILD | WS_VISIBLE).0 as i32,
        );
        SendMessageW(view.context, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
    }
    populate_options(*view, &data.options, data.selected)?;
    set_accessible_name(
        view.context,
        &format!(
            "Полный исходный текст диалога {}. Только чтение",
            data.cli_label
        ),
    )?;
    set_accessible_name(view.options, options_name(*view, false))?;
    set_accessible_name(view.apply, apply_label(*view))?;
    set_accessible_name(view.close, CLOSE_LABEL)?;
    crate::control_state::install(view.apply);
    crate::control_state::install(view.close);
    Ok(())
}

/// Consume all own messages before application shortcuts and terminal input.
/// Snapshot reading never produces CLI input. Only explicit live-control keys
/// become non-closing events for the session owner to validate and dispatch.
pub fn handle_message(message: &MSG) -> bool {
    let Some(view) = current() else {
        return false;
    };
    if !owns_message(view.hwnd, message) {
        return false;
    }
    unsafe {
        if message.message == WM_KEYDOWN {
            let focus = GetFocus();
            let active = DIALOG.with(|dialog| {
                dialog
                    .borrow()
                    .as_ref()
                    .is_some_and(|state| state.control.active())
            });
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;
            if active && control && shift && !alt && message.wParam.0 == b'I' as usize {
                end_control(view);
                return true;
            }
            if active && (focus == view.options || focus == view.context) {
                if let Some(key) = TuiKey::from_virtual_key(message.wParam.0, shift, control, alt) {
                    if is_foreground() {
                        let repeated = (message.lParam.0 as usize & (1usize << 30)) != 0;
                        request_control_key(view, key, repeated);
                    }
                    // Starting/pending/repeated keys are consumed, never queued
                    // or allowed to move the list away from the CLI projection.
                    return true;
                }
            }
            let mut stops = Vec::with_capacity(5);
            for control in [view.context, view.options] {
                if IsWindowEnabled(control).as_bool() {
                    stops.push(control);
                }
            }
            if !view.control.0.is_null() {
                stops.push(view.control);
            }
            if IsWindowEnabled(view.apply).as_bool() {
                stops.push(view.apply);
            }
            stops.push(view.close);
            if handle_tab(message.wParam.0, &stops) {
                return true;
            }
            if message.wParam.0 == VK_ESCAPE.0 as usize {
                finish(view, Outcome::Closed);
                return true;
            }
            if message.wParam.0 == VK_RETURN.0 as usize {
                if focus == view.close {
                    finish(view, Outcome::Closed);
                } else if focus == view.control && !view.control.0.is_null() {
                    toggle_control(view);
                } else if focus == view.apply || (focus == view.options && view.mapped) {
                    submit(view);
                }
                // Enter in the context must not reach IsDialogMessage, whose
                // default-button handling could otherwise turn reading into OK.
                return true;
            }
            if focus == view.context && handle_select_all(message.wParam.0, focus) {
                return true;
            }
        }
        if message.message == WM_CHAR
            && DIALOG.with(|dialog| {
                dialog
                    .borrow()
                    .as_ref()
                    .is_some_and(|state| state.control.active())
            })
            && matches!(GetFocus(), focus if focus == view.options || focus == view.context)
        {
            // Text input is not a TuiKey. In particular, native LISTBOX
            // type-ahead must not move reading selection away from the CLI.
            return true;
        }
        if message.message == WM_CHAR && matches!(message.wParam.0, 9 | 10 | 13 | 27) {
            return true;
        }
        dispatch_owned(view.hwnd, message);
    }
    true
}

/// Keep the original context available, disable every action against it, and
/// announce the first stale reason without changing focus or replacing text.
pub fn invalidate(reason: &str) {
    if DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .is_some_and(|state| state.control.active())
    }) {
        freeze_control(reason);
        return;
    }
    let next = DIALOG.with(|dialog| {
        let mut dialog = dialog.borrow_mut();
        let state = dialog.as_mut()?;
        if !state.ready || state.stale_reason.is_some() {
            return None;
        }
        let reason = if reason.trim().is_empty() {
            "Исходный экран CLI изменился.".to_owned()
        } else {
            reason.replace('\0', "�")
        };
        state.stale_reason = Some(reason);
        Some((state.view, format!("Диалог устарел — {}", state.caption)))
    });
    let Some((view, caption)) = next else {
        return;
    };
    if view.mapped {
        unsafe {
            let _ = EnableWindow(view.options, false);
            let _ = EnableWindow(view.apply, false);
        }
    }
    set_text(view.hwnd, &caption);
    update_status(view);
}

pub fn close() {
    if let Some(view) = current() {
        finish(view, Outcome::Closed);
    }
}

fn submit(view: View) {
    if !view.mapped {
        finish(view, Outcome::DirectInput);
        return;
    }
    if !unsafe { IsWindowEnabled(view.options).as_bool() && IsWindowEnabled(view.apply).as_bool() }
    {
        return;
    }
    let selected = unsafe { SendMessageW(view.options, LB_GETCURSEL, None, None).0 };
    let valid = DIALOG.with(|dialog| {
        dialog.borrow().as_ref().is_some_and(|state| {
            state.view.hwnd == view.hwnd
                && state.ready
                && state.stale_reason.is_none()
                && selected >= 0
                && (selected as usize) < state.options.len()
        })
    });
    if valid {
        finish(view, Outcome::Apply(selected as usize));
    }
}

fn finish(view: View, outcome: Outcome) {
    let restore = unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) == view.hwnd };
    OUTCOME.with(|result| *result.borrow_mut() = Some(outcome));
    if unsafe { DestroyWindow(view.hwnd) }.is_err() {
        OUTCOME.with(|result| *result.borrow_mut() = None);
        return;
    }
    if restore {
        restore_focus(view.owner, view.previous_focus);
    }
}

fn outcome_on_destroy(outcome: &mut Option<Outcome>) {
    if matches!(
        outcome,
        None | Some(Outcome::BeginControl | Outcome::Key(_) | Outcome::EndControl)
    ) {
        *outcome = Some(Outcome::Closed);
    }
}

fn update_status(view: View) {
    let selected = unsafe { SendMessageW(view.options, LB_GETCURSEL, None, None).0 };
    let text = DIALOG.with(|dialog| {
        let mut dialog = dialog.borrow_mut();
        let state = dialog.as_mut()?;
        let text = if state.control == ControlPhase::Starting {
            "Включается управление текущим TUI. Клавиши пока не передаются. Ctrl+Shift+I — читать снимок.".to_owned()
        } else if let ControlPhase::Pending(key) = state.control {
            format!("Клавиша {}: ожидание обновления TUI. Следующая клавиша пока не передаётся. Ctrl+Shift+I — читать снимок.", key.label())
        } else if state.control == ControlPhase::Live {
            let line = state.options.get(state.actual_selected).map_or("", String::as_str);
            format!("Управление TUI. Строка {} из {}: {}. Стрелки, Home/End, Page Up/Down, Tab, Esc и Enter передаются приложению. Ctrl+Shift+I — читать снимок.", state.actual_selected + 1, state.options.len(), status_line(line))
        } else if !view.mapped {
            let changed = if state.stale_reason.is_some() {
                "Экран обновился. Снимок сохранён для чтения. "
            } else {
                ""
            };
            let notice = state.control_notice.as_ref().map_or(String::new(), |notice| format!("{notice} "));
            format!(
                "{notice}{changed}Обзор экрана. Строка {} из {}. Стрелки перемещают только чтение; Enter ничего не отправляет. «{CONTROL_LABEL}» включает управление в этом окне; «{DIRECT_LABEL}» возвращает в терминал.",
                (selected.max(0) as usize + 1).min(state.options.len()),
                state.options.len()
            )
        } else if let Some(reason) = &state.stale_reason {
            format!("Диалог устарел: {reason} Отправка отключена. Закройте окно и откройте актуальный диалог через меню.")
        } else if selected >= 0 && (selected as usize) < state.options.len() {
            format!(
                "{}: выбран вариант {} из {} — {}. Стрелки меняют выбор. Отправка — кнопкой или Enter в списке.",
                state.cli_label,
                selected + 1,
                state.options.len(),
                state.options[selected as usize]
            )
        } else {
            "Вариант не выбран. Отправка недоступна до выбора пункта.".to_owned()
        };
        if state.status_text == text {
            return None;
        }
        state.status_text = text.clone();
        Some(text)
    });
    if let Some(text) = text {
        set_text(view.status.hwnd, &text);
        if is_foreground() {
            let _ = view.status.announce(&text);
        }
    }
}

fn announce_status(view: View) {
    if !is_foreground() {
        return;
    }
    let text = DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .map(|state| state.status_text.clone())
    });
    if let Some(text) = text {
        let _ = view.status.announce(&text);
    }
}

fn status_line(line: &str) -> String {
    let mut text: String = line.chars().take(180).collect();
    if line.chars().nth(180).is_some() {
        text.push('…');
    }
    text
}

fn native_lines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

fn apply_label(view: View) -> &'static str {
    if view.mapped {
        APPLY_LABEL
    } else {
        DIRECT_LABEL
    }
}

fn apply_fonts(view: View) {
    let mut controls = vec![
        view.context,
        view.options,
        view.apply,
        view.close,
        view.status.hwnd,
    ];
    if !view.control.0.is_null() {
        controls.push(view.control);
    }
    apply_theme(
        view.hwnd,
        &view.theme,
        &controls,
        &[view.context, view.options],
    );
    unsafe {
        for control in [view.context, view.options] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(view.theme.terminal_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
}

unsafe fn terminal_control_color(wparam: WPARAM, theme: &Theme) -> LRESULT {
    let dc = HDC(wparam.0 as *mut core::ffi::c_void);
    SetTextColor(dc, theme.colors.text_primary);
    SetBkColor(dc, theme.colors.terminal);
    LRESULT(theme.terminal.0 as isize)
}

fn layout(view: View) {
    unsafe {
        let mut rect = RECT::default();
        if GetClientRect(view.hwnd, &mut rect).is_err() {
            return;
        }
        let margin = dpi::scale(16, view.dpi);
        let gap = dpi::scale(8, view.dpi);
        let width = (rect.right - 2 * margin).max(1);
        let button_h = dpi::scale(38, view.dpi);
        let button_count = if view.mapped { 2 } else { 3 };
        let horizontal = width >= dpi::scale(if view.mapped { 640 } else { 720 }, view.dpi);
        let buttons_h = if horizontal {
            button_h
        } else {
            button_count * button_h + (button_count - 1) * gap
        };
        let button_y = (rect.bottom - margin - buttons_h).max(margin);
        let status_h = dpi::scale(72, view.dpi).min(((button_y - margin) / 3).max(1));
        let status_y = (button_y - gap - status_h).max(margin);
        let content_h = (status_y - gap - margin).max(2);
        let options_h = dpi::scale(180, view.dpi).min((content_h / 2).max(1));
        let context_h = (content_h - gap - options_h).max(1);
        let _ = MoveWindow(view.context, margin, margin, width, context_h, true);
        let _ = MoveWindow(
            view.options,
            margin,
            margin + context_h + gap,
            width,
            options_h,
            true,
        );
        let _ = MoveWindow(view.status.hwnd, margin, status_y, width, status_h, true);
        let apply_w = if horizontal && !view.mapped {
            (width - 2 * gap) / 3
        } else if horizontal {
            (width - gap) * 3 / 5
        } else {
            width
        };
        let close_w = if horizontal && !view.mapped {
            width - 2 * gap - 2 * apply_w
        } else if horizontal {
            width - gap - apply_w
        } else {
            width
        };
        if !view.mapped {
            let _ = MoveWindow(
                view.control,
                margin,
                button_y,
                if horizontal { apply_w } else { width },
                button_h,
                true,
            );
        }
        let apply_x = if horizontal && !view.mapped {
            margin + apply_w + gap
        } else {
            margin
        };
        let apply_y = if !horizontal && !view.mapped {
            button_y + button_h + gap
        } else {
            button_y
        };
        let _ = MoveWindow(view.apply, apply_x, apply_y, apply_w, button_h, true);
        let _ = MoveWindow(
            view.close,
            if horizontal {
                apply_x + apply_w + gap
            } else {
                margin
            },
            if horizontal {
                button_y
            } else {
                apply_y + button_h + gap
            },
            close_w,
            button_h,
            true,
        );
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
            if !view.context.0.is_null() {
                let _ = SetFocus(Some(view.context));
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let result = DefWindowProcW(hwnd, message, wparam, lparam);
            let status_pending = DIALOG.with(|dialog| {
                dialog.borrow().as_ref().is_some_and(|state| {
                    state.stale_reason.is_some()
                        || state.control.active()
                        || state.control_notice.is_some()
                })
            });
            if wparam.0 & 0xffff != 0 && status_pending {
                // Background invalidation never raises speech or steals focus;
                // expose its saved status when this window is activated again.
                announce_status(view);
            }
            result
        }
        WM_CLOSE => {
            finish(view, Outcome::Closed);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            let code = (wparam.0 >> 16) as u32;
            if id == ID_OPTIONS && code == LBN_SELCHANGE {
                let actual = DIALOG.with(|dialog| {
                    dialog
                        .borrow()
                        .as_ref()
                        .and_then(|state| state.control.active().then_some(state.actual_selected))
                });
                if let Some(selected) = actual {
                    let _ = select_option(view, selected);
                }
                update_status(view);
            } else if id == ID_CONTROL && code == BN_CLICKED && !view.mapped {
                toggle_control(view);
            } else if id == ID_APPLY && code == BN_CLICKED {
                submit(view);
            } else if id == ID_CLOSE && code == BN_CLICKED {
                finish(view, Outcome::Closed);
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
            DIALOG.with(|dialog| {
                if let Some(state) = dialog.borrow_mut().as_mut() {
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
                    x: dpi::scale(560, view.dpi),
                    y: dpi::scale(460, view.dpi),
                };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => erase_background(hwnd, wparam, &view.theme),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => terminal_control_color(wparam, &view.theme),
        WM_CTLCOLORSTATIC => {
            if lparam.0 == view.context.0 as isize || lparam.0 == view.options.0 as isize {
                terminal_control_color(wparam, &view.theme)
            } else {
                control_color(wparam, &view.theme, false)
            }
        }
        WM_DRAWITEM if lparam.0 != 0 => {
            let mut item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if crate::control_state::is_hot(item.hwndItem) {
                item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            draw_mode_button(
                &item,
                &view.theme,
                if wparam.0 == ID_APPLY {
                    apply_label(view)
                } else if wparam.0 == ID_CONTROL {
                    control_label()
                } else {
                    CLOSE_LABEL
                },
            );
            LRESULT(1)
        }
        WM_NCDESTROY => {
            let ready =
                DIALOG.with(|dialog| dialog.borrow_mut().take().is_some_and(|state| state.ready));
            if ready {
                OUTCOME.with(|outcome| {
                    outcome_on_destroy(&mut outcome.borrow_mut());
                });
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

    fn data() -> ViewData {
        ViewData {
            title: "Folder access".to_owned(),
            context: "Folder access\nC:\\work\\Ж😀\n1. Allow\n2. Exit\nWindows sandbox disclosure"
                .to_owned(),
            options: vec!["Allow".to_owned(), "Exit".to_owned()],
            selected: 1,
            cli_label: "Codex 0.164.0".to_owned(),
            mapped: true,
        }
    }

    #[test]
    fn rejects_incomplete_choices_instead_of_selecting_a_different_option() {
        let mut data = data();
        assert!(data.validate().is_ok());
        data.selected = 2;
        assert!(data.validate().is_err());
        data.selected = 0;
        data.options.clear();
        assert!(data.validate().is_err());
        for option in ["", "first\0hidden", "first\nsecond"] {
            data.options = vec![option.to_owned()];
            assert!(data.validate().is_err());
        }
    }

    #[test]
    fn native_context_preserves_unicode_prompt_path_choices_and_disclosure() {
        let data = data();
        let native = native_lines(&data.context);
        assert_eq!(native.replace("\r\n", "\n"), data.context);
        assert_eq!(native_lines("a\r\nb\nc\rd"), "a\r\nb\r\nc\r\nd");
    }

    #[test]
    fn reading_mode_accepts_unknown_tool_without_creating_a_mapped_action() {
        let mut data = data();
        data.mapped = false;
        data.title = "Обзор экрана TUI".to_owned();
        data.cli_label = "Unknown CLI".to_owned();
        data.options = vec!["Строка 1: > visible text".to_owned()];
        data.selected = 0;
        assert!(data.validate().is_ok());
        assert_ne!(Outcome::DirectInput, Outcome::Apply(0));
    }

    #[test]
    fn live_control_requires_explicit_begin_and_owner_acknowledgment() {
        let mut phase = ControlPhase::default();
        assert!(!phase.request_key(TuiKey::Enter, false));
        assert!(!phase.accept_update(false));
        assert!(!phase.begin(true));
        assert!(phase.begin(false));
        assert!(!phase.begin(false));
        assert!(!phase.request_key(TuiKey::Up, false));
        assert!(!phase.accept_update(true));
        assert!(phase.accept_update(false));
        assert!(phase.request_key(TuiKey::Up, false));
        assert!(!phase.request_key(TuiKey::Enter, false));
        assert!(!phase.request_key(TuiKey::Down, false));
        assert!(phase.accept_update(false));
        assert!(phase.request_key(TuiKey::Down, false));
        phase = ControlPhase::Snapshot;
        assert!(!phase.accept_update(false));
        assert!(!phase.request_key(TuiKey::Enter, false));
    }

    #[test]
    fn held_enter_cannot_confirm_a_second_frame_without_a_new_press() {
        let mut phase = ControlPhase::Live;
        assert!(phase.request_key(TuiKey::Enter, false));
        assert!(phase.accept_update(false));
        for _ in 0..3 {
            assert!(!phase.request_key(TuiKey::Enter, true));
            assert_eq!(phase, ControlPhase::Live);
        }
        assert!(phase.request_key(TuiKey::Enter, false));
        assert_eq!(phase, ControlPhase::Pending(TuiKey::Enter));
        assert_ne!(Outcome::Key(TuiKey::Enter), Outcome::Apply(0));
    }

    #[test]
    fn passthrough_whitelist_never_drops_modifiers_or_forwards_text() {
        let cases = [
            (VK_UP, TuiKey::Up),
            (VK_DOWN, TuiKey::Down),
            (VK_LEFT, TuiKey::Left),
            (VK_RIGHT, TuiKey::Right),
            (VK_HOME, TuiKey::Home),
            (VK_END, TuiKey::End),
            (VK_PRIOR, TuiKey::PageUp),
            (VK_NEXT, TuiKey::PageDown),
            (VK_ESCAPE, TuiKey::Escape),
            (VK_RETURN, TuiKey::Enter),
            (VK_TAB, TuiKey::Tab),
        ];
        for (native, expected) in cases {
            assert_eq!(
                TuiKey::from_virtual_key(native.0 as usize, false, false, false),
                Some(expected)
            );
            assert_eq!(
                TuiKey::from_virtual_key(native.0 as usize, false, true, false),
                None
            );
            assert_eq!(
                TuiKey::from_virtual_key(native.0 as usize, false, false, true),
                None
            );
            if native != VK_TAB {
                assert_eq!(
                    TuiKey::from_virtual_key(native.0 as usize, true, false, false),
                    None
                );
            }
        }
        assert_eq!(
            TuiKey::from_virtual_key(VK_TAB.0 as usize, true, false, false),
            Some(TuiKey::ShiftTab)
        );
        for key in [b'A' as usize, b'C' as usize, b' ' as usize, 0x70, 0x08] {
            assert_eq!(TuiKey::from_virtual_key(key, false, false, false), None);
        }
    }

    #[test]
    fn background_refresh_cannot_replace_an_independent_reading_snapshot() {
        let mut original = data();
        original.mapped = false;
        let view = View {
            hwnd: HWND::default(),
            owner: HWND::default(),
            previous_focus: HWND::default(),
            context: HWND::default(),
            options: HWND::default(),
            apply: HWND::default(),
            control: HWND::default(),
            close: HWND::default(),
            status: Announcer::default(),
            dpi: dpi::BASE_DPI,
            theme: Theme::default(),
            mapped: false,
        };
        DIALOG.with(|dialog| {
            *dialog.borrow_mut() = Some(State {
                view,
                context: original.context.clone(),
                options: original.options.clone(),
                actual_selected: original.selected,
                caption: original.caption(),
                cli_label: original.cli_label.clone(),
                stale_reason: None,
                status_text: String::new(),
                ready: true,
                control: ControlPhase::Snapshot,
                control_notice: None,
                chrome_live: false,
                control_button_live: false,
            })
        });
        let mut changed = data();
        changed.mapped = false;
        changed.context = "New terminal screen".to_owned();
        changed.options = vec!["New row".to_owned()];
        changed.selected = 0;
        assert!(update_live(changed).is_err());
        DIALOG.with(|dialog| {
            let state = dialog.borrow_mut().take().unwrap();
            assert_eq!(state.context, original.context);
            assert_eq!(state.options, original.options);
            assert_eq!(state.actual_selected, original.selected);
            assert_eq!(state.control, ControlPhase::Snapshot);
        });
    }

    #[test]
    fn live_status_keeps_unicode_valid_and_bounds_a_long_terminal_row() {
        let line = "Ж😀e\u{301}".repeat(100);
        let shortened = status_line(&line);
        assert_eq!(shortened.chars().count(), 181);
        assert!(shortened.ends_with('…'));
        assert!(line.starts_with(shortened.trim_end_matches('…')));
        assert_eq!(status_line("short Ж😀"), "short Ж😀");
    }

    #[test]
    fn selection_marker_changes_patch_only_the_two_changed_native_rows() {
        let previous = vec![
            "> /help  Show help".to_owned(),
            "  /status  Show status".to_owned(),
            "  /model  Choose a model".to_owned(),
        ];
        let mut next = previous.clone();
        next[0] = "  /help  Show help".to_owned();
        next[2] = "> /model  Choose a model".to_owned();
        assert_eq!(
            option_update(&previous, &next),
            OptionUpdate::Rows(vec![0, 2])
        );
        assert_eq!(option_update(&next, &next), OptionUpdate::Unchanged);
    }

    #[test]
    fn filtering_choices_preserves_the_native_prefix_and_suffix() {
        let previous: Vec<String> = ["/help", "/model", "/status", "/exit"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let next: Vec<String> = ["/help", "/status", "/exit"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            option_update(&previous, &next),
            OptionUpdate::Splice {
                start: 1,
                removed: 1,
                added: 0,
            }
        );
        assert_eq!(
            option_update(&next, &previous),
            OptionUpdate::Splice {
                start: 1,
                removed: 0,
                added: 1,
            }
        );
    }

    #[test]
    fn option_splices_cover_empty_and_completely_replaced_lists() {
        let previous = ["old".to_owned()];
        let next = ["new".to_owned(), "second".to_owned()];
        for (previous, next, expected) in [
            (
                &previous[..],
                &next[..],
                OptionUpdate::Splice {
                    start: 0,
                    removed: 1,
                    added: 2,
                },
            ),
            (
                &[][..],
                &next[..],
                OptionUpdate::Splice {
                    start: 0,
                    removed: 0,
                    added: 2,
                },
            ),
            (
                &next[..],
                &[][..],
                OptionUpdate::Splice {
                    start: 0,
                    removed: 2,
                    added: 0,
                },
            ),
        ] {
            assert_eq!(option_update(previous, next), expected);
        }
    }

    #[test]
    fn closing_overrides_pending_keys_and_preserves_explicit_terminal_results() {
        for pending in [
            None,
            Some(Outcome::BeginControl),
            Some(Outcome::Key(TuiKey::Enter)),
            Some(Outcome::EndControl),
        ] {
            let mut outcome = pending;
            outcome_on_destroy(&mut outcome);
            assert_eq!(outcome, Some(Outcome::Closed));
        }
        for final_outcome in [Outcome::Apply(1), Outcome::DirectInput, Outcome::Closed] {
            let mut outcome = Some(final_outcome);
            let expected = match &outcome {
                Some(Outcome::Apply(index)) => Some(Outcome::Apply(*index)),
                Some(Outcome::DirectInput) => Some(Outcome::DirectInput),
                _ => Some(Outcome::Closed),
            };
            outcome_on_destroy(&mut outcome);
            assert_eq!(outcome, expected);
        }
    }
}
