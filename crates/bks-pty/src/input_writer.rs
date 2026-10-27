//! Ограниченная очередь ввода: UI не ожидает записи в pipe.

use std::collections::VecDeque;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::WriteFile;
use windows::Win32::System::Threading::{GetCurrentProcess, WaitForSingleObject};
use windows::Win32::System::IO::CancelSynchronousIo;

use crate::{handle, win_err};

/// Предел незавершённого ввода, включая сообщение, которое пишет worker.
pub const INPUT_QUEUE_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Ограничивает число мелких сообщений независимо от их суммарного размера.
pub const INPUT_QUEUE_MAX_MESSAGES: usize = 256;
const WRITE_CHUNK_BYTES: usize = 8192;

struct OwnedHandle(isize);

impl OwnedHandle {
    fn duplicate(input: HANDLE) -> io::Result<Self> {
        let mut duplicate = HANDLE::default();
        // SAFETY: вызывающий сохраняет input живым; DuplicateHandle передаёт
        // этому guard независимое владение новым handle без наследования.
        unsafe {
            let process = GetCurrentProcess();
            DuplicateHandle(
                process,
                input,
                process,
                &mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        }
        .map_err(win_err)?;
        Ok(Self(duplicate.0 as isize))
    }

    fn handle(&self) -> HANDLE {
        handle(self.0)
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: guard владеет handle; WriteFile уже завершился до Drop.
        unsafe {
            let _ = CloseHandle(self.handle());
        }
    }
}

#[derive(Default)]
struct InputState {
    messages: VecDeque<Vec<u8>>,
    pending_bytes: usize,
    pending_messages: usize,
    error: Option<io::Error>,
}

#[derive(Default)]
struct InputQueue {
    state: Mutex<InputState>,
    ready: Condvar,
    stopped: AtomicBool,
}

impl InputQueue {
    fn enqueue(&self, data: &[u8]) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        self.enqueue_locked(&mut state, data)
    }

    fn enqueue_locked(&self, state: &mut InputState, data: &[u8]) -> io::Result<()> {
        self.check_state(state)?;
        if data.is_empty() {
            return Ok(());
        }
        if data.len() > INPUT_QUEUE_MAX_BYTES.saturating_sub(state.pending_bytes)
            || state.pending_messages >= INPUT_QUEUE_MAX_MESSAGES
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "PTY input queue is full (2 MiB / 256 messages)",
            ));
        }
        // Копируем только после проверки лимитов; при переполнении ни один
        // байт сообщения не принимается. Мьютекс не удерживается при WriteFile.
        state.messages.push_back(data.to_vec());
        state.pending_bytes += data.len();
        state.pending_messages += 1;
        self.ready.notify_one();
        Ok(())
    }

    fn check_state(&self, state: &InputState) -> io::Result<()> {
        if let Some(error) = &state.error {
            return Err(match error.raw_os_error() {
                Some(code) => io::Error::from_raw_os_error(code),
                None => io::Error::new(error.kind(), error.to_string()),
            });
        }
        if self.stopped.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "PTY input writer stopped",
            ));
        }
        Ok(())
    }

    fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        // Serialize stopping with the state check and a guarded enqueue.
        self.stopped.store(true, Ordering::Release);
        state.messages.clear();
        state.pending_bytes = 0;
        state.pending_messages = 0;
        self.ready.notify_all();
    }

    fn fail(&self, error: io::Error) {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        if !self.stopped.load(Ordering::Acquire) {
            state.error = Some(error);
        }
        self.stopped.store(true, Ordering::Release);
        state.messages.clear();
        state.pending_bytes = 0;
        state.pending_messages = 0;
        self.ready.notify_all();
    }
}

/// Один FIFO writer для ConPTY-сеанса с ограниченной очередью.
///
/// `write` подтверждает приём всего сообщения в очередь, а не доставку дочернему
/// процессу. Ошибки worker доступны через `check_error` и следующие `write`.
/// Drop отменяет недоставленный ввод и ждёт завершения записи, прежде чем
/// закрыть дубликат входного handle. Синхронный `PtySession::write_input` сохранён;
/// для порядка сообщений используйте один writer без параллельной sync-записи.
pub struct PtyInputWriter {
    queue: Arc<InputQueue>,
    worker: Option<JoinHandle<()>>,
}

impl PtyInputWriter {
    pub(crate) fn new(input: HANDLE) -> io::Result<Self> {
        let input = OwnedHandle::duplicate(input)?;
        let queue = Arc::new(InputQueue::default());
        let worker_queue = Arc::clone(&queue);
        // При ошибке spawn замыкание освобождает guard, не оставляя duplicate.
        let worker = std::thread::Builder::new()
            .name("bks-pty-writer".into())
            .spawn(move || run_worker(input, worker_queue))?;
        Ok(Self {
            queue,
            worker: Some(worker),
        })
    }

    /// Ставит всё сообщение в очередь без ожидания pipe или свободного места.
    /// `WouldBlock` означает, что сообщение целиком отклонено из-за лимита.
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        self.check_error()?;
        self.queue.enqueue(data)
    }

    /// Atomically accepts input only when no message is queued or in flight.
    /// This holds only the short queue lock; the worker performs WriteFile
    /// after releasing it. May be called while holding the output-buffer lock.
    pub fn write_if_idle(&self, data: &[u8]) -> io::Result<()> {
        let mut state = self
            .queue
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        self.check_locked(&state)?;
        if state.pending_messages != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "PTY input is queued or in flight",
            ));
        }
        self.queue.enqueue_locked(&mut state, data)
    }

    /// Includes the message currently being written, not just queued messages.
    pub fn is_idle(&self) -> bool {
        let state = self
            .queue
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        self.check_locked(&state).is_ok() && state.pending_messages == 0
    }

    /// Проверяет ошибку фоновой записи, в том числе без нового ввода.
    pub fn check_error(&self) -> io::Result<()> {
        let state = self
            .queue
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        self.check_locked(&state)
    }

    fn check_locked(&self, state: &InputState) -> io::Result<()> {
        self.queue.check_state(state)?;
        if self.worker.as_ref().is_none_or(JoinHandle::is_finished) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "PTY input worker exited",
            ));
        }
        Ok(())
    }

    /// Запрещает новый ввод, очищает очередь и запрашивает отмену текущей записи.
    /// Drop повторяет отмену до выхода worker, закрывая race перед WriteFile.
    pub fn cancel(&self) {
        self.queue.stop();
        if let Some(worker) = &self.worker {
            // SAFETY: JoinHandle удерживает валидный handle именно этого worker.
            // ERROR_NOT_FOUND допустим: поток может ещё не войти в WriteFile.
            unsafe {
                let _ = CancelSynchronousIo(HANDLE(worker.as_raw_handle()));
            }
        }
    }
}

impl Drop for PtyInputWriter {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let thread = HANDLE(worker.as_raw_handle());
            // Отмена однократным CancelSynchronousIo недостаточна: stop может
            // прийти между проверкой флага и входом в WriteFile. Повторяем её,
            // пока поток не завершится, и не закрываем handle во время I/O.
            while unsafe { WaitForSingleObject(thread, 5) } == WAIT_TIMEOUT {
                unsafe {
                    let _ = CancelSynchronousIo(thread);
                }
            }
            let _ = worker.join();
        }
    }
}

fn run_worker(input: OwnedHandle, queue: Arc<InputQueue>) {
    loop {
        let data = {
            let mut state = queue.state.lock().unwrap_or_else(|err| err.into_inner());
            while state.messages.is_empty() && !queue.stopped.load(Ordering::Acquire) {
                state = queue
                    .ready
                    .wait(state)
                    .unwrap_or_else(|err| err.into_inner());
            }
            if queue.stopped.load(Ordering::Acquire) {
                return;
            }
            state.messages.pop_front().expect("queue is not empty")
        };
        let mut offset = 0;
        while offset < data.len() {
            if queue.stopped.load(Ordering::Acquire) {
                return;
            }
            let end = (offset + WRITE_CHUNK_BYTES).min(data.len());
            let mut written = 0u32;
            // SAFETY: worker единолично владеет input, буфер data жив до
            // завершения синхронного вызова, Drop сначала отменяет и join-ит.
            let result = unsafe {
                WriteFile(
                    input.handle(),
                    Some(&data[offset..end]),
                    Some(&mut written),
                    None,
                )
            };
            if let Err(error) = result {
                queue.fail(win_err(error));
                return;
            }
            if written == 0 {
                queue.fail(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "PTY input closed",
                ));
                return;
            }
            offset += written as usize;
        }
        let mut state = queue.state.lock().unwrap_or_else(|err| err.into_inner());
        if queue.stopped.load(Ordering::Acquire) {
            return;
        }
        state.pending_bytes -= data.len();
        state.pending_messages -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use windows::Win32::System::Pipes::CreatePipe;

    /// A live worker that leaves queued messages inspectable. It performs no
    /// native I/O and exits on the same stop/Condvar protocol as a real worker.
    fn inspectable_writer() -> PtyInputWriter {
        let queue = Arc::new(InputQueue::default());
        let worker_queue = Arc::clone(&queue);
        let worker = std::thread::spawn(move || {
            let mut state = worker_queue.state.lock().unwrap();
            while !worker_queue.stopped.load(Ordering::Acquire) {
                state = worker_queue.ready.wait(state).unwrap();
            }
        });
        PtyInputWriter {
            queue,
            worker: Some(worker),
        }
    }

    fn pipe() -> (OwnedHandle, OwnedHandle) {
        let mut reader = HANDLE::default();
        let mut writer = HANDLE::default();
        unsafe { CreatePipe(&mut reader, &mut writer, None, 0) }.expect("create test pipe");
        (
            OwnedHandle(reader.0 as isize),
            OwnedHandle(writer.0 as isize),
        )
    }

    #[test]
    fn queue_limits_bytes_and_messages_without_partial_enqueue() {
        let queue = InputQueue::default();
        queue.enqueue(&vec![b'x'; INPUT_QUEUE_MAX_BYTES]).unwrap();
        assert_eq!(
            queue.enqueue(b"y").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let state = queue.state.lock().unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.pending_bytes, INPUT_QUEUE_MAX_BYTES);
        drop(state);

        let queue = InputQueue::default();
        for _ in 0..INPUT_QUEUE_MAX_MESSAGES {
            queue.enqueue(b"x").unwrap();
        }
        assert_eq!(
            queue.enqueue(b"y").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            queue.state.lock().unwrap().messages.len(),
            INPUT_QUEUE_MAX_MESSAGES
        );
    }

    #[test]
    fn worker_error_is_reported_on_later_writes() {
        let (reader, input) = pipe();
        let writer = PtyInputWriter::new(input.handle()).unwrap();
        drop(reader);
        writer.write(b"closed").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while writer.check_error().is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            writer.check_error().is_err(),
            "worker error was not reported"
        );
        assert!(writer.write(b"later").is_err());
    }

    #[test]
    fn drop_cancels_blocked_pipe_and_closes_duplicate() {
        let (reader, input) = pipe();
        let writer = PtyInputWriter::new(input.handle()).unwrap();
        drop(input);
        writer.write(&vec![b'x'; INPUT_QUEUE_MAX_BYTES]).unwrap();
        // Reader остаётся открытым, но не читает: 8 KiB WriteFile не помещается
        // в pipe и зависает без CancelSynchronousIo.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !writer.queue.state.lock().unwrap().messages.is_empty() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(writer.queue.state.lock().unwrap().messages.is_empty());
        assert_eq!(writer.queue.state.lock().unwrap().pending_messages, 1);
        assert_eq!(
            writer
                .write_if_idle(b"must not follow the blocked write")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        let (done, completed) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(writer);
            // После возврата Drop живых дубликатов write handle больше нет:
            // дочитываем принятые до отмены байты, затем получаем EOF/broken pipe.
            // Включаем это чтение в timeout, чтобы регрессия утечки не повесила тест.
            let mut data = [0u8; WRITE_CHUNK_BYTES];
            loop {
                let mut read = 0;
                let result = unsafe {
                    windows::Win32::Storage::FileSystem::ReadFile(
                        reader.handle(),
                        Some(&mut data),
                        Some(&mut read),
                        None,
                    )
                };
                if result.is_err() || read == 0 {
                    break;
                }
            }
            done.send(()).unwrap();
        });
        completed
            .recv_timeout(Duration::from_secs(3))
            .expect("writer Drop blocked");
        dropper.join().unwrap();
    }

    #[test]
    fn newly_published_output_rejects_guarded_input_without_queueing_it() {
        let output = crate::output_buffer::OutputBuffer::new(32);
        let writer = inspectable_writer();
        assert_eq!(output.pending_bytes(), 0);
        assert!(writer.is_idle());
        assert!(output.push(b"new screen after earlier check"));
        let result = output.when_idle(|| writer.write_if_idle(b"guarded choice"));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        let state = writer.queue.state.lock().unwrap();
        assert!(state.messages.is_empty());
        assert_eq!(state.pending_messages, 0);
        assert_eq!(state.pending_bytes, 0);
    }

    #[test]
    fn guarded_input_rejects_queued_and_in_flight_messages_without_partial_enqueue() {
        let output = crate::output_buffer::OutputBuffer::new(32);
        let writer = inspectable_writer();
        writer.write(b"first").unwrap();
        assert_eq!(
            output
                .when_idle(|| writer.write_if_idle(b"guarded choice"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        // The worker has removed this message for a native write, but the
        // pending counters must still keep the semantic input gate closed.
        let in_flight = writer
            .queue
            .state
            .lock()
            .unwrap()
            .messages
            .pop_front()
            .unwrap();
        assert_eq!(in_flight, b"first");
        assert_eq!(
            output
                .when_idle(|| writer.write_if_idle(b"guarded choice"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        let state = writer.queue.state.lock().unwrap();
        assert!(state.messages.is_empty());
        assert_eq!(state.pending_messages, 1);
        assert_eq!(state.pending_bytes, 5);
    }

    #[test]
    fn stopped_or_finished_worker_rejects_guarded_input() {
        let output = crate::output_buffer::OutputBuffer::new(32);
        let writer = inspectable_writer();
        writer.cancel();
        assert_eq!(
            output
                .when_idle(|| writer.write_if_idle(b"guarded choice"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(writer.queue.state.lock().unwrap().pending_messages, 0);

        let worker = std::thread::spawn(|| ());
        let deadline = Instant::now() + Duration::from_secs(1);
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(worker.is_finished());
        let writer = PtyInputWriter {
            queue: Arc::new(InputQueue::default()),
            worker: Some(worker),
        };
        assert_eq!(
            output
                .when_idle(|| writer.write_if_idle(b"guarded choice"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(writer.queue.state.lock().unwrap().pending_messages, 0);
    }

    #[test]
    fn guarded_enqueue_and_normal_writes_keep_fifo_and_whole_messages() {
        let output = crate::output_buffer::OutputBuffer::new(32);
        let writer = inspectable_writer();
        output
            .when_idle(|| writer.write_if_idle(b"guarded choice"))
            .unwrap();
        writer.write(b"next normal input").unwrap();
        writer.write(b"last normal input").unwrap();
        let state = writer.queue.state.lock().unwrap();
        assert_eq!(
            state.messages.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            [
                &b"guarded choice"[..],
                &b"next normal input"[..],
                &b"last normal input"[..]
            ]
        );
        assert_eq!(state.pending_messages, 3);
        assert_eq!(state.pending_bytes, 14 + 17 + 17);
    }
}
