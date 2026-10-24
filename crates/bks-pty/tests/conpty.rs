//! Интеграционные тесты: реальные ConPTY-сеансы cmd.exe и PowerShell.

use std::time::{Duration, Instant};

use bks_pty::{PtySession, PtySize};

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
