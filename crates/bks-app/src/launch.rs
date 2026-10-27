//! Strict launch options; parsing never changes cwd, registry or sessions.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub profile_id: Option<String>,
    pub working_directory: Option<PathBuf>,
    pub choose_profile: bool,
    pub explorer: Option<bool>,
    pub print_help: bool,
}

impl LaunchOptions {
    /// Pass arguments without argv[0], e.g. `std::env::args_os().skip(1)`.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            if contains_control(&argument) {
                return Err("Параметры запуска содержат управляющие символы.".to_owned());
            }
            match argument.to_str() {
                Some("--profile") if options.profile_id.is_none() => {
                    let value = next_value(&mut args, "--profile")?;
                    let id = value.to_str().ok_or_else(|| "Идентификатор профиля должен быть текстом Unicode.".to_owned())?;
                    validate_profile_id(id)?;
                    options.profile_id = Some(id.to_owned());
                }
                Some("--cwd") if options.working_directory.is_none() => {
                    let path = PathBuf::from(next_value(&mut args, "--cwd")?);
                    validate_working_directory(&path)?;
                    options.working_directory = Some(path);
                }
                Some("--choose-profile") if !options.choose_profile => options.choose_profile = true,
                Some("--register-explorer") if options.explorer.is_none() => options.explorer = Some(true),
                Some("--unregister-explorer") if options.explorer.is_none() => options.explorer = Some(false),
                Some("--help") if !options.print_help => options.print_help = true,
                Some("--profile" | "--cwd" | "--choose-profile" | "--register-explorer" | "--unregister-explorer" | "--help") => {
                    return Err("Параметр запуска указан повторно или противоречит другому параметру.".to_owned());
                }
                _ => return Err("Неизвестный параметр запуска. Доступные параметры: --profile, --cwd, --choose-profile, --register-explorer, --unregister-explorer, --help.".to_owned()),
            }
        }
        if options.choose_profile && options.profile_id.is_some() {
            return Err(
                "Выберите один способ открытия: --profile или --choose-profile.".to_owned(),
            );
        }
        let launch = options.profile_id.is_some()
            || options.working_directory.is_some()
            || options.choose_profile;
        if (options.explorer.is_some() && (launch || options.print_help))
            || (options.print_help && launch)
        {
            return Err("Справка и изменение меню Проводника запускаются отдельной командой, без параметров сессии.".to_owned());
        }
        Ok(options)
    }
}

fn next_value(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<OsString, String> {
    args.next()
        .filter(|value| !value.is_empty() && !value.to_string_lossy().starts_with("--"))
        .ok_or_else(|| format!("После {flag} требуется значение."))
}

pub(crate) fn validate_profile_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() || id.chars().count() > 128 || id.chars().any(char::is_control) {
        return Err(
            "Укажите идентификатор профиля длиной до 128 символов без управляющих символов."
                .to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn validate_working_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || contains_control(path.as_os_str())
        || path.as_os_str().encode_wide().count() > 32_767
    {
        return Err(
            "Рабочая папка должна быть абсолютным путём без управляющих символов.".to_owned(),
        );
    }
    if !path.is_dir() {
        return Err("Рабочая папка не существует или недоступна.".to_owned());
    }
    Ok(())
}

fn contains_control(value: &OsStr) -> bool {
    value.to_string_lossy().chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<LaunchOptions, String> {
        LaunchOptions::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn launch_options_preserve_unicode_profiles_and_explicit_actions() {
        assert_eq!(parse(&[]).unwrap(), LaunchOptions::default());
        assert_eq!(
            parse(&["--profile", "профиль с пробелами"])
                .unwrap()
                .profile_id
                .as_deref(),
            Some("профиль с пробелами")
        );
        assert!(parse(&["--choose-profile"]).unwrap().choose_profile);
        assert_eq!(
            parse(&["--register-explorer"]).unwrap().explorer,
            Some(true)
        );
        assert_eq!(
            parse(&["--unregister-explorer"]).unwrap().explorer,
            Some(false)
        );
        assert!(parse(&["--help"]).unwrap().print_help);
    }

    #[test]
    fn ambiguous_missing_duplicate_and_control_arguments_are_rejected() {
        for args in [
            vec!["--unknown"],
            vec!["--profile"],
            vec!["--cwd"],
            vec!["--profile", "--help"],
            vec!["--profile", ""],
            vec!["--profile", "x\0y"],
            vec!["--profile", "x\ny"],
            vec!["--help\0"],
            vec!["--cwd", "relative"],
            vec!["--cwd", r"C:relative"],
            vec!["--cwd", "C:\\bad\0path"],
            vec!["--choose-profile", "--choose-profile"],
            vec!["--help", "--help"],
            vec!["--register-explorer", "--unregister-explorer"],
            vec!["--profile", "a", "--profile", "b"],
            vec!["--choose-profile", "--profile", "a"],
            vec!["--register-explorer", "--choose-profile"],
            vec!["--unregister-explorer", "--help"],
        ] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn existing_working_directory_is_accepted_without_changing_parent_cwd() {
        // Read-only fixture: the repository exists, without changing process
        // environment or writing any user files.
        let parent = std::env::current_dir().unwrap();
        let options = LaunchOptions::parse([
            OsString::from("--choose-profile"),
            OsString::from("--cwd"),
            parent.clone().into_os_string(),
        ])
        .unwrap();
        assert_eq!(options.working_directory, Some(parent.clone()));
        assert_eq!(std::env::current_dir().unwrap(), parent);
        assert!(LaunchOptions::parse([
            OsString::from("--cwd"),
            parent.clone().into_os_string(),
            OsString::from("--cwd"),
            parent.into_os_string(),
        ])
        .is_err());
    }
}
