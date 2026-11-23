# CFA4-DNEG-VFX-EGYEZTETES-001 – a DNEG-adaptációs terv és a VFX-tervek egyeztetése, 0.1

> **Állapot:** JAVASLAT, tulajdonosi jóváhagyásra vár. **Nem implementációs engedély.** A §3–§4 módosításai a jóváhagyásig **nem hatályosak**;
> a coordination.md §2.10 szerint a módosítással érintett munka (lásd §7) a jóváhagyásig áll.
> **Dátum:** 2026-11-22 · **Kérte:** István (ubuntuokos) · **Csak dokumentáció:** kód nem módosult.
> **Alap:** a `main` @ `2ff5b412` és a #1478 ága (`9cc52693`); a nyitott PR-ok állapota 2026-11-22 07:50 UTC.
> **Újraelemzés 2026-11-22 08:30 után (István utasítására):** a mérvadó VFX-terv a `docs/plans/CFA4-VFX-001.md` **v1.1** (PR #1531, fej `0f37dc91`, 08:27), a hozzá tartozó VFX-0a/VFX-1/VFX-0b (PR #1538, #1539, #1551) és a mátrix (PR #1532). A v1.1 már átvette ennek a jegyzéknek a projekthatár-sorait (§3.1) és a D-1…D-9 döntéseket (§8a); ahol a jegyzék korábbi feltevése ellentmondott neki, javítottam (K-15…K-17, K-1, K-10).
> **3D-döntés (2026-11-22 15:36):** Tulajdonosi döntés (István, 2026-11-22 15:36): a 3D kezelése videó-projekt-függő**, nem globális szint: attól függ, mit importált be a felhasználó az adott projektbe. Átvezetve: K-5, §6, D-12 és a [MÓD-8] jelölés (ARF, OPENUSD, POINTCLOUD, DNEG Q5).
> **Cél:** az elfogadott DNEG-terv ([`CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0`](CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md)) és a véglegesített VFX-terv
> (`CFA4-VFX-001` v1.0, PR #1531, jóváhagyva 2026-11-22; donor-pillanatkép és -leltár: PR #1538, #1539; mátrixsorok: PR #1532) összevetése, hogy az ütközések és
> duplikációk megszűnjenek; a donorok mai állapotának hálózati ellenőrzése.

## 1. Eredmény röviden

1. **17 ütközés/duplikáció** van (§3): a súlyosak K-1 katalógus, K-2 kifejezésmotor, K-3 worker/job-szerződés, K-4 USD-útvonal, K-5 3D-döntés, K-12 OpenVDB-licenc, K-14 átnevezés, K-15 a közös kifejezésmag licence és ütemezése; a többi pontosítás vagy elavult állítás. A VFX-001 v1.1 (08:27) ezt a jegyzéket már beépítette; három korábbi feltevésemet kellett javítani (K-1, K-10, K-16).
2. **Egy tévedés a DNEG-tervben (K-12):** az OpenVDB „Apache-2.0” csak a **12.0.0-tól** igaz (2024-10-31). Az Ubuntu 26.04 csomagja (**10.0.1**, G1 és Launchpad igazolja) **MPL-2.0** upstream. Az N1/N2 (dinamikus linkelés, kódátvétel nélkül) ettől még megengedett, de a terv és az ADR-0003 sora pontosítandó.
3. **Egy nyitott nevezéktani ütközés a gazdának (K-14):** a köteg-PR (#1528) az **EFFECTCHAIN** nevet viszi (a #1524-ből), a #1478 mostani feje (`9cc52693`, 07:20) viszont a **FX-AUTHORING** nevet; a két átnevezés kizárja egymást.
   **Ütközésfeloldási megjegyzés (2026-11-22, a `main` beolvasása után):** a `main`-ben a terv már `CFA4-EFFECTCHAIN-001-0.1.md` néven van (a köteg átnevezése beolvadt). E jegyzék és a `CFA4-VFX-001.md` további „FX-AUTHORING” említései erre a tervre értendők; a D-9 („FX-AUTHORING marad”) formális átvezetése a tulajdonosra vár.
4. **Donorok (§5):** a DNEG-forkok hálózaton újraellenőrizve; a tag-egyezések (fork = upstream) igazolva; **az archiváltság nem ellenőrizhető** (a GitHub API és weboldal a munkamenetből 403), ezt nem tekintem igazoltnak.
5. A terveken a javítások a §4 szerint **javaslatként** megtörténtek; mindegyik `[MÓD-n]` jelöléssel látható, tulajdonosi jóváhagyásig nem hatályos.

## 2. Az összevetett dokumentumok

| Dokumentum | Hol van | Állapot |
|---|---|---|
| DNEG-terv 1.0, audit D1–D9 | #1478 ága (nem a `main`-ben); köteg: #1528 | jóváhagyva 2026-11-21 |
| VFX Catalog, FX Authoring, ARF, OpenUSD/3D, Pontfelhő 0.1 | #1478 ága | jóváhagyva 2026-11-21 22:45 |
| **CFA4-VFX-001 v1.1** (X-1…X-8, §3.1, §8a) | PR #1531, fej `0f37dc91` (nem a `main`-ben) | véglegesítve 2026-11-22 08:27; „ez az irányadó”; a §8a a D-1…D-9-et elfogadottként rögzíti, az X-4 és X-6 nyitott |
| VFX-0a donor-pillanatkép, VFX-1 donor-leltár, VFX-0b donor-licenc/eredetaudit | PR #1538, #1539, #1551 (mind `docs/plans/` alatt) | rögzítve / `PRELIMINARY` / előzetes; csak Linux-hatókör |
| `G-xx` képességmátrix (render-F0V), G-29…G-36 VFX-sorok | `main` (G-01…28), PR #1532 (G-29…36) | – |
| `CFA4-RENDER-INTEGRATION-PLAN-001` v1.1 (C0 szerződések, F3–F5, F9) | `main` | jóváhagyott |
| G1 csomagfelmérés (Ubuntu 26.04) | PR #1499 | kész (dokumentum) |
| N1 / N2 implementáció | PR #1503 / #1549 | nyitott, a tervhez képest lásd K-3, K-13 |

**Következmény:** a DNEG-terv G0 kapuja („a kapcsolódó tervek a `main`-ben”) csak akkor teljesül, ha a #1478 **és** a VFX-001 (#1531) beolvadt; a két terv között a viszonyt ez a dokumentum rögzíti.

## 3. Ütközés- és duplikáció-lista

Súly: **S** = súlyos (két terv ugyanazt építené / egymásnak ellentmond), **K** = közepes, **P** = pontosítás. „Hol rendezve”: melyik dokumentumban van a javaslat; ha másik PR tulajdona, az „átadás” oszlop mondja meg.

| ID | Súly | Ütközés / duplikáció | Érintett tervek | Javasolt rendezés | Hol rendezve / átadás |
|---|---|---|---|---|---|
| **K-1** | S | **Két katalógus.** A VFX-CATALOG-001 saját, generált manifesztet és `docs/vfx-catalog.md`-t épít; a VFX-001 X-3 szerint **egy** katalógus van: a `G-xx` mátrix (az „eredeti D-11 külön katalógus felváltva”). | VFX-CATALOG-001 ↔ VFX-001 §6 | A mátrix az egyetlen katalógus. A VFX-CATALOG-001 szűkül: nincs külön katalógusfájl; marad (a) a regisztrációs szabály (új effekt csak mátrixsorral), (b) egy **konzisztencia-ellenőrzés** `EffectDef` ↔ mátrix `EXISTING` sorok közt (xtask), a VFX-1 részeként; **a duplikátum-szabály a VFX-001-hez igazítva** (K-16). | VFX-CATALOG-001 átírva `[MÓD-1]`; VFX-001 §3.1 kiegészítése: B. melléklet (átadás a #1531 gazdájának) |
| **K-2** | S | **Három kifejezésmotor.** (a) FX-AUTHORING EC-3: skalár, Rust; (b) VFX-001 VFX-4: `valueAtTime`/`wiggle`/`loop`, sandbox, Lumit-`eval` átvétel vagy tiszta szoba; (c) DNEG N3 (Q4 lezárva): saját, korlátozott térbeli interpreter. Három parser, három korlátrendszer. | FX-AUTHORING ↔ VFX-001 ↔ DNEG N3 | **Egy kifejezésmag** (parser, kiértékelő, fuel/idő/memória-korlát, determinisztikus seed), amelynek gazdája a VFX-4. Az FX-AUTHORING EC-3 ezt használja (nincs saját kiértékelő). Az N3 a **térbeli kontextus kiterjesztése ugyanazon a magon** (pont-/voxel-attribútum), nem külön parser; a Q4 döntése („nincs LLVM-JIT”) változatlan. | FX-AUTHORING `[MÓD-2]`; DNEG-terv N3, G4 `[MÓD-5]` |
| **K-3** | S | **Worker- és job-szerződés ×5.** DNEG N1/N2/N4: saját indító + worker, ACS TaskHost regisztráció; render C0: `worker protokoll v0` + job-service; VFX-001 X-5: „közös EXR/USD-job-szerződés”; OPENUSD §3.4 és POINTCLOUD PC-4: „worker → EXR/AOV → sima média” külön-külön. A render G-24 azt írja: a job API a TASKSTORE/ACS-t **újrahasznosítja**. | DNEG N1/N2/N4 ↔ render C0 ↔ VFX-001 X-5 ↔ OPENUSD ↔ POINTCLOUD | **Egy szerződés:** a render-terv C0 worker protokollja és job API-ja (amely a TASKSTORE/ACS felett ül). A DNEG-worker (`cfa4-vdb-worker`), az USD- és pontfelhő-render ennek **kliense**. Az N1 (#1503) saját indítója/protokollja **átmeneti**, C0 elkészültekor át kell állítani (migrációs lépés, nem második tartós protokoll). A kimenet (EXR/AOV vagy PNG sima médiaként) változatlan minta. | DNEG-terv §2/N4 `[MÓD-4]`; OPENUSD, POINTCLOUD `[MÓD-6]`, `[MÓD-7]`; N1-migráció a §7-ben |
| **K-4** | S | **USD három úton.** OPENUSD U2: saját USD-import worker (upstream OpenUSD); VFX-001 VFX-8/X-5: a külső backend (Blender, majd Gaffer) **maga olvassa** az USD-t és EXR AOV-t ad; DNEG D7: a fork nem kell (lezárva). Ráadásul **az Ubuntu 26.04-ben nincs `openusd`/`usd-core` forráscsomag** (Launchpad, 2026-11-22), így az OPENUSD „Ubuntu-csomag vagy külön audit” kapuja valójában „külön audit”, a letöltő `build_usd.py` pedig tilos. | OPENUSD-3D ↔ VFX-001 ↔ DNEG D7 | Két világosan szétválasztott út: **A) külső EXR-csere** (VFX-8: a CFA4-nek nincs USD-értelmezője, a backend olvassa a fájlt) – ez az alap; **B) natív import** (U2) csak akkor, ha a tulajdonos X-4/§8 döntése natív 3D-szerkesztést kér. U1 (glTF/OBJ/STL) marad. G-USD-1: forrásbuild-kérdés külön tulajdonosi döntés. | OPENUSD-3D `[MÓD-6]` |
| **K-5** | S | **Egy döntés, öt helyen.** A VISION §8 (3D/VFX mélysége) külön-külön blokkolja: OPENUSD U0, ARF-2, POINTCLOUD PC-0, VFX-8 (X-4), és a DNEG-terv Q5-je „nem dönti el”. | OPENUSD, ARF, POINTCLOUD, VFX-001, DNEG | **Eldőlt (2026-11-22 15:36): a 3D videó-projekt-függő** (a projektbe importált tartalomtól függ, nem globális szint). A §8 formális átvezetése a `VISION.md`-be a tulajdonos/gazda dolga (ez a PR nem módosítja). Korábbi javaslat: egy tulajdonosi döntés (**X-4 = VISION §8**), a függő munkák listája a §6-ban; a DNEG N1–N2 (adat + renderelt előnézet) **nem** 3D-jelenet, ezért nem várja meg. | ez a dok. §6, D-12; `[MÓD-8]` a tervekben |
| **K-6** | K | **A „G” három névtér.** DNEG-kapuk **G0–G9**; render-mátrix **G-01…G-36**; `gaps.md` **G1…G19**. A VFX-CATALOG „G18/ADR-0003” a `gaps.md` G18-ra (effektmélység) utal, miközben a mátrix G-18 az OpenFX-host; a DNEG-terv „G11” a `gaps.md` G11 (plugin-hosting), a mátrix G-11 viszont az alfa/ProRes 4444. | DNEG, VFX-CATALOG, VFX-001, render | Nincs tömeges átnevezés (a #1499/#1503/#1549 hivatkozásait törné). **Konvenció:** új szövegben DNEG-kapu = `DNEG-G0…G9`, mátrixsor = `G-xx` (kötőjellel, „mátrix”), a `gaps.md` tétel = `gaps.md G18`. A szerkesztett dokumentumokban a félreérthető helyek pontosítva. | `[MÓD-1]`, `[MÓD-3]`, DNEG-terv `[MÓD-5]` |
| **K-7** | K | **Részecske/pont.** VFX-6 a 2D `particles` effektet bővíti (seed+Tick); ARF „Points”, POINTCLOUD és DNEG (OpenVDB Points, PointScatter) térbeli pontokról szól. Veszély: második részecskemotor. | VFX-001 VFX-6 ↔ ARF ↔ POINTCLOUD ↔ DNEG | A részecskemotor egy: VFX-6. A térbeli pontkészletek (ARF Points, VDB Points, pontfelhő) **bemenetek** (forrás, emitter-eloszlás), nem szimuláció. A DNEG-terv G5 (volumetrikus/procedurális bővítés) a VFX-6/VFX-5 katalógussorain át kerülhet be, külön lista nélkül. | ARF `[MÓD-3]`; DNEG-terv G5 `[MÓD-5]` |
| **K-8** | K | **Donor-kezelés.** A VFX-001 donorai a 2026-11-22-i SHA-n befagyasztva (VFX-0a); a DNEG-donoroknak nincs rögzített pillanatképe, a terv „14.0.0”-t említ, de a rendszercsomag 10.0.1. | VFX-001 ↔ DNEG | A DNEG-donorok pillanatképe a VFX-0a mintájára rögzítve (§5): SHA/tag, dátum, licenc. Az N1/N2 a **rendszercsomagra** épül (10.0.1), az upstream 14.0.0 csak referencia. | §5; DNEG-terv §1/§5 `[MÓD-5]` |
| **K-9** | K | **Mátrixsor hiány.** A VFX-001 szabálya: egy képességnek egy elsődleges megvalósítása és egy mátrixsora van. A DNEG-képességeknek (N1–N3), az ARF-nek, az FX-AUTHORING preset/macro rétegnek, az USD/glTF-importnak, a pontfelhőnek **nincs** `G-xx` sora; a G-36/G-26 (külső 3D) csak a VFX-8-at fedi. | DNEG, ARF, FX-AUTH, OPENUSD, POINTCLOUD ↔ mátrix | A. melléklet: javasolt mátrixsorok (ideiglenes `DNEG-M1…M7` azonosítóval). A mátrixot a #1532 beolvadása után a mátrix gazdája vezeti át; ebben a PR-ban a CSV-hez **nem nyúlok** (a #1532 ugyanazt a fájlvéget módosítja). | A. melléklet; átadás: #1532 / mátrix gazda |
| **K-10** | P | **Preset/macro vs. `VfxGraph` és property-hivatkozás.** FX-AUTHORING saját preset-formátumot és „macro” (egy csúszka több paramétert hajt) réteget tervez; a VFX-001 `VfxGraph` L1-adatot (additív sémaverzió) és a VFX-4-ben „property-hivatkozást” épít. Két formátum ugyanarra a láncra, és a macro lényegében property-hivatkozás. | FX-AUTHORING ↔ VFX-001 VFX-2 | Az EC-1/EC-2 (lánc-preset, macro) **nem várja** a VfxGraphot, de az EC-0 séma verziózott és előre kompatibilis; ha a VFX-2 sémája elkészül, a preset a VfxGraph egy részgráfjává képezhető (migráció). Nincs második tartós lánc-modell. A macro az EC-2-ben egyszerű, közvetlen kötés; a VFX-4 property-hivatkozása elkészültekor arra képezendő, külön hivatkozás-mechanizmus nem marad. | FX-AUTHORING `[MÓD-2]` |
| **K-11** | P | **Elavult állítások a DNEG-tervben/auditban:** §1.6 „javaslat, jóváhagyásra vár” (az öt terv jóváhagyva); §9 „draft PR-ok, jóváhagyásig `?`” (a #1490–#1494 lezárva, a tervek a #1478-ban); audit U1 „javaslatok megírva”; D1 `?` sorai. | DNEG-terv, audit | Státuszok frissítve (tényközlés, nem döntés). | DNEG-terv, audit `[MÓD-5]` |
| **K-12** | S | **OpenVDB-licenc verziónként.** A terv „Apache-2.0 (igazolt)” a 14.0.0/HEAD-re igaz. A tag-ek fájljai: **v10.0.1 és v11.0.0 = MPL-2.0** (`LICENSE`, `SPDX: MPL-2.0`), **v12.0.0-tól Apache-2.0** (`LICENSE`, `RE-LICENSE_NOTE.txt`). Az Ubuntu 26.04 `libopenvdb-dev` = 10.0.1-3 → a ténylegesen használt könyvtár upstream MPL-2.0. A G1-dokumentum Debian-copyright-sora („Apache-2.0 fő”) ezzel nem egyezik; a 26.04-es csomag tényleges `copyright` fájlját nem olvastam (U-G1-3). | DNEG-terv §1.7, §5, Q2; audit D8; ADR-0003 táblasor | Az MPL-2.0 a CFA4 szabályai szerint megengedett (AGENTS: MPL-2.0 beolvasztható; dinamikus link, kódátvétel nincs). A terv: „Apache-2.0 a 12.0.0-tól; a használt 10.0.1: MPL-2.0”; az ADR-0003 sorát („BSD-3 / Apache-2.0”) **nem** „Apache-2.0”-ra, hanem „MPL-2.0 (≤ 11.x) / Apache-2.0 (≥ 12.0)” alakra kell javítani (Q2 pontosítása). | DNEG-terv/audit `[MÓD-5]`; ADR-0003 külön kis PR (nem itt) |
| **K-13** | K | **N2-3 átvételi feltétel vs. megvalósítás.** A terv: „`git diff` a `crates/render`, `crates/gpu` fájlokra: **0**”; a #1549 jelentése szerint az új `ItemKind::Volume` három kimerítő `match`-ágat tett a `crates/render`-be (új útvonal nincs). | DNEG-terv N2-3 ↔ #1549 | A feltétel szándéka: **nincs új render-útvonal**. Javasolt pontosítás: a diff a `crates/render`/`crates/gpu` fájlokban csak az új elemtípus kimerítő `match`-ágaira korlátozódhat. A döntés a tulajdonosé; amíg nincs, az N2-3 **nem teljesített**. | DNEG-terv N2-3 `[MÓD-5]` |
| **K-14** | S | **Átnevezés-ütközés a köteg-PR-ban.** #1524 (piszkozat, 01:06): a tiltott márkanév kicserélése `EFFECTCHAIN`-re, fájl `CFA4-EFFECTCHAIN-001-0.1.md`; #1478 feje (`9cc52693`, 07:20): `FX-AUTHORING`, fájl `CFA4-FX-AUTHORING-001-0.1.md`, és a DNEG-terv Q8-a, a #1499 ága és a koordinátori utasítás is ezt a nevet használja. A köteg (#1528, 07:24) az **EFFECTCHAIN** változatot tartalmazza, a `9cc52693`-at **nem**. Ha a köteg így olvad be, a `main`-ben az EFFECTCHAIN-fájl és a hivatkozások maradnak, a #1499/#1503 és ez a PR pedig FX-AUTHORING nevet várnak. | #1478, #1524, #1528, #1499 | Javaslat: **FX-AUTHORING** marad (ez a legújabb döntés, 4 dokumentum használja); a #1524 elavult (lezárandó), a köteget a #1478 mostani fejével kell újrarakni. Ha a gazda mégis az EFFECTCHAIN-t tartja, ennek a PR-nak és a #1499-nek az átnevezése külön, kis lépés. Mindkét nevet a márkaszűrő (`test_independence.py`) elfogadja (ellenőrizve). | átadás a gazdának (#1479); nem ennek a PR-nak a döntése |
| **K-15** | S | **A közös kifejezésmag licence és ütemezése** (a K-2 rendezésének következménye, a VFX-001 v1.1 alapján). A VFX-001 szerint a VFX-4 magja a Lumit `eval` **átvétele vagy tiszta szoba**; a Lumit `GPL-3.0-only` (tulajdonosi döntés: az átvett fájl így marad), a CFA4 `GPL-3.0-or-later`. Ha a mag Lumit-eredetű, a rá épülő DNEG N3-bővítés és az FX-AUTHORING EC-3 kötése az adott fájlokban `GPL-3.0-only` lehet. Ütem: a VFX-4 a VFX-2-re, a render C0/F3-ra és a `cfa4-sandbox`-ra vár, tehát az N3/G4 és az EC-3 **sokkal később** indulhat, mint a DNEG-terv G4-e feltételezte. | DNEG N3/G4, FX-AUTHORING EC-3 ↔ VFX-001 VFX-4 | A mag eredete (átvétel vs. tiszta szoba) a VFX-1 auditban dől el, **N3 előtt**; ha Lumit-eredetű, a licenc-következmény a DNEG-oldalon is rögzítendő (fájlszintű SPDX, regisztersor, a VFX-0b szabálya szerint). Az N3 ütemezése a VFX-4 készültségéhez kötött; az N1/N2 nem érintett. | DNEG-terv N3 `[MÓD-5]`; D-10 |
| **K-16** | K | **Saját korábbi feltevésem javítása: azonos név ≠ duplikátum.** A VFX-CATALOG-001 VC-A2 és a fenti K-1 első változata „nincs azonos nevű, eltérő `id`-jű duplikátum”-ot írt elő. A VFX-001 §6 szerint viszont az azonos nevű eszköz **nem ekvivalens**, és az EC-donor effektjei `ec.*` aliasként, hozzáfűzéssel kerülnek a regiszterbe (VFX-5), a CFA4-azonosító nem íródik felül. A tiltás ezzel ellentmondana. | VFX-CATALOG VC-A2 ↔ VFX-001 §6, VFX-5 | Az ellenőrzés: azonos `id` kétszer **tilos**; azonos név eltérő `id`-vel **csak deklarált** `aliases`/`integration_decision`/`parity_level` mezővel megengedett, egyébként hiba. A név nem kulcs. | VFX-CATALOG `[MÓD-1]` |
| **K-17** | P | **Linux-hatókör és port-vagy-marad (VFX-001 v1.1 §2).** Nincs ellentmondás, de a DNEG-oldal szövegét is ehhez kell kötni: a Gaffer C++/Python kódja csak külső worker lehet a C0 szerződésen át; a donorokból macOS/Windows/FreeBSD/web rész nem jön át (a Potree webes megjelenítője eleve csak referencia). | DNEG, POINTCLOUD, OPENUSD ↔ VFX-001 §2 | Megerősítve, nincs szövegváltoztatás; az átvételi PR-ok port-vagy-marad indoklást rögzítenek. | – |

## 4. Mi változott a tervekben és miért

Minden változtatás **javaslat**, a dokumentumok tetején `Állapot` sorban jelölve; az eredeti, jóváhagyott 0.1/1.0 szöveg a git-előzményben marad.

| Jelölés | Dokumentum | Változtatás | Miért |
|---|---|---|---|
| **[MÓD-1]** | `CFA4-VFX-CATALOG-001-0.1.md` | Szűkítés: nincs külön katalógusfájl és -manifeszt; a mátrix az egyetlen katalógus; marad a regisztrációs szabály és az `EffectDef` ↔ mátrix konzisztencia-ellenőrzés; a G18/G11 hivatkozások kikötve (`gaps.md`). | K-1, K-6 |
| **[MÓD-2]** | `CFA4-FX-AUTHORING-001-0.1.md` | EC-3 (saját kiértékelő) helyett a VFX-4 kifejezésmag használata; „csomópontos kompozitor külön terv” → a CFA4-VFX-001 VFX-2/3; az EC-0 séma előre kompatibilis a `VfxGraph`-fal; a skalár/térbeli kifejezés határa az N3-mal. | K-2, K-10 |
| **[MÓD-3]** | `CFA4-ARF-001-0.1.md` | Új §: viszony a CFA4-VFX-001-hez; ARF-reprezentációk mátrixsorként; a VFX-6 az egyetlen részecskemotor, az ARF Points csak bemenet; a worker-szerződés a render C0. | K-3, K-7, K-9 |
| **[MÓD-4]** | DNEG-terv 1.0 §2 és N4 | A worker-szerződés a render C0 worker protokollja; az N1 saját indítója átmeneti; az N4 regisztráció a TASKSTORE/ACS-en át, a render job API ezt újrahasznosítja (G-24). | K-3 |
| **[MÓD-5]** | DNEG-terv 1.0, audit | N3 és G4: kifejezésmag közös a VFX-4-gyel; G0 és G5 a VFX-001/mátrix szerint; §1.6, §9, U1 státuszok frissítve; OpenVDB-licenc verziónként (K-12) és a használt 10.0.1; donor-pillanatkép (§5); N2-3 pontosítás; USD upstream 26.11.1 és a hiányzó 26.04-es csomag (U6, U10). | K-2, K-5, K-7, K-8, K-11, K-12, K-13 |
| **[MÓD-8]** | ARF, OPENUSD-3D, POINTCLOUD, DNEG-terv Q5 | A 3D videó-projekt-függő (tulajdonosi döntés, 15:36): a „VISION §8 döntéséig áll” előfeltétel szűkül; a projektszintű, tartalom szerinti bekapcsolás a modell. | K-5, D-12 |
| **[MÓD-6]** | `CFA4-OPENUSD-3D-001-0.1.md` | Két út (A külső EXR-csere = alap, B natív import = feltételes, X-4 után); a render-kimenet a C0/X-5 szerződésen; ténytáblába: upstream release v26.11.1 (2026-10-21), HEAD 2026-11-18, nincs 26.04-es USD-csomag; G-USD-1 átfogalmazva. | K-3, K-4, K-5 |
| **[MÓD-7]** | `CFA4-POINTCLOUD-001-0.1.md` | PC-4 a C0/X-5 szerződésen; a pontfelhő-effektek (scatter) a VFX-6 mátrixsorain; a Potree-adatok frissítve (HEAD 2026-11-20, PotreeConverter 2.1.5, licenc újraolvasva). | K-3, K-7 |

**Nem változott:** a DNEG-terv scope-ja (N1–N4), kapui, az AAF-döntés (F4 = 3.), a USD-fork elvetése, az öt terv elfogadási feltételei (kivéve a fent jelölteket), minden kód.

## 5. Donorok ellenőrzése (2026-11-22, hálózaton)

**Módszer:** `git ls-remote` és sekély (`--depth 1`, `--filter=blob:none`) klónok a nyilvános repókból; a licenc a `LICENSE`-fájlok és a forrásfejlécek (`SPDX`) olvasásából; Ubuntu 26.04-es csomagverziók a Launchpad API-ról (`distro_series=resolute`, `Published`). **Letöltés/telepítés nem történt.** A GitHub API és a github.com weboldal a munkamenetből `403` (a repó-hozzáférés csak a CFA4-tárolóra szól), ezért **az archiváltság, a „Release” objektumok és a csillag/aktivitás-adatok nem ellenőrizhetők**; ahol „megállt”-ot írok, az az utolsó commit dátumából következtetés.

| Donor | `HEAD` (SHA) | Utolsó commit | Címke / kiadás | Licenc (a fájlokból) | Tag-egyezés / megjegyzés |
|---|---|---|---|---|---|
| `dneg/openvdb` | `587c9ae8…` | 2020-12-24 | – (nincs újabb, mint a `v8.0.0`) | MPL-2.0 (`LICENSE`) | **HEAD = az ASWF `v8.0.0` tag commitja** (`587c9ae8…`, mindkét repón `ls-remote`) → pillanatkép, nincs saját delta |
| `dneg/openvdb_ax` | `fc0a73d2…` | 2021-01-18 | `v1.0.0` | MPL-2.0 (`LICENSE`) | **HEAD = a saját `v1.0.0` tag**; az audit D8 szerint az ASWF-be beolvadt (a README-t ma nem olvastam újra) |
| `dneg/openvdb_points_dev` | `ec7bcad8…` | 2017-01-20 | `0.1.0` | **nincs licencfájl** a gyökérben (csak `README.md`) | utolsó commit: „Final commit – update README to point at DWA OpenVDB repo”; licenc **nem megállapítható** |
| `dneg/USD` | `b53573ea…` | 2023-04-18 | `v23.05` | „Modified Apache 2.0” (`LICENSE.txt`) | **HEAD = az upstream `v23.05` tag commitja** (`b53573ea…`, mindkét repón `ls-remote`) → delta üres (audit D7 megerősítve) |
| `dneg/aaf` | `dfc1931b…` | 2012-12-14 | régi címkék (`r2-38` stb.) | a gyökérben nincs licencfájl (`README.TXT`); az audit D8 a `LEGAL/AAFSDKPSL.TXT`-t olvasta | a forrás továbbra sem olvasható (clean-room); GPL-3-kompatibilitás **nem igazolt** |
| **ASWF `openvdb`** | `f28614d9…` | 2026-11-20 | **`v14.0.0`** (`ecae2bda…`, 2026-10-29); a HEAD CMake-verziója 14.0.0 | **Apache-2.0** (`LICENSE`, `SPDX` a `Grid.h`, `PointDataGrid.h`, AX `AST.h`, `NanoVDB.h` fejlécekben) | az Apache-2.0 **v12.0.0-tól** (2024-10-31); a `v10.0.1` és `v11.0.0` tag **MPL-2.0** (K-12) |
| **`OpenUSD`** (Pixar) | `80c23ad5…` (`dev`) | 2026-11-18 | **`v26.11.1`** (2026-10-21; a címkeobjektum `d3b468cc…`, a commit `96c19891…`), `v26.11.0`; a `release` ág `ee47c679…` | „Tomorrow Open Source Technology License 1.0” (`LICENSE.txt` fejléke: az Apache-2.0-tól csak a 6. szakasz, védjegyek, tér el) | az audit „HEAD 2026-11-18” és a „release ág” megjegyzése pontosítva: az alapértelmezett ág a `dev`; U10 lezárva (kiadás 26.11.1) |
| `potree/potree` | `b58a590e…` | 2026-11-20 | legutóbbi verziócímke: `1.8.2` | BSD-2-Clause jellegű (© 2011–2020 Markus Schütz; `LICENSE`) | – |
| `potree/PotreeConverter` | `db42c63d…` | 2026-11-02 | `2.1.5` | BSD-2-Clause (© 2020 Markus Schütz; `LICENSE`) | – |
| `GafferHQ/gaffer` | `14c1a3f2…` | 2026-11-20 | `1.7.7.0` (2026-11-12) | BSD-3-Clause (© 2011–2019 John Haddon, Image Engine Design Inc.; `LICENSE`) | a VFX-0a-ban rögzített „Gaffer-donor” SHA **azonos** a nyilvános HEAD-del (következtetés: a rögzített állapotban nincs a nyilvánostól eltérő commit; a CFA4-fork tárolóját nem olvastam) |

**Ubuntu 26.04 (`resolute`), Launchpad, 2026-11-22:** `openvdb` **10.0.1-3** (universe; egyezik a G1-gyel) · `openusd`/`usd-core`: **nincs publikált forráscsomag** (más csomagnevet nem kerestem) · `blender` 5.0.1+dfsg-1ubuntu1 · `opencolorio` 2.5.1+dfsg-1 · `openimageio` 2.5.19.1+dfsg-2 · `openexr` 3.1.13-2build1.

A VFX-001 §9 3. nyitott pontját (elágazási pont SHA az EC- és Gaffer-donornál) ez **nem zárja le**: a Gaffer-egyezés csak a rögzített HEAD-re igaz, a fork története nem látható.

**Pillanatkép a VFX-0a mintájára (DNEG-donorok; a tulajdonos döntheti el, befagyasztjuk-e):** ASWF `openvdb` tag `v14.0.0` = `ecae2bda85744412b0cd47e1b8e1e4e76beb7b88`; OpenUSD tag `v26.11.1` (commit `96c1989167d2aaab5bd34627f32a321a6b79866b`); PotreeConverter `2.1.5` = `c2cb61843bf2b901e8e257874b1a6dcf67792fb2`. Az N1/N2 a rendszer-OpenVDB-re (10.0.1) épül, ezért a 14.0.0 csak referencia.

## 6. Mit blokkol ugyanaz a döntés (K-5)

**Tulajdonosi döntés (István, 2026-11-22 15:36): a 3D kezelése videó-projekt-függő**, nem globális szint: attól függ, mit importált be a felhasználó az adott projektbe.

**Mit jelent (javaslat, a tulajdonos hagyja jóvá a részleteket):** (1) a 3D-képességek **projektszinten, a behozott tartalom alapján** kapcsolnak be (glTF/OBJ/STL/USD-jelenet, pontfelhő, `.vdb`); 3D-tartalom nélküli projekt nem lát és nem fizet 3D-ért; (2) a 3D-állapot a **projektben** tárolódik (additív sémaverzió, régi projekt változatlanul nyílik és renderel, AC-15), nem a felhasználói beállításokban; (3) az USD-út (A külső EXR-csere / B natív import) **tartalmanként/projektenként** választható, alapértelmezés A; (4) a képesség-mélység **tartalomtípusonként** külön fokozódik; hogy melyik típusnál meddig, az első szeletek (U1, PC-2, N1/N2) tapasztalata után dől el. Ami **nem** dőlt el: a mélység felső határa típusonként (egyszerű 3D-cím vs. teljes jelenetszerkesztés) és a VFX-8 külső backend ütemezése.

A korábbi „a VISION §8 döntéséig áll” blokk így **szűkül**: az OPENUSD U0 (formátum- és modell-döntés) a projektfüggő modellre épül és indulhat dokumentum-szinten; a kódot érintő részek (U1/U2, ARF-2, PC-2…, VFX-8) továbbra is külön fázisengedélyt kérnek (coordination.md §2.10). Régi szöveg: a **VISION §8 / X-4** (a 3D/VFX mélysége) megszületéséig áll: OPENUSD U0 és U2, ARF-2, POINTCLOUD PC-0…, VFX-8. **Nem** várja meg: DNEG N1 (VDB-adat), N2 (renderelt előnézet sima médiaként), N4, VFX-0a/0b/1, FX-AUTHORING EC-0…2, ARF-0/1 (dokumentum).

## 7. Jóváhagyandó döntések és az érintett munka

**Állapot (2026-11-22 08:30):** a `CFA4-VFX-001` v1.1 §8a a **D-1…D-9-et elfogadottként rögzíti**, és István utasítása szerint a VFX-001 a mérvadó; a DNEG-oldali módosításokat (`[MÓD-n]`) ennek megfelelően igazítottam. A DNEG-oldali dokumentumokra (DNEG-terv, audit, OPENUSD, ARF, POINTCLOUD, FX-AUTHORING, VFX-CATALOG) a formális tulajdonosi jóváhagyás a #1550-en marad, és a D-10, D-11 új. Az X-4 (VISION §8) továbbra is nyitott.

| # | Döntés | Javaslat | Az érintett munka a jóváhagyásig |
|---|---|---|---|
| D-1 | Egy katalógus = a `G-xx` mátrix; a VFX-CATALOG-001 szűkítése (K-1) | igen | VC-1…VC-4 nem indul |
| D-2 | Egy kifejezésmag (VFX-4 gazdája); FX-AUTHORING EC-3 és DNEG N3 erre épül (K-2) | igen | EC-3, N3, G4 nem indul (N3 amúgy is a G4 mögött áll) |
| D-3 | Egy worker/job-szerződés: render C0; az N1 saját indítója átmeneti (K-3) | igen | **N1-nek (#1503) és N2-nek (#1549) a protokollja nem bővül**; C0 után migráció. A már kész szelet beolvasztható, ha D-3 azt „átmenetinek” fogadja el |
| D-4 | USD: A) külső EXR-csere az alap, B) natív import feltételes; G-USD-1 forrásbuild-döntés (K-4) | igen | OPENUSD U2 nem indul |
| D-5 | N2-3 pontosítása a kimerítő `match`-ágakra (K-13) | igen, ha a tulajdonos a szándékot „nincs új render-útvonal”-ként olvassa | N2 átvétele (#1549) |
| D-6 | A „G” névtér-konvenció (K-6) | igen | – |
| D-7 | OpenVDB-licenc és ADR-0003 sor pontosítása (K-12) | igen (külön kis PR) | ADR-0003 sor |
| D-8 | A DNEG-donorok pillanatképe (§5) | a VFX-0a mintájára igen | – |
| D-9 | **Átnevezés (K-14):** FX-AUTHORING marad; a #1524 lezárandó; a köteg újrarakandó | igen | #1528 |
| D-10 | **A közös kifejezésmag eredete és licence (K-15):** átvétel (Lumit, `GPL-3.0-only`) vagy tiszta szoba; döntés a VFX-1 auditban, N3 előtt | a döntés a VFX-1 audité; a DNEG-oldal a fájlszintű licencet követi | N3, EC-3 |
| D-12 | **3D projektfüggő (tulajdonosi döntés, 2026-11-22 15:36):** a javasolt értelmezés a §6-ban (projektszintű bekapcsolás tartalom alapján, projektben tárolt állapot, A/B út projektenként, típusonkénti mélység) | a döntés a tulajdonosé és már megszületett; az értelmezés megerősítése és a `VISION.md` §8 átvezetése kérendő | OPENUSD U0, ARF-0/1, PC-0 dokumentumszint indulhat; kód nem |
| D-11 | **Duplikátum-szabály (K-16):** azonos `id` tilos, azonos név csak deklarált aliasszal | igen | VC-konzisztencia-ellenőrzés |

## 8. Ami ellenőrizetlen maradt

- A GitHub-oldali **archiváltság** és a „Release” objektumok (API/web 403); a fork-repók „deprecated”/„merged” megjegyzése az audit D8 szövegén és a `dneg/openvdb_points_dev` utolsó commit-üzenetén alapul.
- A **CFA4-donorok** (az „EC-donor (#11)”, a Lumit-donor és a Gaffer-fork saját tárolója) nincsenek a munkamenet hozzáférésében; a VFX-1 leltár számaihoz nem nyúltam.
- A 26.04-es csomagok **tényleges `copyright` fájlja** (U-G1-3), a NanoVDB/AX/Points modul jelenléte a bináris csomagban (U-G1-2); az OpenUSD **más nevű** 26.04-es csomagja.
- A `v12.0.0` előtti OpenVDB-relicencelés **minden** közreműködőre (a `RE-LICENSE_NOTE.txt` névsorát nem vetettem össze a commit-előzménnyel).
- Az N1/N2/N3 és minden más **teljesítmény- és biztonsági mérés** (nincs implementáció, vagy a referenciagépen mérendő).
- A #1531, #1538, #1539, #1532 PR-ok tartalmát olvastam, de a bennük hivatkozott számokat (307/306/280/3032/2984) nem mértem újra.

---

## A. melléklet – javasolt mátrixsorok (a #1532 beolvadása után, a mátrix gazdájának)

Formátum: `id,capability,plan_phase,existing_cfa4,evidence,status,measure_or_target,ac,candidate_if_gap`; a VFX-001 §6 további mezői (`family`, `owner_project` = a megnevezett DNEG-/ARF-/… projekt, `license_status`, `verification`, `aliases`) a VFX-1-ben töltendők. Ideiglenes `DNEG-M*` azonosítók; a végleges `G-xx` számot a mátrix gazdája adja.

```
DNEG-M1,[T10] Volume data layer (.vdb inspect/import/relink as media; no new timeline),DNEG-N1,partial: PR #1503 open (crates/vdb + workers/vdb),PR #1503; docs/plans/CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md N1,GAP (implementation in review),N1-1..N1-10,none,system OpenVDB 10.0.1 (Ubuntu 26.04; MPL-2.0 upstream at that tag) via a supervised worker
DNEG-M2,[T10] Volume density preview rendered as plain media (vdb.render),DNEG-N2,partial: PR #1549 open (PNG; no EXR/AOV yet),PR #1549; N2 in the DNEG plan,GAP (first slice),N2-1..N2-5 (N2-3 wording pending decision D-5),AC-04,same worker; output through the C0 job contract (EXR AOV when G-10 multi-part exists)
DNEG-M3,[T5] Spatial (point/voxel) expressions as a context of the shared expression core,DNEG-N3 (gate DNEG-G4),none,no spatial evaluator in crates/*,GAP_STATIC (blocked by G-29 and the sandbox PoC),N3-S1..S3; N3-1..N3-7,AC-03,own restricted interpreter on the VFX-4 core (no LLVM-JIT; owner decision Q4)
DNEG-M4,[T9/T10] Adaptive Representation Fabric (descriptor schema + LOD rule; no renderer),ARF-1..3,none,no ARF in crates/*,GAP_STATIC (blocked by VISION section 8 for code),ARF-A1..A9,AC-03,own L1 crate cfa4-arf (pure Rust)
DNEG-M5,[T9] Scene exchange: glTF/OBJ/STL import (U1); USD import (U2) only after the owner's 3D depth decision,OPENUSD U1/U2,none,no scene types in crates/*,BLOCKED_BY_OWNER_DECISION (VISION section 8; X-4),U1-A1..A5; U2-A1..A5,AC-04,glTF/OBJ/STL Rust readers (licences unchecked); USD only as an external backend (see G-36) unless D-4 says native import
DNEG-M6,[T9] Point-cloud read / LOD / preview (LAS/LAZ/PLY first),POINTCLOUD PC-1..4,none,no point-cloud code in crates/*,BLOCKED_BY_OWNER_DECISION (VISION section 8; ARF approval),PC-A1..A12,AC-04,own readers on the ARF descriptor; PotreeConverter only as a user-started external tool
DNEG-M7,[T5] Effect-chain presets and macro parameters on the existing effect chain,FX-AUTHORING EC-0..2,partial: lumetri presets and graphic templates exist (crates/engine),crates/engine/src/lumetri_presets.rs; graphic_templates.rs,TARGET (extend; no second chain model),EC-A1..A4,AC-15,own versioned preset format; migration path to a VfxGraph sub-graph (G-30)
```

## B. melléklet – a CFA4-VFX-001 §3.1 projekthatár-táblázatának kiegészítése (a VFX-001 v1.1 ezt **már átvette**; itt csak a forrás-szöveg, a #1531 az irányadó)

| Terület | Gazdaprojekt | A VFX-projekt feladata |
|---|---|---|
| OpenVDB-adat, volume-előnézet, térbeli kifejezés (kontextus a közös kifejezésmagon) | `CFA4-DNEG-TECHNOLOGY-ADAPTATION-001` (N1–N3) | A kifejezésmagot (VFX-4) adja; a DNEG-munka a mátrixban `DNEG-M1…M3` |
| Reprezentáció-réteg (mesh, pont, volume, görbe, példány, impostor) | `CFA4-ARF-001` | Csak bemenet a VFX-6-nak; nincs második részecskemotor |
| Effektlánc-preset, macro | `CFA4-FX-AUTHORING-001` | A preset a `VfxGraph` részgráfjává migrálható; a kifejezés a VFX-4 magja |
| Jelenetcsere (glTF/OBJ/STL, natív USD-import) | `CFA4-OPENUSD-3D-001` | VFX-8 külső EXR-csere az alap; a natív import X-4 után |
| Pontfelhő | `CFA4-POINTCLOUD-001` | Effektek (scatter) csak a VFX-6 sorain |
