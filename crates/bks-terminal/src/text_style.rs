use crate::{Color, Flags, NamedColor};

/// Source VT attributes, independent of selection/search overlays and theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextStyle {
    pub foreground: Color,
    pub background: Color,
    pub flags: Flags,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            foreground: Color::Named(NamedColor::Foreground),
            background: Color::Named(NamedColor::Background),
            flags: Flags::empty(),
        }
    }
}

impl TextStyle {
    pub fn new(foreground: Color, background: Color, flags: Flags) -> Self {
        Self {
            foreground,
            background,
            flags: flags
                & (Flags::BOLD
                    | Flags::DIM
                    | Flags::ITALIC
                    | Flags::ALL_UNDERLINES
                    | Flags::STRIKEOUT
                    | Flags::INVERSE),
        }
    }
}

/// Half-open canonical UTF-16 range. Adjacent identical styles are coalesced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleSpan {
    pub start: u32,
    pub end: u32,
    pub style: TextStyle,
}

impl StyleSpan {
    pub fn append(spans: &mut Vec<Self>, start: u32, end: u32, style: TextStyle) {
        if start >= end {
            return;
        }
        if let Some(last) = spans
            .last_mut()
            .filter(|last| last.end == start && last.style == style)
        {
            last.end = end;
        } else {
            spans.push(Self { start, end, style });
        }
    }

    /// Rebase retained journal spans after deleting a UTF-16 prefix.
    pub fn trim_prefix(spans: &mut Vec<Self>, removed: u32) {
        let first = spans.partition_point(|span| span.end <= removed);
        spans.drain(..first);
        for span in spans {
            span.start = span.start.saturating_sub(removed);
            span.end -= removed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_merge_and_trim_without_changing_style_or_splitting_ranges() {
        let plain = TextStyle::default();
        let bold = TextStyle::new(plain.foreground, plain.background, Flags::BOLD);
        let mut spans = Vec::new();
        StyleSpan::append(&mut spans, 0, 3, plain);
        StyleSpan::append(&mut spans, 3, 5, plain);
        StyleSpan::append(&mut spans, 5, 8, bold);
        assert_eq!(spans.len(), 2);
        StyleSpan::trim_prefix(&mut spans, 4);
        assert_eq!((spans[0].start, spans[0].end), (0, 1));
        assert_eq!((spans[1].start, spans[1].end), (1, 4));
        assert_eq!(spans[1].style, bold);
        StyleSpan::trim_prefix(&mut spans, 4);
        assert!(spans.is_empty());
    }
}
