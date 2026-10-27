//! Standard Windows folder chooser. Cancellation has no session side effect.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::core::{w, Error, Result, HRESULT};
use windows::Win32::Foundation::{ERROR_CANCELLED, E_INVALIDARG, HWND};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_DONTADDTORECENT, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR,
    FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

/// The application's UI thread already initializes a COM STA for accessibility.
/// A path is returned only after the user confirms an existing filesystem folder.
pub fn pick(owner: HWND) -> Result<Option<PathBuf>> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(
            dialog.GetOptions()?
                | FOS_PICKFOLDERS
                | FOS_FORCEFILESYSTEM
                | FOS_PATHMUSTEXIST
                | FOS_NOCHANGEDIR
                | FOS_DONTADDTORECENT,
        )?;
        dialog.SetTitle(w!("Рабочая папка новой сессии — BKS Term"))?;
        dialog.SetOkButtonLabel(w!("Выбрать папку"))?;
        if let Err(error) = dialog.Show(Some(owner)) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error);
        }
        let item = dialog.GetResult()?;
        let display = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(OsString::from_wide(display.as_wide()));
        CoTaskMemFree(Some(display.0.cast()));
        if !path.is_absolute() || !path.is_dir() {
            return Err(Error::new(
                E_INVALIDARG,
                "Выберите существующую папку с абсолютным путём.",
            ));
        }
        Ok(Some(path))
    }
}
