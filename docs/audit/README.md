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
| [Feliratozó terv FINAL 1.2](../plans/SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md) | Jóváhagyva (2026-10-30): a [FINAL 1.1](../plans/SUBTITLE-STUDIO-PLAN-001-FINAL-1.1.md) és a CFA3-összevetés egyesítése a meglévő `cfa4-captions`/`cfa4-project` kódra építve; a D1–D4 tulajdonosi döntések (2026-10-30) beépítve |
| [Fejezetjelölők és DASH manifest terv](../plans/CHAPTERS-DASH-2026-10-30.md) | Jóváhagyva (2026-10-30): MP4/MOV/Matroska fejezetek a `MarkerKind::Chapter` markerekből (F1–F8) és DASH MPD a HLS export mellé, közös CMAF-szegmensekkel (D1–D5); a muxide csak minta |
| [Event Fabric terv 1.0](../plans/CFA4-EVENT-FABRIC-001-1.0.md) + [CFA4-illesztési melléklet](../plans/CFA4-EVENT-FABRIC-001-1.0-CFA4-ILLESZTES.md) | Jóváhagyva (2026-11-02): `CFA4-EVENT-FABRIC-001`, közös eseménykommunikáció NATS Core + JetStream, Valkey és Rust Core alapon (F1–F8, EF-001–EF-025); a melléklet a CFA4-szabályokkal való eltéréseket és a nyitott kérdéseket rögzíti |
| [Külső időkód-szinkron: LTC és MTC](../plans/TIMECODE-SYNC-LTC-MTC-2026-11-11.md) | Jóváhagyva (2026-11-13): LTC és MTC generate és chase, clean-room `cfa4-tcsync` (L1) + `cfa4-tcsync-io` (L5), Qt6-felület; fázisok TC-0…TC-8, D1–D3 a javaslatok szerint eldöntve |
| [ADR 0003 – ASWF és GPL-3-as források](../adr/ADR-0003-aswf-and-conversion-sources.md) | Jóváhagyott terv: ASWF-projektek, a HandBrake cseréje, zárt bővítmények, gyártónkénti kódolás, A0–A7 szakaszok |
| [ADR 0009 – CFA4 File Services](../adr/ADR-0009-cfa4-file-services.md) | Elfogadva (2026-11-08): közös fájlszolgáltatás; az adatút (másolás, BLAKE3-kivonat, lezárás, ellenőrzés) Rustban, `files.*` parancsként a `cfa4-cli serve`-en, a katalógus Pythonban (`cfa4_files`) az üzleti adatbázisban, gyökérfüggetlen útvonalakkal; FS-0…FS-4 fázisok; terv: `docs/plans/CFA4-FILE-SERVICES-001-1.0.md` |
| [ADR 0008 – Közös CFA4-auditnapló](../adr/ADR-0008-cfa4-audit.md) | Elfogadva (2026-11-08): egy közös, csak hozzáfűzhető, hash-láncolt auditnapló PostgreSQL-ben (`cfa4_audit` séma, azonos tranzakció az üzleti változással), M1-ben az OS-felhasználó a szereplő; Python és Rust könyvtár közös tesztvektorokkal; terv: `docs/plans/CFA4-AUDIT-001-1.0.md` |
| [ADR 0007 – CFA4 Media Services](../adr/ADR-0007-media-services.md) | Irány elfogadva (2026-11-02): a Media Services a Rust-motor médiaképességeinek kiajánlása a `cfa4-cli serve`-en keresztül (projekt nélküli parancsokkal), nem külön motor; MS-0…MS-4 fázisok a BacklotHub lépcsőihez kötve |
| [PostgreSQL-adatréteg: feladatsor, AI, Media Intelligence](../plans/CFA4-TASKSTORE-PG-001.md) | Elfogadva (2026-11-07): `CFA4-TASKSTORE-PG-001`, a közös feladatsor és ütemezés, az AI-jobok és a Media Intelligence (pgvector) a BacklotHub PostgreSQL-klaszterében, saját sémákkal; kétszeri indítás elleni védelem, PG-0…PG-4 fázisok a CFA4-AI_modules ADR 0006 kapuiban |
| [Helyi AI-modellképzés és finomhangolás](../plans/CFA4-AI-MODEL-TRAINING-001.md) | Tervezet 1.1 (2026-11-07): `CFA4-AI-MODEL-TRAINING-001`, saját tanítású modellek csak a CFA4 alkalmazásban (SFT, LoRA, preferencia, RLVR); a tulajdonosi D1–D6 döntések beépítve, a CFA4-AI_modules ADR 0008 F1, F2, F6 kapujára épül (TR-F0…TR-F10); implementáció nincs engedélyezve |
| [ADR 0006 – BacklotHub: PostgreSQL](../adr/ADR-0006-backlothub-postgresql.md) | Elfogadva (2026-11-02): a BacklotHub adattárolója minden működési módban, az egygépesben is, a PostgreSQL (Ubuntu `postgresql` rendszercsomag, nem beágyazva); a K17 nyilvántartást nem érinti |
| [ADR 0005 – Médiahátterek](../adr/ADR-0005-media-backends.md) | A saját Rust-motor az alap; FFmpeg (≥ 9.0.2) és a GStreamer (alapból az Ubuntué, újabb verzióhoz elkülönített CFA4-futtatókörnyezet) kiegészítő háttér, automatikus útválasztóval (ADR 0004, `cfa4-route`); később további motorok |
| [ADR 0002 – GStreamer](../adr/ADR-0002-gstreamer-backend.md) | GStreamer mint kiegészítő háttérmotor (a saját Rust motor marad az alap), 6 lépéses terv |
| [Smart Cut / Smart Render terv](../plans/CFA4-SMART-CUT-RENDER-R1.md) + [WP-00 kiindulási audit](smart-edit-baseline.md) + [WP-09 validációs jelentés](SMART-CUT-RENDER-WP09-VALIDATION-2026-11-10.md) | `CFA4-SMART-CUT-RENDER-R1`: GOP-tudatos veszteségmentes vágás és határszakasz-újrakódolás; a meglévő és hiányzó képességek, döntés: saját Rust végrehajtás, FFmpeg 9 csak validációs orákulum |
| [CFA4-SCE-001 P0 audit](SCE-001-P0-AUDIT-2026-11-08.md) | A Stream Copy / MPEG-TS / Smart Cut / Smart Render terv P0 fázisa a `main` @ `abcf3d9` alapján: a meglévő TS-muxer, a `smart-edit` állapota (WP-01–03), az atomikus írás hiánya, a bitstream-átalakítók rétege, a módosítandó modulok fázisonként. A Smart Cut állapotára a későbbi WP-09 jelentés az érvényes |
| [Veszteségmentes vágási képességek terve (1.1, végleges)](../plans/CFA4-LOSSLESS-MEDIA-TOOLS-001-1.1.md) + [F0-jelentés](LOSSLESS-MEDIA-TOOLS-001-F0-2026-11-13.md) | CFA4-LOSSLESS-MEDIA-TOOLS-001: a LosslessCut (GPL-2.0-only, csak funkcionális referencia) funkciói közül a CFA4-ből hiányzók átvétele a meglévő Qt6-felületbe és Rust-motorba, külön alkalmazás nélkül; K1–K16 képesség, a szegmensek `.fcproj`-tartományjelölők, a remux és a streamkezelés a SCE-001-gyel; a tulajdonos 2026-11-13-án véglegesítette. F1 csak külön jóváhagyással |
| [Kdenlive-terv (FINAL 1.0)](../plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) | A Kdenlive 26.08 képességei a CFA4-gyel összevetve, a Kdenlive-kód átvételének szabályai, K1–K62 teendők, MI-döntések és a 2026-10-25-i tulajdonosi döntések |
| [AI-fejlesztési infrastruktúra – F0-audit](AI-ORCHESTRATION-F0-2026-10-30.md) | CFA4-DEV-AI-ORCHESTRATION-001 (FINAL 1.0 + 1.1, [terv](../development/ai-orchestration/README.md)): baseline, gap-elemzés TC-01–15 / AC-01–14, AIQT 1.0.6-összevetés, incidensek. F1-től nincs engedélyezve |
| [AI-fejlesztési infrastruktúra – F0-audit v3 (felhő)](AI-ORCHESTRATION-F0-v3-2026-11-06.md) | A v2 kiegészítése a felhős fejlesztési környezetre: helyi célgép-feltevések, R1/R2/R3 rétegek, felhős TC-02 (egy munkamenet = egy konténer = egy ág = legfeljebb egy PR), GAP-C1–C4, D-C1–D-C5 tulajdonosi döntések (2026-11-06: mind elfogadva, FINAL 1.2) |
| [AI-fejlesztési infrastruktúra – F1 felhős felmérés](AI-ORCHESTRATION-F1-felho-2026-11-06.md) | AC-01a a felhős munkamenetben: a működés PASS, a jogosultságok FAIL (merge-jog jóváhagyás nélkül, széles hálózat, felesleges bővítmények, Bash-os „csak olvasó” alügynökök); F1-G1–G5, D-F1-1–3 döntési pontok |
| [Spec Kit – SK-F0 állapotfelmérés](spec-kit/SK-F0-2026-10-31.md) | CFA4-DEV-SPEC-KIT-001 1.1: verzió (v1.2.5), érvényes szabályok, ütközések K1–K4, meglévő integrációk, költségbaseline (BLOCKED). SK-F1 nincs engedélyezve |
| [Spec Kit – SK-F1 izolált próba](spec-kit/SK-F1-2026-11-01.md) | CFA4-DEV-SPEC-KIT-001 1.2: v1.2.5 eldobható worktree-ben; AC-SK-04, -12, -13, -14, -15 PASS (kontextus-többlet 0 token); SkillSpector: csak téves riasztás; új: a beépített workflow kapu nélkül futtatja az implementet. SK-F2 nincs engedélyezve |
| [Spec Kit – SK-F2 hivatkozó constitution és preset](spec-kit/SK-F2-2026-11-01.md) | CFA4-DEV-SPEC-KIT-001 1.2: hivatkozó constitution és `cfa4-policy` preset (forrás: `docs/development/spec-kit/cfa4-policy/`); a #449 után megismételve: AC-SK-02, -03, -04, -13, -14, -15, -16 PASS (preset csak `init --preset`-tel). SK-F3 nincs engedélyezve |
| [Spec Kit – SK-F3 munkafolyamat-integráció](spec-kit/SK-F3-2026-11-06.md) | CFA4-DEV-SPEC-KIT-001 1.2: D9-overlay (`implement` eltávolítva), pontosított `/speckit-specify` wrap; AC-SK-04, -05, -06, -13, -14, -15, -16 PASS; a meglévő `.claude/` fájlok bájtra azonosak. SK-F4 nincs engedélyezve |
| [Spec Kit – SK-F4 biztonsági kapuk](spec-kit/SK-F4-2026-11-08.md) | CFA4-DEV-SPEC-KIT-001 1.2: a három tiltott parancs blokkoló csonkra cserélve (egy mondattal leáll); `check-cfa4-policy.sh` mind a 9 negatív forgatókönyvet kimutatja; AC-SK-07 PASS; bizonyítékmátrix: `docs/development/spec-kit/validation-matrix.md`. SK-F5 nincs engedélyezve |
| [Worktrunk – F0 (felhős környezet)](worktrunk/WT-F0-2026-11-06.md) | CFA4-DEV-WORKTRUNK-001 1.1 ([terv](../development/worktrunk/README.md)): Ubuntu 24.04.5, git 2.43.0, ext4, rustc 1.97.1. A terv előfeltevése (közös klón egy gépen) a felhős munkamenetekre nem áll; a tulajdonos 2026-11-06-án NOT_APPLICABLE-ként lezárta |
| [ADR F10 – stáblista táblázatból](../adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md) + [módosított stáblista-terv](../credits-plan.md) | A Cinecred-szerű stáblista-modul döntése és állapota; az FA3 Credits & Titles terv CFA4-re szabott változata (C1–C9, **javaslat**) |
| [DCP-validációs terv](../plans/DCP-VALIDATION-2026-10-27.md), [DCP-00 képességaudit](DCP-00-CAPABILITY-AUDIT-2026-10-27.md), [DCP-01 forrás- és licencaudit](DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md) | CFA4-DCP-VALIDATION-001: a terv (még nem jóváhagyott), a meglévő CFA4-modulok leltára, crate-javaslat (D1–D7) és a dcp_inspect licenc- és függőségi jegyzéke (L1–L7); tulajdonosi jóváhagyásra vár |
| [CFA4-PY-TYPING-001 T0b – gyanús helyek a Qt6 tesztelőinek](CFA4-PY-TYPING-001-T0b-2026-11-12.md) | A `mypy --strict` 312 összeomlás-gyanús termékkód-találata egyenként besorolva (K1–K7), fájllal, sorral, hibakóddal és kipróbáló GUI-művelettel; megerősített összeomlás nincs, 3 hely célzott kipróbálásra, 70 latens Qt-metódus-elfedés. Csak olvasó jelentés |

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
