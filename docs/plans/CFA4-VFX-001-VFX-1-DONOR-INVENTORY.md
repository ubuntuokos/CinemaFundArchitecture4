# VFX-1 – donor-leltár (előzetes, `PRELIMINARY`)

**Dátum:** 2026-11-22 · **Terv:** `CFA4-VFX-001` §5 (VFX-1) · **Alap:** `docs/plans/CFA4-VFX-001-VFX-0A-DONOR-SNAPSHOT.md` (rögzített SHA-k)

Ez **csak leltár**: olvasással és statikus számolással készült, a donorokat nem fordítottuk le és nem futtattuk. Az effekt-szintű besorolás (`PASS`/`BLOCKED`/`REFERENCE_ONLY`, a render-terv F1/F2 kapuja) még **nincs** kész; azt a render-terv F1/F2 keretében, az `F0-capability-gap-matrix.csv` sorain kell elvégezni. A márkaszűrő miatt a donor-repók neve nincs kiírva.

## 0. Hatókör: csak Linux (tulajdonosi utasítás, 2026-11-22)
A leltárból és az átvételből ki van zárva minden macOS-, Windows-, FreeBSD- és web-specifikus rész; ezek nem kerülnek átvételre, és nem futnak miattuk fordítások vagy tesztek (terv 2. fejezet). Statikus számolás a rögzített SHA-kon (platformfeltételes Rust-sorok és platformkönyvtárak):

| Donor | `cfg(windows)` / `cfg(target_os = "macos")` sor | Platform-könyvtárak |
|---|---|---|
| EC-donor | 42 | `packaging/windows`, `packaging/macos`, `packaging/freebsd`, `packaging/web` |
| Lumit-donor | 101 | `packaging/windows`, `packaging/macos`; a Flutter UI `windows/`, `macos/` mappái (a Flutter UI amúgy sem használható) |
| Gaffer-donor | 0 | – |

A CFA4 saját CI-jében nincs macOS- vagy Windows-job (az összes `runs-on` Ubuntu), és a nyitott VFX-PR-ok csak dokumentumot érintenek, ezért ezek nem indítanak ilyen jobot.

## 1. EC-donor (#11), SHA `171e92b0…`

| Mérés | Érték | Jelleg |
|---|---|---|
| Effekt / paraméter / GPU-jelölés a donor saját, generált dokumentumában | 308 / 2992 / 280 | a donor saját állítása, **nem** mérés |
| Különböző `ec.<kategória>.<név>` azonosító a forrásban (statikus `grep`) | 315 | közelítés (aliasokat és teszteket is tartalmazhat), **nem** egyezik a 308-cal |
| Rust-fájlok crate-enként (legnagyobbak) | `engine` 249, `ui-egui` 273, `aep` 111, `effects` 108, `gpu` 70, `render` 62, `sam2` 26, `track` 25, `script` 24, `media` 37, `expr` 11, `lottie` 8 | statikus számolás |
| Kiadási fordító / kiadás | Rust edition 2024, `rust-version` 1.95 | `Cargo.toml` |
| Licenc | `MIT OR Apache-2.0` | `Cargo.toml`, `NOTICE` |

**Következtetések (előzetes):**
- A bemeneti tervek 307/306/280/3032/2984 számai a donor saját 308/2992/280 számától is eltérnek; a számokat **csak a rögzített SHA-n futtatott regiszterexport** hitelesítheti (terv 6. fejezet). Összeadni a forrásokat nem szabad.
- A donor több külső, git-revízióra rögzített segédkönyvtárra támaszkodik (az upstream azonos szervezetének 5 külön git-forrása a `Cargo.lock`-ban). Átvétel előtt ezek licencét és eredetét külön auditálni kell (VFX-0b), különben az átvett kód elvihet egy nem auditált függőséget.
- A donor effektjei az `effects` crate-ben vannak; a CFA4-ben a meglévő effektregiszterbe, hozzáfűzéssel (`ec.*` alias), a render-terv F1/F2 szerint kerülnének.
- A `aep` (AE-projekt import/export), `expr`, `track`, `lottie`, `sam2` modulok átfednek más CFA4-projektekkel (3.1 projekthatár-táblázat); átvételük a gazdaprojekt jóváhagyásától függ.

## 2. Lumit-donor, SHA `5567ef97…` (`GPL-3.0-only`)

Sorok száma crate-enként (`wc -l`, csak méretrend, nem minőség):

| Crate | ~sor | A VFX-tervben érintett terület | Előzetes hipotézis (az audit dönt) |
|---|---|---|---|
| `lumit-eval` | 7 900 | kifejezésmotor (T5, VFX-4) | átvétel vagy tiszta szobás; sandbox-illesztés kell |
| `lumit-script` | 8 400 | scripting (T16) | vizsgálandó |
| `lumit-flow` | 4 100 | csomópontgráf (VFX-2/3) | az adatmodell-döntés (X-1) szerint illesztendő |
| `lumit-roto`, `lumit-track` | 2 400 / 11 500 | roto, követés (OBJECT-MASK gazda) | csak hiánylista a gazdának (VFX-7) |
| `lumit-lottie` | 14 000 | Lottie (TITLE-CREDITS/KDN-43 gazda) | csak hiánylista (X-7) |
| `lumit-ofx` (+ `-broker`) | 18 300 / 1 300 | OpenFX-host (ADR 0003 A4, G-18 gazda) | a render-terv F6 hatáskörében |
| `lumit-scene` | 4 400 | 3D-jelenet (T9) | **felfüggesztve** az X-4 döntésig |
| `lumit-import` | 23 000 | AE-projekt import (T12) | vizsgálandó (OTIO-terv és KDN-43 átfedés) |
| `lumit-colour` | 9 600 | színkezelés (ADR 0003 A2 gazda) | csak referencia |
| `lumit-gpu`, `lumit-render` | 63 400 / 101 900 | GPU-effektek (176 `.wgsl` shader) | effekt-szintű átvétel a F1/F2 szerint; a CFA4 GPU-ja (`G-07`) a gazda |
| `lumit-ml` | 10 300 | helyi AI | a külön AI-terv hatásköre, nem tervezzük |
| `lumit-bridge` | 191 000 | Flutter-híd | **nem használható** (a CFA4-ben csak Qt6) |
| `lumit-core`, `lumit-project` | 212 700 / 7 100 | alapmodell | referencia; a CFA4-nek saját L1-modellje van |

## 3. Gaffer-donor, SHA `14c1a3f2…` (BSD-3-Clause)

C++/Python, SCons. Fájlszám modulonként (`.cpp`, `.h`, `.py`): `GafferScene` 186, `Gaffer` 85, `GafferImage` 83, `GafferUI` 54, `GafferSceneUI` 37, `GafferCycles` 28. Külső 3D-backend jelölt (VFX-8, X-4/X-5): csak külső folyamatként, EXR/USD-szerződéssel; a döntésig nem érintjük.

## 4. Nyitott a teljes VFX-1-hez
1. Regiszterexport az EC-donor rögzített SHA-ján (fordítás vagy az exportáló teszt futtatása), ebből a hiteles bejegyzés- és paraméterszám.
2. A segédkönyvtárak licenc- és eredetauditja (VFX-0b).
3. Effekt-szintű besorolás a render-terv F1/F2 szerint, soronként forrással és a `G-xx` sorokkal való duplikáció-ellenőrzéssel.
4. A Lumit-modulok mélyebb vizsgálata (API-k, tesztek, függőségek), különösen `lumit-eval`, `lumit-flow`, `lumit-import`.
5. Az AE-referencialeltár (stabil és béta) – a tulajdonos által megadott tervekből; ehhez nincs hozzáférésem Adobe-forráshoz, és nem is használható (`AGENTS.md` §1: tiszta szoba).
