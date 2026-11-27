# PTP for the live studio (linuxptp, `slaveOnly`)

CFA4-LIVE-STUDIO-001 ST 2110 addendum, phase T1b (V-04, V-05). CFA4 never runs a PTP clock of its own and
is never the PTP leader (plan D9, ST 2110-10:2022 §7.2): `ptp4l` and `phc2sys` from `linuxptp` do the
work, CFA4 only reads their state through `pmc` ([`crates/io-core`](../crates/io-core/README.md), shown as
`ptp` in `live.studio.host.status`).

## `ptp4l.conf` (ST 2059-2 profile, follower only)

```ini
[global]
# Never become the PTP leader (ST 2110-10:2022 7.2). Without this line a CFA4 machine with the best
# clock on the network would take over and move every other device.
slaveOnly               1
# ST 2059-2 default domain; CFA4_PTP_DOMAIN must match.
domainNumber            127
# ST 2059-2: 4 announce messages per second, 8 sync messages per second.
logAnnounceInterval     -2
logSyncInterval         -3
logMinDelayReqInterval  -3
announceReceiptTimeout  3
delay_mechanism         E2E
network_transport       UDPv4
# time_stamping         hardware
# (hardware time stamping only with a NIC that supports it; software otherwise)

# The interface that carries the ST 2110 streams.
[enp5s0]
```

`phc2sys -s enp5s0 -c CLOCK_REALTIME -w -n 127` keeps the system clock on UTC: `-w` waits for `ptp4l` and applies the
grandmaster's `currentUtcOffset` (do not add `-O 0`, which would put TAI into `CLOCK_REALTIME`), and `-n` must be the same
domain as `ptp4l`. CFA4 adds `currentUtcOffset` to the UTC system time to get TAI. For AES67 equipment the message rates must
stay inside the AES67 Media Profile limits (AES-R16-2016); the T2 phase checks this.

## What CFA4 reports and does

| `pmc` field | In the report | Meaning |
|---|---|---|
| `slaveOnly` (`DEFAULT_DATA_SET`) | `slaveOnly`, `warnings` | `0` means `ptp4l` may become the leader: a warning that names the fix above |
| `ptpTimescale`, `timeTraceable` (`TIME_PROPERTIES_DATA_SET`) | `traceable` | two of the three conditions for the traceable form |
| `gm.ClockAccuracy` (`PARENT_DATA_SET`) | `traceable` | third condition: code `0x20`–`0x22` (25 ns to 250 ns) |

**Enforcing `slaveOnly`.** With `CFA4_PTP_REQUIRE_SLAVE_ONLY=1` a `ptp4l` that reports `slaveOnly 0` is not
used: the report says why, `locked` is false, and the sender signals its own timebase
(`ts-refclk:localmac=…`, `mediaclk:sender`). A `ptp4l` that does not report the field is not refused.

**`ts-refclk` form.** The SDP names the grandmaster and the domain (`ptp=IEEE1588-2008:<EUI-64>:<domain>`).
The shorter `ptp=IEEE1588-2008:traceable` form is used only when a caller asks for it
(`RefClockForm::TraceableWhenAllowed`) **and** all three conditions hold: PTP timescale, `timeTraceable`,
`clockAccuracy` 250 ns or better. An unanswered condition counts as not met.

**`TSMODE`.** A program output is synthetic (composited) content. With the frame clock locked to PTP and a
locked `ptp4l`, the timestamp is N × T<sub>FRAME</sub> within ±T<sub>FRAME</sub>, so the sender may signal
`TSMODE=SAMP` (`cfa4_st2110::clock::sender_ts_mode`); otherwise `NEW`. This is used by the sender phase T4.
