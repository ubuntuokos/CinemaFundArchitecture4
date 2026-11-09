# CFA4-LIVE-STUDIO-001 – Az OBS Studio képességei a CFA4-ben

**Verzió:** 1.0 (tervezet)
**Dátum:** 2026-11-09
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** tervezet; a tulajdonossal lépésenként egyeztetve (2026-11-09), jóváhagyásra vár. A fejlesztés
indítása külön döntéshez kötött.
**Kapcsolódik:** [`VISION.md`](../../VISION.md) 4. pont (élő adás), [`docs/credits-plan.md`](../credits-plan.md)
C10 (élő grafika, `crates/live`), [ADR 0001](../adr/0001-platform-ffi.md) (FFI),
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
5. **Bemenetek, kép és hang, stúdió mód, kimenetek, kódolók, vezérlés, átállás, mérés:** az
   egyeztetés szerinti felsorolással (IV. fejezet).

## II. Kiinduló helyzet

- **OBS Studio:** a legfrissebb kiadás a 33.0 (2026. október). A kód licence GPL-2.0-or-later
  (a libobs is; egyes részek ISC, MIT, BSD, LGPL – mind GPL-3-mal összeférő), tehát a
  GPL-3.0-or-later CFA4-be átvehető, forrásmegjelöléssel (`NOTICE`, `ATTRIBUTION.md`).
- **Ubuntu:** a 24.04 tárolójában `libobs-dev` 30.0.2 van, böngészőforrás (obs-browser, CEF)
  nélkül; a 26.04-é újabb, de a böngészőforrás ott sincs benne (a C10-es próba ezt mutatta).
- **Ami a CFA4-ben már megvan:** saját renderelő és grafika (címek, stáblista), az élő grafikai
  futtató (C10: cue-k, TAKE / NEXT / HOLD / UPDATE / OUT, élő oldal 127.0.0.1-en, obs-websocket
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
| 2 | Bemenetek | kamera (V4L2), képernyő és ablak (PipeWire, XDG-portál), hangbemenet és -kimenet rögzítése (PipeWire/Pulse), médiafájl és lejátszási lista, kép és képsorozat, szín, szöveg, böngészőforrás, CFA4-grafika és -stáblista (C10), CFA4-idővonal mint forrás, NDI/SRT bemenet (nyitott, VII.) |
| 3 | Videószűrők | chroma key, color key, luma key, színkorrekció, LUT, maszk/keverés, élesítés, vágás/párnázás, görgetés, képarány, késleltetés |
| 4 | Hang | keverő sávonként, hangerő, némítás, monitorozás, sávkiosztás (6 sáv), szűrők: zajszűrő (RNNoise), zajkapu, kompresszor, limiter, expander, erősítés, EQ, VST helyett beépített láncok, szinkroneltolás |
| 5 | Stúdió mód | előnézet és program, átmenetek (vágás, áttűnés, csúsztatás, stinger, luma wipe, színen át), átmenet-felülírás jelenetenként, multiview, kivetítők (teljes képernyős monitorok), gyorsbillentyűk |
| 6 | Kimenetek | felvétel (MKV, MP4, fragmentált MP4, MOV), streamelés (RTMP/RTMPS, SRT, RIST, WHIP/WebRTC), több cél egyszerre, replay buffer, virtuális kamera (v4l2loopback), felvétel szétválasztása, fejezetjelek |
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

1. **Külön folyamat a libobs-nak** (`cfa4-studio-host`, Rust): a libobs C kód, és egy hibája ne
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
6. **A libobs forrása:** rögzített verziójú, saját fordítás a CFA4 csomagban (mint az FFmpeg
   9.0.2), mert az Ubuntu csomagja régebbi és böngészőforrás nélküli (D1).

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
| **S5** – Felvétel | MKV/MP4/fMP4/MOV, sávok, szétválasztás, fejezetjelek; a felvétel egy mozdulattal a projekt idővonalára | egy órás felvétel elejtett képkocka nélkül a referenciagépen |
| **S6** – Streamelés | RTMP/RTMPS, SRT, RIST, WHIP; több cél; újrakapcsolódás; stream-kulcs a kulcstartóban | próbaadás helyi kiszolgálóra (mediamtx) és egy valódi szolgáltatásra |
| **S7** – Kódolók | VA-API (Intel/AMD), NVENC (opcionális), QSV, SVT-AV1 a `cfa4-route`-on át; restricted-extras | a router választ, CPU-tartalék mindig van; a referenciagép GPU-in mérve |
| **S8** – Videószűrők | key-ek, színkorrekció, LUT, maszk, vágás, késleltetés; a CFA4 saját effektjei forrásszűrőként | a szűrők képe összevetve a CFA4 renderelővel |
| **S9** – További kimenetek | replay buffer, virtuális kamera (v4l2loopback), böngészőforrás (CEF, D3), NDI (nyitott) | mindegyik a referenciagépen |
| **S10** – Vezérlés és átállás | obs-websocket 5 kiszolgáló a CFA4-ben, OBS-import (jelenetgyűjtemény, profil) | Stream Deck / websocket-kliens vezérli; egy valódi OBS-gyűjtemény átjön |
| **S11** – Mérés és stabilitás | statisztika-panel, tartós terheléses próbák, hibainjektálás, dokumentáció | 8 órás adás a referenciagépen, összeomlás nélkül |
| **S12+** – Rust-csere | a libobs részei sorban saját Rust kódra: 1. kompozitálás és átmenetek (a CFA4 renderelője), 2. szűrők, 3. kimenetek és kódolás (GStreamer/saját muxerek), 4. rögzítés (PipeWire, V4L2) | minden csere ugyanazon a traiten, a régi úttal összemérve; ha valamelyik lassabb, marad a libobs |

## VII. Nyitott döntések (lépésenként egyeztetjük)

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | A libobs forrása: saját rögzített fordítás (33.x) vagy az Ubuntu tárolójának csomagja? | saját fordítás a CFA4 csomagban (újabb, egységes, böngészőforrással is) |
| D2 | Külön folyamat a libobs-nak, vagy a motorban fut? | külön folyamat (never-crash, újraindítható) |
| D3 | Böngészőforrás (CEF, Chromium): beépítjük-e, és mikor? | az S9-ben, külön csomagként (nagy méret, saját frissítési rend) |
| D4 | NDI és más zárt protokollok | csak nyílt protokollok (SRT, RIST, WHIP); NDI-t nem (zárt SDK) |
| D5 | Az élő stúdió helye a menüben és a munkaterületek között | önálló „Élő” munkaterület a Szerkesztés, Szín, Hang mellett |
| D6 | Műsorterv (rundown) | a VISION 8. pontjában nyitott; ebben a tervben nincs |
| D7 | Ütemezés: a Qt6 befejezése után, vagy azzal párhuzamosan? | az S0 most indulhat (nem érinti a Qt6 fájljait), az S1-től a Qt6 után |

## VIII. Kockázatok

- **Két grafikus réteg:** a libobs saját OpenGL/EGL rétegét és a CFA4 renderelőjét egy gépen,
  egy GPU-n kell futtatni; a GPU-memória és a színterek (sRGB/Rec.709/HDR) egyezését mérni kell.
- **Wayland:** képernyőrögzítés csak az XDG-portálon át, felhasználói engedéllyel.
- **Méret:** a libobs + kódolók + (később) CEF jelentősen növeli a csomagot; a CEF külön csomag.
- **Licenc és szabadalom:** x264/x265 csak a restricted-extras csomagban; a libobs részeinek
  licencei a `NOTICE`-ba és az `ATTRIBUTION.md`-be kerülnek.
- **A Qt6 munka:** az élő munkaterület a Qt6 felületre épül; ütközésnél a Qt6 megy előre.

## IX. Források

- OBS Studio 33.0 kiadás (2026. október): <https://www.deskmodder.de/blog/2026/10/24/obs-studio-33-0-steht-als-rc1-zum-download-bereit/>,
  <https://alternativeto.net/news/2026/10/obs-studio-33-0-arrives-with-a-two-year-browser-engine-jump-and-a-plugin-loading-rework/>
- Az OBS Studio licencei (Debian copyright-fájl: a fő kód és a libobs GPL-2+):
  <https://metadata.ftp-master.debian.org/changelogs/main/o/obs-studio/testing_copyright>
- Az Ubuntu `libobs-dev` csomagja (24.04: 30.0.2) és a böngészőforrás hiánya: a C10-es próba,
  [`docs/credits-plan.md`](../credits-plan.md) C10.
