# Codex 0.164.0 repeated local-command capture

Fresh installed native Codex, real 100x29 ConPTY, managed argv `-c tui.disable_paste_burst=true` (alternate screen enabled). Each run uses an empty fixture cwd, new isolated HOME/USERPROFILE/APPDATA/XDG/short CODEX_HOME, cleared environment, and a fixed loopback Responses/SSE provider without authentication or tool calls. Session IDs and status paths belong only to these fixtures; no user session/account was read.

`status-repeated` records: empty composer -> `/status` -> unsent `BKS_COMMAND33_FIRST` for 500ms -> Enter/model answer -> `/status` -> identical `/status` -> `BKS_COMMAND33_SECOND`/answer -> explicit `/quit` exit0. Each status has a before-Enter menu/source snapshot, after-Enter snapshot and quiet polls. The first status and unsent draft make **0 conversation POSTs**. Both ordinary requests are exact; each complete run has **2 conversation + 2 title POSTs**. Model markers: `BKS_CODEX_RESPONSE_1_BEGIN/END` and `BKS_CODEX_RESPONSE_2_BEGIN/END`.

`status-repeated-model-picker` adds explicit `/model`, a real picker, Win32 Escape cancellation, and an empty Enter before the second ordinary request. The picker contains `Select Model and Effort` and no exact plain `/model` result echo. Cancellation acknowledges the empty composer; empty Enter makes no POST. Escape wire is VK27/scan1/Uc27 down/up, with logical ESC retained separately.

| Fixture | Status | Actual document UTF16 span `[start,end)` | Enter-to-quiet VT bytes `[start,end)` | Prior plain echo count | New raw `/status` count |
| --- | --- | --- | --- | --- | --- |
| status-repeated | 1 | [112,623) | [1688,2665) | 0 | 1 |
| status-repeated | 2 | [110,677) | [9738,10978) | 0 | 1 |
| status-repeated | 3 | [277,844) | [12190,12910) | 1 | 0 |
| status-repeated-model-picker | 1 | [79,557) | [2277,3221) | 0 | 1 |
| status-repeated-model-picker | 2 | [110,644) | [11096,12303) | 0 | 1 |
| status-repeated-model-picker | 3 | [277,811) | [13515,14235) | 1 | 0 |

The third invocation reuses the previous label cells: its new raw VT contains **no literal `/status`**, while the actual new empty-composer snapshot contains the exact plain label and complete body. Body2 and body3 are identical. The visible document retains one plain label, old status tails are displaced, and the next model response further displaces status. Label count or a fresh raw label cannot identify these repeated executions.

A supported source boundary is the exact plain `/status` line and actual body through the line before the confirmed composer first row (zero-based row25 in these quiet phases), trimming trailing blank lines and excluding completion menus/footer. Explicit accepted command input followed by a cleared empty composer, closed menu, matching source body and a stable snapshot is the available command acknowledgement. These quiet snapshots equal their after-Enter snapshots, with over2.7s since the last nonempty drain. The first body reports token usage0; the later bodies report112 and include Context window. A model picker and an empty Enter have no matching service-result body and should not create command blocks.

Each `.json` preserves every real input (accepted native wire plus separate logical bytes), every real drain including empty polls and timing, terminal protocol replies, and complete phase snapshots. `command_contract.commands` contains exact spans and command acknowledgements. VT files contain untouched original bytes (23834 +30078 bytes). No polls, output or timings were synthesized. `*-statusN-source.txt` contains the exact extracted source span; `*-result.json` records natural exit and request counts.

Helper and raw artifacts: `target/codex-command33/`. Build used its own `helper/Cargo.toml` and `build` target directory with `cargo build --offline`; app Cargo/shipping Rust was not modified. `validation-cleanup.json` validates contiguous events, exact original UTF8 phase text, exact original VT and source spans, plus all three generated private homes (including one unexported raw-Escape diagnostic run). All own CLIs quit naturally, listeners/PTYs closed, and zero own Codex processes remain. The unexported raw-Escape run did not acknowledge picker cancellation and is not a positive fixture.
