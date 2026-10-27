//! Optional launch profiles for installed AI CLIs. This is not a menu adapter:
//! all sessions keep the generic VT reader and require ordinary user input.
//! Discovery runs once in a worker, with isolated, bounded version/help probes.

use std::env;
use std::fs;
use std::io::{self, Read};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, HANDLE};
use windows::Win32::System::Pipes::PeekNamedPipe;

use crate::owned_child::OwnedChild;
use crate::profile::{Profile, ProfileKind};

/// Raw TUI input is an explicit CLI-profile choice. Ordinary shells retain
/// their native command editor, selection and application keyboard navigation.
pub fn starts_in_direct_mode(profile: &Profile) -> bool {
    profile.kind == ProfileKind::Local
        && (profile.id == crate::codex_cli::PROFILE_ID || profile.id.starts_with("cli-"))
}

/// A simple, explicitly submitted interactive launch can hand the same PTY
/// to its TUI. Compound commands and arguments keep the ordinary shell path.
/// This never constructs a command or derives an action from terminal output.
pub fn is_interactive_command(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty()
        || command.chars().any(|ch| {
            ch.is_control()
                || matches!(
                    ch,
                    ';' | '|' | '`' | '<' | '>' | '^' | '$' | '(' | ')' | '{' | '}'
                )
        })
    {
        return false;
    }
    let (command, call_operator) = if let Some(command) = command.strip_prefix("& ") {
        (command.trim_start(), true)
    } else {
        (command, false)
    };
    let path = if command.starts_with(['\'', '"']) {
        let quote = command.chars().next().unwrap();
        if !call_operator || command.len() < 2 || !command.ends_with(quote) {
            return false;
        }
        let path = &command[1..command.len() - 1];
        if path.contains(['\'', '"', '&']) {
            return false;
        }
        path
    } else {
        if command
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '\'' | '"' | '&'))
        {
            return false;
        }
        command
    };
    let executable = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = [".exe", ".cmd", ".bat", ".ps1"]
        .iter()
        .find_map(|suffix| executable.strip_suffix(suffix))
        .unwrap_or(&executable);
    matches!(stem, "kimi" | "codex" | "claude" | "gemini")
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const VERSION_LIMIT: usize = 4096;
const HELP_LIMIT: usize = 64 * 1024;
const MAX_SEARCH_DIRECTORIES: usize = 64;
const MAX_CANDIDATES_PER_DIRECTORY: usize = 8;
static NEXT_HOME: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default)]
pub struct Discovery {
    pub profiles: Vec<Profile>,
    /// Normal profile ID and an optional version; missing version keeps TUI usable.
    pub versions: Vec<(String, Option<String>)>,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Codex,
    Kimi,
    Gemini,
    Claude,
}

impl Tool {
    const ALL: [Self; 4] = [Self::Codex, Self::Kimi, Self::Gemini, Self::Claude];

    fn id(self) -> &'static str {
        match self {
            Self::Codex => "cli-codex",
            Self::Kimi => "cli-kimi",
            Self::Gemini => "cli-gemini",
            Self::Claude => "cli-claude",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Kimi => "kimi",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex CLI",
            Self::Kimi => "Kimi Code CLI",
            Self::Gemini => "Gemini CLI",
            Self::Claude => "Claude Code",
        }
    }

    fn accessibility_flag(self) -> Option<&'static str> {
        match self {
            Self::Gemini => Some("--screen-reader"),
            Self::Claude => Some("--ax-screen-reader"),
            Self::Codex | Self::Kimi => None,
        }
    }
}

/// No discovery or process wait is performed by the UI caller.
pub fn start_discovery() -> Receiver<Discovery> {
    start_discovery_with_directories(Vec::new())
}

/// Extra installation roots or npm .bin directories are useful for portable
/// installations and isolated checks; they do not change PATH or saved profiles.
pub fn start_discovery_with_directories(extra: Vec<PathBuf>) -> Receiver<Discovery> {
    let (sender, receiver) = mpsc::channel();
    let worker_sender = sender.clone();
    if thread::Builder::new()
        .name("bks-cli-discovery".to_owned())
        .spawn(move || {
            let directories = search_directories(extra);
            let shell = system_directory().join("cmd.exe");
            let node = directories
                .iter()
                .map(|directory| directory.join("node.exe"))
                .find(|path| executable_file(path));
            let result = discover(
                &directories,
                node.as_deref(),
                &shell,
                |launch| launch.available(),
                probe,
            );
            let _ = worker_sender.send(result);
        })
        .is_err()
    {
        let _ = sender.send(Discovery {
            notes: vec!["Не удалось запустить поиск установленных CLI.".to_owned()],
            ..Discovery::default()
        });
    }
    receiver
}

fn search_directories(extra: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for directory in extra.into_iter().take(8) {
        add_directory(&mut directories, directory);
    }
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path).take(MAX_SEARCH_DIRECTORIES) {
            add_directory(&mut directories, directory);
        }
    }
    for (variable, suffix) in [
        ("APPDATA", "npm"),
        ("USERPROFILE", ".kimi-code/bin"),
        ("USERPROFILE", ".local/bin"),
        ("LOCALAPPDATA", "Microsoft/WinGet/Links"),
        ("LOCALAPPDATA", "Programs/Codex/bin"),
        ("ProgramFiles", "nodejs"),
    ] {
        if let Some(root) = env::var_os(variable) {
            add_directory(&mut directories, PathBuf::from(root).join(suffix));
        }
    }
    directories
}

fn add_directory(directories: &mut Vec<PathBuf>, path: PathBuf) {
    if usable_path(&path)
        && !directories.iter().any(|existing| {
            existing
                .to_string_lossy()
                .eq_ignore_ascii_case(&path.to_string_lossy())
        })
    {
        directories.push(path);
    }
}

fn usable_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|text| !text.chars().any(char::is_control))
}

fn executable_file(path: &Path) -> bool {
    usable_path(path)
        && fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Launch {
    executable: PathBuf,
    args: Vec<String>,
    payload: Option<PathBuf>,
    probe_allowed: bool,
}

impl Launch {
    fn native(path: PathBuf) -> Self {
        Self {
            executable: path,
            args: Vec::new(),
            payload: None,
            probe_allowed: true,
        }
    }

    fn node(node: &Path, script: PathBuf) -> Self {
        Self {
            executable: node.to_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            payload: Some(script),
            probe_allowed: true,
        }
    }

    fn wrapper(shell: &Path, wrapper: PathBuf) -> Option<Self> {
        // Profile quotes Windows argv, not arbitrary cmd expressions. Known npm
        // scripts are resolved to native/node above. For an unknown .cmd, allow
        // only a plain path with cmd-safe characters, and never auto-run it.
        let text = wrapper.to_str()?;
        if !usable_path(&wrapper)
            || text.chars().any(|ch| {
                matches!(
                    ch,
                    '"' | '%' | '!' | '&' | '|' | '<' | '>' | '^' | '(' | ')'
                )
            })
        {
            return None;
        }
        Some(Self {
            executable: shell.to_owned(),
            // Do not add /s or embed quotes: Profile::command_line handles the
            // one wrapper path, while cmd retains its paired space-path quotes.
            args: vec![
                "/d".to_owned(),
                "/q".to_owned(),
                "/c".to_owned(),
                text.to_owned(),
            ],
            payload: Some(wrapper),
            probe_allowed: false,
        })
    }

    fn available(&self) -> bool {
        executable_file(&self.executable) && self.payload.as_deref().is_none_or(executable_file)
    }

    fn profile(&self, tool: Tool) -> Profile {
        Profile {
            id: tool.id().to_owned(),
            name: tool.name().to_owned(),
            kind: ProfileKind::Local,
            executable: self.executable.to_string_lossy().into_owned(),
            args: self.args.clone(),
            working_directory: None,
        }
    }
}

fn candidates(tool: Tool, directory: &Path, node: Option<&Path>, shell: &Path) -> Vec<Launch> {
    let mut result = vec![Launch::native(
        directory.join(format!("{}.exe", tool.command())),
    )];
    let mut package_roots = vec![directory.join("node_modules")];
    if directory.file_name().is_some_and(|name| name == ".bin") {
        if let Some(parent) = directory.parent() {
            package_roots.push(parent.to_owned());
        }
    }
    for root in package_roots {
        match tool {
            Tool::Codex => {
                let package = root.join("@openai/codex");
                for suffix in [
                    "node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe",
                    "vendor/x86_64-pc-windows-msvc/bin/codex.exe",
                ] {
                    result.push(Launch::native(package.join(suffix)));
                }
                if let Some(node) = node {
                    result.push(Launch::node(node, package.join("bin/codex.js")));
                }
            }
            Tool::Gemini => {
                if let Some(node) = node {
                    for suffix in ["bundle/gemini.js", "dist/index.js"] {
                        result.push(Launch::node(
                            node,
                            root.join("@google/gemini-cli").join(suffix),
                        ));
                    }
                }
            }
            Tool::Claude => {
                if let Some(node) = node {
                    result.push(Launch::node(
                        node,
                        root.join("@anthropic-ai/claude-code/cli.js"),
                    ));
                }
            }
            Tool::Kimi => {}
        }
    }
    if let Some(wrapper) = Launch::wrapper(shell, directory.join(format!("{}.cmd", tool.command())))
    {
        result.push(wrapper);
    }
    result
}

fn discover(
    directories: &[PathBuf],
    node: Option<&Path>,
    shell: &Path,
    mut available: impl FnMut(&Launch) -> bool,
    mut run: impl FnMut(&Launch, &str, usize) -> Option<Vec<u8>>,
) -> Discovery {
    let mut result = Discovery::default();
    for tool in Tool::ALL {
        // Probe only the first available installation. An unknown version keeps
        // that normal profile; it never causes repeated retries on UI polling.
        let launch = directories
            .iter()
            .filter(|directory| usable_path(directory))
            .take(MAX_SEARCH_DIRECTORIES + 14)
            .flat_map(|directory| {
                candidates(tool, directory, node, shell)
                    .into_iter()
                    .take(MAX_CANDIDATES_PER_DIRECTORY)
            })
            .find(|launch| available(launch));
        let Some(launch) = launch else {
            continue;
        };
        let version = if launch.probe_allowed {
            run(&launch, "--version", VERSION_LIMIT).and_then(|output| parse_version(tool, &output))
        } else {
            None
        };
        let ax = if version.is_some() && launch.probe_allowed {
            tool.accessibility_flag().is_some_and(|flag| {
                run(&launch, "--help", HELP_LIMIT).is_some_and(|output| has_flag(&output, flag))
            })
        } else {
            false
        };
        result
            .profiles
            .extend(profiles_for(tool, &launch, version.as_deref(), ax));
        if version.is_none() {
            result.notes.push(format!(
                "{}: версия не подтверждена; доступен обычный TUI без специальных параметров.",
                tool.name()
            ));
        }
        result.versions.push((tool.id().to_owned(), version));
    }
    result
}

fn profiles_for(tool: Tool, launch: &Launch, version: Option<&str>, ax: bool) -> Vec<Profile> {
    let normal = launch.profile(tool);
    let mut profiles = vec![normal.clone()];
    if version.is_some() && ax {
        if let Some(flag) = tool.accessibility_flag() {
            let mut accessible = normal;
            accessible.id.push_str("-ax");
            accessible.name.push_str(" — экранный диктор");
            accessible.args.push(flag.to_owned());
            profiles.push(accessible);
        }
    }
    profiles
}

fn parse_version(tool: Tool, output: &[u8]) -> Option<String> {
    if output.len() > VERSION_LIMIT {
        return None;
    }
    let text = std::str::from_utf8(output).ok()?.trim();
    let version = match tool {
        Tool::Codex => text.strip_prefix("codex-cli ")?,
        Tool::Claude => text.strip_suffix(" (Claude Code)")?,
        Tool::Kimi => text
            .strip_prefix("kimi, version ")
            .or_else(|| text.strip_prefix("kimi "))
            .unwrap_or(text),
        Tool::Gemini => text.strip_prefix("gemini ").unwrap_or(text),
    };
    let core = version.split(['-', '+']).next()?;
    let numbers: Vec<_> = core.split('.').collect();
    (version.len() <= 80
        && numbers.len() == 3
        && numbers
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        && !version.ends_with(['-', '+', '.']))
    .then(|| version.to_owned())
}

fn has_flag(output: &[u8], flag: &str) -> bool {
    output.len() <= HELP_LIMIT
        && std::str::from_utf8(output).is_ok_and(|text| {
            text.split_whitespace()
                .any(|word| word.trim_end_matches([',', ':']) == flag)
        })
}

struct ProbeHome(PathBuf);

impl ProbeHome {
    fn new() -> io::Result<Self> {
        let path = env::temp_dir().join(format!(
            "bks-cli-discovery-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_HOME.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for ProbeHome {
    fn drop(&mut self) {
        // Only the unique directory successfully created by this instance.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn system_directory() -> PathBuf {
    env::var_os("SystemRoot")
        .or_else(|| env::var_os("WINDIR"))
        .map(PathBuf::from)
        .filter(|path| usable_path(path))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
}

fn probe(launch: &Launch, flag: &str, limit: usize) -> Option<Vec<u8>> {
    if !launch.probe_allowed || !matches!(flag, "--version" | "--help") {
        return None;
    }
    let home = ProbeHome::new().ok()?;
    let system = system_directory();
    let mut command = Command::new(&launch.executable);
    command
        .args(&launch.args)
        .arg(flag)
        .current_dir(&home.0)
        .env_clear()
        .env("SystemRoot", system.parent()?)
        .env("WINDIR", system.parent()?)
        .env("PATH", &system)
        .env("HOME", &home.0)
        .env("USERPROFILE", &home.0)
        .env("TEMP", &home.0)
        .env("TMP", &home.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    for (variable, directory) in [
        ("APPDATA", "roaming"),
        ("LOCALAPPDATA", "local"),
        ("CODEX_HOME", "codex"),
        ("KIMI_CODE_HOME", "kimi"),
        ("KIMI_HOME", "kimi"),
        ("KIMI_SHARE_DIR", "kimi"),
        ("GEMINI_CLI_HOME", "gemini"),
        ("CLAUDE_CONFIG_DIR", "claude"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        command.env(variable, home.0.join(directory));
    }
    for (variable, value) in [
        ("CI", "1"),
        ("NO_COLOR", "1"),
        ("TERM", "dumb"),
        ("DO_NOT_TRACK", "1"),
        ("KIMI_DISABLE_TELEMETRY", "1"),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
        ("GEMINI_TELEMETRY_ENABLED", "false"),
        ("HTTP_PROXY", "http://127.0.0.1:9"),
        ("HTTPS_PROXY", "http://127.0.0.1:9"),
        ("ALL_PROXY", "http://127.0.0.1:9"),
    ] {
        command.env(variable, value);
    }
    let mut owned = OwnedChild::spawn(&mut command).ok()?;
    let result = read_child(&mut owned.child, limit);
    // Close the private job on both success and failure, before deleting the
    // isolated home. A version/help process may have spawned descendants.
    drop(owned);
    result
}

fn read_child(child: &mut Child, limit: usize) -> Option<Vec<u8>> {
    let mut stdout = child.stdout.take()?;
    let mut output = Vec::with_capacity(limit.min(4096));
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        read_available(&mut stdout, &mut output, limit).ok()?;
        if let Some(status) = child.try_wait().ok()? {
            read_available(&mut stdout, &mut output, limit).ok()?;
            return status.success().then_some(output);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_available(stdout: &mut ChildStdout, output: &mut Vec<u8>, limit: usize) -> io::Result<()> {
    loop {
        let mut available = 0;
        // SAFETY: stdout owns this live pipe; available is a writable u32 and
        // PeekNamedPipe borrows the handle without taking ownership.
        if let Err(error) = unsafe {
            PeekNamedPipe(
                HANDLE(stdout.as_raw_handle()),
                None,
                0,
                None,
                Some(&mut available),
                None,
            )
        } {
            return if error.code() == ERROR_BROKEN_PIPE.to_hresult() {
                Ok(())
            } else {
                Err(io::Error::other(error))
            };
        }
        if available == 0 {
            return Ok(());
        }
        let remaining = limit.saturating_sub(output.len());
        if remaining == 0 {
            return Err(io::Error::other("CLI probe output exceeds limit"));
        }
        let mut buffer = [0u8; 512];
        let length = buffer.len().min(remaining).min(available as usize);
        let read = stdout.read(&mut buffer[..length])?;
        if read == 0 {
            return Ok(());
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_handoff_only_recognizes_explicit_simple_cli_launches() {
        for command in [
            "kimi",
            "  KIMI.exe  ",
            "codex.cmd",
            "claude",
            "gemini.ps1",
            r"C:\Tools\kimi.exe",
            r#"& "C:\Program Files\Kimi\kimi.exe""#,
        ] {
            assert!(is_interactive_command(command), "interactive: {command}");
        }
        for command in [
            "",
            "echo kimi",
            r"echo>.\kimi",
            r"echo<.\codex",
            r"echo^x\claude",
            r"$(echo)\gemini",
            "kimi --version",
            "kimi --help",
            "claude -p test",
            "codex exec test",
            "gemini --prompt test",
            "kimi; echo done",
            "kimi | more",
            "kimi\necho done",
            "'kimi'",
            "unknown-tool",
            "& kimi --help",
        ] {
            assert!(!is_interactive_command(command), "ordinary: {command}");
        }
    }

    #[test]
    fn regular_shells_keep_the_native_editor_and_only_cli_profiles_capture_input() {
        let mut profile = Profile {
            id: "windows-powershell".to_owned(),
            name: "Test profile".to_owned(),
            kind: ProfileKind::Local,
            executable: "not-executed.exe".to_owned(),
            args: Vec::new(),
            working_directory: None,
        };
        for id in [
            "windows-powershell",
            "pwsh",
            "windows-cmd",
            "git-bash",
            "wsl",
            "custom-local",
        ] {
            profile.id = id.to_owned();
            assert!(!starts_in_direct_mode(&profile), "ordinary profile {id}");
        }
        for id in [
            crate::codex_cli::PROFILE_ID,
            "cli-codex",
            "cli-kimi",
            "cli-claude",
            "cli-gemini",
        ] {
            profile.id = id.to_owned();
            assert!(starts_in_direct_mode(&profile), "explicit CLI profile {id}");
        }
        profile.kind = ProfileKind::Ssh;
        assert!(!starts_in_direct_mode(&profile));
    }
    use crate::owned_child::ScopedHandle;
    use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    #[test]
    fn parses_installed_and_future_versions_without_pinning_normal_launch() {
        for (tool, text, version) in [
            (Tool::Codex, "codex-cli 0.164.0\r\n", "0.164.0"),
            (
                Tool::Codex,
                "codex-cli 12.99.123-beta.7",
                "12.99.123-beta.7",
            ),
            (Tool::Kimi, "2.3.3\n", "2.3.3"),
            (Tool::Kimi, "kimi, version 1.34.0", "1.34.0"),
            (Tool::Gemini, "0.65.0", "0.65.0"),
            (Tool::Claude, "2.1.240 (Claude Code)", "2.1.240"),
        ] {
            assert_eq!(
                parse_version(tool, text.as_bytes()).as_deref(),
                Some(version)
            );
        }
        for text in [
            "",
            "warning\n0.65.0",
            "0.65",
            "0.a.0",
            "0.65.0 --danger",
            "0.65.0-",
            "0.65.0\0",
        ] {
            assert!(parse_version(Tool::Gemini, text.as_bytes()).is_none());
        }
        assert!(parse_version(Tool::Gemini, &[b'1'; VERSION_LIMIT + 1]).is_none());
        assert!(parse_version(Tool::Codex, b"0.164.0").is_none());
        assert!(parse_version(Tool::Claude, b"codex-cli 0.164.0").is_none());
    }

    #[test]
    fn accessibility_options_need_exact_flag_help_and_known_version() {
        assert!(has_flag(
            b"  --screen-reader  Enable access\n",
            "--screen-reader"
        ));
        assert!(!has_flag(b"--screen-reader-unsafe", "--screen-reader"));
        assert!(!has_flag(&[0xff], "--screen-reader"));
        let launch = Launch::native(PathBuf::from(r"C:\Tools\gemini.exe"));
        let profiles = profiles_for(Tool::Gemini, &launch, Some("30.4.1"), true);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].id, "cli-gemini");
        assert!(profiles[0].args.is_empty());
        assert_eq!(profiles[1].id, "cli-gemini-ax");
        assert_eq!(profiles[1].args, ["--screen-reader"]);
        assert_eq!(profiles_for(Tool::Gemini, &launch, None, true).len(), 1);
        assert_eq!(
            profiles_for(Tool::Gemini, &launch, Some("30.4.1"), false).len(),
            1
        );
        assert_eq!(
            profiles_for(Tool::Codex, &launch, Some("30.4.1"), true).len(),
            1
        );
    }

    #[test]
    fn known_npm_layouts_resolve_without_running_shell_wrappers() {
        let node = Path::new(r"C:\Program Files\nodejs\node.exe");
        let shell = Path::new(r"C:\Windows\System32\cmd.exe");
        for directory in [
            Path::new(r"C:\npm root"),
            Path::new(r"C:\project\node_modules\.bin"),
        ] {
            let found = candidates(Tool::Gemini, directory, Some(node), shell);
            let bundled = found
                .iter()
                .find(|launch| {
                    launch
                        .payload
                        .as_deref()
                        .is_some_and(|path| path.ends_with("bundle/gemini.js"))
                })
                .unwrap();
            assert_eq!(bundled.executable, node);
            assert!(bundled.probe_allowed);
            let profile = bundled.profile(Tool::Gemini);
            assert!(profile
                .command_line()
                .starts_with(r#""C:\Program Files\nodejs\node.exe" "#));
            assert!(
                profile.args[0].ends_with("bundle\\gemini.js")
                    || profile.args[0].ends_with("bundle/gemini.js")
            );
            assert!(!profile.command_line().contains("cmd.exe"));
        }
        assert!(
            candidates(Tool::Codex, Path::new(r"C:\npm"), Some(node), shell)
                .iter()
                .any(|launch| launch.executable.ends_with("bin/codex.exe"))
        );
    }

    #[test]
    fn plain_cmd_fallback_is_quoted_by_profile_and_never_probed() {
        let shell = Path::new(r"C:\Windows\System32\cmd.exe");
        let launch = Launch::wrapper(shell, PathBuf::from(r"C:\CLI Tools\kimi.cmd")).unwrap();
        assert_eq!(
            launch.profile(Tool::Kimi).command_line(),
            r#"C:\Windows\System32\cmd.exe /d /q /c "C:\CLI Tools\kimi.cmd""#
        );
        assert!(!launch.probe_allowed);
        for path in [
            r"C:\unsafe&run\kimi.cmd",
            r"C:\%TEMP%\kimi.cmd",
            r"C:\unsafe!\kimi.cmd",
            r"C:\tools(2)\kimi.cmd",
        ] {
            assert!(Launch::wrapper(shell, PathBuf::from(path)).is_none());
        }
    }

    fn fixture_output(body: &str, limit: usize) -> Option<Vec<u8>> {
        let home = ProbeHome::new().unwrap();
        let directory = home.0.join("CLI Tools");
        fs::create_dir(&directory).unwrap();
        let wrapper = directory.join("fixture.cmd");
        fs::write(&wrapper, body).unwrap();
        let shell = system_directory().join("cmd.exe");
        let launch = Launch::wrapper(&shell, wrapper).unwrap();
        let mut profile = launch.profile(Tool::Kimi);
        profile.args.push("--version".to_owned());
        let prefix = Launch::native(shell.clone())
            .profile(Tool::Kimi)
            .command_line();
        let line = profile.command_line();
        let tail = line.strip_prefix(&prefix).unwrap().trim_start();
        let mut command = Command::new(&shell);
        command
            .raw_arg(tail)
            .current_dir(&home.0)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        let mut owned = OwnedChild::spawn(&mut command).unwrap();
        let output = read_child(&mut owned.child, limit);
        drop(owned);
        output
    }

    #[test]
    fn cmd_space_path_runs_only_the_harmless_fixture_with_exact_argument() {
        let output = fixture_output("@echo off\r\necho wrapper-ok %1\r\n", VERSION_LIMIT).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap().trim(),
            "wrapper-ok --version"
        );
    }

    #[test]
    fn process_output_limit_and_nonzero_exit_fail_closed() {
        let body = format!("@echo {}\r\n", "x".repeat(VERSION_LIMIT + 1));
        assert!(fixture_output(&body, VERSION_LIMIT).is_none());
        assert!(fixture_output("@exit /b 5\r\n", VERSION_LIMIT).is_none());
    }

    #[test]
    fn probe_timeout_closes_parent_and_descendant_holding_stdout() {
        let home = ProbeHome::new().unwrap();
        let parent_script = home.0.join("parent.ps1");
        let leaf_script = home.0.join("leaf.ps1");
        let leaf_pid_file = home.0.join("leaf.pid");
        fs::write(
            &leaf_script,
            r#"[Console]::Out.WriteLine('fixture-leaf-holds-stdout')
[Console]::Out.Flush()
[IO.File]::WriteAllText($env:BKS_DISCOVERY_LEAF_PID, [string]$PID)
Start-Sleep -Seconds 60
"#,
        )
        .unwrap();
        fs::write(
            &parent_script,
            r#"$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
$start.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $env:BKS_DISCOVERY_LEAF_SCRIPT + '"'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.RedirectStandardOutput = $true
$leaf = [Diagnostics.Process]::Start($start)
$leaf.StandardOutput.BaseStream.CopyTo([Console]::OpenStandardOutput())
$leaf.Dispose()
Start-Sleep -Seconds 60
"#,
        )
        .unwrap();
        let system = system_directory();
        let mut command = Command::new(system.join("WindowsPowerShell/v1.0/powershell.exe"));
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&parent_script)
            .current_dir(&home.0)
            .env_clear()
            .env("SystemRoot", system.parent().unwrap())
            .env("USERPROFILE", &home.0)
            .env("HOME", &home.0)
            .env("BKS_DISCOVERY_LEAF_SCRIPT", &leaf_script)
            .env("BKS_DISCOVERY_LEAF_PID", &leaf_pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut owned = OwnedChild::spawn(&mut command).unwrap();
        let mut prefix = Vec::new();
        let ready_deadline = Instant::now() + Duration::from_secs(8);
        let leaf_pid = loop {
            read_available(
                owned.child.stdout.as_mut().unwrap(),
                &mut prefix,
                VERSION_LIMIT,
            )
            .unwrap();
            let pid = fs::read_to_string(&leaf_pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            if let Some(pid) = pid.filter(|_| {
                prefix
                    .windows(b"fixture-leaf-holds-stdout".len())
                    .any(|part| part == b"fixture-leaf-holds-stdout")
            }) {
                break pid;
            }
            assert!(
                Instant::now() < ready_deadline,
                "Fixture descendant did not start"
            );
            thread::sleep(Duration::from_millis(10));
        };
        // IDs come only from this newly spawned private fixture. Keep handles
        // through job destruction to verify both exits without PID reuse races.
        let parent = ScopedHandle(
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, owned.child.id()) }.unwrap(),
        );
        let leaf =
            ScopedHandle(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, leaf_pid) }.unwrap());
        assert_eq!(unsafe { WaitForSingleObject(parent.0, 0) }, WAIT_TIMEOUT);
        assert_eq!(unsafe { WaitForSingleObject(leaf.0, 0) }, WAIT_TIMEOUT);
        let started = Instant::now();
        assert!(read_child(&mut owned.child, VERSION_LIMIT).is_none());
        assert!(started.elapsed() >= PROBE_TIMEOUT);
        drop(owned);
        assert_eq!(
            unsafe { WaitForSingleObject(parent.0, 2000) },
            WAIT_OBJECT_0
        );
        assert_eq!(unsafe { WaitForSingleObject(leaf.0, 2000) }, WAIT_OBJECT_0);
    }

    #[test]
    fn failures_preserve_normal_profiles_and_unknown_version_disables_ax() {
        let directories = vec![PathBuf::from(r"C:\Tools")];
        let result = discover(
            &directories,
            None,
            Path::new(r"C:\Windows\System32\cmd.exe"),
            |launch| launch.payload.is_none(),
            |_, _, _| None,
        );
        assert_eq!(result.profiles.len(), 4);
        assert_eq!(result.notes.len(), 4);
        assert!(result.versions.iter().all(|(_, version)| version.is_none()));
        assert!(result
            .profiles
            .iter()
            .all(|profile| profile.args.is_empty() && !profile.id.ends_with("-ax")));
    }

    #[test]
    fn discovery_runs_one_version_probe_per_found_tool_and_only_relevant_help() {
        let directories = vec![PathBuf::from(r"C:\Tools"), PathBuf::from(r"C:\Other")];
        let mut probes = Vec::new();
        let result = discover(
            &directories,
            None,
            Path::new(r"C:\Windows\System32\cmd.exe"),
            |launch| launch.payload.is_none(),
            |launch, flag, limit| {
                probes.push((launch.executable.clone(), flag.to_owned(), limit));
                if flag == "--help" {
                    return Some(b"--screen-reader --ax-screen-reader".to_vec());
                }
                Some(
                    match launch.executable.file_stem().unwrap().to_str().unwrap() {
                        "codex" => b"codex-cli 90.0.1".to_vec(),
                        "claude" => b"90.0.1 (Claude Code)".to_vec(),
                        _ => b"90.0.1".to_vec(),
                    },
                )
            },
        );
        assert_eq!(result.profiles.len(), 6);
        assert_eq!(probes.len(), 6);
        assert_eq!(
            probes
                .iter()
                .filter(|(_, flag, _)| flag == "--version")
                .count(),
            4
        );
        assert!(probes
            .iter()
            .all(|(path, _, _)| path.starts_with(r"C:\Tools")));
        assert!(probes
            .iter()
            .filter(|(_, flag, _)| flag == "--version")
            .all(|(_, _, limit)| *limit == VERSION_LIMIT));
        assert!(result.notes.is_empty());
    }

    #[test]
    fn wrapper_fallback_and_absence_do_not_spawn_probe_commands() {
        let directories = vec![PathBuf::from(r"C:\Tools")];
        let result = discover(
            &directories,
            None,
            Path::new(r"C:\Windows\System32\cmd.exe"),
            |launch| !launch.probe_allowed,
            |_, _, _| panic!("Unverified wrapper must not be executed by discovery"),
        );
        assert_eq!(result.profiles.len(), 4);
        assert!(result.versions.iter().all(|(_, version)| version.is_none()));
        let missing = discover(
            &directories,
            None,
            Path::new(r"C:\Windows\System32\cmd.exe"),
            |_| false,
            |_, _, _| panic!("Missing CLI must not be executed"),
        );
        assert!(missing.profiles.is_empty());
    }

    #[test]
    fn search_ignores_relative_and_duplicate_paths() {
        let mut directories = Vec::new();
        add_directory(&mut directories, PathBuf::from("relative"));
        add_directory(&mut directories, PathBuf::from(r"C:\Tools"));
        add_directory(&mut directories, PathBuf::from(r"c:\tools"));
        add_directory(&mut directories, PathBuf::from("C:\\Tools\n"));
        assert_eq!(directories, [PathBuf::from(r"C:\Tools")]);
    }
}
