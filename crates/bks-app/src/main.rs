#![windows_subsystem = "windows"]

mod accessibility;
mod agent_protocol;
mod agent_transport;
mod agent_window;
mod announcements;
pub mod ansi_palette;
mod background;
mod cli_profiles;
mod clipboard;
mod codex_cli;
mod command_blocks;
mod config_lock;
mod control_state;
mod desktop;
mod direct_accessibility;
mod dpi;
mod feedback;
mod folder_picker;
mod help;
mod journal;
mod keyboard_navigation;
mod launch;
mod layout;
mod native_text;
mod output_speech;
mod owned_child;
mod preferences;
mod profile;
mod profile_editor;
mod profile_icons;
mod profile_menu;
mod reading_state;
mod search;
mod search_bar;
mod secret_input;
mod session;
mod settings;
mod shell;
mod shortcuts;
mod system_caret;
mod system_colors;
mod terminal_input;
mod terminal_output;
mod terminal_view;
mod theme;
mod tui_adapter_host;
mod tui_dialog;
mod tui_input_echo;
mod tui_reading;
mod tui_transcript;
mod tui_window;
mod tui_worker;
mod window_frame;
mod workspace_state;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use accessibility::{set_accessible_name, ComApartment};
use bks_pty::PtySize;
use command_blocks::CommandBlocks;
use output_speech::SpeechMode;
use profile::ProfileManager;
use session::{SearchOrigin, SessionManager, SessionState};
use shell::{
    draw_connection_status, draw_sidebar_button, draw_tab_background, draw_tab_button,
    draw_tab_close, draw_title_bar, draw_window_button, SidebarButton, WindowButton,
};
use terminal_input::{
    application_keypad_bytes, control_char, encode_key, paste_bytes, SpecialKey, Utf16Encoder,
};
use theme::{draw_execute_button, draw_mode_button, Theme};
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, InvalidateRect, SetBkColor, SetTextColor, HBRUSH, HDC,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::{SS_CENTERIMAGE, SS_OWNERDRAW};
use windows::Win32::UI::Controls::{
    SetWindowTheme, DRAWITEMSTRUCT, EM_GETFIRSTVISIBLELINE, EM_GETSEL, EM_LINEFROMCHAR,
    EM_LINEINDEX, EM_LINESCROLL, EM_REPLACESEL, EM_SCROLLCARET, EM_SETLIMITTEXT, EM_SETSEL,
    ODS_HOTLIGHT,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetFocus, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture, SetFocus,
    VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT,
    VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor, GetClientRect,
    GetDlgCtrlID, GetDlgItem, GetForegroundWindow, GetMessageW, GetParent, GetScrollInfo,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, IsWindowVisible,
    IsZoomed, KillTimer, LoadCursorW, LoadIconW, MessageBoxW, MoveWindow, PostQuitMessage,
    RegisterClassW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW,
    ShowWindow, TrackPopupMenu, TranslateMessage, BN_CLICKED, BS_OWNERDRAW, CW_USEDEFAULT,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, GA_ROOT, HMENU, HWND_TOP, ICON_BIG,
    ICON_SMALL, IDC_ARROW, IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, MINMAXINFO, MSG,
    SB_VERT, SCROLLINFO, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SIF_PAGE, SIF_POS,
    SIF_RANGE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW,
    TPM_NONOTIFY, TPM_RETURNCMD, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CHAR, WM_COMMAND, WM_COPY,
    WM_CREATE, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM,
    WM_ERASEBKGND, WM_GETDLGCODE, WM_GETMINMAXINFO, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MEASUREITEM, WM_MENUCHAR, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE,
    WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SETICON, WM_SETREDRAW,
    WM_SIZE, WM_SYSCHAR, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const ID_OUTPUT: usize = 1001;
const ID_INPUT: usize = 1002;
const ID_APPEND: usize = 1003;
const ID_SESSION: usize = 1004;
const ID_STATUS_CONNECTION: usize = 1005;
const ID_STATUS_ENCODING: usize = 1006;
const ID_SIDEBAR_TOGGLE: usize = 1007;
const ID_SIDEBAR_TERMINALS: usize = 1008;
const ID_MODE_TOGGLE: usize = 1009;
const ID_NEW_TAB: usize = 1010;
const ID_DIRECT_INPUT: usize = 1011;
const ID_MINIMIZE: usize = 1012;
const ID_MAXIMIZE: usize = 1013;
const ID_CLOSE_WINDOW: usize = 1014;
const ID_STATUS_SIZE: usize = 1015;
const ID_SIDEBAR_PROFILES: usize = 1016;
const ID_TERMINAL: usize = 1017;
const ID_VT_OUTPUT: usize = 1018;
const MENU_INPUT_MODE: usize = 60001;
const MENU_HELP: usize = 60002;
const MENU_READING: usize = 60003;
const MENU_TERMINAL_CAPTURE: usize = 60004;
const MENU_READ_VT: usize = 60005;
const MENU_READ_JOURNAL: usize = 60006;
const MENU_SEARCH: usize = 60007;
const MENU_RESTART: usize = 60008;
const MENU_SSH_PROFILES: usize = 60009;
const MENU_SECRET_INPUT: usize = 60010;
const MENU_SPEECH_ALL: usize = 60011;
const MENU_SPEECH_IMPORTANT: usize = 60012;
const MENU_SPEECH_MANUAL: usize = 60013;
const MENU_TUI_DIALOG: usize = 60014;
const MENU_AGENT_PANEL: usize = 60015;
const MENU_NEW_WINDOW: usize = 60016;
const MENU_OPEN_FOLDER: usize = 60017;
const MENU_PREFERENCES: usize = 60018;
const MENU_RESTORE_PROFILE: usize = 60019;
const MENU_READ_INPUT: usize = 60020;
const MENU_READ_NEW_OUTPUT: usize = 60021;
const ID_TAB_BASE: usize = 2000;
const ID_TAB_CLOSE_BASE: usize = 3000;
const OUTPUT_TIMER_ID: usize = 1;
const OUTPUT_POLL_MS: u32 = 50;
const TUI_STABLE_FOR: Duration = Duration::from_millis(90);
const TUI_OPEN_TIMEOUT: Duration = Duration::from_millis(2_750);
const STARTUP_TEXT: &str =
    "Запуск оболочки...\r\nF1 — справка. Ctrl+Shift+Space — терминал / журнал чтения.\r\n";

#[derive(Clone, Copy, Default)]
struct Controls {
    session_header: HWND,
    output: HWND,
    terminal: HWND,
    vt_output: HWND,
    direct_input: HWND,
    search: search_bar::SearchBar,
    input_label: HWND,
    input: HWND,
    append: HWND,
    status_connection: HWND,
    status_encoding: HWND,
    status_size: HWND,
    announcement: announcements::Announcer,
    sidebar_toggle: HWND,
    sidebar_terminals: HWND,
    sidebar_profiles: HWND,
    new_tab: HWND,
    minimize: HWND,
    maximize: HWND,
    close_window: HWND,
}

struct TuiControl {
    session_id: u64,
    input_revision: u64,
    frame: bks_terminal::TerminalSnapshot,
    observed_revision: u64,
    stable_since: Instant,
    pending: Option<Instant>,
    starting: bool,
}

#[derive(Clone)]
struct TabControls {
    session_id: u64,
    accessible_name: String,
    tab: HWND,
    close: HWND,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TuiSourceKey {
    session: u64,
    revision: u64,
    input_revision: u64,
    cli_version: String,
}

impl TuiSourceKey {
    fn from_source(source: tui_dialog::Source<'_>) -> Option<Self> {
        (source.direct_codex_profile
            && source.cli_version == tui_dialog::SUPPORTED_CODEX_VERSION
            && source.process_alive
            && source.active_session)
            .then(|| Self {
                session: source.session_id,
                revision: source.screen_revision,
                input_revision: source.input_revision,
                cli_version: source.cli_version.to_owned(),
            })
    }
}

struct PendingTuiAnalysis {
    id: u64,
    key: TuiSourceKey,
    screen: bks_terminal::TerminalSnapshot,
}

struct CachedTuiAnalysis {
    key: TuiSourceKey,
    dialog: Option<tui_dialog::DialogSnapshot>,
}

struct PendingTuiOpen {
    key: TuiSourceKey,
    owner: HWND,
    deadline: Instant,
}

#[derive(Default)]
struct TuiAdapterState {
    host: Option<tui_adapter_host::Host>,
    ready: bool,
    disabled: bool,
    observed: Option<TuiSourceKey>,
    stable_since: Option<Instant>,
    last_submitted: Option<TuiSourceKey>,
    pending: Option<PendingTuiAnalysis>,
    cached: Option<CachedTuiAnalysis>,
    open: Option<PendingTuiOpen>,
    next_id: u64,
    notice_pending: bool,
}

impl TuiAdapterState {
    fn observe(&mut self, key: Option<&TuiSourceKey>, idle: bool, now: Instant) {
        if self.observed.as_ref() != key {
            self.observed = key.cloned();
            self.stable_since = None;
            self.last_submitted = None;
            self.cached = None;
            self.notice_pending = false;
        }
        if idle && key.is_some() {
            self.stable_since.get_or_insert(now);
        } else {
            self.stable_since = None;
        }
    }

    fn stable(&self, now: Instant) -> bool {
        self.stable_since
            .is_some_and(|since| now.saturating_duration_since(since) >= TUI_STABLE_FOR)
    }

    fn can_submit(&self, key: &TuiSourceKey, now: Instant) -> bool {
        self.ready
            && !self.disabled
            && self.pending.is_none()
            && self.last_submitted.as_ref() != Some(key)
            && self.observed.as_ref() == Some(key)
            && self.stable(now)
    }

    fn disable(&mut self) -> bool {
        if self.disabled {
            return false;
        }
        self.disabled = true;
        self.ready = false;
        self.pending = None;
        self.cached = None;
        self.notice_pending = false;
        if let Some(host) = &self.host {
            host.stop();
        }
        true
    }

    fn explicit_restart(&mut self) {
        let next_id = self.next_id;
        *self = Self {
            next_id,
            ..Self::default()
        };
    }
}

#[derive(Default)]
struct PendingCliLaunch {
    request: Option<(String, Option<PathBuf>)>,
}

impl PendingCliLaunch {
    fn defer(&mut self, id: Option<&str>, directory: Option<PathBuf>) -> bool {
        self.request = id
            .filter(|id| id.starts_with("cli-") || id.starts_with("codex-tui-"))
            .map(|id| (id.to_owned(), directory));
        self.request.is_some()
    }

    fn cancel(&mut self) {
        self.request = None;
    }

    /// A missing catalog entry waits only for discovery; a resolved request is
    /// consumed before the caller can spawn anything or report an error.
    fn resolve(
        &mut self,
        profiles: &[profile::Profile],
        discoveries_finished: bool,
    ) -> Option<std::result::Result<profile::Profile, String>> {
        let (id, _) = self.request.as_ref()?;
        let found = profiles.iter().find(|profile| profile.id == *id).cloned();
        if found.is_none() && !discoveries_finished {
            return None;
        }
        let (id, directory) = self.request.take()?;
        Some(match found {
            Some(mut profile) => {
                if let Some(directory) = directory {
                    profile.working_directory = Some(directory.to_string_lossy().into_owned());
                }
                Ok(profile)
            }
            None => Err(format!(
                "Профиль запуска «{id}» не обнаружен. Выберите другой профиль кнопкой новой вкладки."
            )),
        })
    }
}

thread_local! {
    static CONTROLS: RefCell<Controls> = RefCell::new(Controls::default());
    static TAB_CONTROLS: RefCell<Vec<TabControls>> = const { RefCell::new(Vec::new()) };
    static TAB_SCROLL: Cell<usize> = const { Cell::new(0) };
    static THEME: RefCell<Theme> = RefCell::new(Theme::default());
    static DPI: Cell<u32> = const { Cell::new(dpi::BASE_DPI) };
    static SIDEBAR_EXPANDED: Cell<bool> = const { Cell::new(true) };
    static UTF16_ENCODER: RefCell<Utf16Encoder> = RefCell::new(Utf16Encoder::new());
    static CONNECTION_STATUS: RefCell<Option<bool>> = const { RefCell::new(None) };
    static PROFILES: RefCell<ProfileManager> = RefCell::new(ProfileManager::discover());
    static SESSIONS: RefCell<SessionManager> = RefCell::new(SessionManager::new());
    static SELECTION_ANCHOR: Cell<usize> = const { Cell::new(0) };
    static WHEEL_REMAINDER: Cell<i32> = const { Cell::new(0) };
    static VT_DISPLAY_KEY: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
    static VT_DISPLAY_DOCUMENT: RefCell<Option<Arc<bks_terminal::ReadingDocument>>> = const { RefCell::new(None) };
    static VT_RESTORE_PENDING: Cell<bool> = const { Cell::new(false) };
    static DIRECT_DOCUMENT: RefCell<Option<(u64, Arc<bks_terminal::ReadingDocument>)>> = const { RefCell::new(None) };
    static DIRECT_FEEDBACK: RefCell<direct_accessibility::InputFeedback> = RefCell::new(direct_accessibility::InputFeedback::default());
    static DIRECT_ECHO: RefCell<tui_input_echo::InputEcho> = RefCell::new(tui_input_echo::InputEcho::default());
    static SEARCH_UI_SYNC: Cell<bool> = const { Cell::new(false) };
    static SEARCH_FEEDBACK: RefCell<feedback::SearchFeedback> = RefCell::new(feedback::SearchFeedback::default());
    static SETTINGS: RefCell<settings::Settings> = RefCell::new(settings::Settings::default());
    static SPEECH_PAUSE_UNTIL: Cell<Option<Instant>> = const { Cell::new(None) };
    static CODEX_CLI: RefCell<Option<codex_cli::SupportedCli>> = const { RefCell::new(None) };
    static CLI_DISCOVERY: RefCell<Option<std::sync::mpsc::Receiver<Option<codex_cli::SupportedCli>>>> = const { RefCell::new(None) };
    static TOOL_DISCOVERY: RefCell<Option<std::sync::mpsc::Receiver<cli_profiles::Discovery>>> = const { RefCell::new(None) };
    static CLI_TOOLS: RefCell<cli_profiles::Discovery> = RefCell::new(cli_profiles::Discovery::default());
    static TUI_BINDING: RefCell<Option<tui_dialog::DialogSnapshot>> = const { RefCell::new(None) };
    static TUI_READING_SOURCE: Cell<Option<(u64, u64, u64)>> = const { Cell::new(None) };
    static TUI_CONTROL: RefCell<Option<TuiControl>> = const { RefCell::new(None) };
    static TUI_ADAPTER: RefCell<TuiAdapterState> = RefCell::new(TuiAdapterState::default());
    static RECENT_PROFILES: RefCell<workspace_state::WorkspaceState> = RefCell::new(workspace_state::WorkspaceState::default());
    static LAUNCH: RefCell<Option<launch::LaunchOptions>> = const { RefCell::new(None) };
    static PENDING_CLI_LAUNCH: RefCell<PendingCliLaunch> = RefCell::new(PendingCliLaunch::default());
    static PREFS_BASELINE: RefCell<Option<settings::Settings>> = const { RefCell::new(None) };
}

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new(tui_worker::WORKER_ARGUMENT))
        && arguments.next().is_none()
    {
        // Worker failures must never initialize COM, create windows, or display
        // the normal application error box in this private subprocess.
        std::process::exit(if tui_worker::run_stdio().is_ok() {
            0
        } else {
            1
        });
    }
    if let Err(error) = run() {
        show_error("Не удалось запустить BKS Term", &error);
    }
}

fn run() -> Result<()> {
    let options = launch::LaunchOptions::parse(std::env::args_os().skip(1))
        .map_err(|message| Error::new(windows::Win32::Foundation::E_INVALIDARG, message))?;
    if options.print_help {
        println!("BKS Term {}\n--profile ID  --cwd DIRECTORY  --choose-profile\n--register-explorer  --unregister-explorer\nСправка интерфейса: F1; документация: docs/help.md", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(enabled) = options.explorer {
        desktop::set_explorer_integration(enabled)
            .map_err(|error| Error::new(windows::Win32::Foundation::E_FAIL, error.to_string()))?;
        unsafe {
            MessageBoxW(
                None,
                w!("Настройка контекстного меню Проводника сохранена для текущего пользователя."),
                w!("BKS Term"),
                MB_OK,
            );
        }
        return Ok(());
    }
    LAUNCH.with(|current| *current.borrow_mut() = Some(options));
    let _com = ComApartment::initialize()?;
    SETTINGS.with(|settings| *settings.borrow_mut() = settings::Settings::load(&settings_path()));
    RECENT_PROFILES.with(|recent| {
        *recent.borrow_mut() =
            workspace_state::WorkspaceState::load(&workspace_path()).unwrap_or_default()
    });
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let initial_dpi = GetDpiForSystem();
        let theme = Theme::new(initial_dpi);
        DPI.set(initial_dpi);
        THEME.with(|current| *current.borrow_mut() = theme);
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let icon = LoadIconW(Some(instance), w!("BKS_TERM"))?;
        let window_class = WNDCLASSW {
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance,
            hbrBackground: theme.window,
            lpszClassName: w!("BksTermWindow"),
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(Error::from_thread());
        }
        let terminal_class = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance,
            lpszClassName: w!("BksTerminalSurface"),
            lpfnWndProc: Some(terminal_window_proc),
            ..Default::default()
        };
        if RegisterClassW(&terminal_class) == 0 {
            return Err(Error::from_thread());
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BksTermWindow"),
            w!("BKS Term"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            dpi::scale(1200, initial_dpi),
            dpi::scale(800, initial_dpi),
            None,
            None,
            Some(instance),
            None,
        )?;
        for icon_type in [ICON_BIG, ICON_SMALL] {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(icon_type as usize)),
                Some(LPARAM(icon.0 as isize)),
            );
        }
        apply_dark_title(hwnd);
        refresh_layout(hwnd);
        CLI_DISCOVERY
            .with(|discovery| *discovery.borrow_mut() = Some(codex_cli::start_discovery()));
        TOOL_DISCOVERY
            .with(|discovery| *discovery.borrow_mut() = Some(cli_profiles::start_discovery()));
        start_terminal(hwnd);
        refresh_layout(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetFocus(Some(input_focus_target()));
        let directory_choice = LAUNCH.with(|launch| {
            launch.borrow().as_ref().and_then(|options| {
                (options.choose_profile
                    || (options.working_directory.is_some() && options.profile_id.is_none()))
                .then(|| options.working_directory.clone())
            })
        });
        if let Some(directory) = directory_choice {
            show_profile_menu_intent(
                hwnd,
                controls().new_tab,
                false,
                ProfileIntent::NewTab(directory),
            );
        }

        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 == -1 {
                return Err(Error::from_thread());
            }
            if result.0 == 0 {
                break;
            }
            if message.hwnd == controls().direct_input
                && matches!(
                    message.message,
                    WM_KEYDOWN | WM_SYSKEYDOWN | WM_CHAR | WM_SYSCHAR
                )
            {
                // Drain a returned shell prompt before routing a new input
                // event. Never turn an Enter intended for the old CLI into
                // submission of a preserved native command draft.
                if SESSIONS.with(|sessions| {
                    sessions
                        .borrow()
                        .active()
                        .is_some_and(|active| active.auto_cli_input)
                }) {
                    poll_terminal();
                }
                if !direct_input_active() {
                    if GetFocus() != controls().input {
                        continue;
                    }
                    if (matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                        && message.wParam.0 == VK_RETURN.0 as usize)
                        || (matches!(message.message, WM_CHAR | WM_SYSCHAR)
                            && message.wParam.0 == 13)
                    {
                        continue;
                    }
                    message.hwnd = controls().input;
                }
            }
            if preferences::handle_message(&message) {
                finish_preferences();
                continue;
            }
            finish_preferences();
            if agent_window::handle_message(&message) {
                finish_agent_window(hwnd);
                continue;
            }
            finish_agent_window(hwnd);
            // Dialogs consume their keys/chars before terminal shortcuts.
            if tui_window::handle_message(&message) {
                finish_tui_dialog();
                continue;
            }
            finish_tui_dialog();
            if secret_input::handle_message(&message) {
                continue;
            }
            if profile_editor::handle_message(&message) {
                finish_profile_edit();
                continue;
            }
            finish_profile_edit();
            if help::handle_message(&message) {
                continue;
            }
            if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                && handle_app_shortcut(hwnd, message.wParam.0)
            {
                continue;
            }
            if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                && handle_search_key(hwnd, message.wParam.0)
            {
                continue;
            }
            if message.message == WM_KEYDOWN
                && direct_input_has_focus()
                && keyboard_navigation::starts_text_selection(
                    message.wParam.0,
                    GetKeyState(VK_SHIFT.0 as i32) < 0,
                    GetKeyState(VK_MENU.0 as i32) < 0,
                )
            {
                if begin_input_selection(hwnd) {
                    // Let the native read-only EDIT extend selection using the
                    // actual Shift/Ctrl state; do not encode this as a CLI key.
                    message.hwnd = controls().vt_output;
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                    capture_vt_reading_position();
                }
                continue;
            }
            if message.message == WM_KEYDOWN
                && message.wParam.0 == VK_ESCAPE.0 as usize
                && GetKeyState(VK_CONTROL.0 as i32) >= 0
                && GetKeyState(VK_MENU.0 as i32) >= 0
                && direct_input_active()
                && reading_active()
                && GetFocus() == reading_control()
            {
                toggle_reading_focus(hwnd);
                continue;
            }
            if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                && message.wParam.0 == VK_TAB.0 as usize
            {
                use keyboard_navigation::TabAction;
                let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let alt = GetKeyState(VK_MENU.0 as i32) < 0;
                let tab_action = if control || alt {
                    TabAction::System
                } else {
                    keyboard_navigation::tab_action(
                        control,
                        shift,
                        alt,
                        terminal_capture_active() && direct_input_has_focus(),
                    )
                };
                match tab_action {
                    TabAction::MoveFocus => move_tab_focus(shift),
                    TabAction::SwitchSession { previous } => {
                        activate_adjacent_session(hwnd, previous)
                    }
                    TabAction::SendToTerminal => {
                        begin_direct_feedback();
                        write_active_input(&encode_key(
                            SpecialKey::Tab,
                            shift,
                            false,
                            false,
                            false,
                        ));
                    }
                    TabAction::System => {
                        DispatchMessageW(&message);
                    }
                }
                continue;
            }
            if message.message == WM_KEYDOWN
                && message.wParam.0 == VK_RETURN.0 as usize
                && message.hwnd == controls().input
                && GetFocus() == controls().input
            {
                if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    insert_input_newline();
                } else {
                    execute_input();
                }
                continue;
            }
            if direct_input_has_focus() {
                if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                    && handle_direct_key(message.wParam.0, message.lParam.0)
                {
                    continue;
                }
                if matches!(message.message, WM_CHAR | WM_SYSCHAR) {
                    handle_direct_char(message.wParam.0 as u16);
                    continue;
                }
                // Terminal Tab/Escape and Alt keys must never enter dialog navigation.
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
                continue;
            }
            if IsDialogMessageW(hwnd, &message).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // Window destruction never waits for a ConPTY or its worker threads.
    SESSIONS.with(|sessions| sessions.borrow_mut().finish_cleanup());
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => window_frame::calculate_client(hwnd, lparam),
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCHITTEST => window_frame::hit_test(hwnd, lparam, current_dpi()),
        WM_CREATE => match create_controls(hwnd) {
            Ok(()) => {
                let window_dpi = GetDpiForWindow(hwnd);
                if window_dpi != current_dpi() {
                    update_dpi(window_dpi);
                }
                LRESULT(0)
            }
            Err(_) => LRESULT(-1),
        },
        WM_PAINT => {
            paint_shell(hwnd);
            LRESULT(0)
        }
        WM_SETFOCUS => {
            let input = input_focus_target();
            if !input.0.is_null() {
                let _ = SetFocus(Some(input));
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (lparam.0 as u32 & 0xffff) as i32;
            let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
            layout_controls(width, height);
            rebuild_tab_controls(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, true);
            let _ = InvalidateRect(Some(controls().maximize), None, true);
            let maximize_name = if IsZoomed(hwnd).as_bool() {
                "Восстановить окно"
            } else {
                "Развернуть окно"
            };
            let _ = set_accessible_name(controls().maximize, maximize_name);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xffff) as u32;
            update_dpi(new_dpi);
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                let (min_width, min_height) = dpi::min_track_size(current_dpi());
                info.ptMinTrackSize = POINT {
                    x: min_width,
                    y: min_height,
                };
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let current_theme = theme();
            if [
                controls().output,
                controls().vt_output,
                controls().direct_input,
            ]
            .contains(&HWND(lparam.0 as *mut c_void))
            {
                control_color(
                    wparam,
                    current_theme.colors.text_primary,
                    current_theme.colors.terminal,
                    current_theme.terminal,
                )
            } else {
                control_color(
                    wparam,
                    current_theme.colors.text_primary,
                    current_theme.colors.surface,
                    current_theme.surface,
                )
            }
        }
        WM_CTLCOLORSTATIC => {
            let control = HWND(lparam.0 as *mut c_void);
            let current_theme = theme();
            if [
                controls().output,
                controls().vt_output,
                controls().direct_input,
            ]
            .contains(&control)
            {
                control_color(
                    wparam,
                    current_theme.colors.text_primary,
                    current_theme.colors.terminal,
                    current_theme.terminal,
                )
            } else if control == controls().status_connection {
                let color = match CONNECTION_STATUS.with(|status| *status.borrow()) {
                    Some(true) => current_theme.colors.status_success,
                    Some(false) => current_theme.colors.status_error,
                    None => current_theme.colors.text_muted,
                };
                control_color(
                    wparam,
                    color,
                    current_theme.colors.toolbar,
                    current_theme.toolbar,
                )
            } else if [
                controls().status_encoding,
                controls().status_size,
                controls().announcement.hwnd,
            ]
            .contains(&control)
            {
                control_color(
                    wparam,
                    current_theme.colors.text_secondary,
                    current_theme.colors.surface,
                    current_theme.surface,
                )
            } else {
                control_color(
                    wparam,
                    current_theme.colors.text_secondary,
                    current_theme.colors.window,
                    current_theme.window,
                )
            }
        }
        WM_MEASUREITEM => {
            if profile_menu::measure(lparam, current_dpi()) {
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_MENUCHAR => profile_menu::menu_char(wparam, lparam)
            .unwrap_or_else(|| DefWindowProcW(hwnd, message, wparam, lparam)),
        WM_DRAWITEM => {
            let mut draw_item = *(lparam.0 as *const DRAWITEMSTRUCT);
            if profile_menu::draw(&draw_item, &theme(), current_dpi()) {
                return LRESULT(1);
            }
            if control_state::is_hot(draw_item.hwndItem) {
                draw_item.itemState.0 |= ODS_HOTLIGHT.0;
            }
            let item = &draw_item;
            let expanded = sidebar_expanded_for_width(client_width(hwnd));
            if wparam.0 == ID_STATUS_CONNECTION {
                draw_connection_status(
                    item,
                    &theme(),
                    current_dpi(),
                    CONNECTION_STATUS.with(|status| *status.borrow()),
                );
                LRESULT(1)
            } else if wparam.0 == ID_APPEND {
                draw_execute_button(item, &theme());
                LRESULT(1)
            } else if wparam.0 == ID_SESSION {
                draw_tab_background(item, &theme(), current_dpi());
                LRESULT(1)
            } else if wparam.0 == ID_NEW_TAB {
                draw_mode_button(item, &theme(), "+");
                LRESULT(1)
            } else if [
                search_bar::ID_PREVIOUS,
                search_bar::ID_NEXT,
                search_bar::ID_CLOSE,
            ]
            .contains(&wparam.0)
            {
                draw_mode_button(item, &theme(), &window_text(item.hwndItem));
                LRESULT(1)
            } else if (ID_TAB_BASE..ID_TAB_CLOSE_BASE).contains(&wparam.0) {
                let id = (wparam.0 - ID_TAB_BASE) as u64;
                draw_tab_button(
                    item,
                    &theme(),
                    current_dpi(),
                    active_session_id() == Some(id),
                );
                LRESULT(1)
            } else if wparam.0 >= ID_TAB_CLOSE_BASE {
                draw_tab_close(
                    item,
                    &theme(),
                    current_dpi(),
                    active_session_id() == Some((wparam.0 - ID_TAB_CLOSE_BASE) as u64),
                );
                LRESULT(1)
            } else if wparam.0 == ID_SIDEBAR_TOGGLE {
                draw_sidebar_button(
                    item,
                    &theme(),
                    current_dpi(),
                    SidebarButton::Toggle,
                    expanded,
                );
                LRESULT(1)
            } else if wparam.0 == ID_SIDEBAR_TERMINALS {
                draw_sidebar_button(
                    item,
                    &theme(),
                    current_dpi(),
                    SidebarButton::Terminals,
                    expanded,
                );
                LRESULT(1)
            } else if wparam.0 == ID_SIDEBAR_PROFILES {
                draw_sidebar_button(
                    item,
                    &theme(),
                    current_dpi(),
                    SidebarButton::Profiles,
                    expanded,
                );
                LRESULT(1)
            } else if [ID_MINIMIZE, ID_MAXIMIZE, ID_CLOSE_WINDOW].contains(&wparam.0) {
                let kind = match wparam.0 {
                    ID_MINIMIZE => WindowButton::Minimize,
                    ID_MAXIMIZE if IsZoomed(hwnd).as_bool() => WindowButton::Restore,
                    ID_MAXIMIZE => WindowButton::Maximize,
                    _ => WindowButton::Close,
                };
                draw_window_button(item, &theme(), current_dpi(), kind);
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_COMMAND => {
            let control_id = wparam.0 & 0xffff;
            let notification = (wparam.0 >> 16) & 0xffff;
            if control_id == search_bar::ID_QUERY && notification == 0x300 {
                if !SEARCH_UI_SYNC.get() && search_active() {
                    change_search_query(hwnd);
                }
                return LRESULT(0);
            }
            if notification == 0x100
                && [
                    ID_INPUT,
                    ID_OUTPUT,
                    ID_VT_OUTPUT,
                    ID_DIRECT_INPUT,
                    search_bar::ID_QUERY,
                ]
                .contains(&control_id)
            {
                // EN_SETFOCUS: preserve whether the user was typing or reading.
                SESSIONS.with(|sessions| {
                    if let Some(active) = sessions.borrow_mut().active_mut() {
                        active.search_focus = control_id == search_bar::ID_QUERY;
                        if !active.search_focus {
                            active.focus_in_reader =
                                ![ID_INPUT, ID_DIRECT_INPUT].contains(&control_id);
                        }
                        if control_id != ID_DIRECT_INPUT {
                            active.terminal_capture = false;
                        } else {
                            // Returning to the terminal is enough to resume
                            // normal TUI input, including Tab completion.
                            active.terminal_capture = true;
                        }
                    }
                });
                if control_id == ID_VT_OUTPUT {
                    update_vt_output(false);
                    if search_active() {
                        reveal_search_result(hwnd, false);
                    }
                } else if control_id == search_bar::ID_QUERY {
                    schedule_search_feedback();
                } else if control_id == ID_DIRECT_INPUT {
                    update_direct_output(true);
                }
            }
            if control_id == ID_DIRECT_INPUT && notification == 0x200 {
                release_terminal_capture();
            }
            if notification != BN_CLICKED as usize {
                return DefWindowProcW(hwnd, message, wparam, lparam);
            }
            match control_id {
                ID_MINIMIZE | ID_MAXIMIZE | ID_CLOSE_WINDOW => {
                    let command = match control_id {
                        ID_MINIMIZE => SC_MINIMIZE,
                        ID_MAXIMIZE if IsZoomed(hwnd).as_bool() => SC_RESTORE,
                        ID_MAXIMIZE => SC_MAXIMIZE,
                        _ => SC_CLOSE,
                    };
                    SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM(command as usize)), None);
                    LRESULT(0)
                }
                ID_APPEND => {
                    execute_input();
                    LRESULT(0)
                }
                ID_SIDEBAR_TOGGLE => {
                    toggle_sidebar(hwnd);
                    LRESULT(0)
                }
                ID_SIDEBAR_TERMINALS => {
                    let _ = SetForegroundWindow(hwnd);
                    let _ = SetFocus(Some(input_focus_target()));
                    LRESULT(0)
                }
                ID_SIDEBAR_PROFILES => {
                    show_profile_menu_at(hwnd, controls().sidebar_profiles, true);
                    LRESULT(0)
                }
                ID_MODE_TOGGLE => {
                    toggle_input_mode(hwnd);
                    LRESULT(0)
                }
                MENU_HELP => {
                    release_terminal_capture();
                    if let Err(error) = help::show(hwnd) {
                        show_error("Не удалось открыть справку", &error);
                    }
                    LRESULT(0)
                }
                MENU_READING => {
                    toggle_reading_focus(hwnd);
                    LRESULT(0)
                }
                MENU_TERMINAL_CAPTURE => {
                    toggle_terminal_capture(hwnd);
                    LRESULT(0)
                }
                MENU_READ_VT | MENU_READ_JOURNAL => {
                    select_reading_source(hwnd, control_id == MENU_READ_VT);
                    LRESULT(0)
                }
                MENU_SEARCH => {
                    open_search(hwnd);
                    LRESULT(0)
                }
                MENU_RESTART => {
                    restart_session(hwnd);
                    LRESULT(0)
                }
                MENU_SSH_PROFILES => {
                    show_ssh_profiles(hwnd);
                    LRESULT(0)
                }
                MENU_SECRET_INPUT => {
                    show_secret_input(hwnd);
                    LRESULT(0)
                }
                MENU_SPEECH_ALL | MENU_SPEECH_IMPORTANT | MENU_SPEECH_MANUAL => {
                    select_speech_mode(control_id);
                    LRESULT(0)
                }
                MENU_TUI_DIALOG => {
                    open_tui_dialog(hwnd);
                    LRESULT(0)
                }
                MENU_AGENT_PANEL => {
                    open_agent_window(hwnd);
                    LRESULT(0)
                }
                MENU_NEW_WINDOW => {
                    show_profile_menu_intent(
                        hwnd,
                        controls().new_tab,
                        false,
                        ProfileIntent::NewWindow,
                    );
                    LRESULT(0)
                }
                MENU_OPEN_FOLDER => {
                    open_folder_session(hwnd);
                    LRESULT(0)
                }
                MENU_PREFERENCES => {
                    open_preferences(hwnd);
                    LRESULT(0)
                }
                MENU_RESTORE_PROFILE => {
                    show_recent_profiles(hwnd);
                    LRESULT(0)
                }
                search_bar::ID_PREVIOUS | search_bar::ID_NEXT => {
                    navigate_search(hwnd, control_id == search_bar::ID_PREVIOUS);
                    LRESULT(0)
                }
                search_bar::ID_CLOSE => {
                    close_search(hwnd, true);
                    LRESULT(0)
                }
                ID_NEW_TAB => {
                    show_profile_menu_at(hwnd, controls().new_tab, false);
                    LRESULT(0)
                }
                id if (ID_TAB_BASE..ID_TAB_CLOSE_BASE).contains(&id) => {
                    activate_session(hwnd, (id - ID_TAB_BASE) as u64);
                    LRESULT(0)
                }
                id if id >= ID_TAB_CLOSE_BASE => {
                    close_session(hwnd, (id - ID_TAB_CLOSE_BASE) as u64);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        WM_TIMER => {
            if wparam.0 == OUTPUT_TIMER_ID {
                poll_terminal();
                agent_window::poll();
                update_connection_status();
                poll_search_feedback();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), OUTPUT_TIMER_ID);
            TUI_ADAPTER.with(|adapter| *adapter.borrow_mut() = TuiAdapterState::default());
            tui_window::close();
            agent_window::close();
            SESSIONS.with(|sessions| {
                sessions.borrow_mut().close_all();
            });
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe extern "system" fn terminal_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            let frame = SESSIONS.with(|sessions| {
                let mut sessions = sessions.borrow_mut();
                let active = sessions.active_mut()?;
                let frame = active.terminal.snapshot();
                let search = if active.search_open {
                    let document = active.reading_document_snapshot();
                    (active.search.document_revision() == Some(document.revision)).then(|| {
                        let matches = terminal_view::visible_matches(
                            &document,
                            &frame,
                            active.search.matches(),
                        )
                        .to_vec();
                        (document, matches, active.search.current())
                    })
                } else {
                    None
                };
                Some((frame, active.screen_selection, search))
            });
            if let Some((frame, selection, search)) = frame {
                if let Some((document, matches, current)) = search {
                    let overlay = terminal_view::SearchOverlay {
                        document: &document,
                        matches: &matches,
                        current,
                    };
                    terminal_view::paint_with_search(
                        hwnd,
                        hdc,
                        &frame,
                        &theme(),
                        selection,
                        GetFocus() == hwnd || direct_input_has_focus(),
                        Some(&overlay),
                    );
                } else {
                    terminal_view::paint(
                        hwnd,
                        hdc,
                        &frame,
                        &theme(),
                        selection,
                        GetFocus() == hwnd || direct_input_has_focus(),
                    );
                }
            } else {
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                FillRect(hdc, &rect, theme().terminal);
            }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_GETDLGCODE => LRESULT(if terminal_capture_active() {
            0x0085
        } else {
            0x0081
        }),
        WM_SETFOCUS | WM_KILLFOCUS => {
            if message == WM_SETFOCUS && direct_input_active() && !reading_active() {
                // The visual canvas is not a second text/input target.
                let _ = SetFocus(Some(controls().direct_input));
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            if message == WM_KILLFOCUS && HWND(wparam.0 as *mut c_void) != controls().direct_input {
                release_terminal_capture();
            } else if message == WM_SETFOCUS {
                SESSIONS.with(|sessions| {
                    if let Some(active) = sessions.borrow_mut().active_mut() {
                        active.search_focus = false;
                        active.focus_in_reader = false;
                    }
                });
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_MOUSEMOVE if message == WM_LBUTTONDOWN || GetCapture() == hwnd => {
            let x = lparam.0 as u16 as i16 as i32;
            let y = (lparam.0 >> 16) as u16 as i16 as i32;
            if message == WM_LBUTTONDOWN {
                let _ = SetFocus(Some(if direct_input_active() && !reading_active() {
                    controls().direct_input
                } else {
                    hwnd
                }));
                SetCapture(hwnd);
            }
            SESSIONS.with(|sessions| {
                if let Some(active) = sessions.borrow_mut().active_mut() {
                    let (cols, rows) = active.terminal.size();
                    let index =
                        terminal_view::hit_test(hwnd, theme().terminal_font, x, y, cols, rows);
                    if message == WM_LBUTTONDOWN {
                        SELECTION_ANCHOR.set(index);
                    }
                    let anchor = SELECTION_ANCHOR.get();
                    active.screen_selection = Some((anchor.min(index), anchor.max(index) + 1));
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if GetCapture() == hwnd {
                let _ = ReleaseCapture();
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16 as i32 + WHEEL_REMAINDER.get();
            WHEEL_REMAINDER.set(delta % 120);
            scroll_terminal(delta / 120 * 3);
            LRESULT(0)
        }
        WM_COPY => {
            copy_terminal_selection();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Keep native text/caret/focus under the opaque VT canvas in the same area.
/// EDIT may scroll its own pixels outside WM_PAINT; sibling clipping prevents
/// those operations from touching the independently rendered terminal surface.
unsafe extern "system" fn direct_surface_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        WM_SETREDRAW => {
            // The provider never paints. Keep WS_VISIBLE and its accessibility
            // object stable; only the separate canvas owns visible pixels.
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut paint);
            let _ = EndPaint(hwnd, &paint);
            system_caret::hide_visible(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_GETDLGCODE | WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP | WM_MOUSEWHEEL | WM_COPY => {
            terminal_window_proc(hwnd, message, wparam, lparam)
        }
        WM_SETFOCUS | WM_SIZE | WM_SETFONT | EM_SETSEL | EM_REPLACESEL | EM_SCROLLCARET => {
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            // Native scrolling/caret updates remain available to the standard
            // provider, while the canvas above it keeps its own VT coordinates.
            system_caret::hide_visible(hwnd);
            result
        }
        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(hwnd, Some(direct_surface_proc), ID_DIRECT_INPUT);
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, message, wparam, lparam),
    }
}

fn invalidate_terminal() {
    unsafe {
        let _ = InvalidateRect(Some(controls().terminal), None, false);
    }
}

fn scroll_terminal(lines: i32) {
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.terminal.scroll_display(lines);
            active.screen_selection = None;
        }
    });
    invalidate_terminal();
}

fn copy_terminal_selection() -> bool {
    let text = SESSIONS.with(|sessions| {
        sessions.borrow().active().map(|active| {
            terminal_view::selected_text(&active.terminal.snapshot(), active.screen_selection)
        })
    });
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        if let Err(error) = clipboard::write(controls().terminal, &text) {
            show_error("Не удалось скопировать выделение", &error);
        }
        true
    } else {
        false
    }
}

fn paste_terminal() {
    match clipboard::read(controls().terminal) {
        Ok(text) => {
            let bracketed = SESSIONS.with(|sessions| {
                sessions
                    .borrow()
                    .active()
                    .is_some_and(|active| active.terminal.modes().bracketed_paste)
            });
            write_active_input(&paste_bytes(&text, bracketed));
        }
        Err(error) => show_error("Не удалось вставить текст из буфера обмена", &error),
    }
}

fn create_controls(parent: HWND) -> Result<()> {
    let module = unsafe { GetModuleHandleW(None)? };
    let instance = HINSTANCE(module.0);
    let new_controls = Controls {
        session_header: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("PowerShell"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(SS_OWNERDRAW.0),
            parent,
            Some(ID_SESSION),
            instance,
        )?,
        output: {
            // Native UIA EDIT proxies derive Name from the preceding STATIC.
            // Keep this label hidden so the reading area keeps its full height.
            let _label = create_control(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Журнал терминала"),
                WS_CHILD,
                parent,
                None,
                instance,
            )?;
            create_control(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!("Запуск PowerShell...\r\nAlt+Up/Down — результаты команд, Ctrl+Shift+C — копировать результат.\r\n"),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
            parent,
            Some(ID_OUTPUT),
            instance,
        )?
        },
        terminal: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BksTerminalSurface"),
            w!("Терминал; Ctrl+Shift+Space — журнал чтения; F1 — справка"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            parent,
            Some(ID_TERMINAL),
            instance,
        )?,
        vt_output: {
            let _label = create_control(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Экран терминала"),
                WS_CHILD,
                parent,
                None,
                instance,
            )?;
            create_control(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD
                    | WS_TABSTOP
                    | WS_VSCROLL
                    | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
                parent,
                Some(ID_VT_OUTPUT),
                instance,
            )?
        },
        direct_input: {
            let _label = create_control(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Терминал — прямой ввод"),
                WS_CHILD,
                parent,
                None,
                instance,
            )?;
            create_control(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD
                    | WS_CLIPSIBLINGS
                    | WS_TABSTOP
                    | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY) as u32),
                parent,
                Some(ID_DIRECT_INPUT),
                instance,
            )?
        },
        search: search_bar::create(parent, instance)?,
        input_label: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Команда (Enter — выполнить, Shift+Enter — новая строка)"),
            WS_CHILD | WS_VISIBLE,
            parent,
            None,
            instance,
        )?,
        input: create_control(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
            parent,
            Some(ID_INPUT),
            instance,
        )?,
        append: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Выполнить"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_APPEND),
            instance,
        )?,
        status_connection: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("PowerShell · Запуск"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_OWNERDRAW.0),
            parent,
            Some(ID_STATUS_CONNECTION),
            instance,
        )?,
        status_encoding: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("UTF-8"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_CENTERIMAGE.0),
            parent,
            Some(ID_STATUS_ENCODING),
            instance,
        )?,
        status_size: create_control(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_CENTERIMAGE.0),
            parent,
            Some(ID_STATUS_SIZE),
            instance,
        )?,
        announcement: announcements::create(parent, instance)?,
        sidebar_toggle: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Свернуть панель"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_SIDEBAR_TOGGLE),
            instance,
        )?,
        sidebar_terminals: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Терминалы"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_SIDEBAR_TERMINALS),
            instance,
        )?,
        sidebar_profiles: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Профили"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_SIDEBAR_PROFILES),
            instance,
        )?,
        new_tab: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("+"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_NEW_TAB),
            instance,
        )?,
        minimize: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Свернуть окно"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_MINIMIZE),
            instance,
        )?,
        maximize: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Развернуть окно"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_MAXIMIZE),
            instance,
        )?,
        close_window: create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Закрыть окно"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
            parent,
            Some(ID_CLOSE_WINDOW),
            instance,
        )?,
    };
    CONTROLS.with(|controls| {
        *controls.borrow_mut() = new_controls;
    });
    set_accessible_name(new_controls.output, "Журнал терминала")?;
    set_accessible_name(new_controls.vt_output, "Экран терминала")?;
    set_accessible_name(new_controls.direct_input, "Терминал — прямой ввод")?;
    unsafe {
        if !SetWindowSubclass(
            new_controls.direct_input,
            Some(direct_surface_proc),
            ID_DIRECT_INPUT,
            0,
        )
        .as_bool()
        {
            return Err(Error::from_thread());
        }
    }
    set_accessible_name(
        new_controls.terminal,
        "Терминал, прямой ввод. Ctrl+Shift+Space — журнал чтения, F1 — справка",
    )?;
    set_accessible_name(new_controls.input, "Команда")?;
    set_accessible_name(new_controls.session_header, "Вкладки терминала")?;
    set_accessible_name(new_controls.sidebar_toggle, "Свернуть боковую панель")?;
    set_accessible_name(new_controls.sidebar_terminals, "Терминалы, выбрано")?;
    set_accessible_name(
        new_controls.sidebar_profiles,
        "Профили оболочек: выбрать и открыть вкладку",
    )?;
    set_accessible_name(new_controls.new_tab, "Новая вкладка: выбрать профиль")?;
    set_accessible_name(new_controls.minimize, "Свернуть окно")?;
    set_accessible_name(new_controls.maximize, "Развернуть окно")?;
    set_accessible_name(new_controls.close_window, "Закрыть окно")?;
    set_accessible_name(new_controls.status_size, "Размер терминала")?;
    unsafe {
        SendMessageW(
            new_controls.output,
            EM_SETLIMITTEXT,
            Some(WPARAM(0x7FFF_FFFE)),
            None,
        );
        SendMessageW(
            new_controls.vt_output,
            EM_SETLIMITTEXT,
            Some(WPARAM(0x7FFF_FFFE)),
            None,
        );
        SendMessageW(
            new_controls.direct_input,
            EM_SETLIMITTEXT,
            Some(WPARAM(0x7FFF_FFFE)),
            None,
        );
        apply_fonts();
        let _ = SetWindowTheme(new_controls.output, w!("DarkMode_Explorer"), PCWSTR::null());
        let _ = SetWindowTheme(
            new_controls.vt_output,
            w!("DarkMode_Explorer"),
            PCWSTR::null(),
        );
        let _ = SetWindowTheme(new_controls.input, w!("DarkMode_Explorer"), PCWSTR::null());
        let _ = SetWindowTheme(
            new_controls.direct_input,
            w!("DarkMode_Explorer"),
            PCWSTR::null(),
        );
    }
    Ok(())
}

fn create_control(
    ex_style: WINDOW_EX_STYLE,
    class_name: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    parent: HWND,
    id: Option<usize>,
    instance: HINSTANCE,
) -> Result<HWND> {
    unsafe {
        let control = CreateWindowExW(
            ex_style,
            class_name,
            text,
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            id.map(|id| HMENU(id as *mut c_void)),
            Some(instance),
            None,
        )?;
        if style.0 & 0xf == BS_OWNERDRAW as u32 {
            control_state::install(control);
        }
        Ok(control)
    }
}

fn apply_dark_title(hwnd: HWND) {
    if theme().high_contrast {
        return;
    }
    let dark = 1_u32;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_ROUND as *const _ as *const c_void,
            std::mem::size_of_val(&DWMWCP_ROUND) as u32,
        );
        let border = theme().colors.selected;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const _ as *const c_void,
            std::mem::size_of_val(&border) as u32,
        );
    }
}

fn control_color(wparam: WPARAM, text: COLORREF, background: COLORREF, brush: HBRUSH) -> LRESULT {
    unsafe {
        let hdc = HDC(wparam.0 as *mut c_void);
        SetTextColor(hdc, text);
        SetBkColor(hdc, background);
    }
    LRESULT(brush.0 as isize)
}

fn current_dpi() -> u32 {
    DPI.with(Cell::get)
}

fn update_dpi(new_dpi: u32) {
    DPI.set(new_dpi);
    THEME.with(|theme| theme.borrow_mut().update_fonts(new_dpi));
    apply_fonts();
    resize_active_terminal();
}

fn apply_fonts() {
    let theme = theme();
    controls().search.apply_font(theme.ui_font);
    unsafe {
        for control in [
            controls().session_header,
            controls().input_label,
            controls().append,
            controls().status_connection,
            controls().status_encoding,
            controls().status_size,
            controls().announcement.hwnd,
            controls().sidebar_toggle,
            controls().sidebar_terminals,
            controls().sidebar_profiles,
            controls().new_tab,
            controls().minimize,
            controls().maximize,
            controls().close_window,
        ] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(theme.ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        for control in [
            controls().output,
            controls().vt_output,
            controls().input,
            controls().direct_input,
        ] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(theme.terminal_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
}

fn paint_shell(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let mut client = RECT::default();
    unsafe {
        let hdc = BeginPaint(hwnd, &mut paint);
        let _ = GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;
        let shell_layout = layout::calculate(
            width,
            height,
            current_dpi(),
            sidebar_expanded_for_width(width),
        );
        let sidebar = native_rect(shell_layout.sidebar);
        FillRect(hdc, &client, theme().terminal);
        draw_title_bar(
            hdc,
            native_rect(shell_layout.title_bar),
            &theme(),
            current_dpi(),
        );
        let sidebar_border = RECT {
            left: (sidebar.right - dpi::scale(1, current_dpi())).max(sidebar.left),
            top: sidebar.top,
            right: sidebar.right,
            bottom: sidebar.bottom,
        };
        let status_rect = native_rect(shell_layout.status_bar);
        let status_border = RECT {
            left: status_rect.left,
            top: status_rect.top,
            right: status_rect.right,
            bottom: status_rect.top + dpi::scale(1, current_dpi()),
        };
        background::paint(
            hdc,
            sidebar,
            sidebar,
            background::Surface::Sidebar,
            &theme(),
        );
        FillRect(hdc, &sidebar_border, theme().border_subtle);
        FillRect(hdc, &status_rect, theme().surface);
        FillRect(hdc, &status_border, theme().border_subtle);
        let _ = EndPaint(hwnd, &paint);
    }
}

fn native_rect(rect: layout::Rect) -> RECT {
    RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x + rect.width,
        bottom: rect.y + rect.height,
    }
}

fn sidebar_expanded_for_width(width: i32) -> bool {
    SIDEBAR_EXPANDED.with(Cell::get) && width >= dpi::scale(640, current_dpi())
}

fn theme() -> Theme {
    THEME.with(|theme| *theme.borrow())
}

fn controls() -> Controls {
    CONTROLS.with(|controls| *controls.borrow())
}

fn with_blocks<R>(action: impl FnOnce(&CommandBlocks) -> R) -> R {
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let active = sessions.active().expect("active terminal session");
        action(active.blocks())
    })
}

fn with_blocks_mut<R>(action: impl FnOnce(&mut CommandBlocks) -> R) -> R {
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut().expect("active terminal session");
        action(active.blocks_mut())
    })
}

fn active_session_id() -> Option<u64> {
    SESSIONS.with(|sessions| sessions.borrow().active_id)
}

fn layout_controls(width: i32, height: i32) {
    if capture_vt_reading_position() {
        VT_RESTORE_PENDING.set(true);
    }
    let controls = controls();
    let journal_view = if controls.output.0.is_null() {
        None
    } else {
        let selection = edit_selection(controls.output);
        Some((
            selection,
            reader_viewport_offset(controls.output),
            selection.0 == selection.1
                && selection.1 == unsafe { GetWindowTextLengthW(controls.output).max(0) as u32 }
                && reader_at_bottom(controls.output),
        ))
    };
    let dpi = current_dpi();
    let expanded = sidebar_expanded_for_width(width);
    let direct_input = direct_input_active();
    let layout = layout::calculate(width, height, dpi, expanded);
    let mut output_rect = layout.output;
    // Both views share one area and the same PTY dimensions. Switching to
    // reading never resizes the application or introduces a second input box.
    // The hidden VT has the same dimensions in command and TUI modes. A
    // handoff must not make ConPTY replay the old screen into the journal.
    let mut terminal_rect = layout::terminal_area(&layout);
    if direct_input {
        output_rect = terminal_rect;
    }
    let search_open = search_active();
    if search_open {
        let height = dpi::scale(search_bar::HEIGHT_DIP, dpi).min(output_rect.height);
        controls.search.show(true);
        controls.search.layout(
            layout::Rect {
                height,
                ..output_rect
            },
            dpi,
        );
        output_rect.y += height;
        output_rect.height -= height;
        terminal_rect.y += height;
        terminal_rect.height = (terminal_rect.height - height).max(0);
    } else {
        controls.search.show(false);
    }
    let sidebar_inset = dpi::scale(8, dpi);
    let sidebar_width = (layout.sidebar.width - 2 * sidebar_inset).max(0);
    let caption_w = dpi::scale(layout::CAPTION_BUTTON_WIDTH, dpi);
    let caption_h = dpi::scale(32, dpi).min(layout.title_bar.height);
    let caption_y = (layout.title_bar.height - caption_h) / 2;
    let status_inset = dpi::scale(16, dpi);
    let size_width = dpi::scale(88, dpi);
    let encoding_width = dpi::scale(56, dpi);
    unsafe {
        for (control, rect) in [
            (controls.session_header, layout.session_header),
            (controls.output, output_rect),
            (controls.vt_output, output_rect),
            (controls.terminal, terminal_rect),
            (controls.input_label, layout.input_label),
            (controls.input, layout.input),
            (controls.direct_input, terminal_rect),
            (controls.append, layout.execute_button),
        ] {
            move_rect(control, rect);
        }
        for (index, control) in [controls.minimize, controls.maximize, controls.close_window]
            .iter()
            .enumerate()
        {
            let _ = MoveWindow(
                *control,
                width - (3 - index as i32) * caption_w,
                caption_y,
                caption_w,
                caption_h,
                true,
            );
        }
        let _ = MoveWindow(
            controls.sidebar_toggle,
            sidebar_inset,
            layout.sidebar.y + dpi::scale(12, dpi),
            sidebar_width,
            dpi::scale(40, dpi),
            true,
        );
        let _ = MoveWindow(
            controls.sidebar_terminals,
            sidebar_inset,
            layout.sidebar.y + layout.session_header.height + dpi::scale(16, dpi),
            sidebar_width,
            dpi::scale(52, dpi),
            true,
        );
        let _ = MoveWindow(
            controls.sidebar_profiles,
            sidebar_inset,
            layout.sidebar.y + layout.session_header.height + dpi::scale(92, dpi),
            sidebar_width,
            dpi::scale(48, dpi),
            true,
        );
        let status_available =
            (layout.status_bar.width - 2 * status_inset - size_width - encoding_width).max(0);
        let announcement_w = (status_available / 2).min(dpi::scale(280, dpi));
        let gap = dpi::scale(8, dpi).min(status_available.saturating_sub(announcement_w));
        let connection_w = status_available - announcement_w - gap;
        let _ = MoveWindow(
            controls.status_connection,
            layout.status_bar.x + status_inset,
            layout.status_bar.y + 1,
            connection_w,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        let _ = MoveWindow(
            controls.announcement.hwnd,
            layout.status_bar.x + status_inset + connection_w + gap,
            layout.status_bar.y + 1,
            announcement_w,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        let _ = MoveWindow(
            controls.status_encoding,
            width - status_inset - size_width - encoding_width,
            layout.status_bar.y + 1,
            encoding_width,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        let _ = MoveWindow(
            controls.status_size,
            width - status_inset - size_width,
            layout.status_bar.y + 1,
            size_width,
            (layout.status_bar.height - 1).max(0),
            true,
        );
        for control in [controls.input, controls.append] {
            let _ = ShowWindow(control, if direct_input { SW_HIDE } else { SW_SHOW });
        }
        let direct_visible = direct_input && !reading_active();
        let _ = ShowWindow(
            controls.direct_input,
            if direct_visible { SW_SHOW } else { SW_HIDE },
        );
        let _ = ShowWindow(
            controls.input_label,
            if !direct_input { SW_SHOW } else { SW_HIDE },
        );
        let _ = ShowWindow(
            controls.output,
            if reading_active() && !vt_reading_active() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let _ = ShowWindow(
            controls.vt_output,
            if reading_active() && vt_reading_active() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let _ = ShowWindow(
            controls.terminal,
            if reading_active() { SW_HIDE } else { SW_SHOW },
        );
        if direct_visible {
            // Cover the text provider without hiding it or taking its focus.
            // WS_CLIPSIBLINGS excludes this area from native EDIT drawing DCs.
            let _ = SetWindowPos(
                controls.terminal,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        let hint = if direct_input {
            "Прямой ввод · Ctrl+Shift+Space — читать экран · F1 — справка"
        } else if expanded {
            "Команда · Enter — выполнить, Shift+Enter — новая строка"
        } else {
            "Команда · Enter / Shift+Enter"
        };
        if window_text(controls.input_label) != hint {
            let _ = SetWindowTextW(controls.input_label, PCWSTR(to_wide(hint).as_ptr()));
        }
        let _ = InvalidateRect(Some(controls.sidebar_toggle), None, true);
        let _ = InvalidateRect(Some(controls.sidebar_terminals), None, true);
        let _ = InvalidateRect(Some(controls.sidebar_profiles), None, true);
    }
    update_sidebar_toggle_name(expanded);
    update_mode_controls();
    resize_active_terminal();
    if let Some((selection, viewport, follow)) = journal_view {
        restore_native_reading(controls.output, selection, viewport, follow);
    }
    update_vt_output(false);
    update_direct_output(false);
    refresh_search();
}

fn move_rect(control: HWND, rect: layout::Rect) {
    unsafe {
        let _ = MoveWindow(control, rect.x, rect.y, rect.width, rect.height, true);
    }
}

fn client_width(hwnd: HWND) -> i32 {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    client.right - client.left
}

fn toggle_sidebar(hwnd: HWND) {
    let expanded = sidebar_expanded_for_width(client_width(hwnd));
    SIDEBAR_EXPANDED.set(!expanded);
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        layout_controls(client.right - client.left, client.bottom - client.top);
        rebuild_tab_controls(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = SetFocus(Some(controls().sidebar_toggle));
    }
}

fn update_sidebar_toggle_name(expanded: bool) {
    let text = if expanded {
        "Свернуть боковую панель"
    } else {
        "Развернуть боковую панель"
    };
    let wide = to_wide(text);
    unsafe {
        let _ = SetWindowTextW(controls().sidebar_toggle, PCWSTR(wide.as_ptr()));
    }
    let _ = set_accessible_name(controls().sidebar_toggle, text);
}

fn direct_input_active() -> bool {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.direct_input)
    })
}

fn reading_active() -> bool {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_none_or(|active| active.reading)
    })
}

fn active_display() -> HWND {
    if reading_active() {
        reading_control()
    } else if direct_input_active() {
        controls().direct_input
    } else {
        controls().terminal
    }
}

fn vt_reading_active() -> bool {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.vt_reading)
    })
}

fn reading_control() -> HWND {
    if vt_reading_active() {
        controls().vt_output
    } else {
        controls().output
    }
}

fn search_active() -> bool {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.search_open)
    })
}

fn announce(text: &str) {
    // Explicit UI feedback takes priority over coalesced automatic output.
    SPEECH_PAUSE_UNTIL.set(Some(Instant::now() + std::time::Duration::from_secs(1)));
    cancel_output_speech();
    let announcer = controls().announcement;
    let owner = unsafe { GetParent(announcer.hwnd).ok() };
    if owner.is_some_and(window_is_foreground) {
        let _ = announcer.announce(text);
    }
}

fn cancel_search_feedback() {
    SEARCH_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
}

fn schedule_search_feedback() {
    let context = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let active = sessions.active()?;
        active
            .search_open
            .then(|| (active.id, active.search.query().to_owned()))
    });
    if let Some((session, query)) = context {
        SEARCH_FEEDBACK.with(|feedback| {
            feedback
                .borrow_mut()
                .schedule(Instant::now(), session, &query)
        });
    }
}

fn poll_search_feedback() {
    let due = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let Some(active) = sessions.active() else {
            return false;
        };
        SEARCH_FEEDBACK.with(|feedback| {
            feedback.borrow_mut().take_due(
                Instant::now(),
                active.id,
                active.search.query(),
                active.search_open,
            )
        })
    });
    if due {
        announce_search_status();
    }
}

fn announce_search_status() {
    cancel_search_feedback();
    refresh_search();
    announce(&search_status_text());
}

fn sync_search_controls() {
    let query = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| active.search.query().to_owned())
            .unwrap_or_default()
    });
    SEARCH_UI_SYNC.set(true);
    if window_text(controls().search.query) != query {
        unsafe {
            let _ = SetWindowTextW(controls().search.query, PCWSTR(to_wide(&query).as_ptr()));
        }
    }
    SEARCH_UI_SYNC.set(false);
    update_search_status();
}

fn search_status_text() -> String {
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let Some(active) = sessions.active() else {
            return String::new();
        };
        if active.search.query().is_empty() {
            "Введите текст".to_owned()
        } else if let Some(index) = active.search.current_index() {
            let edge = match active.search_wrapped {
                Some(true) => " — с конца",
                Some(false) => " — с начала",
                None => "",
            };
            format!("{} из {}{edge}", index + 1, active.search.matches().len())
        } else {
            "Ничего не найдено".to_owned()
        }
    })
}

fn update_search_status() {
    let status = search_status_text();
    // STATIC exposes its current caption as Name. Do not pin it to an old
    // accessibility annotation or announce the same count on every poll.
    if window_text(controls().search.status) != status {
        unsafe {
            let _ = SetWindowTextW(controls().search.status, PCWSTR(to_wide(&status).as_ptr()));
        }
    }
}

fn refresh_search() {
    let changed = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let Some(active) = sessions.active_mut() else {
            return false;
        };
        if !active.search_open
            || active.search.document_revision() == Some(active.terminal.reading_revision())
        {
            return false;
        }
        let document = active.reading_document_snapshot();
        let changed = active.search.refresh(&document);
        if changed {
            active.search_wrapped = None;
        }
        changed
    });
    if changed {
        update_search_status();
        invalidate_terminal();
    }
}

fn open_search(hwnd: HWND) {
    let control_id = unsafe { GetDlgCtrlID(GetFocus()) };
    save_active_view();
    let opened = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let Some(active) = sessions.active_mut() else {
            return false;
        };
        if !active.search_open {
            active.search_origin = Some(SearchOrigin {
                control_id,
                direct_input: active.direct_input,
                reading: active.reading,
                vt_reading: active.vt_reading,
                focus_in_reader: active.focus_in_reader,
            });
        }
        active.search_open = true;
        active.search_wrapped = None;
        active.search_focus = true;
        active.vt_reading = true;
        active.terminal_capture = false;
        true
    });
    if !opened {
        return;
    }
    VT_RESTORE_PENDING.set(true);
    // Populate a newly selected document while its native control is hidden.
    // Query editing itself must not keep changing the reader's caret.
    if !unsafe { IsWindowVisible(controls().vt_output).as_bool() } {
        update_vt_output(true);
    }
    sync_search_controls();
    refresh_layout(hwnd);
    update_search_status();
    reveal_search_result(hwnd, false);
    unsafe {
        let _ = SetFocus(Some(controls().search.query));
        SendMessageW(
            controls().search.query,
            EM_SETSEL,
            Some(WPARAM(0)),
            Some(LPARAM(-1)),
        );
    }
    schedule_search_feedback();
}

fn close_search(hwnd: HWND, restore_origin: bool) {
    cancel_search_feedback();
    if !search_active() {
        return;
    }
    save_active_view();
    let origin = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        active.search_open = false;
        active.search_focus = false;
        active.terminal_capture = false;
        let origin = active.search_origin.take();
        if restore_origin {
            if let Some(origin) = origin {
                active.direct_input = origin.direct_input;
                active.reading = origin.reading;
                active.vt_reading = origin.vt_reading;
                active.focus_in_reader = origin.focus_in_reader;
            }
        }
        origin
    });
    VT_RESTORE_PENDING.set(true);
    refresh_layout(hwnd);
    invalidate_terminal();
    unsafe {
        if restore_origin {
            let focus = origin
                .and_then(|origin| GetDlgItem(Some(hwnd), origin.control_id).ok())
                .filter(|control| {
                    IsWindowVisible(*control).as_bool() && IsWindowEnabled(*control).as_bool()
                })
                .unwrap_or_else(input_focus_target);
            let _ = SetFocus(Some(focus));
        }
    }
}

fn change_search_query(hwnd: HWND) {
    let query = window_text(controls().search.query);
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            let document = active.reading_document_snapshot();
            active.search.set_query(query, &document);
            active.search_wrapped = None;
        }
    });
    update_search_status();
    reveal_search_result(hwnd, false);
    schedule_search_feedback();
    invalidate_terminal();
}

fn navigate_search(hwnd: HWND, previous: bool) {
    if !search_active() {
        return;
    }
    refresh_search();
    let found = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let Some(active) = sessions.active_mut() else {
            return false;
        };
        let index = active.search.current_index();
        let count = active.search.matches().len();
        active.search_wrapped = index
            .filter(|index| {
                if previous {
                    *index == 0
                } else {
                    *index + 1 == count
                }
            })
            .map(|_| previous);
        active.search.navigate(previous)
    });
    update_search_status();
    if found {
        reveal_search_result(hwnd, false);
    }
    announce_search_status();
}

fn reveal_search_result(hwnd: HWND, focus_reader: bool) {
    capture_vt_reading_position();
    refresh_search();
    let result = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        let found = active.search.current()?;
        let document = active.reading_document_snapshot();
        // Native selection endpoints must never split a surrogate or a
        // combining glyph even when the literal query matches only its base.
        let start = document
            .cell_at(found.start)
            .map_or(found.start, |cell| cell.start);
        let end = document
            .cell_at(found.end.saturating_sub(1))
            .map_or(found.end, |cell| cell.end);
        active.vt_reading = true;
        if focus_reader {
            active.reading = true;
            active.focus_in_reader = true;
            active.search_focus = false;
        }
        active.terminal_capture = false;
        active.screen_selection = None;
        active.terminal.scroll_to_reading_offset(start);
        Some((start, end, active.reading))
    });
    let Some((start, end, reading)) = result else {
        return;
    };
    if focus_reader {
        refresh_layout(hwnd);
    }
    if reading && (focus_reader || unsafe { GetFocus() == controls().vt_output }) {
        update_vt_output(false);
        unsafe {
            if focus_reader {
                let _ = SetFocus(Some(controls().vt_output));
            }
            if edit_selection(controls().vt_output) != (start, end) {
                SendMessageW(
                    controls().vt_output,
                    EM_SETSEL,
                    Some(WPARAM(start as usize)),
                    Some(LPARAM(end as isize)),
                );
            }
            SendMessageW(controls().vt_output, EM_SCROLLCARET, None, None);
        }
        capture_vt_reading_position();
    }
    invalidate_terminal();
}

fn handle_search_key(hwnd: HWND, key: usize) -> bool {
    let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let alt = unsafe { GetKeyState(VK_MENU.0 as i32) < 0 };
    if !search_active() {
        return false;
    }
    if !control && !alt && key == VK_ESCAPE.0 as usize {
        close_search(hwnd, true);
        return true;
    }
    if unsafe { GetFocus() == controls().search.query } {
        if key == VK_RETURN.0 as usize && !control && !alt {
            reveal_search_result(hwnd, true);
            announce_search_status();
            return true;
        }
        if (control || alt) && !shift && [VK_UP.0 as usize, VK_DOWN.0 as usize].contains(&key) {
            return true;
        }
    }
    false
}

fn input_focus_target() -> HWND {
    if SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.search_open && active.search_focus)
    }) {
        return controls().search.query;
    }
    let in_reader = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.reading && (active.direct_input || active.focus_in_reader))
    });
    if in_reader {
        reading_control()
    } else if direct_input_active() {
        controls().direct_input
    } else {
        controls().input
    }
}

fn set_reading(hwnd: HWND, reading: bool) {
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.reading = reading;
            active.focus_in_reader = reading;
            active.terminal_capture = false;
        }
    });
    if reading && vt_reading_active() {
        VT_RESTORE_PENDING.set(true);
    }
    refresh_layout(hwnd);
    update_vt_output(false);
    unsafe {
        let _ = SetFocus(Some(input_focus_target()));
    }
}

fn toggle_reading_focus(hwnd: HWND) {
    let leaving_reader = reading_active() && unsafe { GetFocus() == reading_control() };
    save_active_view();
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.search_focus = false;
            if !(leaving_reader && active.finish_cli_input()) {
                active.focus_in_reader = !leaving_reader;
                active.reading = !leaving_reader || !active.direct_input;
                active.terminal_capture = leaving_reader && active.direct_input;
            }
        }
    });
    if vt_reading_active() {
        VT_RESTORE_PENDING.set(true);
    }
    refresh_layout(hwnd);
    unsafe {
        let _ = SetFocus(Some(input_focus_target()));
    }
}

fn read_current_input() {
    if !direct_input_active() {
        let draft = window_text(controls().input);
        announce(if draft.is_empty() {
            "Поле ввода пусто"
        } else {
            &draft
        });
        return;
    }
    poll_terminal();
    let text = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        let mut frame = active.terminal.live_snapshot();
        let confirmed = DIRECT_ECHO.with(|echo| echo.borrow().last_confirmed(&frame));
        frame.cursor = direct_accessibility::reading_cursor(&frame, confirmed);
        frame.cursor?;
        let document = active.reading_document_snapshot();
        let caret = direct_accessibility::cursor_offset(&document, &frame);
        direct_accessibility::current_line(&document, caret)
    });
    match text {
        Some(text) if !text.is_empty() => announce(&text),
        Some(_) => announce("Строка ввода пуста"),
        None => announce("Приложение не показывает позицию ввода. Ctrl+Shift+Space — читать экран"),
    }
}

fn navigate_vt_reading(hwnd: HWND, previous: bool) {
    poll_terminal();
    let reader_position =
        (reading_active() && vt_reading_active()).then(|| edit_selection(controls().vt_output).0);
    let source = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        let document = active.reading_document_snapshot();
        let position = reader_position.unwrap_or_else(|| {
            let mut frame = active.terminal.live_snapshot();
            let confirmed = DIRECT_ECHO.with(|echo| echo.borrow().last_confirmed(&frame));
            frame.cursor = direct_accessibility::reading_cursor(&frame, confirmed);
            direct_accessibility::cursor_offset(&document, &frame)
        });
        let target = direct_accessibility::adjacent_line(&document, position, previous);
        Some((active.id, document, target))
    });
    let Some((id, document, target)) = source else {
        return;
    };
    save_active_view();
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        if let Some(active) = sessions.active_mut().filter(|active| active.id == id) {
            active.vt_reading = true;
            active.reading = true;
            active.focus_in_reader = true;
            active.terminal_capture = false;
            active
                .vt_state
                .capture(&document, (target, target), target, false);
        }
    });
    VT_RESTORE_PENDING.set(true);
    refresh_layout(hwnd);
    update_vt_output(true);
    unsafe {
        let _ = SetFocus(Some(controls().vt_output));
        SendMessageW(controls().vt_output, EM_SCROLLCARET, None, None);
    }
    let text = direct_accessibility::current_line(&document, target).unwrap_or_default();
    announce(if text.is_empty() {
        "Пустая строка"
    } else {
        &text
    });
}

fn begin_input_selection(hwnd: HWND) -> bool {
    poll_terminal();
    if !direct_input_has_focus() || !window_is_foreground(hwnd) {
        return false;
    }
    let source = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        if !active.direct_input || active.reading {
            return None;
        }
        let mut frame = active.terminal.live_snapshot();
        let confirmed = DIRECT_ECHO.with(|echo| echo.borrow().last_confirmed(&frame));
        frame.cursor = direct_accessibility::reading_cursor(&frame, confirmed);
        frame.cursor?;
        let document = active.reading_document_snapshot();
        let caret = direct_accessibility::cursor_offset(&document, &frame);
        Some((active.id, document, caret))
    });
    let Some((id, document, caret)) = source else {
        announce("Приложение не показывает позицию ввода. Ctrl+Shift+Space — читать экран");
        return false;
    };
    save_active_view();
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions
            .borrow_mut()
            .active_mut()
            .filter(|active| active.id == id)
        {
            active.vt_reading = true;
            active.reading = true;
            active.focus_in_reader = true;
            active.terminal_capture = false;
            active
                .vt_state
                .capture(&document, (caret, caret), caret, false);
        }
    });
    DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
    DIRECT_ECHO.with(|echo| echo.borrow_mut().cancel());
    VT_RESTORE_PENDING.set(true);
    refresh_layout(hwnd);
    unsafe {
        let _ = SetFocus(Some(controls().vt_output));
        SendMessageW(controls().vt_output, EM_SCROLLCARET, None, None);
    }
    announce("Выделение для чтения и копирования. Esc — вернуться к вводу TUI");
    true
}

fn read_new_output(hwnd: HWND) {
    poll_terminal();
    save_active_view();
    let first_line = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        let document = active.reading_document_snapshot();
        // With no recognizable change, the current live page is still always
        // available. It is never labeled as an assistant's semantic answer.
        let live_start = document
            .cells
            .iter()
            .find(|cell| cell.line >= 0)
            .map_or(0, |cell| cell.start);
        let start = active.reply_anchor.offset(&document).unwrap_or(live_start);
        active.vt_reading = true;
        active.reading = true;
        active.focus_in_reader = true;
        active.terminal_capture = false;
        active
            .vt_state
            .capture(&document, (start, start), start, false);
        Some(direct_accessibility::current_line(&document, start).unwrap_or_default())
    });
    let Some(first_line) = first_line else {
        return;
    };
    VT_RESTORE_PENDING.set(true);
    refresh_layout(hwnd);
    update_vt_output(true);
    unsafe {
        let _ = SetFocus(Some(controls().vt_output));
        SendMessageW(controls().vt_output, EM_SCROLLCARET, None, None);
    }
    if first_line.is_empty() {
        announce("Документ вывода пуст. Esc — вернуться к вводу");
    } else {
        announce(&format!("Новый вывод. {first_line}"));
    }
}

fn select_reading_source(hwnd: HWND, vt: bool) {
    if search_active() {
        close_search(hwnd, false);
    }
    save_active_view();
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.vt_reading = vt;
            active.reading = true;
            active.focus_in_reader = true;
            active.terminal_capture = false;
        }
    });
    if vt {
        VT_RESTORE_PENDING.set(true);
    }
    refresh_layout(hwnd);
    update_vt_output(true);
    unsafe {
        let _ = SetFocus(Some(reading_control()));
    }
}

fn terminal_capture_active() -> bool {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .is_some_and(|active| active.terminal_capture && active.direct_input && !active.reading)
    })
}

fn direct_input_has_focus() -> bool {
    direct_input_active()
        && !reading_active()
        && unsafe { [controls().direct_input, controls().terminal].contains(&GetFocus()) }
}

fn release_terminal_capture() {
    DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
    DIRECT_ECHO.with(|echo| echo.borrow_mut().cancel());
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.terminal_capture = false;
        }
    });
}

fn toggle_terminal_capture(hwnd: HWND) {
    let enable = !terminal_capture_active();
    set_terminal_capture(hwnd, enable);
}

fn set_terminal_capture(hwnd: HWND, enable: bool) {
    close_search(hwnd, false);
    save_active_view();
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.terminal_capture = enable;
            if active.terminal_capture {
                active.direct_input = true;
                active.vt_reading = true;
                active.reading = false;
                active.focus_in_reader = false;
            }
        }
    });
    refresh_layout(hwnd);
    unsafe {
        let _ = SetFocus(Some(input_focus_target()));
    }
    announce(if enable {
        "Прямой ввод. Клавиши передаются приложению. Ctrl+Shift+Space — читать экран, F1 — справка"
    } else {
        "Захват клавиатуры выключен. Tab переключает элементы окна"
    });
}

fn move_tab_focus(backwards: bool) {
    let controls = controls();
    let mut stops = vec![
        controls.sidebar_toggle,
        controls.sidebar_terminals,
        controls.sidebar_profiles,
    ];
    TAB_CONTROLS.with(|tabs| {
        for tab in tabs.borrow().iter() {
            stops.extend([tab.tab, tab.close]);
        }
    });
    stops.extend([
        controls.new_tab,
        controls.search.query,
        controls.search.previous,
        controls.search.next,
        controls.search.close,
        if direct_input_active() && !reading_active() {
            controls.direct_input
        } else {
            active_display()
        },
        controls.input,
        controls.append,
        controls.minimize,
        controls.maximize,
        controls.close_window,
    ]);
    unsafe {
        stops.retain(|control| {
            !control.0.is_null()
                && IsWindowVisible(*control).as_bool()
                && IsWindowEnabled(*control).as_bool()
        });
        if let Some(next) = keyboard_navigation::next_tab_stop(&stops, GetFocus(), backwards) {
            let _ = SetFocus(Some(next));
        }
    }
}

fn update_mode_controls() {
    let mode = if direct_input_active() {
        "прямой ввод"
    } else {
        "командный режим"
    };
    let _ = set_accessible_name(
        controls().sidebar_profiles,
        &format!("Профили и действия; {mode}"),
    );
}

fn toggle_input_mode(hwnd: HWND) {
    close_search(hwnd, false);
    save_active_view();
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.direct_input = !active.direct_input;
            if !active.direct_input {
                active.cancel_cli_input();
            }
            if active.direct_input {
                active.vt_reading = true;
            }
            active.reading = !active.direct_input;
            active.terminal_capture = false;
            active.focus_in_reader = false;
        }
    });
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        layout_controls(client.right - client.left, client.bottom - client.top);
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = SetFocus(Some(input_focus_target()));
    }
    announce(if direct_input_active() {
        "Терминал готов к вводу. Ctrl+Shift+Space — читать экран; Ctrl+Shift+I — освободить Tab для окна"
    } else {
        "Командный режим. Enter — выполнить, Shift+Enter — новая строка"
    });
}

fn resize_active_terminal() {
    let output = controls().terminal;
    if output.0.is_null() {
        return;
    }
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(output, &mut client);
    }
    let (cell_width, cell_height) = terminal_view::cell_metrics(output, theme().terminal_font);
    if cell_width <= 0
        || cell_height <= 0
        || client.right - client.left < cell_width * 2
        || client.bottom - client.top < cell_height
    {
        return;
    }
    let cols = ((client.right - client.left) / cell_width).clamp(2, i16::MAX as i32) as u16;
    let rows = ((client.bottom - client.top) / cell_height).clamp(1, i16::MAX as i32) as u16;
    let size = SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            let _ = active.resize(PtySize::new(cols, rows));
            Some(active.size)
        } else {
            None
        }
    });
    if let Some(size) = size {
        let text = format!("{}×{}", size.cols, size.rows);
        if window_text(controls().status_size) != text {
            unsafe {
                let _ = SetWindowTextW(controls().status_size, PCWSTR(to_wide(&text).as_ptr()));
            }
            let _ =
                set_accessible_name(controls().status_size, &format!("Размер терминала: {text}"));
        }
    }
}

fn create_session(hwnd: HWND, profile: &profile::Profile) {
    // An explicit user/session action supersedes an earlier deferred startup,
    // even if this new launch subsequently fails validation.
    PENDING_CLI_LAUNCH.with(|pending| pending.borrow_mut().cancel());
    if let Err(error) = profile.validate() {
        report_terminal_error("Профиль не запущен", &error);
        return;
    }
    let size = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| active.size)
            .unwrap_or_default()
    });
    save_active_view();
    let result = SESSIONS.with(|sessions| sessions.borrow_mut().create(profile, size));
    match result {
        Ok(_) => {
            remember_profile(profile);
            SESSIONS.with(|sessions| {
                if let Some(active) = sessions.borrow_mut().active_mut() {
                    CODEX_CLI.with(|cli| {
                        if let Some(cli) = cli.borrow().as_ref() {
                            active.attach_codex_cli(cli);
                        }
                    });
                    active.set_startup_text(STARTUP_TEXT);
                    prepare_cli_view(active);
                }
            });
            load_active_view();
            refresh_layout(hwnd);
            rebuild_tab_controls(hwnd);
            update_connection_status();
            unsafe {
                let _ = SetFocus(Some(input_focus_target()));
            }
        }
        Err(error) => report_terminal_error("Профиль не запущен", &error.to_string()),
    }
}

fn activate_session(hwnd: HWND, id: u64) {
    PENDING_CLI_LAUNCH.with(|pending| pending.borrow_mut().cancel());
    save_active_view();
    if !SESSIONS.with(|sessions| sessions.borrow_mut().activate(id)) {
        return;
    }
    load_active_view();
    refresh_layout(hwnd);
    rebuild_tab_controls(hwnd);
    update_connection_status();
    unsafe {
        let _ = SetFocus(Some(input_focus_target()));
    }
    schedule_search_feedback();
}

fn restart_session(hwnd: HWND) {
    let profile = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| active.profile.clone())
    });
    if let Some(profile) = profile {
        create_session(hwnd, &profile);
    } else {
        announce("Нет сессии для перезапуска");
    }
}

fn show_ssh_profiles(hwnd: HWND) {
    refresh_profiles();
    cancel_search_feedback();
    release_terminal_capture();
    let selected_id = SESSIONS.with(|sessions| {
        sessions.borrow().active().and_then(|active| {
            (active.profile.kind == profile::ProfileKind::Ssh).then(|| active.profile_id.clone())
        })
    });
    let result = PROFILES
        .with(|profiles| profile_editor::show(hwnd, &profiles.borrow(), selected_id.as_deref()));
    if let Err(error) = result {
        show_error("Не удалось открыть SSH-профили", &error);
    }
}

fn finish_profile_edit() {
    let Some(profile_editor::Outcome::Saved(profile)) = profile_editor::take_result() else {
        return;
    };
    let result = PROFILES.with(|profiles| {
        profiles
            .borrow_mut()
            .save_ssh_profile(&profiles_path(), profile)
    });
    match result {
        Ok(()) => {
            retain_runtime_cli_profile();
            announce("SSH-профиль сохранён");
        }
        Err(error) => report_terminal_error("SSH-профиль не сохранён", &error),
    }
}

fn show_secret_input(hwnd: HWND) {
    cancel_search_feedback();
    release_terminal_capture();
    let target = SESSIONS.with(|sessions| {
        sessions.borrow().active().and_then(|session| {
            (session.is_alive()
                && matches!(
                    session.state,
                    SessionState::Starting | SessionState::Running
                ))
            .then(|| (session.id, session.title.clone()))
        })
    });
    let Some((id, title)) = target else {
        announce("Нет работающей сессии для защищённого ввода");
        return;
    };
    let context = format!("Сессия: {title}. Отправляйте только когда программа ожидает скрытый пароль или ключевую фразу.");
    let result = secret_input::show(hwnd, &context, move |secret| {
        // Bind to the session that opened the dialog, without using the
        // command editor, its draft or command-block tracking.
        let result = SESSIONS.with(|sessions| {
            sessions
                .borrow()
                .get(id)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "Сессия уже закрыта")
                })
                .and_then(|session| session.write(secret.as_bytes()))
        });
        match result {
            Ok(()) => announce("Защищённый ввод отправлен"),
            Err(error) => report_terminal_error("Защищённый ввод не отправлен", &error.to_string()),
        }
    });
    if let Err(error) = result {
        show_error("Не удалось открыть защищённый ввод", &error);
    }
}

fn close_session(hwnd: HWND, id: u64) {
    let (title, profile_id, alive) = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .get(id)
            .map(|session| {
                (
                    session.title.clone(),
                    session.profile_id.clone(),
                    session.is_alive(),
                )
            })
            .unwrap_or_default()
    });
    if alive {
        let text = to_wide(&format!(
            "Закрыть вкладку «{title}» ({profile_id}) и завершить её процесс?"
        ));
        let answer = unsafe {
            MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                w!("Закрытие вкладки"),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        if answer != IDYES {
            return;
        }
    }
    save_active_view();
    if !SESSIONS.with(|sessions| sessions.borrow_mut().close(id)) {
        announce("Завершение других сессий ещё идёт. Повторите закрытие позже");
        return;
    }
    if active_session_id().is_some() {
        load_active_view();
    } else {
        set_output_text("");
        unsafe {
            let _ = SetWindowTextW(controls().input, w!(""));
        }
    }
    rebuild_tab_controls(hwnd);
    refresh_layout(hwnd);
    update_connection_status();
    if active_session_id().is_none()
        && SETTINGS
            .with(|settings| settings.borrow().last_tab == settings::LastTabPolicy::CloseWindow)
    {
        unsafe {
            SendMessageW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                None,
                None,
            );
        }
        return;
    }
    unsafe {
        let target = if active_session_id().is_some() {
            input_focus_target()
        } else {
            controls().new_tab
        };
        let _ = SetFocus(Some(target));
    }
}

fn refresh_layout(hwnd: HWND) {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        layout_controls(client.right, client.bottom);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn save_active_view() {
    DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
    DIRECT_ECHO.with(|echo| echo.borrow_mut().cancel());
    if tui_window::is_open() {
        tui_window::invalidate("Источник изменился. Откройте диалог заново в нужной вкладке.");
    }
    cancel_output_speech();
    cancel_search_feedback();
    capture_vt_reading_position();
    let selection = output_selection();
    let first_visible = reader_viewport_offset(controls().output);
    let draft = window_text(controls().input);
    let focus = unsafe { GetFocus() };
    let bar = controls().search;
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.selection = selection;
            active.first_visible_offset = first_visible;
            active.draft = draft;
            active.search_focus = active.search_open
                && [bar.query, bar.previous, bar.next, bar.close].contains(&focus);
            active.terminal_capture = false;
        }
    });
}

fn load_active_view() {
    DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
    DIRECT_ECHO.with(|echo| echo.borrow_mut().cancel());
    DIRECT_DOCUMENT.with(|document| *document.borrow_mut() = None);
    // The destination may have accumulated output while it was inactive.
    cancel_output_speech();
    VT_DISPLAY_KEY.set(None);
    VT_DISPLAY_DOCUMENT.with(|document| *document.borrow_mut() = None);
    VT_RESTORE_PENDING.set(false);
    unsafe {
        let _ = ShowWindow(controls().vt_output, SW_HIDE);
        let _ = SetWindowTextW(controls().vt_output, w!(""));
        let _ = ShowWindow(controls().direct_input, SW_HIDE);
        let _ = SetWindowTextW(controls().direct_input, w!(""));
    }
    UTF16_ENCODER.with(|encoder| *encoder.borrow_mut() = Utf16Encoder::new());
    let (buffer, draft, selection, first_visible) = SESSIONS.with(|sessions| {
        sessions
            .borrow_mut()
            .active_mut()
            .map(|active| {
                active.take_journal_trim();
                (
                    active.buffer().to_owned(),
                    active.draft.clone(),
                    active.selection,
                    active.first_visible_offset,
                )
            })
            .unwrap_or_default()
    });
    set_output_text(&buffer);
    update_vt_output(true);
    invalidate_terminal();
    unsafe {
        let _ = SetWindowTextW(controls().input, PCWSTR(to_wide(&draft).as_ptr()));
        SendMessageW(
            controls().output,
            EM_SETSEL,
            Some(WPARAM(selection.0 as usize)),
            Some(LPARAM(selection.1 as isize)),
        );
        let current_first = SendMessageW(controls().output, EM_GETFIRSTVISIBLELINE, None, None).0;
        let target_first = SendMessageW(
            controls().output,
            EM_LINEFROMCHAR,
            Some(WPARAM(first_visible as usize)),
            None,
        )
        .0;
        SendMessageW(
            controls().output,
            EM_LINESCROLL,
            Some(WPARAM(0)),
            Some(LPARAM(target_first - current_first)),
        );
    }
    if vt_reading_active() {
        VT_RESTORE_PENDING.set(true);
    }
    sync_search_controls();
    refresh_search();
}

fn rebuild_tab_controls(hwnd: HWND) {
    TAB_CONTROLS.with(|controls| {
        for control in controls.borrow_mut().drain(..) {
            unsafe {
                let _ = DestroyWindow(control.tab);
                let _ = DestroyWindow(control.close);
            }
        }
    });
    let metadata = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        sessions
            .sessions
            .iter()
            .map(|session| {
                (
                    session.id,
                    session.title.clone(),
                    sessions.active_id == Some(session.id),
                )
            })
            .collect::<Vec<_>>()
    });
    let active = metadata
        .iter()
        .position(|(_, _, active)| *active)
        .unwrap_or(0);
    let mut client = RECT::default();
    let module = unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        GetModuleHandleW(None)
    };
    let Ok(module) = module else {
        return;
    };
    let dpi = current_dpi();
    let layout = layout::calculate(
        client.right,
        client.bottom,
        dpi,
        sidebar_expanded_for_width(client.right),
    );
    let strip = layout::tabs(
        layout.session_header,
        dpi,
        metadata.len(),
        active,
        TAB_SCROLL.with(Cell::get),
    );
    TAB_SCROLL.set(strip.first_visible);
    move_rect(controls().new_tab, strip.new_tab);
    unsafe {
        let _ = SetWindowPos(
            controls().new_tab,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    let mut new_controls = Vec::new();
    for placement in strip.tabs {
        let (id, title, _active) = &metadata[placement.index];
        let tab = create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            PCWSTR(to_wide(title).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_OWNERDRAW as u32),
            hwnd,
            Some(ID_TAB_BASE + *id as usize),
            HINSTANCE(module.0),
        );
        let close = create_control(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("×"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_OWNERDRAW as u32),
            hwnd,
            Some(ID_TAB_CLOSE_BASE + *id as usize),
            HINSTANCE(module.0),
        );
        let (Ok(tab), Ok(close)) = (tab, close) else {
            continue;
        };
        move_rect(tab, placement.tab);
        move_rect(close, placement.close);
        unsafe {
            let _ = SetWindowPos(
                tab,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = SetWindowPos(
                close,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            SendMessageW(
                tab,
                WM_SETFONT,
                Some(WPARAM(theme().ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
            SendMessageW(
                close,
                WM_SETFONT,
                Some(WPARAM(theme().ui_font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let accessible_name = tab_accessible_name(*id);
        let _ = set_accessible_name(tab, &accessible_name);
        let _ = set_accessible_name(close, &format!("Закрыть вкладку {title}"));
        new_controls.push(TabControls {
            session_id: *id,
            accessible_name,
            tab,
            close,
        });
    }
    TAB_CONTROLS.with(|controls| *controls.borrow_mut() = new_controls);
}

fn tab_accessible_name(id: u64) -> String {
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let Some((index, session)) = sessions
            .sessions
            .iter()
            .enumerate()
            .find(|(_, s)| s.id == id)
        else {
            return String::new();
        };
        let selected = if sessions.active_id == Some(id) {
            ", выбрана"
        } else {
            ""
        };
        format!(
            "Вкладка {}, {} из {}{selected}, {}",
            session.title,
            index + 1,
            sessions.sessions.len(),
            session_status(session)
        )
    })
}

fn refresh_tab_status() {
    TAB_CONTROLS.with(|tabs| {
        for tab in tabs.borrow_mut().iter_mut() {
            let name = tab_accessible_name(tab.session_id);
            if name != tab.accessible_name {
                let _ = set_accessible_name(tab.tab, &name);
                tab.accessible_name = name;
            }
        }
    });
}

fn show_profile_menu(hwnd: HWND) {
    show_profile_menu_at(hwnd, controls().sidebar_profiles, true);
}

enum ProfileIntent {
    NewTab(Option<PathBuf>),
    NewWindow,
}

fn show_profile_menu_at(hwnd: HWND, anchor: HWND, include_actions: bool) {
    show_profile_menu_intent(hwnd, anchor, include_actions, ProfileIntent::NewTab(None));
}

fn show_profile_menu_intent(
    hwnd: HWND,
    anchor: HWND,
    include_actions: bool,
    intent: ProfileIntent,
) {
    refresh_profiles();
    cancel_search_feedback();
    let profiles = PROFILES.with(|profiles| profiles.borrow().profiles.clone());
    if profiles.is_empty() {
        return;
    }
    unsafe {
        let Ok(menu) = profile_menu::ProfileMenu::new(
            &profiles,
            direct_input_active(),
            include_actions,
            &theme(),
            current_dpi(),
        ) else {
            return;
        };
        let mut button = RECT::default();
        let _ = GetWindowRect(anchor, &mut button);
        let selected = TrackPopupMenu(
            menu.handle(),
            TPM_RETURNCMD | TPM_NONOTIFY,
            button.left,
            button.bottom,
            Some(0),
            hwnd,
            None,
        )
        .0 as usize;
        if selected == MENU_INPUT_MODE {
            toggle_input_mode(hwnd);
            return;
        }
        if selected == MENU_HELP {
            release_terminal_capture();
            if let Err(error) = help::show(hwnd) {
                show_error("Не удалось открыть справку", &error);
            }
            return;
        }
        if selected == MENU_READING {
            toggle_reading_focus(hwnd);
            return;
        }
        if selected == MENU_READ_INPUT {
            read_current_input();
            return;
        }
        if selected == MENU_READ_NEW_OUTPUT {
            read_new_output(hwnd);
            return;
        }
        if selected == MENU_TERMINAL_CAPTURE {
            toggle_terminal_capture(hwnd);
            return;
        }
        if [MENU_READ_VT, MENU_READ_JOURNAL].contains(&selected) {
            select_reading_source(hwnd, selected == MENU_READ_VT);
            return;
        }
        if selected == MENU_SEARCH {
            open_search(hwnd);
            return;
        }
        if selected == MENU_RESTART {
            restart_session(hwnd);
            return;
        }
        if selected == MENU_SSH_PROFILES {
            show_ssh_profiles(hwnd);
            return;
        }
        if selected == MENU_SECRET_INPUT {
            show_secret_input(hwnd);
            return;
        }
        if [MENU_SPEECH_ALL, MENU_SPEECH_IMPORTANT, MENU_SPEECH_MANUAL].contains(&selected) {
            select_speech_mode(selected);
            return;
        }
        if selected == MENU_TUI_DIALOG {
            open_tui_dialog(hwnd);
            return;
        }
        match selected {
            MENU_AGENT_PANEL => {
                open_agent_window(hwnd);
                return;
            }
            MENU_NEW_WINDOW => {
                show_profile_menu_intent(hwnd, controls().new_tab, false, ProfileIntent::NewWindow);
                return;
            }
            MENU_OPEN_FOLDER => {
                open_folder_session(hwnd);
                return;
            }
            MENU_PREFERENCES => {
                open_preferences(hwnd);
                return;
            }
            MENU_RESTORE_PROFILE => {
                show_recent_profiles(hwnd);
                return;
            }
            _ => {}
        }
        if let Some(profile) = selected
            .checked_sub(1)
            .and_then(|index| profiles.get(index))
        {
            PROFILES.with(|manager| {
                let mut manager = manager.borrow_mut();
                manager.default_profile_id = profile.id.clone();
            });
            match intent {
                ProfileIntent::NewTab(directory) => {
                    let mut selected = profile.clone();
                    if let Some(directory) = directory {
                        selected.working_directory = Some(directory.to_string_lossy().into_owned());
                    }
                    create_session(hwnd, &selected);
                }
                ProfileIntent::NewWindow => {
                    let directory = active_working_directory();
                    if let Err(error) = desktop::new_window(&profile.id, Some(&directory)) {
                        report_terminal_error("Новое окно не открыто", &error.to_string());
                    }
                }
            }
        }
    }
}

fn start_terminal(hwnd: HWND) {
    let mut manager = ProfileManager::load_or_default(&profiles_path());
    let (profile_id, directory, choose) = LAUNCH.with(|options| {
        options
            .borrow()
            .as_ref()
            .map(|options| {
                (
                    options.profile_id.clone(),
                    options.working_directory.clone(),
                    options.choose_profile,
                )
            })
            .unwrap_or_default()
    });
    let explicitly_requested = profile_id.is_some();
    let profile = if let Some(id) = profile_id.as_deref() {
        manager.get(id).cloned()
    } else if choose || directory.is_some() {
        None
    } else {
        // Restoring profile metadata is not permission to run its commands/SSH.
        let safe = ProfileManager::discover().default_profile().clone();
        if manager.get(&safe.id).is_none() {
            manager.profiles.push(safe.clone());
        }
        Some(safe)
    };
    PROFILES.with(|profiles| *profiles.borrow_mut() = manager);
    let timer = unsafe { SetTimer(Some(hwnd), OUTPUT_TIMER_ID, OUTPUT_POLL_MS, None) };
    if timer == 0 {
        set_output_text("Не удалось запустить обработку вывода.");
        return;
    }
    if let Some(mut profile) = profile {
        if let Some(directory) = directory {
            profile.working_directory = Some(directory.to_string_lossy().into_owned());
        }
        create_session(hwnd, &profile);
    } else if explicitly_requested {
        let deferred = PENDING_CLI_LAUNCH
            .with(|pending| pending.borrow_mut().defer(profile_id.as_deref(), directory));
        set_output_text(if deferred {
            "Поиск запрошенного CLI-профиля…"
        } else {
            "Профиль запуска не найден. Выберите профиль кнопкой новой вкладки."
        });
    }
    update_connection_status();
    resize_active_terminal();
    rebuild_tab_controls(hwnd);
}

fn profiles_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| std::env::var_os("TEMP").unwrap_or_default());
    PathBuf::from(base).join("BKS Term").join("profiles.json")
}

fn prepare_cli_view(session: &mut session::TerminalSession) {
    if !cli_profiles::starts_in_direct_mode(&session.profile) {
        return;
    }
    // Only an explicit CLI profile starts with raw keys. A regular shell keeps
    // its native editable command field and journal, including selection/Tab.
    session.direct_input = true;
    session.reading = false;
    session.vt_reading = true;
    session.focus_in_reader = false;
    session.terminal_capture = true;
}

fn poll_cli_discovery() {
    let result = CLI_DISCOVERY.with(|discovery| {
        let mut discovery = discovery.borrow_mut();
        let receiver = discovery.as_ref()?;
        match receiver.try_recv() {
            Ok(value) => {
                *discovery = None;
                Some(value)
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *discovery = None;
                None
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        }
    });
    if let Some(Some(cli)) = result {
        PROFILES.with(|profiles| {
            let mut profiles = profiles.borrow_mut();
            if profiles.get(codex_cli::PROFILE_ID).is_none() {
                profiles.profiles.push(cli.profile());
            }
        });
        SESSIONS.with(|sessions| {
            for session in &mut sessions.borrow_mut().sessions {
                session.attach_codex_cli(&cli);
            }
        });
        CODEX_CLI.with(|current| *current.borrow_mut() = Some(cli));
    }
}

fn tui_source(session: &session::TerminalSession, active: Option<u64>) -> tui_dialog::Source<'_> {
    tui_dialog::Source {
        session_id: session.id,
        screen_revision: session.revision,
        input_revision: session.input_revision(),
        cli_version: session.verified_codex_version.as_deref().unwrap_or(""),
        direct_codex_profile: session.verified_codex_version.is_some(),
        process_alive: session.is_alive() && session.failure.is_none(),
        active_session: active == Some(session.id),
    }
}

fn active_tui_source_key() -> Option<TuiSourceKey> {
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        TuiSourceKey::from_source(tui_source(sessions.active()?, sessions.active_id))
    })
}

fn bind_tui_worker_response(
    pending: &PendingTuiAnalysis,
    response: tui_worker::Response,
) -> std::result::Result<Option<tui_dialog::DialogSnapshot>, tui_dialog::ActionError> {
    use tui_dialog::ActionError;
    if response.id != pending.id
        || response.session != pending.key.session
        || response.revision != pending.key.revision
        || response.input_revision != pending.key.input_revision
    {
        return Err(ActionError::StaleState);
    }
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let session = sessions
            .get(pending.key.session)
            .ok_or(ActionError::StaleState)?;
        let source = tui_source(session, sessions.active_id);
        if TuiSourceKey::from_source(source).as_ref() != Some(&pending.key)
            || !session.tui_input_ready()
            || session.terminal.live_snapshot() != pending.screen
        {
            return Err(ActionError::StaleState);
        }
        response
            .matched
            .map(|matched| match matched.shape {
                tui_worker::Shape::CodexFolderTrust => {
                    tui_dialog::DialogSnapshot::bind_worker_result(
                        source,
                        &pending.screen,
                        matched.selected,
                        matched.context,
                    )
                }
            })
            .transpose()
    })
}

fn poll_tui_adapter() {
    let now = Instant::now();
    let (key, idle) = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        sessions.active().map_or((None, false), |session| {
            (
                TuiSourceKey::from_source(tui_source(session, sessions.active_id)),
                session.tui_input_ready(),
            )
        })
    });
    let (failure, open) = TUI_ADAPTER.with(|adapter| {
        let mut adapter = adapter.borrow_mut();
        adapter.observe(key.as_ref(), idle, now);
        let mut failure = None;
        // The host has a fixed four-slot event queue. Never turn UI polling
        // into an unbounded drain when a child emits unexpected output.
        for _ in 0..4 {
            let event = adapter.host.as_ref().and_then(|host| host.try_poll());
            match event {
                Some(tui_adapter_host::Event::Ready) => {
                    if !adapter.disabled {
                        adapter.ready = true;
                    }
                }
                Some(tui_adapter_host::Event::Response(response)) => {
                    let Some(pending) = adapter.pending.take() else {
                        continue;
                    };
                    match bind_tui_worker_response(&pending, response) {
                        Ok(dialog) => {
                            adapter.notice_pending = dialog.is_some();
                            adapter.cached = Some(CachedTuiAnalysis {
                                key: pending.key,
                                dialog,
                            });
                        }
                        Err(
                            tui_dialog::ActionError::InvalidContext
                            | tui_dialog::ActionError::InvalidOption,
                        ) => {
                            if adapter.disable() {
                                failure = Some(tui_adapter_host::Failure::InvalidResponse);
                            }
                        }
                        Err(_) => {
                            // Already-arrived terminal output, owner input, a
                            // version change or tab switch invalidates a reply.
                            if adapter.last_submitted.as_ref() == Some(&pending.key) {
                                adapter.last_submitted = None;
                            }
                        }
                    }
                }
                Some(tui_adapter_host::Event::Disabled(reason)) => {
                    if adapter.disable() {
                        failure = Some(reason);
                    }
                }
                Some(tui_adapter_host::Event::Stopped) => {
                    if adapter.disable() {
                        failure = Some(tui_adapter_host::Failure::Exited);
                    }
                }
                None => break,
            }
        }
        if key.is_some()
            && idle
            && adapter.stable(now)
            && !adapter.disabled
            && adapter.host.is_none()
        {
            match tui_adapter_host::Host::start() {
                Ok(host) => adapter.host = Some(host),
                Err(_) => {
                    if adapter.disable() {
                        failure = Some(tui_adapter_host::Failure::Spawn);
                    }
                }
            }
        }
        if let Some(key) = &key {
            if adapter.can_submit(key, now) {
                let id = adapter.next_id;
                adapter.next_id = adapter.next_id.wrapping_add(1);
                adapter.last_submitted = Some(key.clone());
                let source = SESSIONS.with(|sessions| {
                    let sessions = sessions.borrow();
                    let session = sessions.active()?;
                    let source = tui_source(session, sessions.active_id);
                    if TuiSourceKey::from_source(source).as_ref() != Some(key)
                        || !session.tui_input_ready()
                    {
                        return None;
                    }
                    let screen = session.terminal.live_snapshot();
                    tui_worker::Request::from_snapshot(id, source, &screen)
                        .ok()
                        .map(|request| (request, screen))
                });
                if let Some((request, screen)) = source {
                    let result = adapter
                        .host
                        .as_ref()
                        .ok_or(tui_adapter_host::SubmitError::Disabled)
                        .and_then(|host| host.try_submit(request));
                    match result {
                        Ok(()) => {
                            adapter.pending = Some(PendingTuiAnalysis {
                                id,
                                key: key.clone(),
                                screen,
                            });
                        }
                        Err(
                            tui_adapter_host::SubmitError::Disabled
                            | tui_adapter_host::SubmitError::Stopped,
                        ) => {
                            if adapter.disable() {
                                failure = Some(tui_adapter_host::Failure::Exited);
                            }
                        }
                        Err(_) => adapter.last_submitted = None,
                    }
                } else {
                    // Unsupported size/text stays available through the common
                    // reader, and is not retried on each identical timer frame.
                    adapter.cached = Some(CachedTuiAnalysis {
                        key: key.clone(),
                        dialog: None,
                    });
                }
            }
        }
        let open = adapter.open.take().and_then(|request| {
            if tui_window::is_open() || !window_is_foreground(request.owner) {
                return None;
            }
            let same_source = key.as_ref() == Some(&request.key);
            let complete = adapter
                .cached
                .as_ref()
                .is_some_and(|cached| cached.key == request.key);
            if !same_source || adapter.disabled || complete || now >= request.deadline {
                Some((request.owner, same_source && complete && !adapter.disabled))
            } else {
                adapter.open = Some(request);
                None
            }
        });
        (failure, open)
    });
    if let Some(failure) = failure {
        if tui_window::is_open() && TUI_BINDING.with(|binding| binding.borrow().is_some()) {
            tui_window::invalidate(failure.message());
        }
        announce(failure.message());
    }
    if let Some((owner, mapped)) = open {
        // Recheck after notifications: a pending explicit request never steals
        // focus from another window and never upgrades an already open reader.
        if !tui_window::is_open() && window_is_foreground(owner) {
            show_tui_dialog(owner, mapped);
        }
    }
}

fn recognized_tui() -> Option<tui_dialog::DialogSnapshot> {
    let snapshot = TUI_ADAPTER.with(|adapter| {
        let adapter = adapter.borrow();
        if adapter.disabled {
            return None;
        }
        adapter.cached.as_ref()?.dialog.clone()
    })?;
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let session = sessions.active()?;
        if !session.tui_input_ready()
            || snapshot
                .validate_and_map(
                    tui_source(session, sessions.active_id),
                    &session.terminal.live_snapshot(),
                    snapshot.selected,
                )
                .is_err()
        {
            return None;
        }
        if session.tui_applied_context.as_ref() == Some(&snapshot.context) {
            return None;
        }
        Some(snapshot)
    })
}

fn open_tui_dialog(hwnd: HWND) {
    if tui_window::is_open() {
        // The open window owns its original binding; never replace it silently.
        return;
    }
    cancel_search_feedback();
    release_terminal_capture();
    poll_terminal();
    if tui_window::is_open() {
        return;
    }
    if recognized_tui().is_some() {
        show_tui_dialog(hwnd, true);
        return;
    }
    let Some(key) = active_tui_source_key() else {
        show_tui_dialog(hwnd, false);
        return;
    };
    let complete = TUI_ADAPTER.with(|adapter| {
        let mut adapter = adapter.borrow_mut();
        if adapter.disabled {
            // A failure cannot trigger timer retries. Only this explicit user
            // command may replace the disabled host and try the adapter again.
            adapter.explicit_restart();
        }
        adapter
            .cached
            .as_ref()
            .is_some_and(|cached| cached.key == key)
    });
    if complete {
        show_tui_dialog(hwnd, false);
        return;
    }
    TUI_ADAPTER.with(|adapter| {
        adapter.borrow_mut().open = Some(PendingTuiOpen {
            key,
            owner: hwnd,
            deadline: Instant::now() + TUI_OPEN_TIMEOUT,
        });
    });
    announce("Проверяется текущий экран CLI. Доступный обзор откроется после анализа.");
    poll_tui_adapter();
}

fn show_tui_dialog(hwnd: HWND, mapped: bool) {
    if tui_window::is_open() {
        return;
    }
    let snapshot = mapped.then(recognized_tui).flatten();
    let reading = if snapshot.is_none() {
        SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            let session = sessions.active()?;
            Some((
                session.id,
                session.revision,
                session.input_revision(),
                session.title.clone(),
                session.terminal.live_snapshot(),
            ))
        })
    } else {
        None
    };
    let data = if let Some(snapshot) = &snapshot {
        tui_window::ViewData {
            title: snapshot.title.clone(),
            context: snapshot.context.clone(),
            options: snapshot.options.to_vec(),
            selected: snapshot.selected,
            cli_label: format!("Codex CLI {}", snapshot.cli_version),
            mapped: true,
        }
    } else if let Some((_, _, _, label, frame)) = &reading {
        tui_view_data(label, frame)
    } else {
        announce("Нет активной вкладки для чтения экрана TUI");
        return;
    };
    match tui_window::show(hwnd, data) {
        Ok(()) => {
            TUI_BINDING.with(|binding| *binding.borrow_mut() = snapshot);
            TUI_READING_SOURCE.with(|source| {
                source.set(reading.map(|(id, revision, input, _, _)| (id, revision, input)))
            });
        }
        Err(error) => show_error("Не удалось открыть доступный диалог", &error),
    }
}

fn tui_view_data(label: &str, frame: &bks_terminal::TerminalSnapshot) -> tui_window::ViewData {
    let reading = tui_reading::inspect(frame);
    tui_window::ViewData {
        title: "Обзор экрана TUI".to_owned(),
        context: reading.context,
        options: reading.lines,
        selected: reading.selected,
        cli_label: label.to_owned(),
        mapped: false,
    }
}

fn poll_tui_dialog() {
    poll_tui_adapter();
    if TUI_CONTROL.with(|control| control.borrow().is_some()) {
        poll_tui_control();
        return;
    }
    if let Some((id, revision, input)) = TUI_READING_SOURCE.get() {
        let changed = SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            sessions.active_id != Some(id)
                || sessions.get(id).is_none_or(|session| {
                    session.revision != revision || session.input_revision() != input
                })
        });
        if changed && tui_window::is_open() {
            tui_window::invalidate("Исходный экран изменился. Снимок сохранён для чтения.");
        }
        return;
    }
    let stale = TUI_BINDING.with(|binding| {
        let binding = binding.borrow();
        let Some(snapshot) = binding.as_ref() else {
            return false;
        };
        SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            let Some(session) = sessions.get(snapshot.session_id) else {
                return true;
            };
            !session.tui_input_ready()
                || snapshot
                    .validate_and_map(
                        tui_source(session, sessions.active_id),
                        &session.terminal.live_snapshot(),
                        snapshot.selected,
                    )
                    .is_err()
        })
    });
    if stale && tui_window::is_open() {
        tui_window::invalidate("Состояние CLI изменилось. Выбор недоступен; закройте окно и откройте актуальный диалог заново.");
    }
    if tui_window::is_open() {
        return;
    }
    if !TUI_ADAPTER.with(|adapter| std::mem::take(&mut adapter.borrow_mut().notice_pending)) {
        return;
    }
    if let Some(snapshot) = recognized_tui() {
        let notify = SESSIONS.with(|sessions| {
            let mut sessions = sessions.borrow_mut();
            let Some(session) = sessions.get_mut(snapshot.session_id) else {
                return false;
            };
            if session.tui_notice.as_ref() == Some(&snapshot.context) {
                return false;
            }
            session.tui_notice = Some(snapshot.context);
            true
        });
        if notify && speech_mode() != SpeechMode::Manual {
            announce(
                "Codex ожидает решения о доступе к папке. Ctrl+Shift+D — открыть доступный диалог.",
            );
        }
    }
}

fn finish_tui_dialog() {
    let Some(outcome) = tui_window::take_result() else {
        return;
    };
    match outcome {
        tui_window::Outcome::BeginControl => {
            begin_tui_control();
            return;
        }
        tui_window::Outcome::Key(key) => {
            send_tui_control_key(key);
            return;
        }
        tui_window::Outcome::EndControl => {
            TUI_CONTROL.with(|control| *control.borrow_mut() = None);
            return;
        }
        _ => {}
    }
    TUI_CONTROL.with(|control| *control.borrow_mut() = None);
    let snapshot = TUI_BINDING.with(|binding| binding.borrow_mut().take());
    let reading = TUI_READING_SOURCE.take();
    if outcome == tui_window::Outcome::DirectInput {
        let valid = reading.is_some_and(|(id, _, _)| {
            SESSIONS.with(|sessions| {
                let sessions = sessions.borrow();
                sessions.active_id == Some(id)
                    && sessions.get(id).is_some_and(|session| session.is_alive())
            })
        });
        if valid {
            set_terminal_capture(main_window(), true);
        } else {
            announce("Исходная вкладка неактивна или процесс завершён. Выберите вкладку для ввода");
        }
        return;
    }
    let tui_window::Outcome::Apply(index) = outcome else {
        return;
    };
    let Some(snapshot) = snapshot else {
        return;
    };
    // Drain available output before validating; never act from a cached view.
    poll_terminal();
    let foreground = window_is_foreground(main_window());
    let adapter_available = TUI_ADAPTER.with(|adapter| !adapter.borrow().disabled);
    let result: std::result::Result<(), String> = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active_id = sessions.active_id;
        let session = sessions
            .get_mut(snapshot.session_id)
            .ok_or("Исходная сессия закрыта")?;
        if !foreground || !adapter_available || !session.tui_input_ready() {
            return Err("Источник обновляется или окно неактивно. Откройте диалог заново".into());
        }
        let bytes = snapshot
            .validate_and_map(
                tui_source(session, active_id),
                &session.terminal.live_snapshot(),
                index,
            )
            .map_err(|_| "Состояние CLI изменилось. Выбор не отправлен".to_string())?;
        session
            .write_tui(bytes, snapshot.input_revision)
            .map_err(|error| error.to_string())?;
        session.tui_applied_context = Some(snapshot.context.clone());
        Ok(())
    });
    match result {
        Ok(()) => announce("Выбранный вариант передан в CLI"),
        Err(error) => announce(&error),
    }
}

fn freeze_tui_control(reason: &str) {
    TUI_CONTROL.with(|control| *control.borrow_mut() = None);
    tui_window::freeze_control(reason);
}

fn begin_tui_control() {
    let source_id = TUI_READING_SOURCE.get().map(|source| source.0);
    let source = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let active = sessions.active()?;
        (Some(active.id) == source_id && active.is_alive() && tui_window::is_foreground()).then(
            || {
                (
                    active.id,
                    active.revision,
                    active.input_revision(),
                    active.terminal.live_snapshot(),
                )
            },
        )
    });
    let Some((session_id, revision, input_revision, frame)) = source else {
        freeze_tui_control("Исходная вкладка недоступна. Снимок сохранён для чтения.");
        return;
    };
    TUI_CONTROL.with(|control| {
        *control.borrow_mut() = Some(TuiControl {
            session_id,
            input_revision,
            frame,
            observed_revision: revision,
            stable_since: Instant::now(),
            pending: Some(Instant::now()),
            starting: true,
        });
    });
}

fn poll_tui_control() {
    if !tui_window::is_open() || !tui_window::is_foreground() {
        freeze_tui_control("Управление остановлено: окно неактивно. Снимок сохранён для чтения.");
        return;
    }
    let id =
        TUI_CONTROL.with(|control| control.borrow().as_ref().map(|control| control.session_id));
    let source = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let active = sessions.active()?;
        (Some(active.id) == id && active.is_alive()).then(|| {
            (
                active.title.clone(),
                active.revision,
                active.input_revision(),
                active.terminal.live_snapshot(),
                active.tui_input_ready(),
            )
        })
    });
    let Some((title, revision, input, frame, ready)) = source else {
        freeze_tui_control("Исходная вкладка закрыта или процесс завершён. Снимок сохранён.");
        return;
    };
    let now = Instant::now();
    let (update, expired) = TUI_CONTROL.with(|control| {
        let mut control = control.borrow_mut();
        let Some(control) = control.as_mut() else {
            return (false, false);
        };
        if revision != control.observed_revision {
            control.observed_revision = revision;
            control.stable_since = now;
        }
        let deadline = control
            .pending
            .is_some_and(|started| now.duration_since(started) >= Duration::from_millis(700));
        let changed = frame != control.frame;
        let stable = now.duration_since(control.stable_since) >= TUI_STABLE_FOR;
        let update = ready && ((stable && (control.starting || changed)) || deadline);
        // A pending pipe write cannot silently become a successful key reply.
        (update, deadline && !ready)
    });
    if expired {
        freeze_tui_control("Приложение занято. Клавиша не повторяется; управление остановлено.");
        return;
    }
    if !update {
        return;
    }
    let data = tui_view_data(&title, &frame);
    if tui_window::update_live(data).is_err() {
        freeze_tui_control("Экран недоступен. Снимок сохранён для чтения.");
        return;
    }
    TUI_CONTROL.with(|control| {
        if let Some(control) = control.borrow_mut().as_mut() {
            control.input_revision = input;
            control.frame = frame;
            control.pending = None;
            control.starting = false;
            TUI_READING_SOURCE.set(Some((control.session_id, revision, input)));
        }
    });
}

fn send_tui_control_key(key: tui_window::TuiKey) {
    let expected = TUI_CONTROL.with(|control| {
        let mut control = control.borrow_mut();
        let control = control.as_mut()?;
        if control.pending.is_some() {
            return None;
        }
        let expected = (
            control.session_id,
            control.input_revision,
            control.frame.clone(),
        );
        control.pending = Some(Instant::now());
        Some(expected)
    });
    let Some((id, input, frame)) = expected else {
        return;
    };
    // Drain output after capturing what was actually displayed when the user
    // pressed the key. A redraw cannot change that evidence before validation.
    poll_terminal();
    let result: std::result::Result<(), String> = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active_id = sessions.active_id;
        let active = sessions.get_mut(id).ok_or("Исходная вкладка закрыта")?;
        if active_id != Some(id)
            || !tui_window::is_foreground()
            || !active.tui_input_ready()
            || active.input_revision() != input
            || active.terminal.live_snapshot() != frame
        {
            return Err(
                "Экран изменился. Клавиша не отправлена. Включите управление заново.".into(),
            );
        }
        use tui_window::TuiKey;
        let special = match key {
            TuiKey::Up => SpecialKey::Up,
            TuiKey::Down => SpecialKey::Down,
            TuiKey::Left => SpecialKey::Left,
            TuiKey::Right => SpecialKey::Right,
            TuiKey::Home => SpecialKey::Home,
            TuiKey::End => SpecialKey::End,
            TuiKey::PageUp => SpecialKey::PageUp,
            TuiKey::PageDown => SpecialKey::PageDown,
            TuiKey::Tab | TuiKey::ShiftTab => SpecialKey::Tab,
            TuiKey::Escape => SpecialKey::Escape,
            TuiKey::Enter => SpecialKey::Enter,
        };
        let bytes = encode_key(
            special,
            key == TuiKey::ShiftTab,
            false,
            false,
            active.terminal.modes().application_cursor,
        );
        active
            .write_tui(&bytes, input)
            .map_err(|error| error.to_string())?;
        if key == TuiKey::Enter {
            let document = active.reading_document_snapshot();
            active.reply_anchor.submitted(&document);
        }
        Ok(())
    });
    if let Err(error) = result {
        freeze_tui_control(&error);
    }
}

fn settings_path() -> PathBuf {
    profiles_path().with_file_name("settings.json")
}

fn retain_runtime_cli_profile() {
    let mut runtime = CLI_TOOLS.with(|tools| tools.borrow().profiles.clone());
    if let Some(codex) =
        CODEX_CLI.with(|cli| cli.borrow().as_ref().map(codex_cli::SupportedCli::profile))
    {
        runtime.push(codex);
    }
    PROFILES.with(|profiles| {
        let mut profiles = profiles.borrow_mut();
        for runtime in runtime {
            if profiles.get(&runtime.id).is_none() {
                profiles.profiles.push(runtime);
            }
        }
    });
}

fn poll_tool_discovery() {
    let result = TOOL_DISCOVERY.with(|discovery| {
        let mut discovery = discovery.borrow_mut();
        let receiver = discovery.as_ref()?;
        match receiver.try_recv() {
            Ok(value) => {
                *discovery = None;
                Some(value)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *discovery = None;
                Some(cli_profiles::Discovery {
                    notes: vec![
                        "Поиск CLI завершился без результата. Обычные профили доступны.".to_owned(),
                    ],
                    ..Default::default()
                })
            }
        }
    });
    if let Some(result) = result {
        CLI_TOOLS.with(|tools| *tools.borrow_mut() = result);
        retain_runtime_cli_profile();
    }
}

fn poll_pending_cli_launch() {
    let finished = CLI_DISCOVERY.with(|discovery| discovery.borrow().is_none())
        && TOOL_DISCOVERY.with(|discovery| discovery.borrow().is_none());
    let result = PENDING_CLI_LAUNCH.with(|pending| {
        PROFILES.with(|profiles| {
            pending
                .borrow_mut()
                .resolve(&profiles.borrow().profiles, finished)
        })
    });
    match result {
        Some(Ok(profile)) => create_session(main_window(), &profile),
        Some(Err(message)) => {
            set_output_text(&message);
            announce(&message);
        }
        None => {}
    }
}

fn cli_help_text() -> String {
    let pending = TOOL_DISCOVERY.with(|discovery| discovery.borrow().is_some());
    CLI_TOOLS.with(|tools| {
        let tools = tools.borrow();
        let mut text = "УСТАНОВЛЕННЫЕ CLI\r\n".to_owned();
        for (id, name) in [
            ("cli-codex", "Codex"),
            ("cli-kimi", "Kimi"),
            ("cli-claude", "Claude"),
            ("cli-gemini", "Gemini"),
        ] {
            let state = match tools.versions.iter().find(|(key, _)| key == id) {
                Some((_, Some(version))) => version.as_str(),
                Some((_, None)) => "версия не подтверждена; обычный TUI доступен",
                None if pending => "поиск ещё выполняется",
                None => "не обнаружен; можно создать свой профиль",
            };
            text.push_str(&format!("{name}: {state}\r\n"));
        }
        for note in &tools.notes {
            text.push_str(note);
            text.push_str("\r\n");
        }
        text.push_str("\r\n");
        text
    })
}

fn refresh_profiles() {
    if let Some(latest) =
        ProfileManager::load(&profiles_path()).filter(|manager| !manager.profiles.is_empty())
    {
        PROFILES.with(|profiles| *profiles.borrow_mut() = latest);
    }
    retain_runtime_cli_profile();
}

fn workspace_path() -> PathBuf {
    profiles_path().with_file_name("workspace.json")
}

fn active_working_directory() -> PathBuf {
    SESSIONS
        .with(|sessions| {
            sessions.borrow().active().and_then(|session| {
                session
                    .profile
                    .working_directory
                    .as_ref()
                    .map(PathBuf::from)
            })
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

fn remember_profile(profile: &profile::Profile) {
    let result = RECENT_PROFILES.with(|recent| {
        let _lock = config_lock::ConfigLock::acquire(&workspace_path())?;
        if !settings::Settings::load(&settings_path()).remember_workspace {
            return Ok(());
        }
        let mut recent = recent.borrow_mut();
        *recent = workspace_state::WorkspaceState::load(&workspace_path()).unwrap_or_default();
        let directory = profile
            .working_directory
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok());
        recent.record(&profile.id, directory.as_deref())?;
        recent.save(&workspace_path())
    });
    if let Err(error) = result {
        announce(&format!("Список недавних профилей не сохранён: {error}"));
    }
}

fn open_folder_session(hwnd: HWND) {
    release_terminal_capture();
    match folder_picker::pick(hwnd) {
        Ok(Some(directory)) => show_profile_menu_intent(
            hwnd,
            controls().new_tab,
            false,
            ProfileIntent::NewTab(Some(directory)),
        ),
        Ok(None) => {}
        Err(error) => show_error("Не удалось выбрать рабочую папку", &error),
    }
}

fn open_preferences(hwnd: HWND) {
    cancel_search_feedback();
    release_terminal_capture();
    let current = PREFS_BASELINE
        .with(|baseline| baseline.borrow().clone())
        .unwrap_or_else(|| settings::Settings::load(&settings_path()));
    PREFS_BASELINE.with(|baseline| {
        let mut baseline = baseline.borrow_mut();
        if baseline.is_none() {
            *baseline = Some(current.clone());
        }
    });
    if let Err(error) = preferences::show(hwnd, &current) {
        PREFS_BASELINE.with(|baseline| *baseline.borrow_mut() = None);
        show_error("Не удалось открыть настройки", &error);
    }
}

fn finish_preferences() {
    let Some(outcome) = preferences::take_result() else {
        return;
    };
    let expected = PREFS_BASELINE.with(|baseline| baseline.borrow_mut().take());
    let preferences::Outcome::Saved(next) = outcome else {
        return;
    };
    let Some(expected) = expected else {
        return;
    };
    if let Err(error) = next.save_if_unchanged(&settings_path(), &expected) {
        announce(&format!("Настройки не сохранены: {error}"));
        return;
    }
    let remember = next.remember_workspace;
    SETTINGS.with(|settings| *settings.borrow_mut() = next);
    if !remember {
        let result = RECENT_PROFILES.with(|recent| {
            let _lock = config_lock::ConfigLock::acquire(&workspace_path())?;
            let mut recent = recent.borrow_mut();
            recent.clear();
            recent.save(&workspace_path())
        });
        if let Err(error) = result {
            announce(&format!("Недавние профили не очищены: {error}"));
            return;
        }
    }
    announce("Настройки сохранены");
}

fn open_agent_window(hwnd: HWND) {
    cancel_search_feedback();
    release_terminal_capture();
    let path = CODEX_CLI.with(|cli| {
        cli.borrow()
            .as_ref()
            .filter(|cli| cli.matches(&cli.profile()))
            .map(|cli| PathBuf::from(cli.profile().executable))
    });
    if let Err(error) = agent_window::show(hwnd, path, active_working_directory()) {
        show_error("Не удалось открыть панель Codex", &error);
    }
}

fn finish_agent_window(hwnd: HWND) {
    if let Some(agent_window::Outcome::Fallback(directory)) = agent_window::take_result() {
        let profile =
            CODEX_CLI.with(|cli| cli.borrow().as_ref().map(codex_cli::SupportedCli::profile));
        if let Some(mut profile) = profile {
            profile.working_directory = Some(directory.to_string_lossy().into_owned());
            create_session(hwnd, &profile);
        } else {
            announce("Совместимый Codex CLI не обнаружен. Выберите обычный профиль терминала");
        }
    }
}

fn show_recent_profiles(hwnd: HWND) {
    refresh_profiles();
    release_terminal_capture();
    RECENT_PROFILES.with(|recent| {
        *recent.borrow_mut() =
            workspace_state::WorkspaceState::load(&workspace_path()).unwrap_or_default()
    });
    let recent = RECENT_PROFILES.with(|recent| recent.borrow().recent.clone());
    let profiles: Vec<_> = PROFILES.with(|manager| {
        let manager = manager.borrow();
        recent
            .iter()
            .filter_map(|entry| {
                manager.get(&entry.profile_id).map(|profile| {
                    let mut profile = profile.clone();
                    if let Some(directory) = &entry.working_directory {
                        profile.name = format!("{} — {}", profile.name, directory.display());
                        profile.working_directory = Some(directory.to_string_lossy().into_owned());
                    }
                    profile
                })
            })
            .collect()
    });
    if profiles.is_empty() {
        announce("Нет сохранённых недавних профилей. Их запоминание включается в настройках");
        return;
    }
    unsafe {
        let Ok(menu) =
            profile_menu::ProfileMenu::new(&profiles, false, false, &theme(), current_dpi())
        else {
            return;
        };
        let mut anchor = RECT::default();
        let _ = GetWindowRect(controls().sidebar_profiles, &mut anchor);
        let chosen = TrackPopupMenu(
            menu.handle(),
            TPM_RETURNCMD | TPM_NONOTIFY,
            anchor.left,
            anchor.bottom,
            Some(0),
            hwnd,
            None,
        )
        .0 as usize;
        if let Some(profile) = chosen.checked_sub(1).and_then(|index| profiles.get(index)) {
            create_session(hwnd, profile);
        }
    }
}

fn speech_mode() -> SpeechMode {
    SETTINGS.with(|settings| settings.borrow().speech_mode)
}

fn shortcut_caption(label: &str, action: shortcuts::AppAction) -> String {
    let keys = SETTINGS.with(|settings| settings.borrow().shortcuts.binding_text(action));
    if keys.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({keys})")
    }
}

fn shortcut_help_text() -> String {
    SETTINGS.with(|settings| {
        let settings = settings.borrow();
        let mut text = String::from("ТЕКУЩИЕ СОЧЕТАНИЯ КЛАВИШ\r\n");
        for action in shortcuts::AppAction::ALL {
            let keys = settings.shortcuts.binding_text(action);
            text.push_str(&format!("{}: {}\r\n", action.label(), if keys.is_empty() { "не назначено" } else { &keys }));
        }
        text.push_str("\r\nНиже описаны действия и стандартные сочетания. При переназначении используйте список выше. F1 и Ctrl+Shift+Space сохраняются для выхода из захвата.\r\n\r\n");
        text
    })
}

fn cancel_output_speech() {
    SESSIONS.with(|sessions| {
        if let Some(session) = sessions.borrow_mut().active_mut() {
            let screen = session.terminal.live_snapshot();
            session
                .output_speech
                .baseline(&live_screen_text(&screen), screen.alternate_screen);
        }
    });
}

fn select_speech_mode(command: usize) {
    let mode = match command {
        MENU_SPEECH_ALL => SpeechMode::All,
        MENU_SPEECH_IMPORTANT => SpeechMode::Important,
        _ => SpeechMode::Manual,
    };
    let next = match settings::Settings::update_speech(&settings_path(), mode) {
        Ok(next) => next,
        Err(error) => {
            announce(&format!("Настройка озвучивания не сохранена: {error}"));
            return;
        }
    };
    SETTINGS.with(|settings| *settings.borrow_mut() = next);
    let label = match mode {
        SpeechMode::All => "весь новый вывод",
        SpeechMode::Important => "только важные события",
        SpeechMode::Manual => "чтение вывода вручную",
    };
    announce(&format!("Озвучивание: {label}"));
}

fn live_screen_text(screen: &bks_terminal::TerminalSnapshot) -> String {
    screen
        .cells
        .chunks(screen.cols.max(1))
        .map(|row| {
            let text: String = row.iter().map(|cell| cell.text.as_str()).collect();
            text.trim_end_matches(' ').to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn main_window() -> HWND {
    unsafe { GetParent(controls().output).unwrap_or_default() }
}

fn window_is_foreground(window: HWND) -> bool {
    if window.0.is_null() {
        return false;
    }
    // A native UIA proxy may activate the EDIT itself. Its root still identifies
    // our foreground window; a different window/dialog must remain silent.
    let foreground = unsafe { GetForegroundWindow() };
    foreground == window || unsafe { GetAncestor(foreground, GA_ROOT) == window }
}

fn poll_output_speech(appended: &str) {
    let mode = speech_mode();
    let now = Instant::now();
    let eligible = window_is_foreground(main_window())
        && SPEECH_PAUSE_UNTIL.get().is_none_or(|until| now >= until);
    let message = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let session = sessions.active_mut()?;
        if mode != SpeechMode::All {
            session.output_speech.reset();
            return None;
        }
        let screen = session.terminal.live_snapshot();
        session.output_speech.observe(
            now,
            &live_screen_text(&screen),
            appended,
            screen.alternate_screen,
            eligible,
        );
        session.output_speech.take_due(now, mode)
    });
    if let Some(message) = message {
        // Do not use explicit-action announce(), which resets this engine.
        let _ = controls().announcement.announce(&message);
    }
}

fn session_status(session: &session::TerminalSession) -> String {
    match session.state {
        SessionState::Starting => "Запуск".into(),
        SessionState::Running => "Подключено".into(),
        SessionState::Closing => "Завершение: чтение последних данных".into(),
        SessionState::Exited => format!("Завершено, код {}", session.exit_code.unwrap_or_default()),
        SessionState::Failed => format!(
            "Ошибка: {}",
            session.failure.as_deref().unwrap_or("причина неизвестна")
        ),
    }
}

fn update_connection_status() {
    let (text, connected) = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| {
                (
                    format!("{} · {}", active.title, session_status(active)),
                    matches!(active.state, SessionState::Starting | SessionState::Running),
                )
            })
            .unwrap_or_else(|| ("Нет сессии".to_string(), false))
    });
    CONNECTION_STATUS.with(|status| *status.borrow_mut() = Some(connected));
    if window_text(controls().status_connection) == text {
        return;
    }
    let wide = to_wide(&text);
    unsafe {
        let _ = SetWindowTextW(controls().status_connection, PCWSTR(wide.as_ptr()));
        let _ = InvalidateRect(Some(controls().status_connection), None, true);
    }
}

fn poll_terminal() {
    poll_cli_discovery();
    poll_tool_discovery();
    poll_pending_cli_launch();
    let poll_started = Instant::now();
    let active_id = active_session_id();
    let previous_revision =
        SESSIONS.with(|sessions| sessions.borrow().active().map(|s| s.revision));
    let chunks = SESSIONS.with(|sessions| sessions.borrow_mut().poll());
    let mut speech_appended = String::new();
    trace_slow_phase(
        "poll_model",
        poll_started,
        chunks.iter().map(|(_, text)| text.len()).sum(),
    );
    for (id, text) in chunks {
        if Some(id) == active_id {
            let linear = SESSIONS.with(|sessions| {
                sessions
                    .borrow()
                    .active()
                    .is_some_and(|session| session.speech_linear)
            });
            if linear {
                speech_appended.push_str(&text);
            }
            let removed = SESSIONS.with(|sessions| {
                sessions
                    .borrow_mut()
                    .active_mut()
                    .map_or(0, |active| active.take_journal_trim())
            });
            append_output_text(&text, removed);
        }
    }
    let notices = SESSIONS.with(|sessions| sessions.borrow_mut().take_events());
    for notice in &notices {
        let text = format!("\r\n[{}]\r\n", notice.message);
        let removed = SESSIONS.with(|sessions| {
            let mut sessions = sessions.borrow_mut();
            sessions.get_mut(notice.id).map(|session| {
                session.append_notice(&text);
                if Some(notice.id) == active_id {
                    session.take_journal_trim()
                } else {
                    0
                }
            })
        });
        if Some(notice.id) == active_id {
            if let Some(removed) = removed {
                append_output_text(&text, removed);
            }
        }
        if Some(notice.id) == active_id && speech_mode() != SpeechMode::Manual {
            announce(&notice.message);
        }
    }
    restore_finished_cli_input();
    refresh_tab_status();
    let revision = SESSIONS.with(|sessions| sessions.borrow().active().map(|s| s.revision));
    if revision != previous_revision {
        invalidate_terminal();
    }
    if active_id.is_some() {
        let needs_output_start = with_blocks(|blocks| blocks.needs_output_start());
        if needs_output_start {
            let text = window_text(controls().output);
            with_blocks_mut(|blocks| blocks.text_updated(&text));
        }
    }
    update_vt_output(false);
    update_direct_output(false);
    poll_direct_feedback();
    refresh_search();
    poll_tui_dialog();
    poll_output_speech(&speech_appended);
}

fn restore_finished_cli_input() {
    let focus = unsafe { GetFocus() };
    if !focus.is_invalid() && unsafe { GetAncestor(focus, GA_ROOT) } != main_window() {
        return; // An owned dialog retains its controls until the user returns.
    }
    let ready = SESSIONS.with(|sessions| {
        sessions.borrow().active().is_some_and(|active| {
            active.cli_return_ready()
                && !active.search_open
                && (!active.direct_input || !active.reading)
        })
    });
    if !ready {
        return;
    }
    let was_direct = direct_input_active();
    if was_direct {
        save_active_view();
    }
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            if was_direct {
                active.finish_cli_input();
            } else {
                active.cancel_cli_input();
            }
        }
    });
    if !was_direct {
        return;
    }
    refresh_layout(main_window());
    if [controls().direct_input, controls().terminal].contains(&focus)
        && window_is_foreground(main_window())
    {
        unsafe {
            let _ = SetFocus(Some(controls().input));
        }
        announce("CLI завершён. Ввод команды оболочки");
    }
}

/// Explicit, numeric-only diagnostics for investigating output stalls.
/// No terminal text, commands, environment values or credentials are recorded.
fn trace_slow_phase(phase: &str, started: Instant, size: usize) {
    let elapsed = started.elapsed().as_millis();
    if elapsed < 100 {
        return;
    }
    let Some(path) = std::env::var_os("BKS_TERM_PERF_LOG") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{phase},{elapsed},{size}");
    }
}

fn reader_viewport_offset(output: HWND) -> u32 {
    unsafe {
        let first = SendMessageW(output, EM_GETFIRSTVISIBLELINE, None, None)
            .0
            .max(0);
        SendMessageW(output, EM_LINEINDEX, Some(WPARAM(first as usize)), None)
            .0
            .max(0) as u32
    }
}

fn reader_at_bottom(output: HWND) -> bool {
    let mut info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        ..Default::default()
    };
    unsafe {
        GetScrollInfo(output, SB_VERT, &mut info).is_err()
            || info.nPos
                >= info
                    .nMax
                    .saturating_sub(info.nPage.saturating_sub(1) as i32)
    }
}

fn restore_native_reading(output: HWND, selection: (u32, u32), viewport: u32, follow: bool) {
    unsafe {
        let length = GetWindowTextLengthW(output).max(0) as u32;
        SendMessageW(
            output,
            EM_SETSEL,
            Some(WPARAM(selection.0.min(length) as usize)),
            Some(LPARAM(selection.1.min(length) as isize)),
        );
        if follow {
            SendMessageW(output, EM_SCROLLCARET, None, None);
        } else {
            let target = SendMessageW(
                output,
                EM_LINEFROMCHAR,
                Some(WPARAM(viewport as usize)),
                None,
            )
            .0;
            let current = SendMessageW(output, EM_GETFIRSTVISIBLELINE, None, None).0;
            SendMessageW(output, EM_LINESCROLL, None, Some(LPARAM(target - current)));
        }
    }
}

fn capture_vt_reading_position() -> bool {
    if !reading_active()
        || !vt_reading_active()
        || !unsafe { IsWindowVisible(controls().vt_output).as_bool() }
    {
        return false;
    }
    let Some((id, _)) = VT_DISPLAY_KEY.get() else {
        return false;
    };
    if active_session_id() != Some(id) {
        return false;
    }
    let document = VT_DISPLAY_DOCUMENT.with(|document| document.borrow().clone());
    let Some(document) = document else {
        return false;
    };
    let output = controls().vt_output;
    let selection = edit_selection(output);
    let viewport = reader_viewport_offset(output);
    let at_bottom = reader_at_bottom(output);
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active
                .vt_state
                .capture(&document, selection, viewport, at_bottom);
        }
    });
    true
}

/// The focused native EDIT exposes the live VT text/caret while its keys go to
/// ConPTY. Its state is separate from the user's full-screen reading bookmark.
fn update_direct_output(force: bool) {
    if !direct_input_active() || reading_active() {
        return;
    }
    // EDIT may recreate/show its native caret after layout or text messages.
    // Keep the caret object for accessibility while the VT renderer draws it.
    system_caret::hide_visible(controls().direct_input);
    let next = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        Some((
            active.id,
            active.reading_document_snapshot(),
            active.terminal.live_snapshot(),
        ))
    });
    let Some((id, document, mut frame)) = next else {
        return;
    };
    let echo = DIRECT_ECHO.with(|echo| {
        let mut echo = echo.borrow_mut();
        if direct_input_has_focus() && window_is_foreground(main_window()) {
            echo.observe(&frame, Instant::now())
        } else {
            echo.cancel();
            None
        }
    });
    let confirmed = DIRECT_ECHO.with(|echo| echo.borrow().last_confirmed(&frame));
    frame.cursor = direct_accessibility::reading_cursor(&frame, confirmed);
    let caret = direct_accessibility::cursor_offset(&document, &frame);
    let echo_message = echo.map(|echo| direct_accessibility::typing_message(&echo.text));
    let patch = DIRECT_DOCUMENT.with(|previous| {
        let previous = previous.borrow();
        let text = previous
            .as_ref()
            .map_or("", |(_, document)| document.text.as_str());
        native_text::difference(text, &document.text)
    });
    let output = controls().direct_input;
    let caret_changed = edit_selection(output) != (caret, caret);
    if !force && patch.is_none() && !caret_changed {
        DIRECT_DOCUMENT.with(|previous| *previous.borrow_mut() = Some((id, document)));
        if let Some(message) = echo_message {
            announce(&message);
        }
        return;
    }
    let visible = unsafe { IsWindowVisible(output).as_bool() };
    unsafe {
        if visible && patch.is_some() {
            SendMessageW(output, WM_SETREDRAW, Some(WPARAM(0)), None);
        }
        if let Some(patch) = &patch {
            SendMessageW(
                output,
                EM_SETSEL,
                Some(WPARAM(patch.start as usize)),
                Some(LPARAM(patch.end as isize)),
            );
            let replacement = to_wide(&patch.replacement);
            SendMessageW(
                output,
                EM_REPLACESEL,
                Some(WPARAM(0)),
                Some(LPARAM(replacement.as_ptr() as isize)),
            );
        }
        if caret_changed || patch.is_some() {
            SendMessageW(
                output,
                EM_SETSEL,
                Some(WPARAM(caret as usize)),
                Some(LPARAM(caret as isize)),
            );
        }
        if visible && patch.is_some() {
            SendMessageW(output, WM_SETREDRAW, Some(WPARAM(1)), None);
            let _ = InvalidateRect(Some(output), None, false);
        }
        // EDIT does not reliably scroll while redraw is disabled. Keep its
        // native text viewport/caret aligned after enabling painting again.
        SendMessageW(output, EM_SCROLLCARET, None, None);
    }
    DIRECT_DOCUMENT.with(|previous| *previous.borrow_mut() = Some((id, document)));
    // Text/caret notifications must precede typing feedback, so a native edit
    // update cannot immediately replace the just-announced character.
    if let Some(message) = echo_message {
        announce(&message);
    }
}

fn begin_direct_feedback() {
    if !direct_input_has_focus() || !window_is_foreground(main_window()) {
        return;
    }
    // Navigation can move a hidden cursor without changing the old row.
    // A confirmed insertion is no longer evidence of the current input point.
    DIRECT_ECHO.with(|echo| echo.borrow_mut().cancel());
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow().active() {
            DIRECT_FEEDBACK.with(|feedback| {
                feedback
                    .borrow_mut()
                    .begin(&active.terminal.live_snapshot(), Instant::now())
            });
        }
    });
}

fn poll_direct_feedback() {
    if !direct_input_has_focus() || !window_is_foreground(main_window()) {
        DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
        return;
    }
    let message = SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let active = sessions.active()?;
        DIRECT_FEEDBACK.with(|feedback| {
            feedback
                .borrow_mut()
                .observe(&active.terminal.live_snapshot(), Instant::now())
        })
    });
    if let Some(message) = message {
        announce(&message);
    }
}

fn update_vt_output(force: bool) {
    if !reading_active() || !vt_reading_active() {
        return;
    }
    if !force
        && SESSIONS.with(|sessions| {
            sessions
                .borrow()
                .active()
                .is_some_and(|active| active.search_open && active.search_focus)
        })
    {
        // Avoid unfocused EDIT selection/text events while the user types a
        // query. Enter or focusing the reader refreshes it from the live model.
        return;
    }
    let pending = VT_RESTORE_PENDING.replace(false);
    let key = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| (active.id, active.terminal.reading_revision()))
    });
    if !force && !pending && VT_DISPLAY_KEY.get() == key {
        return;
    }
    if !force && !pending {
        capture_vt_reading_position();
    }
    let update = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let active = sessions.active_mut()?;
        let document = active.reading_document_snapshot();
        let restored = active.vt_state.restore(&document);
        Some((active.id, document, restored))
    });
    let Some((id, document, restored)) = update else {
        return;
    };
    let old = VT_DISPLAY_DOCUMENT.with(|old| old.borrow().clone());
    let old_text = old.as_ref().map_or("", |document| document.text.as_str());
    let patch = native_text::difference(old_text, &document.text);
    let output = controls().vt_output;
    let visible = unsafe { IsWindowVisible(output).as_bool() };
    unsafe {
        if visible {
            SendMessageW(output, WM_SETREDRAW, Some(WPARAM(0)), None);
        }
        if let Some(patch) = patch {
            SendMessageW(
                output,
                EM_SETSEL,
                Some(WPARAM(patch.start as usize)),
                Some(LPARAM(patch.end as isize)),
            );
            let replacement = to_wide(&patch.replacement);
            SendMessageW(
                output,
                EM_REPLACESEL,
                Some(WPARAM(0)),
                Some(LPARAM(replacement.as_ptr() as isize)),
            );
        }
        restore_native_reading(
            output,
            restored.selection,
            restored.viewport_offset,
            restored.follow,
        );
        if visible {
            SendMessageW(output, WM_SETREDRAW, Some(WPARAM(1)), None);
            let _ = InvalidateRect(Some(output), None, false);
        }
    }
    VT_DISPLAY_KEY.set(Some((id, document.revision)));
    VT_DISPLAY_DOCUMENT.with(|current| *current.borrow_mut() = Some(document));
}

fn set_output_text(text: &str) {
    let text = to_wide(text);
    unsafe {
        let _ = SetWindowTextW(controls().output, PCWSTR(text.as_ptr()));
    }
}

fn append_output_text(text: &str, removed: u32) {
    if text.is_empty() && removed == 0 {
        return;
    }
    let output = controls().output;
    let visible = unsafe { IsWindowVisible(output).as_bool() };
    let retained = (removed != 0)
        .then(|| {
            SESSIONS.with(|sessions| {
                sessions
                    .borrow()
                    .active()
                    .map(|active| active.buffer().to_owned())
            })
        })
        .flatten();
    let text = to_wide(retained.as_deref().unwrap_or(text));
    unsafe {
        let length = GetWindowTextLengthW(output).max(0) as u32;
        let mut selection_start = 0_u32;
        let mut selection_end = 0_u32;
        SendMessageW(
            output,
            EM_GETSEL,
            Some(WPARAM(&mut selection_start as *mut u32 as usize)),
            Some(LPARAM(&mut selection_end as *mut u32 as isize)),
        );
        let first_visible = reader_viewport_offset(output);
        let follow_output =
            selection_start == length && selection_end == length && reader_at_bottom(output);

        if visible {
            SendMessageW(output, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        if retained.is_some() {
            // Classic EDIT prefix deletion gets very slow for many lines.
            // Rebuild only on retention, keeping HWND/provider and restoring
            // the same rebased UTF-16 selection and viewport below.
            let trim_started = Instant::now();
            let _ = SetWindowTextW(output, PCWSTR(text.as_ptr()));
            trace_slow_phase("native_trim", trim_started, text.len());
        } else {
            SendMessageW(
                output,
                EM_SETSEL,
                Some(WPARAM(length as usize)),
                Some(LPARAM(length as isize)),
            );
            let append_started = Instant::now();
            SendMessageW(
                output,
                EM_REPLACESEL,
                Some(WPARAM(0)),
                Some(LPARAM(text.as_ptr() as isize)),
            );
            trace_slow_phase("native_append", append_started, text.len());
        }
        let final_length = GetWindowTextLengthW(output).max(0) as u32;
        let selection = if follow_output {
            (final_length, final_length)
        } else {
            (
                selection_start.saturating_sub(removed),
                selection_end.saturating_sub(removed),
            )
        };
        let restore_started = Instant::now();
        restore_native_reading(
            output,
            selection,
            first_visible.saturating_sub(removed),
            follow_output,
        );
        trace_slow_phase("native_restore", restore_started, final_length as usize);
        if visible {
            SendMessageW(output, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
            let _ = InvalidateRect(Some(output), None, false);
        }
    }
}

fn execute_input() {
    let input = controls().input;
    let text = window_text(input);
    poll_terminal();
    let can_execute = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| active.can_submit_command())
    });
    if text.trim().is_empty() {
        return;
    }
    cancel_search_feedback();
    match can_execute {
        Some(true) => {}
        Some(false) => {
            let message = SESSIONS.with(|sessions| {
                sessions
                    .borrow()
                    .active()
                    .map(|session| {
                        if matches!(
                            session.state,
                            SessionState::Starting | SessionState::Running
                        ) && session.is_alive()
                        {
                            "Оболочка ещё выполняет команду".into()
                        } else {
                            format!(
                                "{}. Перезапуск доступен в меню профилей",
                                session_status(session)
                            )
                        }
                    })
                    .unwrap_or_else(|| "Нет активной сессии".into())
            });
            announce(&message);
            return;
        }
        None => {
            announce("Нет активной сессии");
            return;
        }
    }
    let result = SESSIONS.with(|sessions| {
        sessions
            .borrow_mut()
            .active_mut()
            .ok_or_else(|| "Нет активной сессии.".to_string())
            .and_then(|active| {
                active
                    .submit_command(&text)
                    .map_err(|error| error.to_string())
            })
    });
    match result {
        Ok(()) => {
            unsafe {
                let _ = SetWindowTextW(input, w!(""));
            }
            let interactive = SESSIONS.with(|sessions| {
                sessions
                    .borrow()
                    .active()
                    .is_some_and(|active| active.auto_cli_input)
            });
            if interactive {
                set_terminal_capture(main_window(), true);
            } else {
                unsafe {
                    let _ = SetFocus(Some(input));
                }
                announce("Команда отправлена");
            }
        }
        Err(error) => report_terminal_error("Ошибка отправки команды", &error),
    }
}

fn window_text(window: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(window);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = GetWindowTextW(window, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

fn handle_app_shortcut(hwnd: HWND, key: usize) -> bool {
    use shortcuts::AppAction;
    let modifiers = shortcuts::Modifiers {
        control: unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 },
        shift: unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 },
        alt: unsafe { GetKeyState(VK_MENU.0 as i32) < 0 },
    };
    if key == 0x41 && modifiers.control && !modifiers.shift && !modifiers.alt {
        let focus = unsafe { GetFocus() };
        if [
            controls().input,
            controls().output,
            controls().vt_output,
            controls().search.query,
        ]
        .contains(&focus)
        {
            unsafe {
                SendMessageW(focus, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            }
            return true;
        }
    }
    if key == 0x73 && modifiers.alt && !modifiers.control && !modifiers.shift {
        unsafe {
            SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM(SC_CLOSE as usize)), None);
        }
        return true;
    }
    let action = SETTINGS.with(|settings| settings.borrow().shortcuts.dispatch(key, modifiers));
    let Some(action) = action else {
        return false;
    };
    match action {
        AppAction::NewTab => show_profile_menu_at(hwnd, controls().new_tab, false),
        AppAction::Profiles => show_profile_menu(hwnd),
        AppAction::CloseTab => {
            if let Some(id) = active_session_id() {
                close_session(hwnd, id);
            }
        }
        AppAction::NextTab => activate_adjacent_session(hwnd, false),
        AppAction::PreviousTab => activate_adjacent_session(hwnd, true),
        AppAction::ToggleReading => toggle_reading_focus(hwnd),
        AppAction::ToggleCapture => toggle_terminal_capture(hwnd),
        AppAction::ReadCurrentInput => read_current_input(),
        AppAction::ReadNewOutput => read_new_output(hwnd),
        AppAction::OpenTuiDialog => open_tui_dialog(hwnd),
        AppAction::OpenAgentPanel => open_agent_window(hwnd),
        AppAction::Search => open_search(hwnd),
        AppAction::NextMatch | AppAction::PreviousMatch => {
            if !search_active() {
                return false;
            }
            navigate_search(hwnd, action == AppAction::PreviousMatch);
        }
        AppAction::PreviousBlock | AppAction::NextBlock => {
            let focus = unsafe { GetFocus() };
            let previous = action == AppAction::PreviousBlock;
            if focus == controls().search.query {
                return true;
            }
            if direct_input_active()
                && (direct_input_has_focus() || (reading_active() && vt_reading_active()))
            {
                navigate_vt_reading(hwnd, previous);
            } else if !focus_adjacent_output_block(previous) {
                if focus == controls().output {
                    // Without command blocks, the native journal can still
                    // handle Ctrl+arrows as ordinary text navigation.
                    return false;
                }
                let position = if previous {
                    unsafe { GetWindowTextLengthW(controls().output).max(0) as u32 }
                } else {
                    0
                };
                focus_output_position(position);
            }
        }
        AppAction::CopyOutput => {
            if !copy_current_output_block() {
                if !direct_input_has_focus() {
                    return false;
                }
                if !copy_terminal_selection() && focus_last_output_block() {
                    copy_current_output_block();
                }
            }
        }
        AppAction::Paste => {
            if !direct_input_has_focus() {
                return false;
            }
            paste_terminal();
        }
        AppAction::ScrollToBottom => {
            if !direct_input_has_focus() {
                return false;
            }
            SESSIONS.with(|sessions| {
                if let Some(active) = sessions.borrow_mut().active_mut() {
                    active.terminal.scroll_to_bottom();
                    active.screen_selection = None;
                }
            });
            invalidate_terminal();
        }
        AppAction::Help => {
            release_terminal_capture();
            if let Err(error) = help::show(hwnd) {
                show_error("Не удалось открыть справку", &error);
            }
        }
        AppAction::NewWindow => {
            show_profile_menu_intent(hwnd, controls().new_tab, false, ProfileIntent::NewWindow)
        }
        AppAction::OpenInFolder => open_folder_session(hwnd),
        AppAction::Preferences => open_preferences(hwnd),
        AppAction::RestoreProfile => show_recent_profiles(hwnd),
        AppAction::Tab1
        | AppAction::Tab2
        | AppAction::Tab3
        | AppAction::Tab4
        | AppAction::Tab5
        | AppAction::Tab6
        | AppAction::Tab7
        | AppAction::Tab8
        | AppAction::Tab9 => {
            let index = match action {
                AppAction::Tab1 => 0,
                AppAction::Tab2 => 1,
                AppAction::Tab3 => 2,
                AppAction::Tab4 => 3,
                AppAction::Tab5 => 4,
                AppAction::Tab6 => 5,
                AppAction::Tab7 => 6,
                AppAction::Tab8 => 7,
                AppAction::Tab9 => 8,
                _ => unreachable!(),
            };
            activate_session_index(hwnd, index);
        }
    }
    true
}

fn activate_adjacent_session(hwnd: HWND, previous: bool) {
    let ids = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>()
    });
    if ids.is_empty() {
        return;
    }
    let active_index = active_session_id()
        .and_then(|active| ids.iter().position(|id| *id == active))
        .unwrap_or(0);
    let next = if previous {
        (active_index + ids.len() - 1) % ids.len()
    } else {
        (active_index + 1) % ids.len()
    };
    activate_session(hwnd, ids[next]);
}

fn activate_session_index(hwnd: HWND, index: usize) {
    let id = SESSIONS.with(|sessions| sessions.borrow().sessions.get(index).map(|s| s.id));
    if let Some(id) = id {
        activate_session(hwnd, id);
    }
}

fn handle_direct_key(key: usize, key_data: isize) -> bool {
    let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let alt = unsafe { GetKeyState(VK_MENU.0 as i32) < 0 };
    let modes = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .map(|active| active.terminal.modes())
            .unwrap_or_default()
    });
    if modes.application_keypad && !shift && !control && !alt {
        if let Some(bytes) = application_keypad_bytes(key, key_data & (1 << 24) != 0) {
            write_active_input(bytes);
            return true;
        }
    }
    if shift && key == VK_INSERT.0 as usize && !control && !alt {
        paste_terminal();
        return true;
    }
    if control && !alt && !matches!(key, 0x43 | 0x44 | 0x5a) {
        if let Some(code) = control_char(key, shift) {
            begin_direct_feedback();
            write_active_input(&[code]);
            return true;
        }
    }
    let special = match key {
        key if key == VK_RETURN.0 as usize => SpecialKey::Enter,
        key if key == VK_BACK.0 as usize => SpecialKey::Backspace,
        key if key == VK_TAB.0 as usize => SpecialKey::Tab,
        key if key == VK_ESCAPE.0 as usize => SpecialKey::Escape,
        key if key == VK_UP.0 as usize => SpecialKey::Up,
        key if key == VK_DOWN.0 as usize => SpecialKey::Down,
        key if key == VK_LEFT.0 as usize => SpecialKey::Left,
        key if key == VK_RIGHT.0 as usize => SpecialKey::Right,
        key if key == VK_HOME.0 as usize => SpecialKey::Home,
        key if key == VK_END.0 as usize => SpecialKey::End,
        key if key == VK_DELETE.0 as usize => SpecialKey::Delete,
        key if key == VK_INSERT.0 as usize => SpecialKey::Insert,
        key if key == VK_PRIOR.0 as usize => SpecialKey::PageUp,
        key if key == VK_NEXT.0 as usize => SpecialKey::PageDown,
        0x70 => SpecialKey::F1,
        0x71 => SpecialKey::F2,
        0x72 => SpecialKey::F3,
        0x73 => SpecialKey::F4,
        0x74 => SpecialKey::F5,
        0x75 => SpecialKey::F6,
        0x76 => SpecialKey::F7,
        0x77 => SpecialKey::F8,
        0x78 => SpecialKey::F9,
        0x79 => SpecialKey::F10,
        0x7a => SpecialKey::F11,
        0x7b => SpecialKey::F12,
        0x43 if control && !alt => SpecialKey::CtrlC,
        0x44 if control && !alt => SpecialKey::CtrlD,
        0x5a if control && !alt => SpecialKey::CtrlZ,
        _ => return false,
    };
    if special == SpecialKey::Enter && !control && !alt {
        SESSIONS.with(|sessions| {
            if let Some(active) = sessions.borrow_mut().active_mut() {
                let document = active.reading_document_snapshot();
                active.reply_anchor.submitted(&document);
            }
        });
    }
    begin_direct_feedback();
    write_active_input(&encode_key(
        special,
        shift,
        alt,
        control,
        modes.application_cursor,
    ));
    true
}

fn handle_direct_char(unit: u16) {
    if let Some(bytes) = UTF16_ENCODER.with(|encoder| encoder.borrow_mut().push(unit)) {
        let alt = unsafe { GetKeyState(VK_MENU.0 as i32) < 0 };
        let control = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
        if alt && !control {
            let mut prefixed = vec![0x1b];
            prefixed.extend(bytes);
            write_active_input(&prefixed);
        } else {
            if !control || alt {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    SESSIONS.with(|sessions| {
                        if let Some(active) = sessions.borrow().active() {
                            DIRECT_ECHO.with(|echo| {
                                echo.borrow_mut().typed(
                                    &active.terminal.live_snapshot(),
                                    text,
                                    Instant::now(),
                                );
                            });
                        }
                    });
                    DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
                }
            }
            write_active_input(&bytes);
        }
    }
}

fn write_active_input(data: &[u8]) {
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.terminal.scroll_to_bottom();
            active.screen_selection = None;
        }
    });
    invalidate_terminal();
    let result = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .active()
            .ok_or_else(|| "Нет активной сессии.".to_string())
            .and_then(|active| active.write(data).map_err(|error| error.to_string()))
    });
    if let Err(error) = result {
        DIRECT_FEEDBACK.with(|feedback| feedback.borrow_mut().cancel());
        DIRECT_ECHO.with(|echo| echo.borrow_mut().cancel());
        report_terminal_error("Ошибка прямого ввода", &error);
    }
}

fn focus_last_output_block() -> bool {
    let anchor = with_blocks(|blocks| blocks.last());
    anchor.is_some_and(focus_output_position)
}

fn focus_adjacent_output_block(previous: bool) -> bool {
    let output = controls().output;
    let in_output = unsafe { GetFocus() == output };
    let position = in_output.then(output_selection).map(|(start, _)| start);
    let anchor = with_blocks(|blocks| blocks.navigate(previous, position));
    anchor.is_some_and(focus_output_position)
}

fn copy_current_output_block() -> bool {
    let output = controls().output;
    if unsafe { GetFocus() != output } {
        return false;
    }
    let (position, _) = output_selection();
    let range = with_blocks(|blocks| blocks.current(position));
    let Some((start, Some(end))) = range else {
        return false;
    };
    if end <= start {
        return false;
    }
    unsafe {
        SendMessageW(
            output,
            EM_SETSEL,
            Some(WPARAM(start as usize)),
            Some(LPARAM(end as isize)),
        );
        SendMessageW(output, WM_COPY, None, None);
    }
    true
}

fn output_selection() -> (u32, u32) {
    edit_selection(controls().output)
}

fn edit_selection(output: HWND) -> (u32, u32) {
    let mut start = 0_u32;
    let mut end = 0_u32;
    unsafe {
        SendMessageW(
            output,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
    }
    (start, end)
}

fn focus_output_position(position: u32) -> bool {
    let output = controls().output;
    let hwnd = unsafe { GetParent(output).unwrap_or_default() };
    close_search(hwnd, false);
    save_active_view();
    SESSIONS.with(|sessions| {
        if let Some(active) = sessions.borrow_mut().active_mut() {
            active.vt_reading = false;
        }
    });
    set_reading(hwnd, true);
    unsafe {
        if position as i32 > GetWindowTextLengthW(output) {
            return false;
        }
        let _ = SetFocus(Some(output));
        SendMessageW(
            output,
            EM_SETSEL,
            Some(WPARAM(position as usize)),
            Some(LPARAM(position as isize)),
        );
        SendMessageW(output, EM_SCROLLCARET, None, None);
    }
    true
}

fn insert_input_newline() {
    let newline = to_wide("\r\n");
    unsafe {
        SendMessageW(
            controls().input,
            EM_REPLACESEL,
            Some(WPARAM(1)),
            Some(LPARAM(newline.as_ptr() as isize)),
        );
        SendMessageW(controls().input, EM_SCROLLCARET, None, None);
    }
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn show_error(context: &str, error: &Error) {
    let text = to_wide(&format!("{context}\r\n{error}"));
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("BKS Term"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn report_terminal_error(context: &str, error: &str) {
    let notice = format!("\r\n[{context}] {error}\r\n");
    let removed = SESSIONS.with(|sessions| {
        sessions.borrow_mut().active_mut().map_or(0, |active| {
            active.append_notice(&notice);
            active.take_journal_trim()
        })
    });
    append_output_text(&notice, removed);
    let text = to_wide(&format!("{context}\r\n{error}"));
    let owner = unsafe { GetParent(controls().terminal).ok() };
    unsafe {
        MessageBoxW(
            owner,
            PCWSTR(text.as_ptr()),
            w!("BKS Term"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod startup_cli_tests {
    use super::*;

    fn profile(id: &str, directory: Option<&str>) -> profile::Profile {
        profile::Profile {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: profile::ProfileKind::Local,
            executable: r"C:\Tools\cli.exe".to_owned(),
            args: Vec::new(),
            working_directory: directory.map(str::to_owned),
        }
    }

    #[test]
    fn ordinary_start_and_non_cli_profiles_never_wait_for_discovery() {
        let mut pending = PendingCliLaunch::default();
        for id in [None, Some("windows-powershell"), Some("ssh-custom")] {
            assert!(!pending.defer(id, None));
            assert!(pending.resolve(&[], false).is_none());
        }
        assert!(pending.defer(Some(codex_cli::PROFILE_ID), None));
    }

    #[test]
    fn late_catalog_launches_once_with_the_original_unicode_directory() {
        let mut pending = PendingCliLaunch::default();
        let directory = PathBuf::from(r"C:\Работа\目录 with spaces");
        assert!(pending.defer(Some("cli-kimi"), Some(directory.clone())));
        assert!(pending
            .resolve(&[profile("cli-codex", None)], false)
            .is_none());
        let catalog = [profile("cli-kimi", Some(r"C:\Other default"))];
        // The matching catalog can resolve while the other worker is pending.
        let launch = pending.resolve(&catalog, false).unwrap().unwrap();
        assert_eq!(launch.id, "cli-kimi");
        assert_eq!(launch.working_directory.as_deref(), directory.to_str());
        assert!(pending.resolve(&catalog, true).is_none());
    }

    #[test]
    fn missing_runtime_profile_errors_once_only_after_discovery_finishes() {
        let mut pending = PendingCliLaunch::default();
        assert!(pending.defer(Some("cli-gemini-ax"), None));
        let other = [
            profile("cli-gemini", None),
            profile("windows-powershell", None),
        ];
        assert!(pending.resolve(&other, false).is_none());
        let error = pending.resolve(&other, true).unwrap().unwrap_err();
        assert!(error.contains("cli-gemini-ax"));
        assert!(pending.resolve(&other, true).is_none());
    }

    #[test]
    fn a_user_session_action_cancels_the_deferred_auto_launch() {
        let mut pending = PendingCliLaunch::default();
        assert!(pending.defer(Some("cli-claude"), Some(PathBuf::from(r"C:\Original"))));
        pending.cancel();
        assert!(pending
            .resolve(&[profile("cli-claude", None)], true)
            .is_none());
    }

    #[test]
    fn no_cwd_override_preserves_the_discovered_profile_directory() {
        let mut pending = PendingCliLaunch::default();
        assert!(pending.defer(Some("cli-codex"), None));
        let catalog = [profile("cli-codex", Some(r"C:\Discovered"))];
        assert_eq!(
            pending.resolve(&catalog, true).unwrap().unwrap(),
            catalog[0]
        );
    }
}

#[cfg(test)]
mod tui_adapter_tests {
    use super::*;

    #[test]
    fn overview_keeps_full_context_and_source_rows_without_mapped_actions() {
        let mut model = bks_terminal::TerminalModel::new(80, 8);
        model.push_bytes(
            b"Permission details must remain readable\r\n  First\r\n> Second\r\n\r\nUse arrows to choose",
        );
        let view = tui_view_data("Unknown CLI", &model.live_snapshot());
        assert!(!view.mapped);
        assert!(view
            .options
            .iter()
            .any(|line| line.contains("Строка 2: First")));
        assert!(view
            .options
            .iter()
            .any(|line| line.contains("Строка 3: > Second")));
        assert_eq!(view.selected, 2);
        assert!(view
            .context
            .contains("Permission details must remain readable"));
        assert!(view.context.contains("Use arrows to choose"));
    }

    fn source() -> tui_dialog::Source<'static> {
        tui_dialog::Source {
            session_id: 11,
            screen_revision: 24,
            input_revision: 0,
            cli_version: tui_dialog::SUPPORTED_CODEX_VERSION,
            direct_codex_profile: true,
            process_alive: true,
            active_session: true,
        }
    }

    #[test]
    fn adapter_source_keys_require_checked_version_and_live_active_owner() {
        assert!(TuiSourceKey::from_source(source()).is_some());
        for changed in [
            tui_dialog::Source {
                cli_version: "",
                ..source()
            },
            tui_dialog::Source {
                cli_version: "0.165.0",
                ..source()
            },
            tui_dialog::Source {
                direct_codex_profile: false,
                ..source()
            },
            tui_dialog::Source {
                process_alive: false,
                ..source()
            },
            tui_dialog::Source {
                active_session: false,
                ..source()
            },
        ] {
            assert!(TuiSourceKey::from_source(changed).is_none());
        }
    }

    #[test]
    fn passive_analysis_requires_stability_and_does_not_rescan_identical_revision() {
        let key = TuiSourceKey::from_source(source()).unwrap();
        let now = Instant::now();
        let mut state = TuiAdapterState {
            ready: true,
            ..TuiAdapterState::default()
        };
        state.observe(Some(&key), true, now);
        assert!(!state.can_submit(&key, now));
        assert!(state.can_submit(&key, now + TUI_STABLE_FOR));
        state.observe(Some(&key), false, now + TUI_STABLE_FOR);
        assert!(!state.can_submit(&key, now + TUI_STABLE_FOR));
        let settled = now + Duration::from_secs(1);
        state.observe(Some(&key), true, settled);
        assert!(!state.can_submit(&key, settled));
        assert!(state.can_submit(&key, settled + TUI_STABLE_FOR));
        state.last_submitted = Some(key.clone());
        state.cached = Some(CachedTuiAnalysis {
            key: key.clone(),
            dialog: None,
        });
        assert!(!state.can_submit(&key, settled + Duration::from_secs(10)));
        let mut changed = key;
        changed.revision += 1;
        state.observe(Some(&changed), true, settled);
        assert!(state.cached.is_none());
        assert!(!state.can_submit(&changed, settled));
        assert!(state.can_submit(&changed, settled + TUI_STABLE_FOR));
    }

    #[test]
    fn adapter_failure_stays_disabled_until_an_explicit_restart() {
        let key = TuiSourceKey::from_source(source()).unwrap();
        let now = Instant::now();
        let mut state = TuiAdapterState {
            ready: true,
            next_id: 17,
            ..TuiAdapterState::default()
        };
        state.observe(Some(&key), true, now);
        assert!(state.disable());
        assert!(!state.disable());
        state.observe(Some(&key), true, now + Duration::from_secs(10));
        assert!(state.disabled);
        assert!(!state.can_submit(&key, now + Duration::from_secs(11)));
        state.explicit_restart();
        assert!(!state.disabled);
        assert!(!state.ready);
        assert_eq!(state.next_id, 17);
        assert!(state.observed.is_none());
        assert!(state.cached.is_none());
    }
}
