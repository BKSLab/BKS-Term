//! Explicit desktop actions. No registry or process operation runs at startup.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_CREATED_NEW_KEY, REG_OPTION_NON_VOLATILE,
    REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

const EXPLORER_KEYS: [(&str, &str); 2] = [
    (r"Software\Classes\Directory\shell\BKSTerm", "%1"),
    (r"Software\Classes\Directory\Background\shell\BKSTerm", "%V"),
];
const OWNER_VALUE: &str = "BksTermOwnerExecutable";

pub fn new_window(profile_id: &str, cwd: Option<&Path>) -> io::Result<()> {
    crate::launch::validate_profile_id(profile_id).map_err(invalid_input)?;
    if let Some(path) = cwd {
        crate::launch::validate_working_directory(path).map_err(invalid_input)?;
    }
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--profile").arg(profile_id);
    if let Some(path) = cwd {
        command.arg("--cwd").arg(path);
    }
    // A GUI application opens the visible window the user requested. No shell
    // and no command string are involved; Child drop leaves that window alive.
    drop(command.spawn()?);
    Ok(())
}

/// Call only for an explicit --register-explorer / --unregister-explorer flag.
/// Changes are restricted to two per-user verbs owned by this executable.
pub fn set_explorer_integration(enabled: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let owner = executable.as_os_str().encode_wide().collect::<Vec<_>>();
    // Preflight both verbs before mutating either one, including on unregister.
    for (path, placeholder) in EXPLORER_KEYS {
        let expected = explorer_command(&executable, placeholder)?;
        if let Some(key) = RegistryKey::open(path)? {
            require_owned(&key, &expected, &owner)?;
        }
    }
    for (path, placeholder) in EXPLORER_KEYS {
        let expected = explorer_command(&executable, placeholder)?;
        if enabled {
            register_verb(path, &expected, &owner)?;
        } else if let Some(key) = RegistryKey::open(path)? {
            require_owned(&key, &expected, &owner)?;
            drop(key);
            remove_verb(path)?;
        }
    }
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    Ok(())
}

fn explorer_command(executable: &Path, placeholder: &str) -> io::Result<Vec<u16>> {
    let path: Vec<_> = executable.as_os_str().encode_wide().collect();
    if !executable.is_absolute()
        || path
            .iter()
            .any(|unit| matches!(*unit, 0..=31 | 127..=159 | 34))
        || !matches!(placeholder, "%1" | "%V")
    {
        return Err(invalid_input(
            "Недопустимый путь программы или параметр Проводника.",
        ));
    }
    let mut command = vec![b'"' as u16];
    command.extend(path);
    command.extend(format!("\" --choose-profile --cwd \"{placeholder}\"").encode_utf16());
    Ok(command)
}

fn is_owned(
    command: Option<&[u16]>,
    owner: Option<&[u16]>,
    expected: &[u16],
    executable: &[u16],
) -> bool {
    command == Some(expected) && owner == Some(executable)
}

fn require_owned(key: &RegistryKey, expected: &[u16], owner: &[u16]) -> io::Result<()> {
    let command = key.read_string(Some("command"), None)?;
    let stored_owner = key.read_string(None, Some(OWNER_VALUE))?;
    if !is_owned(command.as_deref(), stored_owner.as_deref(), expected, owner) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Пункт BKSTerm в меню Проводника принадлежит другому файлу или изменён. Он сохранён без изменений."));
    }
    Ok(())
}

fn register_verb(path: &str, command: &[u16], owner: &[u16]) -> io::Result<()> {
    let (key, created) = RegistryKey::create(HKEY_CURRENT_USER, path)?;
    let result = (|| {
        if !created {
            require_owned(&key, command, owner)?;
        }
        let (child, _) = RegistryKey::create(key.0, "command")?;
        child.write_string(None, command)?;
        key.write_string(Some(OWNER_VALUE), owner)?;
        key.write_string(
            None,
            &"Открыть в BKS Term".encode_utf16().collect::<Vec<_>>(),
        )?;
        Ok(())
    })();
    drop(key);
    if result.is_err() && created {
        // This call created the previously absent root, so cleanup never
        // removes a pre-existing registration after a partial write failure.
        let _ = remove_verb(path);
    }
    result
}

fn remove_verb(path: &str) -> io::Result<()> {
    let path = wide(OsStr::new(path));
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
    if missing(status) {
        Ok(())
    } else {
        checked(status)
    }
}

struct RegistryKey(HKEY);

impl RegistryKey {
    fn open(path: &str) -> io::Result<Option<Self>> {
        let path = wide(OsStr::new(path));
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                KEY_READ,
                &mut key,
            )
        };
        if missing(status) {
            return Ok(None);
        }
        checked(status)?;
        Ok(Some(Self(key)))
    }

    fn create(parent: HKEY, path: &str) -> io::Result<(Self, bool)> {
        let path = wide(OsStr::new(path));
        let mut key = HKEY::default();
        let mut disposition = Default::default();
        checked(unsafe {
            RegCreateKeyExW(
                parent,
                PCWSTR(path.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                None,
                &mut key,
                Some(&mut disposition),
            )
        })?;
        Ok((Self(key), disposition == REG_CREATED_NEW_KEY))
    }

    fn read_string(
        &self,
        subkey: Option<&str>,
        name: Option<&str>,
    ) -> io::Result<Option<Vec<u16>>> {
        let subkey = subkey.map(|text| wide(OsStr::new(text)));
        let name = name.map(|text| wide(OsStr::new(text)));
        let subkey = subkey
            .as_ref()
            .map_or(PCWSTR::null(), |text| PCWSTR(text.as_ptr()));
        let name = name
            .as_ref()
            .map_or(PCWSTR::null(), |text| PCWSTR(text.as_ptr()));
        let mut length = 0;
        let status = unsafe {
            RegGetValueW(
                self.0,
                subkey,
                name,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut length),
            )
        };
        if missing(status) {
            return Ok(None);
        }
        checked(status)?;
        if !(2..=65_536).contains(&length) || !length.is_multiple_of(2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Недопустимая строка команды Проводника.",
            ));
        }
        let mut text = vec![0_u16; length as usize / 2];
        checked(unsafe {
            RegGetValueW(
                self.0,
                subkey,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(text.as_mut_ptr().cast()),
                Some(&mut length),
            )
        })?;
        if length as usize > text.len() * 2 || !length.is_multiple_of(2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Строка команды Проводника изменилась при чтении.",
            ));
        }
        text.truncate(length as usize / 2);
        if text.pop() != Some(0) || text.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Строка команды Проводника содержит лишние нули.",
            ));
        }
        Ok(Some(text))
    }

    fn write_string(&self, name: Option<&str>, text: &[u16]) -> io::Result<()> {
        if text.contains(&0) {
            return Err(invalid_input("Строка реестра содержит нулевой символ."));
        }
        let name = name.map(|text| wide(OsStr::new(text)));
        let name = name
            .as_ref()
            .map_or(PCWSTR::null(), |text| PCWSTR(text.as_ptr()));
        let bytes: Vec<_> = text
            .iter()
            .copied()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        checked(unsafe { RegSetValueExW(self.0, name, None, REG_SZ, Some(&bytes)) })
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

fn missing(status: WIN32_ERROR) -> bool {
    status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND
}

fn checked(status: WIN32_ERROR) -> io::Result<()> {
    if status.0 == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status.0 as i32))
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_commands_quote_the_executable_and_folder_placeholder() {
        let executable = Path::new(r"C:\Программы с пробелами\BKS Term\bks-app.exe");
        assert_eq!(
            String::from_utf16(&explorer_command(executable, "%1").unwrap()).unwrap(),
            r#""C:\Программы с пробелами\BKS Term\bks-app.exe" --choose-profile --cwd "%1""#
        );
        assert_eq!(
            String::from_utf16(&explorer_command(executable, "%V").unwrap()).unwrap(),
            r#""C:\Программы с пробелами\BKS Term\bks-app.exe" --choose-profile --cwd "%V""#
        );
        assert!(explorer_command(Path::new("relative.exe"), "%1").is_err());
        assert!(explorer_command(Path::new("C:\\bad\0.exe"), "%1").is_err());
        assert!(explorer_command(Path::new(r#"C:\bad".exe"#), "%1").is_err());
        assert!(explorer_command(executable, "%*").is_err());
    }

    #[test]
    fn ownership_requires_both_exact_command_and_current_executable() {
        let executable = Path::new(r"C:\BKS Term\bks-app.exe");
        let owner: Vec<_> = executable.as_os_str().encode_wide().collect();
        let expected = explorer_command(executable, "%1").unwrap();
        let another = explorer_command(Path::new(r"C:\other\bks-app.exe"), "%1").unwrap();
        assert!(is_owned(Some(&expected), Some(&owner), &expected, &owner));
        assert!(!is_owned(Some(&another), Some(&owner), &expected, &owner));
        assert!(!is_owned(
            Some(&expected),
            Some(&another),
            &expected,
            &owner
        ));
        assert!(!is_owned(None, Some(&owner), &expected, &owner));
        assert!(!is_owned(Some(&expected), None, &expected, &owner));
        let background = explorer_command(executable, "%V").unwrap();
        assert!(!is_owned(
            Some(&background),
            Some(&owner),
            &expected,
            &owner
        ));
        let mut amended = expected.clone();
        amended.extend(" --profile unsafe".encode_utf16());
        assert!(!is_owned(Some(&amended), Some(&owner), &expected, &owner));
    }
}
