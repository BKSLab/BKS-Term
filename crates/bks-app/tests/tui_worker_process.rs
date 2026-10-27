//! Exercise the shipped internal worker mode and an independent real ConPTY.
//! No real AI CLI, credentials, model calls, global input or visible UI.

use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use bks_pty::{PtySession, PtySize};
use bks_terminal::TerminalModel;
use serde_json::{json, Value};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::PeekNamedPipe;

struct Worker {
    child: Child,
    output: ChildStdout,
    pending: Vec<u8>,
}

impl Worker {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bks-app"))
            .arg("--tui-adapter-worker")
            .creation_flags(0x0800_0000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the owned internal worker");
        let output = child.stdout.take().unwrap();
        Self {
            child,
            output,
            pending: Vec::new(),
        }
    }

    fn receive(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = self.pending.drain(..=end).collect();
                return serde_json::from_slice(&line).expect("bounded JSONL reply");
            }
            assert!(Instant::now() < deadline, "worker response timed out");
            let mut available = 0;
            // SAFETY: this test owns the live stdout pipe, and available is writable.
            unsafe {
                PeekNamedPipe(
                    HANDLE(self.output.as_raw_handle()),
                    None,
                    0,
                    None,
                    Some(&mut available),
                    None,
                )
                .expect("worker output pipe remains open");
            }
            if available == 0 {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let mut bytes = [0u8; 4096];
            let read = self
                .output
                .read(&mut bytes[..(available as usize).min(4096)])
                .unwrap();
            self.pending.extend_from_slice(&bytes[..read]);
            assert!(self.pending.len() <= 65_536, "worker output limit");
        }
    }

    fn send(&mut self, request: &Value) {
        let mut bytes = serde_json::to_vec(request).unwrap();
        bytes.push(b'\n');
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }

    fn exited(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "worker did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request() -> Value {
    let mut terminal = TerminalModel::new(70, 14);
    let fixture = include_str!("fixtures/tui/codex-0.164.0/startup-folder-trust-70x14.txt");
    terminal.push_bytes(
        fixture
            .trim_end_matches('\n')
            .replace('\n', "\r\n")
            .as_bytes(),
    );
    let screen = terminal.live_snapshot();
    let lines: Vec<_> = screen
        .cells
        .chunks(screen.cols)
        .map(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .trim_end_matches(' ')
                .to_owned()
        })
        .collect();
    json!({
        "v":1, "id":1, "session":11, "revision":24, "input_revision":0,
        "cli_version":"0.164.0", "direct_codex_profile":true,
        "cols":70, "rows":14, "alternate":false, "lines":lines
    })
}

#[test]
fn actual_worker_recognizes_pinned_shape_and_degrades_after_cli_update() {
    let mut worker = Worker::start();
    assert_eq!(worker.receive(), json!({"v":1,"ready":true}));
    let mut request = request();
    worker.send(&request);
    let response = worker.receive();
    assert_eq!(response["id"], 1);
    assert_eq!(response["matched"]["shape"], "codex-folder-trust-v1");
    assert_eq!(response["matched"]["selected"], 0);
    assert!(response["matched"]["context"]
        .as_str()
        .unwrap()
        .contains("Trust this folder?"));
    request["id"] = json!(2);
    request["cli_version"] = json!("99.0.0");
    worker.send(&request);
    let response = worker.receive();
    assert_eq!(response["id"], 2);
    assert!(response["matched"].is_null());
    assert!(worker.child.try_wait().unwrap().is_none());
}

#[test]
fn malformed_worker_input_exits_without_opening_an_error_window() {
    let mut worker = Worker::start();
    assert_eq!(worker.receive()["ready"], true);
    worker
        .child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"not JSON\n")
        .unwrap();
    assert!(!worker.exited().success());
}

fn roundtrip(session: &PtySession, marker: &str) {
    // CMD removes the caret; an echoed input line cannot satisfy the marker.
    let (left, right) = marker.split_at(marker.len() / 2);
    session
        .write_input(format!("echo {left}^{right}\r\n").as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        output.extend(session.drain_output_limited(4096));
        if String::from_utf8_lossy(&output).contains(marker) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("independent PTY stopped responding");
}

#[test]
fn killed_worker_preserves_two_real_pty_sessions() {
    let first = PtySession::spawn("cmd.exe /d /q", PtySize::default()).unwrap();
    let second = PtySession::spawn("cmd.exe /d /q", PtySize::default()).unwrap();
    roundtrip(&first, "BKS_FIRST_BEFORE");
    roundtrip(&second, "BKS_SECOND_BEFORE");
    let mut worker = Worker::start();
    assert_eq!(worker.receive()["ready"], true);
    worker.child.kill().unwrap();
    assert!(!worker.exited().success());
    first.resize(PtySize::new(100, 32)).unwrap();
    roundtrip(&first, "BKS_FIRST_AFTER");
    roundtrip(&second, "BKS_SECOND_AFTER");
    assert!(first.is_alive() && second.is_alive());
    first.request_terminate().unwrap();
    second.request_terminate().unwrap();
}
