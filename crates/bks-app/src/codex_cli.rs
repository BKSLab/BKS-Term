//! Optional discovery of the direct native Codex CLI supported by the TUI adapter.
//! No user config or credentials are read. The only child command is --version.

use std::env;
use std::fs;
use std::io::{self, Read};
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, HANDLE};
use windows::Win32::System::Pipes::PeekNamedPipe;

use crate::profile::{Profile, ProfileKind};
use crate::tui_dialog::SUPPORTED_CODEX_VERSION;

pub const PROFILE_ID: &str = "codex-tui-0.164.0";

const VERSION_OUTPUT_LIMIT: usize = 4096;
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const NPM_TARGET: &str = "x86_64-pc-windows-msvc";

#[derive(Clone, Debug)]
pub struct SupportedCli {
    native_path: PathBuf,
    version: String,
    stamp: FileStamp,
}

/// Identity captured before a session starts, independently of asynchronous
/// version discovery. Fields stay private so a timer cannot invent evidence.
#[derive(Clone, Debug)]
pub struct LaunchEvidence {
    profile_id: String,
    executable: String,
    native_path: PathBuf,
    stamp: FileStamp,
}

impl LaunchEvidence {
    pub fn capture(profile: &Profile) -> Option<Self> {
        if !direct_profile(profile) {
            return None;
        }
        let native_path = fs::canonicalize(&profile.executable).ok()?;
        if native_path.to_str().is_none()
            || !native_path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
        {
            return None;
        }
        Some(Self {
            stamp: FileStamp::read(&native_path).ok()?,
            profile_id: profile.id.clone(),
            executable: profile.executable.clone(),
            native_path,
        })
    }

    pub fn native_path(&self) -> &Path {
        &self.native_path
    }
}

fn direct_profile(profile: &Profile) -> bool {
    [PROFILE_ID, "cli-codex"].contains(&profile.id.as_str())
        && profile.kind == ProfileKind::Local
        && profile.args.is_empty()
        && Path::new(&profile.executable).is_absolute()
}

/// Deliberately excludes last-access time: executing --version can update it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    created: u64,
    last_written: u64,
    attributes: u32,
}

impl FileStamp {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a native executable file",
            ));
        }
        Ok(Self {
            size: metadata.file_size(),
            created: metadata.creation_time(),
            last_written: metadata.last_write_time(),
            attributes: metadata.file_attributes(),
        })
    }
}

impl SupportedCli {
    pub fn profile(&self) -> Profile {
        Profile {
            id: PROFILE_ID.to_owned(),
            name: "Codex CLI 0.164.0".to_owned(),
            kind: ProfileKind::Local,
            // Discovery rejects paths that cannot be represented by Profile.
            executable: self.native_path.to_string_lossy().into_owned(),
            args: Vec::new(),
            working_directory: None,
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn matches(&self, profile: &Profile) -> bool {
        LaunchEvidence::capture(profile)
            .is_some_and(|evidence| self.version_for_launch(profile, &evidence).is_some())
    }

    /// Pure comparison with the executable selected before PTY spawn. It does
    /// not inspect today's file at that path or revoke an already running CLI.
    pub fn version_for_launch<'a>(
        &'a self,
        profile: &Profile,
        evidence: &LaunchEvidence,
    ) -> Option<&'a str> {
        (direct_profile(profile)
            && evidence.profile_id == profile.id
            && evidence.executable == profile.executable
            && evidence.native_path == self.native_path
            && evidence.stamp == self.stamp
            && self.version == SUPPORTED_CODEX_VERSION)
            .then_some(self.version())
    }
}

/// Runs exactly one discovery worker; the caller can poll the receiver from UI.
/// Missing/unsupported installations produce None, never a shell-wrapper profile.
pub fn start_discovery() -> Receiver<Option<SupportedCli>> {
    let (sender, receiver) = mpsc::channel();
    let worker_sender = sender.clone();
    let spawned = thread::Builder::new()
        .name("bks-codex-discovery".to_owned())
        .spawn(move || {
            let appdata = env::var_os("APPDATA").map(PathBuf::from);
            let path_dirs = env::var_os("PATH")
                .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
                .unwrap_or_default();
            let found = discover(candidates(appdata.as_deref(), path_dirs));
            let _ = worker_sender.send(found);
        });
    if spawned.is_err() {
        let _ = sender.send(None);
    }
    receiver
}

fn candidates(appdata: Option<&Path>, path_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(appdata) = appdata.filter(|path| path.is_absolute()) {
        add_npm_candidates(&mut candidates, &appdata.join("npm"));
    }
    // A PATH entry may contain codex.exe directly or npm's codex.cmd/ps1
    // wrappers. Inspect the wrappers' known package locations; never run them.
    for directory in path_dirs.into_iter().filter(|path| path.is_absolute()) {
        add_candidate(&mut candidates, directory.join("codex.exe"));
        add_npm_candidates(&mut candidates, &directory);
    }
    candidates
}

fn add_npm_candidates(candidates: &mut Vec<PathBuf>, directory: &Path) {
    let package = directory.join("node_modules").join("@openai").join("codex");
    add_candidate(
        candidates,
        package
            .join("node_modules/@openai/codex-win32-x64/vendor")
            .join(NPM_TARGET)
            .join("bin/codex.exe"),
    );
    add_candidate(
        candidates,
        package
            .join("vendor")
            .join(NPM_TARGET)
            .join("bin/codex.exe"),
    );
}

fn add_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !candidates.contains(&path) {
        candidates.push(path);
    }
}

fn discover(candidates: Vec<PathBuf>) -> Option<SupportedCli> {
    let mut checked_paths = Vec::new();
    for candidate in candidates {
        let Ok(native_path) = fs::canonicalize(&candidate) else {
            continue;
        };
        if native_path.to_str().is_none()
            || !native_path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
            || checked_paths.contains(&native_path)
        {
            continue;
        }
        checked_paths.push(native_path.clone());
        let Ok(stamp) = FileStamp::read(&native_path) else {
            continue;
        };
        let Some(version) = checked_version(&native_path) else {
            continue;
        };
        if FileStamp::read(&native_path).ok() != Some(stamp)
            || fs::canonicalize(&candidate).ok().as_ref() != Some(&native_path)
        {
            continue;
        }
        return Some(SupportedCli {
            native_path,
            version: version.to_owned(),
            stamp,
        });
    }
    None
}

fn parse_version(output: &[u8]) -> Option<&'static str> {
    match output {
        b"codex-cli 0.164.0" | b"codex-cli 0.164.0\n" | b"codex-cli 0.164.0\r\n" => {
            Some(SUPPORTED_CODEX_VERSION)
        }
        _ => None,
    }
}

fn checked_version(native_path: &Path) -> Option<&'static str> {
    let mut child = Command::new(native_path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let result = read_version_child(&mut child);
    // kill addresses only the child returned by this specific spawn. On a
    // successful exit this is harmless; on timeout/error it prevents an orphan.
    if result.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn read_version_child(child: &mut Child) -> Option<&'static str> {
    let mut stdout = child.stdout.take()?;
    let mut output = Vec::with_capacity(VERSION_OUTPUT_LIMIT);
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        read_available(&mut stdout, &mut output).ok()?;
        if let Some(status) = child.try_wait().ok()? {
            // Include the child's final bytes if it exited after the last peek.
            read_available(&mut stdout, &mut output).ok()?;
            return status.success().then(|| parse_version(&output)).flatten();
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Peek before each read to keep the single discovery worker responsive even
/// when a version child is silent. The output allocation never exceeds 4 KiB.
fn read_available(stdout: &mut ChildStdout, output: &mut Vec<u8>) -> io::Result<()> {
    loop {
        let mut available = 0;
        // SAFETY: stdout owns this live pipe handle, PeekNamedPipe does not take
        // ownership, and available is a valid writable u32 for the call.
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
            if error.code() == ERROR_BROKEN_PIPE.to_hresult() {
                return Ok(());
            }
            return Err(io::Error::other(error));
        }
        if available == 0 {
            return Ok(());
        }
        let remaining = VERSION_OUTPUT_LIMIT.saturating_sub(output.len());
        if remaining == 0 {
            return Err(io::Error::other("version output exceeds limit"));
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "bks-codex-discovery-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::write(
                path.join("codex.exe"),
                b"fake native fixture, never executed",
            )
            .unwrap();
            Self(path)
        }

        fn supported(&self) -> SupportedCli {
            let native_path = fs::canonicalize(self.0.join("codex.exe")).unwrap();
            SupportedCli {
                stamp: FileStamp::read(&native_path).unwrap(),
                native_path,
                version: SUPPORTED_CODEX_VERSION.to_owned(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in ["codex.exe", "other.exe", "codex-renamed.exe", "codex.cmd"] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn accepts_only_exact_supported_native_version() {
        for valid in [
            &b"codex-cli 0.164.0"[..],
            &b"codex-cli 0.164.0\n"[..],
            &b"codex-cli 0.164.0\r\n"[..],
        ] {
            assert_eq!(parse_version(valid), Some("0.164.0"));
        }
        for invalid in [
            &b"codex-cli0.164.0"[..],
            &b"codex-cli 0.164.1\n"[..],
            &b"codex-cli 0.164.0-beta\n"[..],
            &b"codex-cli 0.164.0\nextra output"[..],
            &b"Codex CLI 0.164.0"[..],
            &b" codex-cli 0.164.0\n"[..],
            &b"codex-cli 0.164.0 \n"[..],
            &b"codex-cli 0.164.0\n\n"[..],
            &b"\xffcodex-cli 0.164.0"[..],
            &b""[..],
        ] {
            assert_eq!(parse_version(invalid), None);
        }
    }

    #[test]
    fn candidates_include_both_npm_layouts_without_shell_wrappers() {
        let appdata = Path::new(r"C:\fixture\AppData\Roaming");
        let paths = candidates(Some(appdata), vec![PathBuf::from(r"C:\tools")]);
        assert_eq!(paths[0], appdata.join(r"npm\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe"));
        assert_eq!(
            paths[1],
            appdata.join(
                r"npm\node_modules\@openai\codex\vendor\x86_64-pc-windows-msvc\bin\codex.exe"
            )
        );
        assert!(paths.contains(&PathBuf::from(r"C:\tools\codex.exe")));
        assert!(paths
            .iter()
            .all(|path| path.file_name().unwrap() == "codex.exe"));
        assert!(paths.iter().all(|path| path.is_absolute()));
    }

    #[test]
    fn empty_and_relative_environment_entries_are_ignored_and_candidates_deduplicated() {
        let directory = PathBuf::from(r"C:\tools");
        let paths = candidates(
            Some(Path::new("relative-appdata")),
            vec![
                PathBuf::new(),
                PathBuf::from("relative"),
                directory.clone(),
                directory,
            ],
        );
        assert_eq!(paths.len(), 3);
        assert!(paths.iter().all(|path| path.is_absolute()));
    }

    #[test]
    fn generated_profile_is_direct_and_has_no_arguments() {
        let fixture = Fixture::new();
        let supported = fixture.supported();
        let profile = supported.profile();
        assert_eq!(profile.id, PROFILE_ID);
        assert_eq!(profile.name, "Codex CLI 0.164.0");
        assert_eq!(profile.kind, ProfileKind::Local);
        assert!(Path::new(&profile.executable).is_absolute());
        assert!(profile.args.is_empty());
        assert!(profile.working_directory.is_none());
        assert_eq!(supported.version(), "0.164.0");
        assert!(supported.matches(&profile));
    }

    #[test]
    fn normal_discovered_profile_uses_adapter_only_for_the_verified_native_file() {
        let fixture = Fixture::new();
        let supported = fixture.supported();
        let mut profile = supported.profile();
        profile.id = "cli-codex".to_owned();
        assert!(supported.matches(&profile));
        profile.args.push("--no-alt-screen".to_owned());
        assert!(!supported.matches(&profile));
    }

    #[test]
    fn late_discovery_matches_launch_evidence_without_reopening_the_file() {
        let fixture = Fixture::new();
        let profile = fixture.supported().profile();
        let evidence = LaunchEvidence::capture(&profile).unwrap();
        // Discovery completes after the session has already captured its image.
        let discovered = fixture.supported();
        fs::rename(
            fixture.0.join("codex.exe"),
            fixture.0.join("codex-renamed.exe"),
        )
        .unwrap();
        assert!(!discovered.matches(&profile));
        assert_eq!(
            discovered.version_for_launch(&profile, &evidence),
            Some(SUPPORTED_CODEX_VERSION)
        );
        let mut changed = profile.clone();
        changed.args.push("--no-alt-screen".to_owned());
        assert!(discovered.version_for_launch(&changed, &evidence).is_none());
        changed = profile;
        changed.id = "unknown-executable".to_owned();
        assert!(discovered.version_for_launch(&changed, &evidence).is_none());
    }

    #[test]
    fn a_replaced_file_cannot_verify_an_earlier_launch_or_revoke_its_old_identity() {
        let fixture = Fixture::new();
        let original = fixture.supported();
        let profile = original.profile();
        let evidence = LaunchEvidence::capture(&profile).unwrap();
        fs::write(fixture.0.join("codex.exe"), b"different replacement image").unwrap();
        let replacement = fixture.supported();
        assert!(replacement
            .version_for_launch(&profile, &evidence)
            .is_none());
        assert_eq!(
            original.version_for_launch(&profile, &evidence),
            Some(SUPPORTED_CODEX_VERSION)
        );
        let mut unknown = original;
        unknown.version = "0.165.0".to_owned();
        assert!(unknown.version_for_launch(&profile, &evidence).is_none());
    }

    #[test]
    fn matching_rejects_args_other_ids_shells_relative_or_different_paths() {
        let fixture = Fixture::new();
        let supported = fixture.supported();
        let profile = supported.profile();
        let mut changed = profile.clone();
        changed.id = "local-shell".to_owned();
        assert!(!supported.matches(&changed));
        changed = profile.clone();
        changed.args = vec!["--version".to_owned()];
        assert!(!supported.matches(&changed));
        changed = profile.clone();
        changed.kind = ProfileKind::Ssh;
        assert!(!supported.matches(&changed));
        changed = profile.clone();
        changed.executable = "codex.exe".to_owned();
        assert!(!supported.matches(&changed));
        changed = profile;
        fs::write(fixture.0.join("other.exe"), b"different fake executable").unwrap();
        changed.executable = fixture.0.join("other.exe").to_string_lossy().into_owned();
        assert!(!supported.matches(&changed));
        fs::write(fixture.0.join("codex.cmd"), b"fake wrapper, never executed").unwrap();
        changed.executable = fixture.0.join("codex.cmd").to_string_lossy().into_owned();
        assert!(!supported.matches(&changed));
    }

    #[test]
    fn matching_rejects_changed_size_timestamp_deleted_file_or_version() {
        let fixture = Fixture::new();
        let supported = fixture.supported();
        let profile = supported.profile();
        fs::write(fixture.0.join("codex.exe"), b"changed").unwrap();
        assert!(!supported.matches(&profile));
        let supported = fixture.supported();
        fs::File::options()
            .write(true)
            .open(fixture.0.join("codex.exe"))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
        assert!(!supported.matches(&profile));
        let mut supported = fixture.supported();
        supported.version = "0.165.0".to_owned();
        assert!(!supported.matches(&profile));
        let supported = fixture.supported();
        fs::rename(
            fixture.0.join("codex.exe"),
            fixture.0.join("codex-renamed.exe"),
        )
        .unwrap();
        assert!(!supported.matches(&profile));
    }

    #[test]
    fn file_stamp_rejects_directories_and_empty_files() {
        let fixture = Fixture::new();
        assert!(FileStamp::read(&fixture.0).is_err());
        fs::write(fixture.0.join("other.exe"), []).unwrap();
        assert!(FileStamp::read(&fixture.0.join("other.exe")).is_err());
    }
}
