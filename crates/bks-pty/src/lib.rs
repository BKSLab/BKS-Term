//! Управление сеансами Windows ConPTY для BKS Term.

mod input_writer;
mod output_buffer;

pub use input_writer::{PtyInputWriter, INPUT_QUEUE_MAX_BYTES, INPUT_QUEUE_MAX_MESSAGES};
pub use output_buffer::{OUTPUT_BUFFER_MAX_BYTES, OUTPUT_DRAIN_BUDGET};

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use output_buffer::OutputBuffer;

use windows::core::{Error as WinError, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_OPERATION_ABORTED, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use windows::Win32::System::IO::CancelSynchronousIo;

/// Размер псевдотерминала в символьных ячейках.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl Default for PtySize {
    /// Стандартный размер консоли Windows.
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

// HANDLE/HPCON хранятся как isize: это сохраняет Send для PtySession
// (HANDLE в windows-rs — *mut c_void, сырые указатели не Send).
pub struct PtySession {
    input: isize,
    process: isize,
    hpc: Mutex<ConsoleState>,
    output: Arc<OutputBuffer>,
    reader_stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

struct ConsoleState {
    handle: Option<isize>,
    closer: Option<JoinHandle<()>>,
}

fn win_err(e: WinError) -> io::Error {
    io::Error::other(e)
}

fn handle(raw: isize) -> HANDLE {
    HANDLE(raw as *mut c_void)
}

impl PtySession {
    pub fn spawn(command: &str, size: PtySize) -> io::Result<PtySession> {
        Self::spawn_in(command, size, None)
    }

    /// Start a child in an explicit directory without changing the parent
    /// process directory or environment. Validate before allocating ConPTY.
    pub fn spawn_in(command: &str, size: PtySize, cwd: Option<&Path>) -> io::Result<PtySession> {
        if command.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command contains a NUL character",
            ));
        }
        let current_directory = cwd
            .map(|path| {
                if !path.is_absolute() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "working directory must be an absolute path",
                    ));
                }
                let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
                if wide.contains(&0) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "working directory contains a NUL character",
                    ));
                }
                if !std::fs::metadata(path)?.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "working directory must be an existing directory",
                    ));
                }
                wide.push(0);
                Ok(wide)
            })
            .transpose()?;
        // stdin: PTY читает из pty_in_read, мы пишем в our_in_write.
        let mut pty_in_read = HANDLE::default();
        let mut our_in_write = HANDLE::default();
        // SAFETY: CreatePipe создаёт два валидных handle; None = атрибуты по умолчанию.
        unsafe { CreatePipe(&mut pty_in_read, &mut our_in_write, None, 0) }.map_err(win_err)?;

        // stdout: PTY пишет в pty_out_write, мы читаем из our_out_read.
        let mut our_out_read = HANDLE::default();
        let mut pty_out_write = HANDLE::default();
        if let Err(e) = unsafe { CreatePipe(&mut our_out_read, &mut pty_out_write, None, 0) } {
            // SAFETY: закрываем уже созданные handle stdin-pipe.
            unsafe {
                let _ = CloseHandle(pty_in_read);
                let _ = CloseHandle(our_in_write);
            }
            return Err(win_err(e));
        }

        let coord = COORD {
            X: size.cols as i16,
            Y: size.rows as i16,
        };
        // SAFETY: CreatePseudoConsole дублирует переданные handle; hpc валиден при успехе.
        let hpc = match unsafe { CreatePseudoConsole(coord, pty_in_read, pty_out_write, 0) } {
            Ok(hpc) => hpc,
            Err(e) => {
                // SAFETY: освобождаем все созданные к этому моменту handle.
                unsafe {
                    let _ = CloseHandle(pty_in_read);
                    let _ = CloseHandle(our_in_write);
                    let _ = CloseHandle(our_out_read);
                    let _ = CloseHandle(pty_out_write);
                }
                return Err(win_err(e));
            }
        };

        // Определяем размер буфера для списка атрибутов (первый вызов намеренно
        // завершается ошибкой, возвращая требуемый размер).
        let mut attr_size = 0usize;
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut attr_size) };
        if attr_size == 0 {
            // SAFETY: освобождаем созданные ресурсы.
            unsafe {
                ClosePseudoConsole(hpc);
                let _ = CloseHandle(pty_in_read);
                let _ = CloseHandle(our_in_write);
                let _ = CloseHandle(our_out_read);
                let _ = CloseHandle(pty_out_write);
            }
            return Err(io::Error::other("attribute list size is zero"));
        }

        // Выравниваем буфер под указатель.
        let mut attr_buf = vec![0usize; attr_size.div_ceil(std::mem::size_of::<usize>())];
        let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_buf.as_mut_ptr() as *mut c_void);
        // SAFETY: буфер достаточного размера и выровнен; attr_list валиден.
        if let Err(e) =
            unsafe { InitializeProcThreadAttributeList(Some(attr_list), 1, None, &mut attr_size) }
        {
            // SAFETY: освобождаем созданные ресурсы.
            unsafe {
                ClosePseudoConsole(hpc);
                let _ = CloseHandle(pty_in_read);
                let _ = CloseHandle(our_in_write);
                let _ = CloseHandle(our_out_read);
                let _ = CloseHandle(pty_out_write);
            }
            return Err(win_err(e));
        }

        // SAFETY: attr_list инициализирован; lpValue — само значение HPCON,
        // hpc жив до ClosePseudoConsole (образец MSDN передаёт hPC по значению).
        if let Err(e) = unsafe {
            UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(hpc.0 as *const c_void),
                std::mem::size_of::<HPCON>(),
                None,
                None,
            )
        } {
            // SAFETY: освобождаем созданные ресурсы.
            unsafe {
                DeleteProcThreadAttributeList(attr_list);
                ClosePseudoConsole(hpc);
                let _ = CloseHandle(pty_in_read);
                let _ = CloseHandle(our_in_write);
                let _ = CloseHandle(our_out_read);
                let _ = CloseHandle(pty_out_write);
            }
            return Err(win_err(e));
        }

        let mut si_ex = STARTUPINFOEXW::default();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si_ex.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        si_ex.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        si_ex.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        si_ex.lpAttributeList = attr_list;

        let mut cmd: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let mut pi = PROCESS_INFORMATION::default();
        // SAFETY: cmd — изменяемый UTF-16 буфер с нулём; si_ex содержит валидный список атрибутов.
        let create_res = unsafe {
            CreateProcessW(
                None,
                Some(PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                BOOL(0).into(),
                EXTENDED_STARTUPINFO_PRESENT,
                None,
                current_directory
                    .as_ref()
                    .map_or(PCWSTR::null(), |wide| PCWSTR(wide.as_ptr())),
                &si_ex.StartupInfo,
                &mut pi,
            )
        };

        // SAFETY: список атрибутов больше не нужен после CreateProcessW.
        unsafe { DeleteProcThreadAttributeList(attr_list) };

        if let Err(e) = create_res {
            // SAFETY: освобождаем созданные ресурсы.
            unsafe {
                ClosePseudoConsole(hpc);
                let _ = CloseHandle(pty_in_read);
                let _ = CloseHandle(our_in_write);
                let _ = CloseHandle(our_out_read);
                let _ = CloseHandle(pty_out_write);
            }
            return Err(win_err(e));
        }

        // SAFETY: hThread больше не нужен; hProcess сохраняем для Drop.
        unsafe {
            let _ = CloseHandle(pi.hThread);
        }

        // Закрываем PTY-концы pipe: они больше не нужны нашему процессу.
        // SAFETY: CreatePseudoConsole продублировал эти handle.
        unsafe {
            let _ = CloseHandle(pty_in_read);
            let _ = CloseHandle(pty_out_write);
        }

        let process = pi.hProcess.0 as isize;
        let input = our_in_write.0 as isize;
        let hpc_val = hpc.0;
        let out_handle = our_out_read.0 as isize;
        let output = Arc::new(OutputBuffer::new(OUTPUT_BUFFER_MAX_BYTES));
        let output_clone = Arc::clone(&output);
        let reader_stop = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&reader_stop);

        let reader = std::thread::Builder::new()
            .name("bks-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                let mut read_error = None;
                while !stop.load(Ordering::Acquire) {
                    let mut read = 0u32;
                    // SAFETY: out_handle валиден до закрытия в конце потока.
                    let res = unsafe {
                        ReadFile(
                            handle(out_handle),
                            Some(&mut buf[..]),
                            Some(&mut read),
                            None,
                        )
                    };
                    match res {
                        Ok(()) if read > 0 => {
                            // Backpressure blocks only this reader. During
                            // shutdown keep draining ConPTY without buffering
                            // so ClosePseudoConsole can finish on a full pipe.
                            output_clone.push(&buf[..read as usize]);
                        }
                        Ok(()) => break,
                        Err(error) => {
                            let code = error.code().0 as u32 & 0xffff;
                            if code != ERROR_BROKEN_PIPE.0
                                && code != ERROR_HANDLE_EOF.0
                                && !(code == ERROR_OPERATION_ABORTED.0
                                    && stop.load(Ordering::Acquire))
                            {
                                read_error = Some(code as i32);
                            }
                            break;
                        }
                    }
                }
                // SAFETY: поток владеет out_handle и закрывает его ровно один раз.
                unsafe {
                    let _ = CloseHandle(handle(out_handle));
                }
                output_clone.finish(read_error);
            });

        let reader = match reader {
            Ok(reader) => reader,
            Err(e) => {
                // SAFETY: освобождаем созданные ресурсы при ошибке создания потока.
                unsafe {
                    let _ = CloseHandle(handle(input));
                    let _ = TerminateProcess(handle(process), 1);
                    let _ = CloseHandle(handle(out_handle));
                    ClosePseudoConsole(HPCON(hpc_val));
                    let _ = CloseHandle(handle(process));
                }
                return Err(e);
            }
        };

        Ok(PtySession {
            input,
            process,
            hpc: Mutex::new(ConsoleState {
                handle: Some(hpc_val),
                closer: None,
            }),
            output,
            reader_stop,
            reader: Some(reader),
        })
    }

    pub fn drain_output(&self) -> Vec<u8> {
        self.drain_output_limited(usize::MAX)
    }

    /// Drain at most max_bytes without waiting for new output. Remaining raw
    /// bytes stay queued, including incomplete UTF-8 or VT sequences.
    pub fn drain_output_limited(&self, max_bytes: usize) -> Vec<u8> {
        self.output.drain(max_bytes)
    }

    /// No more bytes can be enqueued by the reader; queued bytes may remain.
    pub fn output_finished(&self) -> bool {
        self.output.finished()
    }

    pub fn pending_output_bytes(&self) -> usize {
        self.output.pending_bytes()
    }

    /// Reader completion plus an empty queue. Combine with exit_code() before
    /// marking a session exited, so its final output is consumed first.
    pub fn output_drained(&self) -> bool {
        self.output.drained()
    }

    pub fn output_error(&self) -> Option<io::Error> {
        self.output.error()
    }

    /// After the owned process has exited, finish ConPTY on a background
    /// thread so its output pipe can reach EOF. Keep draining output while
    /// this idempotent operation runs; it never closes the byte queue.
    pub fn request_output_finish(&self) -> io::Result<()> {
        if self.exit_code()?.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "PTY process is still running",
            ));
        }
        let mut console = self.hpc.lock().unwrap_or_else(|error| error.into_inner());
        let Some(hpc) = console.handle.take() else {
            return Ok(());
        };
        match std::thread::Builder::new()
            .name("bks-pty-finalizer".into())
            .spawn(move || {
                // This thread owns the taken HPCON; the reader stays alive until
                // ClosePseudoConsole finishes, including under queue backpressure.
                unsafe { ClosePseudoConsole(HPCON(hpc)) };
            }) {
            Ok(closer) => {
                console.closer = Some(closer);
                Ok(())
            }
            Err(error) => {
                console.handle = Some(hpc);
                Err(error)
            }
        }
    }

    pub fn write_input(&self, data: &[u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < data.len() {
            let mut written = 0u32;
            // SAFETY: self.input — валидный write-конец pipe до Drop.
            unsafe {
                WriteFile(
                    handle(self.input),
                    Some(&data[offset..]),
                    Some(&mut written),
                    None,
                )
            }
            .map_err(win_err)?;
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "PTY input closed",
                ));
            }
            offset += written as usize;
        }
        Ok(())
    }

    /// Создаёт FIFO writer, который не блокирует UI на записи в ConPTY.
    /// Writer нужно освободить до сеанса, чтобы закрыть его duplicate handle.
    pub fn input_writer(&self) -> io::Result<PtyInputWriter> {
        PtyInputWriter::new(handle(self.input))
    }

    /// Queue an explicit semantic action only while both output and input are
    /// idle. Use the writer created by this session. The output-reader enqueue
    /// and this final readiness check share one critical section; pipe I/O is
    /// performed later by the writer, without holding the output-buffer lock.
    pub fn write_when_idle(&self, writer: &PtyInputWriter, data: &[u8]) -> io::Result<()> {
        self.output.when_idle(|| writer.write_if_idle(data))
    }

    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        let coord = COORD {
            X: size.cols as i16,
            Y: size.rows as i16,
        };
        let console = self.hpc.lock().unwrap_or_else(|error| error.into_inner());
        let hpc = console
            .handle
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY console is finishing"))?;
        // The lock prevents transferring/closing HPCON during this call.
        unsafe { ResizePseudoConsole(HPCON(hpc), coord) }.map_err(win_err)
    }

    pub fn is_alive(&self) -> bool {
        if self.process == 0 {
            return false;
        }
        // SAFETY: self.process — валидный handle процесса до Drop.
        unsafe { WaitForSingleObject(handle(self.process), 0) == WAIT_TIMEOUT }
    }

    /// A nonblocking exit status. Waiting for the process signal first avoids
    /// confusing a real exit code 259 with the STILL_ACTIVE sentinel.
    pub fn exit_code(&self) -> io::Result<Option<u32>> {
        let state = unsafe { WaitForSingleObject(handle(self.process), 0) };
        if state == WAIT_TIMEOUT {
            return Ok(None);
        }
        if state == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        if state != WAIT_OBJECT_0 {
            return Err(io::Error::other("unexpected PTY process wait state"));
        }
        let mut code = 0;
        unsafe { GetExitCodeProcess(handle(self.process), &mut code) }.map_err(win_err)?;
        Ok(Some(code))
    }

    /// Request termination of this session's owned process only. No waiting,
    /// queue closure or automatic restart; final output can still be drained.
    pub fn request_terminate(&self) -> io::Result<()> {
        if self.exit_code()?.is_some() {
            return Ok(());
        }
        match unsafe { TerminateProcess(handle(self.process), 1) } {
            Ok(()) => Ok(()),
            Err(_) if self.exit_code()?.is_some() => Ok(()),
            Err(error) => Err(win_err(error)),
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Wake a producer blocked on the cap before closing ConPTY. From this
        // point output is discarded only as part of destroying the session.
        self.output.close();
        // SAFETY: закрываем входной pipe, чтобы дочерний процесс получил EOF.
        unsafe {
            let _ = CloseHandle(handle(self.input));
        }
        // SAFETY: ждём завершения процесса до 3 секунд.
        let wait = unsafe { WaitForSingleObject(handle(self.process), 3000) };
        if wait == WAIT_TIMEOUT {
            // SAFETY: процесс не завершился — принудительно завершаем.
            unsafe {
                let _ = TerminateProcess(handle(self.process), 1);
            }
        }
        let console = self
            .hpc
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(closer) = console.closer.take() {
            let _ = closer.join();
        } else if let Some(hpc) = console.handle.take() {
            // SAFETY: this Drop owns HPCON unless a finalizer has taken it.
            unsafe { ClosePseudoConsole(HPCON(hpc)) };
        }
        self.reader_stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let thread = HANDLE(reader.as_raw_handle());
            // Repeat cancellation to cover stop arriving just before ReadFile.
            // The reader closes its own handle only after the I/O has returned.
            while unsafe { WaitForSingleObject(thread, 0) } == WAIT_TIMEOUT {
                unsafe {
                    let _ = CancelSynchronousIo(thread);
                    WaitForSingleObject(thread, 5);
                }
            }
            let _ = reader.join();
        }
        // SAFETY: закрываем handle процесса.
        unsafe {
            let _ = CloseHandle(handle(self.process));
        }
    }
}
