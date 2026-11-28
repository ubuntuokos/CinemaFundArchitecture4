# cfa4-io-core

Shared media I/O core, layer L1 (CFA4-DECKLINK-READY-001, decision R1). It holds what every media
input/output process of CFA4 needs, independent of how it makes pictures: the live studio host
(`cfa4-obs`) uses it today, and a capture or playout host can use it later without depending on
libobs. Dependencies: `std`, `serde`, `thiserror`, `cfa4-frame` (L1), `cfa4-st2110`, `cfa4-color`, `cfa4-time` (L0). No `unsafe`.

## `ptp`

The PTP state `ptp4l` reports, read through `pmc` over its local socket (no network traffic),
plus the TAI time and the SDP clock signalling (`a=ts-refclk`, `a=mediaclk`) an ST 2110 sender
uses now. Moved unchanged from `cfa4-obs` (READY-V1); `cfa4_obs::ptp` re-exports it, so the
live studio and `live.studio.host.status` are unchanged.

```rust
pub const ST2059_DEFAULT_DOMAIN: u8 = 127;
pub const PMC_TIMEOUT: Duration;                               // 2 s
pub fn query() -> PtpReport;                                   // CFA4_PTP_DOMAIN or 127, PMC_TIMEOUT
pub fn query_with(domain: u8, timeout: Duration) -> PtpReport;
pub fn report_from(pmc: Result<String, String>, now: SystemTime, mac: [u8; 6]) -> PtpReport;
pub fn report_from_with(pmc: Result<String, String>, now: SystemTime, mac: [u8; 6], policy: Policy) -> PtpReport;
```

Nothing here is an error for the caller: without `pmc`, without an answer in time, or with an
unreadable answer, `PtpReport::available` is false, `reason` says why, and the sender clock
falls back to `localmac=…` / `sender`. `PtpReport` carries no studio state (no host, scene or
source), so any process can report it.

T1b: the report also carries `slaveOnly`, `traceable` (the grandmaster's PTP timescale, `timeTraceable` and
`clockAccuracy` ≤ 250 ns) and `warnings` (a `ptp4l` that may become the leader). `report_from_with(.., Policy)`
and `CFA4_PTP_REQUIRE_SLAVE_ONLY=1` refuse such a `ptp4l`. `pmc` also gets `GET PARENT_DATA_SET`. See
[`docs/live-studio-ptp.md`](../../docs/live-studio-ptp.md).

Environment: `CFA4_PTP_REQUIRE_SLAVE_ONLY`, `CFA4_PTP_DOMAIN` (domain for `query()`), `CFA4_PTP_INTERFACE` (the interface whose
MAC goes into `localmac`).

## `frame`

`Frame` (one picture: width, height, stride, `PixelFormat`, `pts_ns`, bytes), `Audio` (signed
32-bit integer PCM, channels interleaved) and `Frame::to_video_frame()`, which turns a picture
into the engine's `cfa4_frame::VideoFrame`. Picture formats:

| format | code | bytes per frame | layout |
|---|---|---|---|
| `Bgra8` | 1 | `stride × h` | packed B, G, R, A |
| `Rgba8` | 2 | `stride × h` | packed R, G, B, A, straight alpha |
| `Yuv422P10` | 3 | `4 w h` | planes of `u16` Y, Cb, Cr (same as `PixelData::Yuv16`, 10 bits, 4:2:2) |
| `Rgb12P` | 4 | `6 w h` | planes of `u16` R, G, B, 12 bits |
| `Uyvy8` | 5 | `stride × h` | packed Cb Y0 Cr Y1 |

## `ring`

The versioned shared-memory frame ring, moved here from `cfa4-obs` (READY-V2.1); `cfa4_obs::ring`
re-exports it. A producer process writes frames, readers (the engine, later the Qt6 GUI) read the
newest. Layout version 2 (little-endian), magic `CFA4RNG2`:

| where | bytes | field |
|---|---|---|
| header 0 | 8 | magic `CFA4RNG2` |
| 8 | 4 | version = 2 |
| 12 | 4 | slots (3) |
| 16 | 8 | slot size |
| 24 | 8 | `latest`, number of the newest frame (0 = none) |
| 32 | 4 | writer pid |
| 36 | 4 | generation (producer start count) |
| 40..64 | 24 | reserved |
| slot header 0 | 8 | `seq` (0 while being written) |
| 8 / 12 / 16 | 4 each | width / height / stride |
| 20 | 4 | pixel format code |
| 24 | 8 | `pts_ns` |
| 32 | 8 | picture bytes |
| 40 | 4 | `timecode`, SMPTE 12M BCD `HHMMSSFF` (0 = none) |
| 44 | 4 | `audio_len` bytes (0 = none) |
| 48 / 50 | 2 each | audio channels / audio format code (1 = s32) |
| 52 | 4 | flags: bit 0 interlaced, bit 1 bottom field first |
| 56..64 | 8 | reserved |
| slot 64 | picture | then the audio block |

Slots are `64 +` the picture bytes of the ring's format `+` the audio bytes the ring was created
for (`create_format`), 3 slots, at most 1 GiB. The writer marks a slot (`seq = 0`), writes
picture and audio, the slot header, then `latest`; a reader that sees a different `seq` after
reading drops the frame (never a torn one). Never crash: a version 1 file (`CFA4LIV1`) is the
error `ring version 1`; a frame with an unknown pixel or audio format code is dropped and counted
(`RingReader::dropped`); an inconsistent header is an error; nothing allocates from a header
before it is checked.
