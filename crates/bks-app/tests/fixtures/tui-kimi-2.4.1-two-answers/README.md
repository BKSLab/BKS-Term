# Kimi Code 2.4.1: actual terminal responses

Captured on 2026-11-02 from the installed Windows `kimi.exe` 2.4.1 in an owned
100×29 ConPTY. These are actual Kimi TUI bytes, not a reconstructed renderer.
`capture.json` and `queued-repeat.json` retain observed transport chunk offsets
and elapsed milliseconds, plus the explicit input phases and final composer
text/caret. The VT stops before the owned process cleanup.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `capture.vt` | 13254 | `EB2ECC88DAAE07E87236E19FAD4B6BA2F1CD805A0FD351F26067C30B6A65B215` |
| `queued-repeat.vt` | 14762 | `315AFE37C81E7DD06F9978E1BEAC16C3ED48CDB27F4F2A5BB364A735F0DA6F55` |

The first capture sends `audit-one`, waits for its response, then sends
`audit-two`. The second sends `audit-one` twice, with the second submission
accepted while the first response is still streaming and its composer remains
editable. The local fixture provider supplies distinct, deterministic assistant
texts and no tool calls. Each run makes exactly two loopback model requests;
there is no remote LLM request, user authentication, or tool approval.

Kimi runs with isolated user/config/data directories, an empty test workspace,
empty explicit skills directory, manual permissions, telemetry/cron/update
preflight disabled, and non-loopback HTTP(S) traffic directed to a refused proxy.
Its `openai` provider uses a dummy local API key and a `127.0.0.1` SSE endpoint.
The ephemeral listener and owned ConPTY are closed after capture.

Isolation and trust follow the official
[environment variable reference](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/env-vars.html):
`KIMI_CODE_HOME`, `KIMI_CODE_TRUST_WORKSPACE=1` (per-process trust for the owned
empty workspace), `KIMI_CODE_NO_AUTO_UPDATE=1`, and `KIMI_DISABLE_TELEMETRY=1`.
Local `--help` has no `--trust` flag. The provider template follows
[providers and models](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/providers.html)
and [configuration files](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/config-files.html).
Raw captures and the helper remain under `target/cli-audit26/kimi/`.

Expected behavior:

- Source-confirmed drafts, carets and both accepted prompt echoes are replayed
  through the production projector and insertion acknowledgements.
- The plain `●` response convention is recognized after an owned request.
  Quotes, fenced code, unowned lists and changed markers remain unclassified.
- Kimi's separately styled braille `Working…` row is a lifecycle boundary only
  immediately above a live, full-width rounded composer with matching source
  bounds. Changed styles, borders, source, hidden caret or protected input do
  not supply that proof.
- Both responses retain distinct IDs, exact body paragraphs, styles and working
  previous/next navigation. Both ordinary turns complete. In the queued capture
  the first turn is retained as incomplete: there is no independently stable
  idle boundary before Kimi starts the next response; the second turn completes.
  Its queue hint `↑ to edit · ctrl-s to steer immediately` remains visible as
  unclassified source output and is excluded from both archived answer bodies.

This acceptance covers CLI renderer bytes and production source/turn replay;
it does not claim an authenticated service, native GUI, or human NVDA test.
