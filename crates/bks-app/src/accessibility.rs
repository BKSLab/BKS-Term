use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{CLSID_AccPropServices, IAccPropServices, PROPID_ACC_NAME};
use windows::Win32::UI::WindowsAndMessaging::OBJID_CLIENT;

pub struct ComApartment;

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
        unsafe { CoUninitialize() };
    }
}

pub fn set_accessible_name(hwnd: HWND, name: &str) -> Result<()> {
    let services: IAccPropServices =
        unsafe { CoCreateInstance(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER)? };
    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        services.SetHwndPropStr(
            hwnd,
            OBJID_CLIENT.0 as u32,
            0,
            PROPID_ACC_NAME,
            PCWSTR(name.as_ptr()),
        )
    }
}
