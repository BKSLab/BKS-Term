//! Isolated compatibility backend for acrylic that remains visible inactive.
//!
//! SetWindowCompositionAttribute is exported by user32, but the accent policy
//! is not a supported DWM contract. The caller owns OS/settings gating and must
//! retain its documented/opaque fallback when this entry point is unavailable.

use std::ffi::c_void;
use std::sync::OnceLock;
use windows::core::{s, w, BOOL};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct AttributeData {
    attribute: u32,
    data: *const c_void,
    size: usize,
}

type SetAttribute = unsafe extern "system" fn(HWND, *const AttributeData) -> BOOL;

static SET_ATTRIBUTE: OnceLock<Option<SetAttribute>> = OnceLock::new();

fn entry_point() -> Option<SetAttribute> {
    *SET_ATTRIBUTE.get_or_init(|| unsafe {
        // user32 is already loaded by the native UI. This is a borrowed handle;
        // it must not be freed, and loading a DLL from another path is unnecessary.
        let module = GetModuleHandleW(w!("user32.dll")).ok()?;
        let address = GetProcAddress(module, s!("SetWindowCompositionAttribute"))?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            SetAttribute,
        >(address))
    })
}

/// Apply or remove only the per-window accent policy. Success means the OS
/// accepted the request; the caller remains responsible for settings/policy.
pub fn set_enabled(hwnd: HWND, enabled: bool) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    let Some(set_attribute) = entry_point() else {
        return false;
    };
    let policy = AccentPolicy {
        state: if enabled { 4 } else { 0 }, // ACRYLICBLURBEHIND / DISABLED
        flags: 0,
        // ABGR: neutral tint, independent of foreground alpha. The material
        // and the per-surface veil share the dimming: neither layer should
        // obscure the blurred desktop by itself.
        gradient_color: if enabled { 0x802c_2825 } else { 0 },
        animation_id: 0,
    };
    let data = AttributeData {
        attribute: 19, // WCA_ACCENT_POLICY
        data: (&policy as *const AccentPolicy).cast(),
        size: std::mem::size_of::<AccentPolicy>(),
    };
    unsafe { set_attribute(hwnd, &data).as_bool() }
}
