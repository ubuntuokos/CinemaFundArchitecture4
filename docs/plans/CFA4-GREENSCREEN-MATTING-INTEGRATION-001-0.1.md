# CFA4-GREENSCREEN-MATTING-INTEGRATION-001: a zöldháttér (chroma key) elemzése és beépíthetőségi terve

| Dokumentumadat | Érték |
|---|---|
| Állapot | **JAVASLAT, tulajdonosi jóváhagyásra vár.** Fejlesztés nem indul belőle jóváhagyás nélkül ([`coordination.md`](../coordination.md) §2.10) |
| Dátum | 2026-11-23 |
| Kérte | István (ubuntuokos) |
| Alap | CFA4 `main` @ `bc3308f` |
| Módszer | **Csak olvasás:** forráskód, tesztek és tervek átnézése. Nem futtattam buildet, tesztet vagy mérést, ezért ez a dokumentum **nem állít validációt**; a „kész” szó csak ott áll, ahol a repó saját tesztje vagy doksija ezt rögzíti, hivatkozással |
| Összevetett tervek | `CFA4-AI-MATTING-ENGINE-001` v1.0 (APPROVAL BASELINE; a szövege **nincs a repóban**, lásd 9. pont), [`OBJECT-MASK-PLAN-001`](OBJECT-MASK-PLAN-001-2026-11-03.md), [`CFA4-VFX-001`](CFA4-VFX-001.md), [`CFA4-ADAPTIVE-CAPABILITY-001-OSSZEVETES`](CFA4-ADAPTIVE-CAPABILITY-001-OSSZEVETES.md), [`CFA4-AI-MODEL-TRAINING-001`](CFA4-AI-MODEL-TRAINING-001.md), [`CFA4-KDENLIVE-PLAN-001-FINAL-1.0`](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) K4, [`CFA4-DATA-PG-001-1.0`](CFA4-DATA-PG-001-1.0.md), [`CFA4-TASKSTORE-PG-001`](CFA4-TASKSTORE-PG-001.md), [`docs/gaps.md`](../gaps.md) G12/G18, [`docs/open-editors-parity.md`](../open-editors-parity.md) E7, [`docs/hardware-parity.md`](../hardware-parity.md), [`VISION.md`](../../VISION.md), [`AGENTS.md`](../../AGENTS.md) |
| Nyelv | magyar |

## 0. Röviden

1. **A klasszikus zöldháttér-kulcsolás megvan, de csak CPU-n, és vékonyan bizonyítva.** A kulcsoló az **Ultra Key** (`crates/render/src/vfx/color.rs`, `ultra_key`) és az egyszerűbb **Color Key**. Van hozzájuk egységteszt, arany kép és Qt6-os parancsteszt (2. pont). Mérést valódi zöldháttéres felvételen, GPU-s utat és önálló despill-effektet a repóban nem találtam.
2. **Az AI-mattolás (BMV2, RVM) a repóban még nincs.** Sem kód, sem a terv szövege. Az RVM csak jelöltként szerepel (`docs/gaps.md` G12, `docs/open-editors-parity.md` E7, K4). Az építőelemek közül hiányzik az **AI Model Manager** és az **AI Video Tools** is: az előbbit csak hivatkozások nevezik (`CFA4-MODEL-MANAGER-001`, „will own the store”), az utóbbi csak kategóriaként szerepel az `AI-MODEL-TRAINING-001` §10-ben. Az ADR 0008 és az ACS az **AI-modulok külön tárolójában** él, amelyet ez a munkamenet nem érhetett el.
3. **A határ:** a kulcsoló **effekt** marad (VFX ▸ Keying), az AI-mattolás **matte-forrás**. A kettő közös pontja: **a matte mint érték**, amelyet a Compositor már ma is fogyaszt (Track Matte Key, klip-alfa), valamint a háttércsere-munkafolyamat, a szegély-/spill-tisztítás és az alfa-export. Ezekben egyszer kell megépíteni, nem kétszer.
4. **A legfontosabb új lelet:** az Object Mask terv maszkmodellje (`MaskRle`, `MaskRef`) **bináris** (0/1 futáshossz). A mattolás **folytonos alfát** ad, finom hajszálakkal. Ezért az Object Mask tárolója **nem használható** matte-nek; külön, tartalomcímzett alfa-matte eszköz kell (5. pont).
5. **Javaslat:** négy lépés (Z0–Z3), amely az AI-mattolás F0–F8 fázisaival egymásba illeszthető (7. pont). A Z0–Z2 AI nélkül is elvégezhető, és az AI-mattolás nyereségét is előkészíti. Nyitott döntések: 8. pont.

## 1. Szabályok, amelyeket a terv betart

| Forrás | Következmény erre a tervre |
|---|---|
| `VISION.md` §1, `AGENTS.md` licenc | A CFA4 **GPL-3.0-or-later**. Az RVM (GPL-3.0, `gaps.md`) elvileg nem licencakadály, de az F0 licenckapu (az AI-mattolás terve szerint) a súlyok és a tanítóadat licencét külön el kell hogy döntse. A BMV2 licencét (MIT a mattolás-terv szerint) és a súlyait a forrásnál kell igazolni; ezt itt nem ellenőriztem |
| `VISION.md` §5 | Helyi AI, **nincs letöltés a felhasználó tudta nélkül**; modellenként licenc és forrás rögzítve. Az AI alsó határa 12 GB-os GPU és 16 GB RAM; **önálló, GPU nélküli AI-út nincs** (2026-11-07) |
| `AGENTS.md` | Gyártófüggetlen gyorsítás (nem csak CUDA); „never crash” (§0); nincs `unsafe`; más nyelven írt komponens csak külön folyamatban, ADR 0008 worker-protokollal; nincs fizetős rész |
| `coordination.md` §2.10, §2.11 | Fejlesztés csak jóváhagyott terv alapján; minden terv Markdownként a `docs/plans/`-ba |
| `coordination.md` §2.8 | Átfedésnél a projekthatárt a tulajdonos és a gazda együtt dönti el: ezért a 6. pont **javaslat**, nem végleges döntés |
| Márkaszűrő (`python/tests/test_independence.py`) | A forrásprojekt neve nem szerepelhet; a szövegben „szülőprojekt” áll |

## 2. Ami a zöldháttér-funkcióból ma létezik (kódolvasás alapján)

| Elem | Hely | Mit tud / mit találtam |
|---|---|---|
| **Ultra Key** (definíció) | `crates/project/src/effect.rs` `ultra_key` (KEYING csoport); a kiterjesztett paraméterek `crates/project/src/effect/vfx.rs` `extend_core` | Kulcsszín, Output (Composite / Alpha Channel / Color Channel), Setting (Default / Relaxed / Aggressive / Custom), Matte Generation (Transparency, Highlight, Shadow, Tolerance, Pedestal), Matte Cleanup (Choke, Soften, Contrast, Mid Point), Spill Suppression (Spill, Desaturate, Range, Luma), Color Correction |
| Ultra Key (megvalósítás) | `crates/render/src/vfx/color.rs` `ultra_key`; hívja `vfx/mod.rs` | Y′CbCr színkülönbség-kulcsoló: a kulcsszín króma-irányára vetít, merőleges távolsággal és Highlight/Shadow luma-taggal ad matte-et; tisztítás (choke, soften, kontraszt), spill-csökkentés a domináns csatornán, utána színkorrekció |
| Setting-előbeállítások | `ultra_key_setting` (`vfx.rs`) | Az „Aggressive” a leírás szerint világos és sötét ruhát tisztít tipikus zöldháttéren; ez **tervezői szándék, nem mért állítás** |
| **Color Key** | `effect.rs` `color_key`; `crates/render/src/effects.rs` `key` | Kulcsszín, tolerancia, szegély-lágyítás. **Lelet:** az „Edge Thin” paraméter (`thin`) a definícióban van, de a `key` függvény **nem olvassa** (csak `tolerance` és `feather`); a hatástalan csúszka vagy rejtendő, vagy megvalósítandó |
| Kettős kód | `effects.rs` `key` kezeli az `ultra_key`-t is | Az `effects::apply` előbb a `vfx::apply`-t hívja, ami az `ultra_key`-t lekezeli, így a `key` Ultra-ága **kódolvasás szerint elérhetetlen** (halott kód); ellenőrizendő és egységesítendő (Z1) |
| **Luma Key** | `effects.rs` `luma_key` | Küszöb és vágás |
| **Track Matte Key** | `vfx/color.rs` `track_matte`, `apply_matte` | Egy másik videósáv **alfáját vagy lumáját** szorozza a rétegre, megfordítható. Ez a **már működő, matte-fogyasztó csatlakozási pont** |
| Alpha Adjust, Logo Cutout | `vfx.rs`, `vfx/color.rs` | Alfa invertálás/átlátszóság; egyszínű (fehér/fekete) háttér kivonása |
| Pipetta | `crates/engine/src/eyedropper.rs` (`effects.eyedropper`) | A kulcsszínt a kulcsolás nélküli képből, 5×5 átlaggal veszi (zajos zöld is jó); egy visszavonási lépés; Qt6-parancs |
| Qt6 felület | `python/cfa4_qt/panels/effect_controls.py`, `effects.py`; teszt: `python/tests/test_qt_gui.py` (≈4547. sor) | Az effektparaméterek a közös Effect Controls panelen át érhetők el; külön „Zöldháttér” munkaterület nincs |
| Tesztek | `crates/render/src/vfx/tests.rs` `keyers_known_values`; `crates/golden/tests/golden.rs` `vfx_ultra_key`; `crates/engine/src/vfx_tests.rs` `ultra_key_setting_writes_aggressive_parameters`; `effects_inspect_tests.rs` | Szintetikus: tiszta zöld → átlátszó, szürke előtér → átlátszatlan; Output-módok; az Aggressive módosítja a mattét; arany kép színsávokkal egy háttér fölött. **Valódi felvételes** (haj, motion blur, zajos zöld) teszt nincs a repóban |
| GPU | `docs/hardware-parity.md`: „Keys (Ultra Key) … CPU, 10%”; `crates/render/src/gpufx.rs` nem tartalmaz kulcsolót (csak LeaveColor/ColorPass-féle színműveleteket) | **A kulcsolók CPU-n futnak**; WGSL-út és paritásteszt nincs |
| Export alfával | `crates/export/src/settings.rs` `supports_alpha`/`keeps_alpha` | Alfát tarthat: PNG/TIFF/TGA/EXR sorozat, GIF, DPX ≥ 12 bit, ProRes 4:4:4:4, CineForm RGBA 4:4:4:4. Más konténer (pl. H.264, VP9/WebM) **nem** hirdet alfát. Ez a kódban deklarált képesség; a kodekenkénti alfa-hűséget itt nem mértem |
| Más tervekben | `CFA4-LIVE-STUDIO-001` (szűrők: chroma/color/luma key), `CFA4-KDENLIVE-PLAN-001` (Chroma key ✅ = Ultra Key), `CFA4-DECKLINK-001` (hardveres key/fill) | A kulcsolás igénye több tervben megjelenik, de **mind a meglévő Ultra Key-re épít**, saját kulcsolót egyik sem tervez |

## 3. Értékelés (a zöldháttér-funkció állapota)

**Erősségei**

- Teljes, Premiere-szerű paraméterkészlet, előbeállításokkal, pipettával, kulcskockázható paraméterekkel (az effektrendszer része).
- A kimenet a Compositor szokásos alfa-csatornája, így minden meglévő fogyasztó (Track Matte, blend, export) használhatja.
- Determinisztikus CPU-referencia, arany kép, szintetikus tesztek.

**Gyengeségei és hiányai (bizonyítékkal vagy jelöléssel)**

| # | Megállapítás | Alap | Súly |
|---|---|---|---|
| H1 | A kulcsolók csak CPU-n futnak; a lejátszás/export gyorsítása nem érinti őket | `hardware-parity.md` | közepes (nagy felbontásnál érzékelhető lehet; mérés nincs) |
| H2 | Nincs valódi felvételes validáció és nincs elfogadási küszöb (haj, átlátszó anyag, motion blur, egyenetlen megvilágítás) | `tests.rs`, golden | magas (a „kész” állítás nem támasztható alá) |
| H3 | Color Key `thin` paraméter hatástalan; az `effects.rs` `key` Ultra-ága halott kód | kódolvasás | alacsony, de félrevezető |
| H4 | Nincs önálló despill-effekt (a spill csak az Ultra Key része), nincs „clean plate” / referencia-háttéres különbségi kulcsolás, nincs külön „garbage matte” munkafolyamat (a meglévő maszkok erre használhatók, de nincs ráhangolva) | kódolvasás | közepes |
| H5 | A kulcsoló nem időbeli: nincs képkockák közti simítás, a matte villoghat | `ultra_key` képkockánként dolgozik | közepes, mérendő |
| H6 | A matte nem önálló eszköz: nem menthető, nem verziózható, nem hivatkozható a projektben; csak az effekt kimenete | `project`-modell, `MaskRef` csak bináris | magas az AI-mattoláshoz |
| H7 | Nincs „háttércsere” munkafolyamat (kulcs → háttér → szín/fény illesztés → export) egy lépésben; a felhasználó kézzel rakja össze | Qt6 panelek | közepes |
| H8 | Az alfa-export mátrixa deklarált, de nincs mérési jelentés a kodekenkénti alfa-hűségről | `supports_alpha` | közepes |

## 4. Összevetés az AI Matting Engine tervével (mi létezik, mi hiányzik, mi ütközik)

A tábla a mattolás-terv szakaszai szerint halad. „Gap-only” = valóban hiányzik; „Van” = a repóban már megvan (hivatkozással); „Ütközik/Pontosítandó” = döntést vagy egyeztetést kér.

| Mattolás-terv | Állapot a repóban | Besorolás |
|---|---|---|
| BMV2 / RVM backend, közös matte-csővezeték, controller, registry | Nincs kód. RVM jelölt: `gaps.md` G12, `open-editors-parity.md` E7 (RVM MobileNetV3, GPL-3.0, K4). BMV2 sehol nem szerepel | **Gap-only** |
| „Meglévő AI Video Tools architektúra” (§2, §4) | **Nincs ilyen komponens a repóban.** Csak az `AI-MODEL-TRAINING-001` §10 sorolja kategóriaként (metaadat, parancsok, jelenetleírás, vizuális felismerés külön modellt kér) | **Ütközik a feltevéssel:** nem „meglévő”, létre kell hozni vagy az ADR 0008 AI-rétegébe kell illeszteni |
| „Meglévő AI Model Manager” (§11) | Csak hivatkozás: `CFA4-MODEL-MANAGER-001` „fogja birtokolni a tárolót és az elfogadást” (`crates/speech/README.md`, `silero.rs`); a `models/manifest.toml` és a `cargo xtask models` terv (`open-editors-parity.md` E7), de a repóban **nincs `models/` mappa** | **Gap / függőség:** a mattolás ezt nem építheti újra, de nem is használhatja, amíg nincs. A mattolás F3/F4 fázisa e fölött áll |
| Adaptive Capability System | Van alap: `crates/tasks` (`Plan`, `Monitor`, `Priority`, költségvetés, erőforrások), `crates/engine` `background`/`task_host`; az ACS-0 összevetés 2026-11-13-i állapota LEZÁRVA | **Van** (újrahasználandó); a részletes ACS/ADR 0008 a külső AI-modul tárolóban van, ezt nem olvastam |
| Job scheduler, preview/full arbitráció, GPU-memória őr | `crates/tasks` ütemező/költségvetés, ACS-4/6/7 az összevetés szerint | **Van** (újrahasználandó); matting-specifikus erőforrás-profil (RVM állapotmemória) **gap-only** |
| „Nincs második adatbázis” (§2) | A CFA4 döntése: **PostgreSQL** (CFA4-DATA-PG-001, TASKSTORE-PG-001, `crates/store`); a maszkok tartalomcímzett lemezes gyorsítótárban vannak (`object.rs`), nem adatbázisban | **Összhangban** (SQLite/redb tilos); a matte-assetek is lemezre, hash-hivatkozással mennek |
| Cache-azonosító (forrás, tartomány, backend, modell, felbontás, paraméterek…) | Mintaként ott a Warp Stabilizer elemzése: médiaujjlenyomat + forrástartomány + beállítások kulcsa (`gaps.md`); `MaskRef` = SHA-256 | **Van minta**, a matting-kulcs **gap-only** |
| Projekt-perzisztencia (job, backend, modell, paraméterek, matte, figyelmeztetések) | `ObjectTrack` provenance (modell azonosító + SHA-256) mintaként (`object.rs`) | **Van minta**; matting-rekord **gap-only** |
| Timeline / Compositor / VFX integráció | Compositor: alfa, blend, Track Matte Key (van). VFX: `CFA4-VFX-001` T2 „kép és matte” család (jóváhagyott, a render-függőségekre vár) | **Van** a fogyasztói oldal; az új „matte-asset → klip/réteg” kötés **gap-only** |
| Háttércsere-munkafolyamat | `gaps.md` G12: háttér-elmosás/csere RVM-mel, MODNet tartalékkal, átlátszó háttér, képernyőfelvétel háttérnek; Portrait Lighting. Munkafolyamat-kód nincs | **Gap-only**, és **ütközik:** `gaps.md` MODNet-et nevez tartaléknak, a mattolás-terv szerint RVM nem cserélhető csendben jóváhagyatlan alternatívára (8. pont) |
| Alfa-export validáció „csak validált codec/konténer” | `supports_alpha` lista a kódban (2. pont) | **Van** a deklaráció; a **validációs jelentés gap-only** (H8) |
| Qt6 AI Matting munkaterület | A Qt6 UI-ban nincs AI-munkaterület; az effektpanel és a Lumetri a minta (`python/cfa4_qt/panels/`) | **Gap-only** |
| „Kizárva: chroma key csere” | Chroma key = Ultra Key/Color Key (VFX ▸ Keying), kész kódként | **Összhangban**, de a határt ki kell mondani (6. pont) |
| „Kizárva: rotoszkóp, tracking, általános szegmentálás” | Az `OBJECT-MASK-PLAN-001` a (nem AI) követést, az objektummaszkolást és a SAM2-t már birtokolja | **Átfedés kezelve:** a mattolás ezt nem veszi át; az Object Mask maszk **bináris**, a matte folytonos (4. pont, 5. pont) |
| Licenckapu (RVM GPL-3.0) | A CFA4 GPL-3.0-or-later; AGPL-worker szabály 2026-11-22; `licensing.md` | **Pontosítandó:** a kapu kimenete itt valószínűleg „jóváhagyva feltételekkel”, de a döntés az F0 feladata, nem ezé a dokumentumé |
| Linux/Ubuntu 26.04/amd64, helyi, offline | `VISION.md`, `AGENTS.md` | **Összhangban** |
| „CPU-s tartalék, ha a backend támogatja; lassabb elfogadható” | `VISION.md` §5: GPU nélküli önálló AI-út nincs (2026-11-07); az ACS-összevetés a CPU-t „elfogadható” útnak tekinti | **Ütközik/Pontosítandó:** tulajdonosi döntés kell, hogy a mattolás CPU-úttal fut-e (8. pont) |

## 5. Közös pontok és határ: chroma key ↔ AI matting

```
 zöldháttér-felvétel ─► Ultra Key / Color Key ─┐                      ┌─► Track Matte Key (alfa/luma)
                                              ├─► MATTE (alfa-sík) ──┼─► klip-alfa / blend / Compositor
 bármilyen felvétel ──► AI Matting (BMV2/RVM) ─┘                      ├─► VFX (maszkolt effekt, háttércsere)
                                                                     └─► alfa-export (validált konténer)
```

**Javasolt határ (a mattolás-terv kizárását megtartva):**

| Terület | Gazdája | Megjegyzés |
|---|---|---|
| Kulcsszín-alapú kulcsolás, despill, szegély-tisztítás **effektként** | VFX ▸ Keying (meglévő) | Az AI-mattolás **nem** építi újra |
| Neurális matte előállítása (BMV2, RVM) | AI Matting Engine | Nem „kulcsolás”: nem kulcsszínt használ |
| **Matte mint eszköz** (tárolás, verzió, hash, hivatkozás, projekt-perzisztencia) | **Közös, új** (Z1) | A két forrás ugyanazt a szerződést használja |
| Matte-tisztítás (choke, soften, kontraszt, élszimmetria) | **Egy** közös sík-műveletkészlet | Ma az Ultra Key-ben van (`blur_plane`, choke, contrast); a mattolás „megosztott utófeldolgozó rétege” **ezt hívja**, nem másolja |
| Háttércsere-munkafolyamat | **Közös, új** (Z3), két matte-forrással | Kulcs vagy AI → háttér → szín-/fényillesztés (meglévő Lumetri) → export |
| Matte fogyasztása | Compositor/VFX (meglévő) | Track Matte Key a belépő; új Compositor/VFX nem készül |
| Objektummaszk (bináris, SAM2) | `OBJECT-MASK-PLAN-001` | A matte-et **nem** `MaskRle`-ben tárolja; ha egy objektummaszkból matte kell, az egy külön finomítási lépés |

**A matte-szerződés minimuma (javaslat, jóváhagyásra):** egyetlen alfa-sík képkockánként, 8 vagy 16 bit (vagy f32), `Tick` időbélyeggel (nem képkockaszámmal), felbontás és szín-/alfa-formátum rögzítve, a tartalom SHA-256-tal címezve a tartalomcímzett gyorsítótárban, a projektben csak hash-hivatkozás + származás (forrás, tartomány, backend/effekt, modell és verzió, paraméterek). A kulcsolóra is ugyanez vonatkozik, ha a felhasználó a matte-et „kisüti” (bake). Az Ultra Key „Alpha Channel” kimenete már most is a matte-et adja, csak nem menthető eszközként.

## 6. Átfedések és kockázatok

| # | Kockázat / átfedés | Hatás | Teendő |
|---|---|---|---|
| R1 | Az AI-mattolás terve olyan „meglévő” rétegekre épít (AI Video Tools, AI Model Manager), amelyek nincsenek a repóban | A terv F3/F4/F5 fázisai nem indulhatnak; a „gap-only” hatókör nagyobb a feltételezettnél | A függőségek tisztázása a Model Manager / ADR 0008 projektjével (nem itt) |
| R2 | A matte-tisztítás duplikálódik (Ultra Key vs. új megosztott utófeldolgozó) | Két, eltérő viselkedésű choke/soften | Z1: közös sík-művelet kiemelése, mindkét fogyasztó azt hívja, regressziós teszttel |
| R3 | A bináris Object Mask-tár matte-nek használása | Minőségvesztés (hajszál), félrevezető alfa | Külön alfa-matte eszköz (5. pont) |
| R4 | `gaps.md` MODNet-tartaléka vs. „RVM nem cserélhető csendben” | Tervek közti ellentmondás | Tulajdonosi döntés: a MODNet külön, jóváhagyott másodlagos backend legyen-e, vagy kikerül a `gaps.md`-ből |
| R5 | CPU vs. „nincs GPU nélküli AI-út” (VISION §5) | A mattolás elérhetősége alacsonyabb hardveren | Tulajdonosi döntés (8. pont) |
| R6 | Az RVM rekurrens állapota és a CFA4 job-/seek-modellje | Elavult állapotú hibás matte; az előnézet és a teljes render ütközése | F4 előtt tervezett állapot-életciklus; **mérés és teszt nélkül nem állítható helyesnek** |
| R7 | A kulcsolók CPU-sebessége nagy felbontáson | Lassú lejátszás, ha a matte-forrás effekt marad | Z0 mérés a referenciagépen; GPU-út csak mért igény alapján |
| R8 | Szabályok: PR-torlódás (30 nyitott PR felett új munka csak kérésre), §2.10 jóváhagyás, §2.8 projekthatár | Csúszás, ütközés más projektekkel | Z0 jóváhagyás előtt kódot nem írunk; a határ-döntés a gazdával együtt születik |
| R9 | A mattolás-terv szövege nem a repóban van (§2.11) | A terv a beszélgetésben marad | 9. pont, külön PR |
| R10 | Az alfa-export mátrix nincs kodekenként mérve | Hamis alfa-állítás kockázata | Z2: mérési jelentés (alfa megőrzése konténerenként) |

## 7. Fázisolt beépítési terv

Minden fázis **csak jóváhagyás után** indul (§2.10), „kész” csak teszt- vagy mérési bizonyítékkal. Az „AI-mattolás” oszlop azt mutatja, melyik F-fázissal fonódik össze.

| Fázis | Tartalom | AI nélkül elvégezhető? | AI-mattolás fázis | Átvételi feltétel (bizonyíték) |
|---|---|---|---|---|
| **Z0 · Alap és döntések** | (a) A 8. pont döntései; (b) a **mérési alap**: Ultra Key/Color Key minőség és sebesség szintetikus + valódi (nyílt licencű, forrásmegjelölt) zöldháttéres felvételeken a referenciagépen (CPU-idők, matte-jellemzők); ha fizikai teszt kimarad, sor a `deferred-physical-tests.md`-ben (§2.14); (c) az átfedés-döntés rögzítése a `coordination-log.md`-ben | igen | F0 (audit) | Mérési jelentés `docs/audit/`-ban; döntések a naplóban |
| **Z1 · Hygiene és közös matte-szerződés** | (a) Az `effects.rs` Ultra-ágának egységesítése/eltávolítása (H3); (b) Color Key `thin`: megvalósítás **vagy** elrejtés (H3); (c) közös sík-műveletek (choke, soften, kontraszt) kiemelése (R2); (d) **matte-eszköz** adatmodell és tartalomcímzett tár (5. pont), hostile-input teszttel (AGENTS §0.4), régi projekttel kompatibilisen | igen | F1–F2 (architektúra, matte-csővezeték) | Egységtesztek, régi projekt megnyitása, `cargo xtask layers`, `cargo deny check licenses`; a teljes validáció azon a fejen zöld |
| **Z2 · Kulcsolói mélység és alfa-export** | (a) Despill önálló effektként; (b) referencia-háttéres (clean plate) különbségi kulcsolás **klasszikus**, nem AI (a BMV2 AI-útjának nem helyettesítője); (c) garbage-matte munkafolyamat a meglévő maszkokkal; (d) opcionális időbeli simítás (H5), mérés alapján; (e) alfa-export mérési jelentés konténerenként (R10); (f) WGSL kulcsoló **csak ha Z0 mérése indokolja**, CPU-referenciával és paritástesztekkel | igen | F2, F6 (export) | Arany képek, valódi felvételes elfogadási küszöbök, alfa-megőrzési jelentés, GPU/CPU paritás, ha van GPU-út |
| **Z3 · Háttércsere-munkafolyamat** | Egy munkafolyamat, két matte-forrással (kulcsoló vagy AI-matte): forrás → matte → háttér → szín/fény (meglévő Lumetri) → előnézet → export; nem romboló; Qt6 felület az Effect Controls/projektpanel mintájára | részben (a kulcsolós út igen; az AI-s út F4 után) | F5–F6 (UI, integráció) | E2E: import → kulcs → háttér → render → alfás export → projekt újranyitása |
| *(F3–F4 az AI-mattolás saját terve)* | BMV2 és RVM backend, Model Manager, ütemező | **nem** | F3–F4, F7–F8 | Az AI-mattolás saját kapui (licenckapu, hardverérvényesítés, csomagolás) |

**Függőségi sorrend:** Z0 → Z1 → (Z2 ∥ az AI-mattolás F0–F2) → Z3. A Model Manager (R1) nélkül az AI-s backendek nem indulhatnak, de a Z0–Z2 igen.

## 8. Nyitott kérdések (tulajdonosi döntést kérnek)

1. **A projekthatár (§2.8):** elfogadod-e az 5. pont szerinti határt (kulcsoló = effekt; AI-mattolás = matte-forrás; közös: matte-szerződés, utófeldolgozó, háttércsere, alfa-export)? A gazdával együtt kell rögzíteni.
2. **Matte-eszköz:** külön alfa-matte eszköz legyen-e (javaslat), nem az Object Mask bináris RLE-je?
3. **MODNet:** a `gaps.md` MODNet-tartaléka maradjon-e külön, jóváhagyott másodlagos backendként, vagy a mattolás csak BMV2 + RVM?
4. **CPU-s mattolás:** a mattolás elérhető legyen-e GPU nélkül (a mattolás-terv szerint igen; `VISION.md` §5 szerint önálló GPU nélküli AI-út nincs)?
5. **AI Video Tools és Model Manager:** ki és mikor hozza létre őket? A mattolás-terv „meglévőnek” tekinti őket.
6. **Z0 mérési felvételek:** használhatók-e nyílt licencű zöldháttéres felvételek (forrásmegjelöléssel), vagy a tulajdonos saját felvételeit adja? A fizikai (referenciagépes) mérés kimaradása esetén a §2.14 feljegyzés kötelező.
7. **Ultra Key halott ága (H3):** az `effects.rs`-beli Ultra-ág törölhető-e (törlés tulajdonosi jóváhagyást igényelhet)?
8. **Bővítés:** a Z2 (b) klasszikus clean-plate kulcsolót kéred-e, vagy az csak a BMV2 F3-mal együtt jöjjön?

## 9. Megjegyzés a mattolás-terv szövegéről

A `CFA4-AI-MATTING-ENGINE-001` v1.0 szövege **nem szerepel a repóban**. A `coordination.md` §2.11 szerint minden terv helye a `docs/plans/`. Javaslat: a tulajdonos jóváhagyásával a terv **változatlan tartalommal, külön PR-ban** kerüljön oda (egy PR = egy téma, §2.2); ez a dokumentum erre relatív hivatkozással mutat majd.

## 10. Mit nem ellenőriztem

- A CFA4-AI_modules tárolót (ADR 0008, ACS-terv, `crates/accel`) és a cfa4-sdk-t nem olvastam: ezek az AI-réteg tényleges szerződéseit tartalmazzák, ezért a 4. pont „Van” besorolásai az ACS-összevetésre és a `crates/tasks` felületére támaszkodnak, nem az AI-modul kódjára.
- Nem futtattam buildet, tesztet, mérést; a „kész” csak a repó saját dokumentációja szerint áll.
- A BMV2 és az RVM kód-, súly- és tanítóadat-licencét a forrásnál nem ellenőriztem; az F0 kapu feladata.
- Nem néztem meg, hány PR nyitott (a §2.6 küszöb).
