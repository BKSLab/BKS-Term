//! Short UIA notifications from a persistent, visible native status control.
//! The UI thread creates/destroys the HWND and calls `announce`; the provider
//! stays alive for external UIA clients until its last COM reference is gone.

use std::ffi::c_void;
use std::sync::atomic::{fence, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{
    w, Error, IUnknown, IUnknown_Vtbl, Interface, Result, BSTR, GUID, HRESULT, PCWSTR,
};
use windows::Win32::Foundation::{
    E_FAIL, E_HANDLE, E_NOINTERFACE, E_POINTER, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, S_OK,
    WPARAM,
};
use windows::Win32::System::SystemServices::{SS_CENTERIMAGE, SS_ENDELLIPSIS};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, IRawElementProviderSimple_Vtbl, NotificationKind_Other,
    NotificationProcessing_MostRecent, ProviderOptions, ProviderOptions_ServerSideProvider,
    ProviderOptions_UseComThreading, UIA_AutomationIdPropertyId, UIA_ControlTypePropertyId,
    UIA_HasKeyboardFocusPropertyId, UIA_IsContentElementPropertyId, UIA_IsControlElementPropertyId,
    UIA_IsKeyboardFocusablePropertyId, UIA_NamePropertyId, UIA_TextControlTypeId,
    UiaDisconnectProvider, UiaHostProviderFromHwnd, UiaRaiseNotificationEvent,
    UiaReturnRawElementProvider, UiaRootObjectId, UIA_E_ELEMENTNOTAVAILABLE, UIA_PATTERN_ID,
    UIA_PROPERTY_ID,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetAncestor, GetForegroundWindow, GetPropW, IsWindowVisible,
    RemovePropW, SetPropW, SetWindowTextW, GA_ROOT, HMENU, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_DESTROY, WM_GETOBJECT, WM_NCDESTROY, WS_CHILD, WS_VISIBLE,
};

pub const ID_ANNOUNCEMENT: usize = 1026;
const SUBCLASS_ID: usize = 1;
// GetWindowSubclass is not exported by the system comctl32 5.82 used without
// a v6 manifest. User32 window properties are available on that same system.
const STATE_PROPERTY: PCWSTR = w!("BksTerm.Announcements.WindowState");
const AUTOMATION_ID: &str = "BksTermAnnouncements";
const ACTIVITY_ID: &str = "bks.feedback";
const ELEMENT_NOT_AVAILABLE: HRESULT = HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32);

#[derive(Clone, Copy, Default)]
pub struct Announcer {
    pub hwnd: HWND,
}

struct Shared {
    hwnd: AtomicIsize,
    text: Mutex<String>,
}

struct WindowState {
    provider: IRawElementProviderSimple,
    shared: Arc<Shared>,
}

/// Parent owns the HWND; keep it visible in the status bar and apply its font
/// and theme with the other native STATIC controls. It is never a Tab stop.
pub fn create(parent: HWND, instance: HINSTANCE) -> Result<Announcer> {
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_CENTERIMAGE.0 | SS_ENDELLIPSIS.0),
            0,
            0,
            240,
            24,
            Some(parent),
            Some(HMENU(ID_ANNOUNCEMENT as *mut c_void)),
            Some(instance),
            None,
        )?;
        let shared = Arc::new(Shared {
            hwnd: AtomicIsize::new(hwnd.0 as isize),
            text: Mutex::new(String::new()),
        });
        let state = Box::new(WindowState {
            provider: new_provider(shared.clone()),
            shared,
        });
        let data = Box::into_raw(state);
        if let Err(error) = SetPropW(hwnd, STATE_PROPERTY, Some(HANDLE(data.cast()))) {
            drop(Box::from_raw(data));
            let _ = DestroyWindow(hwnd);
            return Err(error);
        }
        if !SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, data as usize).as_bool() {
            let _ = RemovePropW(hwnd, STATE_PROPERTY);
            drop(Box::from_raw(data));
            let _ = DestroyWindow(hwnd);
            return Err(Error::from_hresult(E_FAIL));
        }
        Ok(Announcer { hwnd })
    }
}

impl Announcer {
    /// Raise one notification for an explicit action. Callers own debounce
    /// and deduplication: identical text can be intentional (e.g. F3: 1 of 1).
    /// This never changes focus or the terminal/reading document's layout.
    pub fn announce(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        unsafe {
            // UIA.SetFocus may expose the focused EDIT as the foreground HWND.
            // Compare roots on both sides; an owned dialog still has its own
            // root and cannot cause notifications from a background window.
            let root = GetAncestor(self.hwnd, GA_ROOT);
            if root.0.is_null() || root != GetAncestor(GetForegroundWindow(), GA_ROOT) {
                return Ok(());
            }
            let data = GetPropW(self.hwnd, STATE_PROPERTY);
            if data.0.is_null() || !IsWindowVisible(self.hwnd).as_bool() {
                return Err(Error::from_hresult(E_HANDLE));
            }
            // Clone before calling UIA: COM calls can re-enter the UI thread.
            let (provider, shared) = {
                let state = &*(data.0 as *const WindowState);
                (state.provider.clone(), state.shared.clone())
            };
            if shared.hwnd.load(Ordering::Acquire) == 0 {
                return Err(Error::from_hresult(ELEMENT_NOT_AVAILABLE));
            }
            *shared
                .text
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = text.to_owned();
            let caption: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            SetWindowTextW(self.hwnd, PCWSTR(caption.as_ptr()))?;
            // A host provider alone cannot raise events. This is the same
            // server-side provider returned for this HWND's WM_GETOBJECT.
            // https://learn.microsoft.com/en-us/windows/win32/api/uiautomationcoreapi/nf-uiautomationcoreapi-uiahostproviderfromhwnd
            UiaRaiseNotificationEvent(
                &provider,
                NotificationKind_Other,
                NotificationProcessing_MostRecent,
                &BSTR::from(text),
                &BSTR::from(ACTIVITY_ID),
            )
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    match message {
        WM_GETOBJECT if lparam.0 as i32 == UiaRootObjectId => {
            let provider = (&*(data as *const WindowState)).provider.clone();
            UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider)
        }
        WM_DESTROY => {
            let (provider, shared) = {
                let state = &*(data as *const WindowState);
                (state.provider.clone(), state.shared.clone())
            };
            shared.hwnd.store(0, Ordering::Release);
            let _ = UiaDisconnectProvider(&provider);
            // Release UIA's raised-event map while the HWND still exists.
            // https://learn.microsoft.com/en-us/windows/win32/api/uiautomationcoreapi/nf-uiautomationcoreapi-uiareturnrawelementprovider
            UiaReturnRawElementProvider(
                hwnd,
                WPARAM(0),
                LPARAM(0),
                None::<&IRawElementProviderSimple>,
            );
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
        WM_NCDESTROY => {
            // Stop new lookups before releasing the state or making callbacks.
            let _ = RemovePropW(hwnd, STATE_PROPERTY);
            let state = Box::from_raw(data as *mut WindowState);
            state.shared.hwnd.store(0, Ordering::Release);
            let _ = RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID);
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            drop(state);
            result
        }
        _ => DefSubclassProc(hwnd, message, wparam, lparam),
    }
}

// A raw vtable lets GetPatternProvider return S_OK plus a null pointer for
// unsupported patterns, as UIA requires; the generated Rust trait returns
// Result<IUnknown> and cannot express that successful null result.
#[repr(C)]
struct Provider {
    vtable: *const IRawElementProviderSimple_Vtbl,
    references: AtomicU32,
    shared: Arc<Shared>,
}

fn new_provider(shared: Arc<Shared>) -> IRawElementProviderSimple {
    let provider = Box::new(Provider {
        vtable: &PROVIDER_VTABLE,
        references: AtomicU32::new(1),
        shared,
    });
    unsafe { IRawElementProviderSimple::from_raw(Box::into_raw(provider).cast()) }
}

static PROVIDER_VTABLE: IRawElementProviderSimple_Vtbl = IRawElementProviderSimple_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: query_interface,
        AddRef: add_ref,
        Release: release,
    },
    ProviderOptions: provider_options,
    GetPatternProvider: pattern_provider,
    GetPropertyValue: property_value,
    HostRawElementProvider: host_provider,
};

unsafe extern "system" fn query_interface(
    this: *mut c_void,
    iid: *const GUID,
    result: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || result.is_null() {
        return E_POINTER;
    }
    *result = std::ptr::null_mut();
    if *iid == IUnknown::IID || *iid == IRawElementProviderSimple::IID {
        add_ref(this);
        *result = this;
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    let provider = &*(this as *const Provider);
    provider.references.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let provider = &*(this as *const Provider);
    let remaining = provider.references.fetch_sub(1, Ordering::Release) - 1;
    if remaining == 0 {
        fence(Ordering::Acquire);
        drop(Box::from_raw(this as *mut Provider));
    }
    remaining
}

unsafe extern "system" fn provider_options(
    _this: *mut c_void,
    result: *mut ProviderOptions,
) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    *result = ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading;
    S_OK
}

unsafe extern "system" fn pattern_provider(
    _this: *mut c_void,
    _pattern: UIA_PATTERN_ID,
    result: *mut *mut c_void,
) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    *result = std::ptr::null_mut();
    S_OK
}

// Match the original Windows SDK identifiers supplied by windows-rs.
#[allow(non_upper_case_globals)]
unsafe extern "system" fn property_value(
    this: *mut c_void,
    property: UIA_PROPERTY_ID,
    result: *mut VARIANT,
) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    result.write(VARIANT::default());
    let provider = &*(this as *const Provider);
    if provider.shared.hwnd.load(Ordering::Acquire) == 0 {
        return ELEMENT_NOT_AVAILABLE;
    }
    let value = match property {
        UIA_NamePropertyId => VARIANT::from(
            provider
                .shared
                .text
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_str(),
        ),
        UIA_AutomationIdPropertyId => VARIANT::from(AUTOMATION_ID),
        UIA_ControlTypePropertyId => VARIANT::from(UIA_TextControlTypeId.0),
        UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => VARIANT::from(true),
        UIA_IsKeyboardFocusablePropertyId | UIA_HasKeyboardFocusPropertyId => VARIANT::from(false),
        _ => VARIANT::default(), // Host supplies geometry, HWND and other defaults.
    };
    result.write(value);
    S_OK
}

unsafe extern "system" fn host_provider(this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    *result = std::ptr::null_mut();
    let provider = &*(this as *const Provider);
    let hwnd = provider.shared.hwnd.load(Ordering::Acquire);
    if hwnd == 0 {
        return ELEMENT_NOT_AVAILABLE;
    }
    match UiaHostProviderFromHwnd(HWND(hwnd as *mut c_void)) {
        Ok(host) => {
            *result = host.into_raw();
            S_OK
        }
        Err(error) => error.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Accessibility::UIA_InvokePatternId;

    #[test]
    fn notification_provider_exposes_current_unicode_text_without_focus() {
        let shared = Arc::new(Shared {
            hwnd: AtomicIsize::new(1), // Properties do not call Win32 or create a window.
            text: Mutex::new("Команда отправлена".to_owned()),
        });
        let provider = new_provider(shared.clone());
        unsafe {
            let name = provider.GetPropertyValue(UIA_NamePropertyId).unwrap();
            assert_eq!(
                BSTR::try_from(&name).unwrap().to_string(),
                "Команда отправлена"
            );
            *shared.text.lock().unwrap() = "1 из 2 — 😀".to_owned();
            let name = provider.GetPropertyValue(UIA_NamePropertyId).unwrap();
            assert_eq!(BSTR::try_from(&name).unwrap().to_string(), "1 из 2 — 😀");
            assert!(!bool::try_from(
                &provider
                    .GetPropertyValue(UIA_IsKeyboardFocusablePropertyId)
                    .unwrap()
            )
            .unwrap());
            assert!(!bool::try_from(
                &provider
                    .GetPropertyValue(UIA_HasKeyboardFocusPropertyId)
                    .unwrap()
            )
            .unwrap());
        }
    }

    #[test]
    fn unsupported_patterns_return_successful_null_and_destroyed_state_is_unavailable() {
        let shared = Arc::new(Shared {
            hwnd: AtomicIsize::new(1),
            text: Mutex::new(String::new()),
        });
        let provider = new_provider(shared.clone());
        unsafe {
            let mut pattern = std::ptr::dangling_mut::<c_void>();
            let result = (provider.vtable().GetPatternProvider)(
                provider.as_raw(),
                UIA_InvokePatternId,
                &mut pattern,
            );
            assert_eq!(result, S_OK);
            assert!(pattern.is_null());
            shared.hwnd.store(0, Ordering::Release);
            assert_eq!(
                provider
                    .GetPropertyValue(UIA_NamePropertyId)
                    .unwrap_err()
                    .code(),
                ELEMENT_NOT_AVAILABLE
            );
        }
    }
}
