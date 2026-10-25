# CFA4 stáblista és feliratkészítő – módosított terv (javaslat)

**Állapot:** JAVASLAT – a tulajdonos (`ubuntuokos`) jóváhagyására vár  
**Dátum:** 2026-10-24  
**Forrás:** az „FA3 Credits & Titles Composer – végleges materializációs és végrehajtási terv”,
átdolgozva a CFA4 valós állapotára, a [`VISION.md`](../VISION.md) prioritásaira és a
[GPL-3.0-or-later licencre](licensing.md).  
**Kapcsolódó:** [ADR F10](adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md), [credits.md](credits.md), [ADR 0003](adr/ADR-0003-aswf-and-conversion-sources.md) (GPL-3 források)

## 1. Miért kell módosítani

Az eredeti terv az FA3 (`Final-Architecture-v3.0`) környezetére készült. Olyan rendszerekre
épít, amelyek a CFA4-ben nincsenek meg: Story/Screenplay alkalmazás, Live Studio, Show/Rundown,
HRB, Model Router, Canonical Gate, Evidence Registry. A CFA4 egyetlen Qt6-os vágóprogram
Ubuntura, amely a felsorolt programok *képességeit* egyesíti, és saját Rust médiamotort
használ.

A módosítás elve:

- **ami a CFA4-ben értelmes, az megmarad**, a meglévő motorra építve, nem párhuzamosan;
- **ami külön alkalmazást vagy nem létező rendszert feltételez, az halasztott**, és csak a
  tulajdonos döntése után kerül elő (lásd 8. pont);
- a „kész” fogalma a CFA4 szabálya szerint értendő: automata teszt, renderelt képkocka és
  valódi gépes mérés. Csak dokumentáció vagy makett nem elég (`VISION.md` 2.4).

## 2. Kiinduló állapot (2026-10-24)

Elkészült és a `main`-en van (#36), illetve a `cfa4/credits-2` ágon:

| Terület | Állapot |
|---|---|
| Bemenet: CSV, TSV, `.xlsx`, `.ods`, munkalap-választás | kész |
| Blokkok (szerep / nevek / láb), térközök, tartalom- és oldalstílusok, magyar oszlopnevek | kész |
| Gördülő oldal, álló kártya, kártyasor, vegyes; rács, szerep–név, középre zárt, felsorolás, bekezdés | kész |
| Képek és logók a stáblistában, a gördüléssel pontosan együtt mozogva | kész |
| Futásidő elérése a térközök nyújtásával, állandó sebességgel | kész (`cfa4/credits-2`) |
| Eredmény: szerkeszthető grafikus klipek az idővonalon, egy visszavonási lépés | kész |
| Gépi vezérlés: `graphics.credits.import` / `.preview` / `.styles` | kész |
| Renderelés: a szerkesztő normál exportjával | kész |

## 3. Alapdöntések a módosított tervben

1. **Egy közös mag, ahogy az eredeti terv is kéri:** a `crates/credits` crate a mag (adat,
   elrendezés, ellenőrzés), az `engine` a parancsok rétege. Minden felület (egui menü, később
   Qt6, CLI, MCP) ugyanazokat a parancsokat hívja. Nincs külön stáblista-motor.
2. **Nincs külön önálló alkalmazás az első körben.** A „Credits & Titles Composer” a CFA4-en
   belüli **munkaterület** lesz (Qt6), nem külön program. Így nincs második telepítendő,
   frissítendő és együttélési szempontból vizsgálandó bináris.
3. **Nincs saját hangrendszer.** A zene a CFA4 normál idővonalán és keverőjén van. Az eredeti
   terv §11-e itt duplikáció lenne; a §12 (igazítás a zenéhez) viszont megmarad.
4. **A „projektformátum” a CFA4-projekt része.** Külön `.fa3credits` helyett a stáblista
   *forrása* (táblázat elérési útja, munkalap, stílus, ujjlenyomat) a projektfájlban tárolódik a
   létrehozott klipekkel. Külön exportálható fájl (`.cfa4credits`) csak akkor kell, ha a
   tulajdonos önálló alkalmazást is kér (8. pont).
5. **Licenc:** a Cinecred (GPL-3.0-or-later) kódja a GPL-3-váltás óta felhasználható,
   fejléc-megőrzéssel és a [licensing.md §5](licensing.md) nyilvántartásba vétellel. Ahol
   egyszerűbb, saját megvalósítás készül (ahogy eddig).

## 4. Fázisok

Méret: **K** = kicsi (1–2 nap), **K+** = közepes, **N** = nagy.
Minden fázis saját PR, saját tesztekkel és – ahol látható eredmény van – renderelt képkockával.

### C1 – Kapcsolt stáblista és újratöltés (K+)

Az eredeti terv §3 („nincs csendes felülírás”) elve a táblázatra alkalmazva.

- A létrehozott oldalak megjegyzik a forrásukat: fájl, munkalap, stílus, tartalom-ujjlenyomat.
- Új parancs: `graphics.credits.refresh`. Ha a táblázat változott, a program **eltérés-listát**
  mutat (új, törölt, módosult sor), és soronként dönteni lehet: *elfogad*, *elvet*, *megtartja a
  kézi módosítást*.
- A kézzel átírt szövegréteg „kézi módosításként” jelölődik, és újratöltéskor nem íródik felül
  rákérdezés nélkül.
- **Kész, ha:** teszt igazolja, hogy a módosított táblázat csak a változott blokkokat érinti, a
  kézi módosítás megmarad, és az egész egy visszavonási lépés.

### C2 – Ellenőrzések (K)

Az eredeti terv §20-ából a CFA4-ben értelmesek, géppel olvasható (JSON) eredménnyel:

- hiányzó vagy duplikált név, eltérően írt név (ugyanaz a személy két alakban);
- hiányzó kép, hiányzó betűtípus, kis felbontású kép (felnagyítás);
- **safe area** megsértése (címbiztos és akcióbiztos terület);
- **elégtelen kontraszt** a háttérhez képest;
- **túl gyors vagy vibráló gördülés** (nem egész px/képkocka, vagy a sorok olvashatósági határa
  felett);
- időzítési ütközés (átfedő oldalak, kártya rövidebb a beúszás és a kiúszás összegénél).

**Kész, ha:** minden ellenőrzéshez van pozitív és negatív teszt, és az előnézet mutatja a
figyelmeztetéseket.

### C3 – Igazítás a zenéhez (K → K+)

Az eredeti terv §12-e.

- Első lépés: a kijelölt hangklip (vagy idővonal-tartomány) hossza adja a stáblista teljes
  futásidejét. A gördülő oldalak a térközök nyújtásával igazodnak, a kártyák hossza marad.
- Második lépés: **javaslat** csendekre, hangerő-csúcsokra és ütemre (a meglévő `audio-dsp`
  crate-tel): hol kezdődjön a kártya, hol legyen a kiúszás. **Csak javaslat, jóváhagyás nélkül
  nem módosít** – ahogy az eredeti terv is kéri.

### C4 – Háttér, átmenet, futószöveg, segédvonalak (K+)

Az eredeti terv §6, §7, §10 pontjaiból:

- oldalanként **háttér**: átlátszó, szín, színátmenet, kép, videó (kitöltés/illesztés,
  átlátszóság, elmosás);
- oldalak közti **átmenet** (áttűnés, eltolás) a meglévő átmenet-rendszerrel;
- **vízszintes futószöveg** (crawl) oldaltípus – a motor már tudja;
- **segédvonalak** az előnézetben: safe area, gerinc, alapvonal-rács.

### C5 – Gazdagabb elrendezés (N, részekre bontva)

- pontsor a szerep és a név között, egyforma szélességű szerep-oszlop;
- betűstílus soron belül (`{{Stílus}}…{{Style}}`);
- párhuzamos oszlopok (gerincek);
- **portré + név rács** (fotók a nevek fölött vagy mellett);
- **videó** a stáblistában (egy oldalon belül, a gördüléssel együtt mozogva);
- egyszerű mozgás-előbeállítások kártyákra: beúszás oldalról, lépcsőzetes (stagger) megjelenés.

### C6 – Stáblista-sablonok (K+)

Az eredeti terv §13-a, a meglévő `.fcgt` sablonrendszer mintájára.

- Egy sablon: stílusok + oldalszerkezet + hátterek + időzítés. Kategóriák: beépített,
  felhasználói, projekt.
- Verziózott, azonosított, a forrását és a licencét tárolja.
- A beépített sablonok saját tervezésűek, és a tulajdonos döntése szerint MIT/Apache licencűek maradnak.

### C7 – Cinecred-import (K+)

Az eredeti terv §4-éből („Cinecred-kompatibilis import”).

- Egy Cinecred-projektmappa (táblázat + stílusfájl) beolvasása CFA4-stílusokká.
- A Cinecred fájlformátumának leírását és szükség esetén kódját a licensing.md §5 szerint
  nyilvántartásba vesszük.
- Export Cinecred-formátumba csak akkor, ha igény van rá.

### C8 – Qt6 stáblista-munkaterület (N)

Az eredeti terv §14–16-a, a CFA4 Qt6-ütemezéséhez igazítva (`VISION.md` 6.1: előbb a Qt6
alapjai – háttérszálak, médiaböngésző, idővonal).

- Elrendezés az eredeti terv szerint: bal oldalon oldalak és blokkok, középen előnézet, jobb
  oldalon tulajdonságok, alul idővonal.
- **Kontextusfüggő felület** (§16): a vágóprogramból megnyitva csak a vágáshoz szükséges
  funkciók látszanak; a teljes nézet egy gombbal érhető el.
- Első lépésként a csak olvasó `graphics.credits.preview` és `.styles` kerül a Qt6-híd
  engedélyezett parancsai közé; a módosító parancsok a híd tranzakciós szabályainak
  elkészülte után.

### C9 – Csomagolás és együttélés (K)

Az eredeti terv §23–24-e, a CFA4 csomagolására szabva.

- A stáblista-modul nem hoz létre saját binárist, szolgáltatást, portot vagy MIME-típust,
  ezért ütközést sem okozhat; ezt a csomag-ellenőrzés rögzíti.
- Ha C7 után saját fájltípus lesz, CFA4-névtérben (`application/x-cfa4-credits`,
  `io.github.ubuntuokos.cfa4`) – Cinecred fájltársítását nem veszi át.
- Ellenőrzés: Cinecred és CFA4 ugyanazon a gépen, egyszerre futtatva, külön beállításokkal;
  eltávolítás után a másik működik.

## 5. Sorrend és függőségek

```text
C1 kapcsolt stáblista ─┬─> C2 ellenőrzések ─> C3 zene
                       └─> C6 sablonok ─> C7 Cinecred-import
C4 háttér / átmenet ───> C5 gazdagabb elrendezés
Qt6 alapok (F8) ───────> C8 Qt6 munkaterület
C9 együttélés: C7 után, illetve minden kiadás előtt
```

Javasolt sorrend: **C1 → C2 → C3 → C4 → C5 → C6 → C7**, a C8 a Qt6-alapok elkészültekor.

## 6. Kész-feltételek (az eredeti §32 helyett)

Egy fázis akkor kész, ha:

- a `filmcraft-credits` és az engine tesztjei lefedik (beleértve az ellenséges bemenetet:
  sérült táblázat, hiányzó fájl, abszurd számok – soha nem omlik össze);
- a látható eredményről renderelt képkocka készült, és ellenőrizve lett;
- a dokumentáció ([credits.md](credits.md), ADR F10) frissült;
- a kapuk zöldek: `fmt`, `clippy -D warnings`, tesztek, `xtask layers`, `xtask assets`,
  `cargo deny check licenses`.

A teljes stáblista-képesség akkor kész, ha C1–C7 és C8 kész, és egy valódi Ubuntu 26.04
gépen (Kubuntu referenciagép) végigvitt munkamenet igazolja: táblázat → stáblista → módosítás
→ újratöltés → zenéhez igazítás → export.

## 7. Az eredeti tervből kihagyott vagy halasztott részek

| Eredeti pont | Döntés | Indok |
|---|---|---|
| §3 Story/Screenplay híd | halasztva | a CFA4-ben nincs forgatókönyv-alkalmazás; a táblázat tölti be ezt a szerepet |
| §11 saját hangrendszer | kihagyva | a CFA4 idővonala és keverője ezt már tudja |
| §14 önálló alkalmazás | halasztva | munkaterület a CFA4-en belül (C8); lásd 8. pont |
| §17–19 OBS, élő adás, Show/Rundown | halasztva | a `VISION.md` nem tartalmaz élő adást |
| §21, §25, §32 HRB, Model Router, Canonical Gate, Evidence Registry, visszamenőleges audit | kihagyva | FA3-intézmények; a CFA4-ben a tesztek, ADR-ek és a kiadási kapuk (F9) felelnek meg nekik |
| §2 teljes `.fa3credits` séma (személyek, cégek, jogi adatok, revíziók) | szűkítve | C1 kapcsolt forrás; a teljes produkciós adatmodell csak Story-híddal lenne értelmes |

## 8. Döntések a tulajdonosnak

1. **Kell-e valaha önálló stáblista-alkalmazás**, vagy elég a CFA4-en belüli munkaterület?
2. **Kerüljön-e be az élő adás (OBS, élő grafika)** a CFA4 céljai közé? Ez a `VISION.md`
   módosítását jelentené.
3. **Külön `.cfa4credits` fájl** kell-e (például más gépre vinni), vagy elég, ha a stáblista a
   projektfájlban él?
4. **Elfogadod-e a C1 → C7 sorrendet**, vagy előrébb hozzunk valamit (például a Cinecred-importot
   vagy a zenéhez igazítást)?
