# CFA4-SECURITY-001 – A CFA4 futásidejű biztonsági rétege

**Verzió:** 0.1 (tervezet, egyeztetés alatt)
**Dátum:** 2026-11-11
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-11-én kérte; a döntéseket (VII. fejezet)
lépésenként egyeztetjük, utána indul a munka.
**Kapcsolódik:** [`AGENTS.md`](../../AGENTS.md) §0 (never crash) és CFA4-szakasz,
[ADR 0001](../adr/0001-platform-ffi.md) (FFI), [ADR 0005](../adr/ADR-0005-media-backends.md)
(kiegészítő médiahátterek), [`CFA4-LIVE-STUDIO-001`](CFA4-LIVE-STUDIO-001.md) (D2, D3 és a
böngészőforrás négy védelme), [`docs/control-protocol.md`](../control-protocol.md),
[`.github/workflows/cfa4-security.yml`](../../.github/workflows/cfa4-security.yml),
[`docs/development/reference-machine.md`](../development/reference-machine.md)

---

## I. Cél

Ha egy rosszindulatú médiafájl, projekt, weboldal, hálózati adatfolyam, bővítmény vagy MI-modell
megtöri a CFA4 valamelyik részét, a kár **abban az egy folyamatban maradjon**: ne olvashassa a
felhasználó többi fájlját (`~/.ssh`, böngészőprofil, dokumentumok), ne indíthasson programot, ne
nyithasson hálózatot, és ne vigye magával a szerkesztőt és a projektet (never crash, §0).

A réteg a fejlesztésbe épített védelmet (CI-szkennerek) egészíti ki a felhasználó gépén, futás
közben. Nem helyettesíti a hibák javítását: a cél, hogy egy javítatlan hiba ne legyen
kihasználható.

## II. Kiinduló helyzet (2026-11-11, `main`)

**Ami megvan (fejlesztés és CI):**
- Semgrep saját szabályok (blokkoló): hálózati port csak a 127.0.0.1-en, nincs shellparancs,
  `unsafe` csak a `crates/platform`-ban (`.github/workflows/cfa4-security.yml`).
- OSV-Scanner (jelentés), `cargo deny` (blokkoló Rust-advisory és licencellenőrzés), CodeQL.
- A vezérlőcsatorna tokennel védett (`CFA4_CONTROL_TOKEN`, 64 hex karakter, munkamenetenként új).
- A saját kodekek és konténerek Rustban, `unsafe` nélkül (az L0 crate-ek csak a `cfa4-bitstream`-től függnek).

**Ami hiányzik (futásidő):** semmi nem korlátozza, mit tehet egy folyamat a felhasználó gépén.

**Kitett pontok** (ahol idegen adat jut a CFA4-be):

| # | Pont | Mi dolgozza fel | Kockázat |
|---|---|---|---|
| 1 | Médiafájlok | saját Rust dekóderek (biztonságos nyelv, de logikai és erőforrás-hibák); **FFmpeg és GStreamer C kódja a motor folyamatában** (`crates/ffmpeg` linkelve) | a legnagyobb: a C dekóderek hibái kódfuttatáshoz vezethetnek |
| 2 | Projekt- és cserefájlok | `.fcproj`, OTIO, XML/EDL/AAF, `.cfa4credits`, táblázatok | elemző hibák, útvonal-trükkök (`../`), erőforrás-kimerítés |
| 3 | Böngészőforrás (élő stúdió S9) | Qt WebEngine (Chromium) | a Chrome hibái, késve javítva |
| 4 | Hálózati bemenetek (élő stúdió) | SRT, RIST, WHIP/WebRTC, RTMP; libobs | távoli, hitelesítés nélküli adat |
| 5 | Vezérlőcsatorna, MCP, élő oldal | a motor helyi szerverei | helyi folyamatok, böngészőből érkező kérések |
| 6 | MI-modellek | helyi modellek betöltése | egyes formátumok (pl. Python pickle) betöltéskor kódot futtatnak |
| 7 | Bővítmények (OFX, LV2/VST3, saját) | idegen natív kód a folyamatban | teljes hozzáférés a folyamathoz |
| 8 | Betűk, képek, feliratok | FreeType/HarfBuzz, képdekóderek | elemző hibák |

## III. Linuxos eszközök (Ubuntu 26.04)

- **Landlock** (a rendszermagban, jogosultság nélkül használható): egy folyamat maga zárja le,
  mely könyvtárakhoz és fájlokhoz férhet hozzá, és (újabb ABI-n) mely TCP-portokhoz és jelekhez.
  Az Ubuntu 26.04 rendszermagja a 7.0 (a referenciagépen 7.0.0-43), benne jelzés-hatókör és
  naplózás. Rust: `landlock` crate 0.4.7, MIT OR Apache-2.0, „best effort”: régebbi rendszermagon a
  hiányzó részeket kihagyja.
- **seccomp-bpf**: a folyamat tiltja a nem szükséges rendszerhívásokat (pl. `execve`, `ptrace`,
  hálózati hívások). Rust: `seccompiler` 0.5.0 (rust-vmm), Apache-2.0 OR BSD-3-Clause.
- **Külön folyamatok**: a kockázatos részek külön folyamatban, a legszűkebb jogokkal; összeomlás
  után a felügyelő újraindítja (ez a never-crash szabálynak is jó).
- **AppArmor**: az Ubuntu 23.10 óta a jogosultság nélküli felhasználói névterek (user namespace)
  csak AppArmor-profillal engedélyezettek. A Chromium (és így a Qt WebEngine) sandboxa ezt
  használja, ezért a `cfa4-studio-host`-nak saját, `userns` szabályt adó profil kell a `.deb`-ben.
- **XDG-portálok**: fájlválasztás és képernyőrögzítés a felhasználó engedélyével (Wayland).

## IV. Felépítés (javaslat)

1. **`cfa4-sandbox` crate** (L1): egy helyen írja le a folyamattípusok profiljait (Landlock-
   szabályok + seccomp-szűrő), „best effort” alkalmazással és állapotjelentéssel
   (`security.status` parancs, Beállítások ▸ Biztonság). A `unsafe`-mentes Rust crate-ekre épül.
2. **Folyamatok és jogaik** (a D2 szerint):

| Folyamat | Olvas | Ír | Hálózat | Programindítás |
|---|---|---|---|---|
| Qt GUI (Python) | projekt, beállítások | beállítások | nincs (csak a motor helyi csatornája) | csak a motor |
| Motor (`cfa4-cli serve`) | a projekt és a hozzáadott médiafájlok könyvtárai, gyorsítótár | projekt, gyorsítótár, export célja | csak 127.0.0.1 | csak a saját segédfolyamatai |
| **Médiaszolgáltató** (`cfa4-media-host`, új): FFmpeg, GStreamer | az adott médiafájl | semmit (a képkockák osztott memórián mennek vissza) | nincs | nincs |
| Stúdió (`cfa4-studio-host`, élő stúdió D2) | a jelenet forrásai | felvétel célja | a beállított kimenetek | nincs |
| Böngészőforrás (a stúdión belül, Chromium-sandbox) | semmit | semmit | a betöltött oldal | nincs |
| Letöltő (`cfa4-fetch`, új, D5) | semmit | csak a letöltés célját (modell- vagy gyorsítótár-könyvtár) | csak a kért cím | nincs |
| Bővítmények, MI-modellek | a D7 szerint | | | |

3. **A szabályok (never crash, D4):** ha egy védelem hiányzik, a hozzá tartozó kockázatos rész nem
   indul, és a Beállítások ▸ Biztonság megmondja, mi hiányzik; a CFA4 többi része fut. Ha egy
   elszigetelt folyamat megsérti a szabályt vagy összeomlik, a felügyelő újraindítja, a felhasználó
   üzenetet kap, a projekt marad.

## V. Fázisok (javaslat)

| Fázis | Tartalom | Kész, ha |
|---|---|---|
| **B0** – Fenyegetésmodell | a II. táblázat részletesen (adatfolyamok, bizalmi határok), mérőkészlet a referenciagépen | a tulajdonos jóváhagyja; az overhead mérése kész |
| **B1** – `cfa4-sandbox` | crate, profilok, `security.status`, Beállítások ▸ Biztonság a Qt6-ban | egy tesztfolyamat a profiljával nem éri el a `~/.ssh`-t, nem indít programot |
| **B2** – Fuzzing | `cargo fuzz` cél minden L0 elemzőre (konténerek, kodekek) és a projekt-/cserefájlokra; éjszakai CI; később OSS-Fuzz | minden elemzőnek van célja, a talált hibák javítva |
| **B3** – A motor önkorlátozása | Landlock és seccomp a `cfa4-cli serve`-ben; a fájlválasztó adja hozzá az engedélyt (portál) | a meglévő tesztek zöldek, a tiltott hozzáférés tesztje bukik |
| **B4** – Médiaszolgáltató | FFmpeg és GStreamer külön folyamatba, osztott memóriás képkockákkal (D2) | a C dekóder összeomlása nem viszi a motort; a lassulás legfeljebb 5% (D2) |
| **B5** – Projekt- és cserefájlok | méret- és mélységkorlátok, útvonal-ellenőrzés (`../`, szimbolikus link), a hivatkozások listája és figyelmeztetés idegen projektnél, jóváhagyott médiára szűkített hozzáférés (D6) | rosszindulatú mintakészlet tesztje zöld |
| **B6** – Élő stúdió | a `cfa4-studio-host` profilja, AppArmor-profil a `.deb`-ben, a böngészőforrás négy védelme (LIVE-STUDIO S9), hálózati bemenetek a hoston | a négy védelem tesztje zöld |
| **B7** – MI-modellek és bővítmények | csak biztonságos modellformátumok, bővítmények külön folyamatban (a D7 szerint) | — |
| **B8** – Csomagolás és ellátási lánc | `.deb` (AppArmor-profilok), AppImage és Flatpak viselkedése, SBOM, aláírt kiadás | a csomagteszt a profilokat ellenőrzi |
| **B9** – Biztonsági tesztkészlet | rosszindulatú média- és projektminták (a `target/fixtures/`-ben generálva), „mit nem érhet el” tesztek minden folyamatra | a CI-ban fut |

## VI. Ütemezés (javaslat)

B0, B1 és B2 nem érinti a Qt6 felület fájljait, így most indulhatnak; a Beállítások ▸ Biztonság
oldal a Qt6 befejezése után. A B6 az élő stúdió S9-ével együtt.

## VII. Döntések (lépésenként egyeztetve)

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Hatókör | **Eldöntve (2026-11-11): futásidejű réteg és fuzzing együtt.** A fuzzing a hibákat keresi meg, mielőtt a támadók tennék; a futásidejű réteg a javítatlanok kárát korlátozza |
| D2 | Az FFmpeg és a GStreamer helye | **Eldöntve (2026-11-11): külön `cfa4-media-host` folyamat** (B4), mindig; a saját Rust dekóderek a motorban maradnak. A külső dekóder csak a megnyitott fájlt olvashatja, nem ír, nincs hálózata, nem indít programot; a képkockák osztott memórián mennek a motornak; összeomlás után a motor újraindítja. Teljesítménycél: a lejátszás és az export a külső dekóderrel megnyitott fájlokon legfeljebb 5%-kal lassulhat (a referenciagépen mérve); ha többet, az optimalizálandó, nem ok a visszatérésre a motorba |
| D3 | Eszközök | **Eldöntve (2026-11-11): Landlock és seccomp a saját Rust kódban** (`cfa4-sandbox`, a `landlock` és a `seccompiler` crate): minden folyamat induláskor maga zárja le magát, csomagformátumtól függetlenül, rendszergazdai jog nélkül. **AppArmor-profil csak ahol kell:** a `.deb` telepíti a `cfa4-studio-host`-hoz (a Chromium-sandbox `userns` szabálya). A bubblewrap nem része a rétegnek |
| D4 | Ha egy védelem hiányzik | **Eldöntve (2026-11-11): alapból szigorú.** A minimális követelmény az Ubuntu 26.04, amelyben a Landlock, a seccomp és az AppArmor gyárilag megvan, így a támogatott rendszeren a védelmek mindig működnek (B0-ban ellenőrizve). Ha egy hiányzik, a hozzá tartozó kockázatos rész (külső dekóder, böngészőforrás, hálózati bemenet) nem indul, és a CFA4 megmondja, mi hiányzik; a szerkesztő, a projekt és a saját Rust dekóderek működnek (never crash). A felhasználó kifejezett figyelmeztetés után a Beállításokban engedélyezheti a védelem nélküli futást, de **az így futó CFA4 nem támogatott**: a `security.status` és a hibajelentés ezt rögzíti, és az erre épülő hibajelentéseket nem kezeljük |
| D5 | Hálózat | **Eldöntve (2026-11-11): csak ahol kell.** A motor csak a 127.0.0.1-et éri el (Qt GUI, MCP, élő oldal), kifelé nem nyithat kapcsolatot; a `cfa4-media-host` soha nem kap hálózatot; kifelé csak a `cfa4-studio-host` lép (a beállított streamelési célok és a hálózati bemenetek); a felhasználó által kért letöltések (MI-modellek, frissítésellenőrzés) egy külön, csak erre szolgáló kis folyamaton (`cfa4-fetch`) mennek, és mindig látszik, mit tölt le és honnan. Ahol a rendszermag Landlock-ABI-ja engedi, portszinten is kikényszerítve, egyébként seccomppal |
| D6 | Idegen projekt megnyitása | **Eldöntve (2026-11-11): ellenőrzés és figyelmeztetés.** Minden projekt- és cserefájlra méret- és mélységkorlát (a túllépés hibaüzenet, nem összeomlás). Megnyitás előtt a CFA4 megmutatja, mely külső útvonalakra hivatkozik a projekt, és külön figyelmeztet a szokatlanokra (rejtett mappák, a projekten kívüli rendszerkönyvtárak, `..`-os vagy szimbolikus linken át kilépő útvonalak); a motor csak a jóváhagyott médiafájlokhoz kap Landlock-hozzáférést. Az ezen a gépen készült projekt, amelynek hivatkozásai a felhasználó saját mappáiba mutatnak, kérdés nélkül nyílik |
| D7 | MI-modellek és bővítmények | modellek csak biztonságos formátumban (safetensors, ONNX, GGUF; pickle nincs); natív bővítmények külön folyamatban |
| D8 | Ütemezés | B0–B2 most, a többi a Qt6 után (VI.) |

## VIII. Kockázatok

- **Sebesség:** a külön folyamat és az osztott memória másolást és késleltetést hozhat; a D2 célja
  legfeljebb 5%, a referenciagépen mérve (NUMA: a médiafolyamat és a motor ugyanazon a csomóponton).
- **Túl szűk szabályok:** egy elfelejtett könyvtár (betűk, GPU-illesztő, ALSA/PipeWire) hibát
  okozhat; ezért a profilok tesztje a csomagolt alkalmazással fut, és a hibaüzenet megnevezi a
  hiányzó jogot.
- **GPU-illesztők:** az NVIDIA- és Mesa-illesztők sok fájlt és ioctl-t használnak; a seccomp-szűrő
  a GPU-s folyamatokban engedékenyebb.
- **AppImage:** nem tud AppArmor-profilt telepíteni, ezért ott a böngészőforrás a D4 szerint nem
  indul (a `.deb` vagy a Flatpak kell hozzá); ezt a letöltési oldal és a hibaüzenet is megmondja.

## IX. Források

- Ubuntu 26.04 biztonsági újdonságai (rendszermag 7.0, Landlock, AppArmor 5.0 `userns`):
  <https://mondoo.com/blog/whats-new-in-ubuntu-2604-security>
- A felhasználói névterek AppArmor-korlátozása:
  <https://discourse.ubuntu.com/t/understanding-apparmor-user-namespace-restriction/58007>
- `landlock` crate: <https://docs.rs/crate/landlock/latest>; `seccompiler` crate:
  <https://docs.rs/crate/seccompiler/0.4.0> (licencek: `cargo info`, 2026-11-11)
