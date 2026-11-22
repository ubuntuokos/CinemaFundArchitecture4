# F0V: Küszöb@F0

Gépi fájl: [`F0V-thresholds.json`](F0V-thresholds.json) (séma `cfa4.render-validation/1`, a PR #1427 hozza). Itt a származtatás és a hatókör.

## Származtatás (`cfa4_golden::validation::derive_thresholds`)

A küszöb a **mért zajból** jön, nem becslésből. A zajt `measure_noise` méri legalább 10 egymás utáni futásból változatlan HEAD-en.

| Mérőszám | Szabály | Mért zaj | Küszöb |
|---|---|---|---|
| RGB legnagyobb eltérés (16 bit) | `ceil(4 × legrosszabb zaj)`; zérus zajnál bitpontos | 0 | 0 |
| PSNR | zaj-PSNR − 12 dB; zérus zajnál 96 dB padló | végtelen | 96,0 dB |
| SSIM | `1 − 4 × (1 − zaj-SSIM)`, legfeljebb 0,9999 | 1,0 | 0,9999 |
| Alfa legnagyobb eltérés | mint az RGB | 0 | 0 |
| ΔE 2000 | `ceil(4 × zaj)` | 0 | 0,0 |
| PTS legnagyobb hiba (tick) | zaj alapú | 0 | 0 |
| Eldobott/többlet képkocka | mindig 0 | – | 0 |
| Hang null-teszt csúcs | zaj + 12 dB, csendnél −200 dBFS | −∞ | −200 dBFS |

## Hatókör és korlát (őszintén)

- A küszöbök a **CPU-útra, egy gépre** érvényesek. Gép- és GPU-közötti zaj (AC-04 GPU-tolerancia) a referenciagépen mérendő: **PENDING**.
- Pontos kép-hash **nem kapu** (a libm-eltérések platformonként piros CI-t okoznának); a futás hash-ei csak a riportba kerülnek.
- A/V drift (AC-05): a mérő kész (`cfa4_metrics::timing::av_drift`, a villanás- és sípszó-érzékelővel, `validation::compare_with`).
  A `SyncSpec` ideiglenes határai (eltolás ≤ 20 ms, drift ≤ 1 ms) **nem mért zajból** származnak; a hivatalos érték az `sync_flash_beep`
  fixture motoron átvitt, ≥ 10 futásos renderéből jön (PENDING). `compare` (drift nélkül) ítélete továbbra is `Incomplete`.
- AC-07 (RSS/VRAM), AC-09 (főszál-szünet) és AC-16 (renderidő) küszöbe időalapú mérésből jön, nem ebből a mérésből: PENDING.
- Frissítés csak `CFA4_BLESS=1`-gyel és review-zott változtatással lehet.
