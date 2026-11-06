//! Resolve a saved preset into an explicit process and an optional fresh shell.
//! No terminal output, inferred commands, or parent environment mutation is used.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::profile::{quote_windows_arg, Profile, ProfileKind, ProfileManager, PS_PROMPT};
use crate::tui_adapters::AdapterKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    #[default]
    Direct,
    Shell,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterExit {
    #[default]
    KeepTab,
    OpenShell,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchSettings {
    pub adapter: AdapterKind,
    pub mode: LaunchMode,
    pub shell_profile_id: Option<String>,
    pub after_exit: AfterExit,
    pub environment: BTreeMap<String, String>,
}

impl LaunchSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self
            .shell_profile_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.chars().any(char::is_control))
        {
            return Err("Некорректный идентификатор оболочки.".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for (key, value) in &self.environment {
            if key.is_empty()
                || key.contains(['\0', '='])
                || value.contains('\0')
                || !names.insert(key.to_uppercase())
            {
                // Neither names nor values are echoed: either may be private.
                return Err("Некорректная или повторяющаяся переменная среды.".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellFamily {
    PowerShell,
    Cmd,
    GitBash,
}

/// A shell choice is an ordinary local shell, never a CLI hosted by a shell.
pub fn is_shell_profile(profile: &Profile) -> bool {
    profile.kind == ProfileKind::Local
        && matches!(
            profile.launch.adapter,
            AdapterKind::Auto | AdapterKind::Shell
        )
        && profile.launch.mode == LaunchMode::Direct
        && shell_family(profile)
            .and_then(|family| shell_arguments(profile, family).ok())
            .is_some()
}

fn shell_family(profile: &Profile) -> Option<ShellFamily> {
    let executable = profile.executable.replace('\\', "/").to_ascii_lowercase();
    let name = executable.rsplit('/').next()?;
    match name {
        "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe" => Some(ShellFamily::PowerShell),
        "cmd" | "cmd.exe" => Some(ShellFamily::Cmd),
        "bash" | "bash.exe"
            if profile.id == "git-bash"
                || executable.ends_with("/git/bin/bash.exe")
                || executable.ends_with("/git/usr/bin/bash.exe")
                || portable_git_bash(&profile.executable) =>
        {
            Some(ShellFamily::GitBash)
        }
        _ => None,
    }
}

fn portable_git_bash(executable: &str) -> bool {
    let Some(bin) = std::path::Path::new(executable).parent() else {
        return false;
    };
    // Portable Git may have any installation directory. Its MSYS runtime
    // distinguishes it from the WSL alias or a different Bash implementation.
    (bin.join("msys-2.0.dll").is_file() && bin.join("git.exe").is_file())
        || bin.parent().is_some_and(|root| {
            bin.join("git.exe").is_file() && root.join("usr/bin/msys-2.0.dll").is_file()
        })
}

/// Keep supported configured options; reject startup programs instead of
/// silently dropping or executing an arbitrary -Command, /c, or shell script.
fn shell_arguments(profile: &Profile, family: ShellFamily) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < profile.args.len() {
        let arg = &profile.args[index];
        let lower = arg.to_ascii_lowercase();
        match family {
            ShellFamily::PowerShell => match lower.as_str() {
                "-nologo" | "-noprofile" | "-noninteractive" => result.push(arg.clone()),
                "-noexit" => {}
                "-executionpolicy" => {
                    let value = profile.args.get(index + 1).ok_or_else(shell_args_error)?;
                    if value.is_empty() || value.chars().any(char::is_control) {
                        return Err(shell_args_error());
                    }
                    result.extend([arg.clone(), value.clone()]);
                    index += 1;
                }
                "-command"
                    if profile
                        .args
                        .get(index + 1)
                        .is_some_and(|value| value == PS_PROMPT) =>
                {
                    index += 1;
                }
                _ => return Err(shell_args_error()),
            },
            ShellFamily::Cmd => match lower.as_str() {
                "/d" | "/q" | "/a" | "/u" | "/v:off" => result.push(arg.clone()),
                _ => return Err(shell_args_error()),
            },
            ShellFamily::GitBash => match arg.as_str() {
                "--login" | "-l" | "--noprofile" | "--norc" => result.push(arg.clone()),
                "-i" | "--interactive" => {}
                _ => return Err(shell_args_error()),
            },
        }
        index += 1;
    }
    Ok(result)
}

fn shell_args_error() -> String {
    "Аргументы профиля задают программу или неподдерживаемый режим оболочки. Выберите обычную локальную оболочку.".into()
}

#[derive(Clone, Debug)]
pub struct LaunchPlan {
    pub process: Profile,
    pub adapter: AdapterKind,
    pub environment: BTreeMap<String, String>,
    pub followup_shell: Option<Profile>,
    pub selected_id: String,
    pub selected_name: String,
}

impl LaunchPlan {
    pub fn resolve(manager: &ProfileManager, profile: &Profile) -> Result<Self, String> {
        validate_profile_data(profile)?;
        if profile.kind != ProfileKind::Local
            && (profile.launch.mode != LaunchMode::Direct
                || profile.launch.after_exit != AfterExit::KeepTab
                || profile.launch.shell_profile_id.is_some())
        {
            return Err(
                "Запуск через оболочку и переход после выхода доступны только локальным профилям."
                    .into(),
            );
        }
        let adapter = resolve_adapter(profile);
        let needs_shell = profile.launch.mode == LaunchMode::Shell
            || profile.launch.after_exit == AfterExit::OpenShell;
        // is_shell_profile also checks mode, so detect a recursive shell preset
        // with its direct copy rather than allowing it to wrap itself.
        let mut direct = profile.clone();
        direct.launch.mode = LaunchMode::Direct;
        if profile.launch.mode == LaunchMode::Shell && is_shell_profile(&direct) {
            return Err(
                "Оболочка запускается напрямую; выберите профиль CLI для запуска через неё.".into(),
            );
        }
        let shell = needs_shell
            .then(|| selected_shell(manager, profile.launch.shell_profile_id.as_deref()))
            .transpose()?;
        let mut environment = shell
            .as_ref()
            .filter(|_| profile.launch.mode == LaunchMode::Shell)
            .map_or_else(BTreeMap::new, |shell| shell.launch.environment.clone());
        overlay_environment(&mut environment, &profile.launch.environment);
        if profile.launch.mode == LaunchMode::Shell
            && shell.is_some_and(|shell| shell_family(shell) == Some(ShellFamily::GitBash))
        {
            if environment.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("MSYS2_ARG_CONV_EXCL") && value != "*"
            }) {
                return Err("Git Bash должен сохранять аргументы без преобразования путей: MSYS2_ARG_CONV_EXCL=*.".into());
            }
            overlay_environment(
                &mut environment,
                &BTreeMap::from([("MSYS2_ARG_CONV_EXCL".into(), "*".into())]),
            );
        }
        let mut process = if profile.launch.mode == LaunchMode::Shell {
            wrapped_process(shell.expect("shell mode resolved a shell"), profile)?
        } else {
            profile.clone()
        };
        process.launch.mode = LaunchMode::Direct;
        process.launch.after_exit = AfterExit::KeepTab;
        process.launch.shell_profile_id = None;
        let followup_shell = if profile.launch.after_exit == AfterExit::OpenShell {
            let mut shell = shell.expect("after-exit mode resolved a shell").clone();
            shell.launch.mode = LaunchMode::Direct;
            shell.launch.after_exit = AfterExit::KeepTab;
            shell.launch.shell_profile_id = None;
            shell.working_directory = profile
                .working_directory
                .clone()
                .or(shell.working_directory);
            Some(shell)
        } else {
            None
        };
        Ok(Self {
            process,
            adapter,
            environment,
            followup_shell,
            selected_id: profile.id.clone(),
            selected_name: profile.name.clone(),
        })
    }
}

pub fn resolve(manager: &ProfileManager, profile: &Profile) -> Result<LaunchPlan, String> {
    LaunchPlan::resolve(manager, profile)
}

fn selected_shell<'a>(
    manager: &'a ProfileManager,
    id: Option<&str>,
) -> Result<&'a Profile, String> {
    if let Some(id) = id {
        manager
            .get(id)
            .filter(|profile| is_shell_profile(profile))
            .ok_or_else(|| {
                "Выбранная локальная оболочка не найдена или содержит программу запуска.".into()
            })
    } else {
        manager.default_shell_profile()
    }
}

pub fn validate_profile_data(profile: &Profile) -> Result<(), String> {
    profile.launch.validate()?;
    if profile.id.trim().is_empty()
        || profile.name.trim().is_empty()
        || profile.id.chars().any(char::is_control)
        || profile.name.chars().any(char::is_control)
        || profile.executable.trim().is_empty()
        || profile.executable.contains('\0')
        || profile.args.iter().any(|arg| arg.contains('\0'))
        || profile
            .working_directory
            .as_ref()
            .is_some_and(|path| path.contains('\0'))
    {
        return Err("Профиль содержит пустое обязательное поле или недопустимый символ.".into());
    }
    Ok(())
}

pub fn resolve_adapter(profile: &Profile) -> AdapterKind {
    crate::cli_profiles::adapter_for_profile(profile)
}

fn wrapped_process(shell: &Profile, target: &Profile) -> Result<Profile, String> {
    validate_profile_data(shell)?;
    let family = shell_family(shell).ok_or_else(shell_args_error)?;
    let mut args = shell_arguments(shell, family)?;
    let executable = target.executable_for_launch()?;
    match family {
        ShellFamily::PowerShell => {
            // ProcessStartInfo bypasses PowerShell 5's lossy native argv binder.
            // Inherit stdin/stdout/stderr and the same console; no redirected
            // pipes or CREATE_NO_WINDOW are introduced for the child CLI.
            let argv = target
                .args
                .iter()
                .map(|arg| quote_windows_arg(arg))
                .collect::<Vec<_>>()
                .join(" ");
            let command = format!(
                "$ErrorActionPreference='Stop'; try {{ $s=New-Object System.Diagnostics.ProcessStartInfo; $s.FileName={}; $s.Arguments={}; $s.UseShellExecute=$false; $p=[System.Diagnostics.Process]::Start($s); $p.WaitForExit(); exit $p.ExitCode }} catch {{ [Console]::Error.WriteLine('Could not start the configured program.'); exit 1 }}",
                powershell_literal(&executable), powershell_literal(&argv)
            );
            args.extend([
                "-NonInteractive".into(),
                "-EncodedCommand".into(),
                encoded_powershell(&command),
            ]);
        }
        ShellFamily::Cmd => {
            // cmd's grammar is not CRT quoting. This restricted grammar has no
            // expansions, quotes, empty args, spaces or command separators.
            let tokens =
                std::iter::once(executable.as_str()).chain(target.args.iter().map(String::as_str));
            let mut words = Vec::new();
            for token in tokens {
                if token.is_empty()
                    || token.chars().any(|ch| {
                        ch.is_whitespace()
                            || ch.is_control()
                            || matches!(
                                ch,
                                '"' | '%' | '!' | '^' | '&' | '|' | '<' | '>' | '(' | ')'
                            )
                    })
                {
                    return Err("CMD не может точно передать эти аргументы. Выберите PowerShell 7 или Git Bash.".into());
                }
                words.push(token);
            }
            args.extend([
                "/D".into(),
                "/V:OFF".into(),
                "/S".into(),
                "/C".into(),
                words.join(" "),
            ]);
        }
        ShellFamily::GitBash => {
            // MSYS does not parse embedded quotes in Windows argv like CRT.
            // Encode literal UTF-8 bytes before constructing positional args;
            // the only executable statement is this configured child's exec.
            let values = target
                .args
                .iter()
                .map(|arg| bash_literal(arg))
                .collect::<Vec<_>>()
                .join(" ");
            let command = format!(
                "set -- {values}; exec -- {} \"$@\"",
                bash_literal(&bash_executable(&executable))
            );
            args.extend(["-c".into(), command]);
        }
    }
    let mut process = target.clone();
    process.executable = shell.executable_for_launch()?;
    process.args = args;
    process.working_directory = target
        .working_directory
        .clone()
        .or_else(|| shell.working_directory.clone());
    Ok(process)
}

fn powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn encoded_powershell(source: &str) -> String {
    const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(BASE64[(value >> 18) as usize] as char);
        result.push(BASE64[(value >> 12 & 63) as usize] as char);
        result.push(if chunk.len() > 1 {
            BASE64[(value >> 6 & 63) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            BASE64[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn bash_executable(executable: &str) -> String {
    let path = executable.replace('\\', "/");
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
        format!("/{}{}", (bytes[0] as char).to_ascii_lowercase(), &path[2..])
    } else {
        path
    }
}

fn bash_literal(value: &str) -> String {
    let mut literal = String::from("$'");
    for byte in value.as_bytes() {
        use std::fmt::Write;
        write!(&mut literal, "\\x{byte:02x}").expect("writing to a String cannot fail");
    }
    literal.push('\'');
    literal
}

fn overlay_environment(target: &mut BTreeMap<String, String>, values: &BTreeMap<String, String>) {
    for (name, value) in values {
        target.retain(|existing, _| existing.to_uppercase() != name.to_uppercase());
        target.insert(name.clone(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn profile(id: &str, executable: &str, args: &[&str]) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            kind: ProfileKind::Local,
            executable: executable.into(),
            args: args.iter().map(|value| (*value).into()).collect(),
            working_directory: None,
            launch: LaunchSettings::default(),
        }
    }

    fn manager(shell: Profile) -> ProfileManager {
        ProfileManager {
            default_profile_id: shell.id.clone(),
            profiles: vec![shell],
            default_shell_profile_id: None,
        }
    }

    #[test]
    fn direct_preserves_argv_and_defaults_migrate_without_a_shell_string() {
        let environment_before = std::env::var_os("BKS_TEST");
        let shell = profile(
            "pwsh",
            "pwsh.exe",
            &["-NoProfile", "-NoExit", "-Command", PS_PROMPT],
        );
        let mut target = profile(
            "local-1",
            r"C:\CLI with spaces\codex.exe",
            &[
                "",
                "a b",
                "quote\"here",
                r"tail\",
                "Ж🙂",
                "$(never execute)",
            ],
        );
        let original = target.clone();
        target
            .launch
            .environment
            .insert("BKS_TEST".into(), "private test value".into());
        let plan = LaunchPlan::resolve(&manager(shell), &target).unwrap();
        assert_eq!(plan.process.executable, original.executable);
        assert_eq!(plan.process.args, original.args);
        assert_eq!(plan.adapter, AdapterKind::Codex);
        assert!(plan.followup_shell.is_none());
        assert_eq!(plan.selected_id, target.id);
        assert_eq!(plan.selected_name, target.name);
        assert_eq!(plan.environment, target.launch.environment);
        assert!(std::env::var_os("BKS_TEST") == environment_before);
        let settings: LaunchSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.is_default());
        let settings: LaunchSettings =
            serde_json::from_str(r#"{"mode":"shell","after_exit":"open_shell"}"#).unwrap();
        assert_eq!(settings.mode, LaunchMode::Shell);
        assert_eq!(settings.after_exit, AfterExit::OpenShell);
    }

    #[test]
    fn selected_shell_and_global_inheritance_are_local_and_unambiguous() {
        let mut shells = manager(profile("windows-cmd", "cmd.exe", &["/Q"]));
        shells.profiles.push(profile(
            "pwsh",
            "pwsh.exe",
            &["-NoLogo", "-NoProfile", "-NoExit", "-Command", PS_PROMPT],
        ));
        shells.default_shell_profile_id = Some("pwsh".into());
        let mut target = profile("local-1", "tool.exe", &["one"]);
        target.launch.mode = LaunchMode::Shell;
        assert_eq!(
            LaunchPlan::resolve(&shells, &target)
                .unwrap()
                .process
                .executable,
            "pwsh.exe"
        );
        target.launch.shell_profile_id = Some("windows-cmd".into());
        assert_eq!(
            LaunchPlan::resolve(&shells, &target)
                .unwrap()
                .process
                .executable,
            "cmd.exe"
        );
        target.launch.shell_profile_id = Some("missing".into());
        assert!(LaunchPlan::resolve(&shells, &target).is_err());
        for mut invalid in [
            profile("shell", "node.exe", &["cli.js"]),
            profile("shell", "cmd.exe", &["/C", "program"]),
            profile("shell", "pwsh.exe", &["-Command", "program"]),
            profile("shell", "unknown-shell.exe", &[]),
            profile("shell", "bash.exe", &[]),
        ] {
            invalid.launch.adapter = AdapterKind::Shell;
            assert!(!is_shell_profile(&invalid));
        }
        let mut remote = profile("shell", "pwsh.exe", &[]);
        remote.kind = ProfileKind::Ssh;
        assert!(!is_shell_profile(&remote));
        let mut nested = shells.get("pwsh").unwrap().clone();
        nested.launch.mode = LaunchMode::Shell;
        assert!(LaunchPlan::resolve(&shells, &nested).is_err());
    }

    #[test]
    fn fresh_followup_shell_keeps_target_cwd_and_only_its_own_environment() {
        let mut shell = profile(
            "pwsh",
            "pwsh.exe",
            &["-NoProfile", "-NoExit", "-Command", PS_PROMPT],
        );
        shell.working_directory = Some(r"C:\Shell default".into());
        shell
            .launch
            .environment
            .insert("Path".into(), "shell-path".into());
        shell
            .launch
            .environment
            .insert("ONLY_SHELL".into(), "shell-value".into());
        let mut target = profile("custom-cli", "tool.exe", &[]);
        target.working_directory = Some(r"C:\CLI project".into());
        target.launch.mode = LaunchMode::Shell;
        target.launch.after_exit = AfterExit::OpenShell;
        target
            .launch
            .environment
            .insert("PATH".into(), "cli-path".into());
        target
            .launch
            .environment
            .insert("ONLY_CLI".into(), "never follow".into());
        let plan = LaunchPlan::resolve(&manager(shell.clone()), &target).unwrap();
        assert_eq!(plan.environment.get("PATH").unwrap(), "cli-path");
        assert!(!plan.environment.contains_key("Path"));
        let followup = plan.followup_shell.unwrap();
        assert_eq!(followup.working_directory, target.working_directory);
        assert_eq!(followup.launch.environment, shell.launch.environment);
        assert_eq!(followup.launch.after_exit, AfterExit::KeepTab);
        assert_eq!(followup.args, shell.args);
        target.working_directory = None;
        let inherited = LaunchPlan::resolve(&manager(shell.clone()), &target).unwrap();
        assert_eq!(inherited.process.working_directory, shell.working_directory);
        assert_eq!(
            inherited.followup_shell.unwrap().working_directory,
            shell.working_directory
        );
    }

    #[test]
    fn environment_and_cmd_reject_values_without_printing_private_content() {
        for (name, value) in [
            ("", "private"),
            ("A=B", "private"),
            ("A\0B", "private"),
            ("A", "private\0value"),
        ] {
            let mut settings = LaunchSettings::default();
            settings.environment.insert(name.into(), value.into());
            let error = settings.validate().unwrap_err();
            assert!(!error.contains("private"));
        }
        let mut settings = LaunchSettings::default();
        settings
            .environment
            .extend([("Path".into(), "a".into()), ("PATH".into(), "b".into())]);
        assert!(settings.validate().is_err());
        let shells = manager(profile("windows-cmd", "cmd.exe", &["/Q"]));
        for value in [
            "",
            "two words",
            "quote\"here",
            "%PATH%",
            "&whoami",
            "!x!",
            "(group)",
        ] {
            let mut target = profile("cli", "tool.exe", &[value]);
            target.launch.mode = LaunchMode::Shell;
            assert!(LaunchPlan::resolve(&shells, &target).is_err(), "{value:?}");
        }
    }

    struct Probe(std::path::PathBuf);

    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn native_argv_probe() -> Probe {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("bks-argv-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("argv.rs");
        fs::write(&source, r#"fn main(){for value in std::env::args().skip(1){print!("arg:");for unit in value.encode_utf16(){print!("{unit:04x}");}println!();}std::process::exit(23)}"#).unwrap();
        let rustc = std::env::var_os("RUSTC")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("USERPROFILE").unwrap())
                    .join(".cargo/bin/rustc.exe")
            });
        let output = Command::new(rustc)
            .arg(&source)
            .arg("-o")
            .arg(directory.join("argv.exe"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Probe(directory)
    }

    #[test]
    fn native_shell_wrappers_preserve_literal_argv_stdio_and_exit_status() {
        let probe = native_argv_probe();
        let mut target = profile("local-1", probe.0.join("argv.exe").to_str().unwrap(), &[]);
        let ordinary_executable = target.executable.clone();
        let spaced = probe.0.join("path with spaces");
        fs::create_dir(&spaced).unwrap();
        fs::copy(probe.0.join("argv.exe"), spaced.join("argv.exe")).unwrap();
        let spaced_executable = spaced.join("argv.exe").to_string_lossy().into_owned();
        target.launch.mode = LaunchMode::Shell;
        target.launch.adapter = AdapterKind::Universal;
        let mut shells = vec![profile(
            "powershell",
            "powershell.exe",
            &["-NoProfile", "-NoLogo"],
        )];
        let pwsh = std::path::PathBuf::from(std::env::var_os("ProgramFiles").unwrap())
            .join("PowerShell/7/pwsh.exe");
        if pwsh.is_file() {
            shells.push(profile(
                "pwsh",
                pwsh.to_str().unwrap(),
                &["-NoProfile", "-NoLogo", "-NoExit", "-Command", PS_PROMPT],
            ));
        }
        let bash = std::path::PathBuf::from(std::env::var_os("ProgramFiles").unwrap())
            .join("Git/bin/bash.exe");
        if bash.is_file() {
            shells.push(profile(
                "git-bash",
                bash.to_str().unwrap(),
                &["--noprofile", "--norc", "-i"],
            ));
        }
        for shell in shells {
            target.args = [
                "",
                "two words",
                "quote\"here",
                "'single'",
                "Ж🙂",
                "row\r\nvalue",
                r"tail\",
                "/literal/path",
                "$(never execute)",
                "%PATH%",
            ]
            .map(str::to_owned)
            .to_vec();
            let expected: String = target
                .args
                .iter()
                .map(|arg| {
                    format!(
                        "arg:{}\n",
                        arg.encode_utf16()
                            .map(|unit| format!("{unit:04x}"))
                            .collect::<String>()
                    )
                })
                .collect();
            for executable in [&ordinary_executable, &spaced_executable] {
                target.executable.clone_from(executable);
                let plan = LaunchPlan::resolve(&manager(shell.clone()), &target).unwrap();
                let output = Command::new(&plan.process.executable)
                    .args(&plan.process.args)
                    .envs(&plan.environment)
                    .output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(23),
                    "{}: {}",
                    shell.id,
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    String::from_utf8(output.stdout)
                        .unwrap()
                        .replace("\r\n", "\n"),
                    expected,
                    "{}",
                    shell.id
                );
            }
        }
        target.executable = ordinary_executable;
        if !target.executable.chars().any(char::is_whitespace) {
            target.args = ["alpha", "Ж🙂", "C:\\trailing\\", "=value"]
                .map(str::to_owned)
                .to_vec();
            let plan = LaunchPlan::resolve(
                &manager(profile("windows-cmd", "cmd.exe", &["/Q"])),
                &target,
            )
            .unwrap();
            let output = Command::new(&plan.process.executable)
                .args(&plan.process.args)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(23),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected: String = target
                .args
                .iter()
                .map(|arg| {
                    format!(
                        "arg:{}\n",
                        arg.encode_utf16()
                            .map(|unit| format!("{unit:04x}"))
                            .collect::<String>()
                    )
                })
                .collect();
            assert_eq!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .replace("\r\n", "\n"),
                expected
            );
        }
    }
}
