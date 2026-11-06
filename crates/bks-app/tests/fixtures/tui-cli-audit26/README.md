# Installed CLI captures, 2 November 2026

These are actual ConPTY bytes from Codex 0.164.0, Claude Code 2.1.240 and
OpenCode 2.0.32, each at 110 × 36. Every CLI received two fixed replies from
an owned loopback HTTP/SSE server. The replies are fixture text, not model
output. In particular OpenCode's canned second reply names a model; this is
not evidence about the installed CLI's real provider or model.

The capture processes cleared inherited environment variables and used new
private HOME, USERPROFILE, APPDATA, product configuration and working folders.
Codex used a private no-auth Responses provider; Claude used `--bare`, a fixed
fake API key and private onboarding/trust metadata; OpenCode used `--standalone`,
one allowed local provider, and disabled tools, integrations, plugins and model
catalog/update fetches. Other HTTP proxies pointed to refused loopback port 9.
No user credentials, configuration, real LLM service or user window was used.
All three CLIs subsequently exited naturally with code 0. Owned Codex daemon
processes were cleaned up by exact private audit path.

Each JSON records accepted literal input and original ConPTY drain/poll times.
All nonempty byte chunks are retained; redundant empty polls are thinned to
150 ms without inventing source frames or timestamps. VT files end at the
second completed reply, before exit commands. Session tests replay these events
through the production source acknowledgement, composer/history/menu,
submission and turn archive pipeline, retaining Ready events across frames.

Codex capture waited for its actual editor before typing. A previous probe
typed while only the startup heading existed and therefore could not establish
the first submission; it is not the accepted fixture. Endpoint-only replay also
missed intermediate busy controls and is not used to prove completion.

Claude's `● high · /effort` occupies the right aligned row directly above its
ruled composer. Its `✻ <word> for 0s` elapsed annotations use coherent neutral
muted glyph ink, while the separator after the spinner uses default ink. They
must remain source chrome, outside the answer archive and answer speech.
Quoted/fenced text, ordinary ink, different slots and unknown text are covered
by negative tests. No elapsed text itself grants completion or an action.

Working helpers, HTTP request evidence, startup/editor/menu/exit captures and
cleanup results are in `target/cli-audit26/others/`. Gemini was not installed in
any of 29 discovery/PATH directories; its installed smoke is skipped, while
existing bounded adapter fixtures remain automated coverage. NVDA and physical
keyboard/desktop behavior still require human acceptance.
