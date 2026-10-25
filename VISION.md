# CFA4 – Cél és irány

> **Ez a CFA4 legfelső szintű célja.** Minden ember és MI-ügynök (ChatGPT, Claude, Copilot stb.)
> ezt olvassa el először, mielőtt bármilyen munkát választ vagy módosít.
> Ha egy terv, PR vagy audit ellentmond ennek a dokumentumnak, ez a dokumentum az irányadó,
> és az ellentmondást jelezni kell a tulajdonosnak (`ubuntuokos`), nem csendben feloldani.

**Utolsó módosítás:** 2026-10-24 · **Tulajdonos:** ubuntuokos

## 1. Mi a CFA4?

A CFA4 (CinemaFundArchitecture4) egy **egyetlen, egységes, Qt6-os asztali alkalmazás Ubuntura**,
amely több önálló média-program képességeit egyesíti: videóvágás, konvertálás, publikálás,
feliratozás, szinkronizálás, hangszerkesztés, valamint 2D/3D animáció és grafika,
**helyben futó MI-funkciókkal** kiegészítve.

Kiindulópontja egy nyílt forráskódú (MIT OR Apache-2.0) nemlineáris videóvágó 0.10.0-s
változata, amelynek Rust médiamotorja megmarad (az eredet és a szerzői jogi sorok: `NOTICE`).
Az upstream csak olvasható ős; a CFA4 önállóan fejlődik, és nem használja az upstream nevét,
logóit vagy ikonjait.

**Licenc:** a CFA4 egésze **GPL-3.0-or-later** (`LICENSE`, `NOTICE`). Így a lehető legkevesebbet
kell újraírni: GPL-kompatibilis kód és könyvtár felhasználható. Ami nem használható fel, azt más
módon kell megvalósítani. Az örökölt upstream kód eredeti MIT/Apache-megjegyzései (`NOTICE`) megmaradnak.
**Üzleti modell:** a kész CFA4 ingyenes és szabadon terjeszthető; csak a pluszfunkciók és a
hosszabb támogatási idő fizetős. A licencszabályok: [`docs/licensing.md`](docs/licensing.md).

## 2. Alapelvek (nem változnak külön döntés nélkül)

1. **Qt6-felület az első prioritás.** A célfelület Qt6 (Qt6 Widgets + PySide6). A meglévő Rust
   motor (projekt, idővonal, lejátszás, render, kodekek, GPU, export) marad alatta.
   Részletek: `docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`.
   Az örökölt egui-szerkesztő addig marad a működő program, amíg a Qt6-os változat nem tud ugyanannyit.
   A **GStreamer** kiegészítő háttérmotor a saját motor mellett (hardveres kódolás nem NVIDIA GPU-n,
   VP9/AV1 hardveres dekódolás, Qt6-előnézet, felvétel, egyéb formátumok); a saját motor mindig
   tartalék. Részletek: `docs/adr/ADR-0002-gstreamer-backend.md`.
2. **Minden grafikus Ubuntu-változaton fusson** (lásd 3. pont), a mindenkori LTS-en.
   Jelenleg: **Ubuntu 26.04 LTS, AMD64**.
3. **Az MI alapvetően helyi**: a felhasználó gépén fut, a felhasználó adatai nem hagyják el a gépet.
4. **Gyártófüggetlen gyorsítás.** Semmi nem függhet egyetlen GPU-gyártótól. Az NVIDIA mellett az
   AMD és Intel GPU-k, valamint a gépben lévő NPU-k ugyanúgy használhatók legyenek (VA-API, Vulkan,
   ahol van: NPU), és mindig legyen CPU-s tartalék. NVIDIA-specifikus út (NVDEC, NVENC, CUDA) csak
   gyártófüggetlen párja mellett lehet.
5. **Bizonyíték, nem állítás.** Egy funkció csak akkor „kész”, ha teszt vagy valódi gépes mérés
   igazolja. A már igazolt hardveres és kézi teszteket nem kell ok nélkül megismételni.
6. **Minden terv a `main` ágon él.** Terv, cél és döntés nem maradhat csak mellékágon vagy
   piszkozat PR-ban, mert ott elveszik.

## 3. Célplatform: minden grafikus Ubuntu-változat

A Qt6 alkalmazásnak Waylanden **és** X11-en is működnie kell, mert a 26.04 változatai közül
a GNOME-os és a Budgie csak Waylandet használ, több másik pedig még X11-et is kínál.

| Változat (26.04 LTS) | Asztali környezet | Megjegyzés |
|---|---|---|
| Ubuntu | GNOME 50 | csak Wayland |
| Kubuntu | KDE Plasma | **elsődleges fejlesztői környezet** (a referenciagép Kubuntu 26.04.1) |
| Ubuntu Studio | KDE Plasma | kreatív célközönség, kiemelten fontos |
| Xubuntu | Xfce | |
| Lubuntu | LXQt | Qt-alapú, könnyű |
| Ubuntu Budgie | Budgie | csak Wayland |
| Ubuntu Cinnamon | Cinnamon | |
| Edubuntu | GNOME | |
| Ubuntu Kylin | UKUI | |

Az Ubuntu MATE-nek nincs 26.04-es kiadása, az Ubuntu Unity 26.04 nem LTS. Ezeket akkor
vesszük fel, ha LTS-kiadásuk lesz.

Minden változaton ellenőrizendő: telepítés és eltávolítás, menübejegyzés és ikon,
fájltársítás, megnyitás/mentés párbeszédablak, hang, GPU-gyorsítás és CPU-s tartalék.
Csomagformátumok: `.deb`, AppImage, Flatpak, `tar.gz` (lásd F3–F6 auditok).

## 4. Egyesítendő programok és képességeik

A CFA4 ezeknek a programoknak a **képességeit** veszi át, nem a kódjukat, nevüket vagy védjegyeiket.

| Program | Fő terület | Átveendő képességek |
|---|---|---|
| **Adobe Premiere Pro** | videóvágás | Teljes NLE-paritás (meglévő cél, ~56% kész): idővonal, vágóeszközök, multicam, effektek, Lumetri-szín, Essential Sound/Graphics, export |
| **DaVinci Resolve** | vágás, szín, VFX, hang | Csomópont-alapú színkorrekció; Fusion-szerű csomópontos kompozitálás és 2D/3D VFX; Fairlight-szerű hangutómunka; gyors „Cut” vágófelület; kiadási (Deliver) oldal |
| **Kdenlive** | videóvágás, Linux | Linuxos munkamenet és megszokások, proxy-szerkesztés, kulcskockák, render-profilok, beszédből felirat |
| **OpenShot** | egyszerű vágás | Kezdőbarát, egyszerű mód; animált (3D) címek; könnyű kulcskocka-animáció |
| **Shutter Encoder**, **Videomass** (a GPL-2.0-only HandBrake helyett) | konvertálás | Kötegelt átkódolás sorral, előbeállítások, gyártófüggetlen hardveres kódolók, szűrők (deinterlace, zajcsökkentés), felirat- és hangsáv-kezelés, loudness-mérés és hangnormalizálás, újrakódolás nélküli vágás. Motor: a saját motor és a GStreamer ([ADR 0003](docs/adr/ADR-0003-aswf-and-conversion-sources.md)) |
| **Audacity** | hangszerkesztés | Hullámforma-szerkesztő, többsávos hang, zajcsökkentés, spektrális szerkesztés, effektek és bővítmények (LV2/VST3), címkék, sokféle hangformátum |
| **Subtitle Edit** | feliratozás | Sok feliratformátum, vizuális és pontos időzítés hullámformán, beszédfelismerés, fordítás, képes feliratok OCR-je, helyesírás-ellenőrzés |
| **Krita** | rajz, 2D animáció | Raszteres rajzolás és festés, ecsetek, kockánkénti 2D animáció hagymahéjjal (onion skin) és idővonallal |

További, eddig hiányzó területek:

- **Publikálás:** kész videó előkészítése és feltöltése videómegosztókra, platformonkénti előbeállításokkal.
- **Szinkronizálás:** átirat → fordítás → helyi gépi hang vagy saját felvétel → időzítés az eredeti szájmozgáshoz és vágáshoz.
- **3D animáció:** 3D-s jelenetek, címek és animáció (a pontos mélység külön döntés).

## 5. MI-funkciók (alapvetően helyi)

Meglévő alapok: helyi Whisper-átirat és szövegalapú vágás, Kokoro-82M helyi felolvasás.

Tervezett irányok:

- beszédből felirat, felirat fordítása, automatikus időzítés;
- szinkron: fordított szöveg helyi gépi hangon, időzítéssel;
- hang: zaj- és zene-leválasztás, beszédtisztítás;
- kép: objektum- és személymaszkolás, követés, jelenetfelismerés, automatikus színillesztés;
- vágás: jelenetek és csendek felismerése, durva vágás átirat alapján;
- konvertálás és publikálás: platformonkénti beállítás-javaslat.

Szabály: a modellek a felhasználó gépén futnak: CPU-n, vagy bármely gyártó GPU-ján, illetve NPU-n,
ha van (lásd 2.4: gyártófüggetlen gyorsítás). Minden modellnél
rögzíteni kell a licencét és a forrását, a modellek nem töltődnek le a felhasználó tudta nélkül.

## 6. Prioritási sorrend

1. **Qt6-felület** az F8 ADR 2–7. lépése szerint: háttérszálak → médiaböngésző és projekt → idővonal
   → előnézet, hang és GPU → export → csomagolás.
2. **Futás minden grafikus Ubuntu-változaton** (3. pont): Wayland és X11 teszt.
3. A meglévő vágóképességek átemelése a Qt6-felületre, Premiere-paritás folytatása.
4. Új területek fokozatosan: konvertálás → feliratozás → hangszerkesztés → szinkron → publikálás
   → 2D animáció → 3D/VFX.
5. MI-funkciók mindegyik területhez, helyben.

## 7. Kapcsolódó dokumentumok

Minden terv, döntés és audit tartalomjegyzéke: [`docs/audit/README.md`](docs/audit/README.md).
A legfontosabbak:

- [`docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`](docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md) – Qt6 ↔ Rust architektúra
- [`docs/adr/ADR-0002-gstreamer-backend.md`](docs/adr/ADR-0002-gstreamer-backend.md) – GStreamer mint kiegészítő háttérmotor
- [`docs/audit/F9-RELEASE-DECISION-2026-10-23.md`](docs/audit/F9-RELEASE-DECISION-2026-10-23.md) és `f9-release-readiness.json` – kiadási kapuk összesítése
- [`docs/audit/F7-NATIVE-MEDIA-GATE-2026-10-23.md`](docs/audit/F7-NATIVE-MEDIA-GATE-2026-10-23.md) – mi működik ténylegesen a médiakezelésben
- `docs/audit/R0-*.md` – igazolt hardveres mérések (RTX A1000)
- [`docs/adr/ADR-0003-aswf-and-conversion-sources.md`](docs/adr/ADR-0003-aswf-and-conversion-sources.md) – ASWF-projektek (OpenColorIO, OpenImageIO, OpenFX …) és a konvertálás forrásai

A dokumentumok és a hozzájuk tartozó kód a `main`-ben vannak (#35, 2026-10-24).

## 8. Nyitott döntések

- A 3D animáció és VFX mélysége (egyszerű 3D címek vagy teljes 3D jelenetszerkesztés).
- Mely videómegosztókra lehessen publikálni.
- Mennyi MI-modell kerüljön a csomagba, és melyik legyen külön letölthető (a felhasználó jóváhagyásával).
- Az „alapvetően helyi” MI mellett megengedett-e kivételesen, kifejezett felhasználói engedéllyel felhős szolgáltatás.
