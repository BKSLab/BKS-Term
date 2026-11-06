# Paired-role renderer fixtures

These small ANSI tapes are **source-equivalent fixtures**, not captures of an
authenticated model session. They contain two fixed accepted questions and
answers, an in-progress cancellation control, and the source renderer's editor
geometry. The JSON files record byte boundaries, dimensions and SHA-256.
No product-name detection is used by the parser.

## Codex 0.164.0

The formats follow the official `rust-v0.164.0` source:

- [history_cell/messages.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/history_cell/messages.rs):
  `UserHistoryCell` uses `› `; `AgentMessageCell`, `AgentMarkdownCell` and
  `StreamingAgentTailCell` use a dim `• ` prefix and two-column continuation
  indentation. Only the bullet is dimmed for an ordinary answer. Reasoning
  summary content is dim and italic.
- [status_indicator_widget.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/status_indicator_widget.rs):
  the active indicator contains elapsed time and the bound interrupt key,
  separated by `•`, such as `Working (1s • esc to interrupt)`.
- [history_cell/separators.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/tui/src/history_cell/separators.rs):
  `FinalMessageSeparator` records completed-turn duration/time after the final
  response. Its entire row is dim, such as `Worked for 2s • 20:45`.
- The padded editor follows the same version's `chat_composer.rs` and
  `chat_composer/composer_layout.rs`, also covered by the separate composer
  fixtures in `../tui-codex-claude-composers`.

## Claude Code 2.1.240, Windows

The installed Windows binary has SHA-256
`F7EE87C58D315BEDBD38FC8923CE7B955DBBAB2AFE50D9D15C059463A70869DA`.
Its embedded renderer was inspected without executing a model request:

- At byte 302045395, `Xc` selects `●` on Windows (`⏺` is the macOS branch).
- `urg` at 320402013 dispatches assistant messages to `wcc`; its `text` branch
  at 320412446 calls `Zdn`. `Zdn` at 320196052 renders the assistant marker in a
  two-column box, then ordinary Markdown text in a sibling column.
- User text follows `h9e` → `vps` → `gps` (320306349); the ordinary layout uses
  the native pointer followed by a space. The ruled editor separately uses
  pointer + NBSP, as documented in the composer fixtures.
- `NxA` at 322808263 includes the interrupt shortcut while active. `pt` at
  308798826 renders that shortcut as `esc to interrupt`. Its removal followed
  by a fresh editable composer is the existing generic completion boundary.

The fixture deliberately keeps the shortcut footer below the editor. It must
not leak into retained answer text. Separate tests cover NBSP after `›`, quoted
and fenced echoes, stale/unstyled completion text, and next/previous answer IDs.

These checks establish renderer-format compatibility. Live authorized model
responses and NVDA behaviour remain separate acceptance checks.
