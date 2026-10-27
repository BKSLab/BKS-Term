//! Retention for the legacy command journal. VT has its own history limit.

// Classic Win32 EDIT relayout becomes a visible UI stall for megabyte-sized
// Unicode documents. Keep this accessible control below the measured limit.
pub const JOURNAL_MAX_BYTES: usize = 512 * 1024;
pub const JOURNAL_RETAIN_BYTES: usize = 384 * 1024;
pub const JOURNAL_MAX_LINES: usize = 6_000;
const JOURNAL_RETAIN_LINES: usize = 4_000;
const LINE_BOUNDARY_LOOKAHEAD: usize = 64 * 1024;

/// Keep complete lines where practical, but also bound a single huge line.
/// Returns removed UTF-16 units so native selection and command blocks rebase
/// against exactly the same prefix. Never removes half a UTF-8 codepoint/CRLF.
pub fn trim(text: &mut String, lines: usize) -> u32 {
    trim_to(
        text,
        lines,
        JOURNAL_MAX_BYTES,
        JOURNAL_RETAIN_BYTES,
        JOURNAL_MAX_LINES,
        JOURNAL_RETAIN_LINES,
    )
}

fn trim_to(
    text: &mut String,
    lines: usize,
    max_bytes: usize,
    retain_bytes: usize,
    max_lines: usize,
    retain_lines: usize,
) -> u32 {
    if text.len() <= max_bytes && lines <= max_lines {
        return 0;
    }
    let mut cut = if text.len() > max_bytes {
        text.len().saturating_sub(retain_bytes)
    } else {
        0
    };
    if lines > max_lines {
        let drop_lines = lines.saturating_sub(retain_lines);
        if let Some((index, _)) = text
            .bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .nth(drop_lines.saturating_sub(1))
        {
            cut = cut.max(index + 1);
        }
    }
    // Searching bytes is safe even when the desired cut is inside UTF-8.
    if cut < text.len() && cut > 0 && text.as_bytes()[cut - 1] != b'\n' {
        if let Some(next) = text.as_bytes()[cut..]
            .iter()
            .take(LINE_BOUNDARY_LOOKAHEAD)
            .position(|byte| *byte == b'\n')
        {
            cut += next + 1;
        }
    }
    while cut < text.len() && !text.is_char_boundary(cut) {
        cut += 1;
    }
    if cut < text.len()
        && text.as_bytes()[cut] == b'\n'
        && cut > 0
        && text.as_bytes()[cut - 1] == b'\r'
    {
        cut += 1;
    }
    let removed = text[..cut].encode_utf16().count() as u32;
    text.drain(..cut);
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_complete_lines_and_reports_original_utf16_offsets() {
        let original = "старое 😀\r\nвторая\r\nkeep 😀\r\nend";
        let mut text = original.to_owned();
        let removed = trim_to(&mut text, 3, 30, 18, 20, 15);
        // Compare UTF-16 directly: surrogate pairs must survive the cut.
        assert_eq!(
            text.encode_utf16().collect::<Vec<_>>(),
            original
                .encode_utf16()
                .skip(removed as usize)
                .collect::<Vec<_>>()
        );
        assert!(text.len() <= 18);
        assert!(text.starts_with("keep") || text == "end");
        assert_eq!(removed, 19);
    }

    #[test]
    fn line_limit_also_bounds_many_short_lines_and_empty_input_is_unchanged() {
        let mut text = "a\r\nb\r\nc\r\nd\r\n".to_owned();
        assert_eq!(trim_to(&mut text, 4, 100, 80, 3, 2), 6);
        assert_eq!(text, "c\r\nd\r\n");
        assert_eq!(trim_to(&mut text, 2, 100, 80, 3, 2), 0);
        assert_eq!(trim(&mut String::new(), 0), 0);
    }

    #[test]
    fn huge_unbroken_unicode_line_is_bounded_without_splitting_a_codepoint() {
        let mut text = "😀中e\u{301}".repeat(JOURNAL_MAX_BYTES / 5);
        let original = text.clone();
        let removed = trim(&mut text, 0);
        assert!(removed > 0 && text.len() <= JOURNAL_RETAIN_BYTES);
        assert_eq!(
            text.encode_utf16().collect::<Vec<_>>(),
            original
                .encode_utf16()
                .skip(removed as usize)
                .collect::<Vec<_>>()
        );
    }
}
