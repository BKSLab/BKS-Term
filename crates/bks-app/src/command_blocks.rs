#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlock {
    pub command_start: u32,
    pub output_start: Option<u32>,
    pub output_end: Option<u32>,
}

#[derive(Default)]
pub struct CommandBlocks {
    blocks: Vec<CommandBlock>,
    command_anchor: Option<u32>,
    active_command: Option<String>,
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
        self.active_command = Some(command.replace("\r\n", "\n"));
        self.text_updated(current_text);
        true
    }

    pub fn text_updated(&mut self, current_text: &str) {
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

    fn resolved_indices(&self) -> Vec<usize> {
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.output_start.is_some())
            .map(|(i, _)| i)
            .collect()
    }

    pub fn needs_output_start(&self) -> bool {
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
}
