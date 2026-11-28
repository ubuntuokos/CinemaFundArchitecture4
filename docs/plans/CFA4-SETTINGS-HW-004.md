# CFA4-SETTINGS-HW-004 – Kijelzők és Tárolóeszközök oldal a Hardver és eszközök csoportban

**Állapot:** FINAL 1.0 – a tulajdonos 2026-11-28-án jóváhagyta; a D1–D4 a javaslat szerint. A terv minden
további módosítása után a munka a tulajdonos újabb jóváhagyásáig áll
([`docs/coordination.md`](../coordination.md) §2.10).
**Dátum:** 2026-11-28 (véglegesítve 2026-11-28) · **Tulajdonos:** ubuntuokos
**Projekt:** „CFA4 hardver beállítása”.
**Előzmény:** [`CFA4-SETTINGS-HW-003.md`](CFA4-SETTINGS-HW-003.md) (Áttekintés és eszközoldal-minta, R4);
menüterv: [`CFA4-SETTINGS-MENU-001.md`](CFA4-SETTINGS-MENU-001.md) „Hardver és eszközök” és „Ütemezés”
5. szakasz. A menüterv 4. szakasza (GPU Unlock) a HW-003 D6 döntése szerint kimarad.
**Feltétel:** a HW-003 PR-jai (#1932, #1933, #1936, #1939) előbb kerüljenek be, mert ez a terv a
`device_page.py` eszközoldal-mintára és az Áttekintés oldalra épül.

## 1. Kiinduló állapot

- A `cfa4-qt` Beállítások ablakában a *Hardver és eszközök* csoportban ma: Áttekintés, Gyorsítók
  (GPU / NPU), Rendszerbeállítások (root), Vezérlőfelületek, Hangeszközök, Memória.
- A menüterv „később” oldalai közül egyik sincs meg: Kijelzők, Tárolóeszközök, Kamerák és
  kártyaolvasók, Rajztáblák, MIDI és vezérlőfelületek.
- Az eszközoldal-minta (`python/cfa4_qt/panels/device_page.py`: lista balra, fülek jobbra: Adatok,
  Képességek, Meghajtók, Diagnosztika) kész, eddig csak a Gyorsítók oldal használja.
- A meglévő beállítások közül ide tartozik: *Színkezelés* (`color.displayColorManagement`,
  `color.extendedDynamicRange`, `color.hdrGraphicsWhite`) és *Médiagyorsítótár* (`mediaCache.location`,
  `mediaCache.databaseLocation`, a méret- és korkorlát). Ezek ma más oldalakon vannak.

## 2. Hatókör

Javaslat: a „később” oldalak közül először a **Kijelzők** és a **Tárolóeszközök** készüljön el, mert
root nélkül, a referenciagépen teljesen tesztelhetők, és a vágóprogram napi használatához (lejátszás
képernyője, szabad hely a gyorsítótárnak) ezek kellenek leginkább. A Kamerák, Rajztáblák és MIDI
külön tervet kap (D1).

- R1: Kijelzők oldal (csak olvas, plusz hivatkozás a Színkezelés beállításaira).
- R2: Tárolóeszközök oldal (csak olvas, jelzi, melyik CFA4-hely melyik lemezen van, és ha kevés a hely).
- R3: Áttekintés oldal két új sorral.
- R4 (D2-től függ): új beállítások: teljes képernyős lejátszás kijelzője, alacsony szabad hely
  figyelmeztetési küszöbe.

Nem része: root-művelet, új csomag, kijelző- vagy lemezbeállítás módosítása a rendszerben (a CFA4 csak
a saját beállításait írja).

## 3. R1 – Kijelzők oldal

- **Felderítés:** a Qt `QGuiApplication.screens()` listája (Wayland és X11 alatt is működik), ahogy a
  Hangeszközök oldal is a Qt eszközlistáját használja; a motortól nem kér újat. Változáskor
  (`screenAdded`, `screenRemoved`) a lista frissül.
- **Lista:** név, gyártó és típus, felbontás, frissítési frekvencia; az elsődleges kijelző jelölve.
- **Fülek:**
  - *Adatok:* név, gyártó, típus, sorozatszám (ha a rendszer megadja), felbontás, fizikai méret,
    képpontsűrűség, skálázás (device pixel ratio), frissítési frekvencia, helyzet az asztalon.
  - *Képességek:* a CFA4 szemével: alkalmas-e teljes képernyős lejátszásra, a skálázás miatt hány
    képpontos a forrásmonitor egy képpontja, és hogy a Színkezelés be van-e kapcsolva. A HDR-képességet
    csak akkor írja ki, ha a Qt megbízhatóan megadja; különben „nem ismert”.
  - *Beállítások:* hivatkozás a Színkezelés oldalra; D2 = igen esetén itt a „Teljes képernyős lejátszás
    kijelzője” választó.
  - *Diagnosztika:* időbélyeges napló a felismerésekről.
- Object nevek: `settings.displays.page`, `settings.displays.page.list`, `settings.displays.page.tab.<kulcs>`.

## 4. R2 – Tárolóeszközök oldal

- **Felderítés:** a Qt `QStorageInfo.mountedVolumes()` listája; a pszeudo-fájlrendszereket (`proc`,
  `sysfs`, `tmpfs`, `overlay` stb.) kihagyja. A motortól nem kér újat; a CFA4-helyeket a meglévő
  beállításokból (`prefs.get`) olvassa.
- **Lista:** csatolási pont, eszköz, fájlrendszer, szabad és teljes méret; jelölés, ha csak olvasható,
  és ha rajta van valamelyik CFA4-hely.
- **Fülek:**
  - *Adatok:* csatolási pont, eszköz, fájlrendszer típusa, méret, szabad hely, csak olvasható-e.
  - *Képességek:* mely CFA4-helyek vannak itt (médiagyorsítótár, gyorsítótár-adatbázis, automatikus
    mentés, a nyitott projekt mappája), és ha a szabad hely a küszöb alatt van, figyelmeztetés.
  - *Beállítások:* hivatkozás a Médiagyorsítótár oldalra; D2 = igen esetén a figyelmeztetési küszöb.
  - *Diagnosztika:* időbélyeges napló.
- Figyelmeztetés alapértéke (amíg nincs saját beállítás): 10 GB vagy 5% alatt, amelyik nagyobb.
- Object nevek: `settings.storage.page`, `settings.storage.page.list`, `settings.storage.page.tab.<kulcs>`.

## 5. R3 – Áttekintés

Két új sor: *Kijelzők* („2 kijelző, elsődleges: …”) és *Tárolóeszközök* („a médiagyorsítótár lemezén
X GB szabad”), figyelmeztetéssel, ha a gyorsítótár lemeze a küszöb alatt van vagy csak olvasható.

## 6. R4 – új beállítások (csak ha D2 = igen)

- `displays.fullscreenPlayback`: a teljes képernyős lejátszás kijelzője (Auto / a kijelző neve). A
  Qt-szerkesztő teljes képernyős lejátszása ezt használja; ha a kijelző nincs csatlakoztatva, Auto.
- `storage.lowSpaceGb`: figyelmeztetési küszöb (GB).
- A motor beállítás-sémájába (`crates/engine/src/settings.rs`) új kategória-mezőként kerülnek, a meglévő
  `prefs.schema` / `prefs.set` úton; a motor tesztjei (`settings_tests.rs`) bővülnek.

## 7. Sorrend és PR-ok

| PR | Tartalom | Root-művelet |
|---|---|---|
| E | R1 + R2 + R3 (két oldal és az Áttekintés), Qt-oldal és tesztek | nem |
| F | R4 (motor-séma + a két választó), ha D2 = igen | nem |

Egyik sem kockázatos a §2.9 szerint, ezért CodeRabbit-ellenőrzést nem kér. Az új szövegek mind a 32
nyelvre lefordítva kerülnek be (a fordítási lefedettség-ellenőrzés miatt).

## 8. Bizonyíték és mérés

- Python-tesztek (`python/tests/test_qt_settings_hardware.py`) hamis kijelző- és tárolólistával: lista,
  fülek, figyelmeztetés a küszöb alatt, pszeudo-fájlrendszerek kihagyása, Áttekintés-sorok.
- Képernyőkép a két oldalról `--smoke --screenshot`-tal (offscreen), a projekt `settings-menu/` mappájába.
- A referenciagépen ([`docs/development/reference-machine.md`](../development/reference-machine.md)) a
  valódi kijelzőlista és lemezlista ellenőrzése a deferred listába kerül
  ([`docs/development/deferred-physical-tests.md`](../development/deferred-physical-tests.md)).

## 9. Döntések (eldöntve 2026-11-28, a javaslat szerint)

| # | Kérdés | Döntés |
|---|---|---|
| D1 | Ez a két oldal (Kijelzők, Tárolóeszközök) jöjjön először a „később” oldalak közül? | Igen; a Kamerák és kártyaolvasók, Rajztáblák, MIDI külön tervben, ebben a sorrendben. |
| D2 | Kerüljön-e be már most a két új beállítás (teljes képernyős lejátszás kijelzője, szabadhely-küszöb)? | Igen, külön PR-ban (F), hogy az E csak olvasó maradjon. |
| D3 | Rendben van-e, hogy a felderítés a Qt-ból jön (mint a Hangeszközöknél), nem a motorból? | Igen; a motornak ehhez nincs saját adata, és a Qt mindkét kijelzőszerveren működik. |
| D4 | A Színkezelés és a Médiagyorsítótár beállításai maradjanak-e a mai helyükön (csak hivatkozással)? | Igen; az áthelyezés a megszokott Premiere-elrendezést bontaná meg. |
