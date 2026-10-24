use bks_pty::{PtySession, PtySize};
use std::time::Duration;

fn main() {
    let cmd = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "cmd.exe /c echo hello-from-conpty".into());
    println!("spawning: {cmd}");
    let s = PtySession::spawn(&cmd, PtySize::default()).unwrap();
    for i in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        let out = s.drain_output();
        if !out.is_empty() {
            println!(
                "[{i}] {} bytes: {:?}",
                out.len(),
                String::from_utf8_lossy(&out)
            );
        }
        if !s.is_alive() && i > 2 {
            break;
        }
    }
}
