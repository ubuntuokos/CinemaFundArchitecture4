# DCP-01 – dcp_inspect forrás- és licencaudit

**Dátum:** 2026-10-27  
**Státusz:** audit kész, **tulajdonosi jóváhagyásra vár** (7. pont)  
**Terv:** [CFA4-DCP-VALIDATION-001](../plans/DCP-VALIDATION-2026-10-27.md), DCP-01 munkacsomag  
**Párdokumentum:** [DCP-00 képességaudit](DCP-00-CAPABILITY-AUDIT-2026-10-27.md)

A vizsgálat csak olvasásos volt: a `wolfgangw/backports` tárolóból **semmi nem került a CFA4-be**.
A referenciamotort a [`scripts/dev/setup-dcp-inspect.sh`](../../scripts/dev/setup-dcp-inspect.sh)
a repón kívülre, a fejlesztő gépére telepíti (lásd [`docs/development/dcp-validation.md`](../development/dcp-validation.md)).

## 1. Rögzített referencia

| Mező | Érték |
|---|---|
| Tároló | <https://github.com/wolfgangw/backports> |
| Commit | `99d0fd863fe05eb806f1625233f2e93676ead268` |
| Commit ideje és üzenete | 2026-10-03 20:13:51 +0200, „Bump dcp_inspect version to 1.2026.10.03” |
| `VERSION` | `1.2026.10.03` |
| Követett fájlok | 122 (`lib/` 48, `test/` 45, `xsd/` 22, gyökér 7) |
| XSD-manifest | `xsd/MANIFEST.sha256`: 21 bejegyzés, **mind a 21 sha256 egyezik** (`sha256sum -c`, 2026-10-27) |

A 2026-10-27-i `git ls-remote` szerint ez a `HEAD`. A telepítő szkript pontosan ezt a commitot
kéri le, és eltérésnél leáll.

## 2. Fájlonkénti licenc

A tároló gyökerében a `gpl.txt` a GNU GPL 3. változatának szövege. Licencfejléc csak a
`lib/dcp_inspect/inspection/runtime.rb`-ben van: „2011-2026 Wolfgang Woehl”, **GPL-3.0-or-later**.
A többi fájlban nincs fejléc; ezekre a tároló szintű GPL-3 vonatkozik („or later” a runtime.rb
fejléce és a GPLv3 14. pontja alapján). SPDX-azonosító sehol nincs.

| Fájl(ok) | Licenc | Szerzői jog | Megjegyzés |
|---|---|---|---|
| `dcp_inspect`, `Rakefile`, `VERSION`, `lib/dcp_inspect.rb`, `lib/dcp_inspect/**/*.rb` (az alábbi kivételekkel), `benchmark/`, `test/**` | GPL-3.0-or-later | Wolfgang Woehl, 2011–2026 | Fejléc csak a `runtime.rb`-ben |
| `lib/dcp_inspect/inspection/iab.rb`, `jpeg2000_headers.rb`, `timed_text.rb` | **BSD-3-Clause** (`lib/dcp_inspect/inspection/BROWSER_BACKPORT_LICENSE`) | © 2026 Wolfgang Woehl | A fájlok a böngészős dcp_inspect `iab.js`, `jpeg2000-codestream-scan.js`, timed-text moduljaiból adaptáltak, és hivatkoznak a BSD-licencre |
| `lib/dcp_inspect/inspection/content_title.rb` | **BSD-3-Clause** | Wolfgang Woehl | „Adapted from dcp_inspect/src/dcp/content-title-text.js (BSD-3-Clause)” |
| `lib/dcp_inspect/inspection/dcnc_registry.json` | **nincs megadva** | ISDCF (adatforrás) | ISDCF-regiszter pillanatképe (2026-10-01): létesítmény-, nyelv- és területkódok. Harmadik fél adata, licence tisztázatlan |
| `runtime.rb` 657. sortól: `distance_of_time_in_words` | **MIT** (Ruby on Rails, actionpack 3.2.11) | Rails-szerzők | Átvett kód MIT-megjegyzés nélkül |
| `lib/dcp_inspect/timecode.rb` | GPL-3.0-or-later (tárolószint) | – | A README köszönetet mond „Julik … Timecode library” (guerilla-di/timecode, MIT) projektnek; a fájlban nincs eredetjelzés. **Valószínű MIT-eredet, ellenőrizendő** |
| `test/fixtures/pkl_missing_signature_method.xml` | GPL-3.0-or-later (tárolószint) | – | Szintetikus tesztadat |

**Kompatibilitás a CFA4-gyel:** a CFA4 GPL-3.0-or-later (`LICENSE`, `docs/licensing.md`). A
GPL-3.0-or-later, a BSD-3-Clause és az MIT mind kompatibilis vele, így a dcp_inspect kódja
**licencjogilag átvehető** volna a `docs/licensing.md` §3 szerint (eredeti fejléc + §5
regiszterbejegyzés). A terv és a DCP-00 D7 javaslata mégis a szabványból írt natív kódot
részesíti előnyben (lásd 5. pont).

## 3. XSD-k eredete és terjesztési feltételei

| Fájl | Eredet (névtér) | Licenc / feltétel | Javaslat |
|---|---|---|---|
| `SMPTE-429-7-2006-CPL.xsd`, `SMPTE-429-8-2006-PKL.xsd`, `SMPTE-429-9-2007-AM.xsd`, `SMPTE-429-10-2008-Main-Stereo-Picture-CPL.xsd`, `SMPTE-430-1-2006-Amd-1-2009-KDM.xsd`, `SMPTE-430-3-2008-ETM.xsd`, `SMPTE-430-7-2008-FLM.xsd`, `SMPTE-430-9-2008-KDMB.xsd`, `dcmlTypes.xsd` (ST 433), `DCDMSubtitle-2010.xsd` (ST 428-7:2010) | SMPTE (`smpte-ra.org/schemas/...`) | A fájlokban nincs licencnyilatkozat. Az SMPTE a sémákat nyilvánosan közzéteszi, a terjesztési feltételeket külön ellenőrizni kell | **Nem kerül a CFA4-repóba.** A referenciamotor a rögzített commitból, manifest-ellenőrzéssel használja. Csomagba (.deb/Flatpak/AppImage) csak az SMPTE-feltételek igazolása után (DCP-18) |
| `PROTO-ASDCP-AM-20040311.xsd`, `PROTO-ASDCP-PKL-20040311.xsd`, `PROTO-ASDCP-CPL-20040511.xsd`, `437-Y-2007-Main-Stereo-Picture-CPL.xsd` | CineCert / digicine (Interop) | Nincs nyilatkozat | Mint fent |
| `DCSubtitle.v1.mattsson.xsd` | Interop DCSubtitle, szerző Mattsson (a fájlnév szerint) | Nincs nyilatkozat | Mint fent |
| `xmldsig-core-schema.xsd`, `xmldsig-core-schema.dtd` | W3C / IETF (2001) | W3C Software Notice (megengedő) | Terjeszthető a W3C-nyilatkozattal |
| `xenc-schema.xsd`, `xml.xsd`, `XMLSchema.xsd` | W3C | W3C Software/Document Notice | Mint fent |
| `catalog.xml`, `MANIFEST.sha256` | dcp_inspect | GPL-3.0-or-later | Csak a referenciamotorral együtt |

Hiányzó sémakiadások (a dcp_inspect README „What's missing” része szerint is): ST 428-7 2014-es
kiadása, ST 429-16 (CPL-metaadat / CMA), ST 2067 (IMF). A natív motor a hiányzó kiadásokra
`Unchecked`-et ad.

## 4. Futásidejű függőségek (a jóváhagyandó jegyzék)

A forrásban talált tényleges hívások alapján (`require`, `Open3`, `Process.spawn`):

| Függőség | Mire használja | Licenc | Forrás Ubuntu 26.04-en | CFA4-izoláció | Ellenőrzés |
|---|---|---|---|---|---|
| Ruby | futtatókörnyezet; 3.4 alatt és felett eltérő `base64`-kezelés (`runtime.rb`) | Ruby License / BSD-2-Clause | apt `ruby` | A rendszer Rubyját **csak futtatja**, nem módosítja | 3.3.6 alatt statikusan átnézve; 26.04-en a szkript `--check` módja ellenőrzi |
| Ruby stdlib: `json`, `openssl`, `digest`, `optparse`, `open3`, `tempfile`, `date`, `set`, `uri`, `pathname`, `rbconfig`, `io/console`, `ostruct`, `debug` (csak `--debug`) | | Ruby License / BSD-2-Clause | a Rubyval | – | Az `ostruct` újabb Rubyn bundled gem lehet: a szkript ellenőrzi |
| `base64` gem (Ruby ≥ 3.4) | hash- és aláírásértékek | Ruby License / BSD-2-Clause | gem | CFA4 saját `GEM_HOME` | |
| Nokogiri (+ beépített libxml2, libxslt) | XML-olvasás, XSD-validáció, katalógus (`XML_CATALOG_FILES`) | MIT (libxml2, libxslt: MIT) | gem (előfordított x86_64-linux-gnu) vagy apt `ruby-nokogiri` | CFA4 saját `GEM_HOME`, rögzített verzió | |
| `racc` 1.8.1, `bigdecimal` 3.3.1 | a Nokogiri, illetve a TTFunk függősége; natív kiterjesztést fordítanak (`ruby-dev`, `build-essential`) | Ruby License / BSD-2-Clause | gem | CFA4 saját `GEM_HOME` | A szkript `--install` módja 2026-10-27-én Ruby 3.3.6-on lefordította |
| TTFunk | betűkészlet-glyph ellenőrzés (`timed_text.rb`) | Ruby License vagy GPL-2.0-only vagy GPL-3.0-only (választható) | gem vagy apt `ruby-ttfunk` | CFA4 saját `GEM_HOME` | A CFA4 a GPL-3.0-only ágat választja, ha csomagolja |
| OpenSSL (Ruby `openssl` kiterjesztés) | aláírás- és tanúsítványellenőrzés | Apache-2.0 (OpenSSL 3) | rendszer | – | |
| asdcplib CLI: `asdcp-info`, `asdcp-unwrap` | MXF-fejlécek, hang- és timed-text kicsomagolás | BSD-3-Clause (CineCert) | **Ubuntu-csomag elérhetősége nem igazolt** | Ha forrásból kell fordítani, CFA4-prefixbe, `PATH`-ra csak a motor folyamatának | Nyitott (7. pont, L4) |
| FFmpeg: `ebur128`, `astats` szűrők | loudness és csúcsérték a referenciamotorban | GPL build (a CFA4 rögzített 9.0.2-es orákuluma) | `scripts/ci/install-ffmpeg-9.0.2.sh` | A motor `PATH`-ján a rögzített 9.0.2 áll a rendszer FFmpeg előtt; a rendszer FFmpeg-je érintetlen | |
| `fd` / `fdfind` (opcionális) | gyors fájlfelfedezés; hiányában Ruby-bejáró | MIT OR Apache-2.0 | apt `fd-find` | – | Opcionális |
| `mkfifo` (coreutils) | hangelemzési csővezeték | GPL-3.0-or-later | rendszer | – | |

Kimeneti szerződés a DCP-04 adapterhez (forrásból): `--dump-result <path>` a teljes strukturált
eredményt JSON-ban írja (`runtime.rb` 2644–2653), `--dump-model` az `InspectionRun`-modellt; a
kilépési kódok a `runtime.rb` 111–135. sorában vannak (0 hibátlan, 1 talált hiba, 18 megszakítás, 19 nem
támogatott Ruby, 23 könyvtárbetöltési hiba). A JSON-nak **nincs sémaverziója**, ezért az adapter a
rögzített commithoz kötődik, és a beolvasott JSON-t ellenőrzi.

## 5. Átvételi szabály a natív Rust-motorhoz

1. A natív szabályok a SMPTE-szabványokból és a nyilvános specifikációkból készülnek
   (`AGENTS.md` §2), a crate README-jében rögzített kiadásokkal.
2. A dcp_inspect **fekete dobozos referencia**: ugyanazon fixture-ökön összevetjük az eredményt
   (terv 8. pont, DCP-17).
3. Ha egy konkrét logikát mégis portolunk (pl. ISDCF-névelemzés, JPEG 2000 cinema-fejlécszabályok),
   az licencjogilag megengedett: eredeti szerzői jog és licenc a fájl fejlécében (GPL-3.0-or-later
   vagy BSD-3-Clause a 2. pont szerint), sor a `docs/licensing.md` §5 regiszterébe, ugyanabban a
   commitban. Az MIT-eredetű `distance_of_time_in_words` és a valószínűleg MIT-eredetű
   `timecode.rb` nem kell a CFA4-nek (a `cfa4-time` lefedi).
4. A `dcnc_registry.json` nem vehető át, amíg az ISDCF-adat felhasználási feltétele nincs tisztázva.
5. XSD nem kerül a CFA4-repóba (3. pont).

## 6. Statikus biztonsági megfigyelések (a DCP-04 adapter tervezéséhez)

- `runtime.rb` 556., 561. sor: `Open3.capture3` **egyetlen parancssorral** hívja az
  `asdcp-info`-t (shellen át, `Shellwords.shellescape`-pel). Működik, de a CFA4-adapter a motort
  csak argumentumtömbbel, shell nélkül indítja.
- XML: `Nokogiri::XML(File.open(file))` alapbeállítással (RECOVER | NONET; entitáshelyettesítés
  nincs bekapcsolva); a feliratok `strict.nonet`-tel. Hálózati séma-letöltés így nincs, a
  katalógus helyi (`XML_CATALOG_FILES`).
- Írás: csak a kért logfájlba, a `--dump-result` célra, ideiglenes fájlokba (FIFO is) és
  `--autolog` esetén a `$DCP_INSPECT_DIR`-be. Az adapter a folyamatot ideiglenes munkakönyvtárral,
  ideiglenes `HOME`-mal és `TMPDIR`-rel, a DCP-t csak olvasva futtatja; a kimenet a DCP mappáján
  kívül marad.
- Időkorlát és megszakítás: a motor `Interrupt`-ra (SIGINT, `q`) 18-as kóddal leáll (`application.rb` 48–51.); az adapter folyamatcsoportot
  indít, és időtúllépéskor az egész csoportot leállítja.

## 7. Jóváhagyandó pontok

| # | Pont | Javaslat |
|---|---|---|
| L1 | Referencia-commit | `99d0fd863fe05eb806f1625233f2e93676ead268` (1.2026.10.03) |
| L2 | A dcp_inspect a CFA4 számára | Repón kívüli, izolált referenciamotor (fejlesztés, CI-paritás, opcionális backend); semmi nem kerül a CFA4-repóba |
| L3 | Ruby-gemek | Saját `GEM_HOME` a CFA4-gyorsítótárban, pontosan rögzített készlettel, függőségfeloldás nélkül: `racc` 1.8.1, `nokogiri` 1.19.4, `bigdecimal` 3.3.1, `ttfunk` 1.8.0, Ruby ≥ 3.4-en `base64` 0.2.0; a globális gem-környezet nem változik |
| L4 | asdcplib | Ha az Ubuntu 26.04 nem csomagolja: forrásból CFA4-prefixbe (külön, jóváhagyandó lépés); addig az MXF-mély vizsgálat a referenciamotorban `Unchecked` |
| L5 | XSD-k csomagolása | Nem a repóba; csomagba csak az SMPTE-feltételek igazolása után |
| L6 | ISDCF-regiszter | Nem vesszük át; a natív F13 forrása külön döntés |
| L7 | Kódátvétel | Alapból nincs; konkrét portolás csak az 5. pont szerint, regiszterbejegyzéssel |

## 8. Ebben a munkamenetben nem igazolt

- A dcp_inspect saját tesztkészlete (`rake test`) és `rake xsd:check` futtatása: a munkamenet
  szabályai nem engedték külső kód futtatását. A telepítő szkript `--verify` módja ezt a
  fejlesztő gépén elvégzi; az eredményt a DCP-04 PR rögzíti.
- Ubuntu 26.04-csomagnevek és -verziók (Ruby, Nokogiri, TTFunk, asdcplib): a vizsgálat
  Ubuntu 24.04-es konténerben futott; a `--check` mód a referenciagépen (Kubuntu 26.04.1) igazolja.
