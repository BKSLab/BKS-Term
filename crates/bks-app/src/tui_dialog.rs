//! Conservative adapter for the Codex CLI 0.164.0 startup folder-trust screen.
//!
//! This module never writes to a PTY. Only an explicit native user action may
//! call `validate_and_map`, immediately before the caller queues those bytes.

use bks_terminal::TerminalSnapshot;

pub const SUPPORTED_CODEX_VERSION: &str = "0.164.0";

const TITLE: &str = "  Folder access";
const DISCLOSURE: &str = "Trust this folder? Codex can read, edit, and run files here, subject to \
your permission settings. Folder settings can run code automatically, even without a model \
request. Continue only if you trust these files. Your trust decision will be saved.";
const FOOTER: &str = "  enter continue · esc quit";
const SANDBOX_FOOTER: &str = "  enter continue and create sandbox · esc quit";

/// Evidence supplied by the session owner, never inferred from terminal text.
/// `direct_codex_profile` means a direct native Codex executable whose installed
/// version was checked; running `codex` inside an arbitrary shell does not qualify.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub session_id: u64,
    pub screen_revision: u64,
    pub input_revision: u64,
    pub cli_version: &'a str,
    pub direct_codex_profile: bool,
    pub process_alive: bool,
    pub active_session: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogSnapshot {
    pub title: String,
    /// Includes the complete path, disclosure and footer, including any sandbox
    /// creation warning. It must be available for reading in the native dialog.
    pub context: String,
    pub options: [String; 2],
    pub selected: usize,
    pub session_id: u64,
    pub screen_revision: u64,
    pub input_revision: u64,
    pub cli_version: String,
    pub provenance: String,
    original_screen: TerminalSnapshot,
    sealed: SealedFields,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SealedFields {
    title: String,
    context: String,
    options: [String; 2],
    selected: usize,
    session_id: u64,
    screen_revision: u64,
    input_revision: u64,
    cli_version: String,
    provenance: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    UnsupportedSource,
    InactiveSession,
    ProcessExited,
    StaleState,
    InvalidOption,
    InvalidContext,
}

fn validate_source(source: Source<'_>) -> Result<(), ActionError> {
    if !source.direct_codex_profile || source.cli_version != SUPPORTED_CODEX_VERSION {
        return Err(ActionError::UnsupportedSource);
    }
    if !source.active_session {
        return Err(ActionError::InactiveSession);
    }
    if !source.process_alive {
        return Err(ActionError::ProcessExited);
    }
    Ok(())
}

/// Recognizes only a complete, current startup screen with its original text.
/// Restricted folders, trust errors, repository-root warnings, wrapped/truncated
/// paths, other CLI versions and arbitrary extra screen contents stay plain text.
pub fn recognize(source: Source<'_>, screen: &TerminalSnapshot) -> Option<DialogSnapshot> {
    validate_source(source).ok()?;
    if screen.display_offset != 0
        || screen.cols < 40
        || screen.rows < 10
        || screen.cells.len() != screen.cols.checked_mul(screen.rows)?
    {
        return None;
    }
    let lines: Vec<String> = screen
        .cells
        .chunks_exact(screen.cols)
        .map(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .trim_end_matches(' ')
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() < 6 || lines.first()?.as_str() != TITLE {
        return None;
    }
    if lines.iter().flat_map(|line| line.chars()).any(unsafe_char) {
        return None;
    }
    let path = lines.get(1)?.strip_prefix("  ")?;
    if !complete_absolute_path(path) {
        return None;
    }
    let option_start = lines.len().checked_sub(3)?;
    let disclosure_lines = lines.get(2..option_start)?;
    let disclosure = disclosure_lines
        .iter()
        .map(|line| line.strip_prefix("  "))
        .collect::<Option<Vec<_>>>()?
        .join(" ");
    if disclosure != DISCLOSURE {
        return None;
    }
    let first_selected = match lines.get(option_start)?.as_str() {
        "› 1. Trust and continue" => true,
        "  1. Trust and continue" => false,
        _ => return None,
    };
    let second_selected = match lines.get(option_start + 1)?.as_str() {
        "› 2. Quit" => true,
        "  2. Quit" => false,
        _ => return None,
    };
    if first_selected == second_selected {
        return None;
    }
    let footer = lines.last()?.as_str();
    if !matches!(footer, FOOTER | SANDBOX_FOOTER) {
        return None;
    }
    DialogSnapshot::bind_worker_result(
        source,
        screen,
        usize::from(second_selected),
        format!(
            "{}\r\n{path}\r\n\r\n{DISCLOSURE}\r\n\r\n{}\r\n{}\r\n\r\n{}",
            TITLE.trim_start(),
            lines[option_start],
            lines[option_start + 1],
            footer.trim_start()
        ),
    )
    .ok()
}

fn complete_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let absolute = path.starts_with('/')
        || path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'));
    absolute
        && path.trim() == path
        && !path.contains('…')
        && !path.contains("...")
        && !path.chars().any(unsafe_char)
}

fn unsafe_char(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

impl DialogSnapshot {
    /// Bind a successful result from the private, versioned worker to the full
    /// original host snapshot. This checks provenance and source/display
    /// correspondence, but never runs recognition on the UI thread.
    pub fn bind_worker_result(
        source: Source<'_>,
        screen: &TerminalSnapshot,
        selected: usize,
        context: String,
    ) -> Result<Self, ActionError> {
        validate_source(source)?;
        if selected >= 2 {
            return Err(ActionError::InvalidOption);
        }
        if screen.display_offset != 0
            || !(40..=512).contains(&screen.cols)
            || !(10..=256).contains(&screen.rows)
            || screen
                .cols
                .checked_mul(screen.rows)
                .is_none_or(|cells| cells > 65_536 || screen.cells.len() != cells)
            || context.is_empty()
            || context.len() > 24 * 1024
            || context
                .chars()
                .any(|character| unsafe_char(character) && !matches!(character, '\r' | '\n'))
        {
            return Err(ActionError::InvalidContext);
        }
        // Full ordered words must match the complete source, including the
        // folder path, disclosures, selected marker, choices and footer. Only
        // wrapping/padding whitespace may differ in the native reading text.
        let mut source_text = String::new();
        for row in screen.cells.chunks_exact(screen.cols) {
            for cell in row {
                if source_text.len().saturating_add(cell.text.len()) > 256 * 1024 {
                    return Err(ActionError::InvalidContext);
                }
                source_text.push_str(&cell.text);
            }
            source_text.push('\n');
        }
        if source_text
            .chars()
            .any(|character| unsafe_char(character) && !matches!(character, '\r' | '\n'))
            || !source_text
                .split_whitespace()
                .eq(context.split_whitespace())
        {
            return Err(ActionError::InvalidContext);
        }
        let sealed = SealedFields {
            title: "Folder access — Codex CLI 0.164.0".to_owned(),
            context,
            options: ["Trust and continue".to_owned(), "Quit".to_owned()],
            selected,
            session_id: source.session_id,
            screen_revision: source.screen_revision,
            input_revision: source.input_revision,
            cli_version: source.cli_version.to_owned(),
            provenance: "Локальное распознавание экрана Codex CLI 0.164.0; startup Folder access"
                .to_owned(),
        };
        Ok(Self {
            title: sealed.title.clone(),
            context: sealed.context.clone(),
            options: sealed.options.clone(),
            selected: sealed.selected,
            session_id: sealed.session_id,
            screen_revision: sealed.screen_revision,
            input_revision: sealed.input_revision,
            cli_version: sealed.cli_version.clone(),
            provenance: sealed.provenance.clone(),
            original_screen: screen.clone(),
            sealed,
        })
    }

    fn sealed_fields_unchanged(&self) -> bool {
        self.title == self.sealed.title
            && self.context == self.sealed.context
            && self.options == self.sealed.options
            && self.selected == self.sealed.selected
            && self.session_id == self.sealed.session_id
            && self.screen_revision == self.sealed.screen_revision
            && self.input_revision == self.sealed.input_revision
            && self.cli_version == self.sealed.cli_version
            && self.provenance == self.sealed.provenance
    }

    /// Call only after an explicit user choice, while holding the current
    /// session/screen state. Recheck pending PTY output before calling this;
    /// a stale or queued terminal update must not slip through to the writer.
    pub fn validate_and_map(
        &self,
        current_source: Source<'_>,
        current_screen: &TerminalSnapshot,
        option_index: usize,
    ) -> Result<&'static [u8], ActionError> {
        validate_source(current_source)?;
        if self.session_id != current_source.session_id
            || self.screen_revision != current_source.screen_revision
            || self.input_revision != current_source.input_revision
            || self.cli_version != current_source.cli_version
            || self.original_screen != *current_screen
            || !self.sealed_fields_unchanged()
        {
            return Err(ActionError::StaleState);
        }
        // Versioned upstream onboarding/keys.rs and trust_directory.rs:
        // '1' selects Trust (it never confirms), Enter confirms that selection;
        // '2' quits directly, regardless of the highlighted item.
        match option_index {
            0 => Ok(b"1\r"),
            1 => Ok(b"2"),
            _ => Err(ActionError::InvalidOption),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;

    const STARTUP: &str =
        include_str!("../tests/fixtures/tui/codex-0.164.0/startup-folder-trust-70x14.txt");
    const SANDBOX: &str =
        include_str!("../tests/fixtures/tui/codex-0.164.0/official-folder-trust-sandbox-70x14.txt");

    fn source() -> Source<'static> {
        Source {
            session_id: 11,
            screen_revision: 24,
            input_revision: 0,
            cli_version: SUPPORTED_CODEX_VERSION,
            direct_codex_profile: true,
            process_alive: true,
            active_session: true,
        }
    }

    fn terminal(text: &str) -> TerminalModel {
        let mut terminal = TerminalModel::new(70, 14);
        terminal.push_bytes(text.trim_end_matches('\n').replace('\n', "\r\n").as_bytes());
        terminal
    }

    fn screen(text: &str) -> TerminalSnapshot {
        terminal(text).snapshot()
    }

    #[test]
    fn recognizes_versioned_startup_fixture_with_complete_disclosure() {
        let snapshot = recognize(source(), &screen(STARTUP)).expect("complete supported dialog");
        assert_eq!(snapshot.selected, 0);
        assert_eq!(snapshot.options, ["Trust and continue", "Quit"]);
        assert!(snapshot.context.contains("/workspace/project"));
        assert!(snapshot.context.contains(DISCLOSURE));
        assert!(snapshot.context.starts_with("Folder access\r\n"));
        assert!(snapshot.context.contains("› 1. Trust and continue"));
        assert!(snapshot.context.contains("  2. Quit"));
        assert!(snapshot.context.ends_with("enter continue · esc quit"));
        assert_eq!(snapshot.session_id, 11);
        assert_eq!(snapshot.screen_revision, 24);
        assert_eq!(snapshot.cli_version, "0.164.0");
    }

    #[test]
    fn preserves_sandbox_warning_in_context() {
        let snapshot = recognize(source(), &screen(SANDBOX)).expect("supported sandbox variant");
        assert!(snapshot
            .context
            .ends_with("enter continue and create sandbox · esc quit"));
    }

    #[test]
    fn worker_binding_requires_complete_ordered_context_from_the_source() {
        let screen = screen(STARTUP);
        let parsed = recognize(source(), &screen).unwrap();
        let bound = DialogSnapshot::bind_worker_result(
            source(),
            &screen,
            parsed.selected,
            parsed.context.clone(),
        )
        .unwrap();
        assert_eq!(bound, parsed);
        for context in [
            parsed
                .context
                .replace("/workspace/project", "/different/folder"),
            parsed
                .context
                .replace("Your trust decision will be saved.", ""),
            format!("{}\r\nRun another command", parsed.context),
            parsed
                .context
                .replace("Trust and continue", "Continue and trust"),
            parsed
                .context
                .replace("/workspace/project", "/workspace/\u{202e}project"),
        ] {
            assert_eq!(
                DialogSnapshot::bind_worker_result(source(), &screen, 0, context),
                Err(ActionError::InvalidContext)
            );
        }
        let normalized = parsed
            .context
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(DialogSnapshot::bind_worker_result(source(), &screen, 0, normalized).is_ok());
        assert_eq!(
            DialogSnapshot::bind_worker_result(source(), &screen, 2, parsed.context),
            Err(ActionError::InvalidOption)
        );
    }

    #[test]
    fn worker_binding_rejects_unsupported_inactive_dead_and_scrolled_sources() {
        let screen = screen(STARTUP);
        let context = recognize(source(), &screen).unwrap().context;
        for (changed, expected) in [
            (
                Source {
                    cli_version: "0.165.0",
                    ..source()
                },
                ActionError::UnsupportedSource,
            ),
            (
                Source {
                    direct_codex_profile: false,
                    ..source()
                },
                ActionError::UnsupportedSource,
            ),
            (
                Source {
                    active_session: false,
                    ..source()
                },
                ActionError::InactiveSession,
            ),
            (
                Source {
                    process_alive: false,
                    ..source()
                },
                ActionError::ProcessExited,
            ),
        ] {
            assert_eq!(
                DialogSnapshot::bind_worker_result(changed, &screen, 0, context.clone()),
                Err(expected)
            );
        }
        let mut scrolled = screen;
        scrolled.display_offset = 1;
        assert_eq!(
            DialogSnapshot::bind_worker_result(source(), &scrolled, 0, context),
            Err(ActionError::InvalidContext)
        );
    }

    #[test]
    fn every_public_display_and_binding_field_is_sealed() {
        let original = screen(STARTUP);
        let parsed = recognize(source(), &original).unwrap();
        let edits: [fn(&mut DialogSnapshot); 10] = [
            |dialog| dialog.title.push_str(" changed"),
            |dialog| dialog.context.push_str(" changed"),
            |dialog| dialog.options[0].push_str(" changed"),
            |dialog| dialog.selected = 1,
            |dialog| dialog.session_id += 1,
            |dialog| dialog.screen_revision += 1,
            |dialog| dialog.input_revision += 1,
            |dialog| dialog.cli_version.push_str(" changed"),
            |dialog| dialog.provenance.push_str(" changed"),
            |dialog| dialog.options.swap(0, 1),
        ];
        for edit in edits {
            let mut changed = parsed.clone();
            edit(&mut changed);
            let matching_public_source = Source {
                session_id: changed.session_id,
                screen_revision: changed.screen_revision,
                input_revision: changed.input_revision,
                ..source()
            };
            assert_eq!(
                changed.validate_and_map(matching_public_source, &original, 0),
                Err(ActionError::StaleState)
            );
        }
    }

    #[test]
    fn bound_worker_choice_requires_the_exact_full_snapshot() {
        let original = screen(STARTUP);
        let parsed = recognize(source(), &original).unwrap();
        let mut changed = original.clone();
        changed.alternate_screen = !changed.alternate_screen;
        assert_eq!(
            parsed.validate_and_map(source(), &changed, 0),
            Err(ActionError::StaleState)
        );
        changed = original.clone();
        changed.cursor = None;
        if changed.cursor == original.cursor {
            changed.history_size += 1;
        }
        assert_eq!(
            parsed.validate_and_map(source(), &changed, 0),
            Err(ActionError::StaleState)
        );
        changed = original;
        changed.cells[0].flags.toggle(bks_terminal::Flags::INVERSE);
        assert_eq!(
            parsed.validate_and_map(source(), &changed, 0),
            Err(ActionError::StaleState)
        );
    }

    #[test]
    fn owner_input_invalidates_the_choice_even_without_a_new_screen() {
        let current = screen(STARTUP);
        let snapshot = recognize(source(), &current).unwrap();
        let changed = Source {
            input_revision: 1,
            ..source()
        };
        assert_eq!(
            snapshot.validate_and_map(changed, &current, 0),
            Err(ActionError::StaleState)
        );
    }

    #[test]
    fn unicode_path_and_second_selection_are_preserved() {
        let fixture = STARTUP
            .replace("/workspace/project", "C:\\Проекты\\界面-e\u{301}-😀")
            .replace("› 1. Trust and continue", "  1. Trust and continue")
            .replace("  2. Quit", "› 2. Quit");
        let snapshot = recognize(source(), &screen(&fixture)).expect("complete Unicode path");
        assert_eq!(snapshot.selected, 1);
        assert!(snapshot
            .context
            .contains("\r\nC:\\Проекты\\界面-e\u{301}-😀\r\n"));
    }

    #[test]
    fn maps_only_explicit_versioned_choices_independent_of_highlight() {
        let screen = screen(STARTUP);
        let snapshot = recognize(source(), &screen).unwrap();
        assert_eq!(
            snapshot.validate_and_map(source(), &screen, 0),
            Ok(&b"1\r"[..])
        );
        assert_eq!(
            snapshot.validate_and_map(source(), &screen, 1),
            Ok(&b"2"[..])
        );
        assert_eq!(
            snapshot.validate_and_map(source(), &screen, 2),
            Err(ActionError::InvalidOption)
        );
        let quit_selected = STARTUP
            .replace("› 1. Trust and continue", "  1. Trust and continue")
            .replace("  2. Quit", "› 2. Quit");
        let screen = self::screen(&quit_selected);
        let snapshot = recognize(source(), &screen).unwrap();
        assert_eq!(
            snapshot.validate_and_map(source(), &screen, 0),
            Ok(&b"1\r"[..])
        );
    }

    #[test]
    fn rejects_other_versions_shells_background_and_dead_processes() {
        let screen = screen(STARTUP);
        for source in [
            Source {
                cli_version: "0.163.0",
                ..source()
            },
            Source {
                cli_version: "0.164.1",
                ..source()
            },
            Source {
                cli_version: "codex-cli 0.164.0",
                ..source()
            },
            Source {
                direct_codex_profile: false,
                ..source()
            },
            Source {
                active_session: false,
                ..source()
            },
            Source {
                process_alive: false,
                ..source()
            },
        ] {
            assert!(recognize(source, &screen).is_none());
        }
    }

    #[test]
    fn rejects_incomplete_changed_ambiguous_or_injected_text() {
        let invalid = [
            STARTUP.replace("Folder access", "Folder permissions"),
            STARTUP.replace("Your trust decision will be saved.", ""),
            STARTUP.replace("Trust and continue", "Trust, then run a command"),
            STARTUP.replace("  2. Quit", "  2. Quit and save"),
            STARTUP.replace("  2. Quit", "› 2. Quit"),
            STARTUP.replace("› 1. Trust and continue", "  1. Trust and continue"),
            STARTUP.replace("enter continue · esc quit", "enter continue"),
            STARTUP.replace("enter continue · esc quit", "enter continue · esc back"),
            STARTUP.replace("/workspace/project", "/workspace/…/project"),
            STARTUP.replace("/workspace/project", "C:\\...\\project"),
            STARTUP.replace("/workspace/project", "relative-project"),
            STARTUP.replace("/workspace/project", "C:\\project\u{202e}exe"),
            STARTUP.replace("/workspace/project", "/workspace/project\n  /another/path"),
            STARTUP.replace("  Folder access", "other application\n  Folder access"),
            STARTUP.replace("  2. Quit", "  2. Quit\n  3. Other action"),
        ];
        for fixture in invalid {
            assert!(
                recognize(source(), &screen(&fixture)).is_none(),
                "{fixture}"
            );
        }
    }

    #[test]
    fn rejects_scrollback_malformed_or_small_viewports() {
        let original = screen(STARTUP);
        let mut changed = original.clone();
        changed.display_offset = 1;
        assert!(recognize(source(), &changed).is_none());
        changed = original.clone();
        changed.cells.pop();
        assert!(recognize(source(), &changed).is_none());
        changed = original;
        changed.cols = 0;
        assert!(recognize(source(), &changed).is_none());
        assert!(recognize(source(), &TerminalModel::new(39, 14).snapshot()).is_none());
    }

    #[test]
    fn old_dialog_in_history_is_never_recognized_after_terminal_overwrite() {
        let mut terminal = terminal(STARTUP);
        assert!(recognize(source(), &terminal.snapshot()).is_some());
        terminal.push_bytes(b"\x1b[2J\x1b[Hordinary CLI output");
        assert!(recognize(source(), &terminal.snapshot()).is_none());
        terminal = self::terminal(STARTUP);
        for _ in 0..20 {
            terminal.push_bytes(b"\r\nnormal output");
        }
        assert!(terminal.snapshot().history_size > 0);
        assert!(recognize(source(), &terminal.snapshot()).is_none());
    }

    #[test]
    fn ansi_repaint_is_parsed_from_current_cells_not_raw_output() {
        let mut terminal = terminal(STARTUP);
        // The actual footer is on row 12 of the rendered fixture.
        terminal.push_bytes(b"\x1b[12;1H\x1b[2K  different state");
        assert!(recognize(source(), &terminal.snapshot()).is_none());
        terminal.push_bytes(b"\x1b[12;1H\x1b[2K  enter continue \xc2\xb7 esc quit");
        assert!(recognize(source(), &terminal.snapshot()).is_some());
    }

    #[test]
    fn invalidates_actions_after_session_revision_screen_or_option_change() {
        let original = screen(STARTUP);
        let snapshot = recognize(source(), &original).unwrap();
        for changed_source in [
            Source {
                session_id: 12,
                ..source()
            },
            Source {
                screen_revision: 25,
                ..source()
            },
        ] {
            assert_eq!(
                snapshot.validate_and_map(changed_source, &original, 0),
                Err(ActionError::StaleState)
            );
        }
        for changed_text in [
            STARTUP.replace("/workspace/project", "/workspace/other"),
            STARTUP.replace("Trust and continue", "Run a command"),
            STARTUP
                .replace("› 1. Trust and continue", "  1. Trust and continue")
                .replace("  2. Quit", "› 2. Quit"),
        ] {
            assert_eq!(
                snapshot.validate_and_map(source(), &screen(&changed_text), 0),
                Err(ActionError::StaleState)
            );
        }
        let mut altered_view = snapshot.clone();
        altered_view.options[0] = "Something else".to_owned();
        assert_eq!(
            altered_view.validate_and_map(source(), &original, 0),
            Err(ActionError::StaleState)
        );
    }

    #[test]
    fn actions_reject_background_exit_and_version_change_without_producing_keys() {
        let screen = screen(STARTUP);
        let snapshot = recognize(source(), &screen).unwrap();
        for (changed_source, error) in [
            (
                Source {
                    active_session: false,
                    ..source()
                },
                ActionError::InactiveSession,
            ),
            (
                Source {
                    process_alive: false,
                    ..source()
                },
                ActionError::ProcessExited,
            ),
            (
                Source {
                    cli_version: "0.165.0",
                    ..source()
                },
                ActionError::UnsupportedSource,
            ),
            (
                Source {
                    direct_codex_profile: false,
                    ..source()
                },
                ActionError::UnsupportedSource,
            ),
        ] {
            assert_eq!(
                snapshot.validate_and_map(changed_source, &screen, 0),
                Err(error)
            );
            assert_eq!(
                snapshot.validate_and_map(changed_source, &screen, 1),
                Err(error)
            );
        }
    }
}
