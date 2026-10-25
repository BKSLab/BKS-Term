//! Original application icons, extracted without starting the applications.

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL, HICON};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum IconKind {
    GitBash,
    CommandPrompt,
    Wsl,
    Ubuntu,
}

thread_local! {
    // HICON ownership stays on the UI thread; missing icons are cached as well.
    static ICONS: RefCell<HashMap<IconKind, Option<Icons>>> = RefCell::new(HashMap::new());
}

struct Icons {
    large: HICON,
    small: HICON,
}

impl Icons {
    fn extract(path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut icons = Self {
            large: HICON::default(),
            small: HICON::default(),
        };
        // Unlike shell file associations, extraction cannot supply a generic
        // executable icon when the application has no embedded icon resource.
        let count = unsafe {
            ExtractIconExW(
                PCWSTR(path.as_ptr()),
                0,
                Some(&mut icons.large),
                Some(&mut icons.small),
                1,
            )
        };
        if count == 0 || count == u32::MAX || (icons.large.is_invalid() && icons.small.is_invalid())
        {
            None
        } else {
            Some(icons)
        }
    }

    fn for_size(&self, size: i32) -> HICON {
        let (preferred, fallback) = if size <= 16 {
            (self.small, self.large)
        } else {
            (self.large, self.small)
        };
        if preferred.is_invalid() {
            fallback
        } else {
            preferred
        }
    }
}

impl Drop for Icons {
    fn drop(&mut self) {
        // ExtractIconExW returns owned icons, including its small variant.
        unsafe {
            if !self.large.is_invalid() {
                let _ = DestroyIcon(self.large);
            }
            if !self.small.is_invalid() && self.small != self.large {
                let _ = DestroyIcon(self.small);
            }
        }
    }
}

/// Draw an installed profile's original color icon, centered inside `rect`.
///
/// Returns `false` for unsupported names, absent resources, or drawing failure,
/// so the caller can draw its regular chevron. PowerShell uses that fallback.
/// Call this on the UI thread; owned icons are cached until that thread exits.
pub fn draw(hdc: HDC, rect: RECT, name: &str) -> bool {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return false;
    }
    let Some(kind) = icon_kind(name) else {
        return false;
    };
    let size = width.min(height);
    ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let Some(icons) = cache.entry(kind).or_insert_with(|| load(kind)).as_ref() else {
            return false;
        };
        unsafe {
            DrawIconEx(
                hdc,
                rect.left + (width - size) / 2,
                rect.top + (height - size) / 2,
                icons.for_size(size),
                size,
                size,
                0,
                None,
                DI_NORMAL,
            )
            .is_ok()
        }
    })
}

fn icon_kind(name: &str) -> Option<IconKind> {
    let name = name.trim().to_ascii_lowercase();
    if name.contains("powershell") || name.contains("pwsh") {
        None
    } else if name.contains("git bash") || name == "git-bash" {
        Some(IconKind::GitBash)
    } else if name.starts_with("ubuntu") {
        // A distro label must not borrow the unrelated generic WSL icon.
        Some(IconKind::Ubuntu)
    } else if name == "wsl" || name.starts_with("wsl ") || name.starts_with("wsl(") {
        Some(IconKind::Wsl)
    } else if name.starts_with("command prompt") || name == "cmd" || name == "cmd.exe" {
        Some(IconKind::CommandPrompt)
    } else {
        None
    }
}

fn load(kind: IconKind) -> Option<Icons> {
    let roots = program_files_roots();
    let mut paths = Vec::new();
    match kind {
        IconKind::GitBash => {
            for root in roots {
                git_icons(&root.join("Git"), &mut paths);
            }
            if let Some(root) = env::var_os("LOCALAPPDATA") {
                git_icons(&PathBuf::from(root).join("Programs/Git"), &mut paths);
            }
        }
        IconKind::CommandPrompt | IconKind::Wsl => {
            if let Some(root) = env::var_os("SystemRoot") {
                let system = PathBuf::from(root).join("System32");
                if kind == IconKind::CommandPrompt {
                    paths.push(system.join("cmd.exe"));
                } else {
                    paths.extend([system.join("wsl.exe"), system.join("wslg.exe")]);
                }
            }
            if kind == IconKind::Wsl {
                for root in roots {
                    paths.extend([root.join("WSL/wsl.exe"), root.join("WSL/wslg.exe")]);
                }
            }
        }
        IconKind::Ubuntu => {
            for root in roots {
                ubuntu_package_icons(&root.join("WindowsApps"), &mut paths);
            }
            if let Some(root) = env::var_os("LOCALAPPDATA") {
                ubuntu_package_icons(
                    &PathBuf::from(root).join("Microsoft/WindowsApps"),
                    &mut paths,
                );
            }
        }
    }
    paths.iter().find_map(|path| Icons::extract(path))
}

fn program_files_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for variable in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = env::var_os(variable).map(PathBuf::from) {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

fn git_icons(root: &Path, paths: &mut Vec<PathBuf>) {
    // bin/bash.exe is the interpreter and does not carry the Git Bash logo.
    paths.extend([
        root.join("git-bash.exe"),
        root.join("mingw64/share/git/git-for-windows.ico"),
        root.join("mingw32/share/git/git-for-windows.ico"),
    ]);
}

fn ubuntu_package_icons(root: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(packages) = root.read_dir() else {
        return;
    };
    for package in packages.flatten() {
        let name = package.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with("canonicalgrouplimited.ubuntu") || !name.ends_with("79rhkp1fndgsc") {
            continue;
        }
        let Ok(files) = package.path().read_dir() else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("ubuntu") && (name.ends_with(".exe") || name.ends_with(".ico")) {
                paths.push(file.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_distros_do_not_use_generic_wsl_icons() {
        assert!(matches!(icon_kind("Ubuntu (WSL)"), Some(IconKind::Ubuntu)));
        assert!(matches!(icon_kind("WSL"), Some(IconKind::Wsl)));
        assert!(icon_kind("SSH: Ubuntu").is_none());
    }

    #[test]
    fn powershell_and_unknown_profiles_keep_the_fallback() {
        for name in [
            "PowerShell",
            "Windows PowerShell",
            "PowerShell 7",
            "pwsh",
            "SSH",
        ] {
            assert!(icon_kind(name).is_none());
        }
        assert!(matches!(icon_kind("Git Bash"), Some(IconKind::GitBash)));
        assert!(matches!(
            icon_kind("Command Prompt"),
            Some(IconKind::CommandPrompt)
        ));
    }
}
