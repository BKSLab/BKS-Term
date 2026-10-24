//! Управление сеансами Windows ConPTY для BKS Term.

use std::ffi::c_void;
use std::io;
use std::sync::{Arc, Mutex};

use windows::core::{Error as WinError, BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

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
    hpc: isize,
    output: Arc<Mutex<Vec<u8>>>,
}

fn win_err(e: WinError) -> io::Error {
    io::Error::other(e)
}

fn handle(raw: isize) -> HANDLE {
    HANDLE(raw as *mut c_void)
}

impl PtySession {
    pub fn spawn(command: &str, size: PtySize) -> io::Result<PtySession> {
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
                None,
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
        let output = Arc::new(Mutex::new(Vec::new()));
        let output_clone = Arc::clone(&output);

        let reader = std::thread::Builder::new()
            .name("bks-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
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
                            if let Ok(mut v) = output_clone.lock() {
                                v.extend_from_slice(&buf[..read as usize]);
                            }
                        }
                        _ => break,
                    }
                }
                // SAFETY: поток владеет out_handle и закрывает его ровно один раз.
                unsafe {
                    let _ = CloseHandle(handle(out_handle));
                }
            });

        if let Err(e) = reader {
            // SAFETY: освобождаем созданные ресурсы при ошибке создания потока.
            unsafe {
                let _ = CloseHandle(handle(input));
                let _ = TerminateProcess(handle(process), 1);
                ClosePseudoConsole(HPCON(hpc_val));
                let _ = CloseHandle(handle(process));
                let _ = CloseHandle(handle(out_handle));
            }
            return Err(e);
        }

        Ok(PtySession {
            input,
            process,
            hpc: hpc_val,
            output,
        })
    }

    pub fn drain_output(&self) -> Vec<u8> {
        match self.output.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(_) => Vec::new(),
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

    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        let coord = COORD {
            X: size.cols as i16,
            Y: size.rows as i16,
        };
        // SAFETY: self.hpc валиден до Drop.
        unsafe { ResizePseudoConsole(HPCON(self.hpc), coord) }.map_err(win_err)
    }

    pub fn is_alive(&self) -> bool {
        if self.process == 0 {
            return false;
        }
        // SAFETY: self.process — валидный handle процесса до Drop.
        unsafe { WaitForSingleObject(handle(self.process), 0) == WAIT_TIMEOUT }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
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
        // SAFETY: закрываем псевдоконсоль после завершения процесса.
        unsafe { ClosePseudoConsole(HPCON(self.hpc)) };
        // SAFETY: закрываем handle процесса.
        unsafe {
            let _ = CloseHandle(handle(self.process));
        }
    }
}
