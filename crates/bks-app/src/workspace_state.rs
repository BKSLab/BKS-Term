//! Opt-in recent profile metadata. This is deliberately not a session snapshot:
//! no arguments, commands, drafts, terminal output or credentials can be stored.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub const MAX_RECENT: usize = 20;
const MAX_FILE_BYTES: u64 = 4_194_304;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProfile {
    pub profile_id: String,
    pub working_directory: Option<PathBuf>,
}

impl RecentProfile {
    fn validate(&self) -> io::Result<()> {
        if self.profile_id.trim().is_empty()
            || self.profile_id.chars().count() > 128
            || self.profile_id.chars().any(char::is_control)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Недопустимый идентификатор недавнего профиля.",
            ));
        }
        if let Some(path) = &self.working_directory {
            if !path.is_absolute()
                || path.as_os_str().encode_wide().count() > 32_767
                || path.to_string_lossy().chars().any(char::is_control)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Рабочая папка должна быть абсолютным путём без управляющих символов.",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceState {
    /// Most recently selected profile/folder first. Selecting an entry is an
    /// explicit user action; loading this list never starts a process.
    pub recent: Vec<RecentProfile>,
}

#[derive(Serialize, Deserialize)]
struct WorkspaceFile {
    version: u32,
    recent: Vec<RecentProfile>,
}

impl WorkspaceState {
    pub fn load(path: &Path) -> io::Result<Self> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Список недавних профилей слишком велик.",
            ));
        }
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let file: WorkspaceFile = serde_json::from_slice(bytes).map_err(invalid_data)?;
        if file.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Неизвестная версия списка недавних профилей.",
            ));
        }
        let mut state = Self::default();
        for entry in file.recent {
            if state.recent.len() == MAX_RECENT {
                break;
            }
            if entry.validate().is_ok() && !state.recent.contains(&entry) {
                state.recent.push(entry);
            }
        }
        Ok(state)
    }

    pub fn record(&mut self, profile_id: &str, working_directory: Option<&Path>) -> io::Result<()> {
        let entry = RecentProfile {
            profile_id: profile_id.to_owned(),
            working_directory: working_directory.map(Path::to_path_buf),
        };
        entry.validate()?;
        self.recent.retain(|previous| previous != &entry);
        self.recent.insert(0, entry);
        self.recent.truncate(MAX_RECENT);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.recent.clear();
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = self.to_bytes()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, bytes)?;
        let from: Vec<_> = temporary.as_os_str().encode_wide().chain([0]).collect();
        let to: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        if let Err(error) = unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } {
            let _ = fs::remove_file(temporary);
            return Err(io::Error::other(error));
        }
        Ok(())
    }

    fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let recent = self
            .recent
            .iter()
            .take(MAX_RECENT)
            .cloned()
            .collect::<Vec<_>>();
        for entry in &recent {
            entry.validate()?;
        }
        serde_json::to_vec_pretty(&WorkspaceFile { version: 1, recent }).map_err(io::Error::other)
    }
}

fn invalid_data(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_metadata_is_bounded_deduplicated_and_ordered() {
        let mut state = WorkspaceState::default();
        for index in 0..(MAX_RECENT + 4) {
            state.record(&format!("profile-{index}"), None).unwrap();
        }
        assert_eq!(state.recent.len(), MAX_RECENT);
        assert_eq!(state.recent[0].profile_id, "profile-23");
        state
            .record("profile-10", Some(Path::new(r"C:\work")))
            .unwrap();
        state.record("profile-10", None).unwrap();
        state.record("profile-10", None).unwrap();
        assert_eq!(state.recent[0].profile_id, "profile-10");
        assert_eq!(state.recent.len(), MAX_RECENT);
        assert_eq!(
            state
                .recent
                .iter()
                .filter(
                    |entry| entry.profile_id == "profile-10" && entry.working_directory.is_none()
                )
                .count(),
            1
        );
    }

    #[test]
    fn persistence_contains_only_identifiers_and_absolute_folders() {
        let input = br#"{"version":1,"recent":[{"profile_id":"ssh-home","working_directory":"C:\\work","command":"DO NOT STORE","draft":"DO NOT STORE","secret":"DO NOT STORE"}],"transcript":"DO NOT STORE"}"#;
        let state = WorkspaceState::from_bytes(input).unwrap();
        let bytes = state.to_bytes().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("DO NOT STORE"));
        assert!(!text.contains("command"));
        assert!(!text.contains("transcript"));
        assert_eq!(WorkspaceState::from_bytes(&bytes).unwrap(), state);
    }

    #[test]
    fn malformed_metadata_never_becomes_a_launch_instruction() {
        let mut state = WorkspaceState::default();
        assert!(state.record("", None).is_err());
        assert!(state.record("profile\ncommand", None).is_err());
        assert!(state.record("local", Some(Path::new("relative"))).is_err());
        assert!(state
            .record("local", Some(Path::new(r"C:relative")))
            .is_err());
        assert!(state.recent.is_empty());
        assert!(WorkspaceState::from_bytes(br#"{"version":2,"recent":[]}"#).is_err());
        let loaded = WorkspaceState::from_bytes(br#"{"version":1,"recent":[{"profile_id":"a","working_directory":"relative"},{"profile_id":"b","working_directory":null}]}"#).unwrap();
        assert_eq!(loaded.recent.len(), 1);
        assert_eq!(loaded.recent[0].profile_id, "b");
        state.record("local", None).unwrap();
        state.clear();
        assert!(state.recent.is_empty());
    }
}
