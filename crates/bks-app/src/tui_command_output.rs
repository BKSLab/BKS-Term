//! Retain a source-visible local command result after an explicit submission.
//! This is a reading snapshot, never a model turn, completion signal or action.

use std::time::{Duration, Instant};

use bks_terminal::{Flags, ReadingDocument, TerminalSnapshot};

use super::{RichText, SourceLine};
use crate::tui_adapters::AdapterKind;
use crate::tui_projection::Composer;
use crate::tui_suggestions::Menu;

const STABLE_FOR: Duration = Duration::from_millis(150);
const WAIT_FOR: Duration = Duration::from_secs(15);
const MAX_BODY: usize = 128 * 1024;

#[cfg(test)]
#[path = "tui_command_output_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct Capture {
    pending: Option<Pending>,
    completion: Option<(String, (usize, usize, bool), Instant)>,
}

struct Pending {
    command: String,
    revision: u64,
    geometry: (usize, usize, bool),
    started: Instant,
    stable: Option<(u64, Instant)>,
    id: Option<u64>,
}

impl Capture {
    #[allow(clippy::too_many_arguments)]
    pub fn accepted(
        &mut self,
        bytes: &[u8],
        frame: &TerminalSnapshot,
        document: &ReadingDocument,
        composer: Option<&Composer>,
        menu: Option<&Menu>,
        now: Instant,
    ) {
        // A later explicit key seals any captured result and cancels an
        // unconfirmed one. Typing cannot append the next answer to this block.
        let completion = self.completion.take();
        self.pending = None;
        if bytes != b"\r" && bytes != b"\t" {
            return;
        }
        let input = composer.filter(|input| {
            input.adapter() == AdapterKind::Codex
                && !input.protected
                && !input.is_partial()
                && input.matches_source(frame)
        });
        let geometry = (frame.cols, frame.rows, frame.alternate_screen);
        let command = input.and_then(|input| {
            let typed = input.text.trim();
            if !typed.starts_with('/') || typed.contains(['\r', '\n']) || typed.len() > 1_024 {
                return None;
            }
            let command = if let Some(menu) = menu {
                let token = menu
                    .items
                    .get(menu.selected)?
                    .label
                    .split_whitespace()
                    .next()?;
                if !token.starts_with(typed) {
                    return None;
                }
                token
            } else {
                typed
            };
            (command.len() >= 2 && command.len() <= 1_024).then(|| command.to_owned())
        });
        if bytes == b"\t" {
            if menu.is_some() {
                self.completion = command.map(|command| (command, geometry, now));
            }
            return;
        }
        let command = command.or_else(|| {
            let (command, original, started) = completion?;
            (composer.is_none()
                && menu.is_none()
                && original == geometry
                && now.saturating_duration_since(started) <= Duration::from_secs(5)
                && completed_source_field(frame, &command))
            .then_some(command)
        });
        let Some(command) = command else {
            return;
        };
        self.pending = Some(Pending {
            command,
            revision: document.revision,
            geometry,
            started: now,
            stable: None,
            id: None,
        });
    }

    pub fn observe(
        &mut self,
        frame: &TerminalSnapshot,
        document: &ReadingDocument,
        composer: Option<&Composer>,
        menu_open: bool,
        now: Instant,
    ) -> Option<(Option<u64>, String, RichText)> {
        let pending = self.pending.as_mut()?;
        if (frame.cols, frame.rows, frame.alternate_screen) != pending.geometry
            || now.saturating_duration_since(pending.started) > WAIT_FOR
        {
            self.pending = None;
            return None;
        }
        let input = composer.filter(|input| {
            input.adapter() == AdapterKind::Codex && !input.protected && input.matches_source(frame)
        });
        let Some(input) = input else {
            pending.stable = None;
            return None;
        };
        if menu_open || !input.text.is_empty() || document.revision == pending.revision {
            pending.stable = None;
            return None;
        }
        // A cleared source editor acknowledges the explicit command. Repeated
        // /status can reuse exactly the same cells: neither echo count nor new
        // literal slash bytes are required. Only actual visible text is saved.
        let Some(body) = body(document, input.first_row, &pending.command) else {
            pending.stable = None;
            return None;
        };
        let signature = body.signature();
        let (previous, since) = *pending.stable.get_or_insert((signature, now));
        if previous != signature {
            pending.stable = Some((signature, now));
            return None;
        }
        if now.saturating_duration_since(since) < STABLE_FOR {
            return None;
        }
        Some((pending.id, pending.command.clone(), body))
    }

    pub fn committed(&mut self, id: u64) {
        if let Some(pending) = self.pending.as_mut() {
            pending.id = Some(id);
        }
    }
}

// Tab can close Codex's menu in a frame whose ordinary composer projection is
// temporarily unavailable. A recent confirmed menu token is only a hint: the
// exact completed command, prompt ink and caret must also be visible now.
fn completed_source_field(frame: &TerminalSnapshot, command: &str) -> bool {
    let Some(cursor) = frame.cursor.filter(|cursor| cursor.row < frame.rows) else {
        return false;
    };
    if frame.cells.len() != frame.cols.saturating_mul(frame.rows) {
        return false;
    }
    let row = &frame.cells[cursor.row * frame.cols..(cursor.row + 1) * frame.cols];
    let text: String = row.iter().map(|cell| cell.text.as_str()).collect();
    row.first().is_some_and(|cell| cell.flags.contains(Flags::BOLD))
        && row.iter().all(|cell| !cell.flags.contains(Flags::HIDDEN))
        && text.trim_end() == format!("› {command}")
        // Codex appends one space when completing a slash command with Tab.
        && (command.encode_utf16().count() + 2..=command.encode_utf16().count() + 3)
            .contains(&cursor.column)
}

fn body(document: &ReadingDocument, before: usize, command: &str) -> Option<RichText> {
    let lines: Vec<_> = document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .filter(|(line, _)| line.line < before as i32)
        .collect();
    let mut code = false;
    let mut echo = None;
    for (index, (_, text)) in lines.iter().enumerate() {
        if text.trim_start().starts_with("```") || text.trim_start().starts_with("~~~") {
            code = !code;
        }
        if !code && text.trim_end() == command {
            echo = Some(index);
        }
    }
    let after = echo? + 1;
    let first = (after..lines.len()).find(|&index| !lines[index].1.trim().is_empty())?;
    let last = (first..lines.len()).rfind(|&index| !lines[index].1.trim().is_empty())?;
    let mut body = RichText::default();
    for (line, text) in &lines[first..=last] {
        if text.starts_with(['›', '•']) {
            return None; // A later request/answer is not this command's output.
        }
        body.join(&RichText::from_line(
            document,
            &SourceLine {
                text,
                start: line.start,
                row: line.line,
            },
            0,
        ));
    }
    body.keep_tail_with_notice(MAX_BODY, "[Начало вывода удалено]");
    Some(body)
}
