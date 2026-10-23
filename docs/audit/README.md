# CFA4 tervek, döntések és auditok – tartalomjegyzék

A projekt célja és prioritásai: [`VISION.md`](../../VISION.md). Ha az alábbiak bármelyike
ellentmond neki, a `VISION.md` az irányadó.

> **Fontos:** ezek a dokumentumok 2026-10-23-án kerültek át a mellékágakról a `main`-be, hogy a
> tervek ne vesszenek el. A hozzájuk tartozó **kód és szkriptek** (pl. `scripts/dev/*`,
> `scripts/ci/*`, `scripts/release/*`, `python/cfa4_devtools/qt_app.py`,
> `qt_control_bridge.py`, `native_media_e2e.py`) **még nincsenek a `main`-ben**: a piszkozat
> PR-okban (#17, #20, #23–#30) és az `audit/*`, `chore/*`, `design/*`, `fix/*` ágakon vannak.
> A legteljesebb állapot az `audit/f9-evidence-release-hold` ágon található.
> A dokumentumokban szereplő PASS/HOLD/OPEN állapotok a megírásuk napjára vonatkoznak.

## Áttekintés és döntések

| Dokumentum | Miről szól |
|---|---|
| [F9 kiadási döntés](F9-RELEASE-DECISION-2026-10-23.md) + [`f9-release-readiness.json`](f9-release-readiness.json) | **Összesítő:** minden kapu állapota (PASS/OPEN), PR-onként. Kiadás: HOLD |
| [ADR F8 – Qt6 / Rust motor](../adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md) | A Qt6-felület architektúrája és 7 lépéses átállási terve |

## Fázisok

| Fázis | Dokumentum | Röviden |
|---|---|---|
| F1 | [f1-01-dependencies.md](f1-01-dependencies.md) | Külső függőségek és eredetük felmérése |
| F1 | [F1-02-SOURCE-CONFIG-GATE-2026-10-21.md](F1-02-SOURCE-CONFIG-GATE-2026-10-21.md) | Forrás- és konfigurációs ellenőrzés |
| F1 | [f1-02-vendor-migration.md](f1-02-vendor-migration.md) | `craft-docpath` / `craft-ui` beemelése; Ubuntu 26.04 és FFmpeg 9.0.2 döntés |
| F1 | [f1-source-detachment.md](f1-source-detachment.md) | A leválasztás összefoglalója (beolvasztva, #20) |
| F1 | [../development/f1-upstream-independence.md](../development/f1-upstream-independence.md) | CFA4 Linux-identitás (`io.github.ubuntuokos.cfa4`, `cfa4`) |
| F2 | [../development/linux.md](../development/linux.md) | Linux fejlesztői környezet, Ubuntu 26.04 AMD64 |
| F3 | [f3-cfa4-release-chain-2026-10-23.md](f3-cfa4-release-chain-2026-10-23.md) | Saját kiadási lánc, négy csomagformátum |
| F5 | [F5-STAGED-RUNTIME-2026-10-23.md](F5-STAGED-RUNTIME-2026-10-23.md) | Csomagok próbatelepítése elkülönítve; tiszta VM-teszt terve |
| F6 | [F6-XDG-SOURCE-2026-10-23.md](F6-XDG-SOURCE-2026-10-23.md) | Asztali integráció: menü, ikon, fájltársítás, AppStream |
| F7 | [F7-NATIVE-MEDIA-GATE-2026-10-23.md](F7-NATIVE-MEDIA-GATE-2026-10-23.md) + [`f7-media-capability-status.json`](f7-media-capability-status.json) | Mi működik ténylegesen a médiakezelésben |
| — | [HEIF-HEIC-LINUX-2026-10-22.md](HEIF-HEIC-LINUX-2026-10-22.md) | HEIF/HEIC képek importja és exportja |

Az F0-nak, F2-nek, F4-nek és F8-nak a fentieken kívül nincs külön auditdokumentuma; ezek
az azonos nevű ágakon és PR-okban vannak.

## Igazolt hardveres mérések (RTX A1000, Kubuntu 26.04.1)

| Dokumentum | Eredmény |
|---|---|
| [R0-FFMPEG-HWACCEL-2026-10-22.md](R0-FFMPEG-HWACCEL-2026-10-22.md) | FFmpeg 9.0.2 NVENC/NVDEC/Vulkan/OpenCL/libplacebo: PASS (#14) |
| [R0-NATIVE-MEDIA-E2E-2026-10-22.md](R0-NATIVE-MEDIA-E2E-2026-10-22.md) | A CFA4 alkalmazás H.264/HEVC hardveres útvonala és a tar.gz kézi próba: részleges PASS (#16) |
