# CFA4-PY-TYPING-001 – A Python-kód típusellenőrzése (mypy strict) blokkoló CI-kapuvá

**Verzió:** 1.0 (jóváhagyva)
**Dátum:** 2026-11-12
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-12-én kérte a tervet, miután a #1106 (a #1090
javított változata) a `mypy`-t felvette a `dev` csoportba és a lockfájlba, de CI-kapuvá nem
tette. 2026-11-12-én a tulajdonos eldöntette az ütemezést (VI, D8): a Qt6 GUI tesztelésre vár,
ezért most csak a T0 és a gyanús helyek listája (T0b) készül, a T2–T5 a Qt6 tesztelése után. A
tulajdonos ugyanaznap a tervet jóváhagyta, a D1–D7 döntéseket a javaslatok szerint (VII). A
munkát külön munkamenet végzi (D5); a T0 a #1106 beolvasztása után indulhat, a T0b azonnal.
**Kapcsolódik:** [`AGENTS.md`](../../AGENTS.md) §0 (never crash), [`CLAUDE.md`](../../CLAUDE.md)
(Python / Qt ellenőrzések), [`docs/coordination.md`](../coordination.md) (§2 PR-szabályok, §3.4 Qt
GUI-tesztek), [`CFA4-QT6-FINISH-001`](CFA4-QT6-FINISH-001.md) (a Qt6 GUI befejezése),
[`docs/development/linux.md`](../development/linux.md) (`.venv`, `--require-hashes`),
`pyproject.toml` `[tool.mypy]` (#1106), `requirements/linux-amd64.txt`

---

## I. Cél

A `python/` mappa (`cfa4_devtools`, `cfa4_qt`, `python/tests`) minden fájlja átmegy a
`mypy --strict` ellenőrzésen, és ez **blokkoló CI-kapu**: új típushiba nem kerülhet a `main`-be.

Miért kell:
- **Never crash (§0).** A mypy futtatás előtt elkapja azokat a hibákat, amelyek a Qt6 GUI-ban
  futás közben `AttributeError`-ral vagy `TypeError`-ral állnának meg: `None` értéken hívott metódus
  (a Qt sok függvénye `None`-t adhat vissza), elírt vagy nem létező Qt-metódus, rossz típusú
  argumentum.
- A Qt6 GUI kb. 51 000 sor Python. Ekkora kódban a típusok a refaktorálás és a párhuzamos
  munkamenetek közös biztonsági hálója.
- A #1106 óta a `pyproject.toml` már tartalmazza a strict célkonfigurációt, de semmi nem futtatja.

**Nem cél:** a működés megváltoztatása. A típusok pótlása és a `None`-ellenőrzések nem adhatnak új
funkciót, és nem írhatják át a GUI viselkedését (kivétel: egy valódi, összeomlást okozó hiba
javítása, lásd IV.4).

## II. Kiinduló helyzet (2026-11-12)

**Mérés:** a #1106 ága (`e8b960c`, a `main` `db6016e` fölött), `mypy 2.4.1`, `PySide6 6.12.0`
(a csomag saját típusleíróival), `pytest 9.2.0`, `packaging 26.3`, `opentimelineio 0.18.1`,
Python 3.13. Parancs: `mypy python` (a `pyproject.toml` `[tool.mypy]` strict beállításaival).

| | strict (cél) | alapbeállítás (`--config-file /dev/null`) |
|---|---:|---:|
| Összes hiba | **3451**, 108 fájlban | 1711, 83 fájlban |
| ebből termékkód (`cfa4_devtools`, `cfa4_qt`) | 722 | 621 |
| ebből tesztek | 2729 | 1090 |

Ellenőrzött fájlok: 147. Hibátlan ma: `cfa4_devtools` 10/18, `cfa4_qt` 23/89, `tests` 6/40.

**Területenként (strict):**

| Terület | Fájl / sor | Hiba | Hibás fájl |
|---|---|---:|---:|
| `cfa4_devtools/` | 18 / 4 163 | 30 | 8 |
| `cfa4_qt/*.py` (nem panel) | 24 / – | 124 | 14 |
| `cfa4_qt/panels/` | 65 / – | 568 | 52 |
| `tests/` (a `test_qt_gui.py` nélkül) | 39 / – | 448 | 33 |
| `tests/test_qt_gui.py` | 1 / ~14 800 | **2281** | 1 |

A `cfa4_qt` összesen 89 fájl, 51 017 sor; a `tests` 40 fájl, 20 234 sor.

**Hibakódok (strict, összes):** `no-untyped-def` 836, `union-attr` 811, `no-untyped-call` 569,
`arg-type` 266, `attr-defined` 205, `type-arg` 153, `index` 136, `assignment` 89,
`func-returns-value` 87, `no-any-return` 50, `name-defined` 48, `misc` 47, `method-assign` 33,
`operator` 31, `override` 16, `unused-ignore` 12, egyéb 64.

**Mintázatok** (ezekből áll a hibák túlnyomó része):

1. **Hiányzó annotáció** (`no-untyped-def`, `no-untyped-call`): elsősorban a tesztfüggvényekből
   hiányzik a `-> None`. Gépies, működést nem érintő pótlás.
2. **`None` lehetőség** (`union-attr`, 779 db `Item "None"`; termékkódban 101): a Qt sok
   függvénye `Optional`-t ad vissza (pl. `model()`, `item()`, `parentWidget()`, `QApplication.instance()`).
   Egy részük valódi összeomlási lehetőség, a többinél a hívás helye garantálja a nem-`None` értéket.
   Mindkét esetben kifejezett ellenőrzés vagy `assert` kell, nem elnémítás.
3. **Qt-metódus elfedése** (53 `overloaded function has no attribute ...` találat 12 fájlban): egy
   saját attribútum neve egybeesik egy Qt-metódussal, pl. `self.scroll = QScrollArea(...)` elfedi a
   `QWidget.scroll()`-t. Futáskor működik, de félrevezető és törékeny; átnevezéssel javítható.
4. **Dinamikus attribútumok és alosztályok** (`attr-defined`, pl. `_FrameViewBase`): az alaposztály
   nem deklarálja azt, amit a leszármazottak használnak. `Protocol` vagy deklaráció kell.
5. **`func-returns-value`** (87): `lambda: a() or b()` minták, ahol az `a()` mindig `None`-t ad.
   Működik, de olvashatatlan; függvényre bontással javítható.
6. **Szöveges annotációk a tesztekben** (`name-defined`, pl. `"QtWidgets.QMessageBox"`): import
   nélküli hivatkozás.

A `test_qt_gui.py` egymaga a hibák 66%-a. A Qt GUI-tesztek a Qt6 GUI befejezéséig nem blokkolnak
(coordination.md §3.4), és ezt a fájlt a Qt6-munkamenetek folyamatosan bővítik.

## III. Elvek

1. **Működésváltozás nélkül.** Minden PR csak típust, annotációt, `None`-ellenőrzést, átnevezést
   vagy szerkezeti tisztítást tartalmaz. A GUI viselkedése nem változik. Ha egy javítás mégis
   működést érint (IV.4), az külön commit, külön megjelölve.
2. **Nincs tömeges elnémítás.** Nincs fájlszintű `# type: ignore`, nincs `Any`-áradat.
   Egyedi `# type: ignore[kód]` csak indoklással és hibakóddal, ha a PySide6 típusleírója téved
   (a `warn_unused_ignores` már be van kapcsolva, így a felesleges elnémítás maga is hiba).
3. **A Qt6-munka elsőbbsége.** Ütközés esetén a Qt6 GUI-n dolgozó munkamenet PR-ja megy előre,
   a típusmunka utána igazodik (a 2026-11-09-i tulajdonosi döntés mintájára).
4. **Kis PR-ok, egy téma, fájlcsoportonként** (coordination.md §2.2). Közös fájlt csak a saját
   sorainál érint.
5. **Visszalépés nincs (ratchet).** Ami egyszer hibátlan, az a kapu miatt az is marad.

## IV. Felépítés

### IV.1 A kapu: ratchet modullistával

A `pyproject.toml`-ban a strict beállítás mindenre érvényes. A ma hibás fájlok egy
**kivétellistára** kerülnek:

```toml
[[tool.mypy.overrides]]
# CFA4-PY-TYPING-001: files still to be fixed. Only ever shrinks.
module = ["cfa4_qt.main_window", "cfa4_qt.panels.monitor", ...]
ignore_errors = true
```

- Ami nincs a listán, annak hibátlannak kell lennie. Egy új fájl automatikusan a strict alá esik.
- Egy fájl javítása után a PR a listáról is kiveszi a fájlt.
- Egy CI-ellenőrzés figyeli, hogy a lista csak rövidülhet (a lista mérete nem nőhet a `main`-hez képest).
- A tervvel együtt a fázisok végén a lista üres, és a blokk törölhető.

### IV.2 Hol fut

- Új CI-feladat: **„Python typing (mypy)”**. Telepíti a lockfájlt (`--require-hashes`, benne a
  `mypy` és a PySide6 a típusleírók miatt), majd `python -m mypy python`.
- A futás helye nyitott döntés (D3): a gyors, minden PR-on futó kapuban vagy a teljes validálásban.
- A `CLAUDE.md` Python / Qt sora és a `docs/development/linux.md` kiegészül a
  `.venv/bin/python -m mypy python` paranccsal.

### IV.3 Eszközök a gépies részhez

- A hiányzó `-> None` pótlása a tesztfüggvényekben eszközzel történik (pl. a ruff `ANN201`
  javítása vagy a mypy javaslata alapján), utána ember (munkamenet) nézi át.
- Az ismétlődő Qt-`Optional` mintákra kis segédfüggvény kerülhet (pl. `require(x, "leírás")`,
  amely `None` esetén érthető hibaüzenettel áll meg ott, ahol a `None` programhiba).
  Ahol a `None` jogos állapot, ott elágazás kell, nem `require`.

### IV.4 Valódi hibák

Ha a típusellenőrzés valódi, futás közben összeomló hibát talál (pl. `None`-on hívott metódus egy
ténylegesen elérhető ágon), az:
- külön commitban javul, a commit üzenetében megjelölve,
- a PR leírásában külön listában szerepel,
- a never crash szabály szerint elsőbbséget kap: a tulajdonos dönthet úgy, hogy ezek a javítások a
  fázisoktól függetlenül, előre bekerülnek.

## V. Fázisok

A fázisok sorrendje és időzítése a tulajdonos döntése (VI). Az alábbi számozás a függőségeket
tükrözi, nem ütemezést.

| Fázis | Tartalom | Hiba (2026-11-12) | Függ |
|---|---|---:|---|
| **T0** | Kapu bevezetése: CI-feladat (IV.2), kivétellista a ma hibás 108 fájllal (IV.1), a lista rövidülését figyelő ellenőrzés, dokumentáció (`CLAUDE.md`, `linux.md`). Ettől kezdve a 39 hibátlan fájl és minden új fájl védett. | 0 új | #1106 beolvasztva |
| **T0b** | A gyanús helyek listája a Qt6 tesztelésének: a termékkód azon találatai, amelyek futás közbeni összeomlást jelenthetnek (`None`-on hívott metódus, kb. 100; nem létező Qt-metódus vagy attribútum). **Csak olvasó jelentés, javítás nélkül**, helyenként a fájllal, a sorral, a hibakóddal és azzal a GUI-művelettel, amellyel kipróbálható. Ami a tesztben valóban összeomlik, azt a Qt6-munkamenet javítja a szokásos hibajavításként (D6). | – | – |
| **T1** | `cfa4_devtools/`: 8 fájl | 30 | T0 |
| **T2** | `cfa4_qt/*.py` (nem panel): 14 fájl, köztük `main_window.py` (62), `control.py` (31) | 124 | T0 |
| **T3** | `cfa4_qt/panels/`: 52 fájl. A Qt-metódus-elfedések átnevezése (II.3), a `_FrameViewBase`-hez hasonló alaposztályok deklarálása (II.4), a `None`-ellenőrzések. Fájlcsoportonként külön PR. | 568 | T0, T2 (közös típusok); ütközés a Qt6-munkával (VIII) |
| **T4** | `python/tests/` a `test_qt_gui.py` nélkül: 33 fájl | 448 | T0; részben T1–T3 (a tesztelt kód típusai) |
| **T5** | `python/tests/test_qt_gui.py` | 2281 | T3; a Qt GUI-tesztek blokkoló állapota (coordination.md §3.4) |
| **T6** | Lezárás: a kivétellista törlése, a mypy-kapu a teljes `python/`-ra strict. | 0 | T1–T5 |
| **T7** | Karbantartás: a `mypy` és a PySide6 frissítése (Dependabot `pip` csoport) a kapuval együtt; frissítéskor az új hibák javítása vagy indokolt elnémítása ugyanabban a PR-ban. Opcionálisan a ruff `ANN` szabályai az annotációk meglétére (gyorsabb visszajelzés, mint a mypy). | – | T0 |

A IV.4 szerinti valódi hibák bármelyik fázisban előkerülhetnek; ezekről a tulajdonos külön dönthet.

## VI. Ütemezés

A tulajdonos szabálya szerint (2026-11-11) a terv nem ír elő ütemezést: minden fázis indításáról és
sorrendjéről a tulajdonos dönt, az igények szerint.

**Tulajdonosi döntés (D8, 2026-11-12):** a Qt6 GUI tesztelésre vár, és a tesztelésnek stabil kód
kell. Ezért:
- **most:** T0 (a kapu a kivétellistával; a Qt-fájlokhoz nem nyúl) és T0b (a gyanús helyek listája
  a tesztelőknek, javítás nélkül);
- **a Qt6 tesztelése után:** T2–T5 (a `cfa4_qt`, a panelek és a tesztek javítása);
- a T1 (`cfa4_devtools`, Qt-független) ütközés nélkül bármikor mehet; az indításáról a tulajdonos
  dönt;
- a tesztelés közben talált valódi összeomlásokat a Qt6-munkamenet javítja (D6).

Amit a terv a döntéshez jelezett:
- A **T0** önállóan, kis kockázattal elvégezhető, és azonnal megvédi a már hibátlan fájlokat.
- A **T3** és a **T5** a Qt6-munkával érintkezik (a `cfa4_qt/panels/` és a `test_qt_gui.py` a
  Qt6-munkamenetek fő fájljai). Ha a Qt6 GUI befejezése előtt indulnak, sok ütközés várható; utána
  alig.
- A nyitott PR-ok száma ma 30 fölött van (coordination.md §2.6); új PR-t a tulajdonos kérésére
  nyit a munkamenet.

## VII. Döntések (2026-11-12, mind eldöntve)

| # | Kérdés | Döntés (a tulajdonos 2026-11-12-én a javaslatokat jóváhagyta) |
|---|---|---|
| D1 | Hogyan kezeljük a ma hibás fájlokat a kapu bevezetésekor? (a) fájlonkénti kivétellista a `pyproject.toml`-ban (IV.1); (b) külső „baseline” eszköz (pl. `mypy-baseline`), amely a meglévő hibák listáját tárolja; (c) először az alapbeállítás (1711 hiba), csak később strict. | **(a)**: nincs új függőség, a lista a `pyproject.toml`-ban jól látható és csak rövidülhet. |
| D2 | A tesztekre is a teljes strict vonatkozzon? | **Igen.** A tesztekben a hibák nagy része a hiányzó `-> None` (gépies pótlás). Enyhített teszt-beállítás nem kell. |
| D3 | Hol fusson a mypy-kapu? (a) a gyors kapuban (`cfa4-gate`), minden PR-on; (b) a teljes validálásban (`ci-linux.yml` „Python and Qt 6”), csak a `cfa4-full-validation` címkére. | **(a)**: minden PR-on visszajelzés. Ehhez a lockfájl (benne a PySide6) telepítése kell a kapuban, gyorsítótárral. |
| D4 | A Qt-metódusokat elfedő attribútumok (pl. `self.scroll`) átnevezhetők-e a termékkódban? | **Igen**, fájlonként külön PR-ban, a vezérlőcsatorna automatizálási azonosítóinak (`setObjectName`) változatlanul hagyásával. |
| D5 | Melyik munkamenet végzi? | Saját, külön munkamenet (a többi tervhez hasonlóan), amely csak ezen a terven dolgozik, és minden PR-t a gazdának ad át. |
| D6 | A IV.4 szerinti valódi hibák javítása megvárja-e a fázist? | **Nem kell megvárnia**: a never crash szabály miatt a talált összeomlási hibák külön, előre bekerülhetnek. |
| D7 | A T0 kapuja a Qt6 tesztelése alatt blokkoló legyen, vagy csak jelentést adjon (és a teszt után váljon blokkolóvá)? | **Blokkoló.** A ma hibás fájlok a kivétellistán vannak, így a Qt6-javításokat nem akadályozza; csak az új fájloknak kell hibátlannak lenniük. |
| D8 | Mikor javítsunk: most vagy a Qt6 tesztelése után? | **Eldöntve (tulajdonos, 2026-11-12):** most T0 és T0b, a T2–T5 a Qt6 tesztelése után (VI). |

## VIII. Kockázatok

| Kockázat | Kezelés |
|---|---|
| Ütközés a Qt6-munkamenetekkel (`cfa4_qt/panels/`, `test_qt_gui.py`) | III.3: a Qt6 PR megy előre; fájlcsoportonként kis PR-ok; a T3/T5 időzítése a tulajdonos döntése (VI) |
| Egy `None`-ellenőrzés észrevétlenül megváltoztatja a működést (pl. csendben kihagy egy lépést) | III.1; ahol a `None` programhiba, ott `require` (érthető hibaüzenet), nem csendes kihagyás; a PR-ban fel kell sorolni az ilyen helyeket |
| A PySide6 típusleírói pontatlanok | egyedi `# type: ignore[kód]` indoklással (III.2); ha ismétlődik, jelzés a PySide6 felé |
| Egy `mypy`- vagy PySide6-frissítés új hibákat hoz | T7: a frissítés PR-ja javítja őket; a verziók a lockfájlban rögzítettek |
| A kapu lassítja a CI-t (PySide6 telepítése) | D3; pip-gyorsítótár; a mypy saját gyorsítótára (`.mypy_cache`) a CI-ben |
| A kivétellista visszanő (valaki új fájlt vesz fel rá) | IV.1: CI-ellenőrzés, hogy a lista nem nőhet |

## IX. Mérés megismétlése

```bash
.venv/bin/python -m pip install --require-hashes -r requirements/linux-amd64.txt
.venv/bin/python -m pip install --no-deps -e .
.venv/bin/python -m mypy python                       # strict (pyproject.toml)
cd /tmp && .venv/bin/python -m mypy <repo>/python --config-file /dev/null   # alapbeállítás
```

A II. fejezet számai a #1106 ágán mértek; a #1106 beolvasztása után a `main`-en ugyanezek a
parancsok adják az aktuális értéket. A T0 PR-ja a mérést frissíti.
