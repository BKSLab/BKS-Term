#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlock {
    pub command_start: u32,
    pub output_start: Option<u32>,
    pub output_end: Option<u32>,
}

const MAX_COMMAND_BLOCKS: usize = 4096;

struct UntrackedBlock {
    index: usize,
    pending_echo: Option<String>,
}

#[derive(Default)]
pub struct CommandBlocks {
    blocks: Vec<CommandBlock>,
    command_anchor: Option<u32>,
    active_command: Option<String>,
    untracked: Option<UntrackedBlock>,
}

impl CommandBlocks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prompt_started(&mut self, position: u32) {
        if let Some(last) = self.blocks.last_mut() {
            if last.output_end.is_none() {
                last.output_end = Some(position);
            }
        }
    }

    pub fn command_started(&mut self, position: u32) {
        self.command_anchor = Some(position);
    }

    pub fn can_execute(&self) -> bool {
        self.command_anchor.is_some()
    }

    pub fn execute(&mut self, command: &str, current_text: &str) -> bool {
        let Some(anchor) = self.command_anchor.take() else {
            return false;
        };
        self.blocks.push(CommandBlock {
            command_start: anchor,
            output_start: None,
            output_end: None,
        });
        self.keep_recent();
        self.active_command = Some(command.replace("\r\n", "\n"));
        self.text_updated(current_text);
        true
    }

    /// Records only an explicitly submitted command in a shell without OSC 133.
    pub fn execute_untracked(&mut self, command: &str, current_text: &str) {
        let anchor = current_text.encode_utf16().count() as u32;
        if let Some(previous) = self.untracked.take() {
            self.blocks[previous.index].output_end.get_or_insert(anchor);
        }
        self.untracked = Some(UntrackedBlock {
            index: self.blocks.len(),
            pending_echo: Some(command.replace("\r\n", "\n")),
        });
        self.blocks.push(CommandBlock {
            command_start: anchor,
            output_start: Some(anchor),
            output_end: None,
        });
        self.keep_recent();
    }

    fn keep_recent(&mut self) {
        let removed = self.blocks.len().saturating_sub(MAX_COMMAND_BLOCKS);
        if removed == 0 {
            return;
        }
        self.blocks.drain(..removed);
        self.untracked = self.untracked.take().and_then(|mut block| {
            block.index = block.index.checked_sub(removed)?;
            Some(block)
        });
    }

    /// Rebase offsets after the same prefix is removed from model/native text.
    /// A partially retained running result remains navigable at offset zero.
    pub fn trim_prefix(&mut self, removed: u32) {
        if removed == 0 {
            return;
        }
        let dropped = self
            .blocks
            .iter()
            .enumerate()
            .take_while(|(index, block)| {
                block
                    .output_end
                    .or_else(|| self.blocks.get(index + 1).map(|next| next.command_start))
                    .is_some_and(|end| end <= removed)
            })
            .count();
        self.blocks.drain(..dropped);
        self.untracked = self.untracked.take().and_then(|mut block| {
            block.index = block.index.checked_sub(dropped)?;
            if self.blocks[block.index].command_start < removed {
                block.pending_echo = None;
            }
            Some(block)
        });
        for block in &mut self.blocks {
            if block.command_start < removed && block.output_start.is_none() {
                block.output_start = Some(0);
            }
            block.command_start = block.command_start.saturating_sub(removed);
            block.output_start = block
                .output_start
                .map(|offset| offset.saturating_sub(removed));
            block.output_end = block
                .output_end
                .map(|offset| offset.saturating_sub(removed));
        }
        if self
            .blocks
            .last()
            .is_none_or(|block| block.output_start.is_some())
        {
            self.active_command = None;
        }
        self.command_anchor = self
            .command_anchor
            .map(|offset| offset.saturating_sub(removed));
    }

    pub fn text_updated(&mut self, current_text: &str) {
        self.refine_untracked_start(current_text);
        let Some(command) = self.active_command.as_deref() else {
            return;
        };
        let Some(block) = self.blocks.last_mut() else {
            return;
        };
        if block.output_start.is_some() {
            return;
        }
        if let Some(start) = resolve_output_start(command, current_text, block.command_start) {
            block.output_start = Some(start);
            self.active_command = None;
        }
    }

    fn refine_untracked_start(&mut self, current_text: &str) {
        let Some(untracked) = self.untracked.as_mut() else {
            return;
        };
        let Some(command) = untracked.pending_echo.as_deref() else {
            return;
        };
        let block = &mut self.blocks[untracked.index];
        if let Some(start) = resolve_output_start(command, current_text, block.command_start) {
            block.output_start = Some(start);
            untracked.pending_echo = None;
        } else {
            // Wait for split echo, but stop trying after enough complete lines
            // arrived without an exact match. Edited/no echo keeps the anchor.
            let lines = command.split('\n').count();
            let received = current_text
                .encode_utf16()
                .skip(block.command_start as usize)
                .filter(|&c| c == b'\n' as u16)
                .take(lines)
                .count();
            if received == lines {
                untracked.pending_echo = None;
            }
        }
    }

    fn resolved_indices(&self) -> Vec<usize> {
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.output_start.is_some())
            .map(|(i, _)| i)
            .collect()
    }

    pub fn needs_output_start(&self) -> bool {
        // Generic blocks already have a usable anchor; only OSC-based blocks
        // need the UI's timer-driven lookup. Refine generic echo on new chunks.
        self.active_command.is_some()
    }

    pub fn last(&self) -> Option<u32> {
        self.blocks
            .iter()
            .rev()
            .find(|b| b.output_start.is_some())
            .and_then(|b| b.output_start)
    }

    pub fn current(&self, position: u32) -> Option<(u32, Option<u32>)> {
        let resolved = self.resolved_indices();
        let mut current: Option<usize> = None;
        for &i in &resolved {
            let start = self.blocks[i].output_start.unwrap();
            if start <= position {
                current = Some(i);
            } else {
                break;
            }
        }
        let i = current?;
        let b = &self.blocks[i];
        Some((b.output_start.unwrap(), b.output_end))
    }

    pub fn previous(&self, position: u32) -> Option<u32> {
        let resolved = self.resolved_indices();
        let mut current: Option<usize> = None;
        for (k, &i) in resolved.iter().enumerate() {
            let start = self.blocks[i].output_start.unwrap();
            if start <= position {
                current = Some(k);
            } else {
                break;
            }
        }
        let k = current?;
        let start = self.blocks[resolved[k]].output_start.unwrap();
        if position > start {
            Some(start)
        } else if k > 0 {
            self.blocks[resolved[k - 1]].output_start
        } else {
            None
        }
    }

    pub fn next(&self, position: u32) -> Option<u32> {
        self.blocks
            .iter()
            .filter_map(|b| b.output_start)
            .find(|&start| start > position)
    }

    pub fn navigate(&self, previous: bool, position: Option<u32>) -> Option<u32> {
        let first = self.blocks.iter().find_map(|block| block.output_start)?;
        match (previous, position) {
            (true, None) => self.last(),
            (false, None) => Some(first),
            (_, Some(position)) if position < first => Some(first),
            (true, Some(position)) => self.previous(position).or(Some(first)),
            (false, Some(position)) => self.next(position).or_else(|| self.last()),
        }
    }
}

fn resolve_output_start(command: &str, current_text: &str, command_start: u32) -> Option<u32> {
    let text: Vec<u16> = current_text.encode_utf16().collect();
    let mut pos = command_start as usize;
    if pos > text.len() {
        return None;
    }
    let lines: Vec<&str> = command.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let line16: Vec<u16> = line.encode_utf16().collect();
        if pos + line16.len() > text.len() || text[pos..pos + line16.len()] != line16[..] {
            return None;
        }
        pos += line16.len();
        if pos + 2 > text.len() || text[pos] != b'\r' as u16 || text[pos + 1] != b'\n' as u16 {
            return None;
        }
        pos += 2;
        if i + 1 != lines.len()
            && pos + 3 <= text.len()
            && text[pos] == b'>' as u16
            && text[pos + 1] == b'>' as u16
            && text[pos + 2] == b' ' as u16
        {
            pos += 3;
        }
    }
    Some(pos as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_command_closed_by_next_prompt() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        assert!(cb.execute("dir", "$ dir\r\noutput line\r\n$ "));
        cb.prompt_started(20);
        assert_eq!(cb.last(), Some(7));
        assert_eq!(cb.current(10), Some((7, Some(20))));
    }

    #[test]
    fn two_commands_navigation() {
        let text = "$ a\r\nr1\r\n$ b\r\nr2\r\n$ ";
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute("a", text);
        cb.prompt_started(9);
        cb.command_started(11);
        cb.execute("b", text);
        cb.prompt_started(18);

        assert_eq!(cb.last(), Some(14));
        assert_eq!(cb.current(6), Some((5, Some(9))));
        assert_eq!(cb.current(5), Some((5, Some(9))));
        assert_eq!(cb.current(15), Some((14, Some(18))));
        assert_eq!(cb.previous(6), Some(5));
        assert_eq!(cb.previous(5), None);
        assert_eq!(cb.previous(14), Some(5));
        assert_eq!(cb.previous(15), Some(14));
        assert_eq!(cb.next(5), Some(14));
        assert_eq!(cb.next(14), None);
        assert_eq!(cb.current(4), None);
    }

    #[test]
    fn multiline_with_continuation_prompt() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute(
            "echo one\necho two",
            "$ echo one\r\n>> echo two\r\nresult\r\n$ ",
        );
        cb.prompt_started(33);
        assert_eq!(cb.current(26), Some((25, Some(33))));
    }

    #[test]
    fn multiline_without_continuation_prompt() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute("echo one\necho two", "$ echo one\r\necho two\r\nr\r\n");
        cb.prompt_started(25);
        assert_eq!(cb.current(22), Some((22, Some(25))));
    }

    #[test]
    fn split_echo_resolved_later() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        assert!(cb.execute("dir", ""));
        assert_eq!(cb.current(3), None);
        cb.text_updated("$ dir\r\nout\r\n");
        assert_eq!(cb.current(8), Some((7, None)));
        cb.prompt_started(12);
        assert_eq!(cb.current(8), Some((7, Some(12))));
    }

    #[test]
    fn incomplete_echo_does_not_resolve() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute("dir", "$ di");
        cb.text_updated("$ dir");
        assert_eq!(cb.current(4), None);
        cb.text_updated("$ dir\r\n");
        assert_eq!(cb.current(7), Some((7, None)));
    }

    #[test]
    fn unicode_positions_in_utf16() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(5);
        cb.execute("x", "\u{1F642} $ x\r\nok\r\n");
        cb.prompt_started(12);
        assert_eq!(cb.current(8), Some((8, Some(12))));
    }

    #[test]
    fn unicode_command_line_utf16() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute("\u{1F642}", "$ \u{1F642}\r\nok\r\n");
        cb.prompt_started(10);
        assert_eq!(cb.current(6), Some((6, Some(10))));
    }

    #[test]
    fn execute_without_command_started_creates_nothing() {
        let mut cb = CommandBlocks::new();
        assert!(!cb.can_execute());
        assert!(!cb.execute("dir", "$ dir\r\n"));
        assert_eq!(cb.last(), None);
        assert_eq!(cb.current(0), None);
        assert_eq!(cb.previous(0), None);
        assert_eq!(cb.next(0), None);
    }

    #[test]
    fn command_anchor_is_consumed_by_execute() {
        let mut cb = CommandBlocks::new();
        cb.command_started(2);
        assert!(cb.can_execute());
        assert!(cb.execute("one", ""));
        assert!(!cb.can_execute());
        assert!(!cb.execute("two", ""));
    }

    #[test]
    fn prompt_does_not_overwrite_existing_end() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute("a", "$ a\r\nr\r\n");
        cb.prompt_started(8);
        cb.prompt_started(50);
        assert_eq!(cb.current(5), Some((5, Some(8))));
    }

    #[test]
    fn end_closes_even_without_output_start() {
        let mut cb = CommandBlocks::new();
        cb.prompt_started(0);
        cb.command_started(2);
        cb.execute("dir", "");
        cb.prompt_started(10);
        let blocks: Vec<_> = cb.blocks.clone();
        assert_eq!(
            blocks,
            vec![CommandBlock {
                command_start: 2,
                output_start: None,
                output_end: Some(10),
            }]
        );
    }

    fn three_untracked_commands() -> CommandBlocks {
        let mut blocks = CommandBlocks::new();
        let mut text = "$ ".to_owned();
        for command in ["one", "two", "three"] {
            blocks.execute_untracked(command, &text);
            text.push_str(&format!("{command}\r\nout\r\n$ "));
            blocks.text_updated(&text);
        }
        blocks
    }

    #[test]
    fn navigate_three_blocks_from_reader_and_outside_with_clamped_edges() {
        let blocks = three_untracked_commands();
        assert_eq!(blocks.blocks.len(), 3);
        assert_eq!(blocks.current(7), Some((7, Some(14))));
        assert_eq!(blocks.current(19), Some((19, Some(26))));
        assert_eq!(blocks.current(33), Some((33, None)));
        for (previous, position, expected) in [
            (true, None, 33),
            (false, None, 7),
            (true, Some(0), 7),
            (false, Some(0), 7),
            (true, Some(7), 7),
            (false, Some(7), 19),
            (true, Some(10), 7),
            (false, Some(10), 19),
            (true, Some(19), 7),
            (false, Some(19), 33),
            (true, Some(33), 19),
            (false, Some(33), 33),
            (true, Some(40), 33),
            (false, Some(40), 33),
        ] {
            assert_eq!(
                blocks.navigate(previous, position),
                Some(expected),
                "previous={previous}, position={position:?}"
            );
        }
    }

    #[test]
    fn navigate_empty_or_unresolved_integrated_blocks_has_no_target() {
        let mut blocks = CommandBlocks::new();
        assert_eq!(blocks.navigate(true, None), None);
        assert_eq!(blocks.navigate(false, Some(0)), None);
        blocks.command_started(2);
        assert!(blocks.execute("dir", "$ di"));
        assert_eq!(blocks.navigate(true, None), None);
        blocks.text_updated("$ dir\r\n");
        assert_eq!(blocks.navigate(false, None), Some(7));
        assert_eq!(blocks.navigate(true, Some(7)), Some(7));
        assert_eq!(blocks.navigate(false, Some(7)), Some(7));
    }

    #[test]
    fn untracked_anchor_and_refined_echo_use_utf16_positions() {
        let mut blocks = CommandBlocks::new();
        blocks.execute_untracked("echo 🙂", "🙂 $ ");
        assert!(!blocks.needs_output_start());
        assert_eq!(blocks.last(), Some(5));
        assert_eq!(blocks.blocks[0].command_start, 5);
        blocks.text_updated("🙂 $ echo 🙂\r\nout\r\n");
        assert_eq!(blocks.last(), Some(14));
        assert_eq!(blocks.current(16), Some((14, None)));
    }

    #[test]
    fn untracked_split_echo_is_navigable_before_it_is_complete() {
        let mut blocks = CommandBlocks::new();
        blocks.execute_untracked("dir", "$ ");
        for text in ["$ ", "$ d", "$ dir", "$ dir\r"] {
            blocks.text_updated(text);
            assert_eq!(blocks.navigate(true, None), Some(2));
            assert!(!blocks.needs_output_start());
        }
        blocks.text_updated("$ dir\r\nout\r\n");
        assert_eq!(blocks.last(), Some(7));
        assert!(!blocks.needs_output_start());
    }

    #[test]
    fn untracked_multiline_split_echo_refines_after_all_lines() {
        let mut blocks = CommandBlocks::new();
        blocks.execute_untracked("echo one\r\necho two", "$ ");
        blocks.text_updated("$ echo one\r\n>> echo");
        assert_eq!(blocks.last(), Some(2));
        blocks.text_updated("$ echo one\r\n>> echo two\r\nresult\r\n");
        assert_eq!(blocks.last(), Some(25));
    }

    #[test]
    fn untracked_without_matching_echo_keeps_anchor_and_stops_refining() {
        for text in ["$ output without echo\r\n", "$ dXir\r\n"] {
            let mut blocks = CommandBlocks::new();
            blocks.execute_untracked("dir", "$ ");
            blocks.text_updated(text);
            assert_eq!(blocks.last(), Some(2));
            assert!(!blocks.needs_output_start());
            assert!(blocks.untracked.as_ref().unwrap().pending_echo.is_none());
            // A later edit to the reader's contents cannot invent a new start.
            blocks.text_updated("$ dir\r\n");
            assert_eq!(blocks.last(), Some(2));
        }
    }

    #[test]
    fn next_untracked_submit_closes_previous_without_requiring_output() {
        let mut blocks = CommandBlocks::new();
        blocks.execute_untracked("one", "🙂 $ ");
        assert_eq!(blocks.last(), Some(5));
        blocks.execute_untracked("two", "🙂 $ one\r\n🙂 $ ");
        assert_eq!(blocks.current(5), Some((5, Some(15))));
        assert_eq!(blocks.current(15), Some((15, None)));
        blocks.execute_untracked("three", "🙂 $ one\r\n🙂 $ two\r\n🙂 $ ");
        assert_eq!(blocks.current(15), Some((15, Some(25))));
        assert_eq!(blocks.current(25), Some((25, None)));
        assert!(!blocks.needs_output_start());
    }

    #[test]
    fn retention_removes_finished_blocks_and_keeps_a_partial_result_navigable() {
        let mut blocks = CommandBlocks::new();
        blocks.blocks = vec![
            CommandBlock {
                command_start: 0,
                output_start: Some(2),
                output_end: Some(8),
            },
            CommandBlock {
                command_start: 8,
                output_start: Some(10),
                output_end: Some(18),
            },
            CommandBlock {
                command_start: 18,
                output_start: Some(20),
                output_end: None,
            },
        ];
        blocks.command_anchor = Some(26);
        blocks.untracked = Some(UntrackedBlock {
            index: 2,
            pending_echo: Some("cmd".into()),
        });
        blocks.trim_prefix(12);
        assert_eq!(blocks.blocks.len(), 2);
        assert_eq!(blocks.navigate(false, None), Some(0));
        assert_eq!(blocks.navigate(true, None), Some(8));
        assert_eq!(blocks.current(0), Some((0, Some(6))));
        assert_eq!(blocks.command_anchor, Some(14));
        assert_eq!(blocks.untracked.as_ref().unwrap().index, 1);
        assert_eq!(
            blocks.untracked.as_ref().unwrap().pending_echo.as_deref(),
            Some("cmd")
        );
    }

    #[test]
    fn metadata_is_bounded_even_when_the_shell_returns_no_text() {
        let mut blocks = CommandBlocks::new();
        for _ in 0..MAX_COMMAND_BLOCKS + 50 {
            blocks.execute_untracked("cmd", "");
        }
        assert_eq!(blocks.blocks.len(), MAX_COMMAND_BLOCKS);
        assert_eq!(
            blocks.untracked.as_ref().unwrap().index,
            MAX_COMMAND_BLOCKS - 1
        );
        assert_eq!(blocks.last(), Some(0));
        blocks.trim_prefix(1);
        assert_eq!(blocks.blocks.len(), 1);
        assert_eq!(blocks.last(), Some(0));
        assert!(blocks.untracked.as_ref().unwrap().pending_echo.is_none());
    }

    #[test]
    fn lost_integrated_echo_does_not_remain_pending_after_retention() {
        let mut blocks = CommandBlocks::new();
        blocks.command_started(0);
        assert!(blocks.execute("not-echoed", "partial"));
        blocks.trim_prefix(3);
        assert!(!blocks.needs_output_start());
        assert_eq!(blocks.last(), Some(0));
    }
}
