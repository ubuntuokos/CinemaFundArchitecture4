# CFA4-GREENSCREEN-MATTING-INTEGRATION-001: a zöldháttér (chroma key) elemzése és beépíthetőségi terve

| Dokumentumadat | Érték |
|---|---|
| Állapot | **JÓVÁHAGYOTT (István, 2026-11-23; 0.2: ütközésmentes illesztési terv).** A fejlesztés a 7. pont előfeltételei szerint indul ([`coordination.md`](../coordination.md) §2.10) |
| Dátum | 2026-11-23 |
| Kérte | István (ubuntuokos) |
| Alap | CFA4 `main` @ `bc3308f` |
| Módszer | **Csak olvasás:** forráskód, tesztek és tervek átnézése. Nem futtattam buildet, tesztet vagy mérést, ezért ez a dokumentum **nem állít validációt**; a „kész” szó csak ott áll, ahol a repó saját tesztje vagy doksija ezt rögzíti, hivatkozással |
| Összevetett tervek | `CFA4-AI-MATTING-ENGINE-001` v1.0 (APPROVAL BASELINE; a szövege **nincs a repóban**, lásd 9. pont), [`OBJECT-MASK-PLAN-001`](OBJECT-MASK-PLAN-001-2026-11-03.md), [`CFA4-VFX-001`](CFA4-VFX-001.md), [`CFA4-ADAPTIVE-CAPABILITY-001-OSSZEVETES`](CFA4-ADAPTIVE-CAPABILITY-001-OSSZEVETES.md), [`CFA4-AI-MODEL-TRAINING-001`](CFA4-AI-MODEL-TRAINING-001.md), [`CFA4-KDENLIVE-PLAN-001-FINAL-1.0`](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) KDN-4, [`CFA4-DATA-PG-001-1.0`](CFA4-DATA-PG-001-1.0.md), [`CFA4-TASKSTORE-PG-001`](CFA4-TASKSTORE-PG-001.md), [`docs/gaps.md`](../gaps.md) G12/G18, [`docs/open-editors-parity.md`](../open-editors-parity.md) E7, [`docs/hardware-parity.md`](../hardware-parity.md), [`VISION.md`](../../VISION.md), [`AGENTS.md`](../../AGENTS.md) |
| Nyelv | magyar |
| Jelölés | A Kdenlive-pontok jele **KDN-n** (a nyitott PR #1774 átszámozása; a `main`-ben ez még `K<n>`). A `KDN-4` = „AI háttéreltávolítás (SAM2-szerű)”, régi jele K4. A `K1–K16` jelölés a `LOSSLESS-MEDIA-TOOLS-001` tervé, ahhoz itt nem nyúlunk |

## 0. Röviden

1. **A klasszikus zöldháttér-kulcsolás megvan, de csak CPU-n, és vékonyan bizonyítva.** A kulcsoló az **Ultra Key** (`crates/render/src/vfx/color.rs`, `ultra_key`) és az egyszerűbb **Color Key**. Van hozzájuk egységteszt, arany kép és Qt6-os parancsteszt (2. pont). Mérést valódi zöldháttéres felvételen, GPU-s utat és önálló despill-effektet a repóban nem találtam.
2. **Az AI-mattolás (BMV2, RVM) a repóban még nincs.** Sem kód, sem a terv szövege. Az RVM csak jelöltként szerepel (`docs/gaps.md` G12, `docs/open-editors-parity.md` E7, KDN-4). Az építőelemek közül hiányzik az **AI Model Manager** és az **AI Video Tools** is: az előbbit csak hivatkozások nevezik (`CFA4-MODEL-MANAGER-001`, „will own the store”), az utóbbi csak kategóriaként szerepel az `AI-MODEL-TRAINING-001` §10-ben. Az ADR 0008 és az ACS az **AI-modulok külön tárolójában** él, amelyet ez a munkamenet nem érhetett el.
3. **A határ:** a kulcsoló **effekt** marad (VFX ▸ Keying), az AI-mattolás **matte-forrás**. A kettő közös pontja: **a matte mint érték**, amelyet a Compositor már ma is fogyaszt (Track Matte Key, klip-alfa), valamint a háttércsere-munkafolyamat, a szegély-/spill-tisztítás és az alfa-export. Ezekben egyszer kell megépíteni, nem kétszer.
4. **A legfontosabb új lelet:** az Object Mask terv maszkmodellje (`MaskRle`, `MaskRef`) **bináris** (0/1 futáshossz). A mattolás **folytonos alfát** ad, finom hajszálakkal. Ezért az Object Mask tárolója **nem használható** matte-nek; külön, tartalomcímzett alfa-matte eszköz kell (4a. pont).
5. **Javaslat (0.2):** a terv nem új alrendszer. Tizenhét ütközésmentességi szabály (5. pont) és egy tervenkénti illesztési mátrix (6. pont) rögzíti, mit birtokol ki; a Z0–Z3 fázisok (7. pont) a gazdaprojektek kapuihoz kötöttek. A tulajdonosi döntések a 8. pontban vannak.

## 1. Szabályok, amelyeket a terv betart

| Forrás | Következmény erre a tervre |
|---|---|
| `VISION.md` §1, `AGENTS.md` licenc | A CFA4 **GPL-3.0-or-later**. Az RVM (GPL-3.0, `gaps.md`) elvileg nem licencakadály, de az F0 licenckapu (az AI-mattolás terve szerint) a súlyok és a tanítóadat licencét külön el kell hogy döntse. A BMV2 licencét (MIT a mattolás-terv szerint) és a súlyait a forrásnál kell igazolni; ezt itt nem ellenőriztem |
| `VISION.md` §5 | Helyi AI, **nincs letöltés a felhasználó tudta nélkül**; modellenként licenc és forrás rögzítve. Az AI alsó határa 12 GB-os GPU és 16 GB RAM; **önálló, GPU nélküli AI-út nincs** (2026-11-07) |
| `AGENTS.md` | Gyártófüggetlen gyorsítás (nem csak CUDA); „never crash” (§0); nincs `unsafe`; más nyelven írt komponens csak külön folyamatban, ADR 0008 worker-protokollal; nincs fizetős rész |
| `coordination.md` §2.10, §2.11 | Fejlesztés csak jóváhagyott terv alapján; minden terv Markdownként a `docs/plans/`-ba |
| `coordination.md` §2.8 | Átfedésnél a projekthatárt a tulajdonos és a gazda együtt dönti el: ezért az 5–6. pont **javaslat**, nem végleges döntés |
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
| BMV2 / RVM backend, közös matte-csővezeték, controller, registry | Nincs kód. RVM jelölt: `gaps.md` G12, `open-editors-parity.md` E7 (RVM MobileNetV3, GPL-3.0, KDN-4). BMV2 sehol nem szerepel | **Gap-only** |
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
| „Kizárva: chroma key csere” | Chroma key = Ultra Key/Color Key (VFX ▸ Keying), kész kódként | **Összhangban**, de a határt ki kell mondani (4a. pont) |
| „Kizárva: rotoszkóp, tracking, általános szegmentálás” | Az `OBJECT-MASK-PLAN-001` a (nem AI) követést, az objektummaszkolást és a SAM2-t már birtokolja | **Átfedés kezelve:** a mattolás ezt nem veszi át; az Object Mask maszk **bináris**, a matte folytonos (4a. pont) |
| Licenckapu (RVM GPL-3.0) | A CFA4 GPL-3.0-or-later; AGPL-worker szabály 2026-11-22; `licensing.md` | **Pontosítandó:** a kapu kimenete itt valószínűleg „jóváhagyva feltételekkel”, de a döntés az F0 feladata, nem ezé a dokumentumé |
| Linux/Ubuntu 26.04/amd64, helyi, offline | `VISION.md`, `AGENTS.md` | **Összhangban** |
| „CPU-s tartalék, ha a backend támogatja; lassabb elfogadható” | `VISION.md` §5: GPU nélküli önálló AI-út nincs (2026-11-07); az ACS-összevetés a CPU-t „elfogadható” útnak tekinti | **Ütközik/Pontosítandó:** tulajdonosi döntés kell, hogy a mattolás CPU-úttal fut-e (8. pont) |

## 4a. Közös pontok és határ: chroma key ↔ AI matting

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

## 4b. Kockázatok

| # | Kockázat / átfedés | Hatás | Teendő |
|---|---|---|---|
| R1 | Az AI-mattolás terve olyan „meglévő” rétegekre épít (AI Video Tools, AI Model Manager), amelyek nincsenek a repóban | A terv F3/F4/F5 fázisai nem indulhatnak; a „gap-only” hatókör nagyobb a feltételezettnél | A függőségek tisztázása a Model Manager / ADR 0008 projektjével (nem itt) |
| R2 | A matte-tisztítás duplikálódik (Ultra Key vs. új megosztott utófeldolgozó) | Két, eltérő viselkedésű choke/soften | Z1: közös sík-művelet kiemelése, mindkét fogyasztó azt hívja, regressziós teszttel |
| R3 | A bináris Object Mask-tár matte-nek használása | Minőségvesztés (hajszál), félrevezető alfa | Külön alfa-matte eszköz (4a. pont) |
| R4 | `gaps.md` MODNet-tartaléka vs. „RVM nem cserélhető csendben” | Tervek közti ellentmondás | Tulajdonosi döntés: a MODNet külön, jóváhagyott másodlagos backend legyen-e, vagy kikerül a `gaps.md`-ből |
| R5 | CPU vs. „nincs GPU nélküli AI-út” (VISION §5) | A mattolás elérhetősége alacsonyabb hardveren | Tulajdonosi döntés (8. pont) |
| R6 | Az RVM rekurrens állapota és a CFA4 job-/seek-modellje | Elavult állapotú hibás matte; az előnézet és a teljes render ütközése | F4 előtt tervezett állapot-életciklus; **mérés és teszt nélkül nem állítható helyesnek** |
| R7 | A kulcsolók CPU-sebessége nagy felbontáson | Lassú lejátszás, ha a matte-forrás effekt marad | Z0 mérés a referenciagépen; GPU-út csak mért igény alapján |
| R8 | Szabályok: PR-torlódás (30 nyitott PR felett új munka csak kérésre), §2.10 jóváhagyás, §2.8 projekthatár | Csúszás, ütközés más projektekkel | Z0 jóváhagyás előtt kódot nem írunk; a határ-döntés a gazdával együtt születik |
| R9 | A mattolás-terv szövege nem a repóban van (§2.11) | A terv a beszélgetésben marad | 9. pont, külön PR |
| R10 | Az alfa-export mátrix nincs kodekenként mérve | Hamis alfa-állítás kockázata | Z2: mérési jelentés (alfa megőrzése konténerenként) |

## 5. Ütközésmentességi szabályok (a terv kötelező keretei)

Ez a terv **nem új alrendszer**, hanem a meglévő tervek gazdaprojektjeibe illeszkedő igény- és munkajegyzék. Minden szabály mellett ott a forrás, amely miatt kötelező. „Egy képességnek egy elsődleges megvalósítása van” ([`CFA4-VFX-001`](CFA4-VFX-001.md) §5).

| # | Szabály | Forrás |
|---|---|---|
| S1 | **Nincs új kiértékelő, sor, ütemező, cache, adatbázis, Model Manager, Compositor vagy VFX-rendszer.** A kulcsolók effektek maradnak a render-gráfban; az AI-matte egy feladat (`Task`) a meglévő feladatsorban | VFX-001 X-1 és §3.1; `RENDER-INTEGRATION-PLAN-001` F3; `TASKSTORE-PG-001`; `ADR-0007`; mattolás-terv §2 |
| S2 | **A kulcsolók effektazonosítói és paraméterazonosítói befagyasztottak:** csak hozzáfűzés (`extend_core` minta, régi példány az alapértékre esik). Nem törlünk és nem nevezünk át paramétert (pl. Color Key `thin`) | MLT-import leképezés (`SC5-MLT-IMPORT` „kulcsolás → a CFA4 megfelelő effektje”), régi projektek; `coordination.md` §2.5 |
| S3 | **Új képesség csak a közös katalógus saját sorával** (`F0-capability-gap-matrix.csv`, `G-xx`), az effektlistába csak a saját sor kerül | VFX-001 X-3; `coordination.md` §2.5 |
| S4 | **Új kulcsolót nem építünk a VFX-katalógus hiánylistája és a VFX-5 (EC-donor effektek, G-33) nélkül.** A donor leltára még `NOT_AUDITED`: lehet, hogy a despill vagy a clean-plate kulcsoló ott már szerepel | VFX-001 VFX-1/VFX-5, G-33; `VFX-CATALOG-001` hiánylista („kulcsolók”) |
| S5 | **A matte-szerződés a render-szerződések (C0) része, nem külön formátum.** Alfa-mód (straight / premultiplied) és belső ábrázolás (premultiplikált lineáris f32) az `A2 – Egységes Frame ABI` és az MQ-4 szerint; a szerződés változtatása csak ADR-rel | `RENDER-INTEGRATION-PLAN-001` A2, MQ-4, `C0-contracts-spec.md` |
| S6 | **A matte-gyorsítótár a render-gráf csomópont-gyorsítótára** (hash-ellenőrzés, a gyorsítótár kikapcsolásával is PASS), a lemezes tár a meglévő **tartalomcímzett** tár; nincs külön matte-cache-motor | `RENDER-INTEGRATION-PLAN-001` F3 (scheduler, cache); `object.rs` `MaskRef`; `CFA4-DATA-PG-001` (maszkok lemezen) |
| S7 | **A matte-feladat a `cfa4-tasks`/`cfa4-store` feladatsorába kerül** (család: `ai`, feladat: `ai.matting.<backend>`), a `routing.toml` `ai` családjának soraival; a job-állapot PostgreSQL, nem SQLite vagy redb | `ADR-0004` (család, `routing.toml`), `ADR-0007`, ACS-összevetés, OBJECT-MASK Változások (2) |
| S8 | **A modellek az AI-réteg modellkezelőjén át jönnek** (`CFA4-MODEL-MANAGER-001`, ADR 0008, `models/manifest.toml`, négy licencmező: `code`, `dependencies`, `weights`, `dataset`); letöltés csak felhasználói művelettel, a `cfa4-fetch` folyamatban | `VISION.md` §5; `AGENTS.md`; `threat-model.md` P6; OBJECT-MASK §3 |
| S9 | **Csak biztonságos modellformátum: safetensors / ONNX / GGUF.** A pickle-alapú PyTorch-súlyfájl (`.pth`, `.ckpt`) betöltése tilos; csak elszigetelt átalakítóval vihető át | `threat-model.md` T4, P6, B7/D7 |
| S10 | **Az inferencia külön worker-folyamat** az ADR 0008 protokollal, `cfa4-sandbox` profillal (új folyamattípus csak a SECURITY-001 gazdájával egyeztetve); a Python/C++ megtartása indoklással, referenciagépen mért „nem lassít” feltétellel | `AGENTS.md` (Port or keep), `CFA4-SECURITY-001`, G-25 |
| S11 | **Gyártófüggetlenség:** az inferencia útja nem lehet csak CUDA; NVIDIA-út csak AMD/Intel/NPU vagy CPU-út mellett | `VISION.md` §2.4; `ADR-0004` |
| S12 | **A szubjektum-kijelölés (objektummaszk, SAM2, követés) az OBJECT-MASK és a Kdenlive KDN-4 hatásköre.** A mattolás ezt nem építi újra; bemenetként használhatja (lásd 6. pont) | OBJECT-MASK §1, VFX-001 §3.1, G-34 |
| S13 | **Az ellenőrzés a meglévő F0V-eszközökön fut** (`cfa4-golden`, `cfa4-metrics`, `cfa4-testkit`), a küszöb mért (*Küszöb@F0*), nem előre választott szám. A determinisztikus CPU-kulcsoló az F0V-küszöbön (RGB- és alfa-eltérés 0) marad; az AI-matte nem determinisztikus gépek között, ezért külön, méréssel meghatározott küszöböt kap | `RENDER-INTEGRATION-PLAN-001` F0V; `F0V-thresholds.md`; G-06, G-28 |
| S14 | **Alfa-export csak validált konténeren, a meglévő QC-vel.** A `supports_alpha` lista a kiindulás; a validációt a `crates/qc` bővítése adja, nem új ellenőrző | G-11, G-22; mattolás-terv §18 |
| S15 | **Az élő stúdió és a DeckLink a saját útján marad.** Az élő chroma/color/luma key szűrő a `cfa4-studio-host` (libobs) szűrője; a DeckLink hardveres key/fill a LIVE S4 után. Ez a terv ezeket nem érinti | `LIVE-STUDIO-001` (IV. 3., V.), `DECKLINK-001` F7 |
| S17 | **A felhasználó választ a háttéreltávolítási módszerek közül** (tulajdonosi irány, 2026-11-23): *Kulcsolás* (Ultra Key / Color Key / Luma Key, ma is elérhető), *Kijelölés* (objektummaszk, KDN-4/SAM2, bináris), *AI-matte* (Auto, BMV2 vagy RVM). Az „Auto” csak **javasol** (pl. ha van zöld/kék háttér és referenciakép), **nem vált csendben**, nem tölt le modellt, és nem változtatja meg a munkafolyamat követelményeit. Ha a választott módszer nem elérhető (nincs modell, nincs worker, nincs elég hardver), az állapot kimondott hiba/figyelmeztetés, nem néma visszalépés egy másik módszerre. A kulcsolás **mindig elérhető marad**, AI nélkül is | Tulajdonosi utasítás; mattolás-terv §9, §20 (nincs néma backend-váltás); `VISION.md` §5 |
| S16 | **Folyamat:** egy PR = egy téma; fejlesztés csak jóváhagyott terv alapján, és tervmódosításnál a munka megáll; nincs beolvasztás saját kézzel; kockázatos PR-on CodeRabbit; a fizikai teszt kihagyása sort kap a `deferred-physical-tests.md`-ben | `coordination.md` §2.2, §2.3, §2.9, §2.10, §2.14 |

## 6. Illesztési mátrix: minden érintett terv

Oszlopok: **mit birtokol** az a terv; **mit teszünk** ennek tiszteletben tartásával; **állapot** = a viszony ellenőrzöttsége. „Rögzített” = a repó szövege alapján egyértelmű. „Döntés kell” = tulajdonos és gazda együtt dönt (`coordination.md` §2.8), ezt a terv nem old fel.

| Terv / dokumentum | Mit birtokol | Mit teszünk | Állapot |
|---|---|---|---|
| `VISION.md`, `AGENTS.md`, `coordination.md` | Cél, szabályok, folyamat | S1–S17; a VISION §4 nem bővül | Rögzített |
| `CFA4-RENDER-INTEGRATION-PLAN-001` (F0V, C0, F3 render-gráf és cache, F4 GPU, F5 I/O, G-xx mátrix) | Render-gráf, ütemező, cache, szerződések, validációs küszöb | A kulcsolók és a matte-csomópont a gráfban; a matte a C0 Frame ABI része; küszöb az F0V-ből; GPU-kulcsoló csak F4 után, CPU-referenciával | Rögzített; a C0-változtatás ADR-t kér |
| `CFA4-VFX-001` (+ `VFX-CATALOG-001`, `DNEG-VFX-EGYEZTETES`) | `VfxGraph` (szerkesztési adat), T2 „kép és matte” család, effekt-katalógus, kulcsoló-hiánylista | Kulcsoló-igények a hiánylistára; VFX-5/G-33 előz meg; nincs saját csomópontszerkesztő vagy kiértékelő | Rögzített |
| `OBJECT-MASK-PLAN-001` | Objektumkövetés, bináris objektummaszk (`MaskRle`, `MaskRef`), SAM2, `Mask.source` | Nem használjuk a bináris RLE-t matte-nek; a SAM2-maszk bemenet lehet; a tartalomcímzett tár közös | Rögzített |
| `CFA4-KDENLIVE-PLAN-001` KDN-4 („AI háttéreltávolítás, SAM2”), `kdenlive-parity.md` | SAM2-alapú háttéreltávolítás | **Átfedés:** a KDN-4 „háttéreltávolítás” és a mattolás „háttércsere” ugyanazt a felhasználói célt fedi. A javaslat: **egy** „Háttér eltávolítása/cseréje” munkafolyamat, amely forrásként a kulcsolót, a KDN-4 maszkját vagy az AI-mattét veszi (7. pont, Z3). A KDN-4 sorának átírása a Kdenlive-terv gazdájáé | **Döntés kell** |
| `CFA4-AI_modules` ADR 0008, ACS-terv, `CFA4-MODEL-MANAGER-001`, `AI-MODEL-TRAINING-001`, `ACS-0 összevetés` | AI-réteg, modellkezelő, erőforrás-kezelő, worker-protokoll, licenc-elfogadás | Csak interfész; S7–S11. **Az ADR 0008 és a Model Manager szövegét nem láttam** | **Feltételes** (külső tároló) |
| `TASKSTORE-PG-001`, `DATA-PG-001`, `ADR-0006/0007`, Event Fabric, `crates/tasks`, `crates/store` | Feladatsor, jobtár, PostgreSQL, eseménycsatorna | Matte-job a meglévő sorban; nincs második adatbázis | Rögzített |
| `ADR-0004` compute routing, `ADR-0005` media backends | Útválasztás, `ai` család, `routing.toml` | `ai.matting.*` sorok felvétele az AI-réteg gazdájával; nincs közvetlen backend-hívás | Rögzített (a `routing.toml` ONNX-sorai még hiányoznak, OBJECT-MASK Változások 7) |
| `CFA4-SECURITY-001`, `threat-model.md`, `crates/sandbox` | Folyamatprofilok, modellformátum-szabály, letöltés a `cfa4-fetch`-ben | S9, S10; új worker-profil egyeztetve | Rögzített |
| `CFA4-ADAPTIVE-CAPABILITY-001` (ACS) | Erőforrás-költségvetés, prioritás, CPU/GPU szabályok | Az RVM rekurrens állapotának memóriája a költségvetésbe kerül; a CPU-út kérdése nyitott (8. pont) | **Döntés kell** (CPU vs. `VISION.md` §5) |
| `CFA4-LIVE-STUDIO-001` (+ ST2110) | Élő jelenet, libobs-szűrők (chroma/color/luma key), Rust-cseréjük | Nem érintjük. Ha a libobs-szűrőt Rust kódra cserélik, az Ultra Key-t kell hívni (egy elsődleges megvalósítás). Élő mattolás nincs a tervben | Rögzített; a későbbi csere **jegyzendő** |
| `CFA4-DECKLINK-001` F7 (hardveres keyer, key/fill) | Kártyás key/fill | A key/fill alfa-bemenete a program alfa-síkja; nem érintjük | Rögzített |
| `CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001` (+ SC5 MLT-import) | MLT/Shotcut leképezés, pipetta | S2: az effekt- és paraméterazonosítók maradnak; a pipetta (`effects.eyedropper`) újrahasznosítandó | Rögzített |
| `CFA4-EFFECTCHAIN-001` | Effektlánc-preset, macro | Kulcsoló-preset (pl. Ultra Key Setting) az EC-ben is használható; nincs külön preset-tár | Rögzített |
| `CFA4-TITLE-CREDITS-001`, `LINEART-001`, `docs/graphics.md` | Grafika, Lottie | Háttérként használható grafika; nem érintjük | Rögzített |
| `SMART-CUT-RENDER-R1`, `SMART-RENDER`, `SCE-001` | Smart render, stream copy | Kulcsolt/matte-os klip nem lehet stream-copy; **a meglévő szabály ellenőrzendő** Z0-ban | **Ellenőrizendő** |
| `MEDIA-QC`, `crates/qc` | Médiaellenőrzés | Alfa-ellenőrzés itt bővül (S14) | Rögzített |
| `CFA4-MEDIA-INTELLIGENCE-001` | Metaadat, keresés (`mi` séma, pgvector) | A matte-metaadat opcionálisan indexelhető később; nem függőség | Rögzített |
| `CFA4-DNEG-TECHNOLOGY-ADAPTATION-001` | VDB, volume, AX | Nem érintett (a track matte/blend „Kimarad”, D1–D9 A7) | Rögzített |
| `CFA4-AI-MODEL-TRAINING-001` | Tréning, licenc-elfogadás (`awaiting: licenseAcceptance`) | Modelltréning kizárt a mattolásból; a licenc-elfogadás szemantikája újrahasznosítandó | Rögzített |
| `docs/gaps.md` G12, G18; `target-app-parity.md`, `qt-parity.md`, `open-editors-parity.md` E7 | Paritás- és hiánylisták | Csak saját sor. **Ütközés:** G12 a MODNet-et tartaléknak nevezi, a mattolás-terv szerint RVM nem cserélhető csendben jóváhagyatlan alternatívára → a MODNet „jelölt, nem jóváhagyott” marad | **Döntés kell** |
| `CFA4-DEV-AI-ORCHESTRATION-001` | Fejlesztési AI-koordináció (fejlesztői eszköz) | Nem termékfunkció, nem érintett | Rögzített |
| `ADR-0001` (FFI), `licensing.md` | `unsafe`/FFI szabály, licencek | Nincs `unsafe` a CFA4 saját kódjában; RVM/BMV2 licence F0-ban, regisztersorral | Rögzített |
| **Nyitott PR-ok, amelyek e terv fogalmait érintik** (állapot 2026-11-23, nem beolvasztva) | #1774 (Kdenlive `K` → `KDN-` átszámozás, OBJECT-MASK, VFX-001, SHOTCUT stb. szövegét is érinti); #1584 (ACS-8: licenc-elfogadás és modell-kötegelés a `cfa4-tasks` sorában); #1600 (OM-A5: `objects.*` parancsok); #1654 (VFX-4 kifejezésmag); #1766 (PG-4 feladatsor PostgreSQL-ben); #1773, #1761, #1765 (K37 Kdenlive-pont címkék) | Hivatkozásaim a **jelenlegi `main`** szövegére épülnek; a #1774 beolvasztása után a hivatkozott tervekben a jelölések változnak (tartalom nem), ez a dokumentum már a `KDN-` jelölést használja. A #1584 az S8 licenc-elfogadási szemantikáját, a #1766 az S7 PostgreSQL-feladatsorát hozza be; ezek beolvasztása előtt az S7/S8 „terv szerint”, nem „kódban kész” | **Figyelendő** |
| `CFA4-AI-MATTING-ENGINE-001` v1.0 | Mattolás (nincs a repóban) | Minden fázisa a fenti gazdák kapuihoz igazodik (7. pont); a szövege külön PR-ban kerüljön a repóba (9. pont) | **Döntés kell** (§2.11) |

**Szubjektum-kijelölés és matte, egy mondatban (S12):** az objektummaszk (bináris, SAM2) a „*mi* a téma”, a matte (folytonos alfa) a „*mennyire* áttetsző a széle”. A SAM2-maszk lehet az RVM/BMV2 bemenete vagy durva előzetese, a matte-et viszont nem helyettesíti.

## 7. Fázisolt beépítési terv (mindegyik a gazdaprojekt kapujához kötve)

Minden fázis csak jóváhagyás után indul (§2.10); „kész” csak teszt- vagy mérési bizonyítékkal. A fázis nem előzheti meg a gazdaprojekt előfeltételét.

| Fázis | Tartalom | Előfeltétel (gazdaprojekt kapuja) | Átvételi feltétel (bizonyíték) |
|---|---|---|---|
| **Z0 · Alap, döntések, ellenőrzések** | (a) A 8. pont döntései és a §2.8 átfedés-döntés a `coordination-log.md`-ben; (b) mérés az **F0V-eszközökön**: Ultra Key/Color Key idő és matte-jellemzők szintetikus és nyílt licencű valódi zöldháttéres felvételen; fizikai mérés kimaradásakor §2.14 sor; (c) kiderítendő: kulcsolt klip kihagyja-e a smart render / stream copy utat; (d) G12/G18 és `G-xx` saját sorok | A jóváhagyott terv | Mérési jelentés `docs/audit/`-ban; golden/metrikák az F0V-harnessen |
| **Z1a · Kulcsoló-hygiene** | Az `effects.rs` `key` Ultra-ágának egységesítése (kódolvasás: elérhetetlen); a Color Key `thin` megvalósítása **a paraméterazonosító megtartásával** (S2); hostile-paraméter teszt | Z0 | Egységteszt, golden változatlan, régi projekt megnyitható, MLT-import teszt zöld |
| **Z1b · Matte mint érték (ADR-javaslat)** | ADR-javaslat a C0 gazdájának: egycsatornás matte-sík a Frame ABI-ban (alfa-mód, 8/16 bit/f32, `Tick` időbélyeg, adat- és nem szín-átvitel), hash-hivatkozás a tartalomcímzett tárban; kulcsoló és AI-matte ugyanazt használja. A kulcsoló „Alpha Channel” kimenete a belépő (jelenleg sRGB→lineáris átalakítással jelenítődik meg, ezt a szerződésnek tisztáznia kell) | C0 v0 elfogadott; render F3 (render-gráf) | Elfogadott ADR; hostile-input és régi-projekt teszt; `cargo xtask layers` |
| **Z2 · Kulcsolói mélység** | A despill, clean-plate (referencia-háttéres) és garbage-matte igények **hiánylista-sorként** a VFX-katalógusba; megvalósítás csak a VFX-1 donorleltár és a VFX-5 sorrendje szerint, ha a leltár nem fedi; opcionális időbeli simítás mérés alapján; alfa-export QC-bővítés (S14); WGSL kulcsoló csak mért igényre, F4 után, CPU-referenciával | VFX-1, VFX-5, render F3/F4 | Golden, valódi felvételes küszöb (mért), alfa-megőrzési jelentés, CPU–GPU paritás (ha van GPU-út) |
| **Z3 · Egységes háttér-munkafolyamat (felhasználói választással, S17)** | Egy munkafolyamat, ahol a felhasználó **módszert választ** (Kulcsolás / Kijelölés / AI-matte: Auto, BMV2, RVM), ugyanazzal az előnézettel, matte-ellenőrzéssel, háttérválasztással és exporttal; a módszer menet közben váltható, a forrás nem változik (nem romboló), a kapott matte-ek verziózva megmaradnak. A három forrás: forrás → matte → háttér → szín/fény (Lumetri) → előnézet → export; nem romboló; Qt6-os felület a meglévő panelek mintájára; a KDN-4 átfedés feloldása a 6. pont szerinti döntéssel | Z1b; Kdenlive KDN-4 és OBJECT-MASK C szakasz; AI-réteg F-kapuk a mattolás-terv szerint | E2E: import → matte → háttér → render → alfás export → projekt újranyitása |
| *AI-matte backendek (BMV2, RVM)* | A mattolás-terv F0–F8 fázisai, a fenti S7–S11 szabályokkal | ADR 0008 kapuk, Model Manager, licenckapu | A mattolás-terv saját kapui |

**Függőségi sorrend:** Z0 → Z1a ∥ Z1b → Z2 → Z3; az AI-backendek az AI-réteg kapui szerint, a Z1b matte-szerződés elfogadása után kapcsolódnak.

## 8. Döntések (István, 2026-11-23, „javaslatod szerint”)

A tulajdonos a hét kérdésre a dokumentumban írt javaslat szerint döntött. A terv ezzel **jóváhagyott**; a Z0 indulhat, a további fázisok a 7. pont előfeltételei szerint.

| # | Kérdés | Döntés |
|---|---|---|
| 1 | A terv, az S1–S17 szabályok és a határok (kulcsolás = effekt; AI-matte = `ai.matting.*` feladat; módszerválasztás a felhasználónál) | **Jóváhagyva** |
| 2 | CPU-s mattolás | **A `VISION.md` §5 szabálya marad:** önálló, GPU nélküli AI-út nincs. A kulcsolás GPU nélkül is működik. A kérdés újranyitható mért CPU-útvonallal és tulajdonosi döntéssel |
| 3 | MODNet | **„Jelölt, nem jóváhagyott”** marad a `gaps.md`-ben; a mattolás csak BMV2 + RVM |
| 4 | AI Video Tools | **Az AI-réteg (ADR 0008) része**, nem külön terv; az AI-réteg gazdája egyezteti |
| 5 | Z0 mérési felvételek | **Nyílt licencű, forrásmegjelölt felvételek** a méréshez; a tulajdonos saját felvételei a referenciagépes méréshez később (kimaradáskor `deferred-physical-tests.md` sor, §2.14) |
| 6 | Kódtisztítás | **Az `effects.rs` Ultra-ága törölhető** (a Z1a külön PR-ban, hostile-teszttel); a clean-plate kulcsoló **csak hiánylista-sor** a VFX-katalógusban |
| 7 | A mattolás-terv szövege | **Kerüljön a `docs/plans/`-ba**, külön PR-ban, változatlan tartalommal (§2.2, §2.11) |

**Nem tulajdonosi döntés:** a KDN-4 sor átírása a Kdenlive-tervek gazdájáé (az átszámozást a #1774 végzi).

**Még nyitott, nem a tulajdonosé:** az ADR 0008 és a Model Manager tényleges szerződései (külső tároló), a C0-ADR elfogadása (Z1b), a #1774, #1584, #1766 beolvasztása.

## 9. Megjegyzés a mattolás-terv szövegéről

A `CFA4-AI-MATTING-ENGINE-001` v1.0 szövege **nem szerepel a repóban**. A `coordination.md` §2.11 szerint a helye a `docs/plans/`. Javaslat: jóváhagyással, **változatlan tartalommal, külön PR-ban** kerüljön oda (§2.2); ez a dokumentum erre relatív hivatkozással mutat majd.

## 10. Mit nem ellenőriztem, és mit jelent itt az „ütközésmentes”

- Az „ütközésmentes” itt azt jelenti: **a repóban lévő tervek szövegével** (6. pont) összevetve nincs olyan lépés, amely másik terv hatáskörébe nyúlna; ahol átfedés van, a mátrix „Döntés kell” jelzést ad, és a terv nem old fel (a 8. pont döntései után is a gazdák hatásköre marad). Teljes (100%-os) ütközésmentesség **nem bizonyítható** olyan tervekkel, amelyek szövege nincs a repóban.
- A CFA4-AI_modules tárolót (ADR 0008, ACS-terv, `CFA4-MODEL-MANAGER-001`) és a cfa4-sdk-t nem olvastam; az ezekhez fűződő sorok **feltételesek**.
- Nem futtattam buildet, tesztet, mérést; „kész” csak a repó saját dokumentációja szerint áll.
- A BMV2 és az RVM kód-, súly- és tanítóadat-licencét, valamint hivatalos súlyformátumát a forrásnál nem ellenőriztem; az F0 kapu és az S9 szabály feladata.
- Nem néztem meg, hány PR nyitott (a §2.6 küszöb).
