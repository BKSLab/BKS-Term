//! Интеграционные тесты: реальные ConPTY-сеансы cmd.exe и PowerShell.

use std::time::{Duration, Instant};

use bks_pty::{PtySession, PtySize, INPUT_QUEUE_MAX_BYTES, OUTPUT_BUFFER_MAX_BYTES};

/// Ждём появления маркера в выводе, опрашивая drain_output.
fn wait_for_marker(session: &PtySession, marker: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut acc = Vec::new();
    while Instant::now() < deadline {
        acc.extend_from_slice(&session.drain_output());
        let text = String::from_utf8_lossy(&acc);
        if text.contains(marker) {
            return text.into_owned();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    String::from_utf8_lossy(&acc).into_owned()
}

#[test]
fn thirty_two_owned_lifetimes_drain_and_close_reader_writer_and_process() {
    for cycle in 0..32 {
        let normal_exit = cycle % 2 == 0;
        let marker = format!("BKS_LIFETIME_{cycle:02}");
        let command = if normal_exit {
            format!("cmd.exe /d /q /c echo {marker}")
        } else {
            "cmd.exe /d /q".to_owned()
        };
        let session = PtySession::spawn(&command, PtySize::default()).unwrap();
        let writer = session.input_writer().unwrap();
        if !normal_exit {
            writer
                .write(format!("echo {marker}\r\n").as_bytes())
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        while Instant::now() < deadline {
            output.extend(session.drain_output_limited(4096));
            if String::from_utf8_lossy(&output).contains(&marker) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            String::from_utf8_lossy(&output).contains(&marker),
            "missing cycle {cycle}"
        );
        if !normal_exit {
            session.request_terminate().unwrap();
        }
        while Instant::now() < deadline && session.exit_code().unwrap().is_none() {
            output.extend(session.drain_output_limited(4096));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session.exit_code().unwrap(),
            Some(if normal_exit { 0 } else { 1 })
        );
        session.request_output_finish().unwrap();
        while Instant::now() < deadline && !session.output_drained() {
            output.extend(session.drain_output_limited(4096));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            session.output_drained(),
            "reader/queue did not finish for cycle {cycle}"
        );
        drop(writer);
        drop(session);
    }
}

#[test]
fn cmd_echo_roundtrip() {
    let session = PtySession::spawn("cmd.exe /c echo bks-pty-test-marker", PtySize::default())
        .expect("spawn cmd.exe через ConPTY");

    let out = wait_for_marker(&session, "bks-pty-test-marker", Duration::from_secs(15));
    assert!(
        out.contains("bks-pty-test-marker"),
        "маркер не найден в выводе: {out:?}"
    );

    // Процесс cmd /c завершается сам; даём ему момент и проверяем is_alive=false.
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!session.is_alive(), "cmd.exe /c должен был завершиться");
}

#[test]
fn interactive_echo_via_input() {
    let session = PtySession::spawn("cmd.exe /q", PtySize::default()).expect("spawn cmd.exe");

    session
        .write_input(b"echo bks-interactive-marker\r\n")
        .expect("write_input");
    let out = wait_for_marker(&session, "bks-interactive-marker", Duration::from_secs(15));
    assert!(
        out.contains("bks-interactive-marker"),
        "маркер не найден в выводе: {out:?}"
    );

    // resize не должен падать на живом сеансе.
    session.resize(PtySize::new(120, 40)).expect("resize");

    // Завершаем сеанс командой exit, затем Drop доберёт остаток.
    session.write_input(b"exit\r\n").expect("write exit");
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!session.is_alive(), "cmd.exe должен завершиться после exit");
}

#[test]
fn powershell_multiline_input() {
    let session = PtySession::spawn("powershell.exe -NoLogo -NoProfile", PtySize::default())
        .expect("spawn powershell.exe");

    session
        .write_input(b"Write-Host 'bks-multiline-one'\r\nWrite-Host 'bks-multiline-two'\r\n")
        .expect("write multiline input");
    let out = wait_for_marker(&session, "bks-multiline-two", Duration::from_secs(15));
    assert!(
        out.contains("bks-multiline-one") && out.contains("bks-multiline-two"),
        "многострочный ввод не выполнился полностью: {out:?}"
    );

    session.write_input(b"exit\r\n").expect("write exit");
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !session.is_alive(),
        "powershell.exe должен завершиться после exit"
    );
}

#[test]
fn powershell_osc_command_block_marker() {
    let session = PtySession::spawn(
        r#"powershell.exe -NoLogo -NoProfile -NoExit -Command "function global:prompt { [char]27 + ']133;A' + [char]27 + '\PS ' + $executionContext.SessionState.Path.CurrentLocation + '> ' + [char]27 + ']133;B' + [char]27 + '\' }""#,
        PtySize::default(),
    )
    .expect("spawn powershell.exe with OSC prompt");

    let out = wait_for_marker(&session, "\x1b]133;B\x1b\\", Duration::from_secs(15));
    assert!(
        out.contains("\x1b]133;A\x1b\\") && out.contains("\x1b]133;B\x1b\\"),
        "prompt не отправил OSC 133;A/B: {out:?}"
    );
    session
        .write_input(b"Write-Host 'bks-command-block-ready'\r\n")
        .expect("write command");
    let out = wait_for_marker(&session, "bks-command-block-ready", Duration::from_secs(15));
    assert!(out.contains("bks-command-block-ready"), "вывод: {out:?}");

    session.write_input(b"exit\r\n").expect("write exit");
}

#[test]
fn queued_input_roundtrip_preserves_fifo_across_messages() {
    let session = PtySession::spawn("cmd.exe /d /q", PtySize::default()).expect("spawn cmd.exe");
    let writer = session.input_writer().expect("create input writer");
    // Маркеры формируются cmd через переменную; эхо введённой команды само
    // по себе не может удовлетворить проверку выполненного roundtrip.
    writer.write(b"set BKS_").expect("queue first fragment");
    writer
        .write(b"FIFO=first\r\n")
        .expect("queue second fragment");
    writer
        .write(b"echo fifo-%BKS_FIFO%\r\n")
        .expect("queue first echo");
    writer
        .write(b"set BKS_FIFO=second\r\n")
        .expect("queue second value");
    writer
        .write(b"echo fifo-%BKS_FIFO%\r\n")
        .expect("queue second echo");
    writer.write(b"exit\r\n").expect("queue exit");

    let out = wait_for_marker(&session, "fifo-second", Duration::from_secs(15));
    let first = out
        .find("fifo-first")
        .expect("first command did not execute");
    let second = out
        .find("fifo-second")
        .expect("second command did not execute");
    assert!(first < second, "commands were reordered: {out:?}");
    writer
        .check_error()
        .expect("writer failed during roundtrip");

    let deadline = Instant::now() + Duration::from_secs(5);
    while session.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!session.is_alive(), "queued exit did not execute");
    drop(writer);
}

#[test]
fn queued_input_drop_does_not_wait_for_large_unconsumed_input() {
    use std::sync::mpsc;

    let (finished, completion) = mpsc::channel();
    let test_thread = std::thread::spawn(move || {
        let session = PtySession::spawn(
            r#"powershell.exe -NoLogo -NoProfile -Command "[Console]::Write('bks-sleeping'); Start-Sleep -Seconds 30""#,
            PtySize::default(),
        )
        .expect("spawn child that does not read input");
        let out = wait_for_marker(&session, "bks-sleeping", Duration::from_secs(15));
        assert!(out.contains("bks-sleeping"), "child did not start: {out:?}");
        let writer = session.input_writer().expect("create input writer");
        let too_large = vec![b'x'; INPUT_QUEUE_MAX_BYTES + 1];
        assert_eq!(
            writer
                .write(&too_large)
                .expect_err("oversized input accepted")
                .kind(),
            std::io::ErrorKind::WouldBlock,
        );
        let queued = vec![b'x'; INPUT_QUEUE_MAX_BYTES];
        let start = Instant::now();
        writer.write(&queued).expect("queue large input");
        assert!(start.elapsed() < Duration::from_secs(1), "enqueue blocked");
        // Даём worker войти в запись; блокировка pipe не должна блокировать Drop.
        std::thread::sleep(Duration::from_millis(30));
        let start = Instant::now();
        drop(writer);
        let elapsed = start.elapsed();
        // Session освобождается после writer; его существующая политика ждёт
        // процесс до 3 секунд и завершает спящий дочерний процесс.
        drop(session);
        finished.send(elapsed).unwrap();
    });
    let elapsed = completion
        .recv_timeout(Duration::from_secs(20))
        .expect("writer did not finish while child was not reading input");
    assert!(
        elapsed < Duration::from_secs(3),
        "writer Drop was slow: {elapsed:?}"
    );
    test_thread.join().expect("input writer test thread failed");
}

#[test]
fn limited_output_drains_keep_fifo_across_small_budgets() {
    let session = PtySession::spawn(
        r#"cmd.exe /d /q /c "echo BKS_LIMITED_ONE&echo BKS_LIMITED_TWO&echo BKS_LIMITED_THREE""#,
        PtySize::default(),
    )
    .expect("spawn cmd.exe output producer");
    assert!(session.drain_output_limited(0).is_empty());
    let mut received = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let chunk = session.drain_output_limited(7);
        assert!(chunk.len() <= 7);
        received.extend(chunk);
        if String::from_utf8_lossy(&received).contains("BKS_LIMITED_THREE") {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let output = String::from_utf8_lossy(&received);
    let first = output
        .find("BKS_LIMITED_ONE")
        .expect("missing first marker");
    let second = output
        .find("BKS_LIMITED_TWO")
        .expect("missing second marker");
    let third = output
        .find("BKS_LIMITED_THREE")
        .expect("missing third marker");
    assert!(
        first < second && second < third,
        "output reordered: {output:?}"
    );
}

#[test]
fn closing_session_with_full_output_buffer_finishes_without_a_consumer() {
    use std::sync::mpsc;

    let (finished, completion) = mpsc::channel();
    let test_thread = std::thread::spawn(move || {
        // More than four queue capacities remain after the initial ready drain.
        let command = format!(
            r#"powershell.exe -NoLogo -NoProfile -Command "[Console]::WriteLine('BKS_OUTPUT_READY'); [Console]::Write(('x' * {})); [Console]::WriteLine('BKS_OUTPUT_FINISHED')""#,
            OUTPUT_BUFFER_MAX_BYTES * 5,
        );
        let session =
            PtySession::spawn(&command, PtySize::default()).expect("spawn large output producer");
        let ready = wait_for_marker(&session, "BKS_OUTPUT_READY", Duration::from_secs(15));
        assert!(ready.contains("BKS_OUTPUT_READY"), "producer did not start");
        // Stop consuming, allowing output to fill and reader to wait for space.
        std::thread::sleep(Duration::from_millis(500));
        let start = Instant::now();
        drop(session);
        finished.send(start.elapsed()).unwrap();
    });
    let elapsed = completion
        .recv_timeout(Duration::from_secs(25))
        .expect("session close blocked behind output backpressure");
    assert!(
        elapsed < Duration::from_secs(6),
        "session close was slow: {elapsed:?}"
    );
    test_thread
        .join()
        .expect("output shutdown test thread failed");
}

#[test]
fn immediate_exit_reports_code_seven_but_final_tail_must_still_be_drained() {
    let session = PtySession::spawn(
        r#"powershell.exe -NoLogo -NoProfile -Command "[Console]::WriteLine('BKS_EXIT_READY'); for ($i=0; $i -lt 2048; $i++) { [Console]::WriteLine(('ROW{0:D5}:' -f $i) + ('x' * 64)) }; [Console]::WriteLine('BKS_EXIT_FINAL_TAIL'); exit 7""#,
        PtySize::default(),
    ).expect("spawn final-tail producer");
    let deadline = Instant::now() + Duration::from_secs(15);
    while session.exit_code().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(session.exit_code().unwrap(), Some(7));
    let request = Instant::now();
    session.request_output_finish().unwrap();
    assert!(request.elapsed() < Duration::from_millis(500));
    session
        .request_output_finish()
        .expect("finishing is idempotent");
    assert_eq!(
        session.resize(PtySize::new(90, 30)).unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while session.pending_output_bytes() == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(session.pending_output_bytes() > 0);
    assert!(
        !session.output_drained(),
        "process exit must not discard pending output"
    );
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !session.output_drained() && Instant::now() < deadline {
        let chunk = session.drain_output_limited(113);
        assert!(chunk.len() <= 113);
        output.extend(chunk);
        std::thread::yield_now();
    }
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("BKS_EXIT_FINAL_TAIL"), "final tail missing");
    assert!(output.len() > 64 * 1024);
    assert!(
        session.output_finished(),
        "reader did not publish EOF after process exit"
    );
    assert!(session.output_drained());
    assert_eq!(session.pending_output_bytes(), 0);
    assert!(session.output_error().is_none());
}

#[test]
fn exited_code_259_is_not_confused_with_a_running_process() {
    let session = PtySession::spawn("cmd.exe /d /c exit 259", PtySize::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.exit_code().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(session.exit_code().unwrap(), Some(259));
    assert!(!session.is_alive());
    session
        .request_terminate()
        .expect("terminate after exit is idempotent");
}

#[test]
fn explicit_termination_is_nonblocking_idempotent_and_owned() {
    let session = PtySession::spawn(
        r#"powershell.exe -NoLogo -NoProfile -Command "[Console]::WriteLine('BKS_TERMINATE_READY'); Start-Sleep -Seconds 30""#,
        PtySize::default(),
    ).unwrap();
    assert!(
        wait_for_marker(&session, "BKS_TERMINATE_READY", Duration::from_secs(15))
            .contains("BKS_TERMINATE_READY")
    );
    assert_eq!(session.exit_code().unwrap(), None);
    assert_eq!(
        session.request_output_finish().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let start = Instant::now();
    session.request_terminate().unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.exit_code().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(session.exit_code().unwrap(), Some(1));
    session.request_terminate().unwrap();
}

fn temp_working_directory() -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("bks cwd проверка {} {nonce}", std::process::id()));
    std::fs::create_dir(&path).expect("create temporary Unicode working directory");
    path
}

#[test]
fn explicit_working_directory_preserves_spaces_unicode_and_parent_directory() {
    let directory = temp_working_directory();
    let parent_directory = std::env::current_dir().unwrap();
    let session = PtySession::spawn_in(
        "cmd.exe /d /q /c cd",
        PtySize::new(512, 24),
        Some(&directory),
    )
    .expect("spawn cmd.exe in the selected directory");
    let expected = directory.to_str().unwrap();
    let output = wait_for_marker(&session, expected, Duration::from_secs(15));
    assert!(
        output.contains(expected),
        "selected directory was not passed to the child: {output:?}"
    );
    assert_eq!(std::env::current_dir().unwrap(), parent_directory);
    drop(session);
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn invalid_working_directories_fail_before_child_creation() {
    use std::io::ErrorKind;
    use std::path::Path;

    let directory = temp_working_directory();
    let file = directory.join("file.txt");
    std::fs::write(&file, b"fixture").unwrap();
    let missing = directory.join("missing");
    for (path, expected_kind) in [
        (Path::new("relative-directory"), ErrorKind::InvalidInput),
        (Path::new(r"C:relative-directory"), ErrorKind::InvalidInput),
        (Path::new("C:\\bks\0ignored"), ErrorKind::InvalidInput),
        (file.as_path(), ErrorKind::InvalidInput),
        (missing.as_path(), ErrorKind::NotFound),
    ] {
        // This executable does not exist: a native creation attempt would
        // produce an Other error, rather than this directory validation error.
        let error = PtySession::spawn_in(
            "bks-cwd-test-does-not-exist.exe",
            PtySize::default(),
            Some(path),
        )
        .err()
        .expect("invalid directory must fail before spawning");
        assert_eq!(error.kind(), expected_kind, "path={path:?}: {error}");
    }
    let nul_command =
        PtySession::spawn_in("cmd.exe\0 /c ignored", PtySize::default(), Some(&directory))
            .err()
            .expect("NUL command must fail before spawning");
    assert_eq!(nul_command.kind(), ErrorKind::InvalidInput);
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
