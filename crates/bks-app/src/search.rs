use std::cmp::Reverse;

use bks_terminal::ReadingDocument;

/// Half-open range in the original document, measured in UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: u32,
    pub end: u32,
}

#[derive(Default)]
pub struct SearchState {
    query: String,
    needle: Vec<char>,
    failure: Vec<usize>,
    text: String,
    text_len: u32,
    folded: Vec<char>,
    source: Vec<SearchMatch>,
    matches: Vec<SearchMatch>,
    current: Option<usize>,
    revision: Option<u64>,
    alternate_screen: bool,
}

struct CurrentAnchor {
    range: SearchMatch,
    before: Vec<char>,
    after: Vec<char>,
}

impl SearchState {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: String, document: &ReadingDocument) {
        if query == self.query {
            self.refresh(document);
            return;
        }
        self.query = query;
        self.needle = self.query.chars().flat_map(char::to_lowercase).collect();
        self.failure = failure_table(&self.needle);
        self.folded.clear();
        self.source.clear();
        self.matches.clear();
        if !self.needle.is_empty() {
            self.append_folded(&document.text, 0);
            self.scan_matches(0);
        }
        self.current = (!self.matches.is_empty()).then_some(0);
        self.remember(document, 0);
    }

    /// Unchanged revisions do no text work. Otherwise only the changed suffix
    /// and query overlap are rescanned; history truncation permits a full scan.
    pub fn refresh(&mut self, document: &ReadingDocument) -> bool {
        if self.revision == Some(document.revision)
            && self.alternate_screen == document.alternate_screen
        {
            return false;
        }
        if self.text == document.text {
            self.revision = Some(document.revision);
            self.alternate_screen = document.alternate_screen;
            return false;
        }
        if self.needle.is_empty() {
            self.remember(document, 0);
            return false;
        }
        let old_index = self.current;
        let anchor = self.current_anchor();
        let old_range = anchor.as_ref().map(|anchor| anchor.range);
        let old_len = self.text_len;
        let mut prefix_bytes = self
            .text
            .bytes()
            .zip(document.text.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(prefix_bytes) {
            prefix_bytes -= 1;
        }
        let prefix_units = self.text[..prefix_bytes].encode_utf16().count() as u32;
        let suffix_units: u32 = self.text[prefix_bytes..]
            .chars()
            .rev()
            .zip(document.text[prefix_bytes..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(character, _)| character.len_utf16() as u32)
            .sum();
        let prefix_folded = self.source.partition_point(|span| span.end <= prefix_units);
        let mut scan_start = prefix_folded.saturating_sub(self.needle.len() - 1);
        // Start at the whole source character if lowercase expanded it.
        while scan_start > 0
            && self
                .source
                .get(scan_start)
                .is_some_and(|span| span.start == self.source[scan_start - 1].start)
        {
            scan_start -= 1;
        }
        let cutoff = self
            .source
            .get(scan_start)
            .map_or(prefix_units, |span| span.start);
        let keep = self.matches.partition_point(|found| found.start < cutoff);
        let old_tail = self.matches[keep..].to_vec();
        self.matches.truncate(keep);
        self.folded.truncate(prefix_folded);
        self.source.truncate(prefix_folded);
        self.append_folded(&document.text[prefix_bytes..], prefix_units);
        self.scan_matches(scan_start);
        self.current = self.choose_current(anchor.as_ref(), prefix_units, suffix_units, old_len);
        let changed = self.matches[keep..] != old_tail
            || self.current != old_index
            || self.current() != old_range;
        self.remember(document, prefix_bytes);
        changed
    }

    /// Sorted by start and by end (both nondecreasing), with exact duplicates
    /// removed. Overlapping occurrences remain separate matches.
    pub fn matches(&self) -> &[SearchMatch] {
        &self.matches
    }

    pub fn current(&self) -> Option<SearchMatch> {
        self.current
            .and_then(|index| self.matches.get(index))
            .copied()
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn document_revision(&self) -> Option<u64> {
        self.revision
    }

    /// True when a match exists; navigation wraps in either direction.
    pub fn navigate(&mut self, previous: bool) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        let index = self.current.unwrap_or(0);
        self.current = Some(if previous {
            (index + self.matches.len() - 1) % self.matches.len()
        } else {
            (index + 1) % self.matches.len()
        });
        true
    }

    fn remember(&mut self, document: &ReadingDocument, prefix_bytes: usize) {
        self.text.truncate(prefix_bytes);
        self.text.push_str(&document.text[prefix_bytes..]);
        self.text_len = document.lines.last().map_or(0, |line| line.end);
        self.revision = Some(document.revision);
        self.alternate_screen = document.alternate_screen;
    }

    fn append_folded(&mut self, text: &str, mut offset: u32) {
        for character in text.chars() {
            let span = SearchMatch {
                start: offset,
                end: offset + character.len_utf16() as u32,
            };
            for lowercase in character.to_lowercase() {
                self.folded.push(lowercase);
                self.source.push(span);
            }
            offset = span.end;
        }
    }

    fn scan_matches(&mut self, from: usize) {
        let mut matched = 0;
        for index in from..self.folded.len() {
            while matched > 0 && self.folded[index] != self.needle[matched] {
                matched = self.failure[matched - 1];
            }
            if self.folded[index] == self.needle[matched] {
                matched += 1;
            }
            if matched == self.needle.len() {
                let start = index + 1 - matched;
                let found = SearchMatch {
                    start: self.source[start].start,
                    end: self.source[index].end,
                };
                if self.matches.last() != Some(&found) {
                    self.matches.push(found);
                }
                matched = self.failure[matched - 1];
            }
        }
    }

    fn current_anchor(&self) -> Option<CurrentAnchor> {
        let range = self.current()?;
        let start = self.source.partition_point(|span| span.start < range.start);
        let end = self.source.partition_point(|span| span.end <= range.end);
        Some(CurrentAnchor {
            range,
            before: self.folded[start.saturating_sub(32)..start].to_vec(),
            after: self.folded[end..(end + 32).min(self.folded.len())].to_vec(),
        })
    }

    fn choose_current(
        &self,
        anchor: Option<&CurrentAnchor>,
        prefix: u32,
        suffix: u32,
        old_len: u32,
    ) -> Option<usize> {
        let first = (!self.matches.is_empty()).then_some(0)?;
        let Some(anchor) = anchor else {
            return Some(first);
        };
        let new_len = self.source.last().map_or(0, |span| span.end);
        let unchanged = if anchor.range.end <= prefix {
            Some(anchor.range)
        } else if suffix > 0 && anchor.range.start >= old_len - suffix {
            Some(SearchMatch {
                start: new_len - (old_len - anchor.range.start),
                end: new_len - (old_len - anchor.range.end),
            })
        } else {
            None
        };
        if let Some(range) = unchanged {
            if let Ok(index) = self
                .matches
                .binary_search_by_key(&(range.start, range.end), |found| (found.start, found.end))
            {
                return Some(index);
            }
        }
        let expected = unchanged.map_or_else(
            || {
                if prefix == 0 && suffix > 0 {
                    (i64::from(anchor.range.start) + i64::from(new_len) - i64::from(old_len))
                        .clamp(0, i64::from(new_len)) as u32
                } else {
                    anchor.range.start.min(new_len)
                }
            },
            |range| range.start,
        );
        let nearest = (0..self.matches.len())
            .min_by_key(|&index| (self.matches[index].start.abs_diff(expected), index))
            .unwrap();
        let contextual = (0..self.matches.len())
            .max_by_key(|&index| {
                (
                    self.context_score(anchor, self.matches[index]),
                    Reverse(self.matches[index].start.abs_diff(expected)),
                    Reverse(index),
                )
            })
            .unwrap();
        let required = 8.min(anchor.before.len() + anchor.after.len());
        Some(
            if required > 0 && self.context_score(anchor, self.matches[contextual]) >= required {
                contextual
            } else {
                nearest
            },
        )
    }

    fn context_score(&self, anchor: &CurrentAnchor, found: SearchMatch) -> usize {
        let start = self.source.partition_point(|span| span.start < found.start);
        let end = self.source.partition_point(|span| span.end <= found.end);
        let before = anchor
            .before
            .iter()
            .rev()
            .zip(self.folded[..start].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let after = anchor
            .after
            .iter()
            .zip(&self.folded[end..])
            .take_while(|(a, b)| a == b)
            .count();
        before + after
    }
}

fn failure_table(needle: &[char]) -> Vec<usize> {
    let mut failure = vec![0; needle.len()];
    let mut matched = 0;
    for index in 1..needle.len() {
        while matched > 0 && needle[index] != needle[matched] {
            matched = failure[matched - 1];
        }
        if needle[index] == needle[matched] {
            matched += 1;
        }
        failure[index] = matched;
    }
    failure
}

#[cfg(test)]
mod tests {
    use super::*;
    use bks_terminal::TerminalModel;

    fn document(text: &str, revision: u64) -> ReadingDocument {
        let mut terminal = TerminalModel::new(120, 60);
        terminal.push_bytes(text.as_bytes());
        let mut document = terminal.reading_document();
        document.revision = revision;
        document
    }

    fn range(start: u32, end: u32) -> SearchMatch {
        SearchMatch { start, end }
    }

    fn offset(text: &str, needle: &str) -> u32 {
        text[..text.find(needle).unwrap()].encode_utf16().count() as u32
    }

    #[test]
    fn russian_search_is_literal_and_case_insensitive() {
        let document = document("ПРИВЕТ, привет! Привет", 1);
        let mut search = SearchState::default();
        search.set_query("прИвЕТ".into(), &document);
        assert_eq!(search.query(), "прИвЕТ");
        assert_eq!(
            search.matches(),
            &[range(0, 6), range(8, 14), range(16, 22)]
        );
        assert_eq!(search.current(), Some(range(0, 6)));
        search.set_query(".*".into(), &document);
        assert!(search.matches().is_empty());
    }

    #[test]
    fn unicode_ranges_refer_to_original_surrogate_and_combining_codepoints() {
        let document = document("🙂 e\u{301} 🙂E\u{301}", 1);
        let mut search = SearchState::default();
        search.set_query("E\u{301}".into(), &document);
        assert_eq!(search.matches(), &[range(3, 5), range(8, 10)]);
        search.set_query("🙂".into(), &document);
        assert_eq!(search.matches(), &[range(0, 2), range(6, 8)]);
        search.set_query("E".into(), &document);
        assert_eq!(search.matches(), &[range(3, 4), range(8, 9)]);
    }

    #[test]
    fn expanded_lowercase_maps_every_unit_back_to_the_source_character() {
        let document = document("İ i\u{307} I", 1);
        let mut search = SearchState::default();
        search.set_query("i\u{307}".into(), &document);
        assert_eq!(search.matches(), &[range(0, 1), range(2, 4)]);
        search.set_query("İ".into(), &document);
        assert_eq!(search.matches(), &[range(0, 1), range(2, 4)]);
        search.set_query("i".into(), &document);
        assert_eq!(search.matches(), &[range(0, 1), range(2, 3), range(5, 6)]);
        search.set_query("\u{307}".into(), &document);
        assert_eq!(search.matches(), &[range(0, 1), range(3, 4)]);
    }

    #[test]
    fn empty_query_and_no_results_have_no_current_match() {
        let document = document("found", 1);
        let mut search = SearchState::default();
        assert_eq!(search.document_revision(), None);
        search.set_query("missing".into(), &document);
        assert!(search.matches().is_empty());
        assert_eq!(search.current(), None);
        assert_eq!(search.current_index(), None);
        assert!(!search.navigate(false));
        search.set_query("found".into(), &document);
        assert!(search.current().is_some());
        search.set_query(String::new(), &document);
        assert!(search.matches().is_empty());
        assert_eq!(search.current_index(), None);
        assert!(!search.navigate(true));
        assert_eq!(search.document_revision(), Some(1));
    }

    #[test]
    fn overlapping_matches_cycle_in_both_directions_and_new_query_resets_current() {
        let document = document("aaaa", 1);
        let mut search = SearchState::default();
        search.set_query("aa".into(), &document);
        assert_eq!(search.matches(), &[range(0, 2), range(1, 3), range(2, 4)]);
        assert_eq!(search.current_index(), Some(0));
        assert!(search.navigate(true));
        assert_eq!(search.current_index(), Some(2));
        assert!(search.navigate(false));
        assert_eq!(search.current_index(), Some(0));
        assert!(search.navigate(false));
        search.set_query("aa".into(), &document);
        assert_eq!(search.current_index(), Some(1));
        search.set_query("aaaa".into(), &document);
        assert_eq!(search.current_index(), Some(0));
        assert!(search.navigate(true));
        assert_eq!(search.current_index(), Some(0));
    }

    #[test]
    fn appended_chunks_find_matches_crossing_the_old_boundary() {
        let mut search = SearchState::default();
        search.set_query("target".into(), &document("ta", 1));
        assert!(search.matches().is_empty());
        assert!(!search.refresh(&document("targ", 2)));
        assert!(search.refresh(&document("target", 3)));
        assert_eq!(search.matches(), &[range(0, 6)]);
        assert!(search.refresh(&document("target target", 4)));
        assert_eq!(search.matches(), &[range(0, 6), range(7, 13)]);
        assert_eq!(search.current_index(), Some(0));
    }

    #[test]
    fn append_does_not_move_current_to_new_tail_matches() {
        let first = document("hit first\r\nhit second", 1);
        let mut search = SearchState::default();
        search.set_query("hit".into(), &first);
        search.navigate(false);
        let selected = search.current();
        assert!(search.refresh(&document(
            "hit first\r\nhit second\r\nhit third\r\nhit fourth",
            2
        )));
        assert_eq!(search.current(), selected);
        assert_eq!(search.current_index(), Some(1));
    }

    #[test]
    fn current_context_survives_overwrite_and_history_trim_then_uses_nearest_or_none() {
        let first = document("hit first\r\nhit selected unique context\r\nhit last", 1);
        let mut search = SearchState::default();
        search.set_query("hit".into(), &first);
        search.navigate(false);
        let overwritten = document(
            "changed heading\r\nhit first\r\nhit selected unique context\r\nhit last\r\nhit tail",
            2,
        );
        search.refresh(&overwritten);
        assert_eq!(
            search.current().unwrap().start,
            offset(&overwritten.text, "hit selected")
        );
        let trimmed = document(
            "hit selected unique context\r\nhit last\r\nhit tail\r\nhit newer",
            3,
        );
        search.refresh(&trimmed);
        assert_eq!(search.current(), Some(range(0, 3)));
        let removed = document("replacement text\r\nhit last\r\nhit tail\r\nhit newer", 4);
        search.refresh(&removed);
        assert_eq!(
            search.current().unwrap().start,
            offset(&removed.text, "hit last")
        );
        assert_eq!(search.current_index(), Some(0));
        assert!(search.refresh(&document("", 5)));
        assert!(search.matches().is_empty());
        assert_eq!(search.current(), None);
    }

    #[test]
    fn unchanged_revision_or_text_reuses_matches_and_only_updates_revision_tag() {
        let first = document("x x x", 1);
        let mut search = SearchState::default();
        search.set_query("x".into(), &first);
        search.navigate(false);
        let matches = search.matches.as_ptr();
        let folded = search.folded.as_ptr();
        assert!(!search.refresh(&first));
        let same_text = document("x x x", 2);
        assert!(!search.refresh(&same_text));
        assert_eq!(search.document_revision(), Some(2));
        assert_eq!(search.current_index(), Some(1));
        assert_eq!(search.matches.as_ptr(), matches);
        assert_eq!(search.folded.as_ptr(), folded);
    }

    #[test]
    fn changed_suffix_incremental_results_equal_full_rescan_with_expansions() {
        let versions = [
            "a",
            "aaa",
            "aaaİ",
            "aaaİi\u{307}",
            "aaaİi\u{307}🙂",
            "aİi\u{307}🙂",
            "changed aİi\u{307}🙂",
            "",
        ];
        for query in ["aa", "İ", "i", "\u{307}", "🙂", "aİ"] {
            let mut incremental = SearchState::default();
            incremental.set_query(query.into(), &document("", 1));
            for (index, text) in versions.iter().enumerate() {
                let next = document(text, index as u64 + 2);
                incremental.refresh(&next);
                let mut full = SearchState::default();
                full.set_query(query.into(), &next);
                assert_eq!(
                    incremental.matches(),
                    full.matches(),
                    "query={query:?}, text={text:?}"
                );
                assert!(incremental
                    .matches()
                    .windows(2)
                    .all(|pair| pair[0].start <= pair[1].start && pair[0].end <= pair[1].end));
            }
        }
    }

    #[test]
    fn searches_the_entire_ten_thousand_line_history() {
        let mut terminal = TerminalModel::new(32, 3);
        for index in 0..10_005 {
            terminal.push_bytes(format!("row{index:05} needle\r\n").as_bytes());
        }
        let document = terminal.reading_document();
        assert_eq!(terminal.snapshot().history_size, 10_000);
        let mut search = SearchState::default();
        search.set_query("NEEDLE".into(), &document);
        assert_eq!(
            search.matches().len(),
            document.text.matches("needle").count()
        );
        assert!(search.matches().len() > 10_000);
        assert!(search.navigate(true));
        assert_eq!(search.current_index(), Some(search.matches().len() - 1));
        assert_eq!(search.document_revision(), Some(document.revision));
    }
}
