//! Fault containment for the optional local screen adapter.
//! The UI uses bounded nonblocking queues; one supervisor polls both output
//! pipes and owns one child plus one stdin writer. No reader threads are made.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_BROKEN_PIPE, HANDLE};
use windows::Win32::System::Pipes::PeekNamedPipe;

use crate::owned_child::OwnedChild;
use crate::tui_worker::{
    decode_ready, decode_response, encode_request, Request, Response, MAX_RESPONSE_BYTES,
    WORKER_ARGUMENT,
};

const START_TIMEOUT: Duration = Duration::from_secs(2);
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const TICK: Duration = Duration::from_millis(10);
const IO_SLICE_BYTES: usize = 16 * 1024;
const EVENT_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Spawn,
    StartupTimeout,
    ResponseTimeout,
    Exited,
    InvalidResponse,
    Io,
}

impl Failure {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Spawn => "Не удалось запустить адаптер. Чтение и обычный ввод сохранены.",
            Self::StartupTimeout => {
                "Адаптер не подключился за 2 секунды. Чтение и обычный ввод сохранены."
            }
            Self::ResponseTimeout => "Адаптер не ответил вовремя. Чтение и обычный ввод сохранены.",
            Self::Exited => "Адаптер завершился. Чтение и обычный ввод сохранены.",
            Self::InvalidResponse => {
                "Несовместимый ответ адаптера. Чтение и обычный ввод сохранены."
            }
            Self::Io => "Ошибка связи с адаптером. Чтение и обычный ввод сохранены.",
        }
    }
}

#[derive(Debug)]
pub(crate) enum Event {
    Ready,
    Response(Response),
    Disabled(Failure),
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubmitError {
    Starting,
    Busy,
    Disabled,
    Stopped,
    InvalidRequest,
}

#[derive(Default)]
struct Flags {
    cancel: AtomicBool,
    ready: AtomicBool,
    busy: AtomicBool,
    disabled: AtomicBool,
    finished: AtomicBool,
    #[cfg(test)]
    cleanup_complete: AtomicBool,
}

pub(crate) struct Host {
    requests: SyncSender<Request>,
    events: Receiver<Event>,
    flags: Arc<Flags>,
}

struct ProcessSpec {
    path: PathBuf,
    args: Vec<OsString>,
    start_timeout: Duration,
    response_timeout: Duration,
}

impl Host {
    /// Start only the current application in its internal worker mode. A failed
    /// host remains disabled: retry requires explicitly creating a new Host.
    pub(crate) fn start() -> Result<Self, String> {
        let path = std::env::current_exe()
            .map_err(|_| "Не удалось определить путь к worker адаптера.".to_owned())?;
        Self::start_spec(ProcessSpec {
            path,
            args: vec![WORKER_ARGUMENT.into()],
            start_timeout: START_TIMEOUT,
            response_timeout: RESPONSE_TIMEOUT,
        })
    }

    fn start_spec(spec: ProcessSpec) -> Result<Self, String> {
        if !spec.path.is_absolute() {
            return Err("Worker адаптера должен иметь абсолютный путь.".to_owned());
        }
        let (requests, receiver) = mpsc::sync_channel(1);
        let (sender, events) = mpsc::sync_channel(EVENT_CAPACITY);
        let flags = Arc::new(Flags::default());
        let worker_flags = Arc::clone(&flags);
        thread::Builder::new()
            .name("bks-tui-adapter".to_owned())
            .spawn(move || supervise(spec, receiver, sender, worker_flags))
            .map_err(|_| "Не удалось создать supervisor адаптера.".to_owned())?;
        Ok(Self {
            requests,
            events,
            flags,
        })
    }

    /// Exactly one request may be queued, in flight, or awaiting UI consumption.
    /// Serialization and all pipe operations happen on the supervisor/writer.
    pub(crate) fn try_submit(&self, request: Request) -> Result<(), SubmitError> {
        if self.flags.cancel.load(Ordering::Acquire) {
            return Err(SubmitError::Stopped);
        }
        if self.flags.disabled.load(Ordering::Acquire) {
            return Err(SubmitError::Disabled);
        }
        if !self.flags.ready.load(Ordering::Acquire) {
            return Err(SubmitError::Starting);
        }
        request
            .validate()
            .map_err(|_| SubmitError::InvalidRequest)?;
        self.flags
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SubmitError::Busy)?;
        if self.requests.try_send(request).is_err() {
            self.flags.busy.store(false, Ordering::Release);
            return Err(SubmitError::Disabled);
        }
        Ok(())
    }

    pub(crate) fn try_poll(&self) -> Option<Event> {
        let event = self.events.try_recv().ok()?;
        if matches!(event, Event::Response(_)) {
            self.flags.busy.store(false, Ordering::Release);
        }
        Some(event)
    }

    pub(crate) fn stop(&self) {
        self.flags.cancel.store(true, Ordering::Release);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // The supervisor closes its private job, releases blocked pipe writes,
        // and joins its only writer. Neither process waits nor joins run here.
        self.stop();
    }
}

fn supervise(
    spec: ProcessSpec,
    requests: Receiver<Request>,
    events: SyncSender<Event>,
    flags: Arc<Flags>,
) {
    let result = run(&spec, &requests, &events, &flags);
    #[cfg(test)]
    flags.cleanup_complete.store(true, Ordering::Release);
    finish(result, &events, &flags);
}

fn finish(result: Result<(), Failure>, events: &SyncSender<Event>, flags: &Flags) {
    if flags.finished.swap(true, Ordering::AcqRel) {
        return;
    }
    flags.ready.store(false, Ordering::Release);
    if flags.cancel.load(Ordering::Acquire) {
        let _ = events.try_send(Event::Stopped);
    } else {
        flags.disabled.store(true, Ordering::Release);
        // There can be at most Ready + one unconsumed Response before this
        // terminal event, so the fixed four-slot event queue cannot overflow.
        let _ = events.try_send(Event::Disabled(result.err().unwrap_or(Failure::Exited)));
    }
}

fn run(
    spec: &ProcessSpec,
    requests: &Receiver<Request>,
    events: &SyncSender<Event>,
    flags: &Flags,
) -> Result<(), Failure> {
    if flags.cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let startup_deadline = Instant::now() + spec.start_timeout;
    let mut owned = OwnedChild::spawn(
        Command::new(&spec.path)
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .map_err(|_| Failure::Spawn)?;
    let mut stdout = owned.child.stdout.take().ok_or(Failure::Io)?;
    let mut stderr = owned.child.stderr.take().ok_or(Failure::Io)?;
    let mut stdin = owned.child.stdin.take().ok_or(Failure::Io)?;
    let (wire, outgoing) = mpsc::sync_channel::<Vec<u8>>(1);
    let writer_failed = Arc::new(AtomicBool::new(false));
    let writer_flag = Arc::clone(&writer_failed);
    let writer = thread::Builder::new()
        .name("bks-tui-adapter-stdin".to_owned())
        .spawn(move || {
            while let Ok(bytes) = outgoing.recv() {
                if stdin
                    .write_all(&bytes)
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    writer_flag.store(true, Ordering::Release);
                    return;
                }
            }
        })
        .map_err(|_| Failure::Io)?;
    let result = (|| {
        let mut ready = false;
        let mut pending: Option<(Request, Instant)> = None;
        let mut framer = Framer::default();
        loop {
            if flags.cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            if !ready && Instant::now() >= startup_deadline {
                return Err(Failure::StartupTimeout);
            }
            if pending
                .as_ref()
                .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
            {
                return Err(Failure::ResponseTimeout);
            }
            for frame in read_stdout(&mut stdout, &mut framer)? {
                if !ready {
                    decode_ready(&frame).map_err(|_| Failure::InvalidResponse)?;
                    ready = true;
                    flags.ready.store(true, Ordering::Release);
                    events.try_send(Event::Ready).map_err(|_| Failure::Io)?;
                } else if let Some((request, _)) = pending.take() {
                    let response =
                        decode_response(&frame, &request).map_err(|_| Failure::InvalidResponse)?;
                    events
                        .try_send(Event::Response(response))
                        .map_err(|_| Failure::Io)?;
                } else {
                    return Err(Failure::InvalidResponse);
                }
            }
            // An idle worker has no reason to write even a partial frame.
            // Do not allow such bytes to be attributed to the next request.
            if ready && pending.is_none() && !framer.pending.is_empty() {
                return Err(Failure::InvalidResponse);
            }
            discard_stderr(&mut stderr).map_err(|_| Failure::Io)?;
            if writer_failed.load(Ordering::Acquire) {
                return Err(Failure::Io);
            }
            if owned.child.try_wait().map_err(|_| Failure::Io)?.is_some() {
                return Err(Failure::Exited);
            }
            if ready && pending.is_none() {
                match requests.try_recv() {
                    Ok(request) => {
                        let bytes = encode_request(&request).map_err(|_| Failure::Io)?;
                        // The deadline covers a child that stops reading stdin;
                        // it does not depend on the blocking writer completing.
                        pending = Some((request, Instant::now() + spec.response_timeout));
                        wire.try_send(bytes).map_err(|_| Failure::Io)?;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            thread::sleep(TICK);
        }
    })();
    if result.is_err() && !flags.cancel.load(Ordering::Acquire) {
        // Disable promptly even if OS process cleanup takes longer. This is the
        // sole terminal event; the outer supervisor cannot publish it twice.
        finish(result, events, flags);
    }
    drop(wire);
    // The job owns only this subprocess and descendants. Closing it first
    // releases all peer pipe handles before joining a potentially blocked write.
    drop(owned);
    let _ = writer.join();
    result
}

#[derive(Default)]
struct Framer {
    pending: Vec<u8>,
}

impl Framer {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, Failure> {
        let mut frames = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                if self.pending.is_empty() {
                    return Err(Failure::InvalidResponse);
                }
                frames.push(std::mem::take(&mut self.pending));
            } else if self.pending.len() == MAX_RESPONSE_BYTES {
                return Err(Failure::InvalidResponse);
            } else {
                self.pending.push(*byte);
            }
        }
        Ok(frames)
    }
}

fn read_stdout(
    stdout: &mut (impl Read + AsRawHandle),
    framer: &mut Framer,
) -> Result<Vec<Vec<u8>>, Failure> {
    let mut total = 0;
    let mut frames = Vec::new();
    let mut buffer = [0u8; 4096];
    while total < IO_SLICE_BYTES {
        let available = available(stdout).map_err(|_| Failure::Io)?;
        if available == 0 {
            break;
        }
        let count = stdout
            .read(&mut buffer[..available.min(4096)])
            .map_err(|_| Failure::Io)?;
        if count == 0 {
            break;
        }
        total += count;
        frames.extend(framer.push(&buffer[..count])?);
    }
    Ok(frames)
}

fn discard_stderr(stderr: &mut (impl Read + AsRawHandle)) -> io::Result<()> {
    let mut total = 0;
    let mut buffer = [0u8; 4096];
    while total < IO_SLICE_BYTES {
        let available = available(stderr)?;
        if available == 0 {
            break;
        }
        let count = stderr.read(&mut buffer[..available.min(4096)])?;
        if count == 0 {
            break;
        }
        total += count;
    }
    // Child diagnostics can contain screen contents. Do not retain or log them.
    Ok(())
}

fn available(pipe: &impl AsRawHandle) -> io::Result<usize> {
    let mut count = 0;
    // SAFETY: the borrowed object owns this live pipe handle; count is writable.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_worker::PROTOCOL_VERSION;

    fn fixture(script: &str, response_timeout: Duration) -> Host {
        let path = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        Host::start_spec(ProcessSpec {
            path,
            args: ["-NoProfile", "-NonInteractive", "-Command", script]
                .into_iter()
                .map(OsString::from)
                .collect(),
            start_timeout: Duration::from_secs(10),
            response_timeout,
        })
        .unwrap()
    }

    fn request(id: u64) -> Request {
        Request {
            v: PROTOCOL_VERSION,
            id,
            session: 11,
            revision: 24,
            input_revision: 0,
            cli_version: None,
            direct_codex_profile: false,
            cols: 70,
            rows: 14,
            alternate: false,
            lines: vec![String::new(); 14],
        }
    }

    fn until(host: &Host, predicate: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(event) = host.try_poll() {
                if predicate(&event) {
                    return event;
                }
                assert!(!matches!(event, Event::Disabled(_)), "{event:?}");
            }
            assert!(Instant::now() < deadline, "adapter fixture timeout");
            thread::sleep(TICK);
        }
    }

    const HELLO: &str = r#"[Console]::Out.WriteLine('{"v":1,"ready":true}');"#;

    #[test]
    fn fragmented_response_framing_has_a_fixed_bound() {
        let mut framer = Framer::default();
        assert!(framer.push(b"{\"text\":\"").unwrap().is_empty());
        assert!(framer.push("я".as_bytes()).unwrap().is_empty());
        assert_eq!(
            framer.push(b"\"}\r\n").unwrap(),
            vec!["{\"text\":\"я\"}".as_bytes().to_vec()]
        );
        assert!(framer.push(b"\n").is_err());
        assert_eq!(
            framer.push(&vec![b'x'; MAX_RESPONSE_BYTES + 1]),
            Err(Failure::InvalidResponse)
        );
        assert!(framer.pending.len() <= MAX_RESPONSE_BYTES);
    }

    #[test]
    fn fake_worker_pipeline_is_bounded_and_preserves_bindings() {
        let script = format!(
            "{HELLO} while ($null -ne ($line = [Console]::In.ReadLine())) {{ \
             $r = $line | ConvertFrom-Json; \
             @{{v=1;id=$r.id;session=$r.session;revision=$r.revision;input_revision=$r.input_revision;matched=$null}} \
             | ConvertTo-Json -Compress | ForEach-Object {{ [Console]::Out.WriteLine($_) }} }}"
        );
        let host = fixture(&script, Duration::from_secs(3));
        until(&host, |event| matches!(event, Event::Ready));
        for id in 0..2 {
            host.try_submit(request(id)).unwrap();
            assert_eq!(host.try_submit(request(id + 10)), Err(SubmitError::Busy));
            let response = until(&host, |event| matches!(event, Event::Response(_)));
            let Event::Response(response) = response else {
                unreachable!()
            };
            assert_eq!(response.id, id);
            assert_eq!(response.session, 11);
            assert_eq!(response.matched, None);
        }
        host.stop();
        until(&host, |event| matches!(event, Event::Stopped));
    }

    #[test]
    fn fake_worker_bad_json_disables_once_without_restart() {
        let host = fixture(
            &format!("{HELLO} $null = [Console]::In.ReadLine(); [Console]::Out.WriteLine('bad json'); Start-Sleep -Seconds 30"),
            Duration::from_secs(3),
        );
        until(&host, |event| matches!(event, Event::Ready));
        host.try_submit(request(0)).unwrap();
        let event = until(&host, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::InvalidResponse)));
        assert_eq!(host.try_submit(request(1)), Err(SubmitError::Disabled));
        assert!(host.try_poll().is_none());
    }

    #[test]
    fn fake_worker_startup_timeout_and_oversized_stdout_are_terminal_failures() {
        let path = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let silent = Host::start_spec(ProcessSpec {
            path,
            args: [
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
            start_timeout: Duration::from_millis(100),
            response_timeout: Duration::from_millis(300),
        })
        .unwrap();
        let event = until(&silent, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::StartupTimeout)));

        let oversized = fixture(
            &format!(
                "{HELLO} $null = [Console]::In.ReadLine(); [Console]::Out.Write(('x' * {})); [Console]::Out.Flush(); Start-Sleep -Seconds 30",
                MAX_RESPONSE_BYTES + 1
            ),
            Duration::from_secs(3),
        );
        until(&oversized, |event| matches!(event, Event::Ready));
        oversized.try_submit(request(0)).unwrap();
        let event = until(&oversized, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::InvalidResponse)));
    }

    #[test]
    fn fake_worker_partial_response_expires_without_waiting_for_newline() {
        let host = fixture(
            &format!(
                "{HELLO} $null = [Console]::In.ReadLine(); [Console]::Out.Write('{{'); [Console]::Out.Flush(); Start-Sleep -Seconds 30"
            ),
            Duration::from_millis(300),
        );
        until(&host, |event| matches!(event, Event::Ready));
        host.try_submit(request(0)).unwrap();
        let event = until(&host, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::ResponseTimeout)));
    }

    #[test]
    fn fake_worker_crash_and_stderr_flood_do_not_defeat_deadlines() {
        let crashed = fixture("exit 7", Duration::from_millis(200));
        let event = until(&crashed, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::Exited)));

        let flooded = fixture(
            &format!("{HELLO} $null = [Console]::In.ReadLine(); $chunk = 'x' * 4096; while ($true) {{ [Console]::Error.Write($chunk) }}"),
            Duration::from_millis(300),
        );
        until(&flooded, |event| matches!(event, Event::Ready));
        flooded.try_submit(request(0)).unwrap();
        let event = until(&flooded, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::ResponseTimeout)));
    }

    #[test]
    fn fake_worker_that_stops_reading_stdin_is_killed_off_ui() {
        let host = fixture(
            &format!("{HELLO} Start-Sleep -Seconds 30"),
            Duration::from_millis(300),
        );
        until(&host, |event| matches!(event, Event::Ready));
        let mut large = request(0);
        large.lines = vec!["x".repeat(8192); 14];
        host.try_submit(large).unwrap();
        let event = until(&host, |event| matches!(event, Event::Disabled(_)));
        assert!(matches!(event, Event::Disabled(Failure::ResponseTimeout)));
        let cleanup = Arc::clone(&host.flags);
        let started = Instant::now();
        drop(host);
        assert!(started.elapsed() < Duration::from_millis(100));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cleanup.cleanup_complete.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "blocked stdin writer did not finish"
            );
            thread::sleep(TICK);
        }
    }

    #[test]
    fn stop_during_startup_never_waits_on_the_ui() {
        let host = fixture("Start-Sleep -Seconds 30", Duration::from_millis(300));
        assert_eq!(host.try_submit(request(0)), Err(SubmitError::Starting));
        let started = Instant::now();
        host.stop();
        assert!(started.elapsed() < Duration::from_millis(100));
        until(&host, |event| matches!(event, Event::Stopped));
    }
}
