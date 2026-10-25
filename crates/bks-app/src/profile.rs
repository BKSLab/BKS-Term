use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    Local,
    Wsl,
    Ssh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: ProfileKind,
    pub executable: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProfileManager {
    pub profiles: Vec<Profile>,
    pub default_profile_id: String,
}

#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    schema_version: u32,
    default_profile_id: String,
    profiles: Vec<Profile>,
}

const SCHEMA_VERSION: u32 = 1;
const PS_PROMPT: &str = r"function global:prompt { [char]27 + ']133;A' + [char]27 + '\PS ' + $executionContext.SessionState.Path.CurrentLocation + '> ' + [char]27 + ']133;B' + [char]27 + '\' }";

impl Profile {
    pub fn command_line(&self) -> String {
        let mut parts = Vec::with_capacity(1 + self.args.len());
        parts.push(quote_windows_arg(&self.executable));
        parts.extend(self.args.iter().map(|arg| quote_windows_arg(arg)));
        parts.join(" ")
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("profile id must not be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err(format!("profile '{}' name must not be empty", self.id));
        }
        if self.executable.trim().is_empty() {
            return Err(format!(
                "profile '{}' executable must not be empty",
                self.id
            ));
        }
        if !executable_exists(&self.executable) {
            return Err(format!(
                "profile '{}' executable not found: {}",
                self.id, self.executable
            ));
        }
        Ok(())
    }
}

impl ProfileManager {
    pub fn discover() -> Self {
        let mut profiles = vec![
            Profile {
                id: "windows-powershell".to_string(),
                name: "Windows PowerShell".to_string(),
                kind: ProfileKind::Local,
                executable: "powershell.exe".to_string(),
                args: ps_args(),
                working_directory: None,
            },
            Profile {
                id: "windows-cmd".to_string(),
                name: "Command Prompt".to_string(),
                kind: ProfileKind::Local,
                executable: "cmd.exe".to_string(),
                args: vec!["/Q".to_string()],
                working_directory: None,
            },
        ];
        if let Some(executable) = locate("pwsh.exe", &[]) {
            profiles.push(Profile {
                id: "pwsh".to_string(),
                name: "PowerShell 7".to_string(),
                kind: ProfileKind::Local,
                executable,
                args: ps_args(),
                working_directory: None,
            });
        }
        if let Some(executable) = locate(
            "bash.exe",
            &[
                r"C:\Program Files\Git\bin\bash.exe",
                r"C:\Program Files (x86)\Git\bin\bash.exe",
            ],
        ) {
            profiles.push(Profile {
                id: "git-bash".to_string(),
                name: "Git Bash".to_string(),
                kind: ProfileKind::Local,
                executable,
                args: vec!["--login".to_string(), "-i".to_string()],
                working_directory: None,
            });
        }
        if let Some(executable) = locate("wsl.exe", &[]) {
            profiles.push(Profile {
                id: "wsl".to_string(),
                name: "WSL".to_string(),
                kind: ProfileKind::Wsl,
                executable,
                args: Vec::new(),
                working_directory: None,
            });
        }
        ProfileManager {
            profiles,
            default_profile_id: "windows-powershell".to_string(),
        }
    }

    pub fn default_profile(&self) -> &Profile {
        self.get(&self.default_profile_id)
            .unwrap_or(&self.profiles[0])
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Some(manager) if !manager.profiles.is_empty() => manager,
            _ => Self::discover(),
        }
    }

    fn load(path: &Path) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        let file: ProfilesFile = serde_json::from_str(&data).ok()?;
        Some(ProfileManager {
            profiles: file.profiles,
            default_profile_id: file.default_profile_id,
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let file = ProfilesFile {
            schema_version: SCHEMA_VERSION,
            default_profile_id: self.default_profile_id.clone(),
            profiles: self.profiles.clone(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let temp = temp_path(path);
        fs::write(&temp, json.as_bytes())?;
        let temp_wide: Vec<u16> = temp.as_os_str().encode_wide().chain([0]).collect();
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(temp_wide.as_ptr()),
                PCWSTR(path_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(io::Error::other)
        }
    }
}

fn ps_args() -> Vec<String> {
    vec![
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NoExit".to_string(),
        "-Command".to_string(),
        PS_PROMPT.to_string(),
    ]
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let mut temp = path.to_path_buf();
    temp.set_file_name(name);
    temp
}

fn locate(exe: &str, standard: &[&str]) -> Option<String> {
    if executable_exists(exe) {
        return Some(exe.to_string());
    }
    standard
        .iter()
        .find(|path| Path::new(path).is_file())
        .map(|path| (*path).to_string())
}

fn executable_exists(exe: &str) -> bool {
    let path = Path::new(exe);
    if path.is_absolute() || exe.contains('\\') || exe.contains('/') {
        return path.is_file();
    }
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };
    let extensions = executable_extensions(exe);
    for directory in env::split_paths(&path_var) {
        for extension in &extensions {
            let candidate = if extension.is_empty() {
                directory.join(exe)
            } else {
                let mut name = OsString::from(exe);
                name.push(extension);
                directory.join(name)
            };
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

fn executable_extensions(exe: &str) -> Vec<String> {
    let mut extensions = Vec::new();
    if Path::new(exe).extension().is_some() {
        extensions.push(String::new());
    }
    if let Some(pathtext) = env::var_os("PATHEXT") {
        if let Ok(pathtext) = pathtext.into_string() {
            for part in pathtext.split(';') {
                let part = part.trim();
                if !part.is_empty()
                    && !extensions
                        .iter()
                        .any(|extension: &String| extension.eq_ignore_ascii_case(part))
                {
                    extensions.push(part.to_string());
                }
            }
        }
    }
    if !extensions.iter().any(|extension| extension.is_empty()) {
        extensions.push(String::new());
    }
    extensions
}

fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    if !arg.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }
    let mut output = String::with_capacity(arg.len() + 2);
    output.push('"');
    let mut pending = 0_usize;
    for character in arg.chars() {
        match character {
            '\\' => pending += 1,
            '"' => {
                for _ in 0..(2 * pending + 1) {
                    output.push('\\');
                }
                output.push('"');
                pending = 0;
            }
            _ => {
                for _ in 0..pending {
                    output.push('\\');
                }
                pending = 0;
                output.push(character);
            }
        }
    }
    for _ in 0..(2 * pending) {
        output.push('\\');
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample(id: &str, executable: &str, args: Vec<String>) -> Profile {
        Profile {
            id: id.to_string(),
            name: id.to_string(),
            kind: ProfileKind::Local,
            executable: executable.to_string(),
            args,
            working_directory: None,
        }
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("bks-profile-test-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn windows_quoting() {
        assert_eq!(quote_windows_arg("hello"), "hello");
        assert_eq!(quote_windows_arg("a b"), "\"a b\"");
        assert_eq!(quote_windows_arg("\"; calc"), "\"\\\"; calc\"");
        assert_eq!(quote_windows_arg("a b\\"), "\"a b\\\\\"");
        let profile = sample("x", "cmd.exe", vec!["a b".to_string(), "c".to_string()]);
        assert_eq!(profile.command_line(), "cmd.exe \"a b\" c");
    }

    #[test]
    fn validate_executables() {
        assert!(sample("x", "bks_no_such_exe_98765_qwerty.exe", Vec::new())
            .validate()
            .is_err());
        assert!(sample("", "cmd.exe", Vec::new()).validate().is_err());
        assert!(sample("cmd", "cmd.exe", Vec::new()).validate().is_ok());
    }

    #[test]
    fn save_and_load_roundtrip_with_replacement() {
        let directory = unique_temp_dir();
        let file = directory.join("profiles.json");
        let manager = ProfileManager::discover();
        manager.save(&file).unwrap();
        manager.save(&file).unwrap();
        let loaded = ProfileManager::load_or_default(&file);
        assert_eq!(loaded.default_profile_id, manager.default_profile_id);
        assert_eq!(loaded.profiles, manager.profiles);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn load_or_default_missing_file_falls_back() {
        let directory = unique_temp_dir();
        let manager = ProfileManager::load_or_default(&directory.join("absent.json"));
        assert!(!manager.profiles.is_empty());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn discover_has_required_profiles() {
        let manager = ProfileManager::discover();
        assert_eq!(manager.default_profile().id, "windows-powershell");
        assert!(manager.get("windows-powershell").is_some());
        assert!(manager.get("windows-cmd").is_some());
        assert!(manager
            .profiles
            .iter()
            .all(|profile| profile.kind != ProfileKind::Ssh));
    }
}
