use std::cell::RefCell;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CLSID_AccPropServices, IAccPropServices, Name_Property_GUID, PROPID_ACC_NAME,
};
use windows::Win32::UI::WindowsAndMessaging::OBJID_CLIENT;

pub struct ComApartment;

thread_local! {
    // Keep dynamic annotations alive for the UI thread's COM lifetime.
    static PROPERTIES: RefCell<Option<IAccPropServices>> = const { RefCell::new(None) };
}

impl ComApartment {
    pub fn initialize() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        PROPERTIES.with(|properties| *properties.borrow_mut() = None);
        unsafe { CoUninitialize() };
    }
}

pub fn set_accessible_name(hwnd: HWND, name: &str) -> Result<()> {
    let services: IAccPropServices = PROPERTIES.with(|properties| -> Result<_> {
        let mut properties = properties.borrow_mut();
        if properties.is_none() {
            *properties = Some(unsafe {
                CoCreateInstance(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER)?
            });
        }
        Ok(properties
            .as_ref()
            .expect("annotation service initialized")
            .clone())
    })?;
    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        services.SetHwndPropStr(
            hwnd,
            OBJID_CLIENT.0 as u32,
            0,
            PROPID_ACC_NAME,
            PCWSTR(name.as_ptr()),
        )?;
        // Native UIA proxies read the UIA property annotation independently
        // of MSAA accName (notably for EDIT). Set both on the same HWND.
        // https://learn.microsoft.com/en-us/accessibility-tools-docs/items/win32/edit_name
        services.SetHwndPropStr(
            hwnd,
            OBJID_CLIENT.0 as u32,
            0,
            Name_Property_GUID,
            PCWSTR(name.as_ptr()),
        )
    }
}
