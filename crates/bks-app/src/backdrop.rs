//! Windows glass backend selection, respecting system/user accessibility policy.
//!
//! The material alone does not make GDI pixels transparent. The shell must use
//! `glass_buffer` for the shell and native terminal painting, and extend the
//! material over the entire client area. Glyphs retain their own opacity.

use std::cell::Cell;
use std::ffi::c_void;
use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMNCRENDERINGPOLICY, DWMNCRP_DISABLED,
    DWMNCRP_USEWINDOWSTYLE, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_NCRENDERING_POLICY,
    DWMWA_SYSTEMBACKDROP_TYPE, DWM_SYSTEMBACKDROP_TYPE,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Controls::MARGINS;

thread_local! {
    // The application has one main window, owned and painted by its UI thread.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static COMPATIBILITY: Cell<bool> = const { Cell::new(false) };
}

/// Whether the main window currently accepts the requested backdrop material.
/// Windows can still substitute a solid material for power or session policy.
pub fn active() -> bool {
    ACTIVE.get()
}

/// The compatibility material delegates all non-client painting to our shell.
pub fn custom_frame() -> bool {
    COMPATIBILITY.get()
}

/// Refresh after WM_SETTINGCHANGE, WM_THEMECHANGED and WM_DWMCOMPOSITIONCHANGED.
/// Call `extend` afterwards to cover the entire client area.
/// Unsupported versions (including Windows 10) fail the capability probe and
/// use the same opaque path as disabled transparency and high contrast.
pub fn apply(hwnd: HWND, high_contrast: bool) -> bool {
    let disabled = std::env::var_os("BKS_TERM_DISABLE_BLUR").is_some_and(|value| value == "1");
    if high_contrast || disabled || !transparency_enabled() {
        disable(hwnd);
        return false;
    }

    // Gate the compatibility backend on the documented Windows 11 capability.
    // Windows 10 and failed probes retain the ordinary opaque paint path.
    if !set_material(hwnd, DWMSBT_TRANSIENTWINDOW) {
        disable(hwnd);
        return false;
    }
    // System transient Acrylic becomes solid when the window deactivates.
    // The isolated compatibility backend keeps the same glass while inactive;
    // failure falls back to the documented material, never to invisible text.
    let _ = set_material(hwnd, DWMSBT_NONE);
    if crate::accent_backdrop::set_enabled(hwnd, true) {
        COMPATIBILITY.set(true);
        ACTIVE.set(true);
        return true;
    }
    let _ = crate::accent_backdrop::set_enabled(hwnd, false);
    COMPATIBILITY.set(false);
    let _ = set_frame_policy(hwnd, DWMNCRP_USEWINDOWSTYLE);
    if set_material(hwnd, DWMSBT_TRANSIENTWINDOW) {
        ACTIVE.set(true);
        return true;
    }
    disable(hwnd);
    false
}

/// Extend glass beneath the shell, terminal and input without changing layout.
pub fn extend(hwnd: HWND) -> bool {
    if !active() {
        return false;
    }
    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    if unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) }.is_ok() {
        // The shell already paints its own non-client chrome. Leaving DWM's
        // native frame renderer enabled covers inactive caption/thick-frame
        // windows with a solid fill, even when Accent Acrylic was accepted.
        // This changes only painting: native sizing/Snap styles stay intact.
        if COMPATIBILITY.get() && !set_frame_policy(hwnd, DWMNCRP_DISABLED) {
            disable(hwnd);
            return false;
        }
        true
    } else {
        disable(hwnd);
        false
    }
}

fn disable(hwnd: HWND) {
    ACTIVE.set(false);
    COMPATIBILITY.set(false);
    let _ = crate::accent_backdrop::set_enabled(hwnd, false);
    let _ = set_material(hwnd, DWMSBT_NONE);
    let _ = set_frame_policy(hwnd, DWMNCRP_USEWINDOWSTYLE);
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
    }
}

fn set_frame_policy(hwnd: HWND, policy: DWMNCRENDERINGPOLICY) -> bool {
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            (&policy as *const DWMNCRENDERINGPOLICY).cast(),
            std::mem::size_of_val(&policy) as u32,
        )
        .is_ok()
    }
}

fn set_material(hwnd: HWND, material: DWM_SYSTEMBACKDROP_TYPE) -> bool {
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&material as *const DWM_SYSTEMBACKDROP_TYPE).cast::<c_void>(),
            std::mem::size_of_val(&material) as u32,
        )
        .is_ok()
    }
}

fn transparency_enabled() -> bool {
    let mut enabled = 0_u32;
    let mut size = std::mem::size_of_val(&enabled) as u32;
    // This is Windows Settings' current per-user preference, not a public API
    // contract. Read only; DWM remains authoritative about material support.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut enabled as *mut u32).cast::<c_void>()),
            Some(&mut size),
        )
    };
    // An absent preference uses Windows' default. Other read/type errors take
    // the conservative opaque path instead of ignoring an unreadable setting.
    result == ERROR_FILE_NOT_FOUND || (result == ERROR_SUCCESS && enabled != 0)
}
