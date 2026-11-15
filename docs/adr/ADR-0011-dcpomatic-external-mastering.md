# ADR 0011 – DCP-o-matic bekötése külső DCP-mesterelő háttérként

**Dátum:** 2026-11-13 · **Állapot:** javasolt, **a tulajdonos jóváhagyására vár** (a tulajdonos,
`ubuntuokos` 2026-11-13-án kérte a DCP-o-matic beépítését, és a projektszálban ugyanaznap igent
mondott a titkosított DCP-re, a KDM-re és a Qt-panelre; magát az ADR-t még nem hagyta jóvá) ·
**Kapcsolódik:** [ADR 0001](0001-platform-ffi.md) (FFI, 2026-11-07-i kiegészítés: nem Rust
komponensek), [ADR 0005](ADR-0005-media-backends.md) (kiegészítő médiahátterek),
[DCP-00 audit](../audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md), a `cfa4-dcp-inspect` minta
([README](../../crates/dcp-inspect/README.md))

## Háttér

A CFA4-nek van saját DCP-exportja (`crates/export/src/dcp_out.rs`, #115 és folytatásai): SMPTE OV,
JPEG 2000 X'Y'Z' 2K / 4K, 5.1 / 16 csatornás PCM, IAB, egy reel, RDD 52. Ami hiányzik belőle
(`docs/file-format-parity.md`, DCP-sor): Interop, több tartalomfájl egy kompozícióban, 3D,
feliratok, titkosítás.

A [DCP-o-matic](https://dcpomatic.com) (Carl Hetherington és társai) a legelterjedtebb nyílt
forrású DCP-készítő. C++, a libdcp / libsub / asdcplib / OpenJPEG könyvtárakra épül.

**Licenc:** a forrásfájlok fejléce „GNU General Public License … either version 2 of the License,
or (at your option) any later version” (ellenőrizve: `src/tools/dcpomatic_cli.cc`,
`src/lib/create_cli.cc`, `cth103/dcpomatic` `main` @ `27fa194`, 2026-11-12). A
GPL-2.0-or-later kompatibilis a CFA4 GPL-3.0-or-later licencével (`docs/licensing.md` §3), tehát
a licenc egyik bekötési módot sem zárja ki.

## Megfontolt lehetőségek

| Lehetőség | Értékelés |
|---|---|
| A libdcp / DCP-o-matic könyvtár linkelése a CFA4-folyamatba (FFI) | Licenc szerint megengedett, de C++ kódot hozna az editorfolyamatba: `unsafe` FFI, egy összeomlás a CFA4-et is leállítaná (AGENTS.md §0), és a C++ API nem stabil. **Elvetve.** |
| A DCP-o-matic Rustra portolása | Több százezer sor; aránytalanul hosszú (AGENTS.md „Port or keep the original language”). **Most nem.** |
| **A DCP-o-matic parancssori eszközeinek futtatása külön folyamatként** | Az AGENTS.md „Port or keep…” és „Non-Rust components” szabályának elsődleges formája; a `cfa4-dcp-inspect` már így köti be a dcp_inspect motort. Egy összeomlás csak hiba, a CFA4 nem lassul. **Elfogadva.** |

## Döntés

1. **Külön folyamat, Rust-felület mögött.** Új L3 crate: `cfa4-dcpomatic` (`crates/dcpomatic`).
   Két lépés: `dcpomatic2_create --output <film> …` megírja a DCP-o-matic filmkönyvtárát a
   tartalomfájlokból, majd `dcpomatic2_cli --no-remote make-dcp <film>` elkészíti benne a DCP-t.
   Mindkettő argumentumtömbbel (shell nélkül), saját folyamatcsoportban, kiürített környezetben
   (csak `PATH`, `LD_LIBRARY_PATH`, locale), privát `HOME` / `TMPDIR` / munkakönyvtárral fut;
   a kimenet fájlba megy. Megszakítás: `SIGINT` a csoportnak, 5 s után `SIGKILL`; időtúllépés
   (alapból 48 óra): `SIGKILL`. A konzolos haladás (`NN%`) a hívóhoz jut.
2. **A DCP-o-matic kizárólag a CFA4 része** (a [CFA4-DCPOMATIC-001 v1.0](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md) terv
   D2/D9 döntése, a tulajdonos 2026-11-13-án hagyta jóvá; ez váltja ki ennek a pontnak a korábbi, „a felhasználó telepíti”
   változatát). A CFA4 `.deb` a DCP-o-matic parancssori eszközeinek egy rögzített változatát (**2.18.51**, a
   [DCPOM-01 audit](../audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md) szerint) és saját könyvtárait egy privát
   mappában viszi (`/usr/lib/cfa4/dcpomatic/`). Ez nincs a PATH-on, és nincs hozzá menüpont, `.desktop` fájl
   vagy grafikus felület. A CFA4 csak ezt a példányt használja. Ha hiányzik vagy sérült, a háttér „nem
   elérhető”, és a CFA4 többi része változatlanul működik. A futás továbbra is külön folyamat (1. pont).
3. **Parancsok** (minden kliens, a Qt GUI is a `cfa4-cli serve`-en át): `dcpomatic.status`
   (telepítve van-e, hol, milyen változat) és `dcpomatic.makeDcp` (tartalomfájlok → DCP
   háttérfeladatként, `jobs.list` / `jobs.cancel`, vagy `wait`-tel azonnal). A CFA4 saját
   DCP-exportja (`format: "dcp"`) marad az alapértelmezett; a DCP-o-matic a kiegészítő út
   (Interop, több tartalom, 3D, a DCP-o-matic saját skálázása, színkonverziója és
   hangkiosztása). A szokásos munkafolyamat: mesterfájl exportja a CFA4-ből (pl. ProRes 4444 vagy
   TIFF-sorozat + WAV), majd `dcpomatic.makeDcp` erre a fájlra.
4. **A tulajdonos válaszai a projektszálban (2026-11-13, „igen, igen”; az ADR jóváhagyása ettől külön):**
   - **Titkosított DCP és KDM-készítés: igen.** Ez a 2026-11-12-i döntést (#933: a CFA4 maga a
     titkosított DCP-t csak lejátssza és importálja) nem változtatja meg, hanem kiegészíti: a
     titkosítást és a KDM-et a DCP-o-matic készíti, külön folyamatban. A `dcpomatic.makeDcp`
     `encrypt` paramétere `--encrypt`-tel futtat (alapból `--no-encrypt`). A tartalomkulcsok a
     filmkönyvtár `metadata.xml`-jében maradnak, ezért azt meg kell őrizni. A
     `dcpomatic.makeKdm` ebből a könyvtárból a `dcpomatic2_kdm_cli create`-tel KDM-et készít egy
     projektor- (media block-) tanúsítványra. Az érvényesség UTC-ben értendő, a formuláció
     választható, és a törvényszéki jelölés kikapcsolható. A CFA4 a kulcsokat nem olvassa és nem
     tárolja, a KDM-eket nem küldi el (a DCP-o-matic e-mail funkciója nincs bekötve). A kész KDM-et
     a CFA4 saját `dcp.loadKdm`-je le tudja játszani.
   - **Qt GUI: igen**, a [terv](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md) D6 döntése szerint: a CFA4
     designját követi, csak Qt6-ban készül, és nincs külön DCP-o-matic-ablak. A DCP-o-matic az Export ablak
     DCP-formátumának egyik motorja. Két helyről érhető el: Eszközök ▸ Videoeszközök ▸ Publikálás (bármely
     videofájl) és Fájl ▸ Export (az aktuális szekvencia). A KDM-készítés is ezekből a menükből indul.
   - **Csomagolás:** lásd a 2. pontot (szállított saját példány). A korábbi `Suggests: dcpomatic` javaslat elesett.
5. **A DCP-o-matic beállításai** alapból a CFA4 saját könyvtárában vannak
   (`${XDG_CONFIG_HOME:-~/.config}/cfa4/dcpomatic`; a `--config` adja át, a `--config` opció
   nélküli `dcpomatic2_kdm_cli`-nek pedig egy privát `XDG_CONFIG_HOME/dcpomatic2` hivatkozás). Így
   az aláíró tanúsítványlánc futásról futásra ugyanaz, és a felhasználó saját
   DCP-o-matic-beállításai érintetlenek. A `configDir` paraméter más könyvtárat ad meg.
6. **Hálózat:** a `--no-remote` miatt a DCP-o-matic nem küld képkockát a beállításaiban
   esetleg felsorolt kódolószerverekre.

## Következmények

- A CFA4 kódja GPL-3.0-or-later marad. A DCP-o-matic kódjából semmi sem kerül át a CFA4 forrásába. A
  szállított bináris példány viszont harmadik féltől származó komponens: a licencszövegei a csomagba, a
  bejegyzése a `docs/licensing.md`-be és a `NOTICE`-ba kerül, és a pontos forrás a kiadás mellett elérhető
  (terv, 5. fázis).
- A Rust oldalon nincs `unsafe` és nincs C++; a rétegszabály (`cargo xtask layers`) a
  `dcpomatic` crate-et L3-ba sorolja, az `engine` (L4) használja.
- Tesztek: a crate hamis eszközökkel (shell-szkriptek) ellenőrzi az argumentumokat, a privát
  környezetet, a haladást, a hibaüzeneteket, a megszakítást (a háttérben indított gyermek is
  leáll) és az időtúllépést. Valódi DCP-o-matic-kal futó körbejárási teszt (DCP-o-matic → CFA4
  `cfa4-dcp-validate`) a következő lépés, ahol a DCP-o-matic telepíthető.
- A DCP-o-matic parancssora változhat. A CFA4 egy rögzített változatot szállít (2.18.51), és minden
  frissítéskor a DCPOM-01 audit lépéseit és a valódi eszközzel futó CI-jobot újra le kell futtatni.
