//! Native local-profile editor. Editing never starts a process or writes a file.
//! The caller persists the confirmed snapshot against its original baseline.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::accessibility::set_accessible_name;
use crate::dpi;
use crate::launch_profile::{resolve_adapter, AfterExit, LaunchMode, LaunchPlan, LaunchSettings};
use crate::profile::{is_builtin_profile, Profile, ProfileKind, ProfileManager};
use crate::profile_editor::{
    activate, apply_theme, child, control_color, create_dialog, dispatch_owned, erase_background,
    handle_select_all, handle_tab, owns_message, release_theme, restore_focus, set_text,
    show_validation, wide,
};
use crate::theme::{draw_mode_button, Theme};
use crate::tui_adapters::AdapterKind;
use windows::core::{w, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETLIMITTEXT, ODS_HOTLIGHT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL, VK_ESCAPE,
    VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetClientRect, GetWindowTextLengthW, GetWindowTextW, MoveWindow,
    SendMessageW, SetWindowPos, ShowWindow, BN_CLICKED, BS_OWNERDRAW, CBN_SELCHANGE,
    CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, ES_AUTOHSCROLL,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, LBN_SELCHANGE, LBS_NOINTEGRALHEIGHT,
    LBS_NOTIFY, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, MINMAXINFO, MSG,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WS_TABSTOP,
    WS_VSCROLL,
};

// Stable dialog control IDs, also used by the offline native GUI checks.
pub const ID_PROFILE: usize = 4300;
pub const ID_DEFAULT_SHELL: usize = 4301;
pub const ID_NEW: usize = 4302;
pub const ID_DUPLICATE: usize = 4303;
pub const ID_DELETE: usize = 4304;
pub const ID_PAGE: usize = 4305;
pub const ID_SAVE: usize = 4306;
pub const ID_CANCEL: usize = 4307;
pub const ID_NAME: usize = 4310;
pub const ID_EXECUTABLE: usize = 4311;
pub const ID_CWD: usize = 4312;
pub const ID_ADAPTER: usize = 4313;
pub const ID_MODE: usize = 4314;
pub const ID_SHELL: usize = 4315;
pub const ID_AFTER_EXIT: usize = 4316;
pub const ID_ARGUMENTS: usize = 4317;
pub const ID_ARGUMENT: usize = 4318;
pub const ID_ARGUMENT_ADD: usize = 4319;
pub const ID_ARGUMENT_REPLACE: usize = 4320;
pub const ID_ARGUMENT_DELETE: usize = 4321;
pub const ID_ARGUMENT_UP: usize = 4322;
pub const ID_ARGUMENT_DOWN: usize = 4323;
pub const ID_ENVIRONMENT: usize = 4324;
pub const ID_HINT: usize = 4325;

const DEFAULT_SHELL: usize = 0;
const PROFILE: usize = 1;
const PAGE: usize = 2;
const NAME: usize = 3;
const EXECUTABLE: usize = 4;
const CWD: usize = 5;
const ADAPTER: usize = 6;
const MODE: usize = 7;
const SHELL: usize = 8;
const AFTER_EXIT: usize = 9;
const ARGUMENTS: usize = 10;
const ARGUMENT: usize = 11;
const ENVIRONMENT: usize = 12;
const FIELD_COUNT: usize = 13;

const NEW: usize = 0;
const DUPLICATE: usize = 1;
const DELETE: usize = 2;
const ARGUMENT_ADD: usize = 3;
const ARGUMENT_REPLACE: usize = 4;
const ARGUMENT_DELETE: usize = 5;
const ARGUMENT_UP: usize = 6;
const ARGUMENT_DOWN: usize = 7;
const SAVE: usize = 8;
const CANCEL: usize = 9;
const BUTTON_COUNT: usize = 10;

const FIELD_NAMES: [&str; FIELD_COUNT] = [
    "Оболочка по умолчанию",
    "Профиль оболочки или CLI",
    "Раздел параметров",
    "Название профиля",
    "Программа или путь к исполняемому файлу",
    "Рабочая папка (пусто — папка по умолчанию)",
    "Адаптер доступности",
    "Способ запуска",
    "Оболочка этого профиля",
    "После завершения программы",
    "Аргументы в порядке передачи программе",
    "Один аргумент (пробелы сохраняются, кавычки для пути не нужны)",
    "Переменные окружения — ИМЯ=значение, по одной в строке",
];

const BUTTON_LABELS: [&str; BUTTON_COUNT] = [
    "Новый",
    "Дублировать",
    "Удалить",
    "Добавить",
    "Заменить",
    "Удалить аргумент",
    "Выше",
    "Ниже",
    "Сохранить",
    "Отмена",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    General,
    Launch,
    Arguments,
    Environment,
}

impl Page {
    const ALL: [Self; 4] = [
        Self::General,
        Self::Launch,
        Self::Arguments,
        Self::Environment,
    ];
    const NAMES: [&'static str; 4] = ["Основное", "Запуск", "Аргументы", "Окружение"];

    fn fields(self) -> &'static [usize] {
        match self {
            Self::General => &[NAME, EXECUTABLE, CWD],
            Self::Launch => &[ADAPTER, MODE, SHELL, AFTER_EXIT],
            Self::Arguments => &[ARGUMENTS, ARGUMENT],
            Self::Environment => &[ENVIRONMENT],
        }
    }
}

#[derive(Clone)]
pub struct LaunchProfileEditResult {
    pub baseline: ProfileManager,
    /// All local profiles, including unchanged built-ins. Other kinds belong to
    /// the baseline and must be preserved by save_local_profiles.
    pub profiles: Vec<Profile>,
    pub default_shell_profile_id: Option<String>,
    selected_id: Option<String>,
    page: Page,
    argument: Option<usize>,
}

#[derive(Clone, Copy)]
struct View {
    hwnd: HWND,
    owner: HWND,
    previous_focus: HWND,
    labels: [HWND; FIELD_COUNT],
    fields: [HWND; FIELD_COUNT],
    buttons: [HWND; BUTTON_COUNT],
    hint: HWND,
    dpi: u32,
    theme: Theme,
}

#[derive(Clone)]
struct Draft {
    profile: Profile,
    name: String,
    executable: String,
    cwd: String,
    environment: String,
    argument_selection: Option<usize>,
    argument_input: String,
}

impl Draft {
    fn from_profile(profile: Profile) -> Self {
        Self {
            name: profile.name.clone(),
            executable: profile.executable.clone(),
            cwd: profile.working_directory.clone().unwrap_or_default(),
            environment: format_environment(&profile.launch.environment),
            argument_selection: None,
            argument_input: String::new(),
            profile,
        }
    }

    fn preview(&self) -> Profile {
        let mut profile = self.profile.clone();
        if self.name != profile.name {
            profile.name = self.name.trim().to_owned();
        }
        if self.executable != profile.executable {
            profile.executable = self.executable.trim().to_owned();
        }
        if self.cwd != profile.working_directory.as_deref().unwrap_or("") {
            profile.working_directory =
                (!self.cwd.trim().is_empty()).then(|| self.cwd.trim().to_owned());
        }
        profile
    }

    fn validate(&self) -> std::result::Result<Profile, Validation> {
        let mut profile = self.preview();
        if profile.name.is_empty() || profile.name.chars().any(char::is_control) {
            return Err(Validation::new(
                Page::General,
                NAME,
                "Укажите название без управляющих символов.",
            ));
        }
        if profile.executable.is_empty() || profile.executable.chars().any(char::is_control) {
            return Err(Validation::new(
                Page::General,
                EXECUTABLE,
                "Укажите программу или путь без управляющих символов.",
            ));
        }
        if profile
            .working_directory
            .as_ref()
            .is_some_and(|path| path.chars().any(char::is_control))
        {
            return Err(Validation::new(
                Page::General,
                CWD,
                "Укажите рабочую папку без управляющих символов.",
            ));
        }
        if profile.args.iter().any(|argument| argument.contains('\0')) {
            return Err(Validation::new(
                Page::Arguments,
                ARGUMENT,
                "Аргумент не должен содержать нулевой символ.",
            ));
        }
        if self.argument_selection.is_none() && !self.argument_input.is_empty() {
            return Err(Validation::new(
                Page::Arguments,
                ARGUMENT,
                "Новый аргумент ещё не добавлен. Нажмите «Добавить», чтобы включить его в список, или очистите поле.",
            ));
        }
        if self.environment != format_environment(&self.profile.launch.environment) {
            profile.launch.environment = parse_environment(&self.environment)
                .map_err(|message| Validation::new(Page::Environment, ENVIRONMENT, &message))?;
        }
        Ok(profile)
    }
}

struct Validation {
    page: Page,
    field: usize,
    message: String,
}

impl Validation {
    fn new(page: Page, field: usize, message: &str) -> Self {
        Self {
            page,
            field,
            message: message.to_owned(),
        }
    }
}

struct State {
    view: View,
    baseline: ProfileManager,
    drafts: Vec<Draft>,
    selected: Option<usize>,
    page: Page,
    argument: Option<usize>,
    shell_ids: Vec<Option<String>>,
    default_shell_ids: Vec<Option<String>>,
    default_shell_profile_id: Option<String>,
    loading: bool,
}

thread_local! {
    static EDITOR: RefCell<Option<State>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<LaunchProfileEditResult>> = const { RefCell::new(None) };
}

fn current() -> Option<View> {
    EDITOR.with(|editor| editor.borrow().as_ref().map(|state| state.view))
}

pub fn take_result() -> Option<LaunchProfileEditResult> {
    OUTCOME.with(|outcome| outcome.borrow_mut().take())
}

pub fn show(owner: HWND, manager: &ProfileManager, selected_id: Option<&str>) -> Result<()> {
    if let Some(view) = current() {
        activate(view.hwnd, view.fields[PROFILE]);
        return Ok(());
    }
    let view = create_editor(owner, manager, selected_id)?;
    activate(view.hwnd, view.fields[PROFILE]);
    Ok(())
}

/// Reopen a confirmed draft after persistence failed. Keep the original
/// baseline so retrying still detects conflicting edits in other windows.
pub fn restore_draft(owner: HWND, result: LaunchProfileEditResult) -> Result<()> {
    let view = restore_editor(owner, result)?;
    let field = EDITOR.with(|editor| {
        let editor = editor.borrow();
        let state = editor.as_ref().expect("editor restored");
        state
            .page
            .fields()
            .iter()
            .map(|index| view.fields[*index])
            .find(|field| unsafe_enabled(*field))
            .unwrap_or(view.fields[PROFILE])
    });
    activate(view.hwnd, field);
    Ok(())
}

fn restore_editor(owner: HWND, result: LaunchProfileEditResult) -> Result<View> {
    let view = match current() {
        Some(view) => view,
        None => create_editor(owner, &result.baseline, result.selected_id.as_deref())?,
    };
    let drafts: Vec<_> = result
        .profiles
        .into_iter()
        .map(Draft::from_profile)
        .collect();
    let selected = result
        .selected_id
        .as_ref()
        .and_then(|id| drafts.iter().position(|draft| &draft.profile.id == id))
        .or_else(|| (!drafts.is_empty()).then_some(0));
    EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let state = editor.as_mut().expect("editor created");
        state.loading = true;
        state.baseline = result.baseline;
        state.drafts = drafts;
        state.selected = selected;
        state.page = result.page;
        state.default_shell_profile_id = result.default_shell_profile_id;
        if let Some(draft) = selected.and_then(|index| state.drafts.get_mut(index)) {
            draft.argument_selection = result
                .argument
                .filter(|index| *index < draft.profile.args.len());
        }
    });
    OUTCOME.with(|outcome| *outcome.borrow_mut() = None);
    refresh_profiles(view);
    load_profile(view);
    layout(view);
    Ok(view)
}

fn create_editor(owner: HWND, manager: &ProfileManager, selected_id: Option<&str>) -> Result<View> {
    let previous_focus = unsafe { GetFocus() };
    let (hwnd, instance, dpi) = create_dialog(
        owner,
        w!("BksTermLaunchProfileEditor"),
        w!("Профили оболочек и CLI — BKS Term"),
        Some(window_proc),
        820,
        600,
    )?;
    let view = View {
        hwnd,
        owner,
        previous_focus,
        labels: [HWND::default(); FIELD_COUNT],
        fields: [HWND::default(); FIELD_COUNT],
        buttons: [HWND::default(); BUTTON_COUNT],
        hint: HWND::default(),
        dpi,
        theme: Theme::new(dpi),
    };
    let drafts: Vec<_> = manager
        .profiles
        .iter()
        .filter(|profile| profile.kind == ProfileKind::Local)
        .cloned()
        .map(Draft::from_profile)
        .collect();
    let selected = selected_id
        .and_then(|id| drafts.iter().position(|draft| draft.profile.id == id))
        .or_else(|| (!drafts.is_empty()).then_some(0));
    OUTCOME.with(|outcome| *outcome.borrow_mut() = None);
    EDITOR.with(|editor| {
        *editor.borrow_mut() = Some(State {
            view,
            baseline: manager.clone(),
            drafts,
            selected,
            page: Page::General,
            argument: None,
            shell_ids: Vec::new(),
            default_shell_ids: Vec::new(),
            default_shell_profile_id: manager.default_shell_profile_id.clone(),
            loading: true,
        })
    });
    if let Err(error) = create_controls(view, instance) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    let view = current().expect("launch profile editor initialized");
    apply_fonts(view);
    refresh_profiles(view);
    load_profile(view);
    layout(view);
    Ok(view)
}

/// Dispatch before application shortcuts: no editor keystroke reaches a PTY.
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
            let (page, editable) = EDITOR.with(|editor| {
                let editor = editor.borrow();
                let state = editor.as_ref().expect("editor open");
                (
                    state.page,
                    state
                        .selected
                        .and_then(|index| state.drafts.get(index))
                        .is_some_and(|draft| !is_builtin_profile(&draft.profile)),
                )
            });
            let mut stops = vec![
                view.fields[DEFAULT_SHELL],
                view.fields[PROFILE],
                view.buttons[NEW],
                view.buttons[DUPLICATE],
            ];
            if unsafe_enabled(view.buttons[DELETE]) {
                stops.push(view.buttons[DELETE]);
            }
            stops.push(view.fields[PAGE]);
            stops.extend(
                page.fields()
                    .iter()
                    .map(|index| view.fields[*index])
                    .filter(|field| unsafe_enabled(*field)),
            );
            if page == Page::Arguments && editable {
                stops.extend(
                    view.buttons[ARGUMENT_ADD..=ARGUMENT_DOWN]
                        .iter()
                        .copied()
                        .filter(|button| unsafe_enabled(*button)),
                );
            }
            stops.extend([view.hint, view.buttons[SAVE], view.buttons[CANCEL]]);
            if handle_tab(message.wParam.0, &stops) {
                return true;
            }
            if message.wParam.0 == VK_ESCAPE.0 as usize {
                // Native open combo lists consume their first Escape themselves.
                if [
                    DEFAULT_SHELL,
                    PROFILE,
                    PAGE,
                    ADAPTER,
                    MODE,
                    SHELL,
                    AFTER_EXIT,
                ]
                .iter()
                .any(|index| view.fields[*index] == focus && combo_dropped(focus))
                {
                    dispatch_owned(view.hwnd, message);
                } else {
                    close(view, None);
                }
                return true;
            }
            if message.wParam.0 == VK_RETURN.0 as usize {
                if let Some(button) = view.buttons.iter().position(|button| *button == focus) {
                    run_button(view, button);
                    return true;
                }
                if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                    save(view);
                    return true;
                }
                // Multiline environment retains Enter; combos keep native selection.
            }
            if ([NAME, EXECUTABLE, CWD, ARGUMENT, ENVIRONMENT]
                .iter()
                .any(|index| view.fields[*index] == focus)
                || focus == view.hint)
                && handle_select_all(message.wParam.0, focus)
            {
                return true;
            }
        }
        dispatch_owned(view.hwnd, message);
    }
    true
}

fn unsafe_enabled(hwnd: HWND) -> bool {
    unsafe { IsWindowEnabled(hwnd).as_bool() }
}

unsafe fn combo_dropped(hwnd: HWND) -> bool {
    SendMessageW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::CB_GETDROPPEDSTATE,
        None,
        None,
    )
    .0 != 0
}

fn create_controls(view: View, instance: HINSTANCE) -> Result<()> {
    let ids = [
        ID_DEFAULT_SHELL,
        ID_PROFILE,
        ID_PAGE,
        ID_NAME,
        ID_EXECUTABLE,
        ID_CWD,
        ID_ADAPTER,
        ID_MODE,
        ID_SHELL,
        ID_AFTER_EXIT,
        ID_ARGUMENTS,
        ID_ARGUMENT,
        ID_ENVIRONMENT,
    ];
    let mut next = view;
    for index in 0..FIELD_COUNT {
        next.labels[index] = child(
            view.hwnd,
            instance,
            w!("STATIC"),
            FIELD_NAMES[index],
            WINDOW_STYLE::default(),
            4400 + index,
        )?;
        let (class, style) = if [
            DEFAULT_SHELL,
            PROFILE,
            PAGE,
            ADAPTER,
            MODE,
            SHELL,
            AFTER_EXIT,
        ]
        .contains(&index)
        {
            (
                w!("COMBOBOX"),
                WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
            )
        } else if index == ARGUMENTS {
            (
                w!("LISTBOX"),
                WS_VSCROLL | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
            )
        } else if index == ENVIRONMENT {
            (
                w!("EDIT"),
                WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
            )
        } else {
            (w!("EDIT"), WINDOW_STYLE(ES_AUTOHSCROLL as u32))
        };
        next.fields[index] = child(
            view.hwnd,
            instance,
            class,
            "",
            WS_TABSTOP | style,
            ids[index],
        )?;
        set_accessible_name(next.fields[index], FIELD_NAMES[index])?;
        if [NAME, EXECUTABLE, CWD, ARGUMENT, ENVIRONMENT].contains(&index) {
            unsafe {
                SendMessageW(
                    next.fields[index],
                    EM_SETLIMITTEXT,
                    Some(WPARAM(128 * 1024)),
                    None,
                );
            }
        }
    }
    let button_ids = [
        ID_NEW,
        ID_DUPLICATE,
        ID_DELETE,
        ID_ARGUMENT_ADD,
        ID_ARGUMENT_REPLACE,
        ID_ARGUMENT_DELETE,
        ID_ARGUMENT_UP,
        ID_ARGUMENT_DOWN,
        ID_SAVE,
        ID_CANCEL,
    ];
    for index in 0..BUTTON_COUNT {
        next.buttons[index] = child(
            view.hwnd,
            instance,
            w!("BUTTON"),
            BUTTON_LABELS[index],
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            button_ids[index],
        )?;
        set_accessible_name(
            next.buttons[index],
            match index {
                NEW => "Создать новый профиль",
                DUPLICATE => "Создать копию выбранного профиля",
                DELETE => "Удалить выбранный профиль",
                ARGUMENT_UP => "Переместить аргумент выше",
                ARGUMENT_DOWN => "Переместить аргумент ниже",
                _ => BUTTON_LABELS[index],
            },
        )?;
        crate::control_state::install(next.buttons[index]);
    }
    next.hint = child(
        view.hwnd,
        instance,
        w!("EDIT"),
        "",
        WS_TABSTOP
            | WS_VSCROLL
            | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
        ID_HINT,
    )?;
    set_accessible_name(next.hint, "Подсказка о параметрах профиля")?;
    EDITOR.with(|editor| {
        editor
            .borrow_mut()
            .as_mut()
            .expect("editor initialized")
            .view = next
    });
    for name in Page::NAMES {
        add_combo(next.fields[PAGE], name);
    }
    select_combo(next.fields[PAGE], 0);
    for adapter in AdapterKind::ALL {
        add_combo(next.fields[ADAPTER], adapter.label());
    }
    for name in ["Напрямую", "Через оболочку"] {
        add_combo(next.fields[MODE], name);
    }
    for name in [
        "Оставить вкладку с результатом",
        "Открыть оболочку в этой вкладке",
    ] {
        add_combo(next.fields[AFTER_EXIT], name);
    }
    Ok(())
}

fn control_text(hwnd: HWND) -> String {
    unsafe {
        let mut value = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let count = GetWindowTextW(hwnd, &mut value).max(0) as usize;
        String::from_utf16_lossy(&value[..count])
    }
}

fn add_combo(hwnd: HWND, value: &str) {
    let value = wide(value);
    unsafe {
        SendMessageW(
            hwnd,
            CB_ADDSTRING,
            None,
            Some(LPARAM(value.as_ptr() as isize)),
        );
    }
}

fn select_combo(hwnd: HWND, index: usize) {
    unsafe {
        SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}

fn combo_index(hwnd: HWND) -> Option<usize> {
    usize::try_from(unsafe { SendMessageW(hwnd, CB_GETCURSEL, None, None).0 }).ok()
}

fn profile_label(profile: &Profile) -> String {
    let group = if profile.is_shell_profile() {
        "Оболочка"
    } else {
        "CLI"
    };
    let builtin = if is_builtin_profile(profile) {
        ", встроенный"
    } else {
        ""
    };
    format!("{group}{builtin}: {}", profile.name)
}

fn refresh_profiles(view: View) {
    let (profiles, selected) = EDITOR.with(|editor| {
        let editor = editor.borrow();
        let state = editor.as_ref().expect("editor open");
        (
            state.drafts.iter().map(Draft::preview).collect::<Vec<_>>(),
            state.selected,
        )
    });
    unsafe {
        SendMessageW(view.fields[PROFILE], CB_RESETCONTENT, None, None);
    }
    for profile in &profiles {
        add_combo(view.fields[PROFILE], &profile_label(profile));
    }
    if let Some(index) = selected {
        select_combo(view.fields[PROFILE], index);
    }
    refresh_shells(view);
}

fn refresh_shells(view: View) {
    let (profiles, selected_shell, default_shell) = EDITOR.with(|editor| {
        let editor = editor.borrow();
        let state = editor.as_ref().expect("editor open");
        let shell = state
            .selected
            .and_then(|index| state.drafts.get(index))
            .and_then(|draft| draft.profile.launch.shell_profile_id.clone());
        (
            state.drafts.iter().map(Draft::preview).collect::<Vec<_>>(),
            shell,
            state.default_shell_profile_id.clone(),
        )
    });
    let choices: Vec<_> = profiles
        .iter()
        .filter(|profile| profile.is_shell_profile())
        .map(|profile| (profile.id.clone(), profile.name.clone()))
        .collect();
    let fill = |field, current: &Option<String>, inherited: &str| {
        unsafe {
            SendMessageW(field, CB_RESETCONTENT, None, None);
        }
        add_combo(field, inherited);
        let mut ids = vec![None];
        for (id, name) in &choices {
            add_combo(field, name);
            ids.push(Some(id.clone()));
        }
        if let Some(id) = current
            .as_ref()
            .filter(|id| !choices.iter().any(|(candidate, _)| candidate == *id))
        {
            let name = profiles
                .iter()
                .find(|profile| &profile.id == id)
                .map_or(id.as_str(), |profile| profile.name.as_str());
            add_combo(field, &format!("Недоступная оболочка: {name}"));
            ids.push(Some(id.clone()));
        }
        select_combo(field, ids.iter().position(|id| id == current).unwrap_or(0));
        ids
    };
    let shell_ids = fill(
        view.fields[SHELL],
        &selected_shell,
        "Использовать оболочку по умолчанию",
    );
    let default_shell_ids = fill(
        view.fields[DEFAULT_SHELL],
        &default_shell,
        "Автоматически (системная оболочка)",
    );
    EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let state = editor.as_mut().expect("editor open");
        state.shell_ids = shell_ids;
        state.default_shell_ids = default_shell_ids;
    });
}

fn capture(view: View) {
    let values: Vec<_> = [NAME, EXECUTABLE, CWD, ENVIRONMENT]
        .iter()
        .map(|index| control_text(view.fields[*index]))
        .collect();
    let selections = [ADAPTER, MODE, SHELL, AFTER_EXIT, DEFAULT_SHELL]
        .map(|index| combo_index(view.fields[index]));
    let argument_value = control_text(view.fields[ARGUMENT]);
    EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let Some(state) = editor.as_mut().filter(|state| !state.loading) else {
            return;
        };
        if let Some(index) = selections[4] {
            state.default_shell_profile_id = state.default_shell_ids.get(index).cloned().flatten();
        }
        let Some(index) = state.selected else {
            return;
        };
        let Some(draft) = state.drafts.get_mut(index) else {
            return;
        };
        if is_builtin_profile(&draft.profile) {
            return;
        }
        match state.page {
            Page::General => {
                draft.name = values[0].clone();
                draft.executable = values[1].clone();
                draft.cwd = values[2].clone();
            }
            Page::Launch => {
                if let Some(adapter) = selections[0].and_then(|index| AdapterKind::ALL.get(index)) {
                    draft.profile.launch.adapter = *adapter;
                }
                draft.profile.launch.mode = if selections[1] == Some(1) {
                    LaunchMode::Shell
                } else {
                    LaunchMode::Direct
                };
                if let Some(index) = selections[2] {
                    draft.profile.launch.shell_profile_id =
                        state.shell_ids.get(index).cloned().flatten();
                }
                draft.profile.launch.after_exit = if selections[3] == Some(1) {
                    AfterExit::OpenShell
                } else {
                    AfterExit::KeepTab
                };
            }
            Page::Arguments => {
                draft.argument_selection = state.argument;
                draft.argument_input = argument_value.clone();
                // Typing updates the selected argument when leaving the field,
                // changing a page/profile or saving; Add makes an extra argument.
                if let Some(argument) = state
                    .argument
                    .and_then(|index| draft.profile.args.get_mut(index))
                {
                    *argument = argument_value;
                }
            }
            Page::Environment => draft.environment = values[3].clone(),
        }
    });
}

fn load_profile(view: View) {
    let (draft, page) = EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let state = editor.as_mut().expect("editor open");
        state.loading = true;
        state.argument = None;
        (
            state
                .selected
                .and_then(|index| state.drafts.get(index))
                .cloned(),
            state.page,
        )
    });
    for (index, value) in [
        (NAME, draft.as_ref().map(|draft| draft.name.as_str())),
        (
            EXECUTABLE,
            draft.as_ref().map(|draft| draft.executable.as_str()),
        ),
        (CWD, draft.as_ref().map(|draft| draft.cwd.as_str())),
        (
            ENVIRONMENT,
            draft.as_ref().map(|draft| draft.environment.as_str()),
        ),
    ] {
        set_text(view.fields[index], value.unwrap_or(""));
    }
    let launch = draft
        .as_ref()
        .map(|draft| draft.profile.launch.clone())
        .unwrap_or_default();
    select_combo(
        view.fields[ADAPTER],
        AdapterKind::ALL
            .iter()
            .position(|adapter| *adapter == launch.adapter)
            .unwrap_or(0),
    );
    select_combo(
        view.fields[MODE],
        usize::from(launch.mode == LaunchMode::Shell),
    );
    select_combo(
        view.fields[AFTER_EXIT],
        usize::from(launch.after_exit == AfterExit::OpenShell),
    );
    select_combo(
        view.fields[PAGE],
        Page::ALL.iter().position(|item| *item == page).unwrap_or(0),
    );
    refresh_shells(view);
    refresh_arguments(
        view,
        draft.as_ref().and_then(|draft| draft.argument_selection),
    );
    EDITOR.with(|editor| editor.borrow_mut().as_mut().expect("editor open").loading = false);
    show_page(view);
}

fn show_page(view: View) {
    let (page, selected, builtin) = EDITOR.with(|editor| {
        let editor = editor.borrow();
        let state = editor.as_ref().expect("editor open");
        (
            state.page,
            state.selected.is_some(),
            state
                .selected
                .and_then(|index| state.drafts.get(index))
                .is_some_and(|draft| is_builtin_profile(&draft.profile)),
        )
    });
    unsafe {
        for index in NAME..FIELD_COUNT {
            let visible = page.fields().contains(&index);
            let _ = ShowWindow(view.labels[index], if visible { SW_SHOW } else { SW_HIDE });
            let _ = ShowWindow(view.fields[index], if visible { SW_SHOW } else { SW_HIDE });
            let _ = EnableWindow(view.fields[index], selected && !builtin);
        }
        // Read-only native EDIT fields remain focusable and copyable for built-ins.
        for index in [NAME, EXECUTABLE, CWD, ARGUMENT, ENVIRONMENT] {
            let _ = EnableWindow(view.fields[index], selected);
            SendMessageW(
                view.fields[index],
                windows::Win32::UI::Controls::EM_SETREADONLY,
                Some(WPARAM(usize::from(builtin))),
                None,
            );
        }
        let _ = EnableWindow(view.fields[ARGUMENTS], selected);
        let _ = EnableWindow(view.buttons[DUPLICATE], selected);
        let _ = EnableWindow(view.buttons[DELETE], selected && !builtin);
        for index in ARGUMENT_ADD..=ARGUMENT_DOWN {
            let _ = ShowWindow(
                view.buttons[index],
                if page == Page::Arguments {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            let _ = EnableWindow(view.buttons[index], selected && !builtin);
        }
    }
    let hint = if !selected {
        "Нажмите «Новый», чтобы создать профиль, или выберите существующий."
    } else if builtin {
        "Встроенный профиль доступен для просмотра. Нажмите «Дублировать», чтобы настроить свой вариант. Изменения применятся после сохранения."
    } else {
        match page {
            Page::General => "Изменения сохраняются для будущих запусков. Текущая вкладка продолжает работать. Ctrl+Enter — сохранить все профили, Escape — отменить.",
            Page::Launch => "Адаптер задаёт чтение CLI. Способ запуска задаёт запуск программы; оболочка выбирается отдельно. Открытие настроек не запускает программу.",
            Page::Arguments => "Один элемент списка — один аргумент, включая пустую строку. Правка выбранного аргумента сохраняется при переходе; «Добавить» создаёт ещё один. Кавычки для пробелов не нужны.",
            Page::Environment => "По одной переменной ИМЯ=значение в строке. Пустое значение допустимо. Настройки окружения сохраняются в файле профилей.",
        }
    };
    set_text(view.hint, hint);
    layout(view);
}

fn refresh_arguments(view: View, selection: Option<usize>) {
    let (args, pending) = EDITOR.with(|editor| {
        let editor = editor.borrow();
        let state = editor.as_ref().expect("editor open");
        state
            .selected
            .and_then(|index| state.drafts.get(index))
            .map(|draft| {
                (
                    draft.profile.args.clone(),
                    if draft.argument_selection.is_none() {
                        draft.argument_input.clone()
                    } else {
                        String::new()
                    },
                )
            })
            .unwrap_or_default()
    });
    unsafe {
        SendMessageW(view.fields[ARGUMENTS], LB_RESETCONTENT, None, None);
    }
    for (index, argument) in args.iter().enumerate() {
        let label = if argument.is_empty() {
            "(пустой аргумент)".to_owned()
        } else {
            argument
                .replace('\r', "\\r")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
        };
        let label = wide(&format!("{}. {label}", index + 1));
        unsafe {
            SendMessageW(
                view.fields[ARGUMENTS],
                LB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
    }
    let selected = selection.filter(|index| *index < args.len());
    if let Some(index) = selected {
        unsafe {
            SendMessageW(
                view.fields[ARGUMENTS],
                LB_SETCURSEL,
                Some(WPARAM(index)),
                None,
            );
        }
    }
    set_text(
        view.fields[ARGUMENT],
        selected.map_or(pending.as_str(), |index| args[index].as_str()),
    );
    EDITOR.with(|editor| editor.borrow_mut().as_mut().expect("editor open").argument = selected);
}

fn next_profile_id(state: &State) -> String {
    let mut manager = state.baseline.clone();
    manager
        .profiles
        .extend(state.drafts.iter().map(Draft::preview));
    manager.next_local_profile_id()
}

fn create_profile(view: View, duplicate: bool) {
    capture(view);
    EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let state = editor.as_mut().expect("editor open");
        let mut draft = if duplicate {
            let Some(draft) = state
                .selected
                .and_then(|index| state.drafts.get(index))
                .cloned()
            else {
                return;
            };
            draft
        } else {
            Draft::from_profile(Profile {
                id: String::new(),
                name: "Новый профиль".to_owned(),
                kind: ProfileKind::Local,
                executable: String::new(),
                args: Vec::new(),
                working_directory: None,
                launch: LaunchSettings {
                    adapter: AdapterKind::Universal,
                    after_exit: AfterExit::OpenShell,
                    ..LaunchSettings::default()
                },
            })
        };
        if duplicate && draft.profile.launch.adapter == AdapterKind::Auto {
            draft.profile.launch.adapter = resolve_adapter(&draft.preview());
        }
        draft.profile.id = next_profile_id(state);
        if duplicate {
            draft.name = format!("{} — копия", draft.name);
        }
        state.drafts.push(draft);
        state.selected = Some(state.drafts.len() - 1);
        state.page = Page::General;
    });
    refresh_profiles(view);
    load_profile(view);
    unsafe {
        let _ = SetFocus(Some(view.fields[NAME]));
    }
}

fn delete_profile(view: View) {
    capture(view);
    let error = EDITOR.with(|editor| {
        let editor = editor.borrow();
        let state = editor.as_ref().expect("editor open");
        let draft = state.selected.and_then(|index| state.drafts.get(index))?;
        if is_builtin_profile(&draft.profile) {
            return Some("Встроенный профиль нельзя удалить.".to_owned());
        }
        if state.default_shell_profile_id.as_deref() == Some(&draft.profile.id) {
            return Some("Сначала выберите другую оболочку по умолчанию.".to_owned());
        }
        if state.drafts.iter().any(|other| {
            other.profile.id != draft.profile.id
                && other.profile.launch.shell_profile_id.as_deref() == Some(&draft.profile.id)
        }) {
            return Some(
                "Эта оболочка выбрана в другом профиле. Сначала измените оболочку в том профиле."
                    .to_owned(),
            );
        }
        None
    });
    if let Some(error) = error {
        show_validation(view.hwnd, &error);
        return;
    }
    EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let state = editor.as_mut().expect("editor open");
        if let Some(index) = state.selected.take() {
            state.drafts.remove(index);
            state.selected = (!state.drafts.is_empty()).then(|| index.min(state.drafts.len() - 1));
        }
    });
    refresh_profiles(view);
    load_profile(view);
    unsafe {
        let _ = SetFocus(Some(view.fields[PROFILE]));
    }
}

fn edit_argument(view: View, action: usize) {
    let value = control_text(view.fields[ARGUMENT]);
    // Add treats the field as a new argument; capturing first would also
    // overwrite the selected argument with the same text before appending it.
    if action != ARGUMENT_ADD {
        capture(view);
    }
    let selection = EDITOR.with(|editor| {
        let mut editor = editor.borrow_mut();
        let state = editor.as_mut().expect("editor open");
        let draft = state
            .selected
            .and_then(|index| state.drafts.get_mut(index))?;
        if is_builtin_profile(&draft.profile) {
            return state.argument;
        }
        let args = &mut draft.profile.args;
        match action {
            ARGUMENT_ADD => {
                args.push(value);
                Some(args.len() - 1)
            }
            ARGUMENT_REPLACE => state.argument,
            ARGUMENT_DELETE => state.argument.and_then(|index| {
                if index < args.len() {
                    args.remove(index);
                }
                (!args.is_empty()).then(|| index.min(args.len() - 1))
            }),
            ARGUMENT_UP | ARGUMENT_DOWN => state.argument.map(|index| {
                let next = if action == ARGUMENT_UP {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(args.len().saturating_sub(1))
                };
                if index < args.len() && next < args.len() {
                    args.swap(index, next);
                }
                next
            }),
            _ => state.argument,
        }
    });
    refresh_arguments(view, selection);
    unsafe {
        let _ = SetFocus(Some(view.fields[ARGUMENT]));
    }
}

fn run_button(view: View, index: usize) {
    if !unsafe_enabled(view.buttons[index]) {
        return;
    }
    match index {
        NEW => create_profile(view, false),
        DUPLICATE => create_profile(view, true),
        DELETE => delete_profile(view),
        ARGUMENT_ADD..=ARGUMENT_DOWN => edit_argument(view, index),
        SAVE => save(view),
        CANCEL => close(view, None),
        _ => (),
    }
}

fn validated_snapshot(
    state: &State,
) -> std::result::Result<LaunchProfileEditResult, (usize, Validation)> {
    let mut profiles = Vec::with_capacity(state.drafts.len());
    for (index, draft) in state.drafts.iter().enumerate() {
        let profile = if is_builtin_profile(&draft.profile) {
            draft.profile.clone()
        } else {
            draft.validate().map_err(|validation| (index, validation))?
        };
        if state.baseline.get(&profile.id) != Some(&profile) {
            profile
                .validate()
                .map_err(|message| (index, Validation::new(Page::General, EXECUTABLE, &message)))?;
            if profile
                .working_directory
                .as_ref()
                .is_some_and(|path| !std::path::Path::new(path).is_dir())
            {
                return Err((index, Validation::new(Page::General, CWD,
                        "Рабочая папка не найдена. Выберите существующую папку или оставьте поле пустым.")));
            }
        }
        profiles.push(profile);
    }
    if let Some(id) = &state.default_shell_profile_id {
        if !profiles
            .iter()
            .any(|profile| &profile.id == id && profile.is_shell_profile())
        {
            return Err((
                state.selected.unwrap_or(0),
                Validation::new(
                    Page::Launch,
                    DEFAULT_SHELL,
                    "Выберите существующий профиль оболочки по умолчанию.",
                ),
            ));
        }
    }
    for (index, profile) in profiles.iter().enumerate() {
        if let Some(id) = &profile.launch.shell_profile_id {
            if !profiles
                .iter()
                .any(|shell| &shell.id == id && shell.is_shell_profile())
            {
                return Err((index, Validation::new(Page::Launch, SHELL,
                        "Выберите существующий профиль оболочки или наследование настройки по умолчанию.")));
            }
        }
        let mut direct = profile.clone();
        direct.launch.mode = LaunchMode::Direct;
        if direct.is_shell_profile() && profile.launch.mode == LaunchMode::Shell {
            return Err((
                index,
                Validation::new(
                    Page::Launch,
                    MODE,
                    "Профиль оболочки запускается напрямую. Выберите «Напрямую».",
                ),
            ));
        }
    }
    let mut manager = state.baseline.clone();
    manager
        .profiles
        .retain(|profile| profile.kind != ProfileKind::Local);
    manager.profiles.extend(profiles.iter().cloned());
    manager.default_shell_profile_id = state.default_shell_profile_id.clone();
    for (index, profile) in profiles.iter().enumerate() {
        if state.baseline.get(&profile.id) != Some(profile) {
            LaunchPlan::resolve(&manager, profile)
                .map_err(|message| (index, Validation::new(Page::Launch, SHELL, &message)))?;
        }
    }
    Ok(LaunchProfileEditResult {
        baseline: state.baseline.clone(),
        profiles,
        default_shell_profile_id: state.default_shell_profile_id.clone(),
        selected_id: state
            .selected
            .and_then(|index| state.drafts.get(index))
            .map(|draft| draft.profile.id.clone()),
        page: state.page,
        argument: state.argument,
    })
}

fn save(view: View) {
    capture(view);
    let result =
        EDITOR.with(|editor| validated_snapshot(editor.borrow().as_ref().expect("editor open")));
    match result {
        Ok(result) => close(view, Some(result)),
        Err((index, validation)) => {
            EDITOR.with(|editor| {
                let mut editor = editor.borrow_mut();
                let state = editor.as_mut().expect("editor open");
                state.selected = (index < state.drafts.len()).then_some(index);
                state.page = validation.page;
            });
            refresh_profiles(view);
            load_profile(view);
            show_validation(view.hwnd, &validation.message);
            unsafe {
                let _ = SetFocus(Some(view.fields[validation.field]));
            }
        }
    }
}

fn close(view: View, result: Option<LaunchProfileEditResult>) {
    OUTCOME.with(|outcome| *outcome.borrow_mut() = result);
    unsafe {
        let _ = DestroyWindow(view.hwnd);
    }
    restore_focus(view.owner, view.previous_focus);
}

fn apply_fonts(view: View) {
    let mut all = view.labels.to_vec();
    all.extend(view.fields);
    all.extend(view.buttons);
    all.push(view.hint);
    let mut edits = view.fields.to_vec();
    edits.push(view.hint);
    apply_theme(view.hwnd, &view.theme, &all, &edits);
}

fn layout(view: View) {
    let page = EDITOR
        .with(|editor| editor.borrow().as_ref().map(|state| state.page))
        .unwrap_or(Page::General);
    unsafe {
        let mut rect = RECT::default();
        if GetClientRect(view.hwnd, &mut rect).is_err() {
            return;
        }
        let scale = |value| dpi::scale(value, view.dpi);
        let margin = scale(12);
        let gap = scale(8);
        let width = (rect.right - 2 * margin).max(1);
        let label_h = scale(20);
        let field_h = scale(28);
        let row_h = label_h + field_h + gap;
        let combo_h = scale(220);
        let button_h = scale(34);
        let button_y = (rect.bottom - margin - button_h).max(margin);
        let hint_h = scale(44);
        let hint_y = (button_y - gap - hint_h).max(margin);
        for index in 0..3 {
            let y = margin + row_h * index as i32;
            let field_width = if index == PROFILE {
                (width - scale(324) - gap).max(1)
            } else {
                width
            };
            let _ = MoveWindow(view.labels[index], margin, y, field_width, label_h, true);
            let _ = MoveWindow(
                view.fields[index],
                margin,
                y + label_h,
                field_width,
                combo_h,
                true,
            );
            if index == PROFILE {
                let action_width = scale(104);
                for (offset, button) in view.buttons[..=DELETE].iter().enumerate() {
                    let x = margin + field_width + gap + (action_width + scale(4)) * offset as i32;
                    let _ = MoveWindow(*button, x, y + label_h, action_width, field_h, true);
                }
            }
        }
        let content_y = margin + 3 * row_h;
        let content_h = (hint_y - gap - content_y).max(1);
        match page {
            Page::General | Page::Launch => {
                let rows = page.fields();
                let row = (content_h / rows.len() as i32).min(row_h);
                let label = label_h.min((row - scale(20)).max(1));
                let field = (row - label - scale(4)).clamp(1, field_h);
                for (offset, index) in rows.iter().enumerate() {
                    let y = content_y + row * offset as i32;
                    let _ = MoveWindow(view.labels[*index], margin, y, width, label, true);
                    let combo = [ADAPTER, MODE, SHELL, AFTER_EXIT].contains(index);
                    let _ = MoveWindow(
                        view.fields[*index],
                        margin,
                        y + label,
                        width,
                        if combo { combo_h } else { field },
                        true,
                    );
                }
            }
            Page::Arguments => {
                let actions_h = scale(30);
                let list_h =
                    (content_h - 2 * label_h - field_h - actions_h - 2 * gap).max(scale(40));
                let _ = MoveWindow(
                    view.labels[ARGUMENTS],
                    margin,
                    content_y,
                    width,
                    label_h,
                    true,
                );
                let _ = MoveWindow(
                    view.fields[ARGUMENTS],
                    margin,
                    content_y + label_h,
                    width,
                    list_h,
                    true,
                );
                let y = content_y + label_h + list_h + gap;
                let _ = MoveWindow(view.labels[ARGUMENT], margin, y, width, label_h, true);
                let _ = MoveWindow(
                    view.fields[ARGUMENT],
                    margin,
                    y + label_h,
                    width,
                    field_h,
                    true,
                );
                let action_width = ((width - 4 * gap) / 5).max(1);
                for (offset, button) in view.buttons[ARGUMENT_ADD..=ARGUMENT_DOWN]
                    .iter()
                    .enumerate()
                {
                    let _ = MoveWindow(
                        *button,
                        margin + (action_width + gap) * offset as i32,
                        y + label_h + field_h + gap,
                        action_width,
                        actions_h,
                        true,
                    );
                }
            }
            Page::Environment => {
                let _ = MoveWindow(
                    view.labels[ENVIRONMENT],
                    margin,
                    content_y,
                    width,
                    label_h,
                    true,
                );
                let _ = MoveWindow(
                    view.fields[ENVIRONMENT],
                    margin,
                    content_y + label_h,
                    width,
                    (content_h - label_h).max(1),
                    true,
                );
            }
        }
        let _ = MoveWindow(view.hint, margin, hint_y, width, hint_h, true);
        let button_w = scale(128).min((width - gap).max(2) / 2);
        let _ = MoveWindow(
            view.buttons[SAVE],
            rect.right - margin - 2 * button_w - gap,
            button_y,
            button_w,
            button_h,
            true,
        );
        let _ = MoveWindow(
            view.buttons[CANCEL],
            rect.right - margin - button_w,
            button_y,
            button_w,
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
            let _ = SetFocus(Some(view.fields[PROFILE]));
            LRESULT(0)
        }
        WM_CLOSE => {
            close(view, None);
            LRESULT(0)
        }
        WM_COMMAND => {
            let loading =
                EDITOR.with(|editor| editor.borrow().as_ref().is_none_or(|state| state.loading));
            if loading {
                return LRESULT(0);
            }
            let id = wparam.0 & 0xffff;
            let code = (wparam.0 >> 16) as u32;
            if code == BN_CLICKED {
                let ids = [
                    ID_NEW,
                    ID_DUPLICATE,
                    ID_DELETE,
                    ID_ARGUMENT_ADD,
                    ID_ARGUMENT_REPLACE,
                    ID_ARGUMENT_DELETE,
                    ID_ARGUMENT_UP,
                    ID_ARGUMENT_DOWN,
                    ID_SAVE,
                    ID_CANCEL,
                ];
                if let Some(index) = ids.iter().position(|candidate| *candidate == id) {
                    run_button(view, index);
                }
            } else if code == CBN_SELCHANGE && id == ID_PROFILE {
                capture(view);
                EDITOR.with(|editor| {
                    editor.borrow_mut().as_mut().expect("editor open").selected =
                        combo_index(view.fields[PROFILE])
                });
                refresh_profiles(view);
                load_profile(view);
            } else if code == CBN_SELCHANGE && id == ID_PAGE {
                capture(view);
                EDITOR.with(|editor| {
                    let mut editor = editor.borrow_mut();
                    let state = editor.as_mut().expect("editor open");
                    state.page = combo_index(view.fields[PAGE])
                        .and_then(|index| Page::ALL.get(index))
                        .copied()
                        .unwrap_or(Page::General);
                });
                refresh_profiles(view);
                load_profile(view);
            } else if code == CBN_SELCHANGE
                && [
                    ID_DEFAULT_SHELL,
                    ID_ADAPTER,
                    ID_MODE,
                    ID_SHELL,
                    ID_AFTER_EXIT,
                ]
                .contains(&id)
            {
                capture(view);
            } else if code == LBN_SELCHANGE && id == ID_ARGUMENTS {
                capture(view);
                let selected = usize::try_from(
                    SendMessageW(view.fields[ARGUMENTS], LB_GETCURSEL, None, None).0,
                )
                .ok();
                let value = EDITOR.with(|editor| {
                    let mut editor = editor.borrow_mut();
                    let state = editor.as_mut().expect("editor open");
                    state.argument = selected;
                    state
                        .selected
                        .and_then(|index| state.drafts.get(index))
                        .and_then(|draft| selected.and_then(|index| draft.profile.args.get(index)))
                        .cloned()
                        .unwrap_or_default()
                });
                set_text(view.fields[ARGUMENT], &value);
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
            EDITOR.with(|editor| editor.borrow_mut().as_mut().expect("editor open").view = next);
            apply_fonts(next);
            release_theme(view.theme);
            if message == WM_DPICHANGED {
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
                    x: dpi::scale(640, view.dpi),
                    y: dpi::scale(470, view.dpi),
                };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => erase_background(hwnd, wparam, &view.theme),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            control_color(wparam, &view.theme, message == WM_CTLCOLOREDIT)
        }
        WM_DRAWITEM if lparam.0 != 0 => {
            let mut item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if crate::control_state::is_hot(item.hwndItem) {
                item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            if let Some(index) = view
                .buttons
                .iter()
                .position(|button| *button == item.hwndItem)
            {
                draw_mode_button(&item, &view.theme, BUTTON_LABELS[index]);
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_NCDESTROY => {
            EDITOR.with(|editor| *editor.borrow_mut() = None);
            release_theme(view.theme);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn format_environment(environment: &BTreeMap<String, String>) -> String {
    environment
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn parse_environment(text: &str) -> std::result::Result<BTreeMap<String, String>, String> {
    let mut environment = BTreeMap::new();
    let mut names = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fail = |reason: &str| format!("Строка {} окружения: {reason}", index + 1);
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| fail("используйте ИМЯ=значение."))?;
        if name.is_empty() || name.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(fail(
                "имя должно быть непустым, без пробелов и управляющих символов.",
            ));
        }
        if value.contains('\0') || value.contains('\r') {
            return Err(fail("значение содержит недопустимый управляющий символ."));
        }
        if !names.insert(name.to_uppercase()) {
            return Err(fail(
                "имя переменной уже задано (регистр букв не различается).",
            ));
        }
        environment.insert(name.to_owned(), value.to_owned());
    }
    Ok(environment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
        DeleteObject, FillRect, GetBkColor, GetDC, GetPixel, GetTextColor, ReleaseDC, SelectObject,
    };
    use windows::Win32::UI::Controls::{GetComboBoxInfo, SetWindowTheme, COMBOBOXINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, GetDlgCtrlID, GetWindowLongW, GetWindowRect, IsWindowVisible, CB_GETCOUNT,
        GWL_STYLE, PRF_CLIENT, PRF_ERASEBKGND, WINDOW_STYLE, WM_CTLCOLORLISTBOX, WM_PRINTCLIENT,
        WS_EX_NOACTIVATE,
    };

    static NATIVE_EDITOR: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct HiddenEditor {
        owner: HWND,
        view: View,
        _apartment: crate::accessibility::ComApartment,
        _native_editor: std::sync::MutexGuard<'static, ()>,
    }

    impl HiddenEditor {
        fn new(manager: &ProfileManager, selected: &str) -> Self {
            let native_editor = NATIVE_EDITOR
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let apartment = crate::accessibility::ComApartment::initialize().unwrap();
            let owner = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    w!("STATIC"),
                    w!("BKS private launch editor fixture"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    1280,
                    760,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            let view = create_editor(owner, manager, Some(selected)).unwrap();
            assert!(!unsafe { IsWindowVisible(view.hwnd).as_bool() });
            Self {
                owner,
                view,
                _apartment: apartment,
                _native_editor: native_editor,
            }
        }

        fn command(&self, id: usize, notification: u32) {
            unsafe {
                SendMessageW(
                    self.view.hwnd,
                    WM_COMMAND,
                    Some(WPARAM(id | ((notification as usize) << 16))),
                    None,
                );
            }
        }

        fn page(&self, page: Page) {
            select_combo(
                self.view.fields[PAGE],
                Page::ALL
                    .iter()
                    .position(|candidate| *candidate == page)
                    .unwrap(),
            );
            self.command(ID_PAGE, CBN_SELCHANGE);
        }
    }

    impl Drop for HiddenEditor {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.view.hwnd);
                let _ = DestroyWindow(self.owner);
            }
        }
    }

    fn painted_combo_background(combo: HWND) -> COLORREF {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(combo, &mut rect).unwrap();
            let screen = GetDC(Some(combo));
            let dc = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, rect.right, rect.bottom);
            let previous = SelectObject(dc, bitmap.into());
            let sentinel = COLORREF(0x00ff_00ff);
            let brush = CreateSolidBrush(sentinel);
            FillRect(dc, &rect, brush);
            SendMessageW(
                combo,
                WM_PRINTCLIENT,
                Some(WPARAM(dc.0 as usize)),
                Some(LPARAM((PRF_CLIENT | PRF_ERASEBKGND) as isize)),
            );
            let pixel = GetPixel(dc, rect.right - 40, rect.bottom / 2);
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteObject(brush.into());
            let _ = DeleteDC(dc);
            ReleaseDC(Some(combo), screen);
            assert_ne!(
                pixel, sentinel,
                "native combo must actually paint the sampled pixel"
            );
            pixel
        }
    }

    fn manager() -> ProfileManager {
        let shell = Profile {
            id: "windows-cmd".to_owned(),
            name: "Command Prompt".to_owned(),
            executable: "cmd.exe".to_owned(),
            args: vec!["/Q".to_owned()],
            working_directory: None,
            launch: LaunchSettings::default(),
            kind: ProfileKind::Local,
        };
        let mut cli = profile();
        cli.id = "cli-codex".to_owned();
        cli.executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        cli.working_directory = None;
        cli.launch
            .environment
            .insert("EXAMPLE".to_owned(), "one=two".to_owned());
        let mut ssh = shell.clone();
        ssh.kind = ProfileKind::Ssh;
        ssh.id = "ssh-fixture".to_owned();
        ProfileManager {
            profiles: vec![shell, cli, ssh],
            default_profile_id: "windows-cmd".to_owned(),
            default_shell_profile_id: Some("windows-cmd".to_owned()),
        }
    }

    fn profile() -> Profile {
        Profile {
            id: "local-1".to_owned(),
            name: "CLI с настройками".to_owned(),
            kind: ProfileKind::Local,
            executable: "example.exe".to_owned(),
            args: vec![
                "--name".to_owned(),
                "два слова".to_owned(),
                String::new(),
                "a=\"b\"".to_owned(),
            ],
            working_directory: Some(r"C:\Рабочая папка".to_owned()),
            launch: LaunchSettings::default(),
        }
    }

    #[test]
    fn environment_preserves_values_unicode_equals_and_empty_values() {
        let value =
            parse_environment("NAME= два слова = ещё \r\nEMPTY=\r\n\r\nПЕРЕМЕННАЯ=значение")
                .unwrap();
        assert_eq!(value["NAME"], " два слова = ещё ");
        assert_eq!(value["EMPTY"], "");
        assert_eq!(value["ПЕРЕМЕННАЯ"], "значение");
        assert_eq!(
            parse_environment(&format_environment(&value)).unwrap(),
            value
        );
    }

    #[test]
    fn environment_rejects_ambiguous_names_without_echoing_values() {
        for text in [
            "Path=one\r\nPATH=two",
            "=value",
            "NAME ONLY=value",
            "MISSING",
            "NAME=a\0b",
        ] {
            let error = parse_environment(text).unwrap_err();
            assert!(error.starts_with("Строка "));
            assert!(!error.contains("=value"));
            assert!(!error.contains("one"));
        }
    }

    #[test]
    fn draft_roundtrip_preserves_argument_boundaries_and_launch_settings() {
        let mut original = profile();
        original.launch.adapter = AdapterKind::Codex;
        original.launch.mode = LaunchMode::Shell;
        original.launch.after_exit = AfterExit::OpenShell;
        original.launch.shell_profile_id = Some("pwsh".to_owned());
        original
            .launch
            .environment
            .insert("CUSTOM".to_owned(), " x=y ".to_owned());
        let saved = Draft::from_profile(original.clone())
            .validate()
            .unwrap_or_else(|error| panic!("{}", error.message));
        assert_eq!(saved, original);
    }

    #[test]
    fn draft_validation_identifies_the_page_and_field() {
        let mut draft = Draft::from_profile(profile());
        draft.name = " ".to_owned();
        let error = draft.validate().err().unwrap();
        assert_eq!((error.page, error.field), (Page::General, NAME));
        draft.name = "profile".to_owned();
        draft.environment = "MISSING_EQUALS".to_owned();
        let error = draft.validate().err().unwrap();
        assert_eq!((error.page, error.field), (Page::Environment, ENVIRONMENT));
    }

    #[test]
    fn native_editor_copies_templates_keeps_drafts_and_returns_a_baseline_snapshot() {
        let baseline = manager();
        let fixture = HiddenEditor::new(&baseline, "cli-codex");
        let view = fixture.view;
        assert_eq!(unsafe { GetDlgCtrlID(view.fields[NAME]) }, ID_NAME as i32);
        assert!(!unsafe_enabled(view.buttons[DELETE]));
        assert_eq!(
            control_text(view.fields[EXECUTABLE]),
            baseline.profiles[1].executable
        );
        fixture.command(ID_DUPLICATE, BN_CLICKED);
        assert!(unsafe_enabled(view.buttons[DELETE]));
        set_text(view.fields[NAME], "Мой вариант CLI");
        fixture.page(Page::Environment);
        set_text(view.fields[ENVIRONMENT], "INCOMPLETE");
        fixture.page(Page::Launch);
        fixture.page(Page::Environment);
        assert_eq!(control_text(view.fields[ENVIRONMENT]), "INCOMPLETE");
        set_text(
            view.fields[ENVIRONMENT],
            "CUSTOM= два слова = ещё \r\nEMPTY=",
        );
        fixture.page(Page::Arguments);
        set_text(view.fields[ARGUMENT], "новый аргумент с пробелами");
        fixture.page(Page::General);
        fixture.page(Page::Arguments);
        assert_eq!(
            control_text(view.fields[ARGUMENT]),
            "новый аргумент с пробелами"
        );
        fixture.command(ID_ARGUMENT_ADD, BN_CLICKED);
        fixture.command(ID_ARGUMENT_UP, BN_CLICKED);
        fixture.command(ID_ARGUMENT_DOWN, BN_CLICKED);
        fixture.command(ID_ARGUMENT_DELETE, BN_CLICKED);
        fixture.command(ID_NEW, BN_CLICKED);
        assert_eq!(combo_index(view.fields[ADAPTER]), Some(2));
        fixture.command(ID_DELETE, BN_CLICKED);
        capture(view);
        let result = EDITOR
            .with(|editor| validated_snapshot(editor.borrow().as_ref().unwrap()))
            .unwrap_or_else(|(_, error)| panic!("{}", error.message));
        assert_eq!(result.baseline.profiles, baseline.profiles);
        assert_eq!(result.profiles.len(), 3);
        let custom = result
            .profiles
            .iter()
            .find(|profile| profile.id == "local-1")
            .unwrap();
        assert_eq!(custom.name, "Мой вариант CLI");
        assert_eq!(
            custom.launch.adapter,
            resolve_adapter(&baseline.profiles[1])
        );
        assert_eq!(custom.args, baseline.profiles[1].args);
        assert_eq!(custom.launch.environment["CUSTOM"], " два слова = ещё ");
        assert_eq!(custom.launch.environment["EMPTY"], "");
        assert_eq!(
            result.default_shell_profile_id.as_deref(),
            Some("windows-cmd")
        );
        assert!(take_result().is_none());
        assert!(!unsafe { IsWindowVisible(view.hwnd).as_bool() });
    }

    #[test]
    fn native_argument_add_preserves_boundaries_and_save_reopen_roundtrip() {
        let mut baseline = manager();
        let mut existing = baseline.profiles[1].clone();
        existing.id = "local-42".to_owned();
        existing.args.clear();
        existing.launch.adapter = AdapterKind::Codex;
        baseline.profiles.push(existing);
        let mut fixture = HiddenEditor::new(&baseline, "local-42");
        fixture.page(Page::Arguments);
        for value in ["alpha", "beta", "", "two words"] {
            set_text(fixture.view.fields[ARGUMENT], value);
            fixture.command(ID_ARGUMENT_ADD, BN_CLICKED);
        }
        let expected = ["alpha", "beta", "", "two words"]
            .map(str::to_owned)
            .to_vec();
        EDITOR.with(|editor| {
            let editor = editor.borrow();
            let state = editor.as_ref().unwrap();
            assert_eq!(state.drafts[state.selected.unwrap()].profile.args, expected);
        });
        unsafe {
            SendMessageW(
                fixture.view.fields[ARGUMENTS],
                LB_SETCURSEL,
                Some(WPARAM(1)),
                None,
            );
        }
        fixture.command(ID_ARGUMENTS, LBN_SELCHANGE);
        set_text(fixture.view.fields[ARGUMENT], "beta replaced");
        fixture.command(ID_ARGUMENT_REPLACE, BN_CLICKED);
        fixture.command(ID_SAVE, BN_CLICKED);
        assert!(current().is_none());
        let result = take_result().unwrap();
        let expected = ["alpha", "beta replaced", "", "two words"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(
            result
                .profiles
                .iter()
                .find(|profile| profile.id == "local-42")
                .unwrap()
                .args,
            expected
        );
        fixture.view = restore_editor(fixture.owner, result).unwrap();
        assert_eq!(combo_index(fixture.view.fields[PAGE]), Some(2));
        assert_eq!(control_text(fixture.view.fields[ARGUMENT]), "beta replaced");
        capture(fixture.view);
        let reopened = EDITOR
            .with(|editor| validated_snapshot(editor.borrow().as_ref().unwrap()))
            .unwrap_or_else(|(_, error)| panic!("{}", error.message));
        assert_eq!(
            reopened
                .profiles
                .iter()
                .find(|profile| profile.id == "local-42")
                .unwrap()
                .args,
            expected
        );
        assert_eq!(reopened.baseline.profiles, baseline.profiles);
    }

    #[test]
    fn native_argument_add_to_a_copy_preserves_the_template_argument() {
        let mut baseline = manager();
        baseline.profiles[1].args = vec!["base".to_owned()];
        let fixture = HiddenEditor::new(&baseline, "cli-codex");
        fixture.command(ID_DUPLICATE, BN_CLICKED);
        fixture.page(Page::Arguments);
        unsafe {
            SendMessageW(
                fixture.view.fields[ARGUMENTS],
                LB_SETCURSEL,
                Some(WPARAM(0)),
                None,
            );
        }
        fixture.command(ID_ARGUMENTS, LBN_SELCHANGE);
        set_text(fixture.view.fields[ARGUMENT], "additional argument");
        fixture.command(ID_ARGUMENT_ADD, BN_CLICKED);
        capture(fixture.view);
        let result = EDITOR
            .with(|editor| validated_snapshot(editor.borrow().as_ref().unwrap()))
            .unwrap_or_else(|(_, error)| panic!("{}", error.message));
        assert_eq!(
            result
                .profiles
                .iter()
                .find(|profile| profile.id == "local-1")
                .unwrap()
                .args,
            ["base", "additional argument"]
        );
        assert_eq!(
            result
                .profiles
                .iter()
                .find(|profile| profile.id == "cli-codex")
                .unwrap()
                .args,
            ["base"]
        );
        assert_eq!(result.baseline.profiles, baseline.profiles);
    }

    #[test]
    fn native_combos_paint_dark_keep_strings_and_restore_system_theme_for_high_contrast() {
        let fixture = HiddenEditor::new(&manager(), "cli-codex");
        let view = fixture.view;
        let combos = [
            view.fields[DEFAULT_SHELL],
            view.fields[PROFILE],
            view.fields[PAGE],
        ];
        let before: Vec<_> = combos
            .iter()
            .map(|combo| {
                (
                    control_text(*combo),
                    unsafe { SendMessageW(*combo, CB_GETCOUNT, None, None).0 },
                    unsafe { GetWindowLongW(*combo, GWL_STYLE) },
                )
            })
            .collect();
        if !view.theme.high_contrast {
            unsafe {
                SetWindowTheme(
                    combos[0],
                    windows::core::PCWSTR::null(),
                    windows::core::PCWSTR::null(),
                )
                .unwrap();
            }
            let system_background = painted_combo_background(combos[0]);
            unsafe {
                SetWindowTheme(
                    combos[0],
                    w!("DarkMode_Explorer"),
                    windows::core::PCWSTR::null(),
                )
                .unwrap();
            }
            let old_background = painted_combo_background(combos[0]);
            apply_fonts(view);
            let dark_background = painted_combo_background(combos[0]);
            assert_ne!(
                dark_background, old_background,
                "combo needs its native CFD dark theme"
            );
            for combo in combos {
                let pixel = painted_combo_background(combo).0;
                assert!(
                    (pixel & 0xff) < 128
                        && ((pixel >> 8) & 0xff) < 128
                        && ((pixel >> 16) & 0xff) < 128,
                    "native combo background should be dark: {pixel:#x}"
                );
            }
            // Exercise the HC path without changing the user's system setting.
            let system_theme = Theme {
                high_contrast: true,
                ..view.theme
            };
            apply_theme(view.hwnd, &system_theme, &[], &combos);
            assert_eq!(painted_combo_background(combos[0]), system_background);
            apply_fonts(view);
        }
        for (index, combo) in combos.iter().enumerate() {
            assert_eq!(control_text(*combo), before[index].0);
            assert_eq!(
                unsafe { SendMessageW(*combo, CB_GETCOUNT, None, None).0 },
                before[index].1
            );
            assert_eq!(
                unsafe { GetWindowLongW(*combo, GWL_STYLE) },
                before[index].2
            );
        }
        unsafe {
            let mut info = COMBOBOXINFO {
                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            GetComboBoxInfo(combos[0], &mut info).unwrap();
            assert!(!info.hwndList.0.is_null());
            let dc = GetDC(Some(info.hwndList));
            let brush = SendMessageW(
                view.hwnd,
                WM_CTLCOLORLISTBOX,
                Some(WPARAM(dc.0 as usize)),
                Some(LPARAM(info.hwndList.0 as isize)),
            );
            let background = GetBkColor(dc);
            let foreground = GetTextColor(dc);
            ReleaseDC(Some(info.hwndList), dc);
            assert_eq!(brush.0, view.theme.surface.0 as isize);
            assert_eq!(background, view.theme.colors.surface);
            assert_eq!(foreground, view.theme.colors.text_primary);
        }
        assert!(!unsafe { IsWindowVisible(view.hwnd).as_bool() });
    }

    #[test]
    fn native_failed_save_restores_the_draft_and_original_merge_baseline() {
        let mut baseline = manager();
        let mut existing = baseline.profiles[1].clone();
        existing.id = "local-42".to_owned();
        existing.name = "Сохранённый вариант".to_owned();
        existing.launch.adapter = AdapterKind::Codex;
        baseline.profiles.push(existing);
        let mut fixture = HiddenEditor::new(&baseline, "local-42");
        let view = fixture.view;
        set_text(view.fields[NAME], "Исправленный вариант");
        fixture.page(Page::Arguments);
        unsafe {
            SendMessageW(view.fields[ARGUMENTS], LB_SETCURSEL, Some(WPARAM(1)), None);
        }
        fixture.command(ID_ARGUMENTS, LBN_SELCHANGE);
        set_text(view.fields[ARGUMENT], "новый точный аргумент");
        fixture.page(Page::Environment);
        set_text(view.fields[ENVIRONMENT], "EMPTY=\r\nVALUE=Пример = x ");
        select_combo(view.fields[DEFAULT_SHELL], 0);
        fixture.command(ID_DEFAULT_SHELL, CBN_SELCHANGE);
        fixture.command(ID_SAVE, BN_CLICKED);
        assert!(current().is_none());
        let result = take_result().expect("confirmed draft survives window destruction");
        let desired = result.profiles.clone();

        struct Store(std::path::PathBuf);
        impl Drop for Store {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("profiles.json"));
                let _ = std::fs::remove_file(self.0.join("profiles.json.lock"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let directory = std::env::temp_dir().join(format!(
            "bks-launch-editor-retry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let store = Store(directory);
        let path = store.0.join("profiles.json");
        baseline.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        {
            let _lock = crate::config_lock::ConfigLock::acquire(&path).unwrap();
            assert!(ProfileManager::save_local_profiles(
                &path,
                &result.baseline,
                result.profiles.clone(),
                result.default_shell_profile_id.clone()
            )
            .is_err());
        }
        fixture.view = restore_editor(fixture.owner, result).unwrap();
        let view = fixture.view;
        assert!(!unsafe { IsWindowVisible(view.hwnd).as_bool() });
        assert_eq!(combo_index(view.fields[PAGE]), Some(3));
        assert_eq!(combo_index(view.fields[DEFAULT_SHELL]), Some(0));
        assert_eq!(control_text(view.fields[NAME]), "Исправленный вариант");
        assert_eq!(
            control_text(view.fields[ENVIRONMENT]),
            "EMPTY=\r\nVALUE=Пример = x "
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(take_result().is_none());
        capture(view);
        let retried = EDITOR
            .with(|editor| {
                let editor = editor.borrow();
                let state = editor.as_ref().unwrap();
                assert_eq!(state.drafts[state.selected.unwrap()].profile.id, "local-42");
                assert_eq!(state.page, Page::Environment);
                assert_eq!(state.argument, Some(1));
                validated_snapshot(state)
            })
            .unwrap_or_else(|(_, error)| panic!("{}", error.message));
        assert_eq!(retried.baseline.profiles, baseline.profiles);
        assert_eq!(
            retried.baseline.default_shell_profile_id,
            baseline.default_shell_profile_id
        );
        assert_eq!(retried.profiles, desired);
        assert_eq!(retried.default_shell_profile_id, None);
        assert_eq!(&retried.profiles[..2], &baseline.profiles[..2]);

        let mut latest = baseline.clone();
        let mut foreign = baseline.profiles.last().unwrap().clone();
        foreign.id = "local-77".to_owned();
        foreign.name = "Другой вариант".to_owned();
        latest.profiles.push(foreign.clone());
        latest.save(&path).unwrap();
        let saved = ProfileManager::save_local_profiles(
            &path,
            &retried.baseline,
            retried.profiles,
            retried.default_shell_profile_id,
        )
        .unwrap();
        assert_eq!(saved.get("local-77"), Some(&foreign));
        let edited = saved.get("local-42").unwrap();
        assert_eq!(edited.name, "Исправленный вариант");
        assert_eq!(edited.args[1], "новый точный аргумент");
        assert_eq!(edited.launch.environment["VALUE"], "Пример = x ");
        assert_eq!(edited.launch.environment["EMPTY"], "");
        assert_eq!(saved.default_shell_profile_id, None);
    }

    #[test]
    fn native_pages_fit_a_1280_by_800_desktop_at_100_and_150_percent() {
        let fixture = HiddenEditor::new(&manager(), "cli-codex");
        for dpi in [96, 144] {
            let rect = RECT {
                left: 0,
                top: 0,
                right: 1240,
                bottom: 740,
            };
            unsafe {
                SendMessageW(
                    fixture.view.hwnd,
                    WM_DPICHANGED,
                    Some(WPARAM(dpi)),
                    Some(LPARAM((&rect as *const RECT) as isize)),
                );
            }
            for page in Page::ALL {
                fixture.page(page);
                let view = current().unwrap();
                let mut client = RECT::default();
                let mut origin = POINT::default();
                unsafe {
                    GetClientRect(view.hwnd, &mut client).unwrap();
                    assert!(ClientToScreen(view.hwnd, &mut origin).as_bool());
                }
                let mut controls = Vec::new();
                for index in [DEFAULT_SHELL, PROFILE, PAGE].iter().chain(page.fields()) {
                    controls.extend([view.labels[*index], view.fields[*index]]);
                }
                controls.extend(view.buttons[..=DELETE].iter().copied());
                if page == Page::Arguments {
                    controls.extend(view.buttons[ARGUMENT_ADD..=ARGUMENT_DOWN].iter().copied());
                }
                controls.extend([view.hint, view.buttons[SAVE], view.buttons[CANCEL]]);
                for control in controls {
                    let mut bounds = RECT::default();
                    unsafe {
                        GetWindowRect(control, &mut bounds).unwrap();
                    }
                    assert!(bounds.left >= origin.x && bounds.top >= origin.y
                        && bounds.right <= origin.x + client.right && bounds.bottom <= origin.y + client.bottom,
                        "control {} escaped client at dpi {dpi} on {page:?}: {bounds:?}, {client:?}", unsafe { GetDlgCtrlID(control) });
                }
                let mut hint = RECT::default();
                unsafe {
                    GetWindowRect(view.hint, &mut hint).unwrap();
                }
                for index in page.fields() {
                    let mut bounds = RECT::default();
                    unsafe {
                        GetWindowRect(view.fields[*index], &mut bounds).unwrap();
                    }
                    assert!(
                        bounds.bottom <= hint.top,
                        "field {index} overlaps hint at dpi {dpi} on {page:?}"
                    );
                }
            }
        }
    }
}
