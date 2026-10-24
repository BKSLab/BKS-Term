use portable_pty::{Child, ExitStatus, MasterPty};
use std::io::{Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MARKER: &str = "bks-portable-marker";
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const TIMEOUT: Duration = Duration::from_secs(10);

enum ReadEvent {
    Data(Vec<u8>),
    Done,
    Error(String),
}

fn main() {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(portable_pty::PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap_or_else(|error| {
            eprintln!("FAIL: не удалось открыть PTY: {error}");
            std::process::exit(1);
        });
    let reader = pair.master.try_clone_reader().unwrap_or_else(|error| {
        eprintln!("FAIL: не удалось открыть поток вывода PTY: {error}");
        std::process::exit(1);
    });
    let writer = pair.master.take_writer().unwrap_or_else(|error| {
        eprintln!("FAIL: не удалось открыть поток ввода PTY: {error}");
        std::process::exit(1);
    });
    let mut cmd = portable_pty::CommandBuilder::new("cmd.exe");
    cmd.args(["/c", "echo", MARKER]);
    let mut child = pair.slave.spawn_command(cmd).unwrap_or_else(|error| {
        eprintln!("FAIL: не удалось запустить cmd.exe: {error}");
        std::process::exit(1);
    });
    drop(pair.slave);

    match collect_output(pair.master, &mut *child, reader, writer) {
        Ok((output, status)) => {
            println!("OK: маркер получен через ConPTY");
            println!("вывод: {:?}", String::from_utf8_lossy(&output));
            println!("код завершения: {}", status.exit_code());
        }
        Err(error) => {
            eprintln!("FAIL: {error}");
            if child.kill().is_ok() {
                let _ = child.wait();
            }
            std::process::exit(1);
        }
    }
}

fn collect_output(
    master: Box<dyn MasterPty + Send>,
    child: &mut dyn Child,
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
) -> Result<(Vec<u8>, ExitStatus), String> {
    let (sender, receiver) = mpsc::channel();
    let mut reader_thread = Some(thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(ReadEvent::Done);
                    return;
                }
                Ok(count) => {
                    if sender
                        .send(ReadEvent::Data(buffer[..count].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                    let _ = sender.send(ReadEvent::Done);
                    return;
                }
                Err(error) => {
                    let _ = sender.send(ReadEvent::Error(error.to_string()));
                    return;
                }
            }
        }
    }));

    let deadline = Instant::now() + TIMEOUT;
    let mut master = Some(master);
    let mut output = Vec::new();
    let mut exit_status = None;
    let mut reader_done = false;
    let mut cursor_answered = false;

    let result = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("не удалось проверить состояние cmd.exe: {error}"))?
        {
            if !status.success() {
                break Err(format!("cmd.exe завершился с кодом {}", status.exit_code()));
            }
            exit_status = Some(status);
        }

        let marker_received = String::from_utf8_lossy(&output).contains(MARKER);
        if marker_received && exit_status.is_some() && master.is_some() {
            drop(master.take());
        }
        if marker_received && reader_done {
            if let Some(status) = exit_status {
                break Ok(status);
            }
        }
        if reader_done && !marker_received {
            break Err("поток PTY завершился без ожидаемого маркера".to_string());
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err("истёк общий таймаут ожидания".to_string());
        }
        if reader_done {
            thread::sleep(POLL_INTERVAL.min(remaining));
            continue;
        }

        match receiver.recv_timeout(POLL_INTERVAL.min(remaining)) {
            Ok(ReadEvent::Data(data)) => {
                output.extend_from_slice(&data);
                if !cursor_answered && String::from_utf8_lossy(&output).contains("\u{1b}[6n") {
                    writer
                        .write_all(b"\x1b[1;1R")
                        .and_then(|()| writer.flush())
                        .map_err(|error| {
                            format!("не удалось ответить на запрос позиции курсора: {error}")
                        })?;
                    cursor_answered = true;
                }
            }
            Ok(ReadEvent::Done) => {
                join_reader(&mut reader_thread)?;
                reader_done = true;
            }
            Ok(ReadEvent::Error(error)) => {
                let _ = join_reader(&mut reader_thread);
                break Err(format!("ошибка чтения PTY: {error}"));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err("поток чтения PTY неожиданно отключился".to_string());
            }
        }
    };

    match result {
        Ok(status) => Ok((output, status)),
        Err(error) => Err(format!(
            "{error}; вывод: {:?}",
            String::from_utf8_lossy(&output)
        )),
    }
}

fn join_reader(reader_thread: &mut Option<JoinHandle<()>>) -> Result<(), String> {
    if let Some(handle) = reader_thread.take() {
        handle
            .join()
            .map_err(|_| "поток чтения PTY завершился с паникой".to_string())?;
    }
    Ok(())
}
