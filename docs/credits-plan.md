# CFA4 stáblista és feliratkészítő – módosított terv (javaslat)

**Állapot:** JÓVÁHAGYVA (2026-10-25) – mind a négy döntés meghozva; C1, C2, C3 kész  
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
   elrendezés, ellenőrzés), az `engine` a parancsok rétege. Minden felület (Qt6, CLI, MCP)
   ugyanazokat a parancsokat hívja. Nincs külön stáblista-motor.
2. **Csak Qt6 (tulajdonosi döntés, 2026-10-25: „már csak qt6 legyen”).** Nincs önálló
   stáblista-alkalmazás: a „Credits & Titles Composer” a CFA4 Qt6-felületén belüli ablak és
   munkaterület. Az egui-ba nem kerül több stáblista-funkció (az egui be van fagyasztva, és a
   Qt6-paritás után törlődik; a meglévő egui-menüpont addig marad). Így nincs második
   telepítendő, frissítendő és együttélési szempontból vizsgálandó bináris.
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

### C1 – Kapcsolt stáblista, stáblista-fájl és frissítés (K+) – **KÉSZ (2026-10-25)**

Az eredeti terv §3 („nincs csendes felülírás”) elve a táblázatra alkalmazva, a tulajdonos 3.
döntésével (külön stáblista-fájl kell) együtt.

- **`.cfa4credits` stáblista-fájl:** verziózott JSON: forrás (útvonal, fájlnév, munkalap),
  képmappa, stílus és **a táblázat sorai beágyazva**, így más gépen a táblázat nélkül is
  megnyílik. Mentés: a Stáblista ablak *Mentés stáblista-fájlként…* gombja vagy
  `graphics.credits.save`; megnyitás ugyanúgy, mint egy táblázaté.
- **Kapcsolat:** minden elhelyezett oldal megjegyzi a stáblista-dokumentumot (a legutóbb
  elfogadott sorokkal), a hozzá tartozó képklipeket és minden általa készített réteg
  ujjlenyomatát.
- **Eltérés soronként:** `graphics.credits.diff` – új, törölt és módosult sorok (előtte / utána),
  és a kézzel módosított rétegek; jelzi, melyik kézi módosítás veszne el, ha minden változást
  elfogadnál.
- **Frissítés:** *Grafika és címek ▸ Stáblista frissítése a táblázatból…*
  (`graphics.credits.refresh`): soronkénti pipa, *a saját módosításaim maradjanak meg*. A ki nem
  pipált változás marad a régi, és a következő összevetésnél újra felkínálódik. Az oldalak a
  helyükön cserélődnek, **egy visszavonási lépésben**. Ha a táblázat elköltözött, a *Táblázat
  megkeresése…* gombbal megadható az új helye.
- **Igazolva:** crate-tesztek (soronkénti diff, szelektív összefésülés, ellenséges táblák,
  dokumentum be/ki, stabil rétegkulcsok), engine-tesztek (kézi módosítás megtartása, egy
  visszavonás, táblázat nélküli megnyitás, elköltözött forrás), Qt-teszt valódi motorral
  (mentés → elhelyezés → táblázat módosítása → frissítés részleges elfogadással).

### C2 – Ellenőrzések (K) – **KÉSZ (2026-10-26)**

Az eredeti terv §20-ából a CFA4-ben értelmesek, géppel olvasható eredménnyel
(`graphics.credits.validate`; stabil kód, súlyosság, oldal/sor, értékek):

- kétszer szereplő név egy blokkban; **ugyanaz a személy kétféle írásmóddal** (ékezet,
  kis-nagybetű, szóköz, írásjel);
- **nem telepített betűtípus** (hiba) és **betűtípusból hiányzó karakterek** (pl. ő, ű) – a
  valódi szövegmotor dönti el;
- szöveg a **címbiztos területen** kívül (5 % a széleken; gördülésnél csak oldalt);
- **kontraszt** fekete alapon 4,5:1 alatt (WCAG 2.1); 24 px alatti szöveg;
- **vibráló gördülés** (nem egész px/képkocka) és **túl gyors gördülés** (egy sor 6 mp alatt fut
  át);
- 2 mp-nél rövidebb kártya az áttűnések után; átfedő oldalak;
- a képpontjainál nagyobban mutatott kép (elmosódott).

A Qt6 Stáblista ablak súlyosság szerinti ikonnal, magyarul, sorszámmal mutatja; a mintatáblázat
hamis riasztás nélkül átmegy. A kontraszt a C4 óta a valódi háttérszínhez mér.

### C3 – Igazítás a zenéhez (K → K+) – **KÉSZ (2026-10-26)**

Az eredeti terv §12-e.

- **Grafika és címek ▸ Stáblista igazítása a zenéhez…**: a zene a kijelölt hangklip, vagy ami a
  stáblistával a legjobban átfed. Az ablak megmutatja a zene hosszát, hogy hol ér véget a hang,
  a tempót és a stáblista igazítás nélküli hosszát, és **csak javasol**: a teljes zene,
  ameddig a hang tart, az utolsó ütemhatárig (a Remix ütemfelismerésével), vagy kézzel megadott
  hossz. Jóváhagyás (*Igazítás erre a hosszra*) nélkül semmi nem változik.
- Igazításkor a kártyák, az oldalközök és a saját játékidejű oldalak hossza marad; a gördülő
  oldalak a természetes hosszuk arányában osztoznak a maradékon, a blokkok közti térközök
  nyújtásával (állandó sebességgel). Az eredmény képkockára pontos.
- *A kártyák egész ütemig tartanak* (vágás ütemre); *a stáblista a zene kezdetén induljon*.
- Az igazítás a stáblista-dokumentumban tárolódik (`fit`), így a táblázatból frissítés
  megtartja; *Igazítás megszüntetése* visszavonja. Egy visszavonási lépés, a kézi módosítások
  megmaradnak.

### C4 – Háttér, átmenet, futószöveg, segédvonalak (K+) – **KÉSZ (2026-10-26)**

Az eredeti terv §6, §7, §10 pontjaiból:

- oldalstílusonként **háttér**: nincs (átlátszik), szín, színátmenet, kép, videó
  (kitöltés/illesztés, átlátszóság, elmosás). Saját klip közvetlenül az oldal alatt, az oldallal
  azonos hosszal; nem gördül, a beállításai a klipen szerkeszthetők; frissítéskor az oldallal
  együtt cserélődik;
- oldalak közti **átmenet** (áttűnés, eltolás, csúsztatás, törlés, áttűnés feketén át, vagy
  bármely átmenet azonosítója) a meglévő átmenet-rendszerrel, a vágásra középre; a hátterek is
  megkapják; kártyán a vágásnál az áttűnés helyére lép. Csak egymáshoz érő oldalak közt;
- **vízszintes futószöveg** (`Crawl`) oldaltípus: a blokkok egy sorban, jobbról balra, a képek
  vele futnak;
- **segédvonalak** az előnézetben: akcióbiztos és címbiztos terület, középvonalak, gerincek,
  alapvonal-rács (az oldallal mozog);
- a kontraszt-ellenőrzés a valódi háttérszínhez mér; kép vagy videó mögött tájékoztat, hogy
  szemmel kell ellenőrizni.

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

Az eredeti terv §14–16-a. A Qt6-felület (#48 → #50 → #54) már a motor összes parancsát eléri,
ezért a C8 **első része elkészült** (`cfa4/qt6-credits`):

- **Stáblista ablak** (Grafika és címek ▸ Stáblista importálása táblázatból…): táblázat,
  munkalap, képmappa és stílusfájl választása; az oldalak listája típussal és hosszal,
  összidő, figyelmeztetések soronként; az oldal **előnézeti képe** egy csúszkával választott
  időpontban (`graphics.credits.thumbnail`); elhelyezés a lejátszófejnél egy visszavonási
  lépésben. Nem modális, magyarul.

Hátravan:

- Elrendezés az eredeti terv szerint: bal oldalon oldalak és blokkok, középen előnézet, jobb
  oldalon tulajdonságok, alul idővonal.
- **Kontextusfüggő felület** (§16): a vágóprogramból megnyitva csak a vágáshoz szükséges
  funkciók látszanak; a teljes nézet egy gombbal érhető el.
- a C1–C6 új képességeinek felülete (újratöltés eltérés-listával, ellenőrzések, zenéhez
  igazítás, hátterek, sablonok) ebben az ablakban, illetve dokkolható panelben.

### C10 – Élő grafika (tervezet)

A tulajdonos döntése (2026-10-25): **az élő adás kell, de egyelőre tervezet** (`VISION.md` 4. és
8. pont, #60). Az eredeti terv §17–19-éből ez a fázis vázlata; ütemezése és mélysége külön döntés.

- **Élő grafikai futtató:** ugyanaz a stáblista- és grafikamag (`crates/credits`, grafikus
  klipek), élő kimenetre renderelve; nincs külön grafikai motor.
- **Vezérlés:** előre felkészített elemek (cue-k) betöltése, előnézet (PREVIEW), indítás (TAKE),
  léptetés (NEXT / PREVIOUS), megállítás (HOLD / CONTINUE), adatfrissítés (UPDATE), levétel
  (OUT); mind motorparancs, így Qt6-ból, CLI-ből és gépi vezérléssel is elérhető.
- **OBS-kapcsolat:** külső adáskezelőként, első lépésben Browser Source-on át (helyi kiszolgáló,
  csak a gépen elérhető), vezérlés obs-websocketen. Az OBS nem irányítja a CFA4-et.
- **Hang:** három mód: beágyazott hang használata, külön hangforrásként kiadva, figyelmen kívül
  hagyva.
- **Műsorterv (rundown)** csak akkor, ha a tulajdonos a mélységről így dönt (`VISION.md` 8. pont).
- **Együttélés:** saját port és beállításmappa, OBS-szel egyszerre futtatva tesztelve (C9).

### C9 – Csomagolás és együttélés (K)

Az eredeti terv §23–24-e, a CFA4 csomagolására szabva.

- A stáblista-modul nem hoz létre saját binárist, szolgáltatást vagy portot, ezért ütközést
  sem okozhat; ezt a csomag-ellenőrzés rögzíti.
- A `.cfa4credits` saját fájltípus CFA4-névtérben (`application/x-cfa4-credits`,
  `io.github.ubuntuokos.cfa4`) – Cinecred fájltársítását nem veszi át; a MIME-bejegyzés a
  csomagolásba kerül.
- Ellenőrzés: Cinecred és CFA4 ugyanazon a gépen, egyszerre futtatva, külön beállításokkal;
  eltávolítás után a másik működik.

## 5. Sorrend és függőségek

```text
C1 kapcsolt stáblista ─┬─> C2 ellenőrzések ─> C3 zene
                       └─> C6 sablonok ─> C7 Cinecred-import
C4 háttér / átmenet ───> C5 gazdagabb elrendezés
Qt6 felület (F8, #48–#54) ─> C8 Qt6 munkaterület (első része kész, a többi fázisonként)
C9 együttélés: C7 után, illetve minden kiadás előtt
C10 élő grafika: tervezet; ütemezése külön döntés (VISION 8. pont)
```

Javasolt sorrend: **C1 → C2 → C3 → C4 → C5 → C6 → C7**. Minden fázis a motorral együtt a Qt6-os
felületét is megkapja (C8 folyamatosan), egui-felületet nem.

## 6. Kész-feltételek (az eredeti §32 helyett)

Egy fázis akkor kész, ha:

- a `cfa4-credits` és az engine tesztjei lefedik (beleértve az ellenséges bemenetet:
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
| §17–19 OBS, élő adás, Show/Rundown | **tervezet: C10** | a tulajdonos döntése (2026-10-25): kell, de egyelőre tervezet; a műsorterv mélysége nyitott |
| §21, §25, §32 HRB, Model Router, Canonical Gate, Evidence Registry, visszamenőleges audit | kihagyva | FA3-intézmények; a CFA4-ben a tesztek, ADR-ek és a kiadási kapuk (F9) felelnek meg nekik |
| §2 teljes `.fa3credits` séma (személyek, cégek, jogi adatok, revíziók) | szűkítve | a `.cfa4credits` (C1) a forrást, a stílust és a sorokat tárolja; a teljes produkciós adatmodell csak Story-híddal lenne értelmes |

## 8. Döntések a tulajdonosnak

1. ~~Kell-e önálló stáblista-alkalmazás?~~ **Eldöntve (2026-10-25): csak Qt6**, a CFA4-en belül.
2. ~~Kerüljön-e be az élő adás?~~ **Eldöntve (2026-10-25): igen, egyelőre tervezetként** (C10,
   `VISION.md` #60).
3. ~~Külön `.cfa4credits` fájl?~~ **Eldöntve (2026-10-25): igen** – C1 része, kész.
4. ~~A C1 → C7 sorrend?~~ **Eldöntve (2026-10-25): igen.**
