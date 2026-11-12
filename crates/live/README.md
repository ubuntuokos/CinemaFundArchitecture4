# cfa4-live

Live graphics plumbing for CFA4 (credits plan C10, [`docs/credits.md`](../../docs/credits.md)
"Live graphics"). The engine's live runner (`crates/engine/src/live.rs`, `live.*` commands)
renders the program; this crate carries it to a broadcast program:

- `server`: HTTP/1.1 on **127.0.0.1 only**. `/` is the page for OBS's Browser Source (the
  program picture double-buffered, transparent; in the embedded audio mode the cue's sound in
  step), `/frame.png` the latest picture, `/status.json` the live status, `/audio` the cue's
  sound with byte ranges. Requests naming another host (DNS rebinding) and anything but GET /
  HEAD are refused; connections have timeouts and a limit; a failing connection never stops the
  server.
- `obs`: an obs-websocket 5 client (password authentication, requests, events skipped) that
  creates or points the Browser Source and, for the separate sound, a media source. OBS never
  drives CFA4.
- `ws`: the minimal WebSocket client it runs on (text messages, ping/pong, size limits);
  `sha1` only checks the server's handshake answer.

No workspace dependencies; `sha2` and `base64` from crates.io. Tests: the server over real
sockets, the OBS client against a stand-in OBS (`tests/obs_fake.rs`).
