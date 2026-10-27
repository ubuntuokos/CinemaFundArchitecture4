# CFA4 tervek, döntések és auditok – tartalomjegyzék

A projekt célja és prioritásai: [`VISION.md`](../../VISION.md). Ha az alábbiak bármelyike
ellentmond neki, a `VISION.md` az irányadó.

PR- és beolvasztási szabályok, gazda: [`docs/coordination.md`](../coordination.md).

> **Fontos:** a dokumentumok és a hozzájuk tartozó kód (szkriptek, Qt6-héj, Qt6-híd, csomagolás,
> tesztek) 2026-10-24 óta a `main`-ben vannak: a #3 … #33 PR-lánc a #35-tel került be
> (merge commit `6889664`). A régi PR-ok lezárva, az águk megmaradt.
> A dokumentumokban szereplő PASS/HOLD/OPEN állapotok a megírásuk napjára vonatkoznak.

## Áttekintés és döntések

| Dokumentum | Miről szól |
|---|---|
| [F9 kiadási döntés](F9-RELEASE-DECISION-2026-10-23.md) + [`f9-release-readiness.json`](f9-release-readiness.json) | **Összesítő:** minden kapu állapota (PASS/OPEN), PR-onként. Kiadás: HOLD |
| [ADR F8 – Qt6 / Rust motor](../adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md) | A Qt6-felület architektúrája és 7 lépéses átállási terve |
| [OpenTimelineIO integrációs terv](../plans/OTIO-INTEGRATION-2026-10-24.md) | Jóváhagyva: a meglévő Rust OTIO-kód fejlesztése 6 fázisban, a hivatalos `opentimelineio` csomag tesztorákulumként |
| [ADR 0003 – ASWF és GPL-3-as források](../adr/ADR-0003-aswf-and-conversion-sources.md) | Jóváhagyott terv: ASWF-projektek, a HandBrake cseréje, zárt bővítmények, gyártónkénti kódolás, A0–A7 szakaszok |
| [ADR 0005 – Médiahátterek](../adr/ADR-0005-media-backends.md) | A saját Rust-motor az alap; FFmpeg (≥ 9.0.2) és a GStreamer (alapból az Ubuntué, újabb verzióhoz elkülönített CFA4-futtatókörnyezet) kiegészítő háttér, automatikus útválasztóval (ADR 0004, `cfa4-route`); később további motorok |
| [ADR 0002 – GStreamer](../adr/ADR-0002-gstreamer-backend.md) | GStreamer mint kiegészítő háttérmotor (a saját Rust motor marad az alap), 6 lépéses terv |
| [Kdenlive-paritás](../kdenlive-parity.md) | A Kdenlive 26.08 képességei a CFA4-gyel összevetve, a Kdenlive-kód átvételének szabályai, K1–K35 teendők, MI-döntések és a 2026-10-25-i tulajdonosi döntések |
| [DCP-validációs terv](../plans/DCP-VALIDATION-2026-10-27.md), [DCP-00 képességaudit](DCP-00-CAPABILITY-AUDIT-2026-10-27.md), [DCP-01 forrás- és licencaudit](DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md) | CFA4-DCP-VALIDATION-001: a terv (még nem jóváhagyott), a meglévő CFA4-modulok leltára, crate-javaslat (D1–D7) és a dcp_inspect licenc- és függőségi jegyzéke (L1–L7); tulajdonosi jóváhagyásra vár |

## Fázisok

| Fázis | Dokumentum | Röviden |
|---|---|---|
| F1 | [f1-01-dependencies.md](f1-01-dependencies.md) | Külső függőségek és eredetük felmérése |
| F1 | [F1-01-SBOM-2026-10-25.md](F1-01-SBOM-2026-10-25.md) + [`f1-01-rust-inventory.json`](f1-01-rust-inventory.json) | Licenc- és eredetleltár (Rust 473 csomag, Python/Qt), RustSec-figyelmeztetések |
| F1 | [F1-02-SOURCE-CONFIG-GATE-2026-10-21.md](F1-02-SOURCE-CONFIG-GATE-2026-10-21.md) | Forrás- és konfigurációs ellenőrzés |
| F1 | [f1-02-vendor-migration.md](f1-02-vendor-migration.md) | `cfa4-docpath` / `cfa4-ui` beemelése; Ubuntu 26.04 és FFmpeg 9.0.2 döntés |
| F1 | [f1-source-detachment.md](f1-source-detachment.md) | A leválasztás összefoglalója (beolvasztva, #20) |
| F1 | [../development/f1-upstream-independence.md](../development/f1-upstream-independence.md) | CFA4 Linux-identitás (`io.github.ubuntuokos.cfa4`, `cfa4`) |
| F2 | [../development/linux.md](../development/linux.md) | Linux fejlesztői környezet, Ubuntu 26.04 AMD64 |
| F2 | [F2-01-RUNTIME-DEPS-2026-10-25.md](F2-01-RUNTIME-DEPS-2026-10-25.md) | Mért futásidejű függőségek 26.04-en: GLIBC 2.43, NEEDED, dlopen-lista (#22) |
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
