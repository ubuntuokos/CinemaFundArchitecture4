# CFA4-DCPOMATIC-001 – DCP-o-matic bekötése a CFA4-be: megvalósítási terv

**Dátum:** 2026-11-13  
**Státusz:** **tervezet, a tulajdonos jóváhagyására vár.** Amíg nincs jóváhagyva, ebből a tervből nem indul munka.  
**Kapcsolódik:** [ADR 0011](../adr/ADR-0011-dcpomatic-external-mastering.md) (a bekötés módjáról, javasolt), [DCP-00 audit](../audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md), [DCP-VALIDATION](DCP-VALIDATION-2026-10-27.md), a `cfa4-dcp-inspect` minta

> **Előzmény, őszintén.** A tulajdonos 2026-11-13-án ennyit kért: „A DCP-o-matic beépítése a CFA4-be”. Terv és jóváhagyás nélkül elkészült egy megvalósítás (PR #1173, piszkozat). Ez a CLAUDE.md 3. pontjával ellentétes. A PR addig nem olvasztható, amíg ez a terv nincs jóváhagyva. Utána vagy a jóváhagyott tervhez igazítjuk, vagy lezárjuk (D0 döntés). A PR-ben lévő kódot ez a terv nem tekinti késznek, mert valódi DCP-o-matic-kal még nem futott (lásd 1. fázis).

## 1. Cél

A CFA4-ből olyan DCP-k is készüljenek, amelyeket a saját DCP-export ma nem tud:

- Interop szabvány;
- több tartalomfájl egy kompozícióban;
- 3D;
- feliratok;
- titkosított DCP és KDM.

Ezeket a DCP-o-matic (GPL-2.0-or-later, C++) végezze, a CFA4 pedig vezérelje. A CFA4 saját DCP-exportja marad az alapértelmezett.

**Kizárólag a CFA4 része** (a tulajdonos döntése, 2026-11-13): a DCP-o-matic a CFA4-en kívül nem indítható el, csak a CFA4 egyik funkciójaként létezik. A CFA4 csomag a DCP-o-matic parancssori eszközeinek saját, rögzített változatú példányát viszi magával (D9).

**Nem cél:** a DCP-o-matic grafikus felületének kiváltása vagy szállítása, KDM-ek e-mailes küldése, a #933 döntés megváltoztatása (a CFA4 maga titkosított DCP-t csak lejátszik és importál).

## 2. Kiinduló állapot (a `main`-en)

- `crates/export/src/dcp_out.rs`: saját SMPTE OV DCP-export (JPEG 2000 2K/4K, 5.1/16 csatorna PCM, IAB, egy reel). Hiányzik: Interop, több tartalom, 3D, felirat, titkosítás (`docs/file-format-parity.md`).
- `cfa4-dcp-inspect`: a külső folyamatként futó C++ eszköz bekötésének meglévő mintája.
- `dcp.loadKdm`, `cfa4-dcp-validate`: KDM betöltése és DCP-ellenőrzés a CFA4-ben.
- `file.exportMedia`: mesterfájl exportja (pl. ProRes 4444, TIFF-sorozat + WAV).
- DCP-o-matic: nincs az Ubuntu tárolóiban, a projekt maga ad ki `.deb`-et (dcpomatic.com).

## 3. A tulajdonos döntései (kitöltendő)

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| D0 | Mi legyen a már elkészült PR #1173-mal? | Maradjon piszkozat, és fázisonként igazítsuk ehhez a tervhez. Ha egy fázist elvetsz, a hozzá tartozó kódot kivesszük. | |
| D1 | Kell-e egyáltalán a DCP-o-matic, vagy inkább a saját DCP-exportot bővítsük (Interop, 3D, felirat, titkosítás)? | DCP-o-matic most, a saját export bővítése később, lépésenként. A DCP-o-matic évek óta mozikban használt. | |
| D2 | Bekötés módja | Külön folyamat, Rust-felület mögött (ADR 0011). Linkelés elvetve, mert a C++ összeomlás a CFA4-et is vinné. | **A CFA4-gyel szállított saját példány, külön folyamatként** (István, 2026-11-13). |
| D3 | Honnan kapja a DCP-o-matic a képet és a hangot? | **a)** egy kész videofájlból. **b)** az idővonalból: a CFA4 maga készíti el a köztes mestert, majd átadja (sok lemezhelyet kér). | **Mindkettő** (István, 2026-11-13): a Fájl ▸ Export az éppen feldolgozott videót (az aktuális szekvenciát) adja át, b) szerint; az Eszközök ▸ Videoeszközök ▸ Publikálás a gépen található bármely videofájlt, a) szerint. |
| D4 | Titkosított DCP és KDM a DCP-o-matic-kal | Igen (2026-11-13-án igent mondtál rá a szálban). A kulcsok a DCP-o-matic filmkönyvtárában maradnak, a CFA4 nem olvassa és nem tárolja őket. | |
| D5 | Feliratok (SRT, SubRip, DCP XML) átadása | 3. fázis, a Subtitle Studio terv kimenetére építve. | |
| D6 | Felület | A DCP-o-matic a CFA4 designját követi (lásd 4. fázis). Saját ablaka (wxWidgets) soha nem nyílik meg, és nincs külön „DCP-o-matic” stílusú ablak. Javaslat a módjára: a DCP-o-matic a meglévő Export ablak DCP formátumának egy választható motorja („CFA4” / „DCP-o-matic”), a futás az export sorában (Queue) látszik, a KDM-készítés pedig a Load KDM…-hoz hasonló párbeszédablak. | **A CFA4 designját kövesse; menü: Eszközök ▸ Videoeszközök ▸ Publikálás és Fájl ▸ Export; csak Qt6, egui nem** (István, 2026-11-13). A javasolt mód többi része még jóváhagyásra vár. |
| D7 | Hol legyen a DCP-o-matic aláíró lánca és beállítása? | A CFA4 saját könyvtárában (`~/.config/cfa4/dcpomatic`), hogy a felhasználó DCP-o-matic-beállításai ne változzanak. | |
| D8 | DCP-o-matic változat | Egy rögzített 2.18.x változat, amelyet a CFA4 maga fordít és szállít. A pontos szám az 1. fázis után dől el, frissítés csak külön PR-ben. | |
| D9 | Csomagolás | A CFA4 `.deb` a DCP-o-matic parancssori eszközeit (`dcpomatic2_create`, `dcpomatic2_cli`, `dcpomatic2_kdm_cli`) és a hozzájuk kellő könyvtárakat egy privát mappában viszi (`/usr/lib/cfa4/dcpomatic/`). Ez nincs a PATH-on, nincs hozzá menüpont, `.desktop` fájl vagy man-oldal, és a DCP-o-matic grafikus felülete le sem fordul. A CFA4 csak ezt a példányt használja, a gépre esetleg telepített DCP-o-maticot nem. A GPL miatt a pontos forrás (és a CFA4 javításai) a kiadással együtt elérhető, a licencszöveg a csomagban van. | **Szállított saját példány** (István, 2026-11-13); a részletek jóváhagyásra várnak. |
| D10 | CI | Egy job lefordítja a rögzített DCP-o-matic-változatot (gyorsítótárral), és valódi DCP-t készít vele egy rövid tesztmesterből. Kezdetben nem blokkoló. | |

## 4. Fázisok

Minden fázis külön PR, saját feladatazonosítóval (`DCPOM-1` …). A következő fázis csak az előző olvasztása után indul.

### 1. fázis – Valódi DCP-o-matic felmérése (DCPOM-1)

Ez a fázis kódot nem ír a termékbe.

- Fordítsd le a DCP-o-matic legújabb 2.18.x forrását Ubuntu 26.04-en, **csak a parancssori eszközöket**, grafikus felület nélkül. Rögzítsd a függőségeit (libdcp, libsub, asdcplib, OpenJPEG, a DCP-o-matic saját FFmpeg-igénye), a fordítási időt és a kész méretet.
- Döntsd el, hogy a függőségek az Ubuntu csomagjaiból jöhetnek-e, vagy velük együtt kell fordítani.
- Rögzítsd a `dcpomatic2_create`, `dcpomatic2_cli` és `dcpomatic2_kdm_cli` valódi kapcsolóit, kimenetét (haladás, hibák) és kilépési kódjait egy auditfájlban (`docs/audit/DCPOM-01-…`).
- Készíts valódi DCP-t egy 10 mp-es tesztmesterből SMPTE és Interop változatban, és ellenőrizd a CFA4 `cfa4-dcp-validate` eszközével.
- Mérd meg a futási időt és a lemezigényt a referenciagépen (`docs/development/reference-machine.md`).

**Elfogadás:** az audit leírja a kapcsolókat, a függőségeket, a fordítás módját és a csomag méretét. A D8 és a D9 részletei ebből dőlnek el.

### 2. fázis – DCP készítése mesterfájlból (DCPOM-2)

- `crates/dcpomatic` (L3): a három eszköz külön folyamatként fut, argumentumtömbbel, shell nélkül, saját folyamatcsoportban, kiürített környezetben, privát `HOME`/`TMPDIR`-rel. Van megszakítás (SIGINT, majd SIGKILL), időtúllépés és haladásjelzés.
- Engine-parancsok: `dcpomatic.status` (telepítve van-e, milyen változat) és `dcpomatic.makeDcp` (háttérfeladat, `jobs.list` / `jobs.cancel`).
- Paraméterek: név, tartalomtípus, konténer, 2K/4K, SMPTE/Interop, képkockaszám, csatornaszám, bitráta, 3D (bal/jobb szem), mono hangfájlok csatornához rendelése.
- A kérést indulás előtt ellenőrzi; hibás kérésre nem indul el folyamat.

**Tesztek:** hamis eszközökkel (shell-szkriptekkel) az argumentumok, a környezet, a haladás, a hibák, a megszakítás és az időtúllépés. A D10 CI-jobban valódi DCP-o-matic-kal is: mester → DCP → `cfa4-dcp-validate` hiba nélkül.

**Elfogadás:** a CFA4 csak a saját mappájában lévő példányt találja meg, a PATH-on lévő vagy a felhasználó által telepített DCP-o-maticot nem (fejlesztéshez egy környezeti változó adhat meg buildmappát). Ha a példány hiányzik vagy sérült, a CFA4 többi része változatlanul működik, a `status` „nem elérhető”. Vele a CI-ben készült DCP átmegy a CFA4 ellenőrzésén SMPTE és Interop szerint is.

### 3. fázis – Titkosítás, KDM és feliratok (DCPOM-3)

- `dcpomatic.makeDcp` `encrypt` paraméterrel. A filmkönyvtárat meg kell őrizni, mert a kulcsok benne vannak; ezt a válasz és a felület is jelzi.
- `dcpomatic.makeKdm`: projektortanúsítvány, érvényesség UTC-ben, formuláció, trusted device-ok, a törvényszéki jelölés kikapcsolása.
- Feliratfájlok átadása (D5).

**Elfogadás:** a CI-ben titkosított DCP és hozzá egy tesztanúsítványra készült KDM készül, és a CFA4 `dcp.loadKdm`-mel, a teszt magánkulcsával lejátssza.

### 4. fázis – Felület a CFA4 designjával, és közvetlen út az idővonalból (DCPOM-4)

A tulajdonos döntése (D6): a DCP-o-matic **a CFA4 designját követi**. A felhasználó nem DCP-o-matic-ot lát, hanem a CFA4 egy újabb exportlehetőségét.

- **Nincs külön DCP-o-matic-ablak.** A DCP-o-matic saját grafikus felülete soha nem indul el. A CFA4 nem másolja a DCP-o-matic elrendezését (tartalomlista, fülek, ikonok) sem.
- **Export ablak** (`panels/export.py`, `docs/qt-gui.md`): a DCP formátumnál új választó jelenik meg: „Motor: CFA4 / DCP-o-matic”. A DCP-o-matic csak akkor választható, ha a `dcpomatic.status` elérhetőnek mutatja. Ha nem, a választó letiltott, és a tippje megmondja, miért.
  - A DCP-o-matic-hoz tartozó beállítások (Interop, 3D, csatornakiosztás, titkosítás) ugyanazokkal a sor-vezérlőkkel (`export_rows.py`) jelennek meg, mint a többi exportbeállítás. Csak azok a sorok látszanak, amelyeket a választott motor tud.
  - Az Összegzés (`export_summary.py`) írja ki, hogy a DCP-t a DCP-o-matic készíti, és mekkora a becsült lemezigény.
  - A futás az export sorában (Queue) látszik, ugyanazzal a haladásjelzéssel és megszakítással, mint a CFA4 saját exportjainál.
- **Menü helye** (a tulajdonos döntése, 2026-11-13): **Eszközök ▸ Videoeszközök ▸ Publikálás** (`tools_menu.py`, `menu.Tools.Video.Publish`). Két menüpont kerül ide:
  - „DCP készítése DCP-o-matic-kal…” (`tools.video.publish.dcpomatic`): **a gépen található bármely videofájlból** készít DCP-t (D3 a). Fájlválasztóval indul, majd ugyanazokat a DCP-sorokat mutatja, mint az Export ablak, a CFA4 designjával;
  - „KDM készítése…” (`tools.video.publish.createKdm`): a KDM-párbeszédablak.
  - Az **Export menüben** (Fájl ▸ Export) is megjelenik egy „DCP DCP-o-matic-kal…” menüpont (a tulajdonos kérése, 2026-11-13). Ez **az éppen feldolgozott videót, vagyis az aktuális szekvenciát** adja át (D3 b): megnyitja az Export ablakot DCP formátummal és DCP-o-matic motorral. A „KDM készítése…” is itt van.
  - Mindkét helyen letiltottak, ha a DCP-o-matic nincs telepítve, és a tippjük megmondja, miért.
- **Csak a Qt6 felület készül** (a tulajdonos döntése, 2026-11-13). Az egui szerkesztő (`crates/ui-egui`) nem kap DCP-o-matic-felületet. A `docs/qt-parity.md` ezt nem számítja hiánynak a Qt oldalon.
- **KDM-készítés:** párbeszédablak a meglévő Load KDM… (`panels/kdm_dialog.py`) mintájára, ugyanazzal az elrendezéssel és viselkedéssel.
- **Közös szabályok** (`docs/qt-gui.md`):
  - a `theme.py` design-tokenjei (Darkest, Dark, Light), saját színek és stíluslap nélkül;
  - a panelek csak az engine válaszát mutatják, saját logikájuk nincs;
  - minden vezérlőnek automation id-ja van;
  - minden felirat a fordításon (`i18n`) megy át, magyarul is;
  - dokkok esetén a `main_window.py` dokkszabályai.
- Fájl ▸ Export útja (D3 b): az Export ablak az aktuális szekvenciából köztes mestert készít egy ideiglenes mappába, átadja a DCP-o-matic-nak, majd törli. Előtte kiírja a becsült lemezigényt, és nem indul el, ha a hely nem elég.

**Elfogadás:**
- A `--smoke --screenshot` képen az Export ablak DCP-o-matic motorral mind a három témában a CFA4 többi exportjával egyező kinézetű. Összevetés a CFA4 Qt Export ablakának többi formátumával és a `plan/premiere/screenshots/` képeivel.
- A Qt-teszt a valódi engine-en át hamis eszközös DCP-készítést futtat a sorból.
### 5. fázis – Csomagolás és dokumentáció (DCPOM-5)

- A rögzített DCP-o-matic fordítása a `packaging/linux/` folyamatában, a privát mappába (D9). Nincs PATH-bejegyzés, `.desktop` fájl, ikon vagy man-oldal.
- Licenc: a DCP-o-matic és függőségei licencszövege a csomagban, harmadik féltől származó komponensként bejegyezve a `docs/licensing.md`-be és a `NOTICE`-ba. A pontos forrás a kiadás mellett.
- Csomagteszt: telepítés után a `dcpomatic2_cli` nem érhető el a PATH-on, a menüben sincs DCP-o-matic, a CFA4-ből viszont elkészül egy DCP.
- `docs/development/dcpomatic.md`, `docs/file-format-parity.md` és `docs/gaps.md` frissítése a mért eredménnyel.

## 5. Kockázatok

- **A DCP-o-matic parancssora változhat.** Kezelés: a változatellenőrzés és a valódi eszközzel futó CI-job, minden új főváltozatnál újra.
- **Fordítás és méret.** A DCP-o-matic saját FFmpeg-et és könyvtárakat igényelhet, ami a csomagot nagyobbá és a CI-t lassabbá teszi. Kezelés: az 1. fázis méri, a CI gyorsítótárat használ.
- **Biztonsági frissítések.** A szállított példányt a CFA4-nek kell frissítenie. Kezelés: a rögzített változat a kiadási ellenőrzőlistán szerepel.
- **Lemezigény.** Egy játékfilm köztes mestere és DCP-je együtt több száz GB. Kezelés: becslés és figyelmeztetés indulás előtt.
- **Kulcskezelés.** A tartalomkulcsok elvesztése után nem készíthető több KDM. Kezelés: a felület figyelmeztet a filmkönyvtár megőrzésére. A CFA4 nem készít róla másolatot.
- **Hálózat.** A DCP-o-matic más gépekre is küldhet kódolásra képkockát. Kezelés: mindig `--no-remote`.

## 6. Befejezés feltétele

A `docs/agents.md` 7. pontja szerint, és ezen felül: valódi DCP-o-matic-kal készült DCP átmegy a CFA4 ellenőrzésén, és ez CI-ben ismételhető.
