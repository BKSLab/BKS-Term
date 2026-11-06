# OpenCode 2.0.32 profile captures, 2026-11-05

Byte-exact installed Windows renderer in owned 110x36 ConPTYs. Every run used
an empty private home, config, data/cache/state, temp and working directory.
Auto-update, external provider access, plugins, MCP and tools were disabled;
no user credentials, settings or sessions were used. The profile's process
setting was `OPENCODE_CLI_CONFIG_CONTENT={"prompt":{"paste":"full"}}` and
the CLI ran with `--standalone`. Both accepted captures naturally exited with
`/exit`, code 0. Helpers and full provenance are under untracked
`target/opencode-profile28/captures/` (editor-full-profile3 / queued-default).

`editor.vt` SHA256:
`A3030AE1404ACEC122E9EE375DCBB4CEB2BAEFCE117A1138AB014C2A029BD257`.
The editor run made zero HTTP/model/tool requests. It records Unicode typing,
CSI-u Shift/Ctrl/Alt+Enter, Ctrl+J, empty rows, a first paste over 64 characters
with renderer word wrapping, Backspace/Left/Delete after that paste, clear,
Shell mode and Escape. The shell draft was
never submitted. `editor.json` retains actual input/poll times and expected
canonical text and UTF-16 caret at each source phase.

`queued.vt` SHA256:
`CC9D9FEA3825BAAB19272958C1AD2738D4C42888979B7E94DD46285CF38F1515`.
Two fixed responses came from an owned loopback OpenAI-compatible SSE provider.
The second prompt was queued with Ctrl+X then Enter during the first response.
The model names in the text are fixture prose, not a real provider claim.
`queued.json` retains actual input/poll times. There is no independent stable
idle interval before the second prompt: the first full body remains incomplete
under the existing conservative rule; only answer 2 emits Ready. The regression
requires both complete bodies, exact request pairing, separate IDs, previous/
next, independent disclosure and absence of queue chrome. Existing ordinary
two-response fixtures still require two Ready events.
