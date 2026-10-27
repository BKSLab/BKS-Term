//! Clipboard access is only called for an explicit copy or paste action.
use windows::core::{Error, Result};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

const UNICODE_TEXT: u32 = 13;
const MAX_PASTE_BYTES: usize = 2 * 1024 * 1024;

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub fn read(owner: HWND) -> Result<String> {
    unsafe {
        OpenClipboard(Some(owner))?;
        let _clipboard = Clipboard;
        let handle = HGLOBAL(GetClipboardData(UNICODE_TEXT)?.0);
        let size = GlobalSize(handle);
        if size == 0 || size > MAX_PASTE_BYTES {
            return Err(Error::from(windows::Win32::Foundation::E_INVALIDARG));
        }
        let pointer = GlobalLock(handle) as *const u16;
        if pointer.is_null() {
            return Err(Error::from_thread());
        }
        let units = std::slice::from_raw_parts(pointer, size / 2);
        let length = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..length]);
        let _ = GlobalUnlock(handle);
        Ok(text)
    }
}

pub fn write(owner: HWND, text: &str) -> Result<()> {
    let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)?;
        let pointer = GlobalLock(memory) as *mut u16;
        if pointer.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), pointer, units.len());
        let _ = GlobalUnlock(memory);
        let result = (|| {
            OpenClipboard(Some(owner))?;
            let _clipboard = Clipboard;
            EmptyClipboard()?;
            SetClipboardData(UNICODE_TEXT, Some(HANDLE(memory.0)))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = GlobalFree(Some(memory));
        }
        result
    }
}
