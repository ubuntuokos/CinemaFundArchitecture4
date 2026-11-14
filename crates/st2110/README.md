# cfa4-st2110

Clean-room SMPTE ST 2110 essence formats for the CFA4 live studio, phase **T0** of the ST 2110
addendum to CFA4-LIVE-STUDIO-001 (`docs/plans/CFA4-LIVE-STUDIO-001-ST2110.md`), and the clock part of phase **T1**. Layer L0:
depends only on `thiserror`. No `unsafe`, no sockets, clocks or threads; the network phases (T1+)
build on it. Written from the public SMPTE and IETF documents (plan D8). GStreamer is used only
as an external test oracle (`tests/gst_oracle.rs`).

## Specifications

The SMPTE texts are copyrighted and are not in the repository; the tests use their own data built
to the rules, never text from the standards.

| Document | Edition | Used for |
|---|---|---|
| SMPTE ST 2110-10 | 2022 | UDP size limits (1460 / `MAXUDP` up to 8960), dynamic payload types, `ts-refclk` (PTP, traceable, `localmac`; the 2017 traceable form is accepted), `mediaclk` (`direct=`, `sender`), `TSMODE`, `TSDELAY` |
| SMPTE ST 2110-20 | 2022 | packet layout (extended sequence number, up to three SRD headers), pgroups (Tables 1–4, including 4:2:0 and key), GPM and BPM (7 × 180 octets), interlace and PsF field split, `fmtp` parameters, `SSN` (2017 unless ALPHA or LOGS3) |
| SMPTE ST 2059-1 | 2021 | the SMPTE Epoch (the PTP epoch, TAI) and media-clock alignment |
| SMPTE ST 2110-21 | 2022 | `TP` (`2110TPN`, `2110TPNL`, `2110TPW`), `TROFF`, `CMAX` in the SDP; the timing models come with T4 |
| IETF RFC 3550 | — | RTP fixed header, CSRCs, padding |
| IETF RFC 8285 | — | header extensions (skipped; receivers must tolerate them, ST 2110-10:2022) |
| IETF RFC 4566, RFC 8866 | — | SDP lines, `rtpmap`, `fmtp` |
| IETF RFC 7273 | — | `ts-refclk`, `mediaclk` |
| IETF RFC 4175 | — | the base payload format of ST 2110-20 (oracle compatibility) |

## What it does

```rust
use cfa4_st2110::{Packetizer, Depacketizer, VideoFormat, planes_to_rows};
use cfa4_st2110::video::{Sampling, Depth, FrameRate, Colorimetry};

let fmt = VideoFormat::progressive(Sampling::YCbCr422, Depth::D10, 1920, 1080,
    FrameRate::new(50, 1)?, Colorimetry::Bt709);           // GPM, sender type W
let sdp_fmtp = fmt.to_fmtp().to_text();                    // "sampling=YCbCr-4:2:2; width=1920; …"
let rows = planes_to_rows(&fmt, &planes)?;                  // Y, Cb, Cr (u16) → pgroup rows
let mut tx = Packetizer::new(fmt.clone(), 98, ssrc, 0)?;
tx.packetize(&rows, rtp_timestamp, false, |datagram| socket.send(datagram))?;

let mut rx = Depacketizer::new(fmt)?;
rx.push(&datagram)?;                                        // malformed → Err, never a panic
if let Some(image) = rx.pop() { /* image.data, image.complete */ }
```

- **RTP** (`rtp`): parse and write the fixed header; CSRCs, RFC 8285 extensions and padding are
  skipped; every length is checked.
- **SDP** (`sdp`): sessions with media sections in order (round trip of canonical text),
  `rtpmap`, `fmtp` (`name=value` and standalone names, with or without the space after `;`),
  `ts-refclk`, `mediaclk`, redundant pairs (`a=group:DUP`, `a=mid`).
- **Video format** (`video`, `format`): every sampling, depth, colorimetry, TCS, range and
  packing token of ST 2110-20:2022; `exactframerate` reduced to the smallest numerator; the
  combinations the standards forbid are refused (XYZ below 12 bits, 4:2:0 interlaced or at 16
  bits, `segmented` without `interlace`, KEY without ALPHA or with TCS, `MAXUDP` with BPM).
- **Time** (`clock`, T1): PTP (TAI) time since the SMPTE Epoch (= the PTP epoch, ST 2059-1),
  `PtpTime::from_unix` with the TAI − UTC offset; the media clock `floor(t × rate)` and its 32-bit
  RTP timestamp; `FrameClock` numbers frames (or interlaced fields) from the Epoch and gives each
  one's start and exact timestamp from the image number, so 59.94 Hz advances 90 kHz by 1501 and
  1502 in turn. `PtpStatus::parse_pmc` reads `ptp4l` state from `pmc` (`TIME_STATUS_NP`,
  `DEFAULT_DATA_SET`, `PORT_DATA_SET`; linuxptp runs PTP, CFA4 only reads, plan D9);
  `sender_clocks` signals the grandmaster and `mediaclk:direct=0` when locked (slave port,
  grandmaster present, offset within 1 µs), otherwise `ts-refclk:localmac` and `mediaclk:sender`.
  No clock is read in this crate: the studio host reads `CLOCK_TAI` and `pmc` and passes them in.
- **Packing** (`pack`):
  - GPM fills each packet up to the UDP limit, joining short rows with the continuation bit (at
    most three SRD headers); BPM sends 1260 octets of sample data per packet and refuses
    pgroups that do not divide it (4:2:2 16 bit) or rows too short for three SRDs.
  - The last packet of a frame or field carries the marker bit; the 32-bit extended sequence
    number wraps.
  - Reassembly checks every SRD against the format (row, pgroup alignment, row length, field
    bit, order: data arrives top to bottom, left to right, so overlaps are refused). A lost
    packet leaves zeros and marks the image incomplete; a lost marker ends the image when the
    timestamp changes; duplicates and late packets are ignored; lost packets are counted.
  - `planes_to_rows` / `rows_to_planes` convert `u16` sample planes (4:4:4, RGB, XYZ, 4:2:2, key;
    8 to 16 bit) to and from pgroup rows, zero-filling the last pgroup of a row.

## Tests

```sh
cargo test -p cfa4-st2110
```

- Unit tests per module: header and SDP round trips, every pgroup table row, GPM and BPM
  layouts, interlaced fields with an odd height, loss, duplicates, malformed packets.
- `tests/gst_oracle.rs`: GStreamer `rtpvrawpay` packets reassemble bit-exactly, and GStreamer
  `rtpvrawdepay` turns our packets back into our frames (4:2:2 10-bit, `UYVP`). RTP goes through
  RFC 4571-framed files (`rtpstreampay` / `rtpstreamdepay`), so the test needs no network. Needs
  `gstreamer1.0-tools`, `-plugins-base` and `-plugins-good`; skips without them unless
  `CFA4_REQUIRE_ORACLES=1`.
- `tests/damaged_input.rs`: bit-flipped, truncated and random datagrams, SDP texts and `pmc`
  answers never panic, and a garbled `pmc` answer is never taken for a lock.
