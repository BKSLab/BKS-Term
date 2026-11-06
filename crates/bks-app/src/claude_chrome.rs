//! Claude's recorded status rows are chrome, never answer roles or completion
//! evidence. Match their source geometry and coherent muted ink before hiding.

use crate::tui_projection::Composer;
use bks_terminal::{Color, ReadingDocument, TerminalSnapshot};

pub(crate) fn metadata(
    frame: &TerminalSnapshot,
    document: &ReadingDocument,
    composer: Option<&Composer>,
    row: i32,
    start: u32,
    text: &str,
) -> bool {
    let Some(input) = composer.filter(|input| {
        !input.protected
            && input.matches_source(frame)
            && input.first_column == 0
            && input.last_column.checked_add(1) == Some(frame.cols)
            && input.first_row < input.last_row
            && input.last_row < frame.rows
    }) else {
        return false;
    };
    if row < 0
        || row as usize >= input.first_row
        || frame.cols < 8
        || [input.first_row, input.last_row].iter().any(|row| {
            frame
                .cells
                .get(row * frame.cols..(row + 1) * frame.cols)
                .is_none_or(|cells| cells.iter().any(|cell| cell.text != "─"))
        })
    {
        return false;
    }
    let trimmed = text.trim();
    let leading = text.len() - text.trim_start().len();
    let effort = trimmed.strip_prefix("● ").unwrap_or(trimmed);
    let effort = effort
        .strip_suffix(" · /effort")
        .is_some_and(|level| matches!(level, "low" | "medium" | "high" | "max" | "auto"))
        && row as usize + 1 == input.first_row
        && text[..leading].chars().count() >= frame.cols / 2;
    let elapsed = leading == 0 && elapsed_annotation(trimmed);
    if !effort && !elapsed {
        return false;
    }
    let first = start + text[..leading].encode_utf16().count() as u32;
    let Some(style) = document
        .styles
        .iter()
        .find(|span| span.start <= first && first < span.end)
        .map(|span| span.style)
    else {
        return false;
    };
    let muted = matches!(style.foreground, Color::Spec(rgb)
        if rgb.r == rgb.g && rgb.g == rgb.b && (64..=192).contains(&rgb.r));
    if !style.flags.is_empty() || !muted {
        return false;
    }
    let mut at = first;
    for ch in trimmed.chars() {
        let end = at + ch.len_utf16() as u32;
        // Claude prints the separator after its spinner in default ink.
        // Every visible glyph must still share the recorded muted style.
        if !ch.is_whitespace()
            && (at..end).any(|offset| {
                document
                    .styles
                    .iter()
                    .find(|span| span.start <= offset && offset < span.end)
                    .is_none_or(|span| span.style != style)
            })
        {
            return false;
        }
        at = end;
    }
    true
}

fn elapsed_annotation(text: &str) -> bool {
    let Some(label) = text
        .strip_prefix(['✻', '✽', '✶', '✳', '✢', '✦'])
        .and_then(|text| text.strip_prefix(' '))
    else {
        return false;
    };
    let Some((verb, duration)) = label.split_once(" for ") else {
        return false;
    };
    if verb.is_empty() || verb.chars().count() > 40 || !verb.chars().all(char::is_alphabetic) {
        return false;
    }
    let number = |text: &str, unit: &str| {
        text.strip_suffix(unit).is_some_and(|value| {
            !value.is_empty()
                && value.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                && value.parse::<f64>().is_ok_and(|number| number.is_finite())
        })
    };
    let parts = duration.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [value] => number(value, "ms") || number(value, "s"),
        [minutes, seconds] => number(minutes, "m") && number(seconds, "s"),
        [hours, minutes, seconds] => {
            number(hours, "h") && number(minutes, "m") && number(seconds, "s")
        }
        _ => false,
    }
}
