# F0V: AC-01–AC-18 mérőtérkép

Állapotok: **MÉRŐ VAN** (kész, tesztelt), **MÉRŐ KÉSZ, FIXTURE VÁR** (a mérő megvan, a bemenet még nem megy át a motoron), **PENDING** (indokolva).
Minden sor a meglévő elemet nevezi meg; új crate nem készül (tulajdonosi szabály).

| AC | Mérő | Hol van | Állapot | Megjegyzés / indok |
|---|---|---|---|---|
| AC-01 Build | `Cargo.lock` hash (`render_environment.py`), két build összevetése | `python/cfa4_devtools/render_environment.py` | PENDING | két független tiszta build kell; a hash-gyűjtő kész |
| AC-02 Alap export | képkockaszám, PTS-lista, színmetaadat; PSNR/SSIM | `cfa4_metrics` (`psnr`, `timing`), fixture `sdr_1080p25` | MÉRŐ KÉSZ, FIXTURE VÁR | a decode→render→encode→reimport szál a következő lépés |
| AC-03 CPU-only | workflow-lista futtatása GPU nélkül | `crates/cinema-acceptance`, `crates/testkit` | PENDING | az F0-ban listázandó workflow-k még nincsenek felsorolva |
| AC-04 Renderhelyesség | `diff_u16/f32`, SSIM, ΔE, alfa-eltérés; golden | `cfa4_metrics`, `cfa4_golden::validation` | **MÉRŐ VAN** (CPU) | GPU-tolerancia, HDR/EXR golden PENDING |
| AC-05 Időzítés | `pts_diff`, `av_drift`, `rising_edges`, `beep_onsets`, `seek_error_frames` | `cfa4_metrics::timing`, `cfa4_golden::validation::compare_with`, `crates/golden/tests/sync_render.rs` | **MÉRŐ VAN** (motoron átvitt `sync_flash_beep`: eltolás 0,02 ms, drift 0) | VFR, loop, trim és seek-teszt még PENDING |
| AC-06 GPU-hibainjektálás | – | – | PENDING | referenciagép és hibainjektáló eszköz kell |
| AC-07 Erőforrások | `cfa4_metrics::resource::{read_status, growth_after}` (RSS-növekedés bemelegítés után) | `crates/metrics/src/resource.rs` (F0V-3 PR) | MÉRŐ KÉSZ (a mintavevő hurok és a 10 perces futás hiányzik) | VRAM (NVML) és ASan/heaptrack: PENDING, referenciagép kell |
| AC-08 Resume/cancel | atomikus kimenet | `crates/export/src/atomic.rs` (meglévő) | PENDING | a SIGKILL-ciklus tesztje nincs |
| AC-09 Qt6 főszál | `cfa4_metrics::resource::max_pause_ms` (szívverés-sorozat) | `crates/metrics/src/resource.rs` (F0V-3 PR) | MÉRŐ KÉSZ (a Qt oldali szívverés-szonda hiányzik) | `python/cfa4_qt`: szonda és küszöb PENDING |
| AC-10 Coexistence | előtte/utána pillanatkép | – | PENDING | `render_environment.py` bővítendő a fájlrendszer-pillanatképpel |
| AC-11a Licenc | SBOM, licencmátrix | F1-audit (PR #1436), `docs/audit/F1-01-SBOM-2026-10-25.md` | részben | az F1 doksik fedik a jelölteket |
| AC-11b Bemenet-biztonság | path traversal / symlink | – | PENDING | `crates/sandbox` (meglévő) a védelem, a teszt még nincs |
| AC-11c Sandbox | worker-teszt | `crates/sandbox` (meglévő) | PENDING | a sandbox még nincs alkalmazva (SECURITY-001 B3/B4) |
| AC-12 Ismert rések | `F0V-known-deviations.md`, kihagyási lista | ez a PR | MÉRŐ VAN (lista) | a géppel generált lista a riportban a következő lépés |
| AC-13 Audio | `null_test` dBFS csatornánként; ST 428-2 paraméterek (24 bit, 48/96 kHz, 1–16 csatorna, azonos hossz, nincs teljes skála fölötti minta) | `cfa4_metrics::audio`, `cfa4_golden::validation::compare_with` (`Extras::audio_bits`) | **MÉRŐ VAN** | `audio_51/71_24bit` fixture-ök kész. A csomagszintű szabály már létezik (`DCP.AUD.001`, `cfa4-dcp-validate`): ez ugyanazt a renderkimeneten méri, nem másolja a validátort. A 6/8/16 csatorna (ST 429-2) csak tájékoztató. Az MCA-címkéket a `cfa4-mxf` még nem adja (nyitott) |
| AC-14 Preview–export | – | – | PENDING | kell hozzá a preview-render hívása a harnessből |
| AC-15 Projektmigráció | `crates/golden/tests/legacy_project.rs`: nyitás → mentés → újranyitás, a projekt-JSON egyezése és a render-hash | `crates/format` (`decode`, `encode`, `MIGRATIONS`), `crates/project/src/legacy.rs` | MÉRT (részben) | a fixture-ök a mostani projektből származnak (2..aktuális séma, a boríték előtti alak, az upstream tag); valódi, régi build által mentett fájl nincs a repóban (PENDING: a tulajdonos adjon) |
| AC-16 Teljesítmény | renderidő, csúcsmemória | – | PENDING | 10 futás és baseline kell |
| AC-17 Feliratok | – | `crates/captions` (meglévő) | PENDING | a scope-döntés tulajdonosi (terv) |
| AC-18 Mozi minőség (csak szoftveres referencia) | MQ-1–MQ-8, ΔE X'Y'Z' | `cfa4_metrics` részben | PENDING | J2K orákulum és X'Y'Z' konverzió: CINECERT. Nincs mozi szerver/vetítő: az állítás legfeljebb „szoftveresen igazolt, szabványkonform”, nem „moziban lejátszva” |

**Hatókör (tulajdonosi döntés, 2026-11-21):** a mozi célú elfogadás (AC-12, AC-18, DCP/KDM) kizárólag szoftveres referenciával mérhető, mert nincs mozi szerver és vetítő. A riport nyelve ezért „szoftveresen igazolt, szabványkonform”; „moziban lejátszva” állítás nem tehető.
