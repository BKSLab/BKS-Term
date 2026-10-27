//! Opt-in stdio transport. Child I/O and waits never run on the UI thread.
//! All queues, frames, transcript and pending approvals have fixed bounds.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, HANDLE};
use windows::Win32::System::Pipes::PeekNamedPipe;

use crate::agent_protocol::{
    Approval, Decision, Phase, Protocol, Snapshot, MAX_APPROVAL_BYTES, MAX_FRAME_BYTES,
    MAX_PROMPT_BYTES, SUPPORTED_VERSION,
};
use crate::owned_child::OwnedChild;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const COMMAND_CAPACITY: usize = 8;
const WIRE_CAPACITY: usize = 16;
const IO_SLICE_BYTES: usize = 256 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(30);
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(10);

pub enum UserAction {
    Prompt(String),
    Interrupt,
    Decide(Approval, Decision),
}

pub struct Transport {
    commands: SyncSender<UserAction>,
    cancel: Arc<AtomicBool>,
    shared: Arc<Mutex<Snapshot>>,
}

struct ProcessSpec {
    path: PathBuf,
    cwd: PathBuf,
    args: Vec<OsString>,
    check_version: bool,
}

impl Transport {
    /// The caller must bind this to the explicit Start button, never window
    /// creation, restoring settings, polling, or an incoming server request.
    pub fn start(path: PathBuf, cwd: PathBuf) -> Result<Self, String> {
        Self::start_spec(ProcessSpec {
            path,
            cwd,
            args: vec!["app-server".into(), "--listen".into(), "stdio://".into()],
            check_version: true,
        })
    }

    fn start_spec(spec: ProcessSpec) -> Result<Self, String> {
        if !spec.path.is_absolute() || !spec.cwd.is_absolute() {
            return Err("Codex CLI и рабочая папка должны иметь абсолютные пути.".to_owned());
        }
        let protocol = Protocol::new(spec.cwd.to_string_lossy().into_owned());
        let shared = Arc::new(Mutex::new(protocol.snapshot()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (commands, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let worker_shared = Arc::clone(&shared);
        let worker_cancel = Arc::clone(&cancel);
        thread::Builder::new()
            .name("bks-app-server".to_owned())
            .spawn(move || worker(spec, protocol, receiver, worker_shared, worker_cancel))
            .map_err(|error| format!("Не удалось создать транспорт App Server: {error}"))?;
        Ok(Self {
            commands,
            cancel,
            shared,
        })
    }

    pub fn send(&self, action: UserAction) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            return Err("App Server закрывается.".to_owned());
        }
        match &action {
            UserAction::Prompt(text) if text.len() > MAX_PROMPT_BYTES => {
                return Err("Сообщение превышает 16 КиБ. Оно не помещено в очередь.".to_owned());
            }
            UserAction::Decide(approval, _) => {
                let id = &approval.request_id;
                if approval.context.len() > MAX_APPROVAL_BYTES
                    || !(id.is_i64()
                        || id.is_u64()
                        || id.as_str().is_some_and(|id| id.len() <= 1024))
                {
                    return Err(
                        "Неподдерживаемый контекст решения не помещён в очередь.".to_owned()
                    );
                }
            }
            _ => {}
        }
        self.commands.try_send(action).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                "Очередь действий заполнена. Действие не отправлено.".to_owned()
            }
            mpsc::TrySendError::Disconnected(_) => {
                "App Server отключён. Действие не отправлено.".to_owned()
            }
        })
    }

    /// The UI never waits for a mutex held by an I/O operation.
    pub fn snapshot_since(&self, revision: u64) -> Option<Snapshot> {
        let snapshot = self.shared.try_lock().ok()?;
        (snapshot.revision != revision).then(|| snapshot.clone())
    }

    pub fn close(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.close();
    }
}

fn worker(
    spec: ProcessSpec,
    mut protocol: Protocol,
    commands: Receiver<UserAction>,
    shared: Arc<Mutex<Snapshot>>,
    cancel: Arc<AtomicBool>,
) {
    let result = run(&spec, &mut protocol, &commands, &shared, &cancel);
    if cancel.load(Ordering::Acquire) {
        protocol.stop();
    } else if let Err(error) = result {
        protocol.fail(&error.to_string());
    } else {
        protocol.fail("App Server завершил соединение. Автоматический перезапуск отключён.");
    }
    publish(&protocol, &shared);
}

fn run(
    spec: &ProcessSpec,
    protocol: &mut Protocol,
    commands: &Receiver<UserAction>,
    shared: &Arc<Mutex<Snapshot>>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if spec.check_version {
        check_version(&spec.path, cancel)?;
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut owned = OwnedChild::spawn(
        Command::new(&spec.path)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let mut stdout = owned
        .child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout unavailable"))?;
    let mut stderr = owned
        .child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr unavailable"))?;
    let mut stdin = owned
        .child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("stdin unavailable"))?;
    let (wire, outbound) = mpsc::sync_channel::<Vec<u8>>(WIRE_CAPACITY);
    let writer_failed = Arc::new(AtomicBool::new(false));
    let writer_error = Arc::clone(&writer_failed);
    let writer = thread::Builder::new()
        .name("bks-app-server-stdin".to_owned())
        .spawn(move || {
            while let Ok(bytes) = outbound.recv() {
                if stdin
                    .write_all(&bytes)
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    writer_error.store(true, Ordering::Release);
                    break;
                }
            }
        })?;
    let result = (|| {
        queue(&wire, &protocol.initialize())?;
        let mut framer = Framer::default();
        let startup_deadline = Instant::now() + START_TIMEOUT;
        let mut stdout_busy_since = None;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            let quiet = pump_stdout(&mut stdout, &mut framer, protocol, &wire, cancel)?;
            discard_stderr(&mut stderr)?;
            if writer_failed.load(Ordering::Acquire) {
                return Err(io::Error::other("Запись в App Server завершилась ошибкой."));
            }
            if quiet {
                stdout_busy_since = None;
                // Drain notifications before every user action. A resolved,
                // replaced or completed approval cannot use a UI snapshot that
                // was valid before those already-arrived notifications.
                match commands.try_recv() {
                    Ok(action) => {
                        let result = match action {
                            UserAction::Prompt(text) => protocol.send_prompt(&text).map(Some),
                            UserAction::Interrupt => protocol.interrupt(),
                            UserAction::Decide(approval, decision) => {
                                protocol.decide(&approval, decision).map(Some)
                            }
                        };
                        match result {
                            Ok(Some(message)) => queue(&wire, &message)?,
                            Ok(None) => {}
                            Err(error) => protocol.action_error(&error),
                        }
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            } else if stdout_busy_since.get_or_insert_with(Instant::now).elapsed()
                > Duration::from_secs(2)
            {
                return Err(io::Error::other(
                    "Непрерывный поток App Server: соединение закрыто без отправки действий.",
                ));
            }
            publish(protocol, shared);
            if owned.child.try_wait()?.is_some() {
                pump_stdout(&mut stdout, &mut framer, protocol, &wire, cancel)?;
                if !framer.pending.is_empty() {
                    return Err(io::Error::other(
                        "App Server закрыл неполное JSON-сообщение.",
                    ));
                }
                return Ok(());
            }
            if matches!(
                protocol.phase(),
                Phase::Initializing | Phase::StartingThread
            ) && Instant::now() >= startup_deadline
            {
                return Err(io::Error::other(
                    "App Server не завершил подключение за 30 секунд.",
                ));
            }
            thread::sleep(TICK);
        }
    })();
    drop(wire);
    // All waits and pipe cleanup happen on this worker, including blocked
    // writers. Drop on the UI merely sets an atomic cancellation flag.
    drop(owned);
    let _ = writer.join();
    result
}

fn publish(protocol: &Protocol, shared: &Mutex<Snapshot>) {
    if let Ok(mut shared) = shared.lock() {
        if protocol.revision() != shared.revision {
            *shared = protocol.snapshot();
        }
    }
}

fn queue(sender: &SyncSender<Vec<u8>>, value: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::other("Исходящее сообщение превышает 128 КиБ."));
    }
    bytes.push(b'\n');
    sender
        .try_send(bytes)
        .map_err(|_| io::Error::other("Очередь записи App Server заполнена или закрыта."))
}

#[derive(Default)]
struct Framer {
    pending: Vec<u8>,
}

impl Framer {
    fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<Vec<u8>>> {
        let mut complete = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                if !self.pending.is_empty() {
                    complete.push(std::mem::take(&mut self.pending));
                }
            } else if self.pending.len() == MAX_FRAME_BYTES {
                return Err(io::Error::other("Строка App Server превышает 128 КиБ."));
            } else {
                self.pending.push(*byte);
            }
        }
        Ok(complete)
    }
}

fn pump_stdout(
    stdout: &mut ChildStdout,
    framer: &mut Framer,
    protocol: &mut Protocol,
    wire: &SyncSender<Vec<u8>>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let mut read_total = 0;
    let mut buffer = [0u8; 8192];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        let available = available(stdout)?;
        if available == 0 {
            // A partial notification is also backlog: do not send a decision
            // while a possibly invalidating notification is half received.
            return Ok(framer.pending.is_empty());
        }
        if read_total >= IO_SLICE_BYTES {
            return Ok(false);
        }
        let count = stdout.read(&mut buffer[..available.min(8192)])?;
        if count == 0 {
            return Ok(framer.pending.is_empty());
        }
        read_total += count;
        for frame in framer.push(&buffer[..count])? {
            for response in protocol.receive(&frame).map_err(io::Error::other)? {
                queue(wire, &response)?;
            }
        }
    }
}

fn discard_stderr(stderr: &mut impl PipeRead) -> io::Result<()> {
    let mut buffer = [0u8; 4096];
    let mut total = 0;
    while total < IO_SLICE_BYTES {
        let available = available(stderr)?;
        if available == 0 {
            break;
        }
        let read = stderr.read(&mut buffer[..available.min(4096)])?;
        if read == 0 {
            break;
        }
        total += read;
    }
    // Raw diagnostics can contain local credentials. Never append stderr to
    // the transcript or persistent application logs.
    Ok(())
}

trait PipeRead: Read + AsRawHandle {}
impl<T: Read + AsRawHandle> PipeRead for T {}

fn available(pipe: &impl AsRawHandle) -> io::Result<usize> {
    let mut count = 0;
    // SAFETY: the borrowed pipe owns this live handle and count is writable.
    match unsafe {
        PeekNamedPipe(
            HANDLE(pipe.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut count),
            None,
        )
    } {
        Ok(()) => Ok(count as usize),
        Err(error) if error.code() == ERROR_BROKEN_PIPE.to_hresult() => Ok(0),
        Err(error) => Err(io::Error::other(error)),
    }
}

fn check_version(path: &Path, cancel: &AtomicBool) -> io::Result<()> {
    if !path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
    {
        return Err(io::Error::other(
            "Нужен прямой native codex.exe, а не shell-wrapper.",
        ));
    }
    let before = std::fs::metadata(path)?;
    if !before.is_file() {
        return Err(io::Error::other("Codex CLI не является файлом."));
    }
    let mut owned = OwnedChild::spawn(
        Command::new(path)
            .arg("--version")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )?;
    let mut stdout = owned
        .child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("version stdout unavailable"))?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    let mut output = Vec::new();
    loop {
        let count = available(&stdout)?;
        if count != 0 {
            if output.len() + count > 4096 {
                return Err(io::Error::other("Ответ --version превышает 4 КиБ."));
            }
            let mut buffer = vec![0; count];
            stdout.read_exact(&mut buffer)?;
            output.extend_from_slice(&buffer);
        }
        if let Some(status) = owned.child.try_wait()? {
            // The next peek obtains any final bytes not read before exit.
            let count = available(&stdout)?;
            if output.len() + count > 4096 {
                return Err(io::Error::other("Ответ --version слишком длинный."));
            }
            let mut tail = vec![0; count];
            stdout.read_exact(&mut tail)?;
            output.extend_from_slice(&tail);
            let after = std::fs::metadata(path)?;
            if !status.success()
                || !supported_version(&output)
                || before.len() != after.len()
                || before.modified()? != after.modified()?
            {
                return Err(io::Error::other(format!(
                    "Поддерживается только native Codex CLI {SUPPORTED_VERSION}."
                )));
            }
            return Ok(());
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("Проверка --version превысила 2 секунды."));
        }
        thread::sleep(TICK);
    }
}

fn supported_version(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        b"codex-cli 0.164.0" | b"codex-cli 0.164.0\n" | b"codex-cli 0.164.0\r\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_protocol::{MAX_PENDING_APPROVALS, MAX_TRANSCRIPT_BYTES};
    use crate::owned_child::ScopedHandle;
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    fn fixture(mode: &str) -> Transport {
        let path = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Transport::start_spec(ProcessSpec {
            path,
            cwd: cwd.clone(),
            args: [
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]
            .into_iter()
            .map(OsString::from)
            .chain([
                cwd.join("tests/fixtures/codex-app-server.ps1")
                    .into_os_string(),
                "-Mode".into(),
                mode.into(),
            ])
            .collect(),
            check_version: false,
        })
        .unwrap()
    }

    fn until(transport: &Transport, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(snapshot) = transport.snapshot_since(0) {
                if predicate(&snapshot) {
                    return snapshot;
                }
                assert_ne!(snapshot.phase, Phase::Failed, "{}", snapshot.status);
            }
            assert!(Instant::now() < deadline, "fixture timeout");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn line_framing_is_bounded_and_handles_fragmented_unicode_and_crlf() {
        let mut framer = Framer::default();
        assert!(framer.push(b"{\"text\":\"").unwrap().is_empty());
        assert!(framer.push("я".as_bytes()).unwrap().is_empty());
        assert_eq!(
            framer.push(b"\"}\r\n\n{}\n").unwrap(),
            vec!["{\"text\":\"я\"}".as_bytes().to_vec(), b"{}".to_vec()]
        );
        assert!(framer.push(&vec![b'x'; MAX_FRAME_BYTES + 1]).is_err());
        assert!(framer.pending.len() <= MAX_FRAME_BYTES);
    }

    #[test]
    fn version_gate_rejects_future_versions_and_extra_output() {
        assert!(supported_version(b"codex-cli 0.164.0\r\n"));
        for bytes in [
            b"codex-cli 0.164.1".as_slice(),
            b"codex-cli 0.164.0\nextra",
            b"",
        ] {
            assert!(!supported_version(bytes));
        }
    }

    #[test]
    fn fake_process_handshake_prompt_stream_and_explicit_cancellation() {
        let transport = fixture("stream");
        let ready = until(&transport, |s| s.phase == Phase::Ready);
        assert!(!ready.transcript.contains("fixture response"));
        transport
            .send(UserAction::Prompt("fixture request".to_owned()))
            .unwrap();
        let complete = until(&transport, |s| {
            s.phase == Phase::Ready && s.transcript.contains("fixture response")
        });
        assert_eq!(complete.transcript.matches("fixture response").count(), 1);
        transport.close();
        until(&transport, |s| s.phase == Phase::Stopped);
    }

    #[test]
    fn fake_process_receives_interrupt_and_does_not_retry() {
        let transport = fixture("interrupt");
        until(&transport, |s| s.phase == Phase::Ready);
        transport
            .send(UserAction::Prompt("fixture request".to_owned()))
            .unwrap();
        until(&transport, |s| s.phase == Phase::Busy);
        transport.send(UserAction::Interrupt).unwrap();
        let complete = until(&transport, |s| s.phase == Phase::Ready);
        assert!(complete.status.contains("прерван"));
        assert_eq!(complete.transcript.matches("Вы:").count(), 1);
        transport.close();
        until(&transport, |s| s.phase == Phase::Stopped);
    }

    #[test]
    fn fake_process_oversized_line_fails_with_bounded_state() {
        let transport = fixture("oversize");
        let failed = until(&transport, |s| s.phase == Phase::Failed);
        assert!(failed.status.contains("128 КиБ"));
        assert!(failed.transcript.len() <= MAX_TRANSCRIPT_BYTES);
        assert!(failed.pending_count <= MAX_PENDING_APPROVALS);
    }

    #[test]
    fn fake_process_silent_child_can_be_closed_without_ui_wait() {
        let transport = fixture("silent");
        let started = Instant::now();
        transport.close();
        assert!(started.elapsed() < Duration::from_millis(100));
        until(&transport, |s| s.phase == Phase::Stopped);
    }

    #[test]
    fn fake_process_resolved_approval_cannot_be_sent_from_an_old_snapshot() {
        let transport = fixture("stale");
        until(&transport, |s| s.can_send);
        transport
            .send(UserAction::Prompt("fixture request".to_owned()))
            .unwrap();
        let shown = until(&transport, |s| s.approval.is_some())
            .approval
            .unwrap();
        until(&transport, |s| {
            s.phase == Phase::Ready && s.approval.is_none()
        });
        transport
            .send(UserAction::Decide(shown, Decision::AcceptOnce))
            .unwrap();
        let rejected = until(&transport, |s| s.status.contains("устарело"));
        assert_eq!(rejected.phase, Phase::Ready);
        assert!(rejected.approval.is_none());
        transport.close();
        until(&transport, |s| s.phase == Phase::Stopped);
    }

    #[test]
    fn fake_process_that_stops_reading_stdin_can_be_closed() {
        let transport = fixture("blocked");
        until(&transport, |s| s.can_send);
        transport
            .send(UserAction::Prompt(
                "x".repeat(crate::agent_protocol::MAX_PROMPT_BYTES),
            ))
            .unwrap();
        until(&transport, |s| s.phase == Phase::Busy);
        transport.close();
        until(&transport, |s| s.phase == Phase::Stopped);
    }

    #[test]
    fn early_descendant_is_job_owned_and_closes_with_a_blocked_writer() {
        let transport = fixture("earlytree");
        let ready = until(&transport, |s| s.can_send);
        let pid = ready
            .transcript
            .split("fixture-child-")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<u32>()
            .unwrap();
        // The PID came solely from this private fixture's newly spawned child;
        // keep its handle to verify exit without touching any existing process.
        let leaf = ScopedHandle(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.unwrap());
        transport
            .send(UserAction::Prompt("x".repeat(MAX_PROMPT_BYTES)))
            .unwrap();
        until(&transport, |s| s.phase == Phase::Busy);
        let closing = Instant::now();
        transport.close();
        until(&transport, |s| s.phase == Phase::Stopped);
        assert!(closing.elapsed() < Duration::from_secs(3));
        assert_eq!(unsafe { WaitForSingleObject(leaf.0, 1000) }, WAIT_OBJECT_0);
    }

    #[test]
    fn outgoing_queue_is_bounded_and_does_not_block() {
        let (sender, _receiver) = mpsc::sync_channel(WIRE_CAPACITY);
        for _ in 0..WIRE_CAPACITY {
            queue(&sender, &serde_json::json!({"method":"fixture"})).unwrap();
        }
        assert!(queue(&sender, &serde_json::json!({"method":"fixture"})).is_err());
    }

    #[test]
    fn user_command_queue_rejects_oversize_before_enqueuing_and_has_fixed_capacity() {
        let (commands, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let transport = Transport {
            commands,
            cancel: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Mutex::new(
                Protocol::new("C:\\fixture".to_owned()).snapshot(),
            )),
        };
        assert!(transport
            .send(UserAction::Prompt("x".repeat(MAX_PROMPT_BYTES + 1)))
            .is_err());
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        for _ in 0..COMMAND_CAPACITY {
            transport.send(UserAction::Interrupt).unwrap();
        }
        assert!(transport.send(UserAction::Interrupt).is_err());
        assert_eq!(receiver.try_iter().count(), COMMAND_CAPACITY);
    }
}
