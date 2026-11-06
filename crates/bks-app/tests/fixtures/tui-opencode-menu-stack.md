# Model and variant menu stack, 100x29

`tui-opencode-menu-stack.vt` is a 42,476-byte output capture from isolated
OpenCode 2.0.32. SHA256:
`98987FFEEEC91F4C822B52BB6F4FAA55EF36262364F7BEBA715E2A0C4FA31B19`.
Fourteen additional local model descriptions make the model popup scroll.
Choosing a model opens the nested variant popup. Escape closes it before
reopening the model list. No prompt/model/tool request or user profile was used.
The capture helper is `target/tui-followup/capture/`, with private files in
`isolated-menu/`; the source process was terminated after capture.

| Phase | Exclusive byte end |
| --- | ---: |
| Startup | 11,384 |
| Model completion | 11,639 |
| Model menu | 16,347 |
| Next item | 16,596 |
| Nested variant menu | 17,493 |
| Closed variant | 22,038 |
| Reopened model menu | 27,001 |
| Navigation | 27,248 |
| Second variant | 28,185 |
| Closed variant | 32,730 |
| Reopened model menu | 37,681 |
| Navigation | 37,931 |
| Closed menu | 42,476 |

The regression overlays two source-equivalent neutral chat panels by VT writes
at rows 3–7 and 25–28, columns 2–99. This intentionally splits the canvas into
several shades. It is labelled as a derived fixture, not a byte-exact capture
of a conversation. Grid and native-reader tests keep their text/styles aligned.

`tui-opencode-error-menu.vt` is a separate byte-exact 120,417-byte capture of
the model menu over two local synthetic failures, from
`target/tui-errors-capture/run-04/`. Model-menu end: 116,007; closed end: 120,417.
SHA256: `847E89E4F6D073D57A18A73C03470FA3D9CBEEB6C0771690842F5A390FEC68D2`.
Only a loopback provider served fixed errors; no external model, account,
credentials or user session was used. The process and listener were closed.
