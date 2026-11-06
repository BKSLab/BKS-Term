use bks_pty::{PtyInputWriter, PtySession, PtySize, OUTPUT_DRAIN_BUDGET};
use bks_terminal::{ReadingDocument, StyleSpan, TerminalModel, TextStyle};
use std::cell::Cell;
use std::hash::{BuildHasher, Hash, Hasher};
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
const CLR_FAILFAST_EXIT: u32 = 0x8013_1623;
const CLI_RETURN_RECOVERY_AGE: Duration = Duration::from_secs(5);
const CHANNEL_EXIT_GRACE: Duration = Duration::from_secs(5);
const COMPOSER_CONTINUITY_AGE: Duration = Duration::from_millis(750);
const COMPOSER_RESIZE_AGE: Duration = Duration::from_secs(2);

struct ResizedComposer {
    input: crate::tui_projection::Composer,
    size: PtySize,
    started: Instant,
    after_revision: u64,
}

struct CliExitWatch {
    parent_prompt: u64,
    returned_at: Option<Instant>,
    shell_command_queued: bool,
}

impl CliExitWatch {
    fn fresh(&self, now: Instant) -> bool {
        self.returned_at.is_none_or(|returned| {
            now.saturating_duration_since(returned) <= CLI_RETURN_RECOVERY_AGE
        })
    }
}

struct PendingChannelClose {
    started: Instant,
    reason: String,
}

fn recoverable_shell_profile(profile: &Profile) -> bool {
    standard_powershell_prompt(profile)
        && Path::new(&profile.executable)
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("pwsh"))
}

fn standard_powershell_prompt(profile: &Profile) -> bool {
    profile.kind == crate::profile::ProfileKind::Local
        && !crate::cli_profiles::starts_in_direct_mode(profile)
        && profile.args
            == [
                "-NoLogo",
                "-NoProfile",
                "-NoExit",
                "-Command",
                crate::profile::PS_PROMPT,
            ]
        && Path::new(&profile.executable)
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("pwsh") || name.eq_ignore_ascii_case("powershell")
            })
}

fn closed_pipe(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(109 | 232 | 233))
}

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
    pub answers_view: bool,
}

/// Only bytes accepted from the user's direct input can establish a command.
/// Output redraws never contribute text or repair an unknown edit.
#[derive(Default)]
struct DirectCommandInput {
    parent_key: Option<u64>,
    capture: Option<Vec<u8>>,
    waiting_for_prompt: bool,
}

impl DirectCommandInput {
    fn prompt_started(&mut self) {
        self.capture = None;
    }

    fn prompt_ready(&mut self, key: Option<u64>, new_source: bool) {
        if let Some(key) = key {
            if new_source || self.parent_key.is_none() || self.parent_key == Some(key) {
                self.parent_key = Some(key);
                self.capture = Some(Vec::new());
                self.waiting_for_prompt = false;
                return;
            }
        }
        self.capture = None;
    }

    fn resize(&mut self) {
        if self.capture.as_ref().is_some_and(|text| !text.is_empty()) {
            self.capture = None;
        }
    }

    /// The first Enter consumes this prompt even when its command is unknown.
    /// Later bytes cannot describe a fresh shell command without a new prompt.
    fn push(&mut self, data: &[u8]) -> Option<Option<String>> {
        if self.waiting_for_prompt {
            return None;
        }
        for (index, &byte) in data.iter().enumerate() {
            if byte == b'\r' || byte == b'\n' {
                let terminal_enter = byte == b'\r' && matches!(&data[index + 1..], b"" | b"\n");
                let command = self.capture.take().and_then(|text| {
                    terminal_enter
                        .then(|| String::from_utf8(text).ok())
                        .flatten()
                        .filter(|text| !text.chars().any(char::is_control))
                });
                return Some(command);
            }
            let Some(text) = self.capture.as_mut() else {
                continue;
            };
            match byte {
                // terminal_input encodes Ctrl+Backspace as 0x08; its word
                // deletion cannot be reconstructed as one trailing character.
                0x7f if text.is_ascii() => {
                    text.pop();
                }
                0x20..=0x7e | 0x80..=0xff if text.len() < 4_096 => text.push(byte),
                _ => self.capture = None,
            }
        }
        if let Some(text) = &self.capture {
            match std::str::from_utf8(text) {
                Ok(text) if text.chars().any(char::is_control) => self.capture = None,
                Err(error) if error.error_len().is_some() => self.capture = None,
                _ => {}
            }
        }
        None
    }
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
    shell_recovery_pending: bool,
    shell_recovery_bootstrap: bool,
    cli_exit_watch: Option<CliExitWatch>,
    pending_channel_close: Option<PendingChannelClose>,
    pub adapter: crate::tui_adapters::AdapterKind,
    configured_cli: bool,
    configured_followup: Option<Profile>,
    buffer: String,
    journal_styles: Vec<StyleSpan>,
    journal_units: u32,
    journal_lines: usize,
    journal_trimmed: u32,
    journal_reset: bool,
    journal_clear_layout_pending: bool,
    pub draft: String,
    pub draft_selection: (u32, u32),
    pub selection: (u32, u32),
    pub first_visible_offset: u32,
    pub direct_input: bool,
    pub reading: bool,
    /// A CLI launched explicitly from a known parent shell prompt owns input
    /// until the integrated parent shell reports its next ready prompt.
    pub auto_cli_input: bool,
    auto_cli_prompt_seen: bool,
    prompt_capture: Option<Vec<u16>>,
    owned_prompt: Option<u64>,
    parent_prompt_key: Option<u64>,
    auto_cli_prompt_key: Option<u64>,
    direct_command_input: DirectCommandInput,
    pub vt_reading: bool,
    pub vt_state: ReadingState,
    pub answers_view: bool,
    pub answer_cursor: Option<u64>,
    pub answer_state: ReadingState,
    pub turns: crate::tui_turns::TurnLog,
    pub turn_speech: crate::turn_speech::TurnSpeech,
    pub tui_submission: crate::tui_submission::Submission,
    pub source_input: crate::source_input::SourceInput,
    pub reply_anchor: crate::tui_transcript::ReplyAnchor,
    latest_output: crate::tui_transcript::LatestOutput,
    pub search: SearchState,
    pub search_open: bool,
    pub search_focus: bool,
    pub search_wrapped: Option<bool>,
    pub search_origin: Option<SearchOrigin>,
    reading_document: Option<Arc<ReadingDocument>>,
    reading_document_key: Option<(u64, bool)>,
    reading_document_epoch: u64,
    pub tui_composer: Option<crate::tui_projection::Composer>,
    // Private proof across a short hidden-cursor transport split. It never
    // enters presentation, menu detection or answer classification directly.
    composer_continuity: Option<(crate::tui_projection::Composer, Instant)>,
    resized_composer: Option<ResizedComposer>,
    pub tui_menu: Option<crate::tui_suggestions::Menu>,
    pub menu_observer: crate::tui_suggestions::MenuObserver,
    tui_history: crate::tui_projection::ComposerHistory,
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
        Self::spawn_with_env(id, profile, size, &profile.launch.environment)
    }

    fn spawn_with_env(
        id: SessionId,
        profile: &Profile,
        size: PtySize,
        environment: &std::collections::BTreeMap<String, String>,
    ) -> std::io::Result<Self> {
        let codex_launch_evidence = LaunchEvidence::capture(profile);
        let mut launch_profile = profile.clone();
        let owned_prompt = standard_powershell_prompt(profile)
            .then(|| std::collections::hash_map::RandomState::new().hash_one(id));
        if let Some(token) = owned_prompt {
            // Process-local metadata survives ConPTY cursor rewrites and soft
            // wrapping. Keep the persisted profile and custom prompts intact.
            launch_profile.args[4] = crate::profile::PS_PROMPT
                .replace("]133;A", &format!("]133;A;BKS={token:016x}"))
                .replace("]133;B", &format!("]133;B;BKS={token:016x}"));
        }
        if let Some(evidence) = &codex_launch_evidence {
            // Spawn the captured canonical target, not a retargetable alias.
            launch_profile.executable = cli_launch_path(evidence.native_path())
                .to_string_lossy()
                .into_owned();
        }
        let command = launch_profile.command_line();
        let pty = PtySession::spawn_in_with_env(
            &command,
            size,
            profile
                .working_directory
                .as_deref()
                .map(std::path::Path::new),
            environment,
        )?;
        let input = pty.input_writer()?;
        let mut terminal = TerminalModel::new(size.cols as usize, size.rows as usize);
        crate::ansi_palette::configure_terminal(&mut terminal, &crate::theme());
        let adapter = crate::cli_profiles::adapter_for_profile(profile);
        let mut turns = crate::tui_turns::TurnLog::new();
        turns.set_adapter(adapter);
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
            shell_recovery_pending: false,
            shell_recovery_bootstrap: false,
            cli_exit_watch: None,
            pending_channel_close: None,
            adapter,
            configured_cli: false,
            configured_followup: None,
            buffer: String::new(),
            journal_styles: Vec::new(),
            journal_units: 0,
            journal_lines: 0,
            journal_trimmed: 0,
            journal_reset: false,
            journal_clear_layout_pending: false,
            draft: String::new(),
            draft_selection: (0, 0),
            selection: (0, 0),
            first_visible_offset: 0,
            direct_input: false,
            reading: true,
            auto_cli_input: false,
            auto_cli_prompt_seen: false,
            prompt_capture: None,
            owned_prompt,
            parent_prompt_key: None,
            auto_cli_prompt_key: None,
            direct_command_input: DirectCommandInput::default(),
            vt_reading: false,
            vt_state: ReadingState::default(),
            answers_view: false,
            answer_cursor: None,
            answer_state: ReadingState::default(),
            turns,
            turn_speech: crate::turn_speech::TurnSpeech::default(),
            tui_submission: crate::tui_submission::Submission::default(),
            source_input: crate::source_input::SourceInput::default(),
            reply_anchor: crate::tui_transcript::ReplyAnchor::default(),
            latest_output: crate::tui_transcript::LatestOutput::default(),
            search: SearchState::default(),
            search_open: false,
            search_focus: false,
            search_wrapped: None,
            search_origin: None,
            reading_document: None,
            reading_document_key: None,
            reading_document_epoch: 0,
            tui_composer: None,
            composer_continuity: None,
            resized_composer: None,
            tui_menu: None,
            menu_observer: crate::tui_suggestions::MenuObserver::default(),
            tui_history: crate::tui_projection::ComposerHistory::new(),
            terminal_capture: false,
            focus_in_reader: false,
            shell_integration: false,
            screen_selection: None,
            size,
            terminal,
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

    fn set_adapter(&mut self, adapter: crate::tui_adapters::AdapterKind) {
        if self.adapter == adapter {
            return;
        }
        self.adapter = adapter;
        self.turns.set_adapter(adapter);
        self.tui_composer = None;
        self.composer_continuity = None;
        self.resized_composer = None;
        self.tui_menu = None;
        self.menu_observer.clear();
        self.source_input.clear();
        self.tui_submission = crate::tui_submission::Submission::default();
        self.reply_anchor = crate::tui_transcript::ReplyAnchor::default();
        self.latest_output = crate::tui_transcript::LatestOutput::default();
        // Immutable historical rows were already proved by the source that
        // drew them. Their content/absolute-position checks remain valid when
        // selecting a new adapter; they never authorize input or new parsing.
        // ComposerHistory itself invalidates them on source/geometry changes.
        self.reading_document_key = None;
        self.reading_document_epoch = self.reading_document_epoch.wrapping_add(1);
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
        self.poll_limited_at(max_bytes, Instant::now())
    }

    fn poll_limited_at(
        &mut self,
        max_bytes: usize,
        now: Instant,
    ) -> std::io::Result<(Option<String>, usize)> {
        if self.terminal.expire_sync(now) {
            self.revision = self.revision.wrapping_add(1);
            self.screen_selection = None;
            self.flush_terminal_replies();
        }
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
        if self.direct_input && self.turns.has_active_turn() {
            let now = Instant::now();
            let mut remaining = bytes;
            while !remaining.is_empty() {
                let (used, complete) = self.terminal.push_bytes_until_frame(remaining);
                remaining = &remaining[used..];
                if complete {
                    // Model-only observation: several redraws in one bounded
                    // PTY drain must not erase earlier portions of an answer.
                    // Speech/UI still publish once in the ordinary UI poll.
                    self.observe_tui_frame(now);
                }
            }
        } else {
            self.terminal.push_bytes(bytes);
        }
        crate::trace_slow_phase("vt_ingest", vt_started, bytes.len());
        let journal_started = Instant::now();
        self.revision = self.revision.wrapping_add(1);
        self.screen_selection = None;
        self.output.set_shell_lines(
            !self.direct_input
                && !self.auto_cli_input
                && !crate::cli_profiles::starts_in_direct_mode(&self.profile),
        );
        let mut chunk = self.output.push_chunk(bytes);
        self.speech_linear = !chunk.screen_rewrite;
        let allow_clear = !self.direct_input
            && !self.auto_cli_input
            && !self.configured_cli
            && !self.shell_recovery_bootstrap
            && matches!(self.state, SessionState::Starting | SessionState::Running)
            && self.input.check_error().is_ok()
            && (!self.owns_cli_exit(Instant::now())
                || self
                    .cli_exit_watch
                    .as_ref()
                    .is_some_and(|watch| watch.shell_command_queued))
            && !crate::cli_profiles::starts_in_direct_mode(&self.profile);
        let mut reset_at = None;
        for (index, event) in chunk.events.iter().enumerate() {
            match *event {
                TerminalEvent::Clear(position) => {
                    self.journal_clear_layout_pending = allow_clear;
                    if allow_clear {
                        reset_at = Some((index, position));
                    }
                }
                TerminalEvent::ClearLayout(position)
                    if allow_clear
                        && self.journal_clear_layout_pending
                        && (reset_at.is_some()
                            || self
                                .buffer
                                .bytes()
                                .all(|byte| matches!(byte, b'\r' | b'\n'))) =>
                {
                    self.journal_clear_layout_pending = false;
                    reset_at = Some((index, position));
                }
                TerminalEvent::ClearLayout(_) => self.journal_clear_layout_pending = false,
                _ => {}
            }
        }
        if let Some((event_index, position)) = reset_at {
            self.clear_journal_model();
            let mut units = 0;
            let byte_offset = chunk
                .text
                .char_indices()
                .find_map(|(offset, character)| {
                    let found = (units == position).then_some(offset);
                    units += character.len_utf16();
                    found
                })
                .unwrap_or(chunk.text.len());
            chunk.text = chunk.text.split_off(byte_offset);
            StyleSpan::trim_prefix(&mut chunk.styles, position as u32);
            chunk.events.drain(..=event_index);
            for event in &mut chunk.events {
                match event {
                    TerminalEvent::PromptStart(offset)
                    | TerminalEvent::CommandStart(offset)
                    | TerminalEvent::OwnedPromptStart(offset, _)
                    | TerminalEvent::OwnedCommandStart(offset, _) => {
                        *offset = offset.saturating_sub(position);
                    }
                    _ => {}
                }
            }
        }
        let base = self.journal_units;
        for span in chunk.styles {
            StyleSpan::append(
                &mut self.journal_styles,
                base + span.start,
                base + span.end,
                span.style,
            );
        }
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
            let event = match event {
                TerminalEvent::OwnedPromptStart(position, token)
                    if self.owned_prompt == Some(token) =>
                {
                    TerminalEvent::PromptStart(position)
                }
                TerminalEvent::OwnedCommandStart(position, token)
                    if self.owned_prompt == Some(token) =>
                {
                    TerminalEvent::CommandStart(position)
                }
                TerminalEvent::OwnedPromptStart(_, _) | TerminalEvent::OwnedCommandStart(_, _) => {
                    self.prompt_capture = None;
                    continue;
                }
                TerminalEvent::PromptStart(_) | TerminalEvent::CommandStart(_)
                    if self.owned_prompt.is_some() =>
                {
                    self.prompt_capture = None;
                    continue;
                }
                event => event,
            };
            match event {
                TerminalEvent::PromptStart(position) => {
                    self.shell_integration = true;
                    self.prompt_capture = Some(Vec::new());
                    self.direct_command_input.prompt_started();
                    prompt_segment_start = position;
                    if self.auto_cli_input {
                        self.auto_cli_prompt_seen = false;
                    }
                    self.blocks.prompt_started(base + position as u32);
                }
                TerminalEvent::CommandStart(position) => {
                    self.shell_integration = true;
                    self.capture_prompt_segment(&prompt_units, prompt_segment_start, position);
                    let captured = self.prompt_capture.take();
                    let key = if let Some(token) = self.owned_prompt {
                        captured.map(|_| token)
                    } else {
                        captured.filter(|prompt| !prompt.is_empty()).map(|prompt| {
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            prompt.hash(&mut hasher);
                            hasher.finish()
                        })
                    };
                    prompt_segment_start = position;
                    if key.is_some() {
                        self.shell_recovery_bootstrap = false;
                    }
                    if let Some(watch) = self.cli_exit_watch.as_mut() {
                        if key == Some(watch.parent_prompt) {
                            watch.returned_at.get_or_insert_with(Instant::now);
                        }
                    }
                    if self.auto_cli_input {
                        self.auto_cli_prompt_seen =
                            key.is_some() && key == self.auto_cli_prompt_key;
                        if self.auto_cli_prompt_seen {
                            self.direct_command_input.waiting_for_prompt = false;
                        }
                    } else {
                        self.parent_prompt_key = key;
                        if !crate::cli_profiles::starts_in_direct_mode(&self.profile) {
                            self.direct_command_input
                                .prompt_ready(key, !self.direct_input);
                        }
                    }
                    self.blocks.command_started(base + position as u32);
                }
                TerminalEvent::Clear(_)
                | TerminalEvent::ClearLayout(_)
                | TerminalEvent::OwnedPromptStart(_, _)
                | TerminalEvent::OwnedCommandStart(_, _) => {}
            }
        }
        self.capture_prompt_segment(&prompt_units, prompt_segment_start, prompt_units.len());
        self.blocks.text_updated(&self.buffer);
        self.trim_journal();
        crate::trace_slow_phase("journal_model", journal_started, self.buffer.len());
        // Journal/VT ingestion must finish even when a protocol reply cannot
        // be sent. A dead process does not need replies to its final output.
        self.flush_terminal_replies();
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    fn flush_terminal_replies(&mut self) {
        let responses = self.terminal.drain_responses();
        if !responses.is_empty() {
            if let Err(error) = self.input.write(&responses) {
                if error.kind() != std::io::ErrorKind::WouldBlock && self.pty.is_alive() {
                    self.channel_failed(&error, "ошибка ввода", Instant::now());
                }
            }
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

    /// Keep a rejected key out of a replacement shell. A closing native pipe
    /// may be observed by a user write just before the normal poll sees it.
    pub fn write_interactive(&mut self, data: &[u8]) -> std::io::Result<()> {
        let accepting = self.accepts_input();
        let wire = if accepting
            && self.adapter == crate::tui_adapters::AdapterKind::Codex
            && self.terminal.modes().win32_input
        {
            crate::codex::literal_input(data)?
        } else {
            None
        };
        let result = self.write(wire.as_deref().unwrap_or(data));
        if accepting {
            if let Err(error) = &result {
                let now = Instant::now();
                if closed_pipe(error) && (self.configured_cli || self.owns_cli_exit(now)) {
                    self.channel_failed(error, "ошибка ввода", now);
                }
            }
        }
        result
    }

    pub fn write_tui(&self, data: &[u8], expected_input: u64) -> std::io::Result<()> {
        if !self.accepts_input()
            || self.input_revision.get() != expected_input
            || self.terminal.sync_pending()
        {
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

    pub fn journal_styles(&self) -> &[StyleSpan] {
        &self.journal_styles
    }

    pub fn set_startup_text(&mut self, text: &str) {
        self.buffer = text.to_owned();
        self.journal_units = text.encode_utf16().count() as u32;
        self.journal_lines = text.bytes().filter(|byte| *byte == b'\n').count();
        self.journal_styles.clear();
        StyleSpan::append(
            &mut self.journal_styles,
            0,
            self.journal_units,
            TextStyle::default(),
        );
        self.trim_journal();
    }

    pub fn append_notice(&mut self, text: &str) {
        self.buffer.push_str(text);
        let start = self.journal_units;
        self.journal_units += text.encode_utf16().count() as u32;
        StyleSpan::append(
            &mut self.journal_styles,
            start,
            self.journal_units,
            TextStyle::default(),
        );
        self.journal_lines += text.bytes().filter(|byte| *byte == b'\n').count();
        self.trim_journal();
    }

    pub fn take_journal_trim(&mut self) -> u32 {
        std::mem::take(&mut self.journal_trimmed)
    }

    /// A full reset requires replacing native text from the final model,
    /// including when the clearing VT chunk contained no printable text.
    pub fn take_journal_reset(&mut self) -> bool {
        std::mem::take(&mut self.journal_reset)
    }

    fn clear_journal_model(&mut self) {
        self.buffer.clear();
        self.journal_styles.clear();
        self.journal_units = 0;
        self.journal_lines = 0;
        self.journal_trimmed = 0;
        self.journal_reset = true;
        self.selection = (0, 0);
        self.first_visible_offset = 0;
        self.blocks = CommandBlocks::new();
        self.prompt_capture = None;
        self.direct_command_input = DirectCommandInput::default();
        self.output_speech.reset();
        self.speech_linear = false;
    }

    fn trim_journal(&mut self) {
        let removed = journal::trim(&mut self.buffer, self.journal_lines);
        if removed == 0 {
            return;
        }
        self.journal_units -= removed;
        StyleSpan::trim_prefix(&mut self.journal_styles, removed);
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
            && !self.shell_recovery_bootstrap
            && (!self.shell_integration || self.blocks.can_execute())
            && (!self.direct_input || !self.direct_command_input.waiting_for_prompt)
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
        self.set_adapter(crate::cli_profiles::adapter_for_profile(&self.profile));
        self.direct_input = false;
        self.set_answers_view(false);
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
        self.direct_command_input = DirectCommandInput::default();
    }

    /// Identity of the published view, including raw/projected handoff without
    /// any new PTY bytes. Source protocol acknowledgements use TerminalModel's
    /// separate reading_revision and must never use this view epoch.
    pub fn reading_document_revision(&self) -> u64 {
        let key = (self.terminal.reading_revision(), self.direct_input);
        if self.reading_document_key == Some(key) {
            self.reading_document_epoch
        } else {
            self.reading_document_epoch.wrapping_add(1)
        }
    }

    pub fn reading_document(&mut self) -> &ReadingDocument {
        self.reading_document_at(Instant::now())
    }

    fn reading_document_at(&mut self, now: Instant) -> &ReadingDocument {
        let revision = self.terminal.reading_revision();
        let key = (revision, self.direct_input);
        if self.reading_document_key != Some(key) {
            self.reading_document_epoch = self.reading_document_epoch.wrapping_add(1);
            let mut document = if self.direct_input {
                self.terminal.tui_reading_document()
            } else {
                self.terminal.reading_document()
            };
            if self.direct_input {
                if let Some(previous) = self
                    .tui_composer
                    .as_ref()
                    .filter(|input| input.caret.is_some() && !input.protected)
                {
                    self.composer_continuity = Some((previous.clone(), now));
                } else if self
                    .composer_continuity
                    .as_ref()
                    .is_some_and(|(_, started)| {
                        now.saturating_duration_since(*started) > COMPOSER_CONTINUITY_AGE
                    })
                {
                    self.composer_continuity = None;
                }
                let mut frame = self.terminal.live_snapshot();
                if self
                    .composer_continuity
                    .as_ref()
                    .is_some_and(|(baseline, _)| !baseline.matches_source(&frame))
                {
                    // A hidden source switch breaks continuity immediately;
                    // switching back later must not resurrect its old proof.
                    self.composer_continuity = None;
                }
                frame.cursor = crate::direct_accessibility::reading_cursor(&frame, None);
                if self.resized_composer.as_ref().is_some_and(|pending| {
                    pending.size != self.size
                        || now.saturating_duration_since(pending.started) > COMPOSER_RESIZE_AGE
                        || !pending.input.matches_buffer(&frame)
                }) {
                    self.resized_composer = None;
                }
                let raw_document = document;
                document = self.tui_history.filter(&frame, &raw_document);
                let accepted = self.source_input.confirmation(&frame, revision, now);
                let mut projection = crate::tui_projection::project_for_adapter(
                    self.adapter,
                    &frame,
                    &document,
                    self.tui_composer.as_ref(),
                    accepted.map(|(evidence, _)| evidence),
                );
                if let Some((evidence, Some(baseline))) = accepted {
                    let recovered = crate::tui_projection::project_recovered_for_adapter(
                        self.adapter,
                        &frame,
                        &document,
                        baseline,
                        evidence,
                    );
                    // An unchanged intermediate frame can advance the ordinary
                    // panel revision before the accepted insertion is painted.
                    // Exact proof from its original origin outranks the weaker
                    // current palette projection, even when that is Some.
                    if recovered.input.is_some() {
                        projection = recovered;
                    }
                }
                if (projection.input.is_none()
                    || self.adapter == crate::tui_adapters::AdapterKind::OpenCode)
                    && self.tui_menu.is_none()
                {
                    if let Some((evidence, baseline)) =
                        self.source_input.edit_confirmation(&frame, revision, now)
                    {
                        projection = crate::tui_projection::project_edited_for_adapter(
                            self.adapter,
                            &frame,
                            &document,
                            baseline,
                            evidence,
                        );
                    }
                }
                if let Some((baseline, _)) = self.composer_continuity.as_ref() {
                    let continued = crate::tui_projection::project_continued_for_adapter(
                        self.adapter,
                        &frame,
                        &document,
                        baseline,
                    );
                    // Home may place the source cursor at the beginning of
                    // real text. Exact prior glyph/style proof outranks the
                    // ordinary "unentered hint" interpretation at column 0.
                    if continued.input.is_some() {
                        projection = continued;
                    }
                }
                if frame.cursor.is_some()
                    || projection
                        .input
                        .as_ref()
                        .is_some_and(|input| input.protected)
                {
                    self.composer_continuity = None;
                }
                if let Some(pending) = self
                    .resized_composer
                    .as_ref()
                    .filter(|pending| revision > pending.after_revision)
                {
                    let resized = crate::tui_projection::project_resized_for_adapter(
                        self.adapter,
                        &frame,
                        &document,
                        &pending.input,
                    );
                    if resized.input.is_some() {
                        projection = resized;
                    } else if projection
                        .input
                        .as_ref()
                        .is_some_and(|input| input.text != pending.input.text)
                    {
                        self.resized_composer = None;
                    }
                }
                if let Some(input) = projection.input.as_ref() {
                    self.source_input.acknowledge(input);
                }
                self.tui_composer = projection.input;
                if let Some(input) = &self.tui_composer {
                    self.tui_history.remember(&frame, &raw_document, input);
                }
                self.tui_menu = self.menu_observer.observe(
                    &frame,
                    revision,
                    self.tui_composer
                        .as_ref()
                        .map(|input| (input.first_row, input.last_row)),
                );
                if self.tui_menu.is_some() {
                    self.source_input.cancel_edit();
                }
                document = projection.output;
                if let Some(menu) = &self.tui_menu {
                    self.tui_history.remember_rows(
                        &frame,
                        &raw_document,
                        menu.first_row,
                        menu.last_row,
                    );
                    document = crate::tui_projection::exclude_rows(
                        &document,
                        menu.first_row,
                        menu.last_row,
                    );
                }
            } else {
                self.tui_composer = None;
                self.composer_continuity = None;
                self.resized_composer = None;
                self.source_input.clear();
                self.tui_menu = None;
                self.menu_observer.clear();
            }
            document.revision = self.reading_document_epoch;
            self.reading_document = Some(Arc::new(document));
            self.reading_document_key = Some(key);
        }
        self.reading_document
            .as_ref()
            .expect("reading document cached")
    }

    pub fn reading_document_snapshot(&mut self) -> Arc<ReadingDocument> {
        self.reading_document_snapshot_at(Instant::now())
    }

    fn reading_document_snapshot_at(&mut self, now: Instant) -> Arc<ReadingDocument> {
        self.reading_document_at(now);
        Arc::clone(
            self.reading_document
                .as_ref()
                .expect("reading document cached"),
        )
    }

    pub fn observe_tui_frame(&mut self, now: Instant) {
        if !self.direct_input || self.terminal.sync_pending() {
            return;
        }
        let output = self.reading_document_snapshot_at(now);
        let frame = self.terminal.live_snapshot();
        self.tui_submission
            .observe_at(self.tui_composer.as_ref(), self.tui_menu.is_some(), now);
        self.turns.observe_for_adapter(
            self.adapter,
            &frame,
            &output,
            self.tui_composer.as_ref(),
            self.tui_menu.is_some(),
            now,
        );
    }

    pub fn display_key(&self) -> (u64, bool) {
        if self.answers_view {
            (self.turns.revision(), true)
        } else {
            (self.reading_document_revision(), false)
        }
    }

    /// Called only after explicit user input was accepted by the PTY. A menu,
    /// slash command or unknown Enter still has readable terminal output even
    /// though it must not create a semantic assistant request.
    pub fn record_output_submission(
        &mut self,
        bytes: &[u8],
        model_request: bool,
        baseline: &ReadingDocument,
        now: Instant,
    ) {
        if !self.direct_input {
            return;
        }
        self.turns.accepted_output_input(
            bytes,
            &self.terminal.live_snapshot(),
            baseline,
            self.tui_composer.as_ref(),
            self.tui_menu.as_ref(),
            now,
        );
        if self.direct_input && (bytes == b"\r" || model_request) {
            self.latest_output
                .submitted(self.turns.latest_output().map(|output| output.id));
            self.reply_anchor.submitted(baseline);
        }
    }

    pub fn latest_output_uses_archive(&self) -> bool {
        self.direct_input
            && self
                .latest_output
                .prefers_archive(self.turns.latest_output().map(|output| output.id))
    }

    /// The same live/source document used by ordinary TUI reading, including
    /// command results and scrollback, independently of an open answer archive.
    pub fn source_output_for_reading(&mut self) -> (Arc<ReadingDocument>, u32) {
        self.set_answers_view(false);
        let document = self.reading_document_snapshot();
        let live_start = document
            .cells
            .iter()
            .find(|cell| cell.line >= 0)
            .map_or(0, |cell| cell.start);
        let start = self.reply_anchor.offset(&document).unwrap_or(live_start);
        (document, start)
    }

    pub fn set_answers_view(&mut self, enabled: bool) {
        if self.answers_view != enabled {
            self.answers_view = enabled;
            self.search.invalidate_view();
        }
    }

    pub fn display_document_snapshot(&mut self) -> Arc<ReadingDocument> {
        if self.answers_view {
            self.turns.document()
        } else {
            self.reading_document_snapshot()
        }
    }

    pub fn display_reading_state(&mut self) -> &mut ReadingState {
        if self.answers_view {
            &mut self.answer_state
        } else {
            &mut self.vt_state
        }
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
                self.channel_failed(&error, "ошибка ввода", Instant::now());
            }
            return Err(error);
        }
        self.direct_command_input = DirectCommandInput::default();
        self.record_command_submission(command);
        Ok(())
    }

    /// Observe each accepted direct-input batch once, after `write` succeeds.
    /// Neither terminal output nor rejected bytes can establish a CLI launch.
    pub fn record_direct_input(&mut self, data: &[u8]) {
        if !data.is_empty() {
            // A resize proof describes only the untouched editor. Actual
            // input now owns its continuation, including explicit newlines.
            self.resized_composer = None;
        }
        if !self.direct_input
            || !self.shell_integration
            || crate::cli_profiles::starts_in_direct_mode(&self.profile)
        {
            self.direct_command_input = DirectCommandInput::default();
            return;
        }
        if self.auto_cli_input {
            return;
        }
        let Some(command) = self.direct_command_input.push(data) else {
            return;
        };
        let parent_key = self
            .direct_command_input
            .parent_key
            .or(self.parent_prompt_key);
        if let Some(command) = command {
            self.record_direct_command(&command);
        }
        self.direct_command_input.parent_key = parent_key;
        self.direct_command_input.waiting_for_prompt = true;
    }

    /// Records an explicitly typed shell command after its direct-mode Enter
    /// was accepted. The caller supplies its trusted input edit, never text
    /// reconstructed from the terminal's output or an unknown TUI prompt.
    /// No bytes are sent here, and ordinary commands keep manual direct mode.
    pub fn record_direct_command(&mut self, command: &str) -> bool {
        if !self.direct_input
            || self.auto_cli_input
            || !self.shell_integration
            || crate::cli_profiles::starts_in_direct_mode(&self.profile)
            || self.parent_prompt_key.is_none()
            || self.prompt_capture.is_some()
            || !self.can_submit_command()
        {
            return false;
        }
        self.record_command_submission(command);
        true
    }

    fn record_command_submission(&mut self, command: &str) {
        if self.shell_integration {
            self.blocks.execute(command, &self.buffer);
        } else {
            self.blocks.execute_untracked(command, &self.buffer);
        }
        let adapter = crate::cli_profiles::adapter_for_command(command);
        self.auto_cli_input = adapter.is_some();
        self.set_adapter(
            adapter.unwrap_or_else(|| crate::cli_profiles::adapter_for_profile(&self.profile)),
        );
        self.auto_cli_prompt_seen = false;
        self.auto_cli_prompt_key = self
            .auto_cli_input
            .then_some(self.parent_prompt_key)
            .flatten();
        // Process ownership outlives the UI's return to the command field.
        // A restored prompt alone does not prove PowerShell survived teardown.
        if self.auto_cli_input {
            self.cli_exit_watch = recoverable_shell_profile(&self.profile)
                .then_some(self.parent_prompt_key)
                .flatten()
                .map(|parent_prompt| CliExitWatch {
                    parent_prompt,
                    returned_at: None,
                    shell_command_queued: false,
                });
        } else {
            // Enqueue is not delivery: the worker can discover a closed pipe
            // only when writing this first post-CLI command. Keep the original
            // fixed handoff window; further commands never extend it. Explicit
            // shell clear is allowed, since the new command belongs to the user.
            self.cli_exit_watch = self
                .cli_exit_watch
                .take()
                .filter(|watch| watch.returned_at.is_some() && watch.fresh(Instant::now()));
            if let Some(watch) = self.cli_exit_watch.as_mut() {
                watch.shell_command_queued = true;
            }
        }
    }

    pub fn resize(&mut self, size: PtySize) -> std::io::Result<()> {
        self.resize_at(size, Instant::now())
    }

    fn resize_at(&mut self, size: PtySize, now: Instant) -> std::io::Result<()> {
        if self.size == size {
            return Ok(());
        }
        self.composer_continuity = None;
        if self.pty.is_alive() && self.failure.is_none() {
            self.pty.resize(size)?;
        }
        let frame = self.terminal.live_snapshot();
        if let Some(input) = self.tui_composer.as_ref().filter(|input| {
            matches!(
                input.adapter(),
                crate::tui_adapters::AdapterKind::Codex
                    | crate::tui_adapters::AdapterKind::OpenCode
            ) && !input.protected
                && input.matches_source(&frame)
        }) {
            self.resized_composer = Some(ResizedComposer {
                input: input.clone(),
                size,
                started: now,
                after_revision: 0,
            });
        } else if let Some(pending) = self.resized_composer.as_mut() {
            if pending.size == self.size
                && now.saturating_duration_since(pending.started) <= COMPOSER_RESIZE_AGE
                && pending.input.matches_buffer(&frame)
            {
                pending.size = size;
            } else {
                self.resized_composer = None;
            }
        }
        self.source_input.clear();
        self.direct_command_input.resize();
        self.terminal.resize(size.cols as usize, size.rows as usize);
        if let Some(pending) = self.resized_composer.as_mut() {
            // Resizing the VT grid leaves a copy of the old layout. Only
            // subsequent source output can acknowledge the application's reflow.
            pending.after_revision = self.terminal.reading_revision();
        }
        self.screen_selection = None;
        self.revision = self.revision.wrapping_add(1);
        self.size = size;
        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }

    pub fn tui_input_ready(&self) -> bool {
        self.accepts_input()
            && !self.terminal.sync_pending()
            && self.pty.pending_output_bytes() == 0
            && self.input.is_idle()
    }

    pub fn blocks(&self) -> &CommandBlocks {
        &self.blocks
    }

    pub fn blocks_mut(&mut self) -> &mut CommandBlocks {
        &mut self.blocks
    }

    fn update_state(&mut self) {
        self.update_state_at(Instant::now());
    }

    fn update_state_at(&mut self, now: Instant) {
        match self.pty.exit_code() {
            Ok(Some(code)) => {
                if self.exit_code.is_none() {
                    // OpenCode can disconnect PowerShell's cached CONOUT$
                    // handle on leaving its alternate screen. Only this exact
                    // managed FailFast, during an accepted CLI launch in an
                    // owned parent pwsh, permits one fresh shell. Never infer
                    // termination or a command from restored screen contents.
                    self.shell_recovery_pending = code == CLR_FAILFAST_EXIT
                        && self.failure.is_none()
                        && self.owns_cli_exit(now);
                }
                self.pending_channel_close = None;
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
                let expired = self
                    .pending_channel_close
                    .as_ref()
                    .filter(|pending| {
                        now.saturating_duration_since(pending.started) >= CHANNEL_EXIT_GRACE
                    })
                    .map(|pending| pending.reason.clone());
                if let Some(reason) = expired {
                    self.fail(reason);
                } else if let Err(error) = self.input.check_error() {
                    self.channel_failed(&error, "ошибка ввода", now);
                } else if self.state == SessionState::Starting {
                    self.state = SessionState::Running;
                }
            }
            Err(error) => self.fail(format!("ошибка состояния процесса: {error}")),
        }
        if let Some(error) = self.pty.output_error() {
            if self.exit_code.is_none() || !closed_pipe(&error) {
                self.channel_failed(&error, "ошибка чтения", now);
            }
        }
        if let Some(code) = self.exit_code {
            if self.pty.output_drained() && self.failure.is_none() {
                self.state = SessionState::Exited;
                self.issue_notice(format!("{}: сессия завершена, код {code}", self.title));
            }
        }
        if !matches!(self.state, SessionState::Starting | SessionState::Running) {
            self.release_stopped_input();
        }
    }

    pub fn accepts_input(&self) -> bool {
        self.failure.is_none()
            && matches!(self.state, SessionState::Starting | SessionState::Running)
            && self.pty.is_alive()
    }

    fn release_stopped_input(&mut self) {
        if !self.direct_input && !self.auto_cli_input && !self.terminal_capture {
            return;
        }
        self.cancel_cli_input();
        self.direct_input = false;
        self.terminal_capture = false;
        self.source_input.clear();
        self.tui_submission = crate::tui_submission::Submission::default();
        self.tui_composer = None;
        self.composer_continuity = None;
        self.resized_composer = None;
        self.tui_menu = None;
        self.menu_observer.clear();
        if !self.reading {
            self.vt_reading = false;
            self.focus_in_reader = false;
        }
        self.reading = true;
    }

    fn inherit_shell_history(&mut self, previous: &mut Self) {
        self.title = previous.title.clone();
        self.buffer = std::mem::take(&mut previous.buffer);
        self.journal_styles = std::mem::take(&mut previous.journal_styles);
        self.journal_units = previous.journal_units;
        self.journal_lines = previous.journal_lines;
        self.journal_trimmed = previous.journal_trimmed;
        self.journal_reset = previous.journal_reset;
        self.blocks = std::mem::replace(&mut previous.blocks, CommandBlocks::new());
        self.draft = std::mem::take(&mut previous.draft);
        self.draft_selection = previous.draft_selection;
        self.selection = previous.selection;
        self.first_visible_offset = previous.first_visible_offset;
        self.turns = std::mem::take(&mut previous.turns);
        self.turns.cancel_pending();
        self.shell_recovery_bootstrap = true;
        // The tab survives, but source protocols, pending input and projected
        // cells belong to a new process. Old guarded actions cannot be reused.
        self.input_revision
            .set(previous.input_revision().wrapping_add(1));
        self.reading_document_epoch = previous.reading_document_epoch.wrapping_add(1);
        self.revision = previous.revision.wrapping_add(1);
    }

    fn fail(&mut self, reason: String) {
        if self.failure.is_some() {
            return;
        }
        self.failure = Some(reason.clone());
        self.cli_exit_watch = None;
        self.pending_channel_close = None;
        self.state = SessionState::Failed;
        self.issue_notice(format!("{}: ошибка сессии: {reason}", self.title));
        // A failed input/output channel cannot recover in-place. Stop only
        // this owned process, but keep polling its final output and status.
        let _ = self.pty.request_terminate();
    }

    fn owns_cli_exit(&self, now: Instant) -> bool {
        recoverable_shell_profile(&self.profile)
            && self
                .cli_exit_watch
                .as_ref()
                .is_some_and(|watch| self.pending_channel_close.is_some() || watch.fresh(now))
    }

    fn channel_failed(&mut self, error: &std::io::Error, context: &str, now: Instant) {
        if self.failure.is_some() {
            return;
        }
        if !closed_pipe(error) || !(self.configured_cli || self.owns_cli_exit(now)) {
            self.fail(format!("{context}: {error}"));
            return;
        }
        let pending = self
            .pending_channel_close
            .get_or_insert_with(|| PendingChannelClose {
                started: now,
                reason: format!("{context}: {error}"),
            });
        if now.saturating_duration_since(pending.started) >= CHANNEL_EXIT_GRACE {
            let reason = pending.reason.clone();
            self.fail(reason);
        } else {
            // Never terminate the parent before its natural status is known.
            // Input stays disabled, while subsequent polls drain the VT tail.
            self.state = SessionState::Closing;
        }
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

    #[cfg(test)]
    pub fn create(&mut self, profile: &Profile, size: PtySize) -> std::io::Result<SessionId> {
        self.reap_finished();
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let session = TerminalSession::spawn(id, profile, size)?;
        self.sessions.push(session);
        self.active_id = Some(id);
        Ok(id)
    }

    pub fn create_configured(
        &mut self,
        selected: &Profile,
        plan: &crate::launch_profile::LaunchPlan,
        size: PtySize,
    ) -> std::io::Result<SessionId> {
        self.reap_finished();
        let id = self.next_id;
        let mut session =
            TerminalSession::spawn_with_env(id, &plan.process, size, &plan.environment)?;
        session.profile_id = plan.selected_id.clone();
        session.profile = selected.clone();
        session.title = plan.selected_name.clone();
        session.set_adapter(plan.adapter);
        session.configured_cli = !matches!(plan.adapter, crate::tui_adapters::AdapterKind::Shell);
        session.configured_followup = plan.followup_shell.clone();
        if session.configured_cli {
            session.direct_input = true;
            session.reading = false;
            session.vt_reading = true;
            session.terminal_capture = true;
        }
        self.next_id = self.next_id.wrapping_add(1);
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
            self.recover_crashed_cli_shell(index);
            self.open_configured_shell(index);
            if remaining == 0 || Instant::now() >= deadline {
                break;
            }
        }
        output
    }

    fn open_configured_shell(&mut self, index: usize) {
        let session = &mut self.sessions[index];
        if session.exit_code.is_none() || !session.pty.output_drained() {
            return;
        }
        // Consume the explicit after-exit action before any allocation. Never
        // repeat a CLI command, even if shell spawn or cleanup allocation fails.
        let Some(profile) = session.configured_followup.take() else {
            return;
        };
        let failed = |session: &mut TerminalSession, reason: &str| {
            session.notice = Some(SessionNotice {
                id: session.id,
                message: format!(
                    "{}: программа завершена. Оболочка не открыта: {reason}",
                    session.title
                ),
            });
        };
        if self.retiring.len() >= MAX_RETIRING {
            failed(session, "заняты обработчики завершения");
            return;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel::<TerminalSession>(1);
        let Ok(worker) = std::thread::Builder::new()
            .name("bks-profile-exit-retire".into())
            .spawn(move || {
                if let Ok(previous) = receiver.recv() {
                    drop(previous);
                }
            })
        else {
            failed(session, "не удалось запустить обработчик завершения");
            return;
        };
        let mut next = match TerminalSession::spawn(session.id, &profile, session.size) {
            Ok(next) => next,
            Err(error) => {
                failed(session, &error.to_string());
                drop(sender);
                self.retiring.push(worker);
                return;
            }
        };
        next.inherit_shell_history(session);
        next.title = profile.name.clone();
        next.turns.set_adapter(next.adapter);
        next.shell_recovery_bootstrap = recoverable_shell_profile(&profile);
        next.issue_notice(format!("{}: программа завершена. Открыта новая оболочка {}. Журнал сохранён; команды не повторялись", session.title, next.title));
        let previous = std::mem::replace(session, next);
        if sender.try_send(previous).is_err() {
            unreachable!("new profile cleanup receiver cannot exit before its first message");
        }
        self.retiring.push(worker);
    }

    fn recover_crashed_cli_shell(&mut self, index: usize) {
        let session = &mut self.sessions[index];
        if !session.shell_recovery_pending
            || session.state != SessionState::Exited
            || !session.pty.output_drained()
        {
            return;
        }
        // Consume the attempt before any allocation/spawn. A failed recovery
        // cannot become a timer loop, and a voluntary shell exit never enters.
        session.shell_recovery_pending = false;
        let failed = |session: &mut TerminalSession, detail: &str| {
            session.notice = Some(SessionNotice {
                id: session.id,
                message: format!("{}: оболочка аварийно завершилась после CLI (0x80131623). Автовосстановление недоступно: {detail}. Перезапуск — в меню профилей", session.title),
            });
        };
        if self.retiring.len() >= MAX_RETIRING {
            failed(session, "заняты обработчики завершения");
            return;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel::<TerminalSession>(1);
        let Ok(worker) = std::thread::Builder::new()
            .name("bks-shell-recover-retire".into())
            .spawn(move || {
                if let Ok(session) = receiver.recv() {
                    drop(session);
                }
            })
        else {
            failed(session, "не удалось запустить обработчик завершения");
            return;
        };
        let mut next = match TerminalSession::spawn(session.id, &session.profile, session.size) {
            Ok(next) => next,
            Err(error) => {
                failed(session, &error.to_string());
                drop(sender);
                self.retiring.push(worker);
                return;
            }
        };
        next.inherit_shell_history(session);
        next.issue_notice(format!("{}: оболочка аварийно завершилась после CLI (0x80131623). Запущена новая оболочка с тем же профилем. Журнал и черновик сохранены; локальные переменные прежнего процесса утрачены. Команды не повторялись", next.title));
        let previous = std::mem::replace(session, next);
        // The only receiver has just been created and can only exit after
        // receiving this message or losing this sole sender. Capacity is one,
        // so publishing the already exited session never waits for the worker.
        if sender.try_send(previous).is_err() {
            unreachable!("new shell cleanup worker cannot disconnect before its first message");
        }
        self.retiring.push(worker);
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

    fn configured_fixture(
        after_exit: crate::launch_profile::AfterExit,
    ) -> (crate::profile::ProfileManager, Profile) {
        let shell = failfast_shell_profile();
        let mut cli = test_profile("CONFIGURED_CLI");
        cli.launch.adapter = crate::tui_adapters::AdapterKind::Universal;
        cli.launch.after_exit = after_exit;
        cli.launch.environment.insert(
            "BKS_PROFILE_SESSION_FIXTURE".into(),
            "BKS_CONFIGURED_ENV_OK".into(),
        );
        cli.executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
            .to_string_lossy()
            .into_owned();
        cli.args = vec!["-NoLogo".into(), "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "[Console]::WriteLine($env:BKS_PROFILE_SESSION_FIXTURE); for ($i=1; $i -le 256; $i++) { [Console]::WriteLine(('CFG_ROW_' + $i)) }; [Console]::WriteLine('BKS_CONFIGURED_FINAL_TAIL'); exit 23".into()];
        let manager = crate::profile::ProfileManager {
            profiles: vec![shell.clone(), cli.clone()],
            default_profile_id: shell.id.clone(),
            default_shell_profile_id: Some(shell.id),
        };
        (manager, cli)
    }

    #[test]
    fn configured_cli_exit_policy_preserves_the_final_tail_and_runs_no_cli_twice() {
        use crate::launch_profile::{AfterExit, LaunchPlan};
        for after_exit in [AfterExit::KeepTab, AfterExit::OpenShell] {
            let (profiles, cli) = configured_fixture(after_exit);
            let plan = LaunchPlan::resolve(&profiles, &cli).unwrap();
            let mut manager = SessionManager::new();
            let id = manager
                .create_configured(&cli, &plan, PtySize::new(100, 29))
                .unwrap();
            let original_revision = manager.get(id).unwrap().input_revision();
            manager
                .get_mut(id)
                .unwrap()
                .set_startup_text("kept configured history\r\n");
            manager.get_mut(id).unwrap().draft = "unsent configured draft".into();
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                manager.poll();
                let session = manager.get(id).unwrap();
                let done = match after_exit {
                    AfterExit::KeepTab => session.state == SessionState::Exited,
                    AfterExit::OpenShell => {
                        session.profile_id != cli.id && session.can_submit_command()
                    }
                };
                if done {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "configured exit action did not finish"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            let session = manager.get(id).unwrap();
            assert!(session.buffer().starts_with("kept configured history\r\n"));
            assert!(
                session.buffer().contains("CFG_ROW_256"),
                "owned fixture output: {:?}",
                session.buffer()
            );
            assert_eq!(
                session
                    .buffer()
                    .matches("BKS_CONFIGURED_FINAL_TAIL")
                    .count(),
                1
            );
            assert!(session.buffer().contains("BKS_CONFIGURED_ENV_OK"));
            assert_eq!(session.draft, "unsent configured draft");
            assert!(!session.direct_input && !session.terminal_capture);
            assert!(session.configured_followup.is_none());
            if after_exit == AfterExit::OpenShell {
                assert!(session.input_revision() > original_revision);
                manager
                    .get_mut(id)
                    .unwrap()
                    .submit_command("Write-Output ('BKS_FRESH_' + 'SHELL_COMMAND')")
                    .unwrap();
                while !manager
                    .get(id)
                    .unwrap()
                    .buffer()
                    .contains("BKS_FRESH_SHELL_COMMAND")
                {
                    manager.poll();
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
            } else {
                assert_eq!(session.exit_code, Some(23));
            }
            for _ in 0..8 {
                manager.poll();
            }
            assert_eq!(
                manager
                    .get(id)
                    .unwrap()
                    .buffer()
                    .matches("BKS_CONFIGURED_FINAL_TAIL")
                    .count(),
                1
            );
            manager.close_all();
            manager.finish_cleanup();
        }
    }

    #[test]
    fn configured_followup_spawn_failure_is_reported_once_and_preserves_the_cli_result() {
        let (profiles, cli) = configured_fixture(crate::launch_profile::AfterExit::OpenShell);
        let mut plan = crate::launch_profile::LaunchPlan::resolve(&profiles, &cli).unwrap();
        plan.followup_shell.as_mut().unwrap().executable =
            r"C:\BKS-missing-profile-fixture\cmd.exe".into();
        let mut manager = SessionManager::new();
        let id = manager
            .create_configured(&cli, &plan, PtySize::new(100, 29))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            manager.poll();
            let session = manager.get(id).unwrap();
            if session.exit_code.is_some() && session.pty.output_drained() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let session = manager.get(id).unwrap();
        assert_eq!(session.profile_id, cli.id);
        assert!(session.buffer().contains("BKS_CONFIGURED_FINAL_TAIL"));
        assert!(session.configured_followup.is_none());
        let notices = manager.take_events();
        assert_eq!(
            notices
                .iter()
                .filter(|notice| notice.message.contains("Оболочка не открыта"))
                .count(),
            1
        );
        for _ in 0..8 {
            manager.poll();
        }
        assert!(manager.take_events().is_empty());
        manager.close_all();
        manager.finish_cleanup();
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
            launch: Default::default(),
        }
    }

    #[test]
    fn shell_clear_rebases_unicode_styles_prompt_blocks_and_reader_positions() {
        let mut session = literal_direct_session("SHELL_CLEAR_STATE");
        session.direct_input = false;
        session.ingest_output(b"\x1b[31mold result\r\n").unwrap();
        own_parent_prompt(&mut session);
        let parent_key = session.parent_prompt_key;
        session
            .blocks
            .execute_untracked("old", session.buffer.clone().as_str());
        assert!(session.blocks.last().is_some());
        session.selection = (2, 8);
        session.first_visible_offset = 2;
        session.journal_trimmed = 17;
        session.draft = "unsent draft".into();
        let generation = session.input_revision();
        let now = Instant::now();
        session.output_speech.observe(now, "old", "", false, true);
        session
            .output_speech
            .observe(now, "old\r\npending", "pending", false, true);
        session
            .ingest_output("discard🙂\x1b]133;A\x07OTHER> \x1b]133;B\x07\x1b[2J\x1b[32m界🙂\r\n\x1b]133;A\x07PS> \x1b]133;B\x07".as_bytes())
            .unwrap();
        assert_eq!(session.buffer(), "界🙂\r\nPS> ");
        assert_eq!(session.journal_units, 9);
        assert_eq!(session.selection, (0, 0));
        assert_eq!(session.first_visible_offset, 0);
        assert_eq!(session.take_journal_trim(), 0);
        assert_eq!(session.blocks.last(), None);
        assert!(session.blocks.can_execute());
        assert_eq!(session.parent_prompt_key, parent_key);
        assert_eq!(session.draft, "unsent draft");
        assert_eq!(session.input_revision(), generation);
        assert_eq!(session.journal_styles.len(), 1);
        let span = session.journal_styles[0];
        assert_eq!((span.start, span.end), (0, 9));
        assert_eq!(
            span.style.foreground,
            bks_terminal::Color::Named(bks_terminal::NamedColor::Green)
        );
        assert!(session.take_journal_reset());
        assert!(!session.take_journal_reset());
        assert!(session
            .output_speech
            .take_due(
                now + Duration::from_secs(1),
                crate::output_speech::SpeechMode::All
            )
            .is_none());
        session.ingest_output(b"tail").unwrap();
        assert!(!session.take_journal_reset());
        assert_eq!(session.journal_styles.last().unwrap().end, 13);
    }

    #[test]
    fn pure_and_repeated_clear_keep_only_the_final_suffix_and_report_a_reset_without_text() {
        let mut session = literal_direct_session("SHELL_CLEAR_EMPTY");
        session.direct_input = false;
        session.set_startup_text("old🙂");
        assert!(session.ingest_output(b"\x1b[3J").unwrap().is_none());
        assert!(session.buffer().is_empty() && session.journal_styles().is_empty());
        assert!(session.take_journal_reset());
        session
            .ingest_output(
                b"\x1b]133;A\x07BAD> \x1b]133;B\x07\x1b[2J\x1b[31mforgotten\x1b[3J\x1b[34mkept",
            )
            .unwrap();
        assert_eq!(session.buffer(), "kept");
        assert!(!session.blocks.can_execute());
        assert!(session.parent_prompt_key.is_none());
        assert!(session.take_journal_reset());
        assert_eq!(session.journal_styles()[0].start, 0);
        assert_eq!(session.journal_styles()[0].end, 4);
        assert_eq!(
            session.journal_styles()[0].style.foreground,
            bks_terminal::Color::Named(bks_terminal::NamedColor::Blue)
        );
    }

    #[test]
    fn shell_clear_removes_only_proven_conpty_padding_and_preserves_postclear_blank_lines() {
        for bytewise in [false, true] {
            let mut session = literal_direct_session("SHELL_CLEAR_LAYOUT");
            session.direct_input = false;
            session.ingest_output(b"old text").unwrap();
            let bytes = b"\x1b[3J\x1b[?25l\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[H\x1b[?25h\r\n\r\n\x1b]133;A\x07PS> \x1b]133;B\x07";
            for part in bytes.chunks(if bytewise { 1 } else { bytes.len() }) {
                session.ingest_output(part).unwrap();
            }
            assert_eq!(session.buffer(), "\r\n\r\nPS> ");
            assert!(session.take_journal_reset());
            assert!(session.blocks.can_execute());
            assert_eq!(session.journal_styles()[0].start, 0);
            assert_eq!(session.journal_styles().last().unwrap().end, 8);
        }
        let mut session = literal_direct_session("SHELL_CLEAR_BLANKS");
        session.direct_input = false;
        session.ingest_output(b"old\x1b[2J\r\n\r\ntext").unwrap();
        assert_eq!(session.buffer(), "\r\n\r\ntext");
        session.ingest_output(b"\x1b[3J").unwrap();
        assert!(session.take_journal_reset());
        session.append_notice("keep notice\r\n");
        session
            .ingest_output(b"\x1b[K\r\n\x1b[K\x1b[Hafter")
            .unwrap();
        assert!(session.buffer().starts_with("keep notice\r\n"));
        assert!(session.buffer().ends_with("after"));
        assert!(!session.take_journal_reset());
    }

    #[test]
    fn tui_direct_auto_cli_profiles_and_alternate_repaints_keep_the_journal() {
        for mode in 0..6 {
            let mut session = literal_direct_session("TUI_CLEAR_GUARDS");
            session.direct_input = mode == 0;
            session.auto_cli_input = mode == 1;
            if mode == 2 {
                session.profile.id = "cli-kimi".into();
            }
            session.ingest_output(b"old journal\r\n").unwrap();
            if mode >= 3 {
                let alternate = [47, 1047, 1049][mode - 3];
                session
                    .ingest_output(format!("\x1b[?{alternate}h").as_bytes())
                    .unwrap();
            }
            session
                .ingest_output(b"\x1b[2J\x1b[3J\x1b[K\r\n\x1b[K\x1b[Hnew TUI")
                .unwrap();
            assert!(session.buffer().starts_with("old journal\r\n"));
            assert!(session.buffer().ends_with("new TUI"));
            assert!(!session.take_journal_reset());
        }
    }

    #[test]
    fn real_powershell_clear_aliases_reset_the_shell_journal_and_keep_the_next_prompt() {
        for clear in ["Clear-Host", "cls", "clear"] {
            let mut profile = test_profile("POWERSHELL_CLEAR");
            profile.executable = "powershell.exe".into();
            profile.args = vec![
                "-NoLogo".into(), "-NoProfile".into(), "-NoExit".into(), "-Command".into(),
                r"function global:prompt { [char]27 + ']133;A' + [char]27 + '\PS ' + $executionContext.SessionState.Path.CurrentLocation + '> ' + [char]27 + ']133;B' + [char]27 + '\' }".into(),
            ];
            let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while session.parent_prompt_key.is_none() && Instant::now() < deadline {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(session.parent_prompt_key.is_some(), "{}", session.buffer());
            session.take_journal_reset();
            session
                .submit_command("Write-Output 'BKS_BEFORE_CLEAR'")
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !(session.buffer().contains("BKS_BEFORE_CLEAR") && session.can_submit_command())
                && Instant::now() < deadline
            {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(session.buffer().contains("BKS_BEFORE_CLEAR"));
            session.submit_command(clear).unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !(session.journal_reset && session.can_submit_command())
                && Instant::now() < deadline
            {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                session.take_journal_reset(),
                "{clear}: {}",
                session.buffer()
            );
            assert!(
                session.buffer().starts_with("PS "),
                "{clear}: {:?}",
                session.buffer()
            );
            assert!(!session.buffer().contains("BKS_BEFORE_CLEAR"));
            assert!(session.can_submit_command());
            session
                .submit_command("Write-Output 'BKS_AFTER_CLEAR'")
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !(session.buffer().contains("BKS_AFTER_CLEAR") && session.can_submit_command())
                && Instant::now() < deadline
            {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(session.buffer().contains("BKS_AFTER_CLEAR"));
            assert!(!session.buffer().contains("BKS_BEFORE_CLEAR"));
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
    fn journal_styles_rebase_with_text_and_notices_do_not_change_source_attributes() {
        let mut session =
            TerminalSession::spawn(1, &test_profile("JOURNAL_STYLES"), PtySize::default()).unwrap();
        let prefix = "old\r\n".repeat(journal::JOURNAL_MAX_LINES - 3);
        session.set_startup_text(&prefix);
        let base = prefix.encode_utf16().count() as u32;
        session
            .ingest_output("\x1b[31mЖ🙂e\u{301}\r\n\x1b[32mKEEP\r\n".as_bytes())
            .unwrap();
        session.append_notice(&"notice\r\n".repeat(5));
        let removed = session.take_journal_trim();
        assert!(removed > 0);
        let red = session
            .journal_styles()
            .iter()
            .find(|span| {
                span.style.foreground == bks_terminal::Color::Named(bks_terminal::NamedColor::Red)
            })
            .unwrap();
        assert_eq!((red.start, red.end), (base - removed, base + 7 - removed));
        let native: Vec<u16> = session.buffer().encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&native[red.start as usize..red.end as usize]).unwrap(),
            "Ж🙂e\u{301}\r\n"
        );
        assert_eq!(
            session.journal_styles().last().unwrap().style,
            TextStyle::default()
        );
        session.ingest_output(b"tail").unwrap();
        let tail = session.journal_styles().last().unwrap();
        assert_eq!(
            tail.style.foreground,
            bks_terminal::Color::Named(bks_terminal::NamedColor::Green)
        );
        assert_eq!(tail.end, session.journal_units);
        assert_eq!(tail.end - tail.start, 4);
        for pair in session.journal_styles().windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        assert_eq!(session.journal_styles()[0].start, 0);

        session.set_startup_text("replacement🙂");
        assert_eq!(session.journal_styles().len(), 1);
        assert_eq!(session.journal_styles()[0].style, TextStyle::default());
        assert_eq!(session.journal_styles()[0].start, 0);
        assert_eq!(session.journal_styles()[0].end, session.journal_units);
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
            launch: Default::default(),
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
    fn native_and_explicit_direct_cli_launches_return_from_a_real_parent_prompt() {
        let mut profile = test_profile("POWERSHELL_HANDOFF_KEY");
        profile.executable = "powershell.exe".into();
        // This local function shadows executable lookup. It only emits a
        // managed TUI fixture; no installed CLI, settings or session is used.
        profile.args = vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NoExit".into(),
            "-Command".into(),
            r"function global:kimi {
    $e = [char]27
    [Console]::Write($e+'[?1049h'+$e+'[?7l'+$e+'[2J'+$e+'[H'+$e+'[?25l')
    [Console]::Write($e+']133;A'+[char]7+'BKS_OWN_CLI'+$e+']133;B'+[char]7+$e+']133;C'+[char]7)
    [Console]::Write($e+'[?2026h'+$e+'[?1049l'+$e+'[0m'+$e+'[?7h'+[char]13+[char]10+$e+'[?25h'+$e+'[?2026l')
    [Console]::WriteLine('BKS_OWN_CLI_EXIT')
}
function global:prompt { [char]27 + ']133;A' + [char]27 + '\PS ' + $executionContext.SessionState.Path.CurrentLocation + '> ' + [char]27 + ']133;B' + [char]27 + '\' }".into(),
        ];
        for direct in [false, true] {
            let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while session.parent_prompt_key.is_none() && Instant::now() < deadline {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(session.parent_prompt_key.is_some(), "{}", session.buffer());
            session.resize(PtySize { cols: 32, rows: 24 }).unwrap();
            if direct {
                session.direct_input = true;
                for data in [b"ki".as_slice(), b"mx", b"\x7f", b"i\r\n"] {
                    accept_direct_input(&mut session, data);
                }
            } else {
                session.submit_command("kimi").unwrap();
                session.direct_input = true;
            }
            session.reading = false;
            session.terminal_capture = true;
            assert!(session.auto_cli_input && !session.cli_return_ready());
            let deadline = Instant::now() + Duration::from_secs(15);
            while !session.cli_return_ready() && Instant::now() < deadline {
                session.poll().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                session.buffer().contains("BKS_OWN_CLI_EXIT"),
                "direct={direct}: {}",
                session.buffer()
            );
            assert!(
                session.cli_return_ready(),
                "direct={direct}: {}",
                session.buffer()
            );
            let generation = session.input_revision();
            assert!(session.finish_cli_input());
            assert!(!session.direct_input && !session.terminal_capture);
            assert_eq!(session.input_revision(), generation);
        }
    }

    #[test]
    fn stopped_root_releases_direct_input_without_restarting_an_ordinary_exit() {
        let mut profile = test_profile("STOPPED_DIRECT_ROOT");
        profile.executable = "powershell.exe".into();
        profile.args = vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(),
            "[Console]::Write([char]27+'[?1049hBKS_OWN_EXIT'+[char]27+'[?1049l'); Start-Sleep -Milliseconds 150; exit 0".into()];
        let mut manager = SessionManager::new();
        let id = manager.create(&profile, PtySize::default()).unwrap();
        let session = manager.get_mut(id).unwrap();
        session.direct_input = true;
        session.auto_cli_input = true;
        session.reading = false;
        session.terminal_capture = true;
        let deadline = Instant::now() + Duration::from_secs(10);
        while manager.get(id).unwrap().state != SessionState::Exited && Instant::now() < deadline {
            manager.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        let session = manager.get(id).unwrap();
        assert_eq!(session.state, SessionState::Exited);
        assert_eq!(session.exit_code, Some(0));
        assert!(!session.direct_input && !session.auto_cli_input && !session.terminal_capture);
        assert!(session.reading && !session.vt_reading);
        assert!(!session.accepts_input());
        assert!(session.buffer().contains("BKS_OWN_EXIT"));
        for _ in 0..3 {
            manager.poll();
        }
        assert_eq!(manager.get(id).unwrap().exit_code, Some(0));
        manager.finish_cleanup();
    }

    fn failfast_shell_profile() -> Profile {
        let mut profile = test_profile("OWN_PWSH_FAILFAST");
        let pwsh =
            PathBuf::from(std::env::var_os("ProgramFiles").unwrap()).join("PowerShell/7/pwsh.exe");
        profile.executable = pwsh.to_string_lossy().into_owned();
        profile.args = vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NoExit".into(),
            "-Command".into(),
            crate::profile::PS_PROMPT.into(),
        ];
        profile
    }

    #[test]
    fn closing_journal_survives_clear_after_exit_status_without_a_pending_pipe_error() {
        let mut profile = test_profile("CLOSING_CLEAR");
        profile.args = vec![
            "/D".into(),
            "/Q".into(),
            "/C".into(),
            "echo BKS_UNREAD_TAIL & exit 7".into(),
        ];
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        session.ingest_output(b"kept closing journal\r\n").unwrap();
        let history = session.buffer().to_owned();
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.exit_code.is_none() {
            session.poll_limited(0).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(session.state, SessionState::Closing);
        assert!(session.pending_channel_close.is_none());
        session.ingest_output(b"\x1b[?1049l\x1b[2J").unwrap();
        assert!(session.buffer().starts_with(&history));
    }

    #[test]
    fn returned_cli_clear_keeps_history_until_a_new_explicit_shell_clear() {
        let mut manager = SessionManager::new();
        let id = manager
            .create(&failfast_shell_profile(), PtySize::default())
            .unwrap();
        wait_for_own_shell(&mut manager, id);
        let session = manager.get_mut(id).unwrap();
        session.record_command_submission("opencode");
        session.cancel_cli_input();
        session.cli_exit_watch.as_mut().unwrap().returned_at = Some(Instant::now());
        let history = session.buffer().to_owned();
        session.ingest_output(b"\x1b[?1049l\x1b[2J").unwrap();
        assert!(session.buffer().starts_with(&history));
        session.record_command_submission("Clear-Host");
        session.ingest_output(b"\x1b[2J").unwrap();
        assert!(
            session.buffer().is_empty(),
            "a newly accepted user clear retains its normal meaning"
        );
        manager.close_all();
        manager.finish_cleanup();
    }

    fn native_closed_writer_unprobed() -> PtyInputWriter {
        let command = format!(
            "\"{}\\System32\\cmd.exe\" /D /Q /C exit 0",
            std::env::var("SystemRoot").unwrap()
        );
        let pty = PtySession::spawn(&command, PtySize::new(20, 4)).unwrap();
        let writer = pty.input_writer().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pty.exit_code().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        pty.request_output_finish().unwrap();
        writer
    }

    fn native_closed_writer() -> PtyInputWriter {
        let writer = native_closed_writer_unprobed();
        let deadline = Instant::now() + Duration::from_secs(5);
        while writer.check_error().is_ok() {
            let _ = writer.write(b"probe");
            assert!(
                Instant::now() < deadline,
                "owned closed ConPTY did not report a pipe error"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            closed_pipe(&writer.check_error().unwrap_err()),
            "real WriteFile error keeps its native code"
        );
        writer
    }

    #[test]
    fn closed_writer_waits_for_natural_cli_failfast_before_and_after_prompt_handoff() {
        for phase in ["running", "returned", "queued-command"] {
            let returned = phase != "running";
            let closed = if phase == "queued-command" {
                native_closed_writer_unprobed()
            } else {
                native_closed_writer()
            };
            let mut manager = SessionManager::new();
            let profile = failfast_shell_profile();
            let id = manager.create(&profile, PtySize::default()).unwrap();
            wait_for_own_shell(&mut manager, id);
            let setup = format!("function global:opencode {{ [Console]::Write([char]27+'[?1049h'+[char]27+'[2J'+[char]27+'[HRACE_ALT'); Start-Sleep -Milliseconds 200; [Console]::Write([char]27+'[?1049l'); [Console]::Write((prompt)); Start-Sleep -Milliseconds 1000; [Environment]::Exit({}) }}", CLR_FAILFAST_EXIT as i32);
            manager.get_mut(id).unwrap().submit_command(&setup).unwrap();
            wait_for_own_shell(&mut manager, id);
            manager
                .get_mut(id)
                .unwrap()
                .submit_command("opencode")
                .unwrap();
            manager.get_mut(id).unwrap().direct_input = true;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                manager.poll();
                let session = manager.get(id).unwrap();
                if (returned && session.cli_return_ready())
                    || (!returned && session.terminal.live_snapshot().alternate_screen)
                {
                    break;
                }
                assert!(Instant::now() < deadline, "owned CLI phase did not arrive");
                std::thread::sleep(Duration::from_millis(5));
            }
            let session = manager.get_mut(id).unwrap();
            if returned {
                assert!(session.finish_cli_input());
            }
            assert!(session.is_alive());
            session.draft = "unsent race draft Ж🙂".into();
            let revision = session.input_revision();
            session.input = closed;
            if phase == "queued-command" {
                // A successful enqueue is not a successful WriteFile. The
                // untouched worker accepts this command and only then learns
                // that its ConPTY is gone, just like the first post-CLI write.
                session
                    .submit_command("Write-Output ('NEVER_' + 'REPLAY_QUEUED')")
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while session.input.check_error().is_ok() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
            } else if returned {
                assert!(session.write_interactive(b"never replay this").is_err());
            }
            // Exercise native worker failure through the protocol-reply path,
            // before the parent has actually delivered its final exit code.
            let queued_history = session.buffer().to_owned();
            session
                .ingest_output(b"\x1b[?1049l\x1b[2JBKS_RACE_TAIL\r\n\x1b[6n")
                .unwrap();
            assert!(
                session.buffer().starts_with(&queued_history),
                "known writer failure must suppress the first clear before state polling"
            );
            assert_eq!(session.state, SessionState::Closing);
            assert!(session.failure.is_none() && session.is_alive());
            assert!(session.write(b"never replay this").is_err());
            assert!(session.write_interactive(b"never replay this").is_err());
            assert_eq!(session.state, SessionState::Closing);
            let history = session.buffer().to_owned();
            session.update_state();
            session.ingest_output(b"\x1b[?1049l\x1b[2J").unwrap();
            assert!(
                session.buffer().starts_with(&history),
                "late CLI clear must not erase the journal while Closing"
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while manager.get(id).unwrap().input_revision() == revision {
                manager.poll();
                assert!(
                    Instant::now() < deadline,
                    "natural CLR status must recover after pipe closure"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            wait_for_own_shell(&mut manager, id);
            let session = manager.get_mut(id).unwrap();
            assert!(session.failure.is_none() && session.can_submit_command());
            assert_eq!(session.exit_code, None);
            assert_eq!(session.draft, "unsent race draft Ж🙂");
            assert!(session.buffer().starts_with(&history));
            assert!(!session.buffer().contains("never replay this"));
            assert!(!session.buffer().contains("NEVER_REPLAY_QUEUED"));
            session
                .submit_command("Write-Output ('BKS_RACE_AFTER_' + 'OK')")
                .unwrap();
            wait_for_own_shell(&mut manager, id);
            assert!(manager
                .get(id)
                .unwrap()
                .buffer()
                .contains("BKS_RACE_AFTER_OK"));
            let notices = manager.take_events();
            assert_eq!(
                notices
                    .iter()
                    .filter(|notice| notice.message.contains("Запущена новая оболочка"))
                    .count(),
                1
            );
            manager.close_all();
            manager.finish_cleanup();
        }
    }

    #[test]
    fn closed_pipe_deadline_is_fixed_even_without_another_writer_error() {
        let mut manager = SessionManager::new();
        let id = manager
            .create(&failfast_shell_profile(), PtySize::default())
            .unwrap();
        wait_for_own_shell(&mut manager, id);
        let session = manager.get_mut(id).unwrap();
        session.record_command_submission("opencode");
        let now = Instant::now();
        session.channel_failed(&std::io::Error::from_raw_os_error(232), "ошибка ввода", now);
        session.cancel_cli_input();
        assert!(session.cli_exit_watch.is_some());
        session.channel_failed(
            &std::io::Error::from_raw_os_error(233),
            "ошибка ввода",
            now + Duration::from_secs(1),
        );
        assert_eq!(session.pending_channel_close.as_ref().unwrap().started, now);
        assert!(session.failure.is_none() && !session.accepts_input());
        // The real writer is healthy here; expiry cannot depend on receiving
        // a duplicate error, nor may each poll extend the first deadline.
        session.update_state_at(now + CHANNEL_EXIT_GRACE + Duration::from_millis(1));
        assert_eq!(session.state, SessionState::Failed);
        assert!(session.failure.is_some());
        assert!(!session.shell_recovery_pending);
        manager.close_all();
        manager.finish_cleanup();
    }

    #[test]
    fn queued_shell_commands_do_not_extend_the_cli_return_window() {
        let mut manager = SessionManager::new();
        let id = manager
            .create(&failfast_shell_profile(), PtySize::default())
            .unwrap();
        wait_for_own_shell(&mut manager, id);
        let session = manager.get_mut(id).unwrap();
        session.record_command_submission("codex");
        let returned = Instant::now();
        session.cli_exit_watch.as_mut().unwrap().returned_at = Some(returned);
        session.cancel_cli_input();
        for command in ["Write-Output first", "Clear-Host"] {
            session.record_command_submission(command);
            let watch = session.cli_exit_watch.as_ref().unwrap();
            assert!(watch.shell_command_queued);
            assert_eq!(watch.returned_at, Some(returned));
        }
        session.channel_failed(
            &std::io::Error::from_raw_os_error(232),
            "ошибка ввода",
            returned + CLI_RETURN_RECOVERY_AGE + Duration::from_millis(1),
        );
        assert_eq!(session.state, SessionState::Failed);
        assert!(!session.shell_recovery_pending);
        manager.close_all();
        manager.finish_cleanup();
    }

    #[test]
    fn closed_pipe_does_not_recover_an_expired_return_or_a_new_shell_command() {
        for next_command in [false, true] {
            let mut manager = SessionManager::new();
            let id = manager
                .create(&failfast_shell_profile(), PtySize::default())
                .unwrap();
            wait_for_own_shell(&mut manager, id);
            let session = manager.get_mut(id).unwrap();
            session.record_command_submission("opencode");
            if next_command {
                session.record_command_submission("Write-Output ordinary");
                assert!(session.cli_exit_watch.is_none());
            } else {
                session.cli_exit_watch.as_mut().unwrap().returned_at =
                    Some(Instant::now() - CLI_RETURN_RECOVERY_AGE - Duration::from_secs(1));
            }
            session.channel_failed(
                &std::io::Error::from_raw_os_error(232),
                "ошибка ввода",
                Instant::now(),
            );
            assert_eq!(session.state, SessionState::Failed);
            assert!(!session.shell_recovery_pending);
            manager.close_all();
            manager.finish_cleanup();
        }
    }

    fn install_failfast_fixture(manager: &mut SessionManager, id: SessionId, code: u32) {
        let command = format!("function global:opencode {{ [Console]::Write([char]27+'[?1049h'+[char]27+'[2J'+[char]27+'[HBKS_OWN_ALT'); Start-Sleep -Milliseconds 80; [Console]::Write([char]27+'[?1049l'); [Environment]::Exit({}) }}", code as i32);
        manager
            .get_mut(id)
            .unwrap()
            .submit_command(&command)
            .unwrap();
        wait_for_own_shell(manager, id);
    }

    fn wait_for_own_shell(manager: &mut SessionManager, id: SessionId) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            manager.poll();
            let session = manager.get(id).unwrap();
            if session.parent_prompt_key.is_some() && session.can_submit_command() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let session = manager.get(id).unwrap();
        panic!(
            "own shell not ready: state={:?} exit={:?} {}",
            session.state,
            session.exit_code,
            session.buffer()
        );
    }

    fn wait_for_own_shell_marker(
        manager: &mut SessionManager,
        id: SessionId,
        marker: &str,
    ) -> (u32, u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            manager.poll();
            let session = manager.get(id).unwrap();
            if session.parent_prompt_key.is_some() && session.can_submit_command() {
                // The command joins two literals, so its echo cannot satisfy
                // this complete output-line boundary before it has executed.
                if let Some(byte) = session.buffer().find(&format!("\r\n{marker}\r\n")) {
                    let start = session.buffer()[..byte + 2].encode_utf16().count() as u32;
                    return (start, start + marker.encode_utf16().count() as u32 + 2);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let session = manager.get(id).unwrap();
        panic!(
            "own shell output {marker:?} not received: state={:?} ready={} journal={:?}",
            session.state,
            session.can_submit_command(),
            session
                .buffer()
                .chars()
                .skip(session.buffer().chars().count().saturating_sub(2000))
                .collect::<String>()
        );
    }

    #[test]
    fn failfast_during_explicit_cli_recovers_once_and_keeps_history_without_replaying() {
        let profile = failfast_shell_profile();
        let mut manager = SessionManager::new();
        let id = manager.create(&profile, PtySize::default()).unwrap();
        wait_for_own_shell(&mut manager, id);
        for launch in 0..2 {
            // Preserve a completed block with real fixture output. Its setup
            // is explicit: ConPTY may redraw the echoed command with a stray
            // backslash, which is separate from native recovery preservation.
            let marker = format!("BKS_RECOVERY_BASELINE_{launch}");
            manager
                .get_mut(id)
                .unwrap()
                .submit_command(&format!("echo ('BKS_RECOVERY_' + 'BASELINE_{launch}')"))
                .unwrap();
            let (saved_block, saved_end) = wait_for_own_shell_marker(&mut manager, id, &marker);
            let saved_text = {
                let session = manager.get_mut(id).unwrap();
                let units: Vec<_> = session.buffer().encode_utf16().collect();
                let prefix = String::from_utf16(&units[..saved_block as usize]).unwrap();
                session
                    .blocks
                    .execute_untracked("[completed fixture output]", &prefix);
                session.blocks.prompt_started(saved_end);
                assert_eq!(
                    session.blocks.current(saved_block),
                    Some((saved_block, Some(saved_end)))
                );
                String::from_utf16(&units[saved_block as usize..saved_end as usize]).unwrap()
            };
            assert_eq!(saved_text, format!("{marker}\r\n"));
            install_failfast_fixture(&mut manager, id, CLR_FAILFAST_EXIT);
            let session = manager.get_mut(id).unwrap();
            session.draft = "unsent Ж🙂".into();
            session.draft_selection = (2, 7);
            session.submit_command("opencode").unwrap();
            session.direct_input = true;
            session.reading = false;
            session.terminal_capture = true;
            let previous_revision = session.input_revision();
            let history = session.buffer().to_owned();
            let old_epoch = session.reading_document_epoch;
            let deadline = Instant::now() + Duration::from_secs(10);
            while manager.get(id).unwrap().direct_input && Instant::now() < deadline {
                manager.poll();
                std::thread::sleep(Duration::from_millis(10));
            }
            wait_for_own_shell(&mut manager, id);
            let session = manager.get_mut(id).unwrap();
            assert_eq!(session.profile, profile);
            assert_eq!(session.exit_code, None);
            assert!(session.is_alive() && session.accepts_input());
            assert!(!session.direct_input && !session.auto_cli_input && !session.terminal_capture);
            assert_eq!(session.draft, "unsent Ж🙂");
            assert_eq!(session.draft_selection, (2, 7));
            assert!(session.buffer().starts_with(&history));
            assert!(session.blocks.last().is_some());
            assert_eq!(
                session.blocks.current(saved_block),
                Some((saved_block, Some(saved_end))),
                "the actual pre-crash output block survives"
            );
            let units: Vec<_> = session.buffer().encode_utf16().collect();
            assert_eq!(
                String::from_utf16(&units[saved_block as usize..saved_end as usize]).unwrap(),
                saved_text
            );
            assert_eq!(session.input_revision(), previous_revision + 1);
            assert!(session.reading_document_epoch > old_epoch);
            assert!(session.tui_composer.is_none() && session.tui_menu.is_none());
            let notices = manager.take_events();
            assert_eq!(notices.len(), 1);
            assert!(notices[0].message.contains("Запущена новая оболочка"));
            let marker = format!("BKS_AFTER_RECOVERY_{launch}");
            manager
                .get_mut(id)
                .unwrap()
                .submit_command(&format!(
                    "Write-Output ('BKS_AFTER_' + 'RECOVERY_{launch}')"
                ))
                .unwrap();
            wait_for_own_shell_marker(&mut manager, id, &marker);
            assert!(manager.get(id).unwrap().buffer().contains(&marker));
            assert!(
                manager.take_events().is_empty(),
                "polling never repeats recovery"
            );
        }
        assert_eq!(manager.sessions.len(), 1);
        manager.finish_cleanup();
    }

    #[test]
    fn failfast_without_an_owned_parent_cli_and_failed_recovery_never_retry() {
        for mode in [
            "not-submitted",
            "cli-profile",
            "custom-startup",
            "other-exit",
            "spawn-failure",
        ] {
            let mut profile = failfast_shell_profile();
            if mode == "cli-profile" {
                profile.id = "cli-own-fixture".into();
            }
            if mode == "custom-startup" {
                profile
                    .args
                    .last_mut()
                    .unwrap()
                    .push_str("; Write-Output BKS_CUSTOM_STARTUP");
            }
            let mut manager = SessionManager::new();
            let id = manager.create(&profile, PtySize::default()).unwrap();
            wait_for_own_shell(&mut manager, id);
            install_failfast_fixture(
                &mut manager,
                id,
                if mode == "other-exit" {
                    7
                } else {
                    CLR_FAILFAST_EXIT
                },
            );
            let session = manager.get_mut(id).unwrap();
            if mode == "not-submitted" {
                session.write(b"opencode\r\n").unwrap();
            } else {
                session.submit_command("opencode").unwrap();
            }
            session.direct_input = true;
            session.reading = false;
            session.terminal_capture = true;
            if mode == "spawn-failure" {
                session.profile.executable = PathBuf::from(std::env::var_os("TEMP").unwrap())
                    .join("bks-no-such-shell-recovery/pwsh.exe")
                    .to_string_lossy()
                    .into_owned();
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while manager.get(id).unwrap().state != SessionState::Exited
                && Instant::now() < deadline
            {
                manager.poll();
                std::thread::sleep(Duration::from_millis(10));
            }
            let session = manager.get(id).unwrap();
            assert_eq!(session.state, SessionState::Exited, "{mode}");
            assert!(!session.direct_input && !session.terminal_capture, "{mode}");
            assert!(!session.shell_recovery_pending, "{mode}");
            let notices = manager.take_events();
            assert_eq!(notices.len(), 1, "{mode}");
            assert_eq!(
                notices[0].message.contains("Автовосстановление недоступно"),
                mode == "spawn-failure"
            );
            for _ in 0..4 {
                manager.poll();
            }
            assert!(manager.take_events().is_empty(), "{mode}");
            assert!(!manager.get(id).unwrap().is_alive());
            manager.finish_cleanup();
        }
    }

    fn literal_direct_session(marker: &str) -> TerminalSession {
        let mut profile = test_profile(marker);
        profile.executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/more.com")
            .to_string_lossy()
            .into_owned();
        profile.args.clear();
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        session.direct_input = true;
        session
    }

    #[test]
    fn changing_adapter_preserves_proven_historical_composer_rows() {
        let mut session = literal_direct_session("ADAPTER_PRESENTATION_HISTORY");
        session.resize(PtySize::new(64, 12)).unwrap();
        let seed = "ADAPTER_HISTORY_INPUT";
        let source = format!(
            "\x1b[2J\x1b[HBODY\x1b[3;1H┌{}┐\r\n│ > {seed}{}│\r\n└{}┘\x1b[4;{}H\x1b[?25h",
            "─".repeat(32),
            " ".repeat(32 - 3 - seed.len()),
            "─".repeat(32),
            5 + seed.len()
        );
        session.ingest_output(source.as_bytes()).unwrap();
        let first = session.reading_document_snapshot();
        assert_eq!(session.tui_composer.as_ref().unwrap().text, seed);
        assert!(!first.text.contains(seed));
        session
            .ingest_output(
                format!("\x1b[12;1H\r\n{}", "plain new output\r\n".repeat(24)).as_bytes(),
            )
            .unwrap();
        let before = session.reading_document_snapshot();
        assert!(session.terminal.reading_document().text.contains(seed));
        assert!(!before.text.contains(seed));
        session.set_adapter(crate::tui_adapters::AdapterKind::Kimi);
        let after = session.reading_document_snapshot();
        assert!(
            !after.text.contains(seed),
            "adapter identity must not restore the already projected historical input"
        );
        assert!(after.text.contains("plain new output"));
    }

    #[test]
    fn owned_shell_prompt_returns_after_wrapping_without_accepting_foreign_markers() {
        let mut session = literal_direct_session("OWNED_WRAPPED_PROMPT");
        let token = 0x1234_5678_9abc_def0;
        session.owned_prompt = Some(token);
        session.direct_input = false;
        let marker = |kind, token| format!("\x1b]133;{kind};BKS={token:016x}\x07");
        let owned = |text: &str| format!("{}{text}{}", marker('A', token), marker('B', token));
        session
            .ingest_output(owned("PS C:\\long\\cwd> ").as_bytes())
            .unwrap();
        assert_eq!(session.parent_prompt_key, Some(token));
        session.record_command_submission("kimi");
        session.direct_input = true;
        session
            .ingest_output(b"\x1b]133;A\x07PS C:\\long\\cwd> \x1b]133;B\x07")
            .unwrap();
        assert!(
            !session.cli_return_ready(),
            "legacy display cannot impersonate this shell"
        );
        session
            .ingest_output(
                format!(
                    "{}PS C:\\long\\cwd> {}",
                    marker('A', token + 1),
                    marker('B', token + 1)
                )
                .as_bytes(),
            )
            .unwrap();
        assert!(
            !session.cli_return_ready(),
            "another process identity cannot return this shell"
        );
        session
            .ingest_output(marker('B', token).as_bytes())
            .unwrap();
        assert!(
            !session.cli_return_ready(),
            "an end marker needs its matching start"
        );
        // ConPTY's cursor rewrites can duplicate a boundary character, insert
        // a line break and omit the trailing space in the linear journal.
        session
            .ingest_output(owned("PS C:\\long\\\r\n\\cwd>").as_bytes())
            .unwrap();
        assert!(
            session.cli_return_ready(),
            "display wrapping must not hide a live shell"
        );
        assert!(session.finish_cli_input());
        assert!(!session.direct_input);
        assert_eq!(session.adapter, crate::tui_adapters::AdapterKind::Shell);
    }

    #[test]
    fn owned_shell_prompt_identity_requires_the_unchanged_standard_profile() {
        let mut profile = failfast_shell_profile();
        assert!(standard_powershell_prompt(&profile));
        assert!(recoverable_shell_profile(&profile));
        profile.executable = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into();
        assert!(standard_powershell_prompt(&profile));
        assert!(
            !recoverable_shell_profile(&profile),
            "prompt ownership does not expand crash recovery"
        );
        profile
            .args
            .last_mut()
            .unwrap()
            .push_str("; Write-Output custom");
        assert!(!standard_powershell_prompt(&profile));
        profile.args.pop();
        assert!(!standard_powershell_prompt(&profile));
    }

    #[test]
    fn home_then_right_keeps_the_confirmed_text_instead_of_reclassifying_it_as_a_hint() {
        let bytes = include_bytes!(
            "../tests/fixtures/tui-codex-claude-composers/conpty-hidden-home-right.vt"
        );
        let mut session = literal_direct_session("HIDDEN_HOME_RIGHT");
        session.resize(PtySize::new(100, 30)).unwrap();
        session.ingest_output(&bytes[..3767]).unwrap();
        session.reading_document_snapshot();
        let mut begin = 3767;
        for (input, hidden, end, caret) in [
            ("Ж界🙂".as_bytes(), None, 4265, 4),
            (b"\x1b[H".as_slice(), Some(4486), 4561, 0),
            (b"\x1b[C".as_slice(), Some(4782), 4857, 1),
        ] {
            session.source_input.accepted(
                &session.terminal.live_snapshot(),
                session.terminal.reading_revision(),
                input,
                session.tui_composer.as_ref(),
                Instant::now(),
            );
            if let Some(hidden) = hidden {
                session.ingest_output(&bytes[begin..hidden]).unwrap();
                session.reading_document_snapshot();
                assert!(session.tui_composer.is_none());
                begin = hidden;
            }
            session.ingest_output(&bytes[begin..end]).unwrap();
            session.reading_document_snapshot();
            begin = end;
            let composer = session
                .tui_composer
                .as_ref()
                .expect("source composer survives navigation");
            assert_eq!(
                (composer.text.as_str(), composer.caret),
                ("Ж界🙂", Some(caret))
            );
        }
    }

    #[test]
    fn unchanged_menu_composer_survives_a_hidden_navigation_frame_without_input_evidence() {
        let bytes = include_bytes!(
            "../tests/fixtures/tui-codex-claude-composers/conpty-hidden-navigation.vt"
        );
        for cancel in ["none", "expire", "resize", "mode", "source-roundtrip"] {
            let mut session = literal_direct_session("HIDDEN_MENU_NAVIGATION");
            session.resize(PtySize::new(100, 30)).unwrap();
            session.ingest_output(&bytes[..3767]).unwrap();
            session.reading_document_snapshot();
            let revision = session.terminal.reading_revision();
            let frame = session.terminal.live_snapshot();
            session.source_input.accepted(
                &frame,
                revision,
                b"/",
                session.tui_composer.as_ref(),
                Instant::now(),
            );
            session.menu_observer.accepted(b"/", revision);
            session.ingest_output(&bytes[3767..3988]).unwrap();
            session.reading_document_snapshot();
            assert!(session.tui_composer.is_none());
            session.ingest_output(&bytes[3988..4131]).unwrap();
            session.reading_document_snapshot();
            assert_eq!(session.tui_composer.as_ref().unwrap().text, "/");
            assert_eq!(session.tui_menu.as_ref().unwrap().selected, 0);
            let revision = session.terminal.reading_revision();
            let frame = session.terminal.live_snapshot();
            session.source_input.accepted(
                &frame,
                revision,
                b"\x1b[B",
                session.tui_composer.as_ref(),
                Instant::now(),
            );
            session.menu_observer.accepted(b"\x1b[B", revision);
            assert!(session
                .source_input
                .evidence(&frame, revision + 1, Instant::now())
                .is_none());
            session.ingest_output(&bytes[4131..4352]).unwrap();
            session.reading_document_snapshot();
            assert!(session.tui_composer.is_none() && session.tui_menu.is_none());
            match cancel {
                "expire" => {
                    session.composer_continuity.as_mut().unwrap().1 =
                        Instant::now() - Duration::from_secs(1)
                }
                "resize" => session.resize(PtySize::new(100, 31)).unwrap(),
                "mode" => {
                    session.direct_input = false;
                    session.reading_document_snapshot();
                    session.direct_input = true;
                }
                "source-roundtrip" => {
                    session.ingest_output(b"\x1b[?1049h\x1b[?25l").unwrap();
                    session.reading_document_snapshot();
                    session.ingest_output(b"\x1b[?1049l\x1b[?25l").unwrap();
                    session.reading_document_snapshot();
                }
                _ => {}
            }
            session.ingest_output(&bytes[4352..]).unwrap();
            session.reading_document_snapshot();
            if cancel == "none" {
                let input = session.tui_composer.as_ref().unwrap();
                assert_eq!((input.text.as_str(), input.caret), ("/", Some(1)));
                assert_eq!(session.tui_menu.as_ref().unwrap().selected, 1);
            } else {
                assert!(session.tui_composer.is_none(), "cancel={cancel}");
            }
            assert!(
                session.composer_continuity.is_none(),
                "continuity proof ends with the visible frame"
            );
        }
    }

    #[test]
    fn source_input_proof_survives_only_an_authorized_hidden_cursor_transition() {
        let bytes =
            include_bytes!("../tests/fixtures/tui-codex-claude-composers/conpty-hidden-menu.vt");
        for cancel in ["none", "clear", "expire", "control", "resize"] {
            let mut session = literal_direct_session("HIDDEN_COMPOSER_PROOF");
            session.resize(PtySize::new(100, 30)).unwrap();
            session.ingest_output(&bytes[..3707]).unwrap();
            session.reading_document_snapshot();
            assert_eq!(session.tui_composer.as_ref().unwrap().text, "");
            let frame = session.terminal.live_snapshot();
            let revision = session.terminal.reading_revision();
            let now = Instant::now();
            session.source_input.accepted(
                &frame,
                revision,
                b"/",
                session.tui_composer.as_ref(),
                now,
            );
            session.menu_observer.accepted(b"/", revision);
            session.ingest_output(&bytes[3707..3928]).unwrap();
            session.reading_document_snapshot();
            assert!(session.terminal.live_snapshot().cursor.is_none());
            assert!(
                session.tui_composer.is_none(),
                "hidden source never publishes the private baseline"
            );
            match cancel {
                "clear" => session.source_input.clear(),
                "expire" => session.source_input.expire(now + Duration::from_secs(1)),
                "control" => session
                    .source_input
                    .accepted(&frame, revision, b"\x1b", None, now),
                "resize" => session.resize(PtySize::new(100, 31)).unwrap(),
                _ => {}
            }
            session.ingest_output(&bytes[3928..]).unwrap();
            session.reading_document_snapshot();
            if cancel == "none" {
                let input = session
                    .tui_composer
                    .as_ref()
                    .expect("visible source proves the accepted slash");
                assert_eq!((input.text.as_str(), input.caret), ("/", Some(1)));
                assert!(
                    session
                        .source_input
                        .confirmation(
                            &session.terminal.live_snapshot(),
                            session.terminal.reading_revision(),
                            Instant::now()
                        )
                        .is_none(),
                    "full source acknowledgement discards private proof"
                );
            } else {
                assert!(
                    session.tui_composer.is_none(),
                    "cancel={cancel} must not reuse old input proof"
                );
            }
        }
    }

    #[test]
    fn captured_sync_frames_are_independent_of_pty_chunking() {
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-long-scroll.vt");
        let marker = b"\x1b[?2026l";
        let mut reference = TerminalModel::new(110, 36);
        reference.push_bytes(&bytes[..12_435]);
        let mut begin = 12_435;
        let expected: Vec<_> = bytes
            .windows(marker.len())
            .enumerate()
            .filter(|(offset, part)| *part == marker && offset + marker.len() > 12_435)
            .map(|(offset, _)| {
                let end = offset + marker.len();
                reference.push_bytes(&bytes[begin..end]);
                begin = end;
                reference.live_snapshot()
            })
            .collect();
        for chunk_size in [4096, OUTPUT_DRAIN_BUDGET] {
            let mut model = TerminalModel::new(110, 36);
            model.push_bytes(&bytes[..12_435]);
            let mut index = 0;
            for chunk in bytes[12_435..].chunks(chunk_size) {
                let mut remaining = chunk;
                while !remaining.is_empty() {
                    let (used, complete) = model.push_bytes_until_frame(remaining);
                    remaining = &remaining[used..];
                    if complete {
                        assert_eq!(
                            model.live_snapshot(),
                            expected[index],
                            "chunk={chunk_size}, frame={index}"
                        );
                        index += 1;
                    }
                }
            }
            assert_eq!(index, expected.len());
        }
    }

    #[test]
    fn coalesced_pty_drains_retain_the_entire_long_answer_without_replayed_speech() {
        let bytes = include_bytes!("../tests/fixtures/tui-turns-opencode-long-scroll.vt");
        let mut answers = Vec::new();
        for chunk_size in [4096, OUTPUT_DRAIN_BUDGET] {
            let mut session = literal_direct_session("BKS_COALESCED_ANSWER");
            session.resize(PtySize::new(110, 36)).unwrap();
            session.reading = false;
            session.ingest_output(&bytes[..12_435]).unwrap();
            let baseline = session.reading_document_snapshot();
            session
                .turns
                .submitted("Привет, кто ты и что умеешь?", &baseline, Instant::now());
            for chunk in bytes[12_435..].chunks(chunk_size) {
                session.ingest_output(chunk).unwrap();
            }
            // UI publication is separate from the model's per-frame retention.
            // No artificial time advances while a single drain is parsed.
            let mut events = session.turns.take_events();
            let now = Instant::now();
            session.observe_tui_frame(now);
            session.observe_tui_frame(now + Duration::from_secs(1));
            events.extend(session.turns.take_events());
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, crate::tui_turns::TurnEvent::Ready { .. }))
                    .count(),
                1,
                "chunk={chunk_size}, sync={}, status={}, answer={:?}",
                session.terminal.sync_pending(),
                session.turns.status(),
                session.turns.latest_answer().map(|target| (
                    target.complete,
                    session.turns.answer_text(target.id).unwrap().len()
                ))
            );
            let target = session
                .turns
                .latest_answer()
                .expect("a readable owned answer");
            assert!(target.complete);
            let answer = session.turns.answer_text(target.id).unwrap();
            assert!(answer.starts_with("Длинный фиксированный ответ"));
            assert!(answer.ends_with("КОНЕЦ ДЛИННОГО ОТВЕТА — все 120 отмеченных строк завершены."));
            for number in 1..=120 {
                assert_eq!(
                    answer.matches(&format!("MARKER_{number:03} Ж🙂.")).count(),
                    1,
                    "chunk_size={chunk_size}, marker={number}"
                );
            }
            assert!(!answer.contains("Build ·"));
            answers.push(answer.to_owned());
        }
        assert_eq!(
            answers[0], answers[1],
            "PTY read grouping must not change the retained answer"
        );
    }

    fn accept_direct_input(session: &mut TerminalSession, data: &[u8]) {
        // more.com, or the local PowerShell function above, consumes these
        // exact bytes. Tracking must never write them a second time.
        session.write(data).unwrap();
        let generation = session.input_revision();
        session.record_direct_input(data);
        assert_eq!(session.input_revision(), generation);
    }

    fn own_parent_prompt(session: &mut TerminalSession) {
        session
            .ingest_output(b"\x1b]133;A\x07PS> \x1b]133;B\x07")
            .unwrap();
    }

    #[test]
    fn direct_shell_commands_require_a_ready_parent_and_keep_manual_mode() {
        let mut session = literal_direct_session("DIRECT_SHELL_TRACKING");
        accept_direct_input(&mut session, b"kimi\r");
        assert!(!session.auto_cli_input);
        session
            .ingest_output(b"\x1b]133;A\x07\x1b]133;B\x07")
            .unwrap();
        accept_direct_input(&mut session, b"kimi\r");
        assert!(!session.auto_cli_input);
        own_parent_prompt(&mut session);
        accept_direct_input(&mut session, b"echo own fixture\r\n");
        assert!(!session.auto_cli_input && !session.can_submit_command());
        accept_direct_input(&mut session, b"kimi\r");
        assert!(!session.auto_cli_input);
        own_parent_prompt(&mut session);
        assert!(session.direct_input && !session.auto_cli_input);
        assert!(session.can_submit_command() && !session.cli_return_ready());
        assert!(!session.finish_cli_input());
    }

    #[test]
    fn accepted_direct_bytes_survive_output_redraws_and_require_the_parent_to_return() {
        let mut session = literal_direct_session("DIRECT_BYTES");
        own_parent_prompt(&mut session);
        accept_direct_input(&mut session, b"ki");
        session
            .ingest_output(b"\r\x1b[2Kbackground output, not command text\r\n")
            .unwrap();
        accept_direct_input(&mut session, b"mi\r");
        assert!(session.auto_cli_input && !session.cli_return_ready());
        session
            .ingest_output(b"\x1b]133;A\x07OTHER CLI> \x1b]133;B\x07")
            .unwrap();
        accept_direct_input(&mut session, b"/exit\r");
        assert!(!session.cli_return_ready());
        own_parent_prompt(&mut session);
        assert!(session.cli_return_ready());
        assert!(session.finish_cli_input());
        assert!(!session.direct_input);
    }

    #[test]
    fn unknown_direct_enter_consumes_the_stale_prompt_instead_of_guessing_a_cli() {
        for data in [
            b"\x1b[Akimi\r".as_slice(),
            b"\x15kimi\r",
            b"kimx\x08i\r",
            b"kimi\n",
            b"ordinary\r\nkimi\r",
        ] {
            let mut session = literal_direct_session("UNKNOWN_DIRECT_ENTER");
            own_parent_prompt(&mut session);
            accept_direct_input(&mut session, data);
            assert!(!session.auto_cli_input && !session.can_submit_command());
            accept_direct_input(&mut session, b"kimi\r");
            assert!(!session.auto_cli_input);
            session
                .ingest_output(b"\x1b]133;A\x07OTHER CLI> \x1b]133;B\x07")
                .unwrap();
            accept_direct_input(&mut session, b"kimi\r");
            assert!(!session.auto_cli_input && !session.can_submit_command());
            own_parent_prompt(&mut session);
            accept_direct_input(&mut session, b"kimi\r");
            assert!(session.auto_cli_input);
        }
    }

    #[test]
    fn multiline_direct_batches_cannot_arm_a_cli_even_after_partial_capture() {
        for (prefix, data) in [
            (b"".as_slice(), b"kimi\r\nother".as_slice()),
            (b"".as_slice(), b"ki\rmi".as_slice()),
            (b"ki".as_slice(), b"mi\r\nother".as_slice()),
        ] {
            let mut session = literal_direct_session("MULTILINE_DIRECT_INPUT");
            own_parent_prompt(&mut session);
            accept_direct_input(&mut session, prefix);
            accept_direct_input(&mut session, data);
            assert!(!session.auto_cli_input && !session.can_submit_command());
            assert!(session.blocks.last().is_none());
            accept_direct_input(&mut session, b"kimi\r\n");
            assert!(!session.auto_cli_input);
            own_parent_prompt(&mut session);
            accept_direct_input(&mut session, b"kimi\r\n");
            assert!(session.auto_cli_input);
        }
    }

    #[test]
    fn direct_capture_does_not_cross_a_changed_source_mode_or_edited_geometry() {
        for change in 0..4 {
            let mut session = literal_direct_session("DIRECT_CAPTURE_SOURCE");
            own_parent_prompt(&mut session);
            accept_direct_input(&mut session, b"ki");
            match change {
                0 => {
                    session
                        .ingest_output(b"\x1b]133;A\x07OTHER> \x1b]133;B\x07")
                        .unwrap();
                }
                1 => {
                    session.direct_input = false;
                    session.cancel_cli_input();
                    session.direct_input = true;
                }
                2 => session.resize(PtySize { cols: 40, rows: 24 }).unwrap(),
                _ => session.profile.id = "cli-kimi".into(),
            }
            accept_direct_input(&mut session, b"mi\r");
            assert!(!session.auto_cli_input);
            session.profile.id = "test-restored-shell".into();
            own_parent_prompt(&mut session);
            accept_direct_input(&mut session, b"kimi\r");
            assert!(session.auto_cli_input);
        }
    }

    #[test]
    fn direct_capture_is_bounded_and_keeps_only_well_formed_literal_input() {
        let mut session = literal_direct_session("DIRECT_CAPTURE_BOUNDS");
        own_parent_prompt(&mut session);
        accept_direct_input(&mut session, &[b'x'; 4_097]);
        accept_direct_input(&mut session, b"\r");
        assert!(!session.auto_cli_input && session.blocks.last().is_none());
        own_parent_prompt(&mut session);
        accept_direct_input(&mut session, &[0xe7]);
        accept_direct_input(&mut session, &[0x95, 0x8c]);
        assert_eq!(
            session.direct_command_input.capture.as_deref(),
            Some("界".as_bytes())
        );
        assert!(session.can_submit_command());
        accept_direct_input(&mut session, b"\r");
        assert!(!session.auto_cli_input && session.blocks.needs_output_start());
        own_parent_prompt(&mut session);
        accept_direct_input(&mut session, b"ki\xffmi\r");
        assert!(!session.auto_cli_input);
        own_parent_prompt(&mut session);
        accept_direct_input(&mut session, b"kimi\r");
        assert!(session.auto_cli_input);
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
    fn real_powershell_repeated_commands_keep_all_output_blocks() {
        let profiles = crate::profile::ProfileManager::discover();
        let mut profile = profiles
            .get("pwsh")
            .or_else(|| profiles.get("windows-powershell"))
            .unwrap()
            .clone();
        let directory = std::env::temp_dir().join(format!(
            "bks-output-blocks-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        for name in ["one.txt", "two.txt"] {
            std::fs::write(directory.join(name), name).unwrap();
        }
        profile.working_directory = Some(directory.to_string_lossy().into_owned());
        // Only this isolated shell owns the deterministic docker substitute.
        // The regression never contacts the user's daemon or remote context.
        profile.args.last_mut().unwrap().push_str(
            "; function global:docker { [Console]::Error.WriteLine('BKS_BLOCK_TWO: failed to connect to the docker API') }; Import-Module PSReadLine; Set-PSReadLineOption -HistorySaveStyle SaveNothing",
        );
        let mut session = TerminalSession::spawn(1, &profile, PtySize::default()).unwrap();
        fn settle_prompt(session: &mut TerminalSession) {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut revision = session.revision;
            let mut quiet_since = Instant::now();
            loop {
                session.poll().unwrap();
                let ready = session.parent_prompt_key.is_some()
                    && session.can_submit_command()
                    && session.input.is_idle();
                if session.revision != revision || !ready {
                    revision = session.revision;
                    quiet_since = Instant::now();
                }
                if ready && quiet_since.elapsed() >= Duration::from_millis(250) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "isolated shell prompt did not settle"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        // OSC B precedes PSReadLine's first redraw. Exercise a settled prompt,
        // not a race between console cooked echo and editor initialization.
        // Import/setup and disabled history affect only this test process.
        settle_prompt(&mut session);
        let mut anchors = Vec::new();
        for command in ["ls", "docker ps", "ls"] {
            session.submit_command(command).unwrap();
            settle_prompt(&mut session);
            anchors.push(
                session
                    .blocks
                    .last()
                    .expect("submitted output is navigable"),
            );
        }
        assert!(
            anchors.windows(2).all(|pair| pair[0] < pair[1]),
            "three submissions must retain three distinct results: {anchors:?}"
        );
        let units: Vec<_> = session.buffer().encode_utf16().collect();
        for (index, &anchor) in anchors.iter().enumerate() {
            let line = units[anchor as usize..]
                .split(|unit| *unit == b'\n' as u16)
                .next()
                .unwrap();
            let line = String::from_utf16_lossy(line);
            assert!(
                !line.trim().is_empty(),
                "result {index} starts on a blank row"
            );
            let (_, end) = session.blocks.current(anchor).unwrap();
            let body = String::from_utf16_lossy(&units[anchor as usize..end.unwrap() as usize]);
            // PSReadLine may repeat/redraw its echo. That intentionally keeps
            // the conservative source boundary rather than discarding text.
            // Check the live integration's ownership/readability contract;
            // exact first-output anchors are covered by captured echo cases
            // and the native GUI caret/TextPattern regression.
            if index == 1 {
                assert!(body.contains("BKS_BLOCK_TWO: failed to connect to the docker API"));
                assert!(!body.contains("one.txt") && !body.contains("two.txt"));
            } else {
                assert!(
                    body.contains("one.txt") && body.contains("two.txt"),
                    "missing result {index}: {body:?}"
                );
                assert!(!body.contains("BKS_BLOCK_TWO"));
            }
            assert_eq!(
                session.blocks.navigate(true, Some(anchor)),
                Some(anchors[index.saturating_sub(1)])
            );
            assert_eq!(
                session.blocks.navigate(false, Some(anchor)),
                Some(anchors[(index + 1).min(anchors.len() - 1)])
            );
        }
        assert_eq!(session.blocks.navigate(true, None), anchors.last().copied());
        assert_eq!(
            session.blocks.navigate(false, None),
            anchors.first().copied()
        );
        drop(session);
        for name in ["one.txt", "two.txt"] {
            std::fs::remove_file(directory.join(name)).unwrap();
        }
        std::fs::remove_dir(directory).unwrap();
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
    fn idle_poll_publishes_expired_sync_invalidates_cache_and_flushes_only_protocol_input() {
        let mut session =
            TerminalSession::spawn(1, &live_profile("SYNC_IDLE"), PtySize::default()).unwrap();
        session.ingest_output(b"old\r").unwrap();
        session
            .ingest_output(b"\x1b[?2026h\x1b[2Knew\x1b[6n")
            .unwrap();
        let original = session.reading_document_snapshot();
        assert_eq!(original.text, "old");
        assert!(session.terminal.drain_responses().is_empty());
        let journal = session.buffer.clone();
        let revision = session.revision;
        let reading_revision = session.terminal.reading_revision();
        let input_revision = session.input_revision();
        assert!(!session.tui_input_ready());
        assert_eq!(
            session.write_tui(b"\r", input_revision).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        session.screen_selection = Some((0, 1));
        let later = Instant::now() + Duration::from_secs(1);
        assert_eq!(session.poll_limited_at(0, later).unwrap(), (None, 0));
        let updated = session.reading_document_snapshot();
        assert_eq!(updated.text, "new");
        assert!(!Arc::ptr_eq(&original, &updated));
        assert_eq!(session.revision, revision + 1);
        assert_eq!(session.terminal.reading_revision(), reading_revision + 1);
        assert_eq!(session.screen_selection, None);
        assert_eq!(session.input_revision(), input_revision);
        assert!(session.terminal.drain_responses().is_empty());
        assert_eq!(session.buffer, journal);
        assert!(session.failure.is_none());
        assert_eq!(session.poll_limited_at(0, later).unwrap(), (None, 0));
        assert_eq!(session.revision, revision + 1);
        assert!(Arc::ptr_eq(&updated, &session.reading_document_snapshot()));
        session.pty.request_terminate().unwrap();
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
    fn shared_tui_projection_is_mode_scoped_cached_and_keeps_raw_source_and_shell_draft() {
        let mut session = TerminalSession::spawn(
            1,
            &live_profile("SHARED_INPUT"),
            PtySize { cols: 40, rows: 12 },
        )
        .unwrap();
        session.draft = "unsent shell command".into();
        session.draft_selection = (2, 7);
        let source = format!("\x1b[2J\x1b[H\x1b[31mBODY_ONLY\x1b[0m\x1b[5;1H╭{}╮\x1b[6;1H│{}│\x1b[7;1H╰{}╯\x1b[10;1HBODY_AFTER\x1b[6;3H> Ж界e\u{301}🙂", "─".repeat(26), " ".repeat(26), "─".repeat(26));
        session.ingest_output(source.as_bytes()).unwrap();
        let raw = session.reading_document_snapshot();
        assert!(raw.text.contains("Ж界e\u{301}🙂"));
        let revision = session.terminal.reading_revision();
        session.search.set_query("Ж界e\u{301}🙂".into(), &raw);
        assert_eq!(session.search.matches().len(), 1);
        let start = raw.text[..raw.text.find("BODY_AFTER").unwrap()]
            .encode_utf16()
            .count() as u32;
        session
            .vt_state
            .capture(&raw, (start, start + 10), start, false);
        session.direct_input = true;
        let output = session.reading_document_snapshot();
        let composer = session.tui_composer.as_ref().unwrap();
        assert_eq!(composer.text, "Ж界e\u{301}🙂");
        assert_eq!(composer.caret, Some(6));
        assert!(output.text.contains("BODY_ONLY"));
        assert!(!output.text.contains("Ж界"));
        assert!(!output.text.contains('╭'));
        assert_eq!(output.styles[0].style, raw.styles[0].style);
        assert_eq!(session.terminal.reading_document().text, raw.text);
        assert_eq!(session.terminal.reading_revision(), revision);
        assert_ne!(output.revision, raw.revision);
        assert!(session.search.refresh(&output));
        assert!(session.search.matches().is_empty());
        let bookmark = session.vt_state.restore(&output);
        let units: Vec<_> = output.text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(
                &units[bookmark.selection.0 as usize..bookmark.selection.1 as usize]
            )
            .unwrap(),
            "BODY_AFTER"
        );
        assert!(Arc::ptr_eq(&output, &session.reading_document_snapshot()));
        session.direct_input = false;
        let restored = session.reading_document_snapshot();
        assert_eq!(restored.text, raw.text);
        assert_ne!(restored.revision, raw.revision);
        assert_ne!(restored.revision, output.revision);
        assert_eq!(session.terminal.reading_revision(), revision);
        assert!(session.search.refresh(&restored));
        assert_eq!(session.search.matches().len(), 1);
        assert!(session.tui_composer.is_none());
        assert_eq!(session.draft, "unsent shell command");
        assert_eq!(session.draft_selection, (2, 7));
        session.pty.request_terminate().unwrap();
    }

    #[test]
    fn captured_panel_input_ack_and_source_menu_survive_the_session_projection_cache() {
        let mut session =
            TerminalSession::spawn(1, &live_profile("PANEL_SOURCE"), PtySize::new(110, 36))
                .unwrap();
        session.direct_input = true;
        let bytes = include_bytes!("../tests/fixtures/gutter-panel-menu.vt");
        let ends = [13_222, 14_947, 15_233, 15_519, 16_567];
        let mut begin = 0;
        for (phase, end) in ends.into_iter().enumerate() {
            if phase > 0 {
                let data = match phase {
                    1 => b"\x1b[200~/\x1b[201~".as_slice(),
                    2 => b"\x1b[B",
                    3 => b"\x1b[A",
                    _ => b"\x1b",
                };
                let frame = session.terminal.live_snapshot();
                let revision = session.terminal.reading_revision();
                session.source_input.accepted(
                    &frame,
                    revision,
                    data,
                    session.tui_composer.as_ref(),
                    Instant::now(),
                );
                session.menu_observer.accepted(data, revision);
            }
            session.ingest_output(&bytes[begin..end]).unwrap();
            begin = end;
            let output = session.reading_document_snapshot();
            let input = session.tui_composer.as_ref().expect("source composer");
            assert_eq!(input.text, if phase == 0 { "" } else { "/" });
            assert!(!output.text.contains("Ask anything"));
            assert!(!output.text.contains("Big Pickle"));
            if (1..=3).contains(&phase) {
                let menu = session.tui_menu.as_ref().expect("source menu");
                assert_eq!(menu.items.len(), 10);
                assert_eq!(menu.selected, usize::from(phase == 2));
                assert!(!output.text.contains("/agents"));
            } else {
                assert!(session.tui_menu.is_none());
            }
            assert!(Arc::ptr_eq(&output, &session.reading_document_snapshot()));
        }
        session.pty.request_terminate().unwrap();
    }

    #[test]
    fn nbsp_prompt_and_foreground_menu_need_an_accepted_arrow_and_a_new_source_frame() {
        let mut session =
            TerminalSession::spawn(1, &live_profile("TEMPORAL_SOURCE"), PtySize::new(113, 32))
                .unwrap();
        session.direct_input = true;
        let source = |selected: usize| {
            let mut text =
                String::from("\x1b[4;1H────────────────────\r\n❯\u{a0}/m\r\n────────────────────");
            for (index, name) in ["alpha", "bravo", "charlie"].iter().enumerate() {
                let style = if index == selected {
                    "\x1b[22;38;2;80;170;245m"
                } else {
                    "\x1b[2;39m"
                };
                text.push_str(&format!(
                    "\x1b[{};1H{style}  /{name}    description\x1b[0m\x1b[K",
                    8 + index
                ));
            }
            text.push_str("\x1b[5;5H\x1b[?25h");
            text
        };
        session.ingest_output(source(0).as_bytes()).unwrap();
        let before = session.reading_document_snapshot();
        assert_eq!(session.tui_composer.as_ref().unwrap().text, "/m");
        assert!(session.tui_menu.is_none());
        assert!(before.text.contains("/alpha"));
        let revision = session.terminal.reading_revision();
        session.menu_observer.accepted(b"\x1b[B", revision);
        assert!(Arc::ptr_eq(&before, &session.reading_document_snapshot()));
        assert!(session.tui_menu.is_none());
        session.ingest_output(source(1).as_bytes()).unwrap();
        let confirmed = session.reading_document_snapshot();
        let menu = session
            .tui_menu
            .as_ref()
            .expect("reciprocal source style transfer");
        assert_eq!(menu.selected, 1);
        assert_eq!(menu.items.len(), 3);
        assert!(menu.items[1].label.contains("bravo"));
        assert!(!confirmed.text.contains("/alpha"));
        assert_eq!(session.tui_composer.as_ref().unwrap().text, "/m");
        session.pty.request_terminate().unwrap();
    }

    #[test]
    fn answer_reader_keeps_live_bookmarks_and_invalidates_search_without_new_pty_bytes() {
        let mut session =
            TerminalSession::spawn(1, &live_profile("ANSWER_VIEW"), PtySize::default()).unwrap();
        session.ingest_output(b"live chosen body").unwrap();
        let live = session.reading_document_snapshot();
        session.vt_state.capture(&live, (5, 11), 0, false);
        session.search.set_query("chosen".into(), &live);
        assert_eq!(session.search.matches().len(), 1);
        let key = session.display_key();
        session.set_answers_view(true);
        assert_ne!(session.display_key(), key);
        let empty = session.display_document_snapshot();
        session.search.refresh(&empty);
        assert!(session.search.matches().is_empty());
        session
            .display_reading_state()
            .capture(&empty, (0, 0), 0, false);
        session.set_answers_view(false);
        let returned = session.display_document_snapshot();
        assert!(Arc::ptr_eq(&returned, &live));
        assert_eq!(
            session.display_reading_state().restore(&returned).selection,
            (5, 11)
        );
        session.search.refresh(&returned);
        assert_eq!(session.search.matches().len(), 1);
        session.pty.request_terminate().unwrap();
    }

    #[test]
    fn native_suggestions_and_composer_are_removed_from_the_same_output_document() {
        let mut session = TerminalSession::spawn(
            1,
            &live_profile("SHARED_MENU"),
            PtySize { cols: 48, rows: 12 },
        )
        .unwrap();
        session.direct_input = true;
        let source = format!("\x1b[2J\x1b[HBODY_ONLY\x1b[3;1H→ Alpha literal (1)\x1b[4;1H  Bravo literal [2]\x1b[6;1H╭{}╮\x1b[7;1H│{}│\x1b[8;1H╰{}╯\x1b[7;3H> /", "─".repeat(26), " ".repeat(26), "─".repeat(26));
        session.ingest_output(source.as_bytes()).unwrap();
        let output = session.reading_document_snapshot();
        assert_eq!(session.tui_composer.as_ref().unwrap().text, "/");
        let menu = session.tui_menu.as_ref().unwrap();
        assert_eq!(menu.items.len(), 2);
        assert_eq!(menu.items[0].label, "Alpha literal (1)");
        assert!(output.text.contains("BODY_ONLY"));
        assert!(!output.text.contains("Alpha"));
        assert!(!output.text.contains("Bravo"));
        assert!(!output.text.contains('/'));
        let raw = session.terminal.reading_document();
        assert!(raw.text.contains("Alpha"));
        assert!(raw.text.contains('/'));
        session.pty.request_terminate().unwrap();
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
        // A shell erase produces no text but resets both the visible journal
        // and the VT screen, and still needs a native repaint.
        assert_eq!(session.ingest_output(b"\x1b[2J").unwrap(), None);
        assert_ne!(session.revision, revision);
        assert!(session.terminal.visible_text().trim().is_empty());
        assert!(session.buffer.is_empty());
        assert!(session.take_journal_reset());
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

#[cfg(test)]
#[path = "session_cli_replay_tests.rs"]
mod cli_replay_tests;
