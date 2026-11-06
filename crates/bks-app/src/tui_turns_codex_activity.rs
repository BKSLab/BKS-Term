//! Codex 0.164 source-rendered activity cells, not assistant prose.
//! Renderer contract: exec marker/title are bold, output gutter is dim;
//! assistant message markers are dim. Text alone never proves a tool role.
//! Upstream: codex-rs/tui/src/exec_cell/{render,compact}.rs at rust-v0.164.0.

use super::{
    duration_token, Composer, Flags, PhaseKind, ReadingDocument, SourceLine, TerminalSnapshot,
};

#[cfg(test)]
#[path = "tui_turns_codex_activity_tests.rs"]
mod tests;

pub(super) fn outer(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|marker| marker == '•' || ('\u{2800}'..='\u{28ff}').contains(&marker))
        && chars.next() == Some(' ')
}

fn ink(document: &ReadingDocument, start: u32, units: u32, wanted: Flags) -> bool {
    (start..start + units).all(|at| {
        document.styles.iter().any(|span| {
            span.start <= at
                && at < span.end
                && span.style.flags.contains(wanted)
                && (!wanted.contains(Flags::BOLD) || !span.style.flags.contains(Flags::DIM))
                && !span.style.flags.intersects(Flags::HIDDEN | Flags::ITALIC)
        })
    })
}

/// Codex can clip the cancel hint after a long visible work title. The timed
/// dim suffix, bold Esc and its placement above the source editor prove this
/// UI boundary; a timer or an elapsed time in ordinary prose is insufficient.
pub(super) fn busy(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    line: &SourceLine<'_>,
    composer: Option<&Composer>,
) -> bool {
    let Some(input) = composer.filter(|input| !input.protected && input.matches_source(frame))
    else {
        return false;
    };
    if line.text.starts_with(char::is_whitespace) {
        return false;
    }
    if line.row < 0
        || !(input.first_row.saturating_sub(3) as i32..input.first_row as i32).contains(&line.row)
    {
        return false;
    }
    let Some(timer) = line.text.rfind(" (") else {
        return false;
    };
    let suffix = &line.text[timer + 1..];
    let Some((duration, hint)) = suffix
        .strip_prefix('(')
        .and_then(|suffix| suffix.split_once(" • esc"))
    else {
        return false;
    };
    if !duration_token(duration) {
        return false;
    }
    let full = matches!(hint, " to interrupt)" | " to cancel)");
    let clipped = hint.strip_suffix('…').is_some_and(|prefix| {
        prefix.len() >= 4
            && [" to interrupt)", " to cancel)"]
                .iter()
                .any(|known| known.starts_with(prefix))
    });
    if !full && !clipped {
        return false;
    }
    let start = line.start + line.text[..timer + 1].encode_utf16().count() as u32;
    let before_esc = 1 + duration.encode_utf16().count() as u32 + 3;
    ink(document, start, before_esc, Flags::DIM)
        && ink(document, start + before_esc, 3, Flags::BOLD)
        && ink(
            document,
            start + before_esc + 3,
            hint.encode_utf16().count() as u32,
            Flags::DIM,
        )
        && (!clipped
            || document
                .cells
                .iter()
                .rev()
                .find(|cell| {
                    cell.start >= line.start
                        && cell.end <= line.start + line.text.encode_utf16().count() as u32
                })
                .is_some_and(|cell| cell.column + cell.width == frame.cols))
}

pub(super) fn prefix(
    document: &ReadingDocument,
    lines: &[SourceLine<'_>],
    index: usize,
    busy: bool,
) -> Option<(PhaseKind, usize)> {
    let line = &lines[index];
    if !outer(line.text) {
        return None;
    }
    let marker = line.text.chars().next()?;
    let skipped = marker.len_utf8() + 1;
    let body = &line.text[skipped..];
    let title = body.split_whitespace().next()?;
    if !ink(
        document,
        line.start + 2,
        title.encode_utf16().count() as u32,
        Flags::BOLD,
    ) {
        return None;
    }
    let marker_bold = ink(document, line.start, 1, Flags::BOLD);
    let gutter = lines
        .iter()
        .skip(index + 1)
        .take(32)
        .take_while(|next| !outer(next.text))
        .find(|next| !next.text.trim().is_empty())
        .is_some_and(|next| {
            next.text.starts_with("  └ ") && ink(document, next.start + 2, 1, Flags::DIM)
        });
    match title {
        // A Markdown answer can start with **Ran**. Its dim message bullet
        // cannot pass the source exec-marker proof, even with similar prose.
        "Ran" if body.starts_with("Ran ") && marker_bold => Some((PhaseKind::Tool, skipped)),
        "Running"
            if body.starts_with("Running ") && busy && (marker != '•' || marker_bold || gutter) =>
        {
            Some((PhaseKind::Tool, skipped))
        }
        "Explored" | "Exploring" if gutter => Some((PhaseKind::Search, skipped)),
        _ => None,
    }
}
