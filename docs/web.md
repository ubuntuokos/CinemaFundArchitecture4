# CFA4 on the web

`apps/cfa4-web` runs the same engine and egui UI as the desktop app in the browser: the
workspace compiled to `wasm32-unknown-unknown`, started by eframe's web runner on WebGPU (WebGL2
fallback). It is a static site: one `.wasm`, its `wasm-bindgen` JavaScript glue, `index.html`, an
`AudioWorklet` script and an icon. Nothing is uploaded anywhere; media stay on the user's machine.

## Build and run

| Tool | Version | Install |
|---|---|---|
| Rust target | `wasm32-unknown-unknown` | `rustup target add wasm32-unknown-unknown` |
| wasm-bindgen CLI | exactly the `wasm-bindgen` crate version (0.2.129) | `cargo install wasm-bindgen-cli --version 0.2.129 --locked` |
| wasm-opt (optional) | any recent binaryen | `brew install binaryen`; used by `cargo xtask web` when found |

```sh
cargo xtask web                 # release build → target/web/dist (index.html, cfa4_web.js, cfa4_web_bg.wasm, …)
cargo xtask web --dev           # unoptimised build (faster to compile, slow to run)
cargo xtask web --serve 8765    # build, then serve dist on http://127.0.0.1:8765/
```

`--serve` is a tiny localhost-only static server that sends `Cross-Origin-Opener-Policy` /
`Cross-Origin-Embedder-Policy`, so the page is cross-origin isolated like a production deployment
should be. Any static server works (`python3 -m http.server -d target/web/dist 8765`); it must
serve `.wasm` as `application/wasm` for streaming compilation.

The web crate is empty on non-wasm targets, so `cargo test --workspace` and clippy are
unaffected; `cargo xtask wasm` (part of `cargo xtask ci`) checks it, with `cfa4-ui-egui`, for
`wasm32-unknown-unknown`.

URL flags: `?empty` (no demo project), `?norecover` (don't reopen the auto-saved project),
`?fresh` (also don't restore media kept in OPFS), `?cpu` (CPU compositor), `?webgl` (WebGL2 only;
the page reloads with it when WebGPU is present but fails to start), `?nowebcodecs` (decode with
CFA4's own decoders only).

If the app panics after start-up (or runs out of memory), `index.html` shows a "CFA4 stopped
working" overlay with the panic message and a Reload button instead of a frozen canvas, and sets
`window.cfa4Load.fatal`. Code the web build runs must not call `std::env::temp_dir`,
`std::time::{Instant, SystemTime}::now`, `std::thread::sleep`/`spawn` or `std::process::id`: they
panic on `wasm32-unknown-unknown` (use `cfa4_engine::temp_dir`, `web_time`, or a `cfg`).

## How the desktop pieces map to the browser

| Desktop | Web |
|---|---|
| frame worker threads | `FrameServer::pump`: queued frame jobs run on the UI thread between egui frames, highest priority first, within a time budget (24 ms while playing, 40 ms otherwise) |
| export on a worker thread | `cfa4_export::Exporter`: the export advances one frame per step from `Session::pump_jobs`, 30 ms per UI frame; progress shows in the header like on the desktop |
| `std::fs` (`FsServices`) | `fs::WebServices`: a virtual file table. Picked/dropped files are `File` handles (never copied); files the app writes are kept in memory and offered as downloads |
| file dialogs | File System Access `showOpenFilePicker` where available, else `<input type=file>`; drop files anywhere on the page |
| crash-recovery journal | OPFS: `recovery/snapshot.fcproj` every 5 s while there are unsaved changes, plus copies of imported media in `media/`; the next visit reopens the snapshot |
| cpal output | WebAudio `AudioWorklet` |
| VideoToolbox & co. | WebCodecs `VideoDecoder` |
| TCP control channel / MCP | `window.cfa4` (below) |

### Threads

wasm threads need a cross-origin isolated page **and** a wasm build with atomics (a nightly
`build-std` toolchain), so this build is single-threaded everywhere: frame rendering, decoding,
mixing and encoding are cooperative on the UI thread, and `rayon` runs inline.
`cfa4.info()` reports `crossOriginIsolated` so a threaded build can choose at start-up.
Commands that need a thread on the desktop (render previews, proxies, Project Manager, mask
tracking) report an error on the web for now.

### Media reads

Containers are opened through `Services::reader` (a `cfa4_media::ByteReader`): MP4/MOV and
Matroska read their index at import and single samples afterwards, so a multi-gigabyte file is
never loaded whole. Browser `Blob` reads are asynchronous, so `BlobReader` keeps a 384 MiB LRU cache
of 1 MiB chunks: a read whose chunks are missing starts fetching them (plus three chunks of
read-ahead) and fails with `WouldBlock`, setting `cfa4_media::pending`. The frame server then
drops that frame's result and retries the job after the bytes arrive (the fetch wakes the UI);
the stepped exporter re-renders the frame; an import retries `file.import`. Before importing, the
container index is prefetched (an MP4's `moov` box wherever it is, a small Matroska file whole).
Stills, WAV and compressed-audio files are decoded whole, as on the desktop.

Limits: Matroska/WebM files larger than the chunk cache import slowly (opening scans every
cluster header); a file the user picked is only readable while the page is open, unless its OPFS
copy exists (files up to 4 GiB are copied in the background).

### Decoding

CFA4's own decoders (H.264, HEVC, VP9, AV1, ProRes, DNxHR, MJPEG, AAC, Opus…) run in the
browser unchanged. When the browser has WebCodecs, H.264, HEVC, VP9 and AV1 video in MP4/MOV is
decoded by the browser's (usually hardware) `VideoDecoder` instead: `webcodecs::reader_opener` is
registered ahead of the built-in openers and opens such files as a `WcSource`, whose audio and
media info come from our `Mp4Source`. WebCodecs is asynchronous, so a frame request feeds samples
to the decoder and returns "pending"; the decoder's output callback converts the `VideoFrame` to
RGBA (drawn on an `OffscreenCanvas`) into a per-source frame cache, and the retried request finds
it. Codec support is probed at start-up with `VideoDecoder.isConfigSupported`; any decoder error
switches that source to our decoder.

### Audio

`audio::Handle` implements `AudioOut`. The UI thread mixes the sequence 250 ms ahead in 2048-frame
blocks and posts them to the worklet (`web/audio-worklet.js`), which counts the frames it really
played and reports them with its `currentTime`. `played_frames` extrapolates from the last report
on the audio context's clock, never past what was queued — so the audio clock is the playback
master as on the desktop, and a starved worklet holds the playhead. Browsers start audio suspended
until the first click or key press; until then playback runs on the wall clock.

### GPU

eframe creates a WebGPU device when the browser has one, else WebGL2. CFA4's GPU compositor
(`cfa4-gpu`) is used on WebGPU; on WebGL2 frames are composited on the CPU. Check
`cfa4.info().backend` and `.compositor`.

## `window.cfa4`: the agent / test API

The control channel of the desktop app (`docs/control-protocol.md`) as promises. Results resolve
with the method's `result`, errors reject.

| Call | |
|---|---|
| `cfa4.execute(command, params)` | run an engine command (same ids and params as MCP / `cfa4-cli`) |
| `cfa4.request(method, params)` | any control method: `ui.inspect`, `ui.click`, `ui.key`, `ui.playback`, `ui.menu.invoke`… |
| `cfa4.commands()` | the command list |
| `cfa4.inspect()` | `ui.inspect` |
| `cfa4.screenshot(params)` | `{pngBase64, width, height}` of the canvas (or `{panel}`) |
| `cfa4.importFiles(files)` | import `File` / `FileList` / `File[]`; resolves `{items, errors, paths}` |
| `cfa4.importUrl(url, name?)` | fetch a URL and import it (tests) |
| `cfa4.openProject(fileOrUrl)` | open a `.fcproj` |
| `cfa4.files()` | the virtual files: `{path, size, kind}` |
| `cfa4.readFile(path)` | a file's bytes (`Uint8Array`), e.g. an export |
| `cfa4.info()` | backend, compositor, isolation, OPFS, WebCodecs support and counters, start-up time |

`window.cfa4Load` holds `{wasmMs, readyMs}` (module fetch+compile, and until the app runs).

```js
await cfa4.importUrl("/clip.mp4");
await cfa4.execute("file.newSequence", {fromItem: 48});
await cfa4.request("ui.playback", {action: "play"});
const {job} = await cfa4.execute("file.exportMedia", {format: "h264", path: "/exports/out.mp4"});
```

## Browser test

`apps/cfa4-web/tests/smoke.mjs` drives headless Chrome over the DevTools protocol (Node ≥ 22,
no npm packages): load, demo playback, import of a generated MP4, playback, H.264 export
(download). It writes screenshots and `report.json`:

```sh
cargo xtask web --serve 8765 &
ffmpeg -f lavfi -i testsrc2=size=640x360:rate=30 -f lavfi -i sine -t 3 -c:v libx264 -pix_fmt yuv420p -c:a aac target/web/dist/web-test.mp4
node apps/cfa4-web/tests/smoke.mjs --url http://127.0.0.1:8765/ --media web-test.mp4 --out target/web/smoke
```
