# CFA4-LIVE-STUDIO-001 – Az OBS Studio képességei a CFA4-ben

**Verzió:** 1.3
**Dátum:** 2026-11-11 (1.2: 2026-11-11, 1.1: 2026-11-10, 1.0: 2026-11-09)
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-09-én lépésenként egyeztette és jóváhagyta; az **S0
fázis indítását is jóváhagyta** (2026-11-09). Az S1-től a munka a Qt6 felület befejezése után indul (D7).
Az 1.1-ben a D3 újra nyitott (a WPE WebKit nincs az Ubuntu 26.04-ben), és rögzítve, hogy a C10 a
terv előfeltétele; az 1.2-ben a tulajdonos a D3-at a Qt WebEngine-nel döntötte el (2026-11-11). Az 1.3-ban
a D4 kiegészítve: az SMPTE ST 2110 is nyílt protokoll ([kiegészítés](CFA4-LIVE-STUDIO-001-ST2110.md), a
tulajdonos 2026-11-11-i megerősítésével).
**Kapcsolódik:** [`VISION.md`](../../VISION.md) 4. és 8. pont (élő adás), `docs/credits-plan.md`
C10 (élő grafika, `crates/live`; a #803-ban, előfeltétel), [ADR 0001](../adr/0001-platform-ffi.md) (FFI),
[ADR 0004](../adr/ADR-0004-compute-routing.md) (`cfa4-route`), [ADR 0005](../adr/ADR-0005-media-backends.md)
(kiegészítő médiahátterek), [ADR 0002](../adr/ADR-0002-gstreamer-backend.md) (GStreamer),
[`docs/licensing.md`](../licensing.md), [`docs/coordination.md`](../coordination.md)

---

## I. A tulajdonos döntései (2026-11-09)

1. **Hatókör:** az OBS Studio **teljes kiváltása** egy tervben (nem csak egy alapstúdió). A terv
   fázisokra bomlik, hogy menet közben is legyen használható részeredmény.
2. **Alap:** a felület a **CFA4 Qt6 felülete**, a képességek az **OBS Studio** képességei.
3. **Motor: vegyes út.** Most az OBS magja (**libobs**) végzi a valós idejű munkát, később
   **részenként saját Rust kódra** cseréljük (a „Rust first” szabály szerint: nem-Rust komponens
   mehet, ha nem lassítja a CFA4-et, és később Rustra portolható; `AGENTS.md`, 2026-11-07).
4. **A jelenetek a projektben élnek** (a `.fcproj` része), nem külön stúdiógyűjteményben.
5. **A libobs forrása:** saját, rögzített verziójú fordítás (D1).
6. **A libobs külön folyamatban fut** (`cfa4-studio-host`, D2).
7. **Böngészőforrás Qt WebEngine-nel** az Ubuntu tárolójából (D3, 2026-11-11; a WPE WebKit nincs
   az Ubuntu 26.04-ben).
8. **Csak nyílt protokollok** (SRT, RIST, WHIP/WebRTC, RTMP, 2026-11-11 óta SMPTE ST 2110 is); zárt SDK-t igénylő (pl. NDI) nincs (D4;
   az ST 2110: [kiegészítés](CFA4-LIVE-STUDIO-001-ST2110.md)).
9. **Önálló „Élő” munkaterület** a Qt6 felületen; a C10 Élő grafika ablaka beleolvad (D5).
10. **Műsorterv (rundown)** a terv része, az S4a fázisban (D6).
11. **Ütemezés:** az S0 most indulhat, az S1-től minden a Qt6 befejezése után (D7).
12. **Bemenetek, kép és hang, stúdió mód, kimenetek, kódolók, vezérlés, átállás, mérés:** az
   egyeztetés szerinti felsorolással (IV. fejezet).

## II. Kiinduló helyzet

- **OBS Studio:** a legfrissebb kiadás a 33.0 (2026. október). A kód licence GPL-2.0-or-later
  (a libobs is; egyes részek ISC, MIT, BSD, LGPL – mind GPL-3-mal összeférő), tehát a
  GPL-3.0-or-later CFA4-be átvehető, forrásmegjelöléssel (`NOTICE`, `ATTRIBUTION.md`).
- **Ubuntu:** a 24.04 tárolójában `libobs-dev` 30.0.2 van, böngészőforrás (obs-browser, CEF)
  nélkül (a C10-es próba ezt mutatta); a 26.04-es csomag verzióját az S0-ban ellenőrizzük.
- **Előfeltétel: a C10 élő grafikai futtató.** A `crates/live`, a `live.*` parancsok és az Élő grafika
  ablak a stáblista-lánc utolsó PR-jában (#803, `cfa4/credits-c10`) vannak, a `main`-ben még nincsenek.
  Az S1 (CFA4-grafika forrásként) és az S4 (TAKE a stúdió módban) csak a lánc összevonása után
  indulhat; az S0 nem függ tőle.
- **Ami a CFA4-ben már megvan vagy a láncban kész:** saját renderelő és grafika (címek, stáblista), az élő grafikai
  futtató (C10, a #803-ban: cue-k, TAKE / NEXT / HOLD / UPDATE / OUT, élő oldal 127.0.0.1-en, obs-websocket
  kliens), a `cfa4-route` GPU/NPU/CPU-router, a GStreamer és az FFmpeg mint kiegészítő
  háttér, saját kódolók és muxerek, a Qt6 felület, parancsrendszer és vezérlőcsatorna (ügynökök,
  MCP).
- **A Qt6 felület Pythonban (PySide6) fut, a motor külön folyamat** (`cfa4-cli serve`,
  JSON-sorok). Ez meghatározza, hogyan jut el az élő kép a felületre (V. fejezet).

## III. Célkép

A CFA4-ben egy **Élő stúdió** munkaterület: jelenetek forrásokkal, előnézet és program,
átmenetek, hangkeverő, felvétel és streamelés – mindaz, amit az OBS Studio tud –, a CFA4
grafikáival, idővonalával és parancsrendszerével egy alkalmazásban. A CFA4-ben készült
címek, stáblisták és sablonok közvetlenül források a jelenetekben; ami élőben ment, felvételként
egy mozdulattal a vágóprogramba kerül.

## IV. Képességek (a teljes kiváltás listája)

| # | Terület | Képességek |
|---|---|---|
| 1 | Jelenetek | jelenetek, források, csoportok, egymásba ágyazott jelenet, sorrend, zárolás, láthatóság, transzformáció (méret, forgatás, vágás, igazítás) |
| 2 | Bemenetek | kamera (V4L2), képernyő és ablak (PipeWire, XDG-portál), hangbemenet és -kimenet rögzítése (PipeWire/Pulse), médiafájl és lejátszási lista, kép és képsorozat, szín, szöveg, böngészőforrás, CFA4-grafika és -stáblista (C10), CFA4-idővonal mint forrás, hálózati bemenet SRT-n, RIST-en, WHIP-en, SMPTE ST 2110-en (kép, hang, ANC, TTML; csak nyílt protokollok, D4) |
| 3 | Videószűrők | chroma key, color key, luma key, színkorrekció, LUT, maszk/keverés, élesítés, vágás/párnázás, görgetés, képarány, késleltetés |
| 4 | Hang | keverő sávonként, hangerő, némítás, monitorozás, sávkiosztás (6 sáv), szűrők: zajszűrő (RNNoise), zajkapu, kompresszor, limiter, expander, erősítés, EQ, beépített szűrőláncok, szinkroneltolás |
| 5 | Stúdió mód | előnézet és program, átmenetek (vágás, áttűnés, csúsztatás, stinger, luma wipe, színen át), átmenet-felülírás jelenetenként, multiview, kivetítők (teljes képernyős monitorok), gyorsbillentyűk |
| 6 | Kimenetek | felvétel (MKV, MP4, fragmentált MP4, MOV), streamelés (RTMP/RTMPS, SRT, RIST, WHIP/WebRTC), SMPTE ST 2110 programkimenet PTP-hez igazítva, több cél egyszerre, replay buffer, virtuális kamera (v4l2loopback), felvétel szétválasztása, fejezetjelek |
| 7 | Kódolók | gyártófüggetlen hardveres kódolás (VA-API: Intel/AMD; NVENC opcionálisan; Intel QSV) a `cfa4-route`-on át; szoftveres: AV1 (SVT-AV1/rav1e), H.264/HEVC csak a `cfa4-restricted-extras` csomaggal (szabadalmi modell) |
| 8 | Vezérlés | minden művelet CFA4-parancs (ügynökök, MCP, vezérlőcsatorna); obs-websocket 5-kompatibilis kiszolgáló (Stream Deck, Touch Portal és társai), gyorsbillentyűk |
| 9 | Átállás | OBS jelenetgyűjtemény- és profilimport (JSON), kimeneti beállítások és szűrők átvétele |
| 10 | Mérés, stabilitás | képkocka-időzítés, elejtett képkockák, CPU/GPU-terhelés, statisztika-panel; a referenciagépen mérve, never-crash |

## V. Felépítés

```
Qt6 felület (Python)          CFA4 motor (cfa4-cli serve)          cfa4-studio-host (új folyamat)
 Élő stúdió munkaterület  ──►  live.studio.* parancsok        ──►   libobs (C) + Rust burok
 előnézet / program  ◄────────── megosztott memória (memfd) ◄──────  kimenő képkockák
                               jelenetmodell a projektben           kódolás, felvétel, stream
```

1. **Külön folyamat a libobs-nak** (`cfa4-studio-host`, Rust; D2, eldöntve 2026-11-09): a libobs C kód, és egy hibája ne
   vigye magával a szerkesztőt (never-crash, `AGENTS.md` §0). A motor indítja, felügyeli, és
   összeomlás után újraindítja; az adás közben ez a leggyorsabb helyreállás.
2. **Rust burok a libobs körül** (`crates/obs`, FFI a [ADR 0001](../adr/0001-platform-ffi.md)
   szerint, trait mögött): forrás, szűrő, átmenet, kimenet, kódoló Rust-típusokként. Erre épül a
   későbbi csere: ugyanaz a trait, Rust megvalósítással.
3. **A CFA4 grafikái libobs-forrásként:** egy Rust-ban írt libobs-forrás a C10 élő futtatóját
   rendereli (címek, stáblista, sablonok), böngésző és weboldal nélkül.
4. **Kép a Qt6 felületre:** a libobs kimeneti képkockái megosztott memórián (memfd, kerethiba
   esetén eldobva) jutnak a felületre, amely OpenGL-lel rajzolja ki őket. Így X11-en és Wayland-en
   is működik, idegen ablak beágyazása nélkül.
5. **Jelenetmodell a projektben:** a jelenetek, források, szűrők és kimeneti beállítások a
   `.fcproj` részei (serde), a parancsok szerkesztik (visszavonható), és a motor ebből állítja
   össze a libobs jeleneteit. Jelszavak és stream-kulcsok nem kerülnek a projektbe: a felhasználói
   kulcstartóban (Secret Service) vannak.
6. **A libobs forrása (D1, eldöntve 2026-11-09):** rögzített verziójú, saját fordítás a CFA4
   csomagban (mint az FFmpeg 9.0.2), mert az Ubuntu csomagja régebbi és böngészőforrás nélküli.

## VI. Fázisok

Minden fázis végén működő, tesztelt, beolvasztható állapot; a minőségi kapuk zöldek, a Qt6
munkával ütközésnél a Qt6 megy előre.

| Fázis | Tartalom | Kész, ha |
|---|---|---|
| **S0** – Előkészítés | libobs 33 rögzített fordítása (CI-parancsfájl, ellenőrzőösszeg, licencek), `crates/obs` FFI-váz, `cfa4-studio-host` folyamat felügyelettel, újraindítással | a host elindul, egy üres jelenet képkockái megjelennek a Qt6-ban; összeomlásteszt: a szerkesztő fut tovább |
| **S1** – Jelenetek és alapforrások | jelenetmodell a projektben, parancsok (`live.studio.scene.*`, `.source.*`), szín, kép, szöveg, médiafájl, CFA4-grafika (C10) forrásként, transzformáció; Élő stúdió munkaterület a Qt6-ban (jelenet- és forráslista, előnézet) | egy jelenet CFA4-címmel és médiafájllal 60 fps-sel látszik; mentés–betöltés–visszavonás |
| **S2** – Rögzítő bemenetek | kamera (V4L2), képernyő és ablak (PipeWire-portál), hangbemenet/-kimenet (PipeWire) | kamera + képernyő + mikrofon egy jelenetben, a referenciagépen mérve |
| **S3** – Hangkeverő | sávok, hangerő, némítás, monitorozás, VU-mérők, hangszűrők, szinkroneltolás | a keverő minden forrásra; a szűrők hangja mérve |
| **S4** – Stúdió mód | előnézet/program, átmenetek, multiview, kivetítők, gyorsbillentyűk | TAKE-kel jelenetváltás átmenettel; a C10 élő grafika is itt fut |
| **S4a** – Műsorterv (rundown) | tételek és blokkok a projektben (jelenet, C10-grafika, bejátszás, megjegyzés), tervezett hossz, adás közben „következő tétel” egy gombbal, hátralévő és csúszási idő; később import (CSV/táblázat) és ügynöki vezérlés | egy próbaműsor végigvezetve a tételeken, az időszámlálás mérve |
| **S5** – Felvétel | MKV/MP4/fMP4/MOV, sávok, szétválasztás, fejezetjelek; a felvétel egy mozdulattal a projekt idővonalára | egy órás felvétel elejtett képkocka nélkül a referenciagépen |
| **S6** – Streamelés | RTMP/RTMPS, SRT, RIST, WHIP; több cél; újrakapcsolódás; stream-kulcs a kulcstartóban | próbaadás helyi kiszolgálóra (mediamtx) és egy valódi szolgáltatásra |
| **S7** – Kódolók | VA-API (Intel/AMD), NVENC (opcionális), QSV, SVT-AV1 a `cfa4-route`-on át; restricted-extras | a router választ, CPU-tartalék mindig van; a referenciagép GPU-in mérve |
| **S8** – Videószűrők | key-ek, színkorrekció, LUT, maszk, vágás, késleltetés; a CFA4 saját effektjei forrásszűrőként | a szűrők képe összevetve a CFA4 renderelővel |
| **S9** – További kimenetek | replay buffer, virtuális kamera (v4l2loopback), böngészőforrás (Qt WebEngine, D3) | mindegyik a referenciagépen |
| **S10** – Vezérlés és átállás | obs-websocket 5 kiszolgáló a CFA4-ben, OBS-import (jelenetgyűjtemény, profil) | Stream Deck / websocket-kliens vezérli; egy valódi OBS-gyűjtemény átjön |
| **S11** – Mérés és stabilitás | statisztika-panel, tartós terheléses próbák, hibainjektálás, dokumentáció | 8 órás adás a referenciagépen, összeomlás nélkül |
| **S12+** – Rust-csere | a libobs részei sorban saját Rust kódra: 1. kompozitálás és átmenetek (a CFA4 renderelője), 2. szűrők, 3. kimenetek és kódolás (GStreamer/saját muxerek), 4. rögzítés (PipeWire, V4L2) | minden csere ugyanazon a traiten, a régi úttal összemérve; ha valamelyik lassabb, marad a libobs |

## VII. Döntések (2026-11-09-én lépésenként egyeztetve, mind eldöntve; a D3 2026-11-11-én újra)

| # | Kérdés | Döntés |
|---|---|---|
| D1 | A libobs forrása | **Eldöntve (2026-11-09): saját, rögzített verziójú fordítás (33.x) a CFA4 csomagban**, ellenőrzőösszeggel, mint az FFmpeg 9.0.2 |
| D2 | Hol fut a libobs | **Eldöntve (2026-11-09): külön folyamatban** (`cfa4-studio-host`): a motor indítja, felügyeli, összeomlás után újraindítja; a szerkesztő és a projekt megmarad |
| D3 | Böngészőforrás | **Eldöntve (2026-11-11): Qt WebEngine** (`qt6-webengine`, az Ubuntu 26.04-ben 6.10.2, csomagfüggőségként; a PySide6-ban is benne van; nyílt, LGPL, az Ubuntu adja a biztonsági frissítéseit), a `cfa4-studio-host` folyamatban, képernyőn kívüli rendereléssel, saját Rust libobs-forrással, az S9-ben. Az első döntés (2026-11-09: WPE WebKit az Ubuntu tárolójából) nem teljesíthető, mert az Ubuntu a 24.04-től kivette a `libwpe`-t és a `wpebackend-fdo`-t, a `wpewebkit` pedig a szinkronizálási tiltólistán van. Elvetve: WebKitGTK 6.0 (GTK-t hozna a Qt mellé), saját rögzített CEF (saját biztonsági frissítések). A `window.obsstudio` JS-felületet a forrás adja; a gyakori streameres widgetek az S9-ben kipróbálva |
| D4 | Zárt protokollok | **Eldöntve (2026-11-09): csak nyílt protokollok** (SRT, RIST, WHIP/WebRTC, RTMP); NDI és más zárt SDK-t igénylő protokoll nincs. **Kiegészítve (2026-11-11):** az SMPTE ST 2110 is a nyílt protokollok közé tartozik; fázisai (T0–T7) és döntései (D8–D12) a [kiegészítésben](CFA4-LIVE-STUDIO-001-ST2110.md) |
| D5 | Az élő stúdió helye | **Eldöntve (2026-11-09): önálló „Élő” munkaterület** a szerkesztés, szín, hang mellett (középen előnézet és program, lent jelenetek, források, hangkeverő, oldalt átmenetek és vezérlés); a C10 Élő grafika ablaka ide kerül át; közös projekt |
| D6 | Műsorterv (rundown) | **Eldöntve (2026-11-09): a terv része**, az S4a fázisban (az S4 stúdió mód után); a `VISION.md` 8. pontjának nyitott kérdését ezzel a tulajdonos lezárta |
| D7 | Ütemezés | **Eldöntve (2026-11-09): az S0 most indulhat** (nem érinti a Qt6 fájljait), **az S1-től minden a Qt6 befejezése után** |

## VIII. Kockázatok

- **Két grafikus réteg:** a libobs saját OpenGL/EGL rétegét és a CFA4 renderelőjét egy gépen,
  egy GPU-n kell futtatni; a GPU-memória és a színterek (sRGB/Rec.709/HDR) egyezését mérni kell.
- **Wayland:** képernyőrögzítés csak az XDG-portálon át, felhasználói engedéllyel.
- **Méret:** a libobs + kódolók növelik a csomagot; a böngészőforrás motorja az Ubuntu tárolójából jön (nem a CFA4 csomagjában; D3).
- **Webes widgetek:** Chromiumra készülnek, a Qt WebEngine Chromium-alapú, így eltérés kevés várható (S9-ben a gyakori szolgáltatásokkal kipróbálva).
- **A Qt WebEngine biztonsága (a tulajdonossal egyeztetve, 2026-11-11):** belül Chromium (Blink,
  V8), így a Chrome-ra írt támadó kód nagyrészt rajta is működik, és a Chrome biztonsági javításait
  késve kapja (a Qt veszi át, utána az Ubuntu csomagolja). A kisebb felhasználói kör nem véd: a
  böngésző azonosítója (user agent) minden oldalnak elárulja a Chromium verzióját. Ezért a védelem
  nem a rejtettségen, hanem ezeken alapul (S9 elfogadási feltétele):
  1. **Elszigetelés:** a Chromium-sandbox mindig bekapcsolva, a böngészőforrás a külön
     `cfa4-studio-host` folyamatban fut (D2), a szerkesztőtől és a projekttől elválasztva.
  2. **Frissítés:** a Qt WebEngine mindig az Ubuntu friss csomagjából jön, nem rögzített
     verzióból (a D1 rögzítése csak a libobs-ra vonatkozik).
  3. **Alapból kikapcsolva:** a böngészőforrás csak akkor él, ha a felhasználó bekapcsolja.
  4. **Figyelmeztetés idegen oldalnál:** a CFA4 jelez, ha egy forrás külső weboldalt tölt be;
     helyi oldalak (a C10 élő oldala, helyi fájlok) és az ismert streamer-szolgáltatások
     figyelmeztetés nélkül futnak.
  A méret: nagy függőség, az Ubuntu tárolójából jön, nem a CFA4 csomagjában.
- **Licenc és szabadalom:** x264/x265 csak a restricted-extras csomagban; a libobs részeinek
  licencei a `NOTICE`-ba és az `ATTRIBUTION.md`-be kerülnek.
- **A Qt6 munka:** az élő munkaterület a Qt6 felületre épül; ütközésnél a Qt6 megy előre.

## IX. Források

- OBS Studio 33.0 kiadás (2026. október): <https://www.deskmodder.de/blog/2026/10/24/obs-studio-33-0-steht-als-rc1-zum-download-bereit/>,
  <https://alternativeto.net/news/2026/10/obs-studio-33-0-arrives-with-a-two-year-browser-engine-jump-and-a-plugin-loading-rework/>
- Az OBS Studio licencei (Debian copyright-fájl: a fő kód és a libobs GPL-2+):
  <https://metadata.ftp-master.debian.org/changelogs/main/o/obs-studio/testing_copyright>
- Az Ubuntu `libobs-dev` csomagja (24.04: 30.0.2) és a böngészőforrás hiánya: a C10-es próba,
  `docs/credits-plan.md` C10 (a #803-ban, a `main`-ben még nincs).
- A WPE kikerülése az Ubuntuból: <https://bugs.launchpad.net/bugs/2061605> (a `libwpe` és a
  `wpebackend-fdo` eltávolítása a 24.04-től; a `wpewebkit` és a `cog` a szinkronizálási tiltólistán).
- Qt WebEngine az Ubuntu 26.04-ben: <https://packages.ubuntu.com/resolute/libqt6webenginequick6>,
  <https://lists.snapcraft.io/archives/resolute-changes/2026-February/020982.html> (qt6-webengine 6.10.2).
