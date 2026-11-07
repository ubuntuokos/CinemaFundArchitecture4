# ADR 0007 – CFA4 Media Services: a Rust-motor kiajánlása szolgáltatásként

**Dátum:** 2026-11-02 · **Állapot:** az irány elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-02-án:
„a Rust-motor kiajánlása”, nem külön szolgáltatás); a felület részletei az MS-1 fázis G2 kapuján
dőlnek el · **Érintett rész:** `apps/cfa4-cli` (`serve`), `crates/engine`; új médiamotor nem készül

## Háttér

A BacklotHub (CFA4-BACKLOTHUB-001) produkciókezelő a médiafeladatait egy „CFA4 Media Services”
szolgáltatásra bízza. Ilyen feladat a beérkező fájl vizsgálata, a review-proxy, a bélyegkép és a
Review lejátszása. Ilyen nevű szolgáltatásnak a CFA4-ben nem volt terve. A szabály szerint nem
létező közös szolgáltatásra modul nem épülhet, és stubbal sem kerülhető meg. Ezért előbb el kell
dönteni, mi a Media Services.

A szükséges képességek nagy része már megvan a Rust-motorban:

| Képesség | Hol van ma |
|---|---|
| Fájlvizsgálat (formátum, kodek, időalap, időkód, színinfó) | `cfa4-media`, `cfa4-cli probe`, `media.colorInfo`, `media.timecode` |
| Dekódolás saját dekóderekkel, automatikus útválasztással, CPU-tartalékkal | `cfa4-media`, `cfa4-route` (ADR 0004), ADR 0005 |
| Képkocka adott időpontban, binárisan | `cfa4-cli serve` → `frame.render` (RGBA8), MCP `render_frame` |
| Bélyegkép projekten kívüli fájlról | `serve` → `mediaBrowser.thumbnail` |
| Hanghullám-csúcsok | `serve` → `media.peaks` |
| Proxykészítés háttérfeladatként (ProRes 422 Proxy/LT, H.264, ¼ vagy ½ méret) | `media.createProxies`, `media.proxyPresets` (`crates/engine/src/proxies.rs`) |
| Átkódolás, export, előrehaladás és megszakítás | `cfa4-export`, `export.queue.add`, `jobs.list`, `jobs.cancel` |
| Időalap és időkód (DF is) | `cfa4_time::Tick`, `time.parse`, `time.readout` |
| Hang-kép lejátszás a Qt-felületen | ADR F8 5. lépés, **IN PROGRESS** (az A/V-szinkron még nincs mérve) |
| Fejetlen (ablak nélküli) futás, hibatűrés | `cfa4-cli serve` (stdin/stdout, `catch_unwind` kérésenként), CLI, MCP |

Ha külön médiaszolgáltatás készülne, második dekóderkészlet, második időalap és második proxylogika
jönne létre, amelyeket a motorral párhuzamosan kellene karbantartani. A CFA4 „egy igazságforrás” és
„minden egy parancs” elvével ez ellentétes.

## Döntés

1. **A CFA4 Media Services a Rust-motor médiaképességeinek kiajánlása, nem új motor.**
   - A dekódolás, a proxy- és bélyegképkészítés, a képkocka-kiszolgálás, az átkódolás és az időalap
     egyetlen helyen él: a meglévő crate-ekben (`cfa4-media`, `cfa4-render`, `cfa4-export`,
     `cfa4-route`, `cfa4-time`).
   - Új dekóder, saját időalap vagy saját proxylogika a BacklotHubban (vagy más fogyasztóban) nem
     készülhet.
2. **A belépési pont a meglévő fejetlen motorszerver, a `cfa4-cli serve`.**
   - Ugyanaz a protokoll (JSON-sorok és bináris farok), amelyet a Qt-felület használ (`docs/qt-gui.md`).
   - A Media Services egy **parancscsalád** benne (munkanév: `media.service.*`), engine-parancsként
     regisztrálva. Így a CLI, az MCP és a vezérlőcsatorna is eléri.
   - A Python-fogyasztó (BacklotHub Qt-felület) a meglévő `EngineClient` mintájára gyermekfolyamatként
     indítja.
3. **Projekt nélküli műveletek.** A motor ma projektközpontú: a proxy például projektelemhez kötődik.
   A BacklotHubnak fájlra vagy verzióra van szüksége, projekt nélkül. Az MS-1 fázisban ezért
   projekttől független parancsok készülnek, a meglévő kódra építve:
   - `probe` (metaadat és ujjlenyomat);
   - `thumbnail`;
   - `frame` (képkocka egy fájlból `Tick`, képkockaszám vagy időkód szerint);
   - `proxy.create` (célútvonal, preset, feladatazonosító);
   - `peaks`;
   - `job.status` és `job.cancel`.
4. **Időalap.** Minden időérték `Tick`. A képkockaszám és az időkód (DF is) oda-vissza konverzióját a
   motor végzi; a fogyasztó nem számol saját időt. Ez a REV-03 (képkockapontos annotáció) alapja.
5. **A Review lejátszása a motor lejátszási útján megy**, ugyanúgy, mint a Qt-szerkesztőben:
   `frame.render` és `audio.render`, legfeljebb egy kérés repülésben, az elavult képkockák eldobva.
   A BacklotHub Review G5 kapujának előfeltétele, hogy az ADR F8 5. lépése elkészüljön, és az
   A/V-szinkron mérése meglegyen.
6. **Szállítás és biztonság.**
   - M1-ben csak csövek (stdin/stdout), mint ma. Hálózati port nincs, rejtett hálózati forgalom nincs.
   - A vezérlőcsatorna (`--control`, hitelesítés nélküli loopback TCP) **nem** lehet a Media Services
     útja.
   - Ha később több folyamatnak kell egy közös példány (M2/M3), az a felhasználó saját Unix-socketjén
     menjen (`$XDG_RUNTIME_DIR`, 0600), vagy hitelesített hálózati úton. Erről akkor külön döntés kell.
7. **Feladatok és események.**
   - A hosszú feladatok (proxy, átkódolás) a motor meglévő jobsorában futnak, azonosítóval,
     állapottal, előrehaladással és megszakítással.
   - Az Event Fabric elkészültéig (EF-R1, CFA4-EVENT-FABRIC-001) a fogyasztó kérdezéssel vagy a
     `serve` eseménysoraival követi őket. Az Event Fabricre nem épít.
   - Az EF-R1 után a jobesemények a Fabricon is megjelennek, a 13.4 eseménytípusaival.
8. **Proxyszabvány.** A review-proxy formátumát és felbontását a BacklotHub V1 CPU-only mérése
   választja ki a meglévő presetek közül, a minimális kliensgépen (`CFA4-MIN`, 4 mag/8 szál vagy 8 mag,
   16 GB). Jelöltek: ProRes 422 Proxy ½, H.264 ½, H.264 ¼. Az Asset Management G2 kapuján rögzül.
9. **Hibakezelés.**
   - Never crash (`AGENTS.md` §0): hiányzó, sérült vagy nem támogatott média kezelhető hibát ad.
     Összeomlás és félbehagyott kimenet nincs; a proxy ideiglenes fájlba készül, és atomi átnevezéssel
     véglegesül.
   - A hibák angol hibakódok angol szöveggel; a felület fordít (BacklotHub I18N-döntés).
10. **A motor fejlesztése nem áll meg, a szerződés véd.** A Rust-motor kiegészítései és javításai
    tovább érkeznek (tulajdonosi megerősítés, 2026-11-02). Ezért:
    - A `media.service.*` parancsokra **szerződéstesztek** kerülnek a CFA4 saját tesztkészletébe
      (`cargo test --workspace`). Így a motoron dolgozó bármely projekt a saját PR-jában látja, ha a
      BacklotHub által használt viselkedést eltöri.
    - A tesztek a kötött viselkedést ellenőrzik: paraméterek, eredménymezők, hibakódok, `Tick`-
      pontosság. A motor belsejét nem kötik.
    - Nem visszafelé kompatibilis változásnál a `serve` protokollverzióját emelni kell. A BacklotHub
      induláskor ellenőrzi a verziót, és eltérés esetén kezelhető hibát ad.
    - Nagyobb dekódolási vagy lejátszási változás után a V1 CPU-only mérést újra le kell futtatni a
      `CFA4-MIN` gépen. A REV-01…05 nem romolhat.
11. **Licenc.** Új külső függőség nem kell; a meglévő médiahátterek szabályai (ADR 0002, 0004, 0005)
    érvényesek.

## Fázisok és kapuk

| Fázis | Tartalom | Kell, mielőtt … |
|---|---|---|
| MS-0 | Ez az ADR | a BacklotHub Asset Management (8. lépcső) G2 kapuja (V0) |
| MS-1 | Projekt nélküli parancsok (3. pont), jobállapot, hibakódok, mutációs fuzz a bemenetekre, szerződéstesztek (10. pont); CPU-only mérés (V1) | a BacklotHub Asset Management (8.) megvalósítása |
| MS-2 | Képkocka-kiszolgálás verziókhoz; két verzió szinkron léptetése (REV-07); `Tick`↔képkocka↔időkód property-tesztek minden fps-en (REV-03) | a Review és Approval (13.) megvalósítása |
| MS-3 | Lejátszás a Review felületén az ADR F8 5. lépésére építve; A/V-szinkron mérés (REV-05) | a Review G5 kapuja (V2) |
| MS-4 | Jobesemények az Event Fabricon | az EF-R1 után, a Notification Centerrel (12.) |

## Következmények

- Egy dekóder, egy időalap, egy proxylogika: amit a motor javít (új kodek, hardveres út), azt a
  BacklotHub is megkapja.
- A `cfa4-cli serve` protokollja ezzel nyilvános szerződés lesz a BacklotHub felé. Változásához
  verzióemelés kell (`hello` → `protocol`).
- A motor felelőssége bővül a projekt nélküli műveletekkel. Ezek a meglévő kódot hívják, nem
  másolják.
- A BacklotHub Review csak akkor indulhat, ha az ADR F8 5. lépése kész. Ez a 13. lépcső előfeltétele.

## Kapcsolódó dokumentumok

- `docs/qt-gui.md` – a `serve` protokoll; `docs/architecture.md` 5–7. fejezet
- ADR 0004 (`cfa4-route`), ADR 0005 (médiahátterek), ADR F8 (Qt6/Rust motor)
- `docs/plans/CFA4-EVENT-FABRIC-001-1.0.md` (PR #509), ADR 0006 BacklotHub PostgreSQL (PR #511)
- A BacklotHub-elemzések (NFR-javaslat 4. fejezet: V0–V2, REV-01…08) a projekt munkaanyagai.
