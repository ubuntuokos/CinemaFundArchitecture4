# F0V: AC-01–AC-18 mérőtérkép

Állapotok: **MÉRŐ VAN** (kész, tesztelt), **MÉRŐ KÉSZ, FIXTURE VÁR** (a mérő megvan, a bemenet még nem megy át a motoron), **PENDING** (indokolva).
Minden sor a meglévő elemet nevezi meg; új crate nem készül (tulajdonosi szabály).

| AC | Mérő | Hol van | Állapot | Megjegyzés / indok |
|---|---|---|---|---|
| AC-01 Build | `Cargo.lock` hash (`render_environment.py`), két build összevetése | `python/cfa4_devtools/render_environment.py` | PENDING | két független tiszta build kell; a hash-gyűjtő kész |
| AC-02 Alap export | képkockaszám, PTS-lista, színmetaadat; PSNR/SSIM | `cfa4_metrics` (`psnr`, `timing`), fixture `sdr_1080p25` | MÉRŐ KÉSZ, FIXTURE VÁR | a decode→render→encode→reimport szál a következő lépés |
| AC-03 CPU-only | workflow-lista futtatása GPU nélkül | `crates/cinema-acceptance`, `crates/testkit` | PENDING | az F0-ban listázandó workflow-k még nincsenek felsorolva |
| AC-04 Renderhelyesség | `diff_u16/f32`, SSIM, ΔE, alfa-eltérés; golden | `cfa4_metrics`, `cfa4_golden::validation` | **MÉRŐ VAN** (CPU) | GPU-tolerancia, HDR/EXR golden PENDING |
| AC-05 Időzítés | `pts_diff`, `av_drift`, `seek_error_frames` | `cfa4_metrics::timing` | MÉRŐ KÉSZ, FIXTURE VÁR | `sync_flash_beep` fixture kész, motoros átvitel kell |
| AC-06 GPU-hibainjektálás | – | – | PENDING | referenciagép és hibainjektáló eszköz kell |
| AC-07 Erőforrások | `/proc`, NVML mintavétel | – | PENDING | rootless mérő még nincs; hosszú futás kell |
| AC-08 Resume/cancel | atomikus kimenet | `crates/export/src/atomic.rs` (meglévő) | PENDING | a SIGKILL-ciklus tesztje nincs |
| AC-09 Qt6 főszál | watchdog | `python/cfa4_qt` (nincs ilyen mérő) | PENDING | interfész-definíció a következő PR-ban |
| AC-10 Coexistence | előtte/utána pillanatkép | – | PENDING | `render_environment.py` bővítendő a fájlrendszer-pillanatképpel |
| AC-11a Licenc | SBOM, licencmátrix | F1-audit (PR #1436), `docs/audit/F1-01-SBOM-2026-10-25.md` | részben | az F1 doksik fedik a jelölteket |
| AC-11b Bemenet-biztonság | path traversal / symlink | – | PENDING | `crates/sandbox` (meglévő) a védelem, a teszt még nincs |
| AC-11c Sandbox | worker-teszt | `crates/sandbox` (meglévő) | PENDING | a sandbox még nincs alkalmazva (SECURITY-001 B3/B4) |
| AC-12 Ismert rések | `F0V-known-deviations.md`, kihagyási lista | ez a PR | MÉRŐ VAN (lista) | a géppel generált lista a riportban a következő lépés |
| AC-13 Audio | `null_test` dBFS csatornánként | `cfa4_metrics::audio` | **MÉRŐ VAN** | `audio_51/71_24bit` fixture-ök kész |
| AC-14 Preview–export | – | – | PENDING | kell hozzá a preview-render hívása a harnessből |
| AC-15 Projektmigráció | – | `crates/project/src/legacy.rs` (meglévő) | PENDING | nincs régi sémájú projekt-fixture |
| AC-16 Teljesítmény | renderidő, csúcsmemória | – | PENDING | 10 futás és baseline kell |
| AC-17 Feliratok | – | `crates/captions` (meglévő) | PENDING | a scope-döntés tulajdonosi (terv) |
| AC-18 Mozi minőség | MQ-1–MQ-8, ΔE X'Y'Z' | `cfa4_metrics` részben | PENDING | J2K orákulum és X'Y'Z' konverzió: CINECERT |
