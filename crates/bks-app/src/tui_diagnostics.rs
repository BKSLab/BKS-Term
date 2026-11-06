//! Explicit opt-in, bounded metadata trace. Disk IO never runs on the UI thread.
use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};

use crate::direct_accessibility::FeedbackDiagnostic;

const MAX_BYTES: u64 = 1_048_576;

fn sender() -> Option<&'static mpsc::SyncSender<Vec<FeedbackDiagnostic>>> {
    static SENDER: OnceLock<Option<mpsc::SyncSender<Vec<FeedbackDiagnostic>>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let path = PathBuf::from(std::env::var_os("BKS_TERM_TUI_TRACE")?.into_string().ok()?);
            if path.as_os_str().is_empty() {
                return None;
            }
            let (sender, receiver) = mpsc::sync_channel::<Vec<FeedbackDiagnostic>>(8);
            std::thread::Builder::new()
                .name("tui-metadata-trace".into())
                .spawn(move || {
                    use std::io::Write;
                    let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    else {
                        return;
                    };
                    let mut written = file.metadata().map_or(MAX_BYTES, |metadata| metadata.len());
                    while let Ok(events) = receiver.recv() {
                        for event in events {
                            let Ok(mut line) = serde_json::to_vec(&event) else {
                                continue;
                            };
                            line.push(b'\n');
                            if written.saturating_add(line.len() as u64) > MAX_BYTES {
                                return;
                            }
                            if file.write_all(&line).is_err() {
                                return;
                            }
                            written += line.len() as u64;
                        }
                    }
                })
                .ok()?;
            Some(sender)
        })
        .as_ref()
}

pub fn enabled() -> bool {
    sender().is_some()
}

pub fn record(events: Vec<FeedbackDiagnostic>) {
    if !events.is_empty() {
        if let Some(sender) = sender() {
            let _ = sender.try_send(events);
        }
    }
}
