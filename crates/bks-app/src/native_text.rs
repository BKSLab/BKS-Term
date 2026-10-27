pub struct TextPatch {
    pub start: u32,
    pub end: u32,
    pub replacement: String,
}

/// One replacement on Unicode scalar boundaries, using native EDIT offsets.
pub fn difference(old: &str, new: &str) -> Option<TextPatch> {
    if old == new {
        return None;
    }
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    Some(TextPatch {
        start: old[..prefix].encode_utf16().count() as u32,
        end: old[..old.len() - suffix].encode_utf16().count() as u32,
        replacement: new[prefix..new.len() - suffix].to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacements_cover_append_erase_unicode_and_changed_rows() {
        for (old, new) in [
            ("", "hello"),
            ("hello", ""),
            ("same", "same"),
            ("row1\r\nrow2", "row1\r\nrow2\r\nrow3"),
            ("😀oldЖ", "😀newЖ"),
            ("e\u{301} one", "é two"),
            ("progress10%\r\nkeep", "progress20%\r\nkeep"),
        ] {
            let mut units: Vec<u16> = old.encode_utf16().collect();
            if let Some(patch) = difference(old, new) {
                units.splice(
                    patch.start as usize..patch.end as usize,
                    patch.replacement.encode_utf16(),
                );
            }
            assert_eq!(String::from_utf16(&units).unwrap(), new);
        }
    }
    #[test]
    fn append_does_not_replace_previous_text_or_split_surrogates() {
        let patch = difference("😀Ж", "😀Ж中").unwrap();
        assert_eq!((patch.start, patch.end), (3, 3));
        assert_eq!(patch.replacement, "中");
    }
}
