use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use crate::config_lock::ConfigLock;

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

/// Only connection metadata. OpenSSH owns config, host keys and credentials.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshOptions {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

impl SshOptions {
    pub fn parse(host: &str, user: &str, port: &str, identity_file: &str) -> Result<Self, String> {
        let port = if port.trim().is_empty() {
            None
        } else {
            Some(
                port.trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| "Порт SSH должен быть числом от 1 до 65535.".to_owned())?,
            )
        };
        let optional = |value: &str| (!value.trim().is_empty()).then(|| value.trim().to_owned());
        let options = Self {
            host: host.trim().to_owned(),
            user: optional(user),
            port,
            identity_file: optional(identity_file),
        };
        options.arguments()?;
        Ok(options)
    }

    pub fn arguments(&self) -> Result<Vec<String>, String> {
        if self.host.is_empty()
            || self.host.starts_with('-')
            || self
                .host
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
        {
            return Err("Укажите адрес или алиас SSH без пробелов, управляющих символов и начального дефиса.".to_owned());
        }
        if self.host.contains('@') || self.host.contains("://") {
            return Err(
                "Укажите только адрес или алиас; пользователя и порт задайте в отдельных полях."
                    .to_owned(),
            );
        }
        if self.port == Some(0) {
            return Err("Порт SSH должен быть числом от 1 до 65535.".to_owned());
        }
        let mut args = Vec::new();
        if let Some(user) = &self.user {
            if user.is_empty() || user.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
                return Err(
                    "Пользователь SSH не должен содержать пробелы или управляющие символы."
                        .to_owned(),
                );
            }
            args.extend(["-l".to_owned(), user.clone()]);
        }
        if let Some(port) = self.port {
            args.extend(["-p".to_owned(), port.to_string()]);
        }
        if let Some(path) = &self.identity_file {
            if path.is_empty() || path.chars().any(char::is_control) {
                return Err("Укажите путь к файлу ключа без управляющих символов.".to_owned());
            }
            args.extend(["-i".to_owned(), path.clone()]);
        }
        args.push(self.host.clone());
        Ok(args)
    }

    fn from_arguments(args: &[String]) -> Result<Self, String> {
        let unsupported = || {
            "Редактор поддерживает адрес/алиас и параметры -l, -p, -i. Дополнительные параметры сохранённого профиля не изменены.".to_owned()
        };
        let mut user = None;
        let mut port = None;
        let mut identity = None;
        let mut host = None;
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            if argument == "--" {
                if index + 2 != args.len() || host.is_some() {
                    return Err(unsupported());
                }
                host = Some(args[index + 1].clone());
                break;
            }
            if argument.starts_with('-') {
                let (flag, inline) = if argument.len() > 2
                    && matches!(
                        argument.as_bytes().get(1).copied(),
                        Some(b'l' | b'p' | b'i')
                    ) {
                    (&argument[..2], Some(argument[2..].to_owned()))
                } else {
                    (argument.as_str(), None)
                };
                let slot = match flag {
                    "-l" => &mut user,
                    "-p" => &mut port,
                    "-i" => &mut identity,
                    _ => return Err(unsupported()),
                };
                if host.is_some() || slot.is_some() {
                    return Err(unsupported());
                }
                let value = match inline {
                    Some(value) => value,
                    None => {
                        index += 1;
                        args.get(index).cloned().ok_or_else(unsupported)?
                    }
                };
                if value.trim().is_empty() {
                    return Err(unsupported());
                }
                *slot = Some(value);
            } else if host.replace(argument.clone()).is_some() {
                return Err(unsupported());
            }
            index += 1;
        }
        let mut host = host.ok_or_else(unsupported)?;
        if let Some((login, address)) = host.rsplit_once('@') {
            if user.is_some() {
                return Err(unsupported());
            }
            user = Some(login.to_owned());
            host = address.to_owned();
        }
        Self::parse(
            &host,
            user.as_deref().unwrap_or(""),
            port.as_deref().unwrap_or(""),
            identity.as_deref().unwrap_or(""),
        )
    }
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
        let executable = self
            .executable_for_launch()
            .unwrap_or_else(|_| self.executable.clone());
        parts.push(quote_windows_arg(&executable));
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
        let executable = self.executable_for_launch()?;
        if !executable_exists(&executable) {
            return Err(format!(
                "profile '{}' executable not found: {}",
                self.id, executable
            ));
        }
        Ok(())
    }

    pub fn ssh_options(&self) -> Result<SshOptions, String> {
        if self.kind != ProfileKind::Ssh {
            return Err("Это не SSH-профиль.".to_owned());
        }
        SshOptions::from_arguments(&self.args)
    }

    fn executable_for_launch(&self) -> Result<String, String> {
        if self.id == "git-bash" && self.kind == ProfileKind::Local {
            let path = if self.executable.eq_ignore_ascii_case("bash.exe") {
                executable_path(&self.executable)
            } else {
                Some(PathBuf::from(&self.executable))
            };
            if path.as_deref().is_some_and(is_wsl_bash_alias) {
                return git_bash_executable().ok_or_else(|| "Git Bash не найден: встроенный профиль указывает на псевдоним WSL. Установите Git for Windows или исправьте профиль.".to_owned());
            }
        }
        Ok(self.executable.clone())
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
        if let Some(executable) = git_bash_executable() {
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

    pub fn make_ssh_profile(
        &self,
        existing_id: Option<&str>,
        name: &str,
        options: &SshOptions,
    ) -> Result<Profile, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err("Укажите название SSH-профиля без управляющих символов.".to_owned());
        }
        let args = options.arguments()?;
        let mut profile = if let Some(id) = existing_id {
            let profile = self
                .get(id)
                .filter(|profile| profile.kind == ProfileKind::Ssh)
                .ok_or_else(|| "Сохранённый SSH-профиль не найден.".to_owned())?;
            profile.ssh_options()?; // Never erase options the editor cannot represent.
            profile.clone()
        } else {
            let executable = system_ssh_executable().ok_or_else(|| "Системный клиент OpenSSH (ssh.exe) не найден. Установите компонент OpenSSH Client Windows.".to_owned())?;
            let mut number = 1_u64;
            while self.get(&format!("ssh-{number}")).is_some() {
                number += 1;
            }
            Profile {
                id: format!("ssh-{number}"),
                name: String::new(),
                kind: ProfileKind::Ssh,
                executable,
                args: Vec::new(),
                working_directory: None,
            }
        };
        profile.name = name.to_owned();
        profile.args = args;
        profile.validate()?;
        Ok(profile)
    }

    /// Merge one SSH edit into the latest disk state under an exclusive lock.
    /// Failed or conflicting saves leave both this manager and the file intact.
    pub fn save_ssh_profile(&mut self, path: &Path, mut profile: Profile) -> Result<(), String> {
        if profile.kind != ProfileKind::Ssh {
            return Err("Можно сохранить только SSH-профиль.".to_owned());
        }
        profile.ssh_options()?;
        profile.validate()?;
        let baseline = self.get(&profile.id);
        if let Some(existing) = baseline {
            if existing.kind != ProfileKind::Ssh {
                return Err("Идентификатор занят локальным профилем.".to_owned());
            }
            existing.ssh_options()?;
        }
        let _lock = ConfigLock::acquire(path).map_err(|error| {
            if error.raw_os_error().is_some_and(|code| {
                code == ERROR_SHARING_VIOLATION.0 as i32 || code == ERROR_LOCK_VIOLATION.0 as i32
            }) {
                "Хранилище профилей занято другим окном. Повторите сохранение.".to_owned()
            } else {
                format!("Не удалось заблокировать хранилище профилей: {error}")
            }
        })?;
        let latest = Self::load_latest(path).map_err(|error| {
            format!("Хранилище профилей не прочитано. Файл не изменён: {error}")
        })?;
        if baseline.is_some()
            && latest.as_ref().and_then(|manager| manager.get(&profile.id)) != baseline
        {
            return Err("SSH-профиль изменён или удалён в другом окне. Сохранение отменено; файл не изменён.".to_owned());
        }
        if latest.is_none()
            && self
                .profiles
                .iter()
                .any(|item| item.kind == ProfileKind::Ssh)
        {
            return Err("Хранилище профилей удалено в другом окне. Сохранение отменено, чтобы не восстановить удалённые профили из устаревшего списка.".to_owned());
        }
        let mut next = latest.unwrap_or_else(|| self.initial_persistent_state());
        if baseline.is_some() {
            let existing = next
                .profiles
                .iter_mut()
                .find(|item| item.id == profile.id)
                .expect("baseline matched the latest saved profile");
            *existing = profile;
        } else {
            // Two editors may independently allocate ssh-1 before either one
            // saves. A newly created profile never replaces that foreign id.
            if next.get(&profile.id).is_some() {
                let mut number = 1_u64;
                while next.get(&format!("ssh-{number}")).is_some() {
                    number += 1;
                }
                profile.id = format!("ssh-{number}");
            }
            next.profiles.push(profile);
        }
        if next.get(&next.default_profile_id).is_none() {
            next.default_profile_id = next
                .profiles
                .first()
                .map_or(String::new(), |profile| profile.id.clone());
        }
        next.save(path)
            .map_err(|error| format!("Не удалось сохранить SSH-профиль: {error}"))?;
        *self = next;
        Ok(())
    }

    fn initial_persistent_state(&self) -> Self {
        let mut next = self.clone();
        // This discovered CLI is a runtime entry, not a persisted user profile.
        next.profiles
            .retain(|profile| profile.id != crate::codex_cli::PROFILE_ID);
        if next.get(&next.default_profile_id).is_none() {
            next.default_profile_id = next
                .profiles
                .first()
                .map_or(String::new(), |profile| profile.id.clone());
        }
        next
    }

    /// Save-side loading is strict: discovery/fallback must never overwrite a
    /// malformed file, a future schema, or an ambiguous set of profile ids.
    pub(crate) fn load_latest(path: &Path) -> io::Result<Option<Self>> {
        const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Файл профилей превышает 4 МиБ.",
            ));
        }
        let file: ProfilesFile = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut ids = BTreeSet::new();
        if file.schema_version != SCHEMA_VERSION
            || file.profiles.is_empty()
            || file.profiles.iter().any(|profile| {
                profile.id.trim().is_empty()
                    || profile.id.chars().any(char::is_control)
                    || !ids.insert(profile.id.as_str())
            })
            || !ids.contains(file.default_profile_id.as_str())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Несовместимая версия, повторные идентификаторы или неверный профиль по умолчанию.",
            ));
        }
        Ok(Some(Self {
            profiles: file.profiles,
            default_profile_id: file.default_profile_id,
        }))
    }

    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Some(manager) if !manager.profiles.is_empty() => manager,
            _ => Self::discover(),
        }
    }

    pub(crate) fn load(path: &Path) -> Option<Self> {
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
    standard
        .iter()
        .find(|path| Path::new(path).is_file())
        .map(|path| (*path).to_string())
        .or_else(|| executable_exists(exe).then(|| exe.to_owned()))
}

fn executable_exists(exe: &str) -> bool {
    executable_path(exe).is_some()
}

fn executable_path(exe: &str) -> Option<PathBuf> {
    let path = Path::new(exe);
    if path.is_absolute() || exe.contains('\\') || exe.contains('/') {
        return path.is_file().then(|| path.to_path_buf());
    }
    let path_var = env::var_os("PATH")?;
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
                return Some(candidate);
            }
        }
    }
    None
}

fn system_ssh_executable() -> Option<String> {
    let windows = env::var_os("SystemRoot").or_else(|| env::var_os("WINDIR"))?;
    let executable = PathBuf::from(windows)
        .join("System32")
        .join("OpenSSH")
        .join("ssh.exe");
    executable
        .is_file()
        .then(|| executable.to_string_lossy().into_owned())
}

fn git_bash_executable() -> Option<String> {
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(directory) = env::var_os(variable) {
            let executable = PathBuf::from(directory)
                .join("Git")
                .join("bin")
                .join("bash.exe");
            if executable.is_file() {
                return Some(executable.to_string_lossy().into_owned());
            }
        }
    }
    for executable in [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
    ] {
        if Path::new(executable).is_file() {
            return Some(executable.to_owned());
        }
    }
    let path = env::var_os("PATH")?;
    git_bash_in_paths(env::split_paths(&path), is_wsl_bash_alias)
}

fn git_bash_in_paths(
    directories: impl Iterator<Item = PathBuf>,
    is_alias: impl Fn(&Path) -> bool,
) -> Option<String> {
    directories
        .map(|directory| directory.join("bash.exe"))
        .find(|path| path.is_file() && !is_alias(path))
        .map(|path| path.to_string_lossy().into_owned())
}

fn is_wsl_bash_alias(path: &Path) -> bool {
    let mut aliases = Vec::new();
    if let Some(directory) = env::var_os("LOCALAPPDATA") {
        aliases.push(
            PathBuf::from(directory)
                .join("Microsoft")
                .join("WindowsApps")
                .join("bash.exe"),
        );
    }
    if let Some(directory) = env::var_os("SystemRoot").or_else(|| env::var_os("WINDIR")) {
        aliases.push(PathBuf::from(directory).join("System32").join("bash.exe"));
    }
    let normalized = path.to_string_lossy().replace('/', "\\");
    aliases
        .iter()
        .any(|alias| normalized.eq_ignore_ascii_case(&alias.to_string_lossy().replace('/', "\\")))
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

    #[test]
    fn ssh_alias_defaults_do_not_override_openssh_config() {
        let options = SshOptions::parse(" dev-api ", "", "", "").unwrap();
        assert_eq!(options.arguments().unwrap(), ["dev-api"]);
        assert_eq!(options.port, None);
        assert_eq!(options.user, None);
        assert_eq!(options.identity_file, None);
        let options = SshOptions::parse(
            "2001:db8::1",
            "deploy",
            "2222",
            r"C:\Users\Test User\.ssh\id_ed25519",
        )
        .unwrap();
        assert_eq!(
            options.arguments().unwrap(),
            [
                "-l",
                "deploy",
                "-p",
                "2222",
                "-i",
                r"C:\Users\Test User\.ssh\id_ed25519",
                "2001:db8::1"
            ]
        );
    }

    #[test]
    fn ssh_rejects_option_hosts_control_characters_and_invalid_ports() {
        for host in [
            "",
            "-oStrictHostKeyChecking=no",
            "two hosts",
            "host\nother",
            "user@host",
            "ssh://host",
        ] {
            assert!(SshOptions::parse(host, "", "", "").is_err(), "{host:?}");
        }
        for port in ["0", "65536", "-1", "22x", "2 2"] {
            assert!(SshOptions::parse("host", "", port, "").is_err(), "{port}");
        }
        assert!(SshOptions::parse("host", "user name", "22", "").is_err());
        assert!(SshOptions::parse("host", "user", "22", "key\0file").is_err());
        for port in ["1", "65535"] {
            assert!(SshOptions::parse("host", "", port, "").is_ok());
        }
    }

    #[test]
    fn ssh_parser_accepts_saved_simple_profiles_and_refuses_lossy_edits() {
        let mut profile = sample(
            "ssh-old",
            "ssh.exe",
            ["-p2222", "-i", "C:\\Keys\\My key", "user@example.org"]
                .map(str::to_owned)
                .to_vec(),
        );
        profile.kind = ProfileKind::Ssh;
        let options = profile.ssh_options().unwrap();
        assert_eq!(options.host, "example.org");
        assert_eq!(options.user.as_deref(), Some("user"));
        assert_eq!(options.port, Some(2222));
        assert_eq!(options.identity_file.as_deref(), Some("C:\\Keys\\My key"));
        for args in [
            vec!["-o", "ProxyJump=jump", "host"],
            vec!["host", "remote-command"],
            vec!["-i", "first", "-i", "second", "host"],
            vec!["-флаг", "host"],
            vec!["-l", "", "host"],
        ] {
            profile.args = args.into_iter().map(str::to_owned).collect();
            assert!(profile.ssh_options().is_err());
        }
    }

    fn ssh_fixture(directory: &Path) -> Profile {
        let executable = directory.join("ssh client.exe");
        fs::write(&executable, b"test fixture, never executed").unwrap();
        Profile {
            id: "ssh-1".to_owned(),
            name: "Original".to_owned(),
            kind: ProfileKind::Ssh,
            executable: executable.to_string_lossy().into_owned(),
            args: vec!["original-alias".to_owned()],
            working_directory: Some("C:\\Work folder".to_owned()),
        }
    }

    fn cleanup_ssh_fixture(directory: &Path) {
        for filename in [
            "ssh client.exe",
            "profiles.json",
            "profiles.json.lock",
            "blocker",
        ] {
            let _ = fs::remove_file(directory.join(filename));
        }
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn ssh_save_update_preserves_local_args_default_and_working_directory() {
        let directory = unique_temp_dir();
        let path = directory.join("profiles.json");
        let mut manager = ProfileManager::discover();
        let locals = manager.profiles.clone();
        let default = manager.default_profile_id.clone();
        let fixture = ssh_fixture(&directory);
        manager.save_ssh_profile(&path, fixture.clone()).unwrap();
        let options =
            SshOptions::parse("next-alias", "deploy", "2222", r"C:\Keys\Key with spaces").unwrap();
        let edited = manager
            .make_ssh_profile(Some(&fixture.id), "Updated", &options)
            .unwrap();
        assert_eq!(edited.executable, fixture.executable);
        assert_eq!(edited.working_directory, fixture.working_directory);
        assert!(edited
            .command_line()
            .contains(r#"-i "C:\Keys\Key with spaces""#));
        assert!(edited
            .args
            .iter()
            .all(|arg| !arg.contains("StrictHostKeyChecking")));
        manager.save_ssh_profile(&path, edited.clone()).unwrap();
        let loaded = ProfileManager::load_or_default(&path);
        assert_eq!(loaded.default_profile_id, default);
        assert_eq!(&loaded.profiles[..locals.len()], &locals);
        assert_eq!(loaded.get(&fixture.id), Some(&edited));
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["profiles"][locals.len()].as_object().unwrap().len(), 6);
        cleanup_ssh_fixture(&directory);
    }

    #[test]
    fn ssh_new_profile_uses_system_client_unique_id_and_failed_save_keeps_memory() {
        let directory = unique_temp_dir();
        let mut manager = ProfileManager::discover();
        let fixture = ssh_fixture(&directory);
        manager.profiles.push(fixture.clone());
        let options = SshOptions::parse("new-alias", "", "", "").unwrap();
        let created = manager.make_ssh_profile(None, "New", &options);
        match system_ssh_executable() {
            Some(executable) => {
                let created = created.unwrap();
                assert_eq!(created.id, "ssh-2");
                assert_eq!(created.executable, executable);
                assert_eq!(created.args, ["new-alias"]);
            }
            None => assert!(created.is_err()),
        }
        fs::write(directory.join("blocker"), b"not a directory").unwrap();
        let before = manager.profiles.clone();
        let edited = manager
            .make_ssh_profile(Some(&fixture.id), "Changed", &options)
            .unwrap();
        assert!(manager
            .save_ssh_profile(&directory.join("blocker").join("profiles.json"), edited)
            .is_err());
        assert_eq!(manager.profiles, before);
        assert!(manager
            .make_ssh_profile(Some("windows-cmd"), "No", &options)
            .is_err());
        cleanup_ssh_fixture(&directory);
    }

    fn persistent_fixture() -> ProfileManager {
        let mut first = sample(
            "local-one",
            r"C:\Foreign shell\missing.exe",
            vec!["--foreign-argument".to_owned()],
        );
        first.working_directory = Some(r"C:\Foreign working folder".to_owned());
        let second = sample(
            "local-two",
            "other-missing.exe",
            vec!["keep".to_owned(), "exactly".to_owned()],
        );
        let mut foreign_ssh = sample(
            "custom-ssh",
            "foreign-ssh.exe",
            vec![
                "-o".to_owned(),
                "ProxyJump=keep-me".to_owned(),
                "host".to_owned(),
            ],
        );
        foreign_ssh.kind = ProfileKind::Ssh;
        ProfileManager {
            profiles: vec![first, second, foreign_ssh],
            default_profile_id: "local-one".to_owned(),
        }
    }

    #[test]
    fn stale_new_profile_saves_merge_ids_and_keep_every_foreign_profile_and_default() {
        let directory = unique_temp_dir();
        let path = directory.join("profiles.json");
        persistent_fixture().save(&path).unwrap();
        let mut first_window = ProfileManager::load_or_default(&path);
        let mut second_window = first_window.clone();
        second_window.profiles.push(sample(
            crate::codex_cli::PROFILE_ID,
            "runtime-only.exe",
            Vec::new(),
        ));
        let first = ssh_fixture(&directory);
        let mut second = first.clone();
        second.name = "Second window profile".to_owned();
        second.args = vec!["second-alias".to_owned()];
        first_window.save_ssh_profile(&path, first.clone()).unwrap();
        let mut changed_on_disk = ProfileManager::load_or_default(&path);
        changed_on_disk.default_profile_id = "local-two".to_owned();
        changed_on_disk.profiles.push(sample(
            "another-window-profile",
            "another.exe",
            vec!["new".to_owned()],
        ));
        changed_on_disk.save(&path).unwrap();
        let preserved = changed_on_disk.profiles.clone();
        second_window
            .save_ssh_profile(&path, second.clone())
            .unwrap();
        let loaded = ProfileManager::load_or_default(&path);
        assert_eq!(loaded.default_profile_id, "local-two");
        for profile in &preserved {
            assert_eq!(loaded.get(&profile.id), Some(profile));
        }
        assert_eq!(loaded.profiles.len(), preserved.len() + 1);
        second.id = "ssh-2".to_owned();
        assert_eq!(loaded.get("ssh-1"), Some(&first));
        assert_eq!(loaded.get("ssh-2"), Some(&second));
        assert!(loaded.get(crate::codex_cli::PROFILE_ID).is_none());
        assert_eq!(second_window.profiles, loaded.profiles);
        cleanup_ssh_fixture(&directory);
    }

    #[test]
    fn stale_edits_to_different_ssh_profiles_both_survive() {
        let directory = unique_temp_dir();
        let path = directory.join("profiles.json");
        let mut manager = persistent_fixture();
        let first = ssh_fixture(&directory);
        let mut second = first.clone();
        second.id = "ssh-2".to_owned();
        second.name = "Second".to_owned();
        manager.profiles.extend([first.clone(), second.clone()]);
        manager.save(&path).unwrap();
        let mut first_window = manager.clone();
        let mut second_window = manager.clone();
        let mut edited_first = first;
        edited_first.name = "First changed".to_owned();
        let mut edited_second = second;
        edited_second.args = vec!["second-changed-alias".to_owned()];
        first_window
            .save_ssh_profile(&path, edited_first.clone())
            .unwrap();
        second_window
            .save_ssh_profile(&path, edited_second.clone())
            .unwrap();
        let loaded = ProfileManager::load_or_default(&path);
        assert_eq!(loaded.get("ssh-1"), Some(&edited_first));
        assert_eq!(loaded.get("ssh-2"), Some(&edited_second));
        for profile in &manager.profiles[..3] {
            assert_eq!(loaded.get(&profile.id), Some(profile));
        }
        assert_eq!(loaded.default_profile_id, manager.default_profile_id);
        cleanup_ssh_fixture(&directory);
    }

    #[test]
    fn stale_edit_conflicts_leave_disk_and_manager_unchanged_even_after_deletion() {
        let directory = unique_temp_dir();
        let path = directory.join("profiles.json");
        let mut first_window = persistent_fixture();
        first_window.save(&path).unwrap();
        let original = ssh_fixture(&directory);
        first_window
            .save_ssh_profile(&path, original.clone())
            .unwrap();
        let mut stale_window = first_window.clone();
        let before_memory = stale_window.profiles.clone();
        let mut edited = original.clone();
        edited.name = "Changed by first window".to_owned();
        first_window.save_ssh_profile(&path, edited).unwrap();
        let disk = fs::read(&path).unwrap();
        let mut stale_edit = original;
        stale_edit.name = "Stale second window edit".to_owned();
        let error = stale_window
            .save_ssh_profile(&path, stale_edit.clone())
            .unwrap_err();
        assert!(error.contains("другом окне"));
        assert_eq!(fs::read(&path).unwrap(), disk);
        assert_eq!(stale_window.profiles, before_memory);
        fs::remove_file(&path).unwrap();
        assert!(stale_window
            .save_ssh_profile(&path, stale_edit.clone())
            .is_err());
        stale_edit.id = "ssh-2".to_owned();
        assert!(stale_window.save_ssh_profile(&path, stale_edit).is_err());
        assert!(
            !path.exists(),
            "a stale editor must not resurrect a deleted configuration"
        );
        assert_eq!(stale_window.profiles, before_memory);
        cleanup_ssh_fixture(&directory);
    }

    #[test]
    fn malformed_future_or_ambiguous_profile_files_are_never_overwritten() {
        let directory = unique_temp_dir();
        let path = directory.join("profiles.json");
        let mut manager = persistent_fixture();
        let before_memory = manager.profiles.clone();
        let profile = ssh_fixture(&directory);
        let valid = serde_json::to_value(ProfilesFile {
            schema_version: SCHEMA_VERSION,
            default_profile_id: manager.default_profile_id.clone(),
            profiles: manager.profiles.clone(),
        })
        .unwrap();
        let mut future = valid.clone();
        future["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        let mut duplicate = valid.clone();
        duplicate["profiles"]
            .as_array_mut()
            .unwrap()
            .push(valid["profiles"][0].clone());
        let mut unknown_default = valid;
        unknown_default["default_profile_id"] = serde_json::json!("missing-profile");
        for bytes in [
            b"{broken file".to_vec(),
            serde_json::to_vec(&future).unwrap(),
            serde_json::to_vec(&duplicate).unwrap(),
            serde_json::to_vec(&unknown_default).unwrap(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(manager.save_ssh_profile(&path, profile.clone()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(manager.profiles, before_memory);
        }
        cleanup_ssh_fixture(&directory);
    }

    #[test]
    fn busy_profile_lock_fails_immediately_and_is_released_by_drop() {
        let directory = unique_temp_dir();
        let path = directory.join("profiles.json");
        let mut manager = persistent_fixture();
        manager.save(&path).unwrap();
        let before_memory = manager.profiles.clone();
        let disk = fs::read(&path).unwrap();
        let profile = ssh_fixture(&directory);
        let lock = ConfigLock::acquire(&path).unwrap();
        let start = std::time::Instant::now();
        let error = manager
            .save_ssh_profile(&path, profile.clone())
            .unwrap_err();
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(error.contains("занято другим окном"));
        assert_eq!(fs::read(&path).unwrap(), disk);
        assert_eq!(manager.profiles, before_memory);
        drop(lock);
        manager.save_ssh_profile(&path, profile).unwrap();
        assert!(manager.get("ssh-1").is_some());
        cleanup_ssh_fixture(&directory);
    }

    #[test]
    fn legacy_git_bash_alias_resolution_never_rewrites_saved_profile_or_custom_ids() {
        let windows = env::var_os("SystemRoot")
            .or_else(|| env::var_os("WINDIR"))
            .unwrap();
        let alias = PathBuf::from(windows)
            .join("System32")
            .join("bash.exe")
            .to_string_lossy()
            .into_owned();
        let mut profile = sample(
            "git-bash",
            &alias,
            vec!["--login".to_owned(), "-i".to_owned()],
        );
        let original = profile.clone();
        match git_bash_executable() {
            Some(git) => {
                assert!(profile.command_line().starts_with(&quote_windows_arg(&git)));
                assert!(profile.validate().is_ok());
            }
            None => assert!(profile.validate().is_err()),
        }
        assert_eq!(profile, original);
        profile.id = "custom-bash".to_owned();
        assert!(profile
            .command_line()
            .starts_with(&quote_windows_arg(&alias)));
        profile.id = "git-bash".to_owned();
        profile.executable = r"C:\Custom Git\bin\bash.exe".to_owned();
        assert!(profile
            .command_line()
            .starts_with(r#""C:\Custom Git\bin\bash.exe""#));
    }

    #[test]
    fn git_bash_path_search_skips_an_existing_wsl_alias_before_portable_git() {
        let directory = unique_temp_dir();
        let alias_directory = directory.join("WindowsApps");
        let git_directory = directory.join("PortableGit").join("bin");
        fs::create_dir(&alias_directory).unwrap();
        fs::create_dir_all(&git_directory).unwrap();
        let alias = alias_directory.join("bash.exe");
        let git = git_directory.join("bash.exe");
        fs::write(&alias, b"alias fixture, never executed").unwrap();
        fs::write(&git, b"portable fixture, never executed").unwrap();
        let found = git_bash_in_paths(
            [alias_directory.clone(), git_directory.clone()].into_iter(),
            |path| path == alias,
        );
        assert_eq!(found, Some(git.to_string_lossy().into_owned()));
        fs::remove_file(alias).unwrap();
        fs::remove_file(git).unwrap();
        fs::remove_dir(alias_directory).unwrap();
        fs::remove_dir(&git_directory).unwrap();
        fs::remove_dir(git_directory.parent().unwrap()).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
