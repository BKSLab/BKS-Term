use bks_pty::{PtyInputWriter, PtySession, PtySize, OUTPUT_DRAIN_BUDGET};
use bks_terminal::{ReadingDocument, TerminalModel};
use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::codex_cli::{LaunchEvidence, SupportedCli};
use crate::command_blocks::CommandBlocks;
use crate::journal;
use crate::output_speech::OutputSpeech;
use crate::profile::Profile;
use crate::reading_state::ReadingState;
use crate::search::SearchState;
use crate::terminal_output::{TerminalEvent, TerminalOutput};

pub type SessionId = u64;
pub const MAX_RETIRING: usize = 16;

/// Keep the captured target, but avoid a verbatim argv[0] for ordinary paths:
/// .NET Framework configuration APIs reject that spelling after startup. Long
/// or special names retain their verbatim spelling and its exact semantics.
fn cli_launch_path(canonical: &Path) -> PathBuf {
    let Some(text) = canonical.to_str() else {
        return canonical.to_owned();
    };
    let spelling = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'\\'
        {
            return canonical.to_owned();
        }
        rest.to_owned()
    } else {
        return canonical.to_owned();
    };
    let components = if let Some(components) = spelling.strip_prefix(r"\\") {
        components
    } else {
        &spelling[3..]
    };
    if spelling.encode_utf16().count() >= 260
        || !components.split('\\').all(|component| {
            let device = component
                .split('.')
                .next()
                .unwrap_or_default()
                .trim_end_matches(' ')
                .to_ascii_uppercase();
            let reserved = matches!(
                device.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) || device
                .strip_prefix("COM")
                .or_else(|| device.strip_prefix("LPT"))
                .is_some_and(|number| {
                    matches!(
                        number,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                });
            !component.is_empty()
                && !reserved
                && component != "."
                && component != ".."
                && !component.ends_with(['.', ' '])
                && !component.chars().any(|ch| {
                    ch.is_control() || matches!(ch, ':' | '/' | '"' | '<' | '>' | '|' | '*' | '?')
                })
        })
    {
        return canonical.to_owned();
    }
    let spelling = PathBuf::from(spelling);
    // This one-time check is before spawn, never in the timer's evidence attach.
    // If DOS/UNC normalization changes the target, preserve the captured path.
    if std::fs::canonicalize(&spelling).ok().as_deref() == Some(canonical) {
        spelling
    } else {
        canonical.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionNotice {
    pub id: SessionId,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Starting,
    Running,
    Exited,
    Failed,
    Closing,
}

/// The source and native control to return to when closing a tab's search.
#[derive(Clone, Copy)]
pub struct SearchOrigin {
    pub control_id: i32,
    pub direct_input: bool,
    pub reading: bool,
    pub vt_reading: bool,
    pub focus_in_reader: bool,
}

pub struct TerminalSession {
    pub id: SessionId,
    pub profile_id: String,
    pub profile: Profile,
    pub title: String,
    pub state: SessionState,
    pub exit_code: Option<u32>,
    pub failure: Option<String>,
    notice: Option<SessionNotice>,
    notice_issued: bool,
    output_finishing: bool,
    buffer: String,
    journal_units: u32,
    journal_lines: usize,
    journal_trimmed: u32,
    pub draft: String,
    pub selection: (u32, u32),
    pub first_visible_offset: u32,
    pub direct_input: bool,
    pub reading: bool,
    /// A CLI launched explicitly from the ordinary command editor owns input
    /// until the integrated parent shell reports its next ready prompt.
    pub auto_cli_input: bool,
    auto_cli_prompt_seen: bool,
    prompt_capture: Option<Vec<u16>>,
    parent_prompt_key: Option<u64>,
    auto_cli_prompt_key: Option<u64>,
    pub vt_reading: bool,
    pub vt_state: ReadingState,
    pub reply_anchor: crate::tui_transcript::ReplyAnchor,
    pub search: SearchState,
    pub search_open: bool,
    pub search_focus: bool,
    pub search_wrapped: Option<bool>,
    pub search_origin: Option<SearchOrigin>,
    reading_document: Option<Arc<ReadingDocument>>,
    pub terminal_capture: bool,
    pub focus_in_reader: bool,
    shell_integration: bool,
    pub screen_selection: Option<(usize, usize)>,
    pub size: PtySize,
    pub terminal: TerminalModel,
    pub revision: u64,
    pub output_speech: OutputSpeech,
    pub tui_notice: Option<String>,
    pub tui_applied_context: Option<String>,
    /// Evidence checked once for the executable selected at session launch.
    /// Later UI polling must not probe the filesystem or a replacement CLI.
    pub verified_codex_version: Option<String>,
    codex_launch_evidence: Option<LaunchEvidence>,
    input_revision: Cell<u64>,
    pub speech_linear: bool,
    // Stop the input worker before closing the session's original pipe handles.
    input: PtyInputWriter,
    pty: PtySession,
    output: TerminalOutput,
    blocks: CommandBlocks,
}

impl TerminalSession {
    pub fn spawn(id: SessionId, profile: &Profile, size: PtySize) -> std::io::Result<Self> {
        let codex_launch_evidence = LaunchEvidence::capture(profile);
        let mut launch_profile = profile.clone();
        if let Some(evidence) = &codex_launch_evidence {
            // Spawn the captured canonical target, not a retargetable alias.
            launch_profile.executable = cli_launch_path(evidence.native_path())
                .to_string_lossy()
                .into_owned();
        }
        let command = launch_profile.command_line();
        let pty = PtySession::spawn_in(
            &command,
            size,
            profile
                .working_directory
                .as_deref()
                .map(std::path::Path::new),
        )?;
        let input = pty.input_writer()?;
        Ok(Self {
            id,
            profile_id: profile.id.clone(),
            profile: profile.clone(),
            title: profile.name.clone(),
            state: SessionState::Starting,
            exit_code: None,
            failure: None,
            notice: None,
            notice_issued: false,
            output_finishing: false,
            buffer: String::new(),
            journal_units: 0,
            journal_lines: 0,
            journal_trimmed: 0,
            draft: String::new(),
            selection: (0, 0),
            first_visible_offset: 0,
            direct_input: false,
            reading: true,
            auto_cli_input: false,
            auto_cli_prompt_seen: false,
            prompt_capture: None,
            parent_prompt_key: None,
            auto_cli_prompt_key: None,
            vt_reading: false,
            vt_state: ReadingState::default(),
            reply_anchor: crate::tui_transcript::ReplyAnchor::default(),
            search: SearchState::default(),
            search_open: false,
            search_focus: false,
            search_wrapped: None,
            search_origin: None,
            reading_document: None,
            terminal_capture: false,
            focus_in_reader: false,
            shell_integration: false,
            screen_selection: None,
            size,
            terminal: TerminalModel::new(size.cols as usize, size.rows as usize),
            revision: 0,
            output_speech: OutputSpeech::new(),
            tui_notice: None,
            tui_applied_context: None,
            verified_codex_version: None,
            codex_launch_evidence,
            input_revision: Cell::new(0),
            speech_linear: true,
            input,
            pty,
            output: TerminalOutput::new(),
            blocks: CommandBlocks::new(),
        })
    }

    /// Called once when discovery is ready, including for an existing session.
    /// Comparison is in memory; replacing the installed file never stops a PTY
    /// or clears evidence already attached to its running image.
    pub fn attach_codex_cli(&mut self, cli: &SupportedCli) {
        if self.verified_codex_version.is_none() {
            self.verified_codex_version = self
                .codex_launch_evidence
                .as_ref()
                .and_then(|evidence| cli.version_for_launch(&self.profile, evidence))
                .map(str::to_owned);
        }
    }

    #[cfg(test)]
    pub fn poll(&mut self) -> std::io::Result<Option<String>> {
        self.poll_limited(OUTPUT_DRAIN_BUDGET).map(|(text, _)| text)
    }

    fn poll_limited(&mut self, max_bytes: usize) -> std::io::Result<(Option<String>, usize)> {
        let bytes = self.pty.drain_output_limited(max_bytes);
        let used = bytes.len();
        let text = if bytes.is_empty() {
            None
        } else {
            self.ingest_output(&bytes)?
        };
        self.update_state();
        Ok((text, used))
    }

    fn ingest_output(&mut self, bytes: &[u8]) -> std::io::Result<Option<String>> {
        let vt_started = Instant::now();
        self.terminal.push_bytes(bytes);
        crate::trace_slow_phase("vt_ingest", vt_started, bytes.len());
        let journal_started = Instant::now();
        self.revision = self.revision.wrapping_add(1);
        self.screen_selection = None;
        let responses = self.terminal.drain_responses();
        let base = self.journal_units;
        let chunk = self.output.push_chunk(bytes);
        self.speech_linear = !chunk.screen_rewrite;
        let text = chunk.text;
        self.buffer.push_str(&text);
        self.journal_units += text.encode_utf16().count() as u32;
        self.journal_lines += text.bytes().filter(|byte| *byte == b'\n').count();
        // Marker offsets address this chunk, not the complete journal. Scan
        // UTF-16 once per marked chunk and retain only one bounded prompt.
        let prompt_units: Vec<_> = if !chunk.events.is_empty() || self.prompt_capture.is_some() {
            text.encode_utf16().collect()
        } else {
            Vec::new()
        };
        let mut prompt_segment_start = 0;
        for event in chunk.events {
            match event {
                TerminalEvent::PromptStart(position) => {
                    self.shell_integration = true;
                    self.prompt_capture = Some(Vec::new());
                    prompt_segment_start = position;
                    if self.auto_cli_input {
                        self.auto_cli_prompt_seen = false;
                    }
                    self.blocks.prompt_started(base + position as u32);
                }
                TerminalEvent::CommandStart(position) => {
                    self.shell_integration = true;
                    self.capture_prompt_segment(&prompt_units, prompt_segment_start, position);
                    let key = self
                        .prompt_capture
                        .take()
                        .filter(|prompt| !prompt.is_empty())
                        .map(|prompt| {
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            prompt.hash(&mut hasher);
                            hasher.finish()
                        });
                    prompt_segment_start = position;
                    if self.auto_cli_input {
                        self.auto_cli_prompt_seen =
                            key.is_some() && key == self.auto_cli_prompt_key;
                    } else {
                        self.parent_prompt_key = key;
                    }
                    self.blocks.command_started(base + position as u32);
                }
            }
        }
        self.capture_prompt_segment(&prompt_units, prompt_segment_start, prompt_units.len());
        self.blocks.text_updated(&self.buffer);
        self.trim_journal();
        crate::trace_slow_phase("journal_model", journal_started, self.buffer.len());
        // Journal/VT ingestion must finish even when a protocol reply cannot
        // be sent. A dead process does not need replies to its final output.
        if !responses.is_empty() {
            if let Err(error) = self.input.write(&responses) {
                if error.kind() != std::io::ErrorKind::WouldBlock && self.pty.is_alive() {
                    self.fail(format!("ошибка ввода: {error}"));
                }
            }
        }
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    pub fn write(&self, data: &[u8]) -> std::io::Result<()> {
        if !self.accepts_input() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "Session is not accepting input",
            ));
        }
        self.input.write(data)?;
        if !data.is_empty() {
            self.input_revision
                .set(self.input_revision.get().wrapping_add(1));
        }
        Ok(())
    }

    pub fn input_revision(&self) -> u64 {
        self.input_revision.get()
    }

    pub fn write_tui(&self, data: &[u8], expected_input: u64) -> std::io::Result<()> {
        if !self.accepts_input() || self.input_revision.get() != expected_input {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "CLI input state changed",
            ));
        }
        self.pty.write_when_idle(&self.input, data)?;
        if !data.is_empty() {
            self.input_revision
                .set(self.input_revision.get().wrapping_add(1));
        }
        Ok(())
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn set_startup_text(&mut self, text: &str) {
        self.buffer = text.to_owned();
        self.journal_units = text.encode_utf16().count() as u32;
        self.journal_lines = text.bytes().filter(|byte| *byte == b'\n').count();
        self.trim_journal();
    }

    pub fn append_notice(&mut self, text: &str) {
        self.buffer.push_str(text);
        self.journal_units += text.encode_utf16().count() as u32;
        self.journal_lines += text.bytes().filter(|byte| *byte == b'\n').count();
        self.trim_journal();
    }

    pub fn take_journal_trim(&mut self) -> u32 {
        std::mem::take(&mut self.journal_trimmed)
    }

    fn trim_journal(&mut self) {
        let removed = journal::trim(&mut self.buffer, self.journal_lines);
        if removed == 0 {
            return;
        }
        self.journal_units -= removed;
        self.journal_lines = self.buffer.bytes().filter(|byte| *byte == b'\n').count();
        self.journal_trimmed = self.journal_trimmed.saturating_add(removed);
        self.selection = (
            self.selection.0.saturating_sub(removed),
            self.selection.1.saturating_sub(removed),
        );
        self.first_visible_offset = self.first_visible_offset.saturating_sub(removed);
        self.blocks.trim_prefix(removed);
    }

    fn capture_prompt_segment(&mut self, units: &[u16], start: usize, end: usize) {
        let Some(mut capture) = self.prompt_capture.take() else {
            return;
        };
        let Some(segment) = units.get(start..end) else {
            return;
        };
        if capture.len().saturating_add(segment.len()) > 4_096 {
            return;
        }
        capture.extend_from_slice(segment);
        self.prompt_capture = Some(capture);
    }

    pub fn can_submit_command(&self) -> bool {
        self.accepts_input()
            && (!self.shell_integration || self.blocks.can_execute())
            && (!self.auto_cli_input || !self.shell_integration || self.auto_cli_prompt_seen)
    }

    pub fn cli_return_ready(&self) -> bool {
        self.auto_cli_input
            && self.auto_cli_prompt_seen
            && self.shell_integration
            && self.can_submit_command()
    }

    /// UI calls this only when returning to input; a reader may keep its
    /// snapshot and selection after the CLI has already returned to the shell.
    pub fn finish_cli_input(&mut self) -> bool {
        if !self.cli_return_ready() {
            return false;
        }
        self.cancel_cli_input();
        self.direct_input = false;
        self.reading = true;
        self.vt_reading = false;
        self.focus_in_reader = false;
        self.terminal_capture = false;
        true
    }

    pub fn cancel_cli_input(&mut self) {
        self.auto_cli_input = false;
        self.auto_cli_prompt_seen = false;
        self.auto_cli_prompt_key = None;
    }

    pub fn reading_document(&mut self) -> &ReadingDocument {
        let revision = self.terminal.reading_revision();
        if self
            .reading_document
            .as_ref()
            .is_none_or(|document| document.revision != revision)
        {
            self.reading_document = Some(Arc::new(self.terminal.reading_document()));
        }
        self.reading_document
            .as_ref()
            .expect("reading document cached")
    }

    pub fn reading_document_snapshot(&mut self) -> Arc<ReadingDocument> {
        self.reading_document();
        Arc::clone(
            self.reading_document
                .as_ref()
                .expect("reading document cached"),
        )
    }

    pub fn submit_command(&mut self, command: &str) -> std::io::Result<()> {
        if !self.can_submit_command() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Shell is not ready for a command",
            ));
        }
        let mut bytes = command.as_bytes().to_vec();
        bytes.extend_from_slice(b"\r\n");
        if let Err(error) = self.write(&bytes) {
            if error.kind() != std::io::ErrorKind::WouldBlock && self.pty.is_alive() {
                self.fail(format!("ошибка ввода: {error}"));
            }
            return Err(error);
        }
        if self.shell_integration {
            self.blocks.execute(command, &self.buffer);
        } else {
            self.blocks.execute_untracked(command, &self.buffer);
        }
        self.auto_cli_input = crate::cli_profiles::is_interactive_command(command);
        self.auto_cli_prompt_seen = false;
        self.auto_cli_prompt_key = self
            .auto_cli_input
            .then_some(self.parent_prompt_key)
            .flatten();
        Ok(())
    }

    pub fn resize(&mut self, size: PtySize) -> std::io::Result<()> {
        if self.size == size {
            return Ok(());
        }
        if self.pty.is_alive() && self.failure.is_none() {
            self.pty.resize(size)?;
        }
        self.terminal.resize(size.cols as usize, size.rows as usize);
        self.screen_selection = None;
        self.revision = self.revision.wrapping_add(1);
        self.size = size;
        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }

    pub fn tui_input_ready(&self) -> bool {
        self.accepts_input() && self.pty.pending_output_bytes() == 0 && self.input.is_idle()
    }

    pub fn blocks(&self) -> &CommandBlocks {
        &self.blocks
    }

    pub fn blocks_mut(&mut self) -> &mut CommandBlocks {
        &mut self.blocks
    }

    fn update_state(&mut self) {
        match self.pty.exit_code() {
            Ok(Some(code)) => {
                self.exit_code = Some(code);
                if self.failure.is_none() && self.state != SessionState::Exited {
                    self.state = SessionState::Closing;
                }
                if !self.output_finishing {
                    match self.pty.request_output_finish() {
                        Ok(()) => self.output_finishing = true,
                        Err(error) => self.fail(format!("ошибка завершения вывода: {error}")),
                    }
                }
            }
            Ok(None) => {
                if let Err(error) = self.input.check_error() {
                    self.fail(format!("ошибка ввода: {error}"));
                } else if self.state == SessionState::Starting {
                    self.state = SessionState::Running;
                }
            }
            Err(error) => self.fail(format!("ошибка состояния процесса: {error}")),
        }
        if let Some(error) = self.pty.output_error() {
            self.fail(format!("ошибка чтения: {error}"));
        }
        if let Some(code) = self.exit_code {
            if self.pty.output_drained() && self.failure.is_none() {
                self.state = SessionState::Exited;
                self.issue_notice(format!("{}: сессия завершена, код {code}", self.title));
            }
        }
    }

    fn accepts_input(&self) -> bool {
        self.failure.is_none()
            && matches!(self.state, SessionState::Starting | SessionState::Running)
            && self.pty.is_alive()
    }

    fn fail(&mut self, reason: String) {
        if self.failure.is_some() {
            return;
        }
        self.failure = Some(reason.clone());
        self.state = SessionState::Failed;
        self.issue_notice(format!("{}: ошибка сессии: {reason}", self.title));
        // A failed input/output channel cannot recover in-place. Stop only
        // this owned process, but keep polling its final output and status.
        let _ = self.pty.request_terminate();
    }

    fn issue_notice(&mut self, message: String) {
        if !self.notice_issued {
            self.notice_issued = true;
            self.notice = Some(SessionNotice {
                id: self.id,
                message,
            });
        }
    }
}

pub struct SessionManager {
    pub sessions: Vec<TerminalSession>,
    pub active_id: Option<SessionId>,
    next_id: SessionId,
    poll_cursor: usize,
    retiring: Vec<JoinHandle<()>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active_id: None,
            next_id: 1,
            poll_cursor: 0,
            retiring: Vec::new(),
        }
    }

    pub fn create(&mut self, profile: &Profile, size: PtySize) -> std::io::Result<SessionId> {
        self.reap_finished();
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let session = TerminalSession::spawn(id, profile, size)?;
        self.sessions.push(session);
        self.active_id = Some(id);
        Ok(id)
    }

    pub fn activate(&mut self, id: SessionId) -> bool {
        if self.sessions.iter().any(|session| session.id == id) {
            self.active_id = Some(id);
            true
        } else {
            false
        }
    }

    pub fn active(&self) -> Option<&TerminalSession> {
        self.active_id.and_then(|id| self.get(id))
    }

    pub fn active_mut(&mut self) -> Option<&mut TerminalSession> {
        let id = self.active_id?;
        self.get_mut(id)
    }

    pub fn get(&self, id: SessionId) -> Option<&TerminalSession> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut TerminalSession> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    pub fn poll(&mut self) -> Vec<(SessionId, String)> {
        self.reap_finished();
        let mut output = Vec::new();
        if self.sessions.is_empty() {
            return output;
        }
        let deadline = Instant::now() + Duration::from_millis(8);
        let mut remaining = 4 * OUTPUT_DRAIN_BUDGET;
        for _ in 0..self.sessions.len() {
            let index = self.poll_cursor % self.sessions.len();
            self.poll_cursor = (index + 1) % self.sessions.len();
            let session = &mut self.sessions[index];
            let limit = remaining.min(OUTPUT_DRAIN_BUDGET);
            match session.poll_limited(limit) {
                Ok((text, used)) => {
                    remaining -= used;
                    if let Some(text) = text {
                        output.push((session.id, text));
                    }
                }
                Err(error) => {
                    session.fail(error.to_string());
                    remaining -= limit;
                }
            }
            if remaining == 0 || Instant::now() >= deadline {
                break;
            }
        }
        output
    }

    pub fn close(&mut self, id: SessionId) -> bool {
        self.reap_finished();
        if self.retiring.len() >= MAX_RETIRING {
            return false;
        }
        let Some(position) = self.sessions.iter().position(|session| session.id == id) else {
            return false;
        };
        // Start the worker before moving the session. Spawn/send failure must
        // leave the live tab owned by the manager instead of dropping on UI.
        let (sender, receiver) = std::sync::mpsc::sync_channel::<TerminalSession>(1);
        let Ok(worker) = std::thread::Builder::new()
            .name("bks-session-retire".into())
            .spawn(move || {
                if let Ok(mut session) = receiver.recv() {
                    session.state = SessionState::Closing;
                    let _ = session.pty.request_terminate();
                    drop(session);
                }
            })
        else {
            return false;
        };
        let session = self.sessions.remove(position);
        if let Err(error) = sender.try_send(session) {
            let (std::sync::mpsc::TrySendError::Full(session)
            | std::sync::mpsc::TrySendError::Disconnected(session)) = error;
            self.sessions.insert(position, session);
            self.retiring.push(worker);
            return false;
        }
        self.retiring.push(worker);
        if self.active_id == Some(id) {
            self.active_id = if self.sessions.is_empty() {
                None
            } else {
                let next = position.min(self.sessions.len() - 1);
                Some(self.sessions[next].id)
            };
        }
        true
    }

    pub fn take_events(&mut self) -> Vec<SessionNotice> {
        self.sessions
            .iter_mut()
            .filter_map(|session| session.notice.take())
            .collect()
    }

    /// Bounded nonblocking retirement; false leaves remaining tabs for the
    /// final cleanup after the GUI message loop has ended.
    pub fn close_all(&mut self) -> bool {
        while let Some(id) = self.sessions.last().map(|session| session.id) {
            if !self.close(id) {
                return false;
            }
        }
        true
    }

    /// Call after leaving the GUI loop. This is the only cleanup entry point
    /// that waits for all remaining owned sessions and retirement workers.
    pub fn finish_cleanup(&mut self) {
        for session in &mut self.sessions {
            session.state = SessionState::Closing;
            let _ = session.pty.request_terminate();
        }
        self.sessions.clear();
        self.active_id = None;
        for worker in self.retiring.drain(..) {
            let _ = worker.join();
        }
    }

    fn reap_finished(&mut self) {
        let mut index = 0;
        while index < self.retiring.len() {
            if self.retiring[index].is_finished() {
                let worker = self.retiring.swap_remove(index);
                let _ = worker.join();
            } else {
                index += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileKind;
    use std::time::{Duration, Instant};

    #[test]
    fn tui_actions_are_blocked_by_in_flight_input_and_stopped_writer() {
        let mut profile = test_profile("TUI_IO_GUARD");
        profile.args = vec!["/d".into(), "/q".into()];
        let session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        session
            .input
            .write(&vec![b'x'; bks_pty::INPUT_QUEUE_MAX_BYTES])
            .unwrap();
        assert!(!session.input.is_idle());
        assert!(!session.tui_input_ready());
        session.input.cancel();
        assert!(!session.tui_input_ready());
        session.pty.request_terminate().unwrap();
    }

    #[test]
    fn accepted_user_input_invalidates_tui_before_any_output_is_polled() {
        let mut profile = test_profile("TUI_INPUT_GENERATION");
        profile.args = vec!["/d".into(), "/q".into()];
        let session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        let generation = session.input_revision();
        let screen_revision = session.revision;
        session.write(b"x").unwrap();
        assert_eq!(session.revision, screen_revision);
        assert_eq!(session.input_revision(), generation + 1);
        assert_eq!(
            session.write_tui(b"1\r", generation).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(session.input_revision(), generation + 1);
        session.pty.request_terminate().unwrap();
    }

    fn live_profile(marker: &str) -> Profile {
        let mut profile = test_profile(marker);
        profile.args = vec!["/d".into(), "/q".into()];
        profile
    }

    #[test]
    fn exit_seven_publishes_one_notice_after_large_final_tail_is_drained() {
        let mut profile = test_profile("EXIT_SEVEN");
        profile.executable = "powershell.exe".into();
        profile.args = vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(),
            "for ($i=0; $i -lt 2048; $i++) { [Console]::WriteLine(('ROW{0:D5}:' -f $i) + ('x' * 64)) }; [Console]::WriteLine('BKS_SESSION_FINAL_TAIL'); exit 7".into()];
        let mut manager = SessionManager::new();
        let id = manager.create(&profile, PtySize::default()).unwrap();
        assert_eq!(manager.get(id).unwrap().profile, profile);
        let mut received = String::new();
        let mut notices = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            for (session, text) in manager.poll() {
                assert_eq!(session, id);
                received.push_str(&text);
            }
            notices.extend(manager.take_events());
            if manager.get(id).unwrap().state == SessionState::Exited {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let session = manager.get(id).unwrap();
        assert_eq!(session.state, SessionState::Exited);
        assert_eq!(session.exit_code, Some(7));
        assert!(session.failure.is_none());
        assert!(session.pty.output_drained());
        assert!(received.len() >= 64 * 1024);
        assert!(received.contains("BKS_SESSION_FINAL_TAIL"));
        assert!(session.buffer().contains("BKS_SESSION_FINAL_TAIL"));
        assert!(!session.can_submit_command());
        assert!(session.write(b"late").is_err());
        assert_eq!(
            notices,
            vec![SessionNotice {
                id,
                message: format!("{}: сессия завершена, код 7", profile.name)
            }]
        );
        for _ in 0..3 {
            manager.poll();
            assert!(manager.take_events().is_empty());
        }
        manager.finish_cleanup();
    }

    #[test]
    fn writer_failure_keeps_journal_and_vt_tail_and_reports_once() {
        let mut manager = SessionManager::new();
        let id = manager
            .create(&live_profile("WRITER_FAILURE"), PtySize::default())
            .unwrap();
        let session = manager.get_mut(id).unwrap();
        session.input.cancel();
        let tail = session
            .ingest_output(b"BKS_WRITER_FINAL_TAIL\r\n\x1b[6n")
            .unwrap()
            .unwrap();
        assert!(tail.contains("BKS_WRITER_FINAL_TAIL"));
        assert!(session.buffer().contains("BKS_WRITER_FINAL_TAIL"));
        assert!(session
            .terminal
            .visible_text()
            .contains("BKS_WRITER_FINAL_TAIL"));
        assert_eq!(session.state, SessionState::Failed);
        let failure = session.failure.clone();
        assert!(failure.is_some());
        assert!(!session.can_submit_command());
        assert!(session.write(b"late").is_err());
        assert!(session
            .ingest_output(b"additional final tail")
            .unwrap()
            .is_some());
        let notices = manager.take_events();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].id, id);
        assert!(notices[0].message.contains("ошибка сессии"));
        for _ in 0..3 {
            manager.poll();
            assert!(manager.take_events().is_empty());
        }
        assert_eq!(manager.get(id).unwrap().failure, failure);
        assert_eq!(manager.get(id).unwrap().state, SessionState::Failed);
        manager.finish_cleanup();
    }

    #[test]
    fn protocol_reply_backpressure_keeps_the_live_session_and_its_output() {
        let mut session =
            TerminalSession::spawn(1, &live_profile("REPLY_BACKPRESSURE"), PtySize::default())
                .unwrap();
        session
            .input
            .write(&vec![b'x'; bks_pty::INPUT_QUEUE_MAX_BYTES])
            .unwrap();
        let tail = session
            .ingest_output(b"BKS_REPLY_TAIL\x1b[6n")
            .unwrap()
            .unwrap();
        assert!(tail.contains("BKS_REPLY_TAIL"));
        assert!(session.buffer().contains("BKS_REPLY_TAIL"));
        assert!(session.failure.is_none());
        assert_ne!(session.state, SessionState::Failed);
        assert!(session.pty.is_alive());
        session.pty.request_terminate().unwrap();
    }

    #[test]
    fn closing_live_sessions_is_quick_and_cleanup_joins_owned_retirements() {
        let unrelated = PtySession::spawn("cmd.exe /d /q", PtySize::default()).unwrap();
        let mut manager = SessionManager::new();
        let a = manager
            .create(&live_profile("RETIRE_A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&live_profile("RETIRE_B"), PtySize::default())
            .unwrap();
        let start = Instant::now();
        assert!(manager.close(b));
        assert!(start.elapsed() < Duration::from_millis(250));
        assert_eq!(manager.active_id, Some(a));
        assert!(manager.get(a).unwrap().pty.is_alive());
        assert!(unrelated.is_alive());
        let start = Instant::now();
        assert!(manager.close_all());
        assert!(start.elapsed() < Duration::from_millis(250));
        let (done, completed) = std::sync::mpsc::channel();
        let cleanup = std::thread::spawn(move || {
            manager.finish_cleanup();
            assert!(manager.sessions.is_empty());
            assert!(manager.retiring.is_empty());
            done.send(()).unwrap();
        });
        completed
            .recv_timeout(Duration::from_secs(5))
            .expect("retirement cleanup hung");
        cleanup.join().unwrap();
        assert!(unrelated.is_alive());
    }

    #[test]
    fn full_retirement_budget_keeps_the_requested_tab_alive() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut manager = SessionManager::new();
        let id = manager
            .create(&live_profile("RETIRE_LIMIT"), PtySize::default())
            .unwrap();
        let release = Arc::new(AtomicBool::new(false));
        for _ in 0..MAX_RETIRING {
            let release = Arc::clone(&release);
            manager.retiring.push(std::thread::spawn(move || {
                while !release.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }));
        }
        let start = Instant::now();
        let removed = manager.close(id);
        let retained = manager
            .get(id)
            .is_some_and(|session| session.pty.is_alive());
        let elapsed = start.elapsed();
        release.store(true, Ordering::Release);
        manager.finish_cleanup();
        assert!(!removed);
        assert!(retained);
        assert!(elapsed < Duration::from_millis(250));
        assert!(manager.retiring.is_empty());
    }

    fn test_profile(marker: &str) -> Profile {
        Profile {
            id: format!("test-{marker}"),
            name: format!("test-{marker}"),
            kind: ProfileKind::Local,
            executable: "cmd.exe".into(),
            args: vec!["/c".into(), format!("echo BKS_SESSION_{marker}")],
            working_directory: None,
        }
    }

    #[test]
    fn journal_retention_rebases_reader_and_command_blocks_without_touching_draft() {
        let mut session =
            TerminalSession::spawn(1, &test_profile("RETENTION"), PtySize::default()).unwrap();
        let prefix = "old\r\n".repeat(journal::JOURNAL_MAX_LINES - 5);
        let marker = "KEEP_🙂";
        let initial = format!("{prefix}{marker}\r\ntail\r\n");
        session.set_startup_text(&initial);
        let start = prefix.encode_utf16().count() as u32;
        session.selection = (start, start + marker.encode_utf16().count() as u32);
        session.first_visible_offset = start;
        session.draft = "unsent draft".into();
        session.blocks.execute_untracked("cmd", &prefix);
        session
            .ingest_output("more\r\n".repeat(12).as_bytes())
            .unwrap();
        let removed = session.take_journal_trim();
        assert!(removed > 0);
        assert!(session.buffer.len() <= journal::JOURNAL_MAX_BYTES);
        assert!(session.journal_lines <= journal::JOURNAL_MAX_LINES);
        assert_eq!(session.selection.0, start - removed);
        assert_eq!(session.first_visible_offset, start - removed);
        let text16: Vec<_> = session.buffer.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&text16[session.selection.0 as usize..session.selection.1 as usize])
                .unwrap(),
            marker
        );
        assert_eq!(session.blocks.last(), Some(start - removed));
        assert_eq!(session.journal_units as usize, text16.len());
        assert_eq!(session.draft, "unsent draft");
        assert_eq!(session.take_journal_trim(), 0);
    }

    #[test]
    fn retention_clamps_deleted_reader_selection_instead_of_following_output() {
        let mut session =
            TerminalSession::spawn(1, &test_profile("CLAMP"), PtySize::default()).unwrap();
        session.set_startup_text(&"old\r\n".repeat(journal::JOURNAL_MAX_LINES));
        session.selection = (0, 3);
        session.first_visible_offset = 0;
        session.append_notice("new\r\n");
        assert!(session.take_journal_trim() > 0);
        assert_eq!(session.selection, (0, 0));
        assert_eq!(session.first_visible_offset, 0);
        assert!(session.buffer.ends_with("new\r\n"));
    }

    fn wait_for_markers(
        manager: &mut SessionManager,
        ids: &[SessionId],
        markers: &[&str],
    ) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buffers = vec![String::new(); ids.len()];
        while Instant::now() < deadline {
            for (session_id, text) in manager.poll() {
                if let Some(index) = ids.iter().position(|id| *id == session_id) {
                    buffers[index].push_str(&text);
                }
            }
            if markers
                .iter()
                .enumerate()
                .all(|(index, marker)| buffers[index].contains(marker))
            {
                return buffers;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        buffers
    }

    #[test]
    fn poll_collects_marker() {
        let mut manager = SessionManager::new();
        let id = manager
            .create(&test_profile("ONE"), PtySize::default())
            .unwrap();
        let buffers = wait_for_markers(&mut manager, &[id], &["BKS_SESSION_ONE"]);
        assert!(buffers[0].contains("BKS_SESSION_ONE"));
    }

    #[test]
    fn new_sessions_start_with_native_command_and_reading_controls() {
        let session =
            TerminalSession::spawn(1, &test_profile("ACCESSIBLE"), PtySize::default()).unwrap();
        assert!(!session.direct_input);
        assert!(session.reading);
        assert!(!session.terminal_capture);
        assert!(!session.focus_in_reader);
    }

    #[test]
    fn captured_cli_launch_runs_managed_configuration_and_serializer_in_unicode_cwd() {
        use crate::owned_child::OwnedChild;
        use std::fs;
        use std::process::{Command, Stdio};

        struct FixtureDirectory(PathBuf);
        impl Drop for FixtureDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = FixtureDirectory(
            std::env::temp_dir().join(format!("bks-managed-cli-{}-{unique} Ж", std::process::id())),
        );
        fs::create_dir(&directory.0).unwrap();
        let cwd = directory.0.join("working folder Ж");
        fs::create_dir(&cwd).unwrap();
        let source = directory.0.join("fixture.cs");
        let executable = directory.0.join("codex.exe");
        fs::write(
            &source,
            r#"using System;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;
internal static class ManagedCliFixture {
    public static int Main() {
        string json = new JavaScriptSerializer().Serialize(new {
            Marker = "BKS_MANAGED_CONFIG_READY", Cwd = Environment.CurrentDirectory
        });
        File.WriteAllText("managed-config.json", json, new UTF8Encoding(false));
        Console.WriteLine("BKS_MANAGED_CONFIG_READY");
        return 0;
    }
}
"#,
        )
        .unwrap();
        let windows = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        let framework = windows.join("Microsoft.NET/Framework64/v4.0.30319");
        let mut command = Command::new(framework.join("csc.exe"));
        command
            .args(["/nologo", "/noconfig", "/target:exe"])
            .arg(format!("/out:{}", executable.display()))
            .arg(format!(
                "/reference:{}",
                framework.join("System.Web.Extensions.dll").display()
            ))
            .arg(&source)
            .current_dir(&directory.0)
            .env_clear()
            .env("SystemRoot", &windows)
            .env("WINDIR", &windows)
            .env("HOME", &directory.0)
            .env("USERPROFILE", &directory.0)
            .env("TEMP", &directory.0)
            .env("TMP", &directory.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut compiler = OwnedChild::spawn(&mut command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = compiler.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "Managed fixture compiler timeout"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            status.success(),
            "Managed fixture compiler failed: {status}"
        );
        drop(compiler);
        let profile = Profile {
            id: "cli-codex".to_owned(),
            name: "Managed configuration fixture".to_owned(),
            kind: ProfileKind::Local,
            executable: executable.to_string_lossy().into_owned(),
            args: Vec::new(),
            working_directory: Some(cwd.to_string_lossy().into_owned()),
        };
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        let evidence = session.codex_launch_evidence.as_ref().unwrap();
        assert_eq!(
            evidence.native_path(),
            fs::canonicalize(&executable).unwrap()
        );
        assert!(session.verified_codex_version.is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.exit_code.is_none() && Instant::now() < deadline {
            session.poll().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(session.exit_code, Some(0), "{}", session.buffer());
        let marker: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(cwd.join("managed-config.json")).unwrap())
                .unwrap();
        assert_eq!(marker["Marker"], "BKS_MANAGED_CONFIG_READY");
        assert_eq!(marker["Cwd"], cwd.to_string_lossy().as_ref());
    }

    #[test]
    fn command_editor_works_without_osc_and_keeps_integrated_prompt_gating() {
        let mut profile = test_profile("NO_OSC");
        profile.args = vec!["/Q".into()];
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        assert!(session.can_submit_command());
        session.ingest_output(b"\x1b]133;B\x07").unwrap();
        assert!(session.can_submit_command());
        assert!(session.blocks.execute("echo test", ""));
        assert!(!session.can_submit_command());
        session
            .ingest_output(b"\x1b]133;A\x07\x1b]133;B\x07")
            .unwrap();
        assert!(session.can_submit_command());
    }

    #[test]
    fn accepted_cli_submission_arms_one_handoff_without_guessing_an_untracked_exit() {
        // more.com consumes literal stdin, so test input can never launch a
        // real CLI or be executed by a command shell.
        let mut profile = test_profile("CLI_HANDOFF_LITERAL");
        profile.executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/more.com")
            .to_string_lossy()
            .into_owned();
        profile.args.clear();
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        let before = session.input_revision();
        session.submit_command("kimi").unwrap();
        assert!(session.auto_cli_input);
        assert_eq!(session.input_revision(), before + 1);
        assert!(!session.cli_return_ready());
        assert!(!session.finish_cli_input());
        session.submit_command("ordinary literal text").unwrap();
        assert!(!session.auto_cli_input);
        assert_eq!(session.input_revision(), before + 2);
    }

    #[test]
    fn cli_handoff_waits_for_a_fresh_parent_prompt_and_keeps_reader_state() {
        let mut profile = test_profile("CLI_HANDOFF_PROMPT");
        profile.args = vec!["/D".into(), "/Q".into()];
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        session
            .ingest_output(b"\x1b]133;A\x07PS> \x1b]133;B\x07")
            .unwrap();
        assert!(session
            .blocks
            .execute("kimi", session.buffer().to_owned().as_str()));
        session.auto_cli_input = true;
        session.auto_cli_prompt_key = session.parent_prompt_key;
        session.direct_input = true;
        session.reading = false;
        session.vt_reading = true;
        session.terminal_capture = true;
        assert!(!session.cli_return_ready());
        session
            .ingest_output(b"FAKE CLI READY\x1b]133;B\x07")
            .unwrap();
        assert!(
            !session.cli_return_ready(),
            "a command marker alone is not a returned parent prompt"
        );
        session
            .ingest_output(b"\x1b]133;A\x07OTHER CLI> \x1b]133;B\x07")
            .unwrap();
        assert!(
            !session.cli_return_ready(),
            "a TUI's own prompt is not the parent shell prompt"
        );
        assert!(!session.can_submit_command());
        session.reading = true;
        session.focus_in_reader = true;
        session.draft = "unsent draft".into();
        let document = session.reading_document_snapshot();
        session.vt_state.capture(&document, (1, 3), 0, false);
        let input_generation = session.input_revision();
        session
            .ingest_output(b"\r\n\x1b]133;A\x07PS> \x1b]133;B\x07")
            .unwrap();
        assert!(session.cli_return_ready());
        session
            .ingest_output(b"\x1b]133;A\x07OTHER CLI> \x1b]133;B\x07")
            .unwrap();
        assert!(!session.cli_return_ready() && !session.can_submit_command());
        session.ingest_output(b"\x1b]133;B\x07").unwrap();
        assert!(!session.cli_return_ready() && !session.can_submit_command());
        session.ingest_output(b"\x1b]133;A\x07PS").unwrap();
        assert!(!session.cli_return_ready());
        session.ingest_output(b"> \x1b]133;B\x07").unwrap();
        assert!(session.cli_return_ready());
        assert!(session.direct_input && session.reading && session.focus_in_reader);
        let document = session.reading_document_snapshot();
        assert_eq!(session.vt_state.restore(&document).selection, (1, 3));
        assert!(session.finish_cli_input());
        assert!(!session.auto_cli_input && !session.direct_input && !session.terminal_capture);
        assert!(session.reading && !session.focus_in_reader && !session.vt_reading);
        assert_eq!(session.draft, "unsent draft");
        assert_eq!(
            session.input_revision(),
            input_generation,
            "restoring the editor never sends input"
        );
    }

    #[test]
    fn prompt_capture_stays_bounded_with_a_long_journal_and_many_markers() {
        let mut profile = test_profile("PROMPT_FLOOD");
        profile.args = vec!["/D".into(), "/Q".into()];
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        session.ingest_output(&vec![b'Z'; 400_000]).unwrap();
        let markers = b"\x1b]133;A\x07PS> \x1b]133;B\x07".repeat(2_000);
        session.ingest_output(&markers).unwrap();
        assert!(session.parent_prompt_key.is_some());
        assert!(session.prompt_capture.is_none());
        session.ingest_output(b"\x1b]133;A\x07").unwrap();
        session.ingest_output(&vec![b'x'; 5_000]).unwrap();
        assert!(session.prompt_capture.is_none());
        session.ingest_output(b"\x1b]133;B\x07").unwrap();
        assert!(session.prompt_capture.is_none());
    }

    #[test]
    fn submitting_to_cmd_creates_navigable_blocks_without_prompt_markers() {
        let mut profile = test_profile("CMD_BLOCKS");
        profile.args = vec!["/Q".into()];
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        let mut anchors = Vec::new();
        for marker in ["BKS_BLOCK_ONE", "BKS_BLOCK_TWO", "BKS_BLOCK_Ж"] {
            // CMD removes ^ when executing, so the marker cannot match input echo.
            let last = marker.char_indices().next_back().unwrap().0;
            session
                .submit_command(&format!("echo {}^{}", &marker[..last], &marker[last..]))
                .unwrap();
            assert!(session.blocks.navigate(true, None).is_some());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !session.buffer.contains(marker) && Instant::now() < deadline {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(session.buffer.contains(marker), "missing output {marker}");
            anchors.push(session.blocks.last().unwrap());
        }
        assert!(!session.shell_integration);
        assert!(anchors.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            session.blocks.navigate(true, Some(anchors[2])),
            Some(anchors[1])
        );
        assert_eq!(
            session.blocks.navigate(false, Some(anchors[0])),
            Some(anchors[1])
        );
    }

    #[test]
    fn failed_submission_does_not_create_a_phantom_block() {
        let mut profile = test_profile("REJECTED_BLOCK");
        profile.args = vec!["/Q".into()];
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        let command = "x".repeat(bks_pty::INPUT_QUEUE_MAX_BYTES);
        assert_eq!(
            session.submit_command(&command).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(session.blocks.last(), None);
    }

    #[test]
    fn vt_reading_document_updates_separately_from_the_command_journal() {
        let mut session =
            TerminalSession::spawn(1, &test_profile("READ_DOC"), PtySize::default()).unwrap();
        session.ingest_output(b"old\r\x1b[2Knew").unwrap();
        assert_eq!(session.reading_document().text, "new");
        assert!(session.buffer.contains("old"));
        let revision = session.reading_document().revision;
        session.ingest_output(b"\x1b[?1049h\x1b[Halt").unwrap();
        assert!(session.reading_document().alternate_screen);
        assert_eq!(session.reading_document().text, "alt");
        assert_ne!(session.reading_document().revision, revision);
        session.ingest_output(b"\x1b[?1049l").unwrap();
        assert_eq!(session.reading_document().text, "new");
        assert!(!session.vt_reading);
        assert!(session.reading);
    }

    #[test]
    fn background_session_updates_keep_separate_reader_bookmarks() {
        let mut sessions = SessionManager::new();
        let a = sessions
            .create(&test_profile("READ_A"), PtySize::default())
            .unwrap();
        let b = sessions
            .create(&test_profile("READ_B"), PtySize::default())
            .unwrap();
        for (id, text, needle) in [
            (a, "first\r\nchosenA\r\nlast", "chosenA"),
            (b, "different prefix\r\nchosenB\r\nlast", "chosenB"),
        ] {
            let session = sessions.get_mut(id).unwrap();
            session.ingest_output(text.as_bytes()).unwrap();
            let document = session.reading_document_snapshot();
            let byte = document.text.find(needle).unwrap();
            let start = document.text[..byte].encode_utf16().count() as u32;
            session
                .vt_state
                .capture(&document, (start, start + needle.len() as u32), 0, false);
        }
        assert!(sessions.activate(b));
        sessions
            .get_mut(a)
            .unwrap()
            .ingest_output(b"\r\nbackground output")
            .unwrap();
        for (id, expected) in [(b, "chosenB"), (a, "chosenA")] {
            assert!(sessions.activate(id));
            let session = sessions.active_mut().unwrap();
            let document = session.reading_document_snapshot();
            let restored = session.vt_state.restore(&document);
            let units: Vec<u16> = document.text.encode_utf16().collect();
            assert_eq!(
                String::from_utf16(
                    &units[restored.selection.0 as usize..restored.selection.1 as usize]
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn two_sessions_do_not_mix_buffers() {
        let mut manager = SessionManager::new();
        let a = manager
            .create(&test_profile("A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&test_profile("B"), PtySize::default())
            .unwrap();
        let buffers = wait_for_markers(&mut manager, &[a, b], &["BKS_SESSION_A", "BKS_SESSION_B"]);
        assert!(buffers[0].contains("BKS_SESSION_A"));
        assert!(!buffers[0].contains("BKS_SESSION_B"));
        assert!(buffers[1].contains("BKS_SESSION_B"));
        assert!(!buffers[1].contains("BKS_SESSION_A"));
        let a_screen = manager.get(a).unwrap().terminal.visible_text();
        let b_screen = manager.get(b).unwrap().terminal.visible_text();
        assert!(a_screen.contains("BKS_SESSION_A"));
        assert!(!a_screen.contains("BKS_SESSION_B"));
        assert!(b_screen.contains("BKS_SESSION_B"));
        assert!(!b_screen.contains("BKS_SESSION_A"));
    }

    #[test]
    fn activate_preserves_buffer() {
        let mut manager = SessionManager::new();
        let a = manager
            .create(&test_profile("ACT_A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&test_profile("ACT_B"), PtySize::default())
            .unwrap();
        let _ = wait_for_markers(
            &mut manager,
            &[a, b],
            &["BKS_SESSION_ACT_A", "BKS_SESSION_ACT_B"],
        );
        let a_buffer = manager.get(a).unwrap().buffer.clone();
        assert!(manager.activate(a));
        assert_eq!(manager.active_id, Some(a));
        assert_eq!(manager.get(a).unwrap().buffer, a_buffer);
        assert!(manager.activate(b));
        assert_eq!(manager.active_id, Some(b));
    }

    #[test]
    fn close_selects_another_session() {
        let mut manager = SessionManager::new();
        let a = manager
            .create(&test_profile("CLOSE_A"), PtySize::default())
            .unwrap();
        let b = manager
            .create(&test_profile("CLOSE_B"), PtySize::default())
            .unwrap();
        assert_eq!(manager.active_id, Some(b));
        assert!(manager.close(b));
        assert_eq!(manager.active_id, Some(a));
        assert!(manager.get(a).is_some());
        assert!(manager.get(b).is_none());
        assert!(manager.close(a));
        assert_eq!(manager.active_id, None);
    }

    #[test]
    fn vt_and_reading_journal_receive_the_same_raw_stream() {
        let mut session =
            TerminalSession::spawn(1, &test_profile("RAW"), PtySize::default()).unwrap();
        let bytes = "\u{1b}]133;A\u{7}PS> \u{1b}]133;B\u{7}\r\n\u{1b}[31mПривет\u{1b}[0m";
        session.ingest_output(bytes.as_bytes()).unwrap();
        assert!(session.buffer.contains("Привет"));
        assert!(!session.buffer.contains('\u{1b}'));
        assert!(session.terminal.visible_text().contains("Привет"));
        assert!(session.blocks.can_execute());
        let revision = session.revision;
        // An erase produces no journal text, but still needs a screen repaint.
        assert_eq!(session.ingest_output(b"\x1b[2J").unwrap(), None);
        assert_ne!(session.revision, revision);
        assert!(session.terminal.visible_text().trim().is_empty());
        assert!(session.buffer.contains("Привет"));
    }

    #[test]
    fn resize_updates_conpty_and_vt_together_and_preserves_view_state() {
        let mut session =
            TerminalSession::spawn(1, &test_profile("SIZE"), PtySize::default()).unwrap();
        session.reading = true;
        session.draft = "unfinished".into();
        session.selection = (3, 7);
        session.resize(PtySize::new(100, 30)).unwrap();
        assert_eq!(session.terminal.size(), (100, 30));
        assert_eq!(session.size, PtySize::new(100, 30));
        assert!(session.reading);
        assert_eq!(session.draft, "unfinished");
        assert_eq!(session.selection, (3, 7));
        let revision = session.revision;
        session.resize(PtySize::new(100, 30)).unwrap();
        assert_eq!(session.revision, revision);
    }
}
