# F1 kiegészítő audit: GEGL és babl (CFA4-RENDER-INTEGRATION-PLAN-001)

**Dátum:** 2026-11-23 · **Jelleg:** csak olvasás (GitHub keresési API metaadat); sem build, sem telepítés, sem kódátvétel.
**Ok:** a tulajdonos 2026-11-23-án az F2 POC-jelöltek közé vette a GEGL-t és a babl-t (terv rev 117; S1 5. szabály: 4 jelölt). A terv F1 kiegészítést kér:
licenc, karbantartók és aktivitás, host-lábnyom (GLib/GObject), sandbox-profil, Python-igény.
**Bizonyítási szabály:** ami nincs itt mérve vagy forrásból igazolva, az PENDING; "általános ismeret" nem PASS.

## 1. Eredmény

| Szempont | Eredmény | Alap |
|---|---|---|
| Licenc | **PASS**: a GEGL mag LGPL-3+, 47 fájlcsoport (köztük az `operations/common-gpl3+/` operációk) GPL-3+, a babl LGPL-3+ (és Expat); egyik sem GPL-inkompatibilis; a CFA4 GPL-3.0-or-later | 2. pont (a kiadási tarballok `debian/copyright` és fejlécszkennelése) |
| Tranzitív függőségek licence | **PENDING (részben)**: a 21 futásidejű csomag névsora megvan (3. pont); a poppler, SuiteSparse/UMFPACK, LibRaw és lensfun fájlszintű licencét nem vizsgáltam; a javasolt minimum-konfiguráció ezeket kihagyja | 3. pont |
| Karbantartók | **PASS**: ≥ 2 aktív szerző mindkettőnél (küszöb: ≥ 2) | 4. pont |
| Aktivitás és kiadás | **PASS**: GEGL 0.4.70 (2026-03), babl 0.1.124 (2026-03); mindkettőből 12 hónapon belül több kiadás | 4. pont |
| Host-lábnyom (GLib/GObject) | **PASS-feltétellel**: a mag kötelező függősége GLib, GObject, GModule, GThread, GIO, JSON-GLib és babl; minden más képkodek választható (`meson_options.txt`); a futásidejű mérés **PENDING** | 5. pont |
| Sandbox-profil | A `renderWorker` profil a terv része (nem kér külön jóváhagyást); a moduláris operációk betöltésének Landlock alatti mérése **PENDING** (felhőből nem mérhető: lásd 6. pont) | 6. pont |
| Python-igény | **PASS (nem várt)**: a `pygobject` opció választható; a használat C-könyvtárként vagy külön workerként megy | 5. pont |
| **Összesített döntés** | **PASS F2-POC-jelöltként, csak külön workerben** | |

BLOCKED nincs. A fennmaradó PENDING tételek mérése a 6. pontban.

## 2. Életképesség (GitHub-tükör, `GNOME/gegl`, `GNOME/babl`)

Az adatok a GitHub keresési API-jából valók (commit-szerzők 2025-11-23 óta). A "karbantartó" itt a commit-szerző, nem a jogosultság;
a bot-fiókokat és az egyazon személy több azonosítóját nem vontam össze, ezért a szerzőszám felső becslés.

| | GEGL | babl |
|---|---|---|
| Licenc (metaadat) | LGPL-3.0 | LGPL-3.0 |
| Utolsó push | 2026-11-21 | 2026-10-24 |
| Archivált | nem | nem |
| Commit 12 hónapon belül | 385 | 133 |
| Különböző szerző (az első 100 commitból) | 20 | 6 |
| Legaktívabb szerzők | Ondřej Míchal (38), Bruno Lopes (30), Øyvind Kolås (7), Jehan (3) | Bruno Lopes (71, két azonosító), Øyvind Kolås (19), Jehan (5) |

Megjegyzés: a GEGL 385 commitjából csak az első 100 szerzői listája érkezett meg (a keresési API oldalhatára); az eloszlás ezért a legutóbbi időszakot tükrözi.
A babl commitjainak zöme egy szerzőtől jön (Bruno Lopes), de legalább három további szerző is hozzájárult; az "egyszemélyes" kockázat (ReelForge, avio, rust-motion esete) itt nem áll fenn.

## 2. Licenc (forrásolvasás a kiadási tarballokon)

Forrás: az Ubuntu 26.04 (`resolute`) forráscsomagjainak upstream tarballja, gegl 0.4.70 és babl 0.1.124 (a `debian/copyright` és a `COPYING*`/forrásfejlécek).
Letöltés és olvasás történt, build nem.

| Komponens | Licenc | Megjegyzés |
|---|---|---|
| GEGL mag és a legtöbb operáció | LGPL-3+ (91 fájlcsoport; 240 operációfájl hivatkozik LGPL-ra) | linkelhető; a CFA4 mégis külön workerben használja |
| GEGL `operations/common-gpl3+/` (53 operációfájl hivatkozik GPL-re) és 47 fájlcsoport | GPL-3+ | a kombinált mű GPL-3+; összefér a CFA4 GPL-3.0-or-later licencével |
| GEGL kisebb részek | LGPL-2+ (3), ISC (2), Expat (2), BSD-3-clause-Poly2Tri (2), X-Open-Group-DEC (2) | mind megengedő vagy kompatibilis |
| babl mag | LGPL-3+ és Expat | |
| babl `extensions/gggl*.c`, `gegl-fixups.c` | GPL-3+ "GGGL kivétellel" | a kivétel a szöveg szerint a GPL-t nem szigorítja; a babl-ban modulként épül |

**Következtetés:** nincs GPL-inkompatibilis komponens a két tarballban. A GPL-3+ operációk miatt a *teljes* GEGL-build GPL-3+ hatású, ami a CFA4 GPL-3.0-or-later licencével összefér;
ha a CFA4 később LGPL-hatású (nem-GPL) csomagot is akarna kiadni, az operációkészletet szűkíteni kell (`common-gpl3+` kihagyása). Ez most nem követelmény.

## 3. Tranzitív függőségek (Ubuntu 26.04 bináris csomagok)

`libgegl-0.4-0t64` 0.4.70-1 (telepítve 8,1 MB, plusz `libgegl-common` 6,1 MB) függ ezektől: `libbabl-0.1-0`, GLib, JSON-GLib, Cairo, Pango/PangoCairo, gdk-pixbuf, libjpeg, libpng, LibTIFF, libwebp, OpenEXR, LibRaw, librsvg, poppler-glib, SDL2, lcms2, UMFPACK (SuiteSparse), libgomp.
`libbabl-0.1-0` 0.1.124-1 (2,2 MB) függ: libc, lcms2. A build-függőségek közt van még lensfun, libmaxflow, libspiro, libswscale/libavformat (FFmpeg), gexiv2, v4l.

| Kategória | Elemek | Licenc-állapot |
|---|---|---|
| Kötelező (mag) | GLib, GObject, GModule, GThread, GIO, JSON-GLib, babl | LGPL-2.1+ (általános ismeret, nincs fájlszintűen vizsgálva) |
| Választható, a javasolt minimumból kihagyandó | poppler, UMFPACK/SuiteSparse, LibRaw, lensfun, FFmpeg (`libav`), SDL2, v4l, gexiv2, SVG, Pango/Cairo | **PENDING**: fájlszintű licenc nem vizsgált; a poppler (GPL), SuiteSparse (vegyes) és LibRaw (LGPL/CDDL kettős) külön figyelendő |
| Képformátum (EXR, PNG, TIFF, JPEG, WebP) | OpenEXR, libpng, LibTIFF, libjpeg, libwebp | megengedő/BSD típusú (általános ismeret); az OpenImageIO már lefedi az EXR/TIFF I/O-t, így itt kihagyható |

SBOM: a fenti névsor és a `Build-Depends` mezők (Sources index, `resolute`) a nyers SBOM; verziók a bináris csomagoknál fent. A CycloneDX formátumú SBOM az F2 indulásakor készül.

## 4. Életképesség (GitHub-tükör és kiadási napló)

A commit-szerzők a GitHub keresési API-jából valók (2025-11-23 óta); a kiadási dátumok a Debian-changelogból (a kiadási címkék dátumát jól közelítik).
A "karbantartó" itt a commit-szerző, nem a jogosultság; az egyazon személy több azonosítóját nem vontam össze, ezért a szerzőszám felső becslés. A GEGL szerzőlistája az első 100 commitból való (a keresési API oldalhatára).

| | GEGL | babl |
|---|---|---|
| Licenc (metaadat) | LGPL-3.0 | LGPL-3.0 |
| Utolsó push | 2026-11-21 | 2026-10-24 |
| Archivált | nem | nem |
| Commit 12 hónapon belül | 385 | 133 |
| Különböző szerző | 20 (az első 100 commitból) | 6 |
| Legaktívabb szerzők | Ondřej Míchal (38), Bruno Lopes (30), Øyvind Kolås (7), Jehan (3) | Bruno Lopes (71), Øyvind Kolås (19), Jehan (5) |
| Utolsó kiadás | 0.4.70 (2026-03-29) | 0.1.124 (2026-03-15) |
| Kiadások az elmúlt 12 hónapban | 0.4.64, 0.4.66, 0.4.68, 0.4.70 és csomagolási frissítések | 0.1.116, 0.1.118, 0.1.120, 0.1.122, 0.1.124 |
| Hivatalos karbantartó (`MAINTAINERS`) | Øyvind Kolås (egy név a fájlban) | – |

Megjegyzés: a `MAINTAINERS` egyetlen nevet tartalmaz, de a tényleges commit-aktivitás legalább három szerzőre oszlik; az "egyszemélyes" kockázat (ReelForge, avio, rust-motion esete) nem áll fenn.
A babl zöme egy szerzőtől jön, de legalább három másik is hozzájárult.

## 5. Host-lábnyom és Python

- **Kötelező a magnál** (`meson.build`): `glib-2.0`, `gobject-2.0`, `gmodule-2.0`, `gthread-2.0`, `gio-2.0`, `json-glib-1.0`, `babl-0.1`.
- **Minden más** a `meson_options.txt` szerint `feature` típusú (auto/enabled/disabled): `gdk-pixbuf`, `gexiv2`, `graphviz`, `lcms`, `lensfun`, `libav`, `libraw`, `librsvg`, `libspiro`, `libtiff`, `libv4l`, `libv4l2`, `lua`, `mrg`, `maxflow`, `openexr`, `openmp`, `cairo`, `pango`, `pangocairo`, `poppler`, `pygobject`, `sdl2`, `umfpack`, `webp`. A javasolt F2-konfiguráció: ezek kikapcsolva, kivéve amit a próba kér (pl. `openmp`, `lcms`).
- **Python:** a `pygobject` opció választható; a CFA4 worker nem használja. Új Python-verzió nem kerül be, A11 érintetlen.
- **Szálmodell:** a GEGL saját szálkészletet és GLib-ot használ; a külön workerben ez nem keveredik a CFA4 fő folyamatának szálaival. Az alkalmazás-szintű mérés (memória, indulási idő) az F2 első napjaiban.

## 6. Ami felhőből nem (vagy a "csak olvasás" szabály miatt nem) mérhető

| Tétel | Miért PENDING | Hol mérhető |
|---|---|---|
| Futásidejű lábnyom és indulási idő (minimum-konfiguráció) | építés nélkül nem mérhető; a szabály ennél az auditnál "csak olvasás", a build a felhőben nem kapott engedélyt | az F2 első napja, referenciagépen vagy külön engedéllyel a felhőben |
| Landlock/seccomp alatti operáció-modulbetöltés | a `renderWorker` profil és a futtatás kell hozzá | F2 első napjai a referenciagépen |
| A poppler, SuiteSparse, LibRaw, lensfun fájlszintű licence | a minimum-konfiguráció kihagyja őket; csak akkor kell, ha a próba bekapcsolja | ha egy ilyen opció bekerül az F2-be |
| CycloneDX SBOM | a pontos build-konfiguráció után készül | F2 kezdete |
| A kiadási címkék pontos dátuma | a Debian-changelog dátumát használtam (közelítés) | az F2 elején a címkék |

A `renderWorker` sandbox-profil a terv része; külön jóváhagyást nem kérek hozzá.
