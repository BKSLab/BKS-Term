# Installed Codex 0.164.0 contracts

Actual Windows ConPTY recordings made on 2026-11-06. Every case ran the installed native Codex 0.164.0 in a new empty fixture directory with a cleared inherited environment, separate HOME/USERPROFILE/APPDATA/XDG/TEMP/CODEX_HOME, and an isolated config. A loopback HTTP provider returned fixed text-only Responses SSE, with `requires_openai_auth = false`. No real credentials, external model, tool call, user window, global config change, commit, or push was used.

The command was the native npm vendor `codex.exe --no-alt-screen --ask-for-approval never --sandbox read-only`. The fixture config disabled updates, analytics, feedback, and web search; external HTTP proxies pointed at refused loopback port 9. Only the newly created empty fixture cwd was trusted before launch. Foreground processes exited naturally using `/quit` with code 0 in every exported case. Persistent daemons from the isolated CODEX_HOME directories were subsequently closed by verified executable path, PID, and creation time.

Capture helper and complete evidence remain under `target/codex-production30/captures/`: `helper/src/main.rs`, source run directories, requests/classification files, exact SSE, source snapshots, launch/config/version/result files, and daemon cleanup records. No preliminary exploratory run was exported.

## Ledger

Each `NAME.vt` contains every actual `drain_output()` byte from startup through exit, unmodified. `NAME.json` contains every actual drain, including empty drains, and each accepted input message with monotonic `at_ms` and exact bytes. Protocol replies are separate in `terminal_responses`. No empty polls or timing were synthesized. `phases` give observed source cursor/size/history and local request counts. `NAME-result.json` gives transport/exit counts. `expected` lists complete answer markers, rather than every attempted user turn; it does not assert speech or Ready behavior.

Startup awaited the actual model footer (`bks-offline default`). The initial `› Ask Codex to do anything` can appear before the backend is ready; an Enter during that earlier stage only shows `Waiting for startup` and later input can extend that pending draft.

The current CLI makes an additional `/responses` request to generate a task title for each conversation request. The fixture provider classifies that request separately. Two completed conversation turns produce four HTTP requests: two conversation requests and two title requests. Title responses do not appear as assistant answers in the terminal.

## Keyboard

Codex emits `ESC[?9001h` (Win32 input mode), as well as bracketed paste/focus/synchronized output modes. Each key case starts in a fresh process with `BEFORE`, sends exactly the indicated key, inserts `AFTER`, then submits with plain Enter. The actual conversation request independently confirms the resulting text.

| Case | Bytes for the tested key | Result before the final Enter |
| --- | --- | --- |
| `key-enter` | `CR` | Submits `BEFORE`; final Enter submits `AFTER` as the second turn |
| `key-win-shift` | `ESC[13;28;13;1;16;1_` followed by `ESC[13;28;13;0;16;1_` | `BEFORE\nAFTER`, no request |
| `key-win-alt` | `ESC[13;28;13;1;2;1_` followed by `ESC[13;28;13;0;2;1_` | `BEFORE\nAFTER`, no request |
| `key-alt-legacy` | `ESC CR` | `BEFORE\nAFTER`, no request |
| `key-win-ctrl` | `ESC[13;28;13;1;8;1_` followed by `ESC[13;28;13;0;8;1_` | Ignored: `BEFOREAFTER`, no request |
| `key-win-ctrl-j` | `ESC[74;36;10;1;8;1_` followed by `ESC[74;36;10;0;8;1_` | Ignored: `BEFOREAFTER`, no request |
| `key-csi-shift-ignored` | `ESC[13;2u` | Ignored: `BEFOREAFTER`, no request |
| `key-csi-alt-ignored` | `ESC[13;3u` | Ignored: `BEFOREAFTER`, no request |
| `key-csi-ctrl-ignored` | `ESC[13;5u` | Ignored: `BEFOREAFTER`, no request |
| `key-lf-ignored` | `LF` | Ignored: `BEFOREAFTER`, no request |

Win32 parameters are Vk, Sc, Uc, Kd, Cs, Rc. These are observations for this installed native CLI and ConPTY mode, not claims about non-Windows terminals.

## Paste

`paste-80` and `paste-400` insert the first bracketed paste into an empty field. Both display the complete literal text and send the exact payload. The CLI has no 64-character boundary in these cases.

`paste-4k-collapsed` uses the default `tui.disable_paste_burst = false`: the source field displays `[Pasted Content 4096 chars]`; the eventual request contains the exact 4096-byte payload. `paste-4k-expanded` enables `tui.disable_paste_burst = true` in the isolated config: the placeholder disappears and the field renders literal text. The launch-only equivalent is `-c tui.disable_paste_burst=true`; the option and override are documented in the official [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) and [CLI reference](https://learn.chatgpt.com/docs/cli/reference).

The expanded 4K editor is still clipped by the 100×29 viewport. Its paste snapshot contains only 2350 of the 4096 `Z` characters; the request contains all 4096. This fixture does **not** prove that a current screen snapshot can confirm the complete pending payload. Full pending input must not be presented as source-confirmed solely because its prefix and caret are visible.

`paste-unicode` preserves three lines, Cyrillic, CJK, and the emoji. Its combining U+0301 after `e` is lost before reaching the fixture provider: the actual request contains plain `e`. This is visible in both the CLI source and the independently recorded request. `paste_contract` retains the original payload, actual request, and exact-match result; the mismatch is an observed native CLI/ConPTY limitation, not a successful round trip.

## Queue and steering

`queue-repeat` submits the same literal request twice. While response 1 is held open, Tab clears the source composer and displays `• Queued follow-up inputs`, `↳ BKS_SAME_REQUEST`, and the edit-queued hint. The provider still has only one conversation request. After response 1 completes, Codex prints a fresh `› BKS_SAME_REQUEST`, starts conversation request 2, and completes response 2. Both answers, including the first tail, remain in the source. A queued acknowledgement is not an assistant answer or an early Ready boundary.

`steer-repeat` repeats the same literal request with Enter during the held response. Codex cancels response 1, removes its partial assistant line from the live source transcript, prints the second `› BKS_SAME_REQUEST`, and starts conversation request 2 immediately. Only response 2 completes. The original partial bytes remain in the recording; their later removal must not fabricate a completed first answer.

`queue-then-steer` first queues `BKS_SAME_REQUEST` using Tab, then steers with `BKS_THIRD_STEER` using Enter. Conversation 1 is cancelled; conversation 2 is THIRD and completes; the original queued SAME runs afterward as conversation 3 and completes. There are three user submissions, two completed answers, and six total POSTs including titles. A replay helper with a fixed two-submission assertion cannot cover this case unchanged.

## Viewport and menu

`long-two-answers` streams 75 Unicode paragraphs beyond the viewport and then a second answer. `retained_first_tail` is `BKS_CODEX_LONG_END`; this marker is present in the source after the second answer.

`resize-menu` resizes a nonempty, multiline Unicode field from 100×29 to 48×19 and then 120×35; it then records slash-menu navigation and Escape dismissal, followed by `/quit`. It makes zero HTTP requests. Its ledger has real `resize` events; replay must apply those events to the model. These source captures do not constitute a human NVDA test.

## Full user echo and native clipboard transport

`paste-4k-unique-echo` is the actual `unique-4k-disabled-1793960517082-19516` recording: a 4096-byte payload with BEGIN/END markers and numbered `wNNNN` words. Its editor shows a suffix viewport; after submit the fresh source user echo contains all 678 tokens. Removing only layout wraps and indentation reproduces the original 4096 bytes exactly, as recorded in `full_echo_contract`. Its VT file matches the original capture byte for byte.

`native-clipboard-unicode` copies the successful `unicode-guard-matrix-packet-bmp-clipboard-1793961352310-20872` recording from `target/codex-production30/captures/final-packet-bmp-fixture/`. It invokes the actual `terminal_input::paste_bytes(payload, true)`, preserves framing/control bytes, encodes each non-control BMP scalar as VK_PACKET231 down/up, and leaves supplementary scalars as UTF8. All 23 cases and 408 UTF16 units reach the local provider exactly, including brackets, combining marks, `❤️`, `👩‍💻`, and adjacent `🙂🙂`. The helper's 24 CR become 24 LF in the exact original request; `/quit` exits with code 0.

The native clipboard ledger records actual **wire** inputs. Its `transport_contract` separately retains logical payload and normalization evidence; the original pre-transport `logical-paste-bytes.bin` and detailed verification remain beside the target capture. Wire packet bytes must not be replayed as logical source-editor text.
