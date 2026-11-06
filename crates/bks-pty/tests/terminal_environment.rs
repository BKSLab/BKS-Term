//! An isolated parent verifies the actual CreateProcessW environment without
//! changing globals shared by this test process or launching any installed CLI.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bks_pty::{PtySession, PtySize};

const RELAY: &str = "BKS_TEST_CONPTY_ENV_RELAY";
const TEST: &str = "terminal_capabilities_preserve_user_environment_and_parent_state";
const MARKER: &str = "BKS_TERMINAL_ENV_OK";

#[test]
fn terminal_capabilities_preserve_user_environment_and_parent_state() {
    if std::env::var_os(RELAY).as_deref() != Some(OsStr::new("relay")) {
        isolated_parent();
        return;
    }

    let names = [
        "TERM",
        "COLORTERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "NO_COLOR",
        "FORCE_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "PATH",
        "SSH_AUTH_SOCK",
        "BKS_TEST_ENV_UNICODE",
    ];
    let before = names.map(std::env::var_os);
    let parent_directory = std::env::current_dir().unwrap();
    let child_directory = PathBuf::from(std::env::var_os("BKS_TEST_ENV_CWD").unwrap());
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = concat!(
        "$ErrorActionPreference='Stop'; ",
        "if($env:TERM -cne 'xterm-256color'){throw 'TERM'}; ",
        "if($env:COLORTERM -cne 'truecolor'){throw 'COLORTERM'}; ",
        "if($env:TERM_PROGRAM -cne 'bks-term'){throw 'TERM_PROGRAM'}; ",
        "if($null -ne [Environment]::GetEnvironmentVariable('TERM_PROGRAM_VERSION')){throw 'host version'}; ",
        "if($env:NO_COLOR -cne '1'){throw 'NO_COLOR'}; ",
        "if($env:FORCE_COLOR -cne '0'){throw 'FORCE_COLOR'}; ",
        "if($env:CLICOLOR -cne '0'){throw 'CLICOLOR'}; ",
        "if($env:CLICOLOR_FORCE -cne '1'){throw 'CLICOLOR_FORCE'}; ",
        "if($env:PATH -cne $env:BKS_TEST_ENV_PATH){throw 'PATH'}; ",
        "if($env:SSH_AUTH_SOCK -cne 'pipe=fixture'){throw 'SSH_AUTH_SOCK'}; ",
        "if($env:BKS_TEST_ENV_UNICODE -cne 'Ж🙂=keep'){throw 'Unicode value'}; ",
        "if((Get-Location).Path -cne $env:BKS_TEST_ENV_CWD){throw 'working directory'}; ",
        "[Console]::WriteLine('BKS_TERMINAL_ENV_OK')"
    );
    let command = format!(
        "\"{}\" -NoLogo -NoProfile -NonInteractive -Command \"{script}\"",
        powershell.display()
    );
    let session = PtySession::spawn_in(&command, PtySize::new(512, 24), Some(&child_directory))
        .expect("spawn the environment fixture through ConPTY");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        output.extend(session.drain_output());
        if !session.is_alive() && String::from_utf8_lossy(&output).contains(MARKER) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        String::from_utf8_lossy(&output).contains(MARKER),
        "the child environment assertions did not complete"
    );
    assert_eq!(session.exit_code().unwrap(), Some(0));
    assert_eq!(std::env::current_dir().unwrap(), parent_directory);
    for (name, value) in names.into_iter().zip(before) {
        assert!(
            std::env::var_os(name) == value,
            "parent environment changed: {name}"
        );
    }
}

fn isolated_parent() {
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir(self.0.join("child Ж🙂"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = Directory(
        std::env::temp_dir().join(format!("bks env проверка {} {nonce}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let child_directory = directory.0.join("child Ж🙂");
    std::fs::create_dir(&child_directory).unwrap();
    let path = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .current_dir(&directory.0)
        .env(RELAY, "relay")
        .env("tErM", "dumb")
        .env("CoLoRtErM", "host-color")
        .env("TeRm_PrOgRaM", "vscode")
        .env("term_program_version", "host-version")
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env("CLICOLOR", "0")
        .env("CLICOLOR_FORCE", "1")
        .env("Path", &path)
        .env("SSH_AUTH_SOCK", "pipe=fixture")
        .env("BKS_TEST_ENV_UNICODE", "Ж🙂=keep")
        .env("BKS_TEST_ENV_PATH", &path)
        .env("BKS_TEST_ENV_CWD", &child_directory)
        .output()
        .expect("start the isolated environment parent");
    assert!(
        result.status.success(),
        "isolated environment regression failed: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn profile_environment_is_private_to_each_simultaneous_conpty() {
    let key = "BKS_PROFILE_ENVIRONMENT_FIXTURE";
    let before = std::env::var_os(key);
    let command = format!(
        "\"{}\" -NoLogo -NoProfile -NonInteractive -Command \"[Console]::WriteLine('BKS_PRIVATE_' + $env:{key}); exit 0\"",
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe").display()
    );
    let first = PtySession::spawn_in_with_env(
        &command,
        PtySize::new(80, 12),
        None,
        &std::collections::BTreeMap::from([(key.to_owned(), "FIRST".to_owned())]),
    )
    .unwrap();
    let second = PtySession::spawn_in_with_env(
        &command,
        PtySize::new(80, 12),
        None,
        &std::collections::BTreeMap::from([(key.to_owned(), "SECOND".to_owned())]),
    )
    .unwrap();
    for (session, own, other) in [
        (&first, "BKS_PRIVATE_FIRST", "BKS_PRIVATE_SECOND"),
        (&second, "BKS_PRIVATE_SECOND", "BKS_PRIVATE_FIRST"),
    ] {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut bytes = Vec::new();
        while Instant::now() < deadline {
            bytes.extend(session.drain_output());
            if session.exit_code().unwrap().is_some() {
                session.request_output_finish().unwrap();
                if session.output_drained() {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains(own),
            "the configured child marker was not received: {text:?}"
        );
        assert!(
            !text.contains(other),
            "another session's value leaked into this child"
        );
        assert_eq!(session.exit_code().unwrap(), Some(0));
    }
    assert_eq!(std::env::var_os(key), before);
}
