use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use crate::output_speech::SpeechMode;
use crate::shortcuts::ShortcutMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LastTabPolicy {
    #[default]
    KeepWindow,
    CloseWindow,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub speech_mode: SpeechMode,
    pub shortcuts: ShortcutMap,
    pub last_tab: LastTabPolicy,
    /// Only safe profile/folder metadata; never automatically resume sessions.
    pub remember_workspace: bool,
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_if_unchanged(&self, path: &Path, expected: &Self) -> io::Result<()> {
        let _lock = crate::config_lock::ConfigLock::acquire(path)?;
        if Self::load(path) != *expected {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Настройки изменены в другом окне. Откройте их заново.",
            ));
        }
        self.save_atomic(path)
    }

    pub fn update_speech(path: &Path, speech_mode: SpeechMode) -> io::Result<Self> {
        let _lock = crate::config_lock::ConfigLock::acquire(path)?;
        let mut next = Self::load(path);
        next.speech_mode = speech_mode;
        next.save_atomic(path)?;
        Ok(next)
    }

    fn save_atomic(&self, path: &Path) -> io::Result<()> {
        self.shortcuts
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_preferences_cannot_overwrite_other_windows_and_speech_merges_latest() {
        let name = format!(
            "bks-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(name);
        fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.json");
        let baseline = Settings::default();
        let changed = Settings {
            remember_workspace: true,
            ..baseline.clone()
        };
        changed.save_if_unchanged(&path, &baseline).unwrap();
        let stale = Settings {
            speech_mode: SpeechMode::All,
            ..baseline.clone()
        };
        assert_eq!(
            stale
                .save_if_unchanged(&path, &baseline)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(Settings::load(&path).remember_workspace);
        let merged = Settings::update_speech(&path, SpeechMode::All).unwrap();
        assert!(merged.remember_workspace);
        assert_eq!(merged.shortcuts, changed.shortcuts);
        {
            let _locked = crate::config_lock::ConfigLock::acquire(&path).unwrap();
            assert!(Settings::update_speech(&path, SpeechMode::Manual).is_err());
        }
        fs::remove_file(&path).unwrap();
        fs::remove_file(directory.join("settings.json.lock")).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn invalid_settings_keep_the_quiet_default_and_modes_round_trip() {
        assert_eq!(
            serde_json::from_str::<Settings>("{}").unwrap().speech_mode,
            SpeechMode::Important
        );
        for mode in [SpeechMode::Manual, SpeechMode::Important, SpeechMode::All] {
            let settings = Settings {
                speech_mode: mode,
                ..Settings::default()
            };
            let bytes = serde_json::to_vec(&settings).unwrap();
            assert_eq!(
                serde_json::from_slice::<Settings>(&bytes)
                    .unwrap()
                    .speech_mode,
                mode
            );
        }
        assert!(serde_json::from_str::<Settings>(r#"{"speech_mode":"unknown"}"#).is_err());
    }

    #[test]
    fn legacy_settings_keep_existing_mode_and_safe_workspace_defaults() {
        let old: Settings = serde_json::from_str(r#"{"speech_mode":"all"}"#).unwrap();
        assert_eq!(old.speech_mode, SpeechMode::All);
        assert_eq!(old.last_tab, LastTabPolicy::KeepWindow);
        assert!(!old.remember_workspace);
        assert_eq!(old.shortcuts, ShortcutMap::default());
        let updated = Settings {
            last_tab: LastTabPolicy::CloseWindow,
            remember_workspace: true,
            ..old
        };
        let bytes = serde_json::to_vec(&updated).unwrap();
        assert_eq!(serde_json::from_slice::<Settings>(&bytes).unwrap(), updated);
    }
}
