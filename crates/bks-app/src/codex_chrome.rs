//! Recorded Codex pending-message previews. Geometry and renderer ink prove
//! these source blocks; identical prose elsewhere remains ordinary content.

use bks_terminal::{Flags, ReadingDocument, ReadingLine, TerminalSnapshot};

use crate::tui_projection::Composer;

type Line<'a> = (&'a ReadingLine, &'a str);

pub(crate) fn metadata(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    composer: Option<&Composer>,
    row: i32,
    _start: u32,
    text: &str,
) -> bool {
    if !text.starts_with("• Queued follow-up inputs")
        && !text.starts_with("• Messages to be submitted after next tool call")
        && !text.starts_with("  ↳ ")
        && !text.starts_with("    ")
    {
        return false;
    }
    let input = composer.filter(|input| {
        !input.protected
            && input.matches_source(frame)
            && input.adapter() == crate::tui_adapters::AdapterKind::Codex
            && input.first_column == 0
            && input.last_column.checked_add(1) == Some(frame.cols)
            && input.first_row.saturating_add(2) <= input.last_row
            && input.last_row < frame.rows
    });
    let before = match (input, composer) {
        (Some(input), _) => input.first_row as i32,
        (None, None) => match empty_source_field(frame) {
            Some(before) => before,
            None => return false,
        },
        _ => return false,
    };
    if row < 0 || row >= before || before < 2 {
        return false;
    }
    let lines: Vec<_> = document
        .lines
        .iter()
        .zip(document.text.split("\r\n"))
        .collect();
    let mut before = before;
    let mut matched = false;
    if let Some((hint, text)) = line_at(&lines, before - 1) {
        if text == "    shift+← edit last queued message"
            && flags(document, hint.start + 4, "shift+←", Flags::BOLD)
            && flags(
                document,
                hint.start + 12,
                "edit last queued message",
                Flags::DIM,
            )
        {
            let Some(first) = block_start(document, &lines, before - 2, true) else {
                return false;
            };
            matched = (first..before).contains(&row);
            before = first;
            if line_at(&lines, before - 1).is_some_and(|(_, text)| text.is_empty()) {
                before -= 1;
            }
        }
    }
    if let Some(first) = block_start(document, &lines, before - 1, false) {
        matched |= (first..before).contains(&row);
    }
    matched
}

// During Codex's synchronized redraw its cursor can temporarily leave the
// editor. The complete empty placeholder still proves this read-only chrome
// slot: bold source prompt, dim exact placeholder, padded by two blank rows.
// This evidence never supplies an editable field or a lifecycle boundary.
fn empty_source_field(frame: &TerminalSnapshot) -> Option<i32> {
    if frame.cols < 30
        || frame.rows < 3
        || frame.cols.checked_mul(frame.rows) != Some(frame.cells.len())
    {
        return None;
    }
    let blank = |row: usize| {
        frame.cells[row * frame.cols..(row + 1) * frame.cols]
            .iter()
            .all(|cell| cell.text.trim().is_empty())
    };
    (1..frame.rows - 1).find_map(|row| {
        let cells = &frame.cells[row * frame.cols..(row + 1) * frame.cols];
        let text: String = cells.iter().map(|cell| cell.text.as_str()).collect();
        (text.trim_end() == "› Ask Codex to do anything"
            && cells[0].flags == Flags::BOLD
            && cells[2..26].iter().all(|cell| cell.flags == Flags::DIM)
            && blank(row - 1)
            && blank(row + 1))
        .then_some(row as i32 - 1)
    })
}

fn line_at<'a>(lines: &[Line<'a>], row: i32) -> Option<Line<'a>> {
    lines.iter().copied().find(|(line, _)| line.line == row)
}

fn block_start(
    document: &ReadingDocument,
    lines: &[Line<'_>],
    mut row: i32,
    queued: bool,
) -> Option<i32> {
    let last = row;
    let mut entries = 0;
    while row >= 0 && last - row < 64 {
        let (line, text) = line_at(lines, row)?;
        let text = text.trim_end();
        if heading(document, line.start, text, queued) {
            return (entries > 0).then_some(row);
        }
        let arrow = text.starts_with("  ↳ ");
        let payload = text
            .strip_prefix("  ↳ ")
            .or_else(|| text.strip_prefix("    "))?;
        let ink = if queued {
            Flags::DIM | Flags::ITALIC
        } else {
            Flags::DIM
        };
        if payload.is_empty()
            || (arrow && !flags(document, line.start + 2, "↳", Flags::DIM))
            || !flags(document, line.start + 4, payload, ink)
        {
            return None;
        }
        entries += usize::from(arrow);
        row -= 1;
    }
    None
}

fn heading(document: &ReadingDocument, start: u32, text: &str, queued: bool) -> bool {
    let label = if queued {
        "Queued follow-up inputs"
    } else {
        "Messages to be submitted after next tool call"
    };
    let suffix = if queued {
        ""
    } else {
        " (press esc to interrupt and send immediately)"
    };
    if text != format!("• {label}{suffix}")
        || !flags(document, start, "•", Flags::DIM)
        || !flags(document, start + 2, label, Flags::empty())
    {
        return false;
    }
    if queued {
        return true;
    }
    let start = start + 2 + label.encode_utf16().count() as u32;
    flags(document, start, " (press ", Flags::DIM)
        && flags(document, start + 8, "esc", Flags::BOLD)
        && flags(
            document,
            start + 11,
            " to interrupt and send immediately)",
            Flags::DIM,
        )
}

fn flags(document: &ReadingDocument, start: u32, text: &str, flags: Flags) -> bool {
    let end = start + text.encode_utf16().count() as u32;
    (start..end).all(|offset| {
        document
            .styles
            .iter()
            .find(|span| span.start <= offset && offset < span.end)
            .is_some_and(|span| span.style.flags == flags)
    })
}
