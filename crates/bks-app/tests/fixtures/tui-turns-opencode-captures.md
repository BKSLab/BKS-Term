# Plain Markdown TUI answer fixtures

These are byte-exact output captures from the installed OpenCode 2.0.32
renderer in an isolated ConPTY, 110 columns by 36 rows. A local loopback
OpenAI-compatible fixture returned predefined text. No real model requests,
user profiles, user sessions, tools, or external network were involved.

## Two answers

`tui-turns-opencode-two-answers.vt` is the complete 124,084-byte capture.
SHA-256: `A1D30A63E2CC7B7235AFCF459B7468464796B50C9D512F573044AFC5E613B4C2`.

The accepted inputs were:

1. `Привет, кто ты и что умеешь?`
2. `Понял. На какой модели ты работаешь?`

The first response contains paragraphs, styled Markdown headings, bullets,
wrapped prose, Cyrillic text, and a two-column emoji. The second response is a
separate model-description fixture. Neither has an `Assistant:` or bullet-role
prefix. The echoed user input is inside a `┃` panel; composer and toolbar rows
are separate from the response.

| Phase | Exclusive byte end |
| --- | ---: |
| Startup | 13,400 |
| First submission | 16,241 |
| First streaming frame | 21,524 |
| First completed frame | 78,185 |
| Second submission | 81,591 |
| Second streaming frame | 93,551 |
| Second completed frame | 121,923 |

## Response longer than the viewport

`tui-turns-opencode-long-scroll.vt` is the first 163,316 bytes of a separate
capture, through its first completed answer.
SHA-256: `E535DFB5DB518DEEF3037713A7E483BFF8D1E2E489098F15FBF3110B20E23CCF`.
The first input is identical to the first input above. The predefined response
contains 120 numbered paragraphs, each with a unique `MARKER_NNN Ж🙂.`.
The accepted prompt and beginning disappear from the live screen during
streaming. The final screen contains paragraphs 113–120 and the final summary.

The input was composed at byte 12,435, before its explicit submission. Tests
replay actual renderer frame boundaries (`ESC[?2026l`) after that baseline.
Snapshots taken only every 350 ms miss early source frames. Production PTY
draining must observe completed redraws inside a coalesced read as well.

The frame ending at byte 52,801 has a hidden cursor and a partially overwritten
body. A previously confirmed composer remains useful for layout, but its absent
caret does not prove new answer content. A regression replays the production
composer-history and menu projection pipeline, rejects that intermediate body,
and retains each of the 120 markers exactly once after completion.

The installed renderer's summary duration is derived from
`message.time.completed`; the agent and model names are opaque text. The parser
requires an accepted source prompt plus owned body evidence, and does not use
the process name or execute anything read from terminal text. Tests also cover
multiline prompt panels, collapsed reasoning, stale/quoted summaries, errors,
interruptions, reflow of a partial word, and code fences across viewport changes.

The renderer's `Wc` failure element uses the dedicated error foreground for the
entire final block, aligned with the summary and followed by a blank row before
it. A failed result retains that block, its wrapping and source styles in the
answer archive. The source-equivalent error test uses the installed material
theme's `#f07178` error color. Plain `Error:` prose, colored inline examples,
quoted text and fenced code are retained as answer content rather than failure
signals.

## Retry followed by two failed results

`tui-turns-opencode-errors.vt` is a 131,129-byte prefix from a separate isolated
OpenCode 2.0.32 ConPTY capture at 100 columns by 29 rows, recorded 2026-11-02.
SHA-256: `AF3A25FB3CCB804C840F0E9FE5BBCED0CCBA3E881CCE4E47882810E61C4C217E`.

The fixture accepts `привет` twice. Its loopback provider returns HTTP 503 twice
with a fixed `ECONNRESET` diagnostic, then HTTP 403 for each request with the
fixed message `Free usage limit reached.` and subscription URLs from the user
report. These are synthetic local responses; no account quota, external model,
user profile, tools or external network were used. The owned process and local
listener were closed. Capture helper and phase metadata are in the untracked
build directory `target/tui-errors-capture/`; only renderer bytes are retained
here.

| Phase | Exclusive byte end |
| --- | ---: |
| Startup / empty composer | 11,067 |
| First prompt composed | 11,138 |
| First submission | 14,128 |
| First retry | 21,936 |
| First final error, after toast disappeared | 126,426 |
| Second prompt composed | 126,492 |
| Second submission | 127,555 |
| Second final error | 131,129 |

The regression replays every synchronized renderer commit, including the
temporary `Session failed` toast, through composer history, accepted input
evidence and menu exclusion. The final diagnostic replaces the intermediate
retry under the same answer ID. The second equal prompt has a separate owned
source region above an unchanged application footer; it retains a second failed
answer. Both errors remain readable without the retry, summary, toast or
shortcuts, and neither emits successful completion.
