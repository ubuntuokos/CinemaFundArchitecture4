# DCPOM-01 – A DCP-o-matic felmérése (valódi fordítás és futtatás)

**Dátum:** 2026-11-13  
**Terv:** [CFA4-DCPOMATIC-001 v1.0](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md), 1. fázis (DCPOM-1)  
**Státusz:** felmérés kész. A D8 (változat) és a D9 (csomagolás) részleteihez javaslatot ad (7. pont).

A fázis termékkódot nem változtatott. Minden szám mérés ebben a környezetben, kivéve ahol becslésnek jelölöm.

## 0. Környezet

- **Gép:** a felhőben futó fejlesztői konténer, **Ubuntu 24.04.5 LTS**, 4 mag. **Nem** a referenciagép (`docs/development/reference-machine.md`), és **nem** a célplatform (Ubuntu 26.04). Az időmérések ezért csak tájékoztató jellegűek. A referenciagépen és 26.04-en a 2. fázis CI-jobja ismétli őket.
- **Forrás:** `git.carlh.net` (a DCP-o-matic saját git-szervere). A `github.com/cth103` tükör a függőségek egy részénél hitelesítést kért.

| Komponens | Változat (címke) | Commit | Licenc (a forrásfejlécből) |
|---|---|---|---|
| DCP-o-matic | v2.18.51 (a legújabb 2.18.x) | `920fb9b` | GPL-2.0-or-later |
| libdcp | v1.10.64 | `891facc` | GPL-2.0-or-later |
| libsub | v1.6.65 | `60c9a08` | GPL-2.0-or-later |
| libcxml | v0.17.17 | `efde22c` | GPL-2.0-or-later |
| asdcplib (DCP-o-matic változat) | v1.0.10 | `98aa300` | BSD-stílusú (`COPYING`, John Hurst) |
| leqm-nrt | – | `3913040` | GPL-3.0-or-later |
| libttf | v0.0.7 | `6d830c6` | GPL-2.0-or-later |
| OpenSSL (DCP-o-matic javítással) | 3.2.0 + 1 javítás | `5429836` | Apache-2.0 |
| fast_float | v8.0.2 (fejlécek) | – | Apache-2.0 / MIT / BSL-1.0 |

Mindegyik licenc összeegyeztethető a CFA4 GPL-3.0-or-later licencével (`docs/licensing.md` §3). A rendszerből linkelt könyvtárak (FFmpeg 6.1, Boost, libxml++, xmlsec, xerces-c stb.) az Ubuntu csomagjai.

## 1. Fordítás

- **Csak parancssori eszközök:** a DCP-o-matic `--disable-gui --disable-tests` kapcsolóval fordul. A wxWidgets nem kell, grafikus felület nem készül.
- **Sorrend:** asdcplib → libcxml → libdcp → libsub → leqm-nrt → libttf → DCP-o-matic. Mindegyik waf-fal fordul.
- **Idő:** a teljes lánc nagyjából 12 perc (21:32–21:44), ebből a DCP-o-matic kb. 5 perc. Ehhez jön a javított OpenSSL kb. 2 perce.
- **Méret:** az eszközök 10 MB, a saját könyvtárak 17 MB, a javított OpenSSL 7,5 MB, összesen **kb. 35 MB**. A rendszerkönyvtárak ebben nincsenek benne.

A fordítás közben kiderült akadályok, amelyeket a csomagolásnak (5. fázis) kezelnie kell:

1. **A `fast_float` az Ubuntu 24.04-ben túl régi.** A libdcp egész számokat is ezzel olvas, ehhez 6.x vagy újabb kell. Megoldás: a fejlécek a fordítási előtagba kerültek, és ott elsőbbséget kaptak. Ubuntu 26.04-en újra kell ellenőrizni.
2. **A libdcp `dcpdumpimage` eszköze ImageMagick-fejléceket keres**, és ezek hiányában nem fordul. Nem kell, ezért `--disable-dumpimage`.
3. **A KDM-hez és az első induláshoz egy javított OpenSSL-binárist kell mellékelni.** Neve `dcpomatic2_openssl`, a DCP-o-matic eszközeivel azonos mappában. Ezzel készíti az aláíró és a visszafejtő tanúsítványláncot. Nélküle már a `dcpomatic2_kdm_cli --help` is ezzel a hibával áll le: `error 127 in "dcpomatic2_openssl"`. Ez az OpenSSL 3.2.0 egyetlen javítással: a tanúsítványok `notBefore` ideje 7 nappal korábbi.
4. **A változatszám a forrás gitállapotából jön.** Ebben a felmérésben a sekély klónozás miatt a DCP-k `Issuer`/`Creator` mezőjébe ez került: `DCP-o-matic 2.18.51 fatal: amb`. A csomagolásnak teljes, címkézett állapotból kell fordítania, és a CI-nek ellenőriznie kell a `--version` kimenetét.
5. **A telepítési mappa `lib64`.** A pkg-config- és a futtatási útvonalaknak ezt is tartalmazniuk kell.

## 2. Valódi parancssor (2.18.51)

A teljes súgók ennek az auditnak a mellékletei a 6. pontban. A tervhez és a PR #1173-ban lévő vázlatkódhoz képest ezek az eltérések számítanak:

| Téma | Valóság | Következmény |
|---|---|---|
| `--container-ratio` | Elfogadott értékek: 119, 133, 137, 138, 166, 178, 185, 239. **Ismeretlen értéket (például 190) hiba nélkül elfogad** (kilépési kód 0). | A vázlatkód a „Full” konténerhez `190`-et ad át, ami **hibás**. A CFA4-nek magának kell ellenőriznie az értéket, mert a DCP-o-matic nem teszi. |
| `--output` létező mappába | Hiba nélkül lefut (0). | A vázlatkód helyesen előre megtagadja. Ez marad. |
| Hiányzó tartalom | Kilépési kód 1, az ok a stderr-en (`boost::filesystem::canonical: No such file …`). | Az utolsó stderr-sor az üzenet. |
| `make-dcp` hiányzó filmre | Kilépési kód 1 (`error reading film …`). | Ugyanígy. |
| Haladás | Stdout, ANSI-sorvezérlővel: `Transcoding CFA4 Test; Encoding: 89%; … 221/240 frames; 9.0 fps`. | A vázlatkód az utolsó `NN%`-ot olvassa, ez a valódi formátummal is működik. |
| `--config <dir>` | A beállítás egy változatszámos almappába kerül (`<dir>/2.18/config.xml`). Itt jön létre a visszafejtő kulcs és tanúsítvány is. | A D7 mappája így `…/cfa4/dcpomatic/2.18/`. Változatfrissítéskor a lánc átvitelét kezelni kell. |
| KDM érvényessége | A súgó szerint „a mozi helyi idejében”. Mozibeállítás nélkül (`-C` tanúsítvánnyal) mérve UTC: `-f "2026-11-20 10:00:00"` `TZ=Europe/Budapest` mellett is `2026-11-20T10:00:00+00:00` lett. | A CFA4 mozibeállítást nem használ, ezért az UTC-állítás a CFA4 útján igaz. A felületnek UTC-t kell kiírnia. |
| KDM titkosítatlan filmre | Kilépési kód 1: `Cannot make a KDM as this project is not encrypted.` | A vázlatkód ezt előre megtagadja. Ez marad. |
| Formulációk | `modified-transitional-1`, `multiple-modified-transitional-1`, `dci-any`, `dci-specific`. | Egyezik a vázlatkóddal. |
| `--disable-forensic-marking-audio` | Opcionális értéke egy csatornaszám. | Egyezik. |
| `dcpomatic2_kdm_cli` | Nincs `--config` opciója, az `XDG_CONFIG_HOME/dcpomatic2` mappát olvassa. | A vázlatkód hivatkozásos megoldása így működött. |

## 3. Valódi DCP-k

Bemenet: 10 mp-es tesztmester, 1998×1080, 24 kép/s, ProRes 4444, 5.1 PCM (ffmpeg-gel generálva, nincs a repóban).

| Eset | `create` | `make-dcp` | Méret | libdcp `dcpverify` |
|---|---|---|---|---|
| SMPTE, 2K, 1.85, 6 csatorna | 1 s, rc 0 | 31 s, rc 0 (kb. 9 kép/s, 4 szálon) | 146 MB | „DCP verified OK.” |
| Interop, ugyanígy | rc 0 | 31 s, rc 0 | 146 MB | OK, de egy „Bv2.1 error”: a DCP nem SMPTE (Interopnál ez várható) |
| SMPTE, titkosítva (`--encrypt`) | rc 0 | rc 0 | – | – |
| KDM a titkosított filmre | `dcpomatic2_kdm_cli … create`: rc 0, 1 KDM-fájl | | | |

**A CFA4 saját ellenőrzője** (`cfa4-dcp-validate`, `NativeValidator`):

| Csomag | Profil | Eredmény |
|---|---|---|
| SMPTE | `full-integrity` | **Pass** (15 pass) |
| SMPTE | `smpte-cinema` | **Fail**: `DCP.J2K.001` – „multiple component transform” a képfolyamban. 4 szabály még nincs megírva (Unchecked). |
| Interop | `full-integrity` | **Fail**: `DCP.AM.002` – az ASSETMAP-ben a `Creator` az `Issuer` után van |
| Interop | `interop-cinema` | **Fail**: ugyanez és `DCP.J2K.001`; a markerek hiánya figyelmeztetés |

**Két eltérés a CFA4 ellenőrzője és a libdcp között.** Ezek a DCP-validáció projektjére tartoznak, nem erre:

1. **`DCP.AM.002`, Interop asset map:** a CFA4 egy elemsorrendet használ SMPTE-re és Interopra is (`crates/dcp/src/assetmap.rs:55`: `Creator` a `VolumeCount` előtt). A libdcp Interop sémája (`xsd/PROTO-ASDCP-AM-20040311.xsd`) viszont `… IssueDate, Issuer, Creator, AssetList` sorrendet ír elő, és a DCP-o-matic ennek megfelelően ír. Az Interop-dokumentumoknál valószínűleg a CFA4 szabálya pontatlan. Ezt nem javítom, mert más projekt kódja. A koordinátornak jelzem.
2. **`DCP.J2K.001`, többkomponensű transzformáció (MCT):** a CFA4 szerint a mozis profil X'Y'Z'-t MCT nélkül kódol. A DCP-o-matic (OpenJPEG, libdcp) képfolyamában van MCT, és a libdcp `dcpverify` elfogadja. Hogy melyiknek van igaza, azt a szabvány (ISO/IEC 15444-1 mozis profiljai, SMPTE 429-4) alapján kell eldönteni. Ezt itt nem döntöm el, ugyanúgy a koordinátornak jelzem.

Amíg ezek nincsenek tisztázva, a 2. fázis elfogadási feltétele („a DCP-o-matic DCP-je átmegy a CFA4 ellenőrzésén”) nem teljesülhet. Ezért javaslom, hogy a feltétel a `full-integrity` profilra szóljon (SMPTE: ma is Pass), a mozis profilok pedig a két kérdés tisztázása után kerüljenek bele.

### A PR #1173 vázlatkódja a valódi eszközökkel

A `cfa4-cli serve` (a PR #1173 fejénél fordítva) `CFA4_DCPOMATIC_BIN_DIR`-rel a fenti fordításra mutatva:

| Parancs | Eredmény |
|---|---|
| `dcpomatic.status` | `available: true`, `version: "2.18.51"` (a „fatal: amb” utótagot a változatolvasás levágta) |
| `dcpomatic.makeDcp` (SMPTE, 2K, flat, `encrypt: true`, `wait: true`) | kész, 152 155 134 bájt, a DCP mappáját megtalálta |
| `dcpomatic.makeKdm` (1 hét, a teszt-visszafejtő tanúsítványra) | 1 KDM-fájl. A neve `…_SMPTE_OV__.xml`, mert mozi- és teremnév nélkül üresek a mezők. Ez csak szépséghiba. |
| `dcp.loadKdm` (a CFA4 saját KDM-betöltője, a teszt magánkulcsával) | **betöltve**: 2 kulcs (MDIK, MDAK), érvényesség UTC-ben |

A kiürített környezet (csak `PATH`, `LD_LIBRARY_PATH`, locale) és a privát `HOME` mellett is lefutott, és a `dcpomatic2_openssl`-t az eszközök mappájából megtalálta. A CFA4 → DCP-o-matic → KDM → CFA4 kör tehát valódi eszközökkel is zárt. Ez a 3. fázis elfogadási feltételének előzetes, kézi igazolása, nem CI-teszt.

## 4. Futtatási függőségek

A `dcpomatic2_cli` kb. 190 rendszerkönyvtárat linkel (`ldd`), főleg az FFmpeg 6.1 láncát (x264, x265, dav1d, libvpx stb.), valamint a Boost 1.83-at, a libxml++ 2.6-ot, az xmlsec1-et, a xerces-c 3.2-t, a pangomm/cairomm 1.4-et és az ICU 74-et. A CFA4 `.deb`-je ezeket `Depends` sorral húzhatja be, így nem kell szállítani őket. A 26.04 csomagnevei eltérhetnek, ezt az 5. fázis ellenőrzi.

## 5. Kockázatok, mérés alapján

- **Fordítási lánc:** 7 saját komponens és egy javított OpenSSL. A fordítás nem bonyolult, de minden DCP-o-matic-frissítéskor a rögzített commitokat is frissíteni kell. Ezeket a `cscript` `dependencies()` függvénye sorolja fel.
- **Változatszám a gitből:** lásd 1/4. CI-ellenőrzés kell rá.
- **A CLI csendben elfogad hibás értékeket:** lásd `--container-ratio`. A CFA4-nek minden paramétert előre ellenőriznie kell.
- **Biztonság:** a javított OpenSSL 3.2.0 csak tanúsítványkészítésre fut, hálózatot nem használ. A frissítését mégis követni kell.

## 6. Mellékletek (a mérés nyers kimenete)

- `dcpomatic2_create --help`, `dcpomatic2_cli --help`, `dcpomatic2_kdm_cli --help`: [DCPOM-01-help.txt](DCPOM-01-help.txt)
- A fordítószkript: [DCPOM-01-build.sh](DCPOM-01-build.sh). A javított OpenSSL: `./Configure no-shared no-tests -fPIC && make build_programs`, az `apps/openssl` másolata `dcpomatic2_openssl` néven.

## 7. Javaslat a D8 és a D9 részleteire

- **D8:** a DCP-o-matic **2.18.51** a rögzített változat, a 0. pont táblázatának függőségi commitjaival.
- **D9:** a privát mappa tartalma a 10 MB eszköz és a 17 MB saját könyvtár, `RUNPATH`-szal a saját `lib` mappára, plusz a `dcpomatic2_openssl`. A rendszerkönyvtárak a `.deb` `Depends` sorába kerülnek. Ez összesen kb. 35 MB, becsülve tömörítés nélkül.
- **A vázlatkód javítandó** (a 2. fázisban): a `Container::Full` (`190`) kikerül, vagy a DCP-o-matic valódi értékeire cserélődik. A változatellenőrzés legkisebb értéke 2.18.51, és a keresés csak a CFA4 privát mappáját nézi (D9).
