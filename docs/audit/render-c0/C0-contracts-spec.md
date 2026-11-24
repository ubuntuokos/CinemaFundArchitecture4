# C0 – szerződés- és job API specifikáció v0.1 (VÉGLEGES)

**Dátum:** 2026-11-24 · **Állapot:** VÉGLEGES, JÓVÁHAGYVA (István, 2026-11-24: a 11. pont D1–D6 javaslatait elfogadta) ·
**Alap:** `CFA4-RENDER-INTEGRATION-PLAN-001` FINAL 1.1 (jóváhagyva), C0 fejezet; a repóban lévő vázlat: `docs/audit/render-c0/C0-contracts-spec.md` és `C0-existing-work-inventory.md`; a már beolvadt C0-kód: `cfa4-tasks` `render.rs`, `cfa4-frame` `side.rs` és `convert.rs`, a cancel-mérő az `cfa4-export`-ban.
**Ez a dokumentum:** a vázlat véglegesített változata, a mai `main` állapotához igazítva, kiegészítve a matte-síkkal (a zöldháttér/AI-mattolás terv Z1b pontja). Ez a szöveg váltja a repóbeli `C0-contracts-spec.md` vázlatot.
**Licencszabály (István, 2026-11-24):** ahol nincs licenc, saját kód; a C0 semmilyen külső kódot nem vesz át.

## 1. Alapelvek

1. **Nincs párhuzamos típusrendszer és nincs második sor.** A job API a meglévő `cfa4-tasks` `Queue`-ján áll (István döntése, 2026-11-23); a Frame ABI a meglévő `cfa4-frame` `VideoFrame`-jére épül.
2. **Csak bővítés.** Új mező a `VideoFrame` mellett (`FrameSideData`), nem benne, mert a `VideoFrame` struct-literállal épül 54 fájlban.
3. **Nincs csendes minőségromlás.** Nem támogatott pár, hiányzó képesség: nevesített hiba (exportnál FAIL_CLOSED, előnézetnél jelzett fallback; A4).
4. **A CPU a referencia** (A5, MQ-6): a szerződés GPU nélkül is teljes.
5. **A szerződés értéktípusokból áll** (serde, a rendererre nem függ), hogy a vezérlőcsatorna, a CLI és a Qt6 ugyanazt olvassa.

## 2. Állapot: mi van meg, mi hiányzik

| Terület | Megvan a `main`-en | Hiányzik a v0-hoz |
|---|---|---|
| Állapotgép | `cfa4_tasks::queue::State` + `transition`; `RenderState` nézet (Queued, Running, Paused, Cancelling, Completed, Failed, Cancelled) | – |
| Render-esemény | `RenderEvent`, `RenderEventKind`, `RenderProgress`, `RenderFallback` (`cfa4-tasks/src/render.rs`) | az adapter, amely az `Exporter` haladását ebbe tölti (a kliens rétege) |
| Cancel | `Queue::cancel`, `Progress.cancel`; mérés: 20/20 megszakítás, medián ≈ 5–7 ms (`render-c0/C0-cancel-latency.md`) | Küszöb@F0 PENDING (referenciagép) |
| Atomikus kimenet | `AtomicOutput` (`atomic.rs`), SIGKILL-mérés a #1738-ban; a fő exportút (`Out::Atomic`) már használja, a megszakítás utáni fájlmentességet teszt kényszeríti ki (`cancel_latency_tests.rs`) | **a csomag-kimenetek (DCP, IMF, MXF, OP-Atom: `Out::create_path`) még helyben írnak**; ezek átállítása nyitott |
| Frame ABI | `VideoFrame` + `FrameSideData` (forrás-azonosító, forgatás, HDR, `Residency`); matte-sík (`MatteFrame`, `MatteRef`) a #1823-ban | timecode a `FrameSideData`-ban (a matte már hordozza) |
| Konverziós mátrix | `cfa4_frame::convert::plan/convert`, 25 pár tesztelve, `Unsupported` explicit | GPU-sor (PENDING, `crates/gpu` kötés) |
| Audio ABI | `AudioBuffer` (planáris f32, `sample_rate`); `ChannelLayout` és `AudioFormat` a #1823-ban | 24 bit és 48/96 kHz mérése az F0V-harnessen (MQ-7) |
| Időmodell | `Tick` (254 016 000 000/s), racionális `FrameRate` | timeline→média PTS és a VFR-szabály igazolása valódi VFR klippel |
| Hiba / capability | `cfa4_route::{Error, Declined, Reason, Family}`; `RenderCapability` és `decide` a #1823-ban | A4 szabálytábla tartalma (G-18, ADR 0003 A4) |
| Worker protokoll | `crates/sandbox` profilok; üzenettípusok és handshake a #1823-ban | profilok: `renderWorker`, `externalRender` (SECURITY-001 felelőse); a képkocka-csere módja (D5) |
| Capture I/O | – | csak interfész, implementáció nélkül |

## 3. Job API v0 (a `cfa4-tasks`-on)

- **Osztály:** a render `Class::FinalRender` (export) vagy az előnézeti osztály; a prioritás „előnézet előzi az exportot” a meglévő `class_rank` szerint (leképezés ellenőrizendő tesztet kap).
- **Parancsok a vezérlőcsatornán** (`cfa4-cli serve`, a meglévő parancskészlet bővítése, minden parancsnak id és címke): `render.status {job}`, `render.list`, `render.cancel {job}` (az adapter megvalósítása, külön PR). A beküldés (submit) a meglévő export-parancsok (pl. `file.exportMedia`), amelyek `job` azonosítót adnak; az események (events) a `render.status` `event` mezőjében jönnek. A hívó szálon semmi nem blokkol; a Qt6 és a CLI ugyanezt hívja (A8).
- **Esemény:** a `RenderEvent` JSON-alakja a szerződés része (`camelCase`), `kind` = `progress` / `log` / `warning` / `fallback`; `is_consistent()` a szerződésteszt tárgya.
- **Cancel:** kooperatív; a kérés és a leállás közti idő mért érték (AC-09), nem előre rögzített küszöb, amíg a referenciagépes Küszöb@F0 nincs meg.
- **Atomikus kimenet:** minden render a végleges néven csak sikeres lezárás után jelenik meg (ideiglenes név + rename, `AtomicOutput`). **Kilépési feltétel:** 20 véletlen idejű cancel és SIGKILL után végleges néven 0 csonka fájl. A fő exportútra ez teljesül; a csomag-kimenetekre (DCP, IMF, MXF, OP-Atom) az átállítás nyitott, külön PR-ban, a DCP/IMF-export gazdáival egyeztetve.
- **Retry:** csak idempotens lépésre; a `checkpoint` és a lease a meglévő sor szerint.
- **Backendválasztás:** `cfa4_route::Router`, alapértelmezés a meglévő CPU-renderer; egyetlen feature flag. Render-család bejegyzése: **döntés kell** (D4).

## 4. Matte-sík a Frame ABI-ban (a Z1b javaslat normatív szövege)

A kulcsolók és az AI-matte ugyanazt állítják elő: folytonos, egycsatornás alfát. A szerződés ezt értékként rögzíti; a bináris `MaskRef` (Object Mask) változatlan marad.

| Mező | Tartalom |
|---|---|
| Elemtípus | egycsatornás sík: `u8`, `u16` vagy `f32` (a v0-ban mindhárom: D3) |
| Értéktartomány | [0, 1], **adat, nem szín**: nincs transfer-függvény és színtér-címke |
| Alfa-mód | mindig **straight**; a premultiplikálás a kompozitor lépése |
| Méret | szélesség × magasság, sűrű csomagolás; azonos a forrásképpel vagy explicit skálázási szabály |
| Időbélyeg | `Tick`, a forrás képkockájához kötve; nincs csendes CFR-kerekítés |
| Forrás-azonosító, timecode | mint a `FrameSideData`-ban |
| Eredet | módszer (`key.ultra`, `key.color`, `key.luma`, `ai.matting.*`), paraméter-hash; AI esetén modell-azonosító és SHA-256 |
| Hivatkozás | `MatteRef { sha256, width, height, depth }`, tartalomcímzett lemezes gyorsítótár; **nem** új adatbázis |

Tisztázandó a szerződésben: az Ultra Key „Alpha Channel” kimenete ma sRGB→lineáris megjelenítés (szürke kép), ez megjelenítési segéd, a matte-sík kimenet ettől külön, transfer nélküli adat; a kulcsoló ma `a · src_a`-val premultiplikál, a szerződés szerint a matte straight. Konverzió: maszk → matte (0/1) veszteségmentes; matte → maszk explicit küszöbbel.

## 5. Audio ABI és időmodell

`AudioBuffer` kiegészítése csatornaelrendezés-címkével (mono, sztereó, 5.1, 7.1 és „ismeretlen”; ismeretlen esetén nincs találgatás) és forrás-bitmélységgel; 24 bit és 48/96 kHz (MQ-7) mérése az F0V-harnessen. Időmodell: `Tick`; a VFR-szabály („csendes CFR-kerekítés nélkül”) igazolása valódi VFR klippel az F0V nyitott tétele, a v0 ezt kimondja, de nem zárja le.

## 6. Konverziós mátrix, hiba- és capability modell

A mátrix változatlanul a `cfa4_frame::convert` (azonos formátum: Share; RgbaF32 felé: veszteségmentes; Rgba8 felé: veszteséges; többi pár: `Unsupported { from, to }`). A render-szintű capability-leírás vékony típus: formátum × bitmélység × helyzet (CPU/GPU) × környezet → `Supported` / `FailClosed` / `FallbackWarned`. Az A4 szabálytábla az ADR 0003 A4 (OpenFX) és a `route` táblájához igazodik.

## 7. Worker protokoll v0 és capture I/O

Külön folyamat, verzió-handshake, képkocka-csere, timeout, erőforrás-limit, cancel, útvonal-korlát. A képkocka-csere módja (memfd/shm vagy fájl) az F2 POC mérésén múlik: a v0 az interfészt rögzíti, nem a módot (D5). A `renderWorker` és `externalRender` sandbox-profilt a CFA4-SECURITY-001 felelőse veszi fel. Capture I/O: kicsi trait, implementáció nélkül.

## 8. A `cfa4-render-contracts` crate váza (PR #1823, a jóváhagyás után elkészült)

- **Szerep:** vékony crate, amely a szerződés típusait egy helyről nevezi meg (`pub use` a `cfa4-frame`, `cfa4-tasks`, `cfa4-route` meglévő típusaira) és csak a valóban újat hordozza: `ContractVersion`, `MatteFrame` / `MatteRef`, `RenderCapability`, a worker-protokoll üzenettípusai, a capture trait.
- **Réteg:** a crate `cfa4-tasks`-ra (L2) is támaszkodik, ezért javaslat: **L3**; a `cargo xtask layers` a megvalósítás első lépése (a rétegtáblázat bővítése a `docs/architecture.md` §1-ben).
- **Szabályok:** `unsafe` tilos, a `unwrap/expect/panic` lint szabályai érvényesek, a típusok `serde`-sek, a dokumentumok magyar, a kód angol.
- **Kód a vázban:** csak típusok és szerződésteszt; viselkedés (renderelés, sor) nem. A render-job-service külön crate nem készül (a `cfa4-tasks` a gazdája).

## 9. Verziózás és szerződésteszt

Szemantikus verzió (`ContractVersion { major, minor }`). Szerződésteszt: (1) a `VideoFrame` mezőlistája kimerítő destrukturálással, (2) `State` ↔ `RenderState` tábla, (3) a konverziós mátrix mind a 25 párja, (4) a `RenderEvent` JSON körút, (5) a matte-sík hostile-input teszt (NaN, ±inf, 0 méret, túlméret, nem egyező méret, hibás hash: hiba, nem pánik), (6) régi projekt matte nélkül változatlanul nyílik, a golden változatlan. Befagyasztás után törő változás csak ADR-rel és verzióemeléssel.

## 10. Kilépési feltételek (a terv C0-ja szerint) és jelenlegi állásuk

| Feltétel | Állás |
|---|---|
| A meglévő renderer a szerződésen át bit-azonos a golden halmazzal, PTS-diff 0 | RÉSZBEN: az adapter (`render_job.rs`) a meglévő jobokon nézetet ad, a renderelést nem érinti; a golden/PTS-diff összevetés még nincs mérve |
| Cancel latencia ≤ Küszöb@F0; 20 cancel/SIGKILL után 0 csonka fájl | RÉSZBEN: a latencia mért; a fő exportúton a csonka végleges fájl 0, a csomag-kimeneteken (DCP, IMF, MXF, OP-Atom) a fájlonkénti atomi írás megvan (PR #1825), a SIGKILL-ciklusos mérés még nyitott |
| Minden S1 jelölt leképezhető, BLOCKED, vagy ADR-rel módosul | NYITOTT: F2 előtt |
| A Qt6 felelőse írásban elfogadja a job API-t | ELFOGADVA: István mint a Qt6 felelőse elfogadta (2026-11-24, „elfogadom”; ez a job API-t is fedi) |

## 11. Döntések (István, 2026-11-24: mind a hat javaslat elfogadva)

| # | Kérdés | Döntés |
|---|---|---|
| D1 | A fenti szöveg legyen a C0 v0 specifikáció, és erre épüljön a crate váza? | **Elfogadva:** igen; a váz külön PR-ban (#1823) |
| D2 | A matte-sík (4. pont) része-e a v0 Frame ABI-nak? | **Elfogadva:** igen; a zöldháttér- és AI-matte-terv ezen áll, a bináris `MaskRef` nem változik |
| D3 | A matte elemtípusa: egy (`f32`) vagy három (`u8`/`u16`/`f32`)? | **Elfogadva:** három: 8 bit előnézet/gyorsítótár, 16 bit alfa-export, f32 belső munka |
| D4 | Kerüljön-e `Family::Render` a `cfa4-route` táblájába, vagy a meglévő `Display`/`Compute` maradjon? | **Elfogadva:** maradjon a meglévő a v0-ban; új család csak mért igényre, a `route` gazdájának egyetértésével |
| D5 | A képkocka-csere módja a worker protokollban | **Elfogadva:** később, az F2 POC mérése alapján; a v0 csak az interfészt rögzíti |
| D6 | Az exportút `AtomicOutput`-ra állítása a C0 kilépési feltétele (a fő útvonal már kész, a csomag-kimenetek hátra vannak), külön PR-ban az érintett gazdákkal | **Elfogadva:** igen; külön PR, az érintett gazdákkal, a C0 kilépési feltétele |

## 12. Mit nem ellenőriztem

A #1823 crate lefordul, 20 tesztje zöld, a `cargo xtask layers` ok (L3). A jelen állapotot a `main` forrásából és a repóban lévő C0-dokumentumokból olvastam; ebben a lépésben nem futtattam új mérést. A `cfa4-export`, `cfa4-route` és a SECURITY-001 gazdáinak egyetértése a D4–D6 pontokra nincs meg. A CFA4-AI_modules tárolót (ADR 0008 szerződései) nem olvastam.
