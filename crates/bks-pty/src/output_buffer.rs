use std::collections::VecDeque;
use std::io;
use std::sync::{Condvar, Mutex};

/// Maximum queued PTY bytes per session, excluding the reader's 8 KiB chunk.
pub const OUTPUT_BUFFER_MAX_BYTES: usize = 1024 * 1024;
/// Suggested per-session UI polling budget; callers may choose a smaller one.
pub const OUTPUT_DRAIN_BUDGET: usize = 64 * 1024;

pub(crate) struct OutputBuffer {
    capacity: usize,
    state: Mutex<State>,
    space: Condvar,
}

struct State {
    bytes: VecDeque<u8>,
    closed: bool,
    reader_finished: bool,
    reader_error: Option<i32>,
}

impl OutputBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            state: Mutex::new(State {
                bytes: VecDeque::with_capacity(capacity),
                closed: false,
                reader_finished: false,
                reader_error: None,
            }),
            space: Condvar::new(),
        }
    }

    /// Reader-only backpressure. False means shutdown: the reader may discard
    /// subsequent output while draining ConPTY during ClosePseudoConsole.
    pub(crate) fn push(&self, bytes: &[u8]) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut offset = 0;
        while offset < bytes.len() {
            while state.bytes.len() == self.capacity && !state.closed && !state.reader_finished {
                state = self
                    .space
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed || state.reader_finished {
                return false;
            }
            let count = (self.capacity - state.bytes.len()).min(bytes.len() - offset);
            state.bytes.extend(&bytes[offset..offset + count]);
            offset += count;
        }
        true
    }

    /// Never waits for new output or producer progress, and retains all bytes
    /// beyond the requested budget for the next FIFO drain.
    pub(crate) fn drain(&self, max_bytes: usize) -> Vec<u8> {
        if max_bytes == 0 {
            return Vec::new();
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let count = state.bytes.len().min(max_bytes);
        let output = state.bytes.drain(..count).collect();
        if count > 0 {
            self.space.notify_all();
        }
        output
    }

    pub(crate) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        self.space.notify_all();
    }

    /// Publish reader completion after its last push and handle close. Queued
    /// bytes remain available: reader completion alone is not a final drain.
    pub(crate) fn finish(&self, error: Option<i32>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reader_error = error;
        state.reader_finished = true;
        self.space.notify_all();
    }

    pub(crate) fn finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reader_finished
    }

    pub(crate) fn pending_bytes(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bytes
            .len()
    }

    /// Linearize a short input-queue operation against output publication.
    /// The callback may lock the input queue, but must never perform pipe I/O,
    /// wait for the worker, or call back into this output buffer.
    pub(crate) fn when_idle<T>(&self, enqueue: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed || state.reader_finished {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "PTY output is closed",
            ));
        }
        if !state.bytes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "PTY output changed before input could be queued",
            ));
        }
        let result = enqueue();
        // Keep the guard alive through enqueue: a reader's push either precedes
        // this check or follows the completed input-queue operation.
        drop(state);
        result
    }

    pub(crate) fn drained(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.reader_finished && state.bytes.is_empty()
    }

    pub(crate) fn error(&self) -> Option<io::Error> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reader_error
            .map(io::Error::from_raw_os_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Barrier, TryLockError};
    use std::time::{Duration, Instant};

    #[test]
    fn limited_drain_keeps_fifo_and_zero_does_not_consume() {
        let output = OutputBuffer::new(32);
        let bytes = "\x1b[31mЯ🙂e\u{301}\x1b[0m".as_bytes();
        assert!(output.push(&bytes[..7]));
        assert!(output.push(&bytes[7..]));
        assert!(output.drain(0).is_empty());
        let mut drained = Vec::new();
        while drained.len() < bytes.len() {
            let chunk = output.drain(3);
            assert!(chunk.len() <= 3);
            drained.extend(chunk);
        }
        assert_eq!(drained, bytes);
        assert!(output.drain(usize::MAX).is_empty());
    }

    #[test]
    fn backpressure_preserves_all_bytes_across_ring_wraps() {
        let output = Arc::new(OutputBuffer::new(17));
        let expected: Vec<u8> = (0..4099).map(|index| (index % 251) as u8).collect();
        let payload = expected.clone();
        let producer_output = Arc::clone(&output);
        let producer = std::thread::spawn(move || producer_output.push(&payload));
        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while received.len() < expected.len() && Instant::now() < deadline {
            assert!(output.state.lock().unwrap().bytes.len() <= 17);
            received.extend(output.drain(7));
            std::thread::yield_now();
        }
        output.close();
        let finished = producer.join().unwrap();
        assert!(finished, "producer did not finish before timeout");
        assert_eq!(received, expected);
    }

    #[test]
    fn closing_full_buffer_wakes_waiting_producer() {
        let output = Arc::new(OutputBuffer::new(8));
        assert!(output.push(b"12345678"));
        let producer_output = Arc::clone(&output);
        let (done, completed) = mpsc::channel();
        let producer = std::thread::spawn(move || {
            done.send(producer_output.push(b"more output")).unwrap();
        });
        assert!(completed.recv_timeout(Duration::from_millis(30)).is_err());
        output.close();
        assert!(!completed
            .recv_timeout(Duration::from_secs(1))
            .expect("producer remained blocked"));
        producer.join().unwrap();
        assert_eq!(output.drain(usize::MAX), b"12345678");
        assert!(!output.push(b"after close"));
    }

    #[test]
    fn reader_finished_keeps_its_final_tail_until_it_is_drained() {
        let output = OutputBuffer::new(16);
        assert!(output.push(b"final tail"));
        assert!(!output.finished());
        assert!(!output.drained());
        output.finish(None);
        assert!(output.finished());
        assert_eq!(output.pending_bytes(), 10);
        assert!(!output.drained());
        assert_eq!(output.drain(4), b"fina");
        assert_eq!(output.pending_bytes(), 6);
        assert!(!output.drained());
        assert_eq!(output.drain(usize::MAX), b"l tail");
        assert!(output.drained());
        assert!(output.error().is_none());
        assert!(!output.push(b"after finish"));
    }

    #[test]
    fn read_error_preserves_buffer_and_can_be_read_more_than_once() {
        let output = OutputBuffer::new(16);
        assert!(output.push(b"tail"));
        output.finish(Some(5));
        assert_eq!(output.error().unwrap().raw_os_error(), Some(5));
        assert_eq!(output.error().unwrap().raw_os_error(), Some(5));
        assert_eq!(output.drain(16), b"tail");
        assert!(output.drained());
    }

    #[test]
    fn completion_waits_for_the_last_backpressured_enqueue() {
        let output = Arc::new(OutputBuffer::new(8));
        let producer_output = Arc::clone(&output);
        let payload = b"backpressured final tail";
        let producer = std::thread::spawn(move || {
            let accepted = producer_output.push(payload);
            producer_output.finish(None);
            accepted
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while output.pending_bytes() < 8 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(output.pending_bytes(), 8);
        assert!(!output.finished());
        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !output.drained() && Instant::now() < deadline {
            received.extend(output.drain(3));
            std::thread::yield_now();
        }
        output.close();
        assert!(producer.join().unwrap());
        assert!(output.finished());
        assert!(output.drained());
        assert_eq!(received, payload);
    }

    #[test]
    fn idle_guard_rejects_pending_closed_or_finished_output_without_calling_enqueue() {
        let output = OutputBuffer::new(32);
        assert_eq!(output.pending_bytes(), 0);
        assert!(output.push(b"arrived after the earlier check"));
        let mut called = false;
        let result = output.when_idle(|| {
            called = true;
            Ok(())
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert!(!called);
        output.drain(32);
        output.close();
        assert_eq!(
            output
                .when_idle::<()>(|| panic!("closed queue called enqueue"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        let output = OutputBuffer::new(32);
        output.finish(None);
        assert_eq!(
            output
                .when_idle::<()>(|| panic!("finished queue called enqueue"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn reader_push_cannot_overtake_a_guarded_input_enqueue() {
        let output = Arc::new(OutputBuffer::new(32));
        let in_critical_section = Arc::new(Barrier::new(2));
        let order = Arc::new(Mutex::new(Vec::new()));
        let (release, released) = mpsc::channel();
        let guard_output = Arc::clone(&output);
        let guard_barrier = Arc::clone(&in_critical_section);
        let guard_order = Arc::clone(&order);
        let guard = std::thread::spawn(move || {
            guard_output.when_idle(|| {
                guard_barrier.wait();
                released.recv_timeout(Duration::from_secs(2)).unwrap();
                guard_order.lock().unwrap().push("input enqueue");
                Ok(())
            })
        });
        in_critical_section.wait();
        let producer_output = Arc::clone(&output);
        let producer_order = Arc::clone(&order);
        let (observed, blocked) = mpsc::channel();
        let producer = std::thread::spawn(move || {
            // This check is deterministic while the enqueue callback waits on
            // released; it fails if when_idle ever releases its lock too soon.
            assert!(matches!(
                producer_output.state.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            observed.send(()).unwrap();
            assert!(producer_output.push(b"new output"));
            producer_order.lock().unwrap().push("output push");
        });
        blocked.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(order.lock().unwrap().is_empty());
        release.send(()).unwrap();
        guard.join().unwrap().unwrap();
        producer.join().unwrap();
        assert_eq!(*order.lock().unwrap(), ["input enqueue", "output push"]);
        assert_eq!(output.drain(32), b"new output");
    }
}
