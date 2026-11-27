# Render validation: sync_flash_beep, engine render, run 1 against itself

**Verdict: Pass**

| Metadata | |
|---|---|
| harness | `cfa4-golden 0.10.0` |
| HEAD | `17173087bcbeedbec148b4576820d92464a287be` |
| rustc | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| OS | `Ubuntu 24.04.5 LTS` |
| kernel | `6.18.44-fc-v726` |
| CPU | `Intel(R) Xeon(R) Processor @ 2.10GHz` |
| GPU | `none (CPU path)` |
| reference sha256 | `411da0c68aa6e55cd94074fec41a308236fb4fd9358c538c8bf579d6d0ade462` |
| thresholds sha256 | `8841f671560d2f9870c465c3633383b9db94a4e8984e4268b16f6e6d267070a0` |

| Check | Status | Value | Limit | Gating |
|---|---|---|---|---|
| `video.frame_count` | Pass | 100 | 100 | yes |
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
| `sync.av_drift` | Pass | offset -0.020833 ms, drift 0.000000 ms | offset ≤ 0.100000 ms, drift ≤ 0.100000 ms | yes |
