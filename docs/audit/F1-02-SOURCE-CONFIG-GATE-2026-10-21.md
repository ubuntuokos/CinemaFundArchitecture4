# CFA4 — F1-02 forrás- és konfigurációs konzisztencia auditkapu

**Audit dátuma:** 2026-10-21  
**Tároló:** https://github.com/ubuntuokos/CinemaFundArchitecture4  
**Vizsgált commit (audit előtt):** `a8431a1d4e320856067e8a7d12d866ed6581ca2d`  
**Vizsgált ág:** `chore/f1-02-vendor-ui`  
**Kimenet:** **STATIKUS AUDIT: PASS / TELJES F1-02 INTEGRÁCIÓ: HOLD**.

Az audit forrás- és konfigurációellenőrzés, nem Rust build, nem teljes teszt,
nem licencjóváhagyás. A vizsgálat során **nem** indult teljes CI,
új FFmpeg-fordítás, csomagolás vagy disztribúció.

## A. Bizonyított eredmények

| ID | Ellenőrzés | Eredmény | Bizonyíték |
|---|---|---|---|
| A01 | `cfa4-docpath` upstream forráseredet | **PASS** | 5/5 deklarált Git-blob SHA egyezik a CFA4 fával és az upstream `82a24b006eefa036b43c09a67fb9a3fbd17ffe63` fával |
| A02 | `cfa4-ui` upstream forráseredet | **PASS** | 25/25 deklarált Git-blob SHA egyezik a CFA4 fával és az upstream `5c874d51eeb3de70e735c8ce87a59a7dedd47675` fával |
| A03 | Eredeti `Cargo.toml` származási hivatkozások | **PASS** | Mindkét `original_crate_manifest_blob_sha1` megegyezik a rögzített upstream commit manifestjével |
| A04 | Cargo közvetlen Git-függőségek | **PASS (statikus)** | Az aktuális fában található **58** `Cargo.toml` ellenőrzése: **0** `git =` deklaráció |
| A05 | `Cargo.lock` Git-források | **PASS (statikus)** | **709** csomagbejegyzés, **0** `git+` forrás, `cfa4-docpath` és `cfa4-ui` helyi bejegyzésekkel |
| A06 | Saját Linux AMD64 platformbeállítások | **PASS (konfiguráció)** | Linux x86_64 korlát a dev/bootstrap, ellenőrzés, telepítő, nFPM, csomagoló és Flatpak útvonalon |
| A07 | Automatikus CI terhelés | **PASS (konfiguráció + korábbi CI)** | PR/push eseményre csak a könnyű `platform-scope` gate aktív; **7** nagy ellenőrzés `workflow_dispatch` vagy az explicit `cfa4-full-validation` címke felvételére indul |
| A08 | Rust workspace release teszt scope | **PASS (konfiguráció)** | `cargo test --workspace --exclude cfa4-ui --release --locked --timings`; az eredeti `cfa4-ui` tesztek külön CI-jobban |
| A09 | FFmpeg teszt-oracle centralizálás | **PASS (konfiguráció)** | FFmpeg 9.0.2 install/cache a `rust-release-tests` jobban; az `xtask ci` nem futtat párhuzamos teljes release-tesztet |
| A10 | PR-függőségi lánc és egyesítési státusz | **PASS (állapot rögzítve)** | `#3 → #5 → #7 → #8 → #10`; minden PR draft, egyik sincs a `main` ágba egyesítve |

A `SOURCE_MANIFEST.json` önmagában csak állítás lenne; itt az eredeti
the upstream library repository **pontos commitjainak Git-fájlfájával** is
összehasonlítás történt. A licencek és NOTICE a két upstream commit gyökérszintjén
vannak, nem az egyes `crates/*` mappákban; ezt a forrásösszehasonlítás figyelembe vette.

## B. Még nem igazolt kapuk / ismert nyitott kockázatok

| ID | Állapot | Teendő |
|---|---|---|
| B01 | **NEM IGAZOLT** | A legújabb commiton **nem** futott teljes Rust/Qt6/FFmpeg build és regressziós teszt. A korábbi automatikus zöld CI csak a statikus kaput igazolja. Egyszeri célzott teljes CI az *utolsó* audit/javító commiton szükséges az integráció előtt. |
| B02 | **JAVÍTVA A FORRÁSBAN; CI ELLENŐRZÉS FÜGGŐBEN** | A `packaging/linux/package.sh` automatikus `continuous` AppImage-letöltését megszüntettük. Csak meglévő, futtatható helyi `APPIMAGETOOL` és kötelező, előzetesen megbízható forrásból rögzített `CFA4_APPIMAGETOOL_SHA256` mellett engedélyezett AppImage-csomagolás. A hash megadása önmagában nem hitelesíti a forrást; az eszköz eredete külön auditálandó. |
| B03 | **NEM IGAZOLT** | A crates.io tranzitív függőségek, PySide6/Qt, FFmpeg, Flatpak/AppImage és egyéb csomagolóeszközök teljes SBOM-ja, licenc- és terjesztési vizsgálata. |
| B04 | **NEM IGAZOLT** | Teljes offline build; a két saját forrásra vett Git-függőség önmagában nem függetleníti az egész crates.io/Python ökoszisztémától a projektet. |
| B05 | **NYITOTT KOCKÁZAT** | Örökölt nem-Linux/AMD64 forrás és csomagolófájlok továbbra is megtalálhatók, de az aktív CI/csomagolás nem célozza őket. Későbbi, külön tesztelt takarításban kezelendők. A `Cargo.lock` Android-rekordjai *nem* önálló Android-buildcélok. |
| B06 | **KIADÁSI BLOKKOLÓ** | A harmadik felek attribúciói, márkaelemei, Qt-migráció és a csomagok továbbterjeszthetősége még nincs jóváhagyva; `CFA4_ALLOW_LOCAL_PACKAGING` kizárólag helyi fejlesztői próba. |
| B07 | **ELJÁRÁSI KOCKÁZAT** | `cfa4-full-validation` címkével indított zöld teljes CI nem érvényes automatikusan újabb commitokra: az aktuális HEAD-re újra kell indítani a teljes validációt. |

## C. Audit után hozzáadott olcsó ellenőrzés

Az `scripts/ci/check-linux-amd64.sh` kapuba bekerül az alábbi, külső csomag
nélkül és Rust-fordítás nélkül futó Python-ellenőrzés:

```bash
PYTHONPATH=python python3 -m cfa4_devtools.dependency_audit
```

Ez a Git-függőségek tiltását, a `Cargo.lock` helyi eredetét, a két
`SOURCE_MANIFEST.json` fájlt és mind a 30 átvett forrásfájl Git-blob SHA-ját
automatikusan ellenőrzi minden PR/push statikus kapujában. A **meglévő**
`dependency_audit.py` hajtja végre; nem igényel Cargo-buildet vagy hálózatot.
A Python egységtesztek és a teljes CI továbbra is csak kifejezett indításra futnak.

### Kiegészítő B02 javítás – AppImage eszközellátás

A változó, ellenőrizetlen AppImage-eszközletöltést kivezettük.
A csomagoló nem tölthet le futtatható eszközt: csak meglévő helyi
`APPIMAGETOOL` binárist fogad el, érvényes, előzetesen ellenőrzött
`CFA4_APPIMAGETOOL_SHA256` értékkel. Eltérés vagy hiány esetén
hibával leáll, mielőtt az eszközt elindítaná. A statikus CI-kapu
védi e követelmények fennmaradását. A csomagolás továbbra is csak
helyi tesztcélra engedélyezett, nem kiadási jóváhagyás.

## D. Auditdöntés és integrációs sorrend

**Forrás- és konfigurációs konzisztencia audit: PASS**, a fenti korlátozásokkal.  
**F1-02 teljes validáció: HOLD**, mivel az új HEAD kötelező teljes CI-je
még nincs sikeres futással igazolva.  
**Merge: TILOS** a teljes gate lezárásáig.  
**Terjesztés: NEM ENGEDÉLYEZETT** a nyitott licencek/márkaelem-korlátok miatt.

Biztonságos PR-sorrend: **#3 → #5 → #7 → #8 → #10**. A külön
F0-bázisauditot (#2) önállóan kell kezelni; nem az F1-02 közvetlen előfeltétele.
Az egyesítési sorrendet az akkori PR-fejek, diffek és kötelező CI-kapuk
alapján ismét ellenőrizni kell.

## E. Ellenőrizhető hivatkozások

- CFA4 aktuális audit előtti commit: https://github.com/ubuntuokos/CinemaFundArchitecture4/commit/a8431a1d4e320856067e8a7d12d866ed6581ca2d
- Korábbi statikus sikeres CI: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39634711506
- F1-02/1 PR: https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/8
- F1-02/2 PR: https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/10
- docpath eredeti: upstream library `crates/docpath` @ `82a24b006eef`
- UI eredeti: upstream library `crates/ui` @ `5c874d51eeb3`
- Korábbi F1-01 audit: [f1-01-dependencies.md](f1-01-dependencies.md)
- F1-02 migráció és korábbi CI-történet: [f1-02-vendor-migration.md](f1-02-vendor-migration.md)

**Értelmezés:** PASS kizárólag a felsorolt statikus ellenőrzésekre vonatkozik.
