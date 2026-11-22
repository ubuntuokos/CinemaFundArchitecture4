# BacklotHub G3: függőség- és licencnyilvántartás (javaslat, jóváhagyásra vár)

Dátum: 2026-11-21. Forrás: a 26 G2 „Licenc (G3-ra előre)" szakasza, a CFA4 `main` (`pyproject.toml`, `requirements/`).
Ez még elemzés, implementáció nincs. A CFA4 licence GPL-3.0-or-later (CLAUDE.md, AGENTS.md CFA4 szakasz).

## 1. A repó mai állapota (ellenőrizve)
- `pyproject.toml`: `requires-python >= 3.12`, a `dependencies` üres, az `qt` extra `PySide6>=6.10,<7`, az `otio` extra csak tesztorákulum.
- **SQLAlchemy, Alembic, psycopg, python-dateutil nincs** a függőségek között: a G2-02 és G2-05 szerinti bevezetésük **az 1. lépcső (Audit Trail/Production Management) része**, hash-rögzítéssel (`--require-hashes`, `docs/development/linux.md`).
- A G2-ek „Python 3.14" ellenőrzési kérései a mai 3.12+ megkötéshez képest **új feltétel**: a G3 ezt a **3.12-es alsó határra** javasolja szűkíteni (a 3.14-támogatás nem blokkoló, csak figyelendő).

## 2. Függőségek lépcsőnként

| Függőség | Licenc | Kell | Lépcső | Beillesztés | Tennivaló a bevezetéskor |
|---|---|---|---|---|---|
| PostgreSQL + contrib (`ltree`, `unaccent`, `pg_trgm`, `btree_gist`) | PostgreSQL License | igen | 1–3, 7, 18 | Ubuntu rendszercsomag (ADR 0006) | A kiválasztott PG-verzión a `postgresql-contrib` elérhetősége |
| SQLAlchemy 2, Alembic | MIT | igen | 2 | pip, hash-rögzítve | verzió és hash rögzítése |
| psycopg 3 | LGPL-3.0-or-later (GPL-3-mal összeegyeztethető) | igen | 1 | pip/Ubuntu csomag | dinamikus használat, nincs módosítás |
| python-dateutil | Apache-2.0 / BSD-3 | igen | 5 | Ubuntu `python3-dateutil` | – |
| PySide6 (`QtPdf`, `QPdfWriter`, `QtDBus`, QtWidgets) | LGPL-3.0 | már van | 6, 13, 14, 16, 18–24 | meglévő | **Ellenőrizendő:** a rögzített csomagkészletben van-e a `QtPdf` és az oldalankénti szövegkinyerés (14. lépcső) |
| `holidays` | MIT | opcionális (kézi ünnepnap nélküle is teljes) | 15 | Ubuntu `python3-holidays` | magyar naptár tesztelése ismert évekre |
| `tzdata`/rendszer tzdata | IANA, public domain | igen | 17 | rendszer | melyik forrást használja a futtató környezet |
| `icalendar` | BSD-2-Clause | csak 17-2 | 17-2 | Ubuntu `python3-icalendar` | külön G3 a 17-2 előtt |
| pyHanko (+tranzitív) | MIT | csak 14-2 | 14-2 | pip, hash-rögzítve | tranzitív licencek, méret; külön G3 a 14-2 előtt |
| OR-Tools | Apache-2.0 | csak 15-2, 23-2 | később | – | külön G3 |
| `bagit-python` | CC0 | **csak tesztorákulum**, dev-extra | 26 | dev-extra, a termékbe nem kerül | G3 döntése: legyen-e egyáltalán (saját BagIt-ellenőrző + RFC 8493 vektorok is elég) |
| Event Fabric Python SDK (NATS-kliens) | az EF-terv G3-ja | igen | 12 | – | a G2-12 szerint az EF saját G3-ja dönt |
| Rust: `blake3`, `sha2` | Apache-2.0/MIT/CC0 | igen | 9, 25, 26 | meglévő a QC/`cinema-pkg` útján | File Services ADR 0009 |

Nincs új függőség: a 3., 4., 6–11a., 13., 16., 18–25. lépcsőben a standard könyvtáron és a fentieken túl.

## 3. Referenciaprojektek (kódátvétel nélkül, István 2026-11-02 szabálya szerint)

| Projekt | Licenc | Használat |
|---|---|---|
| Kitsu/Zou | AGPL-3.0 | csak funkcionális és szerkezeti referencia |
| Stalker | LGPL-3.0 | csak minta |
| AYON core | Apache-2.0 | minta; kódátvételnél `ATTRIBUTION.md`/`NOTICE` |
| AYON (FSL-1.1) | FSL-1.1 | csak a háromszintű minta ötlete |
| Pyblish | LGPL-3.0 | a fázisok mintája |
| OpenRV, xSTUDIO | Apache-2.0 | minta; beágyazás elvetve |
| Mayan EDMS | **GPL-2.0-only** | **nem kompatibilis** a GPL-3.0-or-later-rel: kód semmiképp nem vehető át |
| paperless-ngx | GPL-3.0 | minta, kód nem |
| TaskJuggler | GPL-2.0 | nem vesszük át, külső program sem fut |
| GnuCash | GPL-2.0-or-later | csak funkcionális referencia |
| Snipe-IT | AGPL-3.0 | csak funkcionális referencia |
| Screenplain, Better Fountain | MIT | referencia és tesztforrás; kódátvétel csak külön döntéssel, attribúcióval |
| Story Architect | GPL-3.0 | funkcionális referencia |
| Kimai | AGPL-3.0 | funkcionális referencia |
| StudioBinder, Celtx, Movie Magic, Gorilla, Rentman, Flex, Current RMS, Showbiz | zárt | csak nyilvános funkciókra hivatkozunk, formátumot/arculatot nem veszünk át |

## 4. Adatok és szabványok
- **ISO 4217** (G2-24): az adatfájl használati feltételét a pénznemlista beillesztése előtt ellenőrizni kell.
- **EKB referencia-árfolyam** (G2-24): nyilvános, forrásmegjelöléssel; a megjelenítés mutatja a forrást és a dátumot.
- BagIt RFC 8493, Fountain, FDX, `geo:` RFC 5870: nyílt szabványok/nyilvános specifikációk. A „Final Draft" név csak a formátum megnevezésére.

## 5. Nyitott pontok a G3-hoz (nem döntési kérdés, ellenőrzési feladat)
1. Az internetes csomagadatok (verzió, hash, tranzitív licencek) ebből a környezetből nem érhetők el; **a rögzítés az adott lépcső bevezetésekor** történik a `docs/development/linux.md` szerint (`--require-hashes`).
2. PySide6 `QtPdf` jelenléte (14. lépcső) és a PG-contrib elérhetősége (18. lépcső).
3. Python-alsó határ: marad 3.12.

## 6. Javaslat
A G3 „átment" jelzést a G2-26-ig minden lépcsőre ez a nyilvántartás adja, **a fenti ellenőrzési feladatokkal feltételként** az érintett lépcső bevezetése előtt. A 14-2, 15-2, 17-2 és 23-2 külön G3-at kap. Új döntési kérdés nincs.
