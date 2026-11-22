# F0V: validációs riport (változatlan HEAD, 2. futás az 1.-hez képest)

> **Eredet:** a `cargo test -p cfa4-golden --release` (`crates/golden/tests/validation.rs`, PR #1427) írta a
> `target/f0v-report/` alá; ide változtatás nélkül másoltam, a fejléc és a lábjegyzet az egyetlen hozzáadott szöveg.
> A teszt a CPU-útvonalat méri, 320×180, 24 fps, 12 képkocka, procedurális jelenet. **Az ítélet `Incomplete`, nem PASS:**
> ebben a futásban az A/V drift ellenőrzés nem mért (`Skipped`), mert a procedurális jelenetben nincs villanás és sípszó, és mért
> hiányzó ellenőrzés sosem lehet PASS (tervszabály). A drift mérése külön, a `sync_flash_beep` fixture-rel megvan:
> `crates/golden/tests/sync_render.rs` (PR #1546; eltolás −0,02 ms, drift 0, a 80 ms-os késleltetést elkapja). Az a teszt nem része
> ennek a riportnak; a két futás egyesítése (egy riport mindkét jelenettel) még nincs kész.
> A futtató gép az Ubuntu 24.04 ellenőrző konténer, nem a referenciagép (Ubuntu 26.04); a referenciagépes futás PENDING.


**Verdict: Incomplete**

| Metadata | |
|---|---|
| harness | `cfa4-golden 0.10.0` |
| HEAD | `5d5cd4cce80f3678833aa07cad3df30397c53739` |
| rustc | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| OS | `Ubuntu 24.04.5 LTS` |
| kernel | `6.18.44-fc-v726` |
| CPU | `Intel(R) Xeon(R) Processor @ 2.10GHz` |
| GPU | `none (CPU path)` |
| reference sha256 | `a59be96c80a1c9fea0debcce49b963e15683d00036f5cfe3f35437bc791b6a34` |
| thresholds sha256 | `8841f671560d2f9870c465c3633383b9db94a4e8984e4268b16f6e6d267070a0` |

| Check | Status | Value | Limit | Gating |
|---|---|---|---|---|
| `video.frame_count` | Pass | 12 | 12 | yes |
| `video.frame_size` | Pass | equal | equal | yes |
| `timing.dropped` | Pass | 0 | 0 | yes |
| `timing.extra` | Pass | 0 | 0 | yes |
| `timing.non_monotonic` | Pass | 0 | 0 | yes |
| `timing.pts_error` | Pass | 0 | 0 | yes |
| `video.rgb.max_abs` | Pass | 0 | 0 | yes |
| `video.rgb.psnr` | Pass | inf | 96.000000 | yes |
| `video.alpha.max_abs` | Pass | 0 | 0 | yes |
| `video.rgb.ssim` | Pass | 1.000000 | 0.999900 | yes |
| `video.delta_e` | Pass | 0.000000 | 0.000000 | yes |
| `video.bit_identical` | Pass | true | informational | no |
| `audio.null_peak_dbfs` | Pass | -inf | -200.000000 | yes |
| `sync.av_drift` | Skipped | needs a flash-and-beep fixture rendered through the engine (F0V fixtures) |  | yes |

## Mutációs önteszt (a valódi renderelőn)

| Mutation | Changed the capture | Caught | Failed checks |
|---|---|---|---|
| `drop_frame` | true | true | video.frame_count, timing.dropped, timing.pts_error |
| `matrix_swap_601_709` | true | true | video.rgb.max_abs, video.rgb.psnr, video.rgb.ssim, video.delta_e |
| `pts_shift_one_frame` | true | true | timing.dropped, timing.extra, timing.pts_error |
| `alpha_loss` | true | true | video.alpha.max_abs |
| `round_to_8_bit` | true | true | video.rgb.max_abs, video.rgb.psnr, video.alpha.max_abs, video.rgb.ssim |
| `mute_channel` | true | true | audio.null_peak_dbfs |

> A hat mutáció mindegyike megváltoztatta a capture-t, és mindegyiket elkapta legalább egy kapuzó ellenőrzés.
> Zaj 10 futásban: minden mérőszám nulla, a futások bit-azonosak (CPU-út, egy gép).
