//! Native profile popup with dark owner drawing and standard menu semantics.
//!
//! Keep `ProfileMenu` alive until `TrackPopupMenu` returns. Forward
//! `WM_MEASUREITEM`, `WM_DRAWITEM`, and `WM_MENUCHAR` to the helpers below;
//! unrecognized messages must continue through the normal window procedure.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::profile::Profile;
use crate::theme::Theme;
use windows::core::{Error, Result, PWSTR};
use windows::Win32::Foundation::{
    COLORREF, E_INVALIDARG, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, GetDC, GetTextExtentPoint32W,
    IntersectClipRect, Polyline, ReleaseDC, RestoreDC, SaveDC, SelectObject, SetBkMode,
    SetTextColor, DT_END_ELLIPSIS, DT_HIDEPREFIX, DT_LEFT, DT_SINGLELINE, DT_VCENTER, HBRUSH, HDC,
    HFONT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::UI::Accessibility::{MSAAMENUINFO, MSAA_MENU_SIG};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_CHECKED, ODS_DISABLED, ODS_GRAYED, ODS_HOTLIGHT,
    ODS_NOACCEL, ODS_SELECTED, ODT_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, InsertMenuItemW, SetMenuInfo,
    HMENU, MENUINFO, MENUITEMINFOW, MFS_CHECKED, MFS_DISABLED, MFS_ENABLED, MFS_HILITE,
    MFT_OWNERDRAW, MFT_SEPARATOR, MIIM_DATA, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING,
    MIM_BACKGROUND, MIM_STYLE, MNC_EXECUTE, MNC_IGNORE, MNC_SELECT, MNS_NOCHECK,
};

const ROW_HEIGHT: i32 = 32;
const SEPARATOR_HEIGHT: i32 = 9;
const MIN_WIDTH: i32 = 240;
const PADDING: i32 = 12;
const ICON_SIZE: i32 = 16;
const ICON_GAP: i32 = 10;
const CHECK_GAP: i32 = 12;

thread_local! {
    // Check ownership before dereferencing itemData from any owner-draw control.
    // Both menu ownership and callbacks stay on the creating UI thread.
    static MENUS: RefCell<HashMap<usize, Vec<usize>>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Profile,
    InputMode,
    Separator,
}

#[repr(C)]
struct MenuEntry {
    // Documented MSAA contract: this header must be the first field of itemData.
    // https://learn.microsoft.com/windows/win32/api/oleacc/ns-oleacc-msaamenuinfo
    msaa: MSAAMENUINFO,
    _accessible_text: Vec<u16>,
    native_text: Vec<u16>,
    label: String,
    kind: EntryKind,
    text_width: i32,
    measured_dpi: u32,
}

impl MenuEntry {
    fn new(label: &str, kind: EntryKind, theme: &Theme, dpi: u32) -> Result<Box<Self>> {
        // Embedded NULs have always ended the native menu caption.
        let label = label.split('\0').next().unwrap_or_default().to_owned();
        let mut accessible_text: Vec<u16> = visible_caption(&label).encode_utf16().collect();
        let length = u32::try_from(accessible_text.len()).map_err(|_| Error::from(E_INVALIDARG))?;
        let text_width = if theme.high_contrast || kind == EntryKind::Separator {
            0
        } else {
            text_width(&accessible_text, theme.ui_font)?
        };
        accessible_text.push(0);
        let native_text = label.encode_utf16().chain([0]).collect();
        let msaa = MSAAMENUINFO {
            dwMSAASignature: MSAA_MENU_SIG as u32,
            cchWText: length,
            pszWText: PWSTR(accessible_text.as_mut_ptr()),
        };
        Ok(Box::new(Self {
            msaa,
            _accessible_text: accessible_text,
            native_text,
            label,
            kind,
            text_width,
            measured_dpi: if dpi == 0 { 96 } else { dpi },
        }))
    }

    fn width(&self, dpi: u32) -> u32 {
        let dpi = if dpi == 0 { 96 } else { dpi };
        let text = (i64::from(self.text_width) * i64::from(dpi) + i64::from(self.measured_dpi) / 2)
            / i64::from(self.measured_dpi);
        let chrome = crate::dpi::scale(2 * PADDING + 2 * ICON_SIZE + ICON_GAP + CHECK_GAP, dpi);
        text.saturating_add(i64::from(chrome))
            .max(i64::from(crate::dpi::scale(MIN_WIDTH, dpi)))
            .min(i64::from(u32::MAX)) as u32
    }
}

/// Owns the native menu and its accessible owner-draw data on the UI thread.
pub struct ProfileMenu {
    handle: HMENU,
    background: HBRUSH,
    dpi: u32,
    // Vec reallocation must not move structures referenced by Win32 itemData.
    #[allow(clippy::vec_box)]
    entries: Vec<Box<MenuEntry>>,
    _ui_thread: PhantomData<Rc<()>>,
}

impl ProfileMenu {
    /// Include the input-mode action only for the full profile menu.
    pub fn new(
        profiles: &[Profile],
        direct: bool,
        include_actions: bool,
        theme: &Theme,
        dpi: u32,
    ) -> Result<Self> {
        // Reserve the existing input-mode command ID and keep WM_MENUCHAR
        // positions representable in its documented low WORD return value.
        if profiles.len() >= crate::MENU_INPUT_MODE {
            return Err(Error::from(E_INVALIDARG));
        }
        let mut menu = Self {
            handle: unsafe { CreatePopupMenu()? },
            background: HBRUSH::default(),
            dpi,
            entries: Vec::with_capacity(profiles.len() + if include_actions { 8 } else { 0 }),
            _ui_thread: PhantomData,
        };
        if !theme.high_contrast {
            menu.background = unsafe { CreateSolidBrush(theme.colors.surface) };
            if menu.background.is_invalid() {
                return Err(Error::from_thread());
            }
            let info = MENUINFO {
                cbSize: std::mem::size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND | MIM_STYLE,
                // The owner draws its check mark; native MFS_CHECKED still
                // exposes the state to accessibility clients.
                dwStyle: MNS_NOCHECK,
                hbrBack: menu.background,
                ..Default::default()
            };
            unsafe { SetMenuInfo(menu.handle, &info)? };
            MENUS.with(|menus| {
                menus
                    .borrow_mut()
                    .insert(menu.handle.0 as usize, Vec::new());
            });
        }
        for (index, profile) in profiles.iter().enumerate() {
            menu.append(
                &profile.name,
                EntryKind::Profile,
                index as u32 + 1,
                false,
                theme,
            )?;
        }
        if include_actions {
            menu.append("", EntryKind::Separator, 0, false, theme)?;
            menu.append(
                "Прямой ввод",
                EntryKind::InputMode,
                crate::MENU_INPUT_MODE as u32,
                direct,
                theme,
            )?;
            menu.append(
                "Журнал чтения (Ctrl+Shift+Space)",
                EntryKind::InputMode,
                crate::MENU_READING as u32,
                crate::reading_active(),
                theme,
            )?;
            menu.append(
                &crate::shortcut_caption(
                    "Передавать Tab приложению",
                    crate::shortcuts::AppAction::ToggleCapture,
                ),
                EntryKind::InputMode,
                crate::MENU_TERMINAL_CAPTURE as u32,
                crate::terminal_capture_active(),
                theme,
            )?;
            menu.append(
                "Читать экран терминала",
                EntryKind::InputMode,
                crate::MENU_READ_VT as u32,
                crate::vt_reading_active(),
                theme,
            )?;
            menu.append(
                "Читать журнал команд",
                EntryKind::InputMode,
                crate::MENU_READ_JOURNAL as u32,
                !crate::vt_reading_active(),
                theme,
            )?;
            for (label, action, command) in [
                (
                    "Прочитать строку ввода",
                    crate::shortcuts::AppAction::ReadCurrentInput,
                    crate::MENU_READ_INPUT,
                ),
                (
                    "Читать новый вывод",
                    crate::shortcuts::AppAction::ReadNewOutput,
                    crate::MENU_READ_NEW_OUTPUT,
                ),
            ] {
                menu.append(
                    &crate::shortcut_caption(label, action),
                    EntryKind::InputMode,
                    command as u32,
                    false,
                    theme,
                )?;
            }
            menu.append(
                &crate::shortcut_caption(
                    "Поиск по истории терминала",
                    crate::shortcuts::AppAction::Search,
                ),
                EntryKind::InputMode,
                crate::MENU_SEARCH as u32,
                false,
                theme,
            )?;
            menu.append(
                "SSH-профили: создать или изменить",
                EntryKind::InputMode,
                crate::MENU_SSH_PROFILES as u32,
                false,
                theme,
            )?;
            for (label, command, mode) in [
                (
                    "Озвучивание: весь новый вывод",
                    crate::MENU_SPEECH_ALL,
                    crate::SpeechMode::All,
                ),
                (
                    "Озвучивание: только важное",
                    crate::MENU_SPEECH_IMPORTANT,
                    crate::SpeechMode::Important,
                ),
                (
                    "Озвучивание: чтение вручную",
                    crate::MENU_SPEECH_MANUAL,
                    crate::SpeechMode::Manual,
                ),
            ] {
                menu.append(
                    label,
                    EntryKind::InputMode,
                    command as u32,
                    crate::speech_mode() == mode,
                    theme,
                )?;
            }
            menu.append(
                &crate::shortcut_caption(
                    "Доступный диалог TUI",
                    crate::shortcuts::AppAction::OpenTuiDialog,
                ),
                EntryKind::InputMode,
                crate::MENU_TUI_DIALOG as u32,
                false,
                theme,
            )?;
            for (label, command, action) in [
                (
                    "Панель Codex App Server",
                    crate::MENU_AGENT_PANEL,
                    crate::shortcuts::AppAction::OpenAgentPanel,
                ),
                (
                    "Новое окно: выбрать профиль",
                    crate::MENU_NEW_WINDOW,
                    crate::shortcuts::AppAction::NewWindow,
                ),
                (
                    "Новая вкладка в папке",
                    crate::MENU_OPEN_FOLDER,
                    crate::shortcuts::AppAction::OpenInFolder,
                ),
                (
                    "Настройки",
                    crate::MENU_PREFERENCES,
                    crate::shortcuts::AppAction::Preferences,
                ),
                (
                    "Недавние профили: открыть",
                    crate::MENU_RESTORE_PROFILE,
                    crate::shortcuts::AppAction::RestoreProfile,
                ),
            ] {
                menu.append(
                    &crate::shortcut_caption(label, action),
                    EntryKind::InputMode,
                    command as u32,
                    false,
                    theme,
                )?;
            }
            menu.append(
                "Защищённый ввод пароля или ключевой фразы",
                EntryKind::InputMode,
                crate::MENU_SECRET_INPUT as u32,
                false,
                theme,
            )?;
            menu.append(
                "Перезапустить в новой вкладке",
                EntryKind::InputMode,
                crate::MENU_RESTART as u32,
                false,
                theme,
            )?;
            menu.append(
                "Справка (F1)",
                EntryKind::InputMode,
                crate::MENU_HELP as u32,
                false,
                theme,
            )?;
        }
        Ok(menu)
    }

    pub fn handle(&self) -> HMENU {
        self.handle
    }

    fn append(
        &mut self,
        label: &str,
        kind: EntryKind,
        command: u32,
        checked: bool,
        theme: &Theme,
    ) -> Result<()> {
        let mut entry = MenuEntry::new(label, kind, theme, self.dpi)?;
        let data = (&*entry as *const MenuEntry) as usize;
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_ID | MIIM_STATE,
            wID: command,
            fState: if checked { MFS_CHECKED } else { MFS_ENABLED },
            ..Default::default()
        };
        if kind == EntryKind::Separator {
            info.fType = MFT_SEPARATOR;
        } else {
            // Keep an actual native caption in addition to the MSAA header.
            info.fMask |= MIIM_STRING;
            info.dwTypeData = PWSTR(entry.native_text.as_mut_ptr());
            info.cch = (entry.native_text.len() - 1) as u32;
        }
        if !theme.high_contrast {
            info.fMask |= MIIM_DATA;
            info.fType |= MFT_OWNERDRAW;
            info.dwItemData = data;
            MENUS.with(|menus| {
                if let Some(items) = menus.borrow_mut().get_mut(&(self.handle.0 as usize)) {
                    items.push(data);
                }
            });
        }
        let position = self.entries.len() as u32;
        // Keep the box alive even if native insertion fails and Drop runs.
        self.entries.push(entry);
        unsafe { InsertMenuItemW(self.handle, position, true, &info) }
    }
}

impl Drop for ProfileMenu {
    fn drop(&mut self) {
        // DestroyMenu runs before the boxed captions and the background brush
        // are released, including during a partially failed construction.
        unsafe {
            let _ = DestroyMenu(self.handle);
        }
        MENUS.with(|menus| {
            menus.borrow_mut().remove(&(self.handle.0 as usize));
        });
        if !self.background.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.background.into());
            }
        }
    }
}

fn with_entry<T>(
    data: usize,
    menu: Option<usize>,
    visit: impl FnOnce(&MenuEntry) -> T,
) -> Option<T> {
    MENUS.with(|menus| {
        let menus = menus.borrow();
        let known = if let Some(menu) = menu {
            menus.get(&menu).is_some_and(|items| items.contains(&data))
        } else {
            menus.values().any(|items| items.contains(&data))
        };
        if !known {
            return None;
        }
        // Registered addresses belong to live stable boxes on this UI thread.
        // Keep the registry borrowed until all access to the entry is finished.
        let entry = unsafe { &*(data as *const MenuEntry) };
        (entry.msaa.dwMSAASignature == MSAA_MENU_SIG as u32).then(|| visit(entry))
    })
}

/// Handle `WM_MEASUREITEM` only for this module's owner-drawn menu entries.
pub fn measure(lparam: LPARAM, dpi: u32) -> bool {
    let pointer = lparam.0 as *mut MEASUREITEMSTRUCT;
    if pointer.is_null() || !pointer.is_aligned() {
        return false;
    }
    // WM_MEASUREITEM supplies a writable MEASUREITEMSTRUCT for this call.
    let item = unsafe { &mut *pointer };
    if item.CtlType != ODT_MENU {
        return false;
    }
    with_entry(item.itemData, None, |entry| {
        item.itemWidth = entry.width(dpi);
        item.itemHeight = crate::dpi::scale(
            if entry.kind == EntryKind::Separator {
                SEPARATOR_HEIGHT
            } else {
                ROW_HEIGHT
            },
            dpi,
        ) as u32;
        true
    })
    .unwrap_or(false)
}

/// Handle `WM_DRAWITEM` before applying button-specific state or drawing.
pub fn draw(item: &DRAWITEMSTRUCT, theme: &Theme, dpi: u32) -> bool {
    if item.CtlType != ODT_MENU {
        return false;
    }
    with_entry(item.itemData, Some(item.hwndItem.0 as usize), |entry| {
        let saved = unsafe { SaveDC(item.hDC) };
        if saved == 0 {
            return false;
        }
        unsafe {
            let _ = IntersectClipRect(
                item.hDC,
                item.rcItem.left,
                item.rcItem.top,
                item.rcItem.right,
                item.rcItem.bottom,
            );
            paint(item, entry, theme, dpi);
            let _ = RestoreDC(item.hDC, saved);
        }
        true
    })
    .unwrap_or(false)
}

unsafe fn paint(item: &DRAWITEMSTRUCT, entry: &MenuEntry, theme: &Theme, dpi: u32) {
    let rc = item.rcItem;
    let hdc = item.hDC;
    let pad = crate::dpi::scale(PADDING, dpi);
    let icon_size = crate::dpi::scale(ICON_SIZE, dpi);
    let _ = FillRect(hdc, &rc, theme.surface);
    if entry.kind == EntryKind::Separator {
        let y = (rc.top + rc.bottom) / 2;
        let line = RECT {
            left: rc.left + pad,
            top: y,
            right: rc.right - pad,
            bottom: y + crate::dpi::scale(1, dpi).max(1),
        };
        let _ = FillRect(hdc, &line, theme.border_subtle);
        return;
    }
    let disabled = item.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0) != 0;
    let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = item.itemState.0 & ODS_HOTLIGHT.0 != 0;
    if !disabled && (selected || hot) {
        let _ = FillRect(
            hdc,
            &rc,
            if selected {
                theme.selected
            } else {
                theme.hover
            },
        );
        let indicator = RECT {
            left: rc.left + crate::dpi::scale(2, dpi),
            top: rc.top + crate::dpi::scale(6, dpi),
            right: rc.left + crate::dpi::scale(4, dpi),
            bottom: rc.bottom - crate::dpi::scale(6, dpi),
        };
        let _ = FillRect(hdc, &indicator, theme.brand);
    }
    let color = if disabled {
        theme.colors.text_muted
    } else {
        theme.colors.text_primary
    };
    let icon = RECT {
        left: rc.left + pad,
        top: rc.top + (rc.bottom - rc.top - icon_size) / 2,
        right: rc.left + pad + icon_size,
        bottom: rc.top + (rc.bottom - rc.top + icon_size) / 2,
    };
    if entry.kind == EntryKind::Profile && !crate::profile_icons::draw(hdc, icon, &entry.label) {
        stroke(
            hdc,
            (icon.left, icon.top),
            [(3, 3), (8, 8), (3, 13)],
            color,
            dpi,
        );
        stroke(hdc, (icon.left, icon.top), [(9, 13), (14, 13)], color, dpi);
    }
    if item.itemState.0 & ODS_CHECKED.0 != 0 {
        stroke(
            hdc,
            (rc.right - pad - icon_size, icon.top),
            [(3, 8), (7, 12), (14, 4)],
            if disabled { color } else { theme.colors.brand },
            dpi,
        );
    }
    let _ = SelectObject(hdc, theme.ui_font.into());
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, color);
    let mut text_rc = RECT {
        left: icon.right + crate::dpi::scale(ICON_GAP, dpi),
        right: rc.right - pad - icon_size - crate::dpi::scale(CHECK_GAP, dpi),
        ..rc
    };
    let mut flags = DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
    if item.itemState.0 & ODS_NOACCEL.0 != 0 {
        flags |= DT_HIDEPREFIX;
    }
    let mut text = entry.native_text[..entry.native_text.len() - 1].to_vec();
    let _ = DrawTextW(hdc, &mut text, &mut text_rc, flags);
}

unsafe fn stroke<const N: usize>(
    hdc: HDC,
    origin: (i32, i32),
    points: [(i32, i32); N],
    color: COLORREF,
    dpi: u32,
) {
    let points = points.map(|(x, y)| POINT {
        x: origin.0 + crate::dpi::scale(x, dpi),
        y: origin.1 + crate::dpi::scale(y, dpi),
    });
    let pen = CreatePen(PS_SOLID, crate::dpi::scale(2, dpi).max(1), color);
    if !pen.is_invalid() {
        let old = SelectObject(hdc, pen.into());
        let _ = Polyline(hdc, &points);
        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(pen.into());
    }
}

fn text_width(text: &[u16], font: HFONT) -> Result<i32> {
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return Err(Error::from_thread());
        }
        let old = SelectObject(hdc, font.into());
        let mut size = SIZE::default();
        let measured = GetTextExtentPoint32W(hdc, text, &mut size).ok();
        let _ = SelectObject(hdc, old);
        let _ = ReleaseDC(None, hdc);
        measured.map(|()| size.cx)
    }
}

/// Restore native mnemonic/first-letter selection for the owner-drawn menu.
/// Returns `None` for unrelated menus, including the native High Contrast path.
pub fn menu_char(wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let menu = HMENU(lparam.0 as *mut _);
    MENUS.with(|menus| {
        let menus = menus.borrow();
        let known = menus.get(&(menu.0 as usize))?;
        let character = char::from_u32((wparam.0 & 0xffff) as u32)?;
        let mut candidates = Vec::new();
        let mut selected = None;
        for position in 0..unsafe { GetMenuItemCount(Some(menu)) } {
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_DATA | MIIM_FTYPE | MIIM_STATE,
                ..Default::default()
            };
            if unsafe { GetMenuItemInfoW(menu, position as u32, true, &mut info) }.is_err() {
                continue;
            }
            if info.fState.contains(MFS_HILITE) {
                selected = Some(position as u32);
            }
            if info.fType.contains(MFT_SEPARATOR)
                || info.fState.0 & MFS_DISABLED.0 != 0
                || !known.contains(&info.dwItemData)
            {
                continue;
            }
            // The same registry lifetime rule as with_entry applies here.
            let entry = unsafe { &*(info.dwItemData as *const MenuEntry) };
            if entry.msaa.dwMSAASignature == MSAA_MENU_SIG as u32 {
                candidates.push((position as u32, entry.label.as_str()));
            }
        }
        let (position, action) = match select_character(&candidates, character, selected) {
            Some((position, true)) => (position, MNC_EXECUTE),
            Some((position, false)) => (position, MNC_SELECT),
            None => (0, MNC_IGNORE),
        };
        Some(LRESULT(((action << 16) | position) as isize))
    })
}

fn select_character(
    candidates: &[(u32, &str)],
    character: char,
    selected: Option<u32>,
) -> Option<(u32, bool)> {
    let same = |value: char| value.to_lowercase().eq(character.to_lowercase());
    let mut matches: Vec<u32> = candidates
        .iter()
        .filter(|(_, label)| caption_keys(label).1.is_some_and(same))
        .map(|(position, _)| *position)
        .collect();
    if matches.is_empty() {
        matches = candidates
            .iter()
            .filter(|(_, label)| caption_keys(label).0.is_some_and(same))
            .map(|(position, _)| *position)
            .collect();
    }
    let first = *matches.first()?;
    let next = selected
        .and_then(|current| matches.iter().copied().find(|position| *position > current))
        .unwrap_or(first);
    Some((next, matches.len() == 1))
}

fn caption_keys(label: &str) -> (Option<char>, Option<char>) {
    let mut first = None;
    let mut mnemonic = None;
    let mut chars = label.chars();
    while let Some(mut character) = chars.next() {
        if character == '&' {
            let Some(next) = chars.next() else { break };
            character = next;
            if next != '&' && mnemonic.is_none() {
                mnemonic = Some(next);
            }
        }
        if first.is_none() && !character.is_whitespace() {
            first = Some(character);
        }
    }
    (first, mnemonic)
}

fn visible_caption(label: &str) -> String {
    let mut visible = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(character) = chars.next() {
        if character == '&' {
            if let Some(next) = chars.next() {
                visible.push(next);
            }
        } else {
            visible.push(character);
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_initials_cycle_and_wrap_without_executing() {
        let items = [
            (0, "PowerShell 7"),
            (1, "Command Prompt"),
            (2, "PowerShell"),
        ];
        assert_eq!(select_character(&items, 'p', None), Some((0, false)));
        assert_eq!(select_character(&items, 'P', Some(0)), Some((2, false)));
        assert_eq!(select_character(&items, 'p', Some(2)), Some((0, false)));
        assert_eq!(select_character(&items, 'c', None), Some((1, true)));
        assert_eq!(select_character(&items, 'x', None), None);
    }

    #[test]
    fn explicit_mnemonics_take_priority_and_escaped_ampersands_stay_literal() {
        let items = [(0, "PowerShell"), (1, "Git &Prompt"), (2, "&&Tools")];
        assert_eq!(select_character(&items, 'p', None), Some((1, true)));
        assert_eq!(caption_keys("&&Tools"), (Some('&'), None));
        assert_eq!(
            visible_caption("Git &Prompt && tools"),
            "Git Prompt & tools"
        );
    }

    #[test]
    fn unicode_initials_and_native_item_positions_are_preserved() {
        let items = [(0, "WSL"), (6, "Прямой ввод")];
        assert_eq!(select_character(&items, 'п', None), Some((6, true)));
    }
}
