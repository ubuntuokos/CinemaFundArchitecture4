# cfa4-io-core

Shared media I/O core, layer L1 (CFA4-DECKLINK-READY-001, decision R1). It holds what every media
input/output process of CFA4 needs, independent of how it makes pictures: the live studio host
(`cfa4-obs`) uses it today, and a capture or playout host can use it later without depending on
libobs. Dependencies: `std`, `serde`, `cfa4-st2110` (L0). No `unsafe`.

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
```

Nothing here is an error for the caller: without `pmc`, without an answer in time, or with an
unreadable answer, `PtpReport::available` is false, `reason` says why, and the sender clock
falls back to `localmac=…` / `sender`. `PtpReport` carries no studio state (no host, scene or
source), so any process can report it.

Environment: `CFA4_PTP_DOMAIN` (domain for `query()`), `CFA4_PTP_INTERFACE` (the interface whose
MAC goes into `localmac`).

## Next

The versioned shared-memory frame ring (ring v2: more pixel formats, timecode, audio) moves here
from `cfa4-obs` in READY-V2.
