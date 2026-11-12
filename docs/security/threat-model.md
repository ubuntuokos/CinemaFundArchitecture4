# CFA4 fenyegetésmodell (CFA4-SECURITY-001, B0)

**Verzió:** 1.1 (a tulajdonos jóváhagyta 2026-11-12-én; a mérés lefutott, a B0 lezárult)
**Dátum:** 2026-11-11, jóváhagyva 2026-11-12
**Terv:** [`CFA4-SECURITY-001`](../plans/CFA4-SECURITY-001.md) V. fejezet, B0 fázis
**Kész, ha:** a tulajdonos jóváhagyja ezt a dokumentumot, és a többletterhelés mérése lefutott
(`scripts/dev/security-overhead.sh`, lásd 6. pont). **Teljesült 2026-11-12-én.**

Ez a dokumentum a terv II. fejezetének táblázatát bontja ki: kitől védünk, mit, hol lép be az idegen
adat, melyik folyamat dolgozza fel, mi védi ma, és melyik fázis zárja le. A B1-ben elkészült
`cfa4-sandbox` crate ([README](../../crates/sandbox/README.md)) a 4. pont folyamatprofiljait
valósítja meg.

## 1. Mit védünk

| # | Érték | Miért fontos |
|---|---|---|
| E1 | A felhasználó saját fájljai (`~/.ssh`, böngészőprofil, jelszókezelő, dokumentumok, más projektek) | egy feltört dekóder innen lopna |
| E2 | A nyitott projekt és a még nem mentett munka | never crash (AGENTS.md §0): órák munkája |
| E3 | A felhasználó gépe mint program-futtató környezet | egy feltört folyamat ne indíthasson programot, ne telepíthessen semmit |
| E4 | A hálózat és a felhasználó fiókjai | ne legyen adatkiszivárgás és ne legyen a gép támadási pont |
| E5 | A CFA4 hitelessége | aláírt kiadás, ellenőrizhető függőségek (B8) |

## 2. Kitől védünk

| Támadó | Mit tud | Példa |
|---|---|---|
| T1 Rosszindulatú fájl készítője | a felhasználó megnyit egy általa készített médiát, projektet, cserefájlt, betűt, modellt | „nézd meg ezt a klipet” e-mailben, letöltött sablonprojekt |
| T2 Weboldal | a böngészőforrás betölti, vagy a felhasználó böngészőjéből a helyi portokra küld kérést | élő stúdió S9, DNS-rebinding a vezérlőcsatorna ellen |
| T3 Hálózati partner | adatfolyamot küld az élő stúdió bemenetére | SRT/RIST/WHIP/RTMP forrás |
| T4 Bővítmény vagy modell szerzője | natív kódot vagy betöltéskor futó modellt ad | OFX/LV2/VST3 bővítmény, pickle-alapú `.ckpt` |
| T5 Helyi, más felhasználói fiók | ugyanazon a gépen fut | a helyi portok elérése |

**Nem célunk** (a modell határai): a root jogú vagy a felhasználó nevében már kódot futtató támadó
(az a gép elvesztése), a kernel hibái (a Landlock és a seccomp a kernelre épül), a fizikai hozzáférés,
és a rendszer csomagforrásainak kompromittálása (az Ubuntu feladata; a saját kiadásunkat a B8 védi).

## 3. Belépési pontok, adatfolyamok és bizalmi határok

A bizalmi határ ott van, ahol a T1–T5 által irányított bájtok először egy CFA4-folyamatba érnek. A
cél: minden határ mögött olyan folyamat álljon, amelynek profilja (4. pont) a lehető legszűkebb.

| # | Belépési pont | Adatfolyam | Feldolgozó ma (2026-11-11, `main`) | Ami ma véd | Lezárja |
|---|---|---|---|---|---|
| P1 | Médiafájlok | fájl → motor (`cfa4-cli serve`) → saját L0 elemző/dekóder; vagy → `cfa4-ffmpeg` / `cfa4-gstreamer` (C kód) **a motor folyamatában** | motor | Rust, `unsafe` csak a `crates/platform`-ban; AGENTS.md §0.4 (minden számot ellenségesnek tekintünk); a crate-ek mutációs tesztjei (`fuzz_hostile`, `robustness`) | B2 (fuzzing), B3 (motor önkorlátozása), B4 (`cfa4-media-host`) |
| P2 | Projekt- és cserefájlok (`.fcproj`, OTIO, XML/EDL/AAF, `.cfa4credits`, táblázatok) | fájl → motor → `cfa4-project` / `cfa4-interchange` / `cfa4-cfb` | motor | `Result`-alapú elemzők; a DCP-nek már van `Limits` korlátja | B2, B5 (korlátok, útvonal-ellenőrzés, figyelmeztetés: D6) |
| P3 | Böngészőforrás | weboldal → Qt WebEngine (Chromium) | (még nincs; élő stúdió S9) | – | B6 (Chromium-sandbox, AppArmor-profil a `.deb`-ben, LIVE-STUDIO S9 négy védelme) |
| P4 | Hálózati bemenetek (SRT, RIST, WHIP/WebRTC, RTMP; libobs) | hálózat → stúdió | (még nincs) | – | B6 (a bemenet a `cfa4-studio-host`-ban, D5) |
| P5 | Vezérlőcsatorna, MCP, élő oldal | helyi folyamat vagy böngésző → 127.0.0.1 → motor / Qt GUI | motor, Qt GUI | csak 127.0.0.1 (Semgrep-szabály), 64 hex karakteres munkamenet-token (`CFA4_CONTROL_TOKEN`), soronként 4 MiB, 16 kapcsolat | B3 (Landlock-portszabály: csak a saját port; D5) |
| P6 | MI-modellek | letöltés → fájl → motor (`cfa4-speech`, `cfa4-tts`: safetensors a candle-lel) | motor; **a letöltés ma a motorban fut** (`cfa4-speech` `download` funkció, `ureq` + rustls) | csak safetensors-betöltés a meglévő kódban | B7 (D7: csak safetensors/ONNX/GGUF, elszigetelt átalakító); a letöltés átköltözik a `cfa4-fetch`-be (D5) |
| P7 | Bővítmények (OFX, LV2/VST3, saját) | natív kód → motor | (még nincs) | – | B7 (külön `pluginHost` folyamat, D7) |
| P8 | Betűk, képek, feliratok | fájl → `cfa4-text` (harfrust, saját raszterizáló), képdekóderek, `cfa4-captions` | motor, Qt GUI (Qt saját FreeType/HarfBuzz-a) | Rust-elemzők a motorban | B2 (fuzz a betű- és feliratelemzőkre), B3 |

**Megállapítások a mai kódban** (a B3-hoz és B7-hez):

1. **A modelletöltés a motorban fut** (P6). A D5 szerint a motor kifelé nem nyithat kapcsolatot, ezért
   a B3 előtt a letöltést a `cfa4-fetch` folyamatba kell költöztetni; addig a motor profilja nem
   lehet `Loopback`. A `security.status` a `fetch` profilt már mutatja.
2. **Az FFmpeg és a GStreamer a motor folyamatában fut** (P1). A B3 önmagában a fájlrendszert és a
   hálózatot zárja le a C dekóder előtt, de egy C-hiba a motort (és a nem mentett munkát) viszi; ezt
   a B4 oldja meg.
3. **A vezérlőcsatorna portszintű védelme** (P5) a Landlock 4-es ABI-jától működik (Ubuntu 26.04:
   8-as ABI, 7.0-s rendszermag); a 127.0.0.1-re kötést a Landlock nem tudja kikényszeríteni (csak
   portot), azt a kód és a Semgrep-szabály védi.

## 4. Folyamatok és jogaik

A terv IV.2 táblázata, a `cfa4-sandbox` profiljaiként (`Profile::for_process`). Minden folyamat
olvassa a rendszer könyvtárait (`/usr`, `/lib*`, `/etc`, `/opt/cfa4`, `/sys`, a saját `/proc`
bejegyzéseit), de **soha nem a `$HOME`-ot** és nem más folyamatok `/proc` bejegyzéseit; ezen felül csak
azt, amit a profil megad.

| Folyamat (`kind`) | Fájlrendszer | Hálózat | Programindítás | seccomp |
|---|---|---|---|---|
| Qt GUI (`qtGui`) | olvas: projekt, betűk; ír: beállítások, futásidejű könyvtár | csak a megadott helyi portok | csak a motor | tiltólista |
| Motor (`engine`) | olvas: hozzáadott média; ír: projekt, gyorsítótár, beállítások, export célja | csak a megadott helyi portok | csak a saját segédfolyamatai | tiltólista |
| Médiaszolgáltató (`mediaHost`) | csak a megnyitott fájl, olvasásra | nincs (IPv4/IPv6 socket sem) | nincs | tiltólista + `execve` + `socket(AF_INET*)` |
| Stúdió (`studioHost`) | olvas: jelenet forrásai; ír: felvétel célja | bármi (streamelési célok, bemenetek) | a Chromium segédfolyamata | tiltólista, de névterek engedve (Chromium-sandbox) |
| Böngészőforrás (`browserSource`) | a Chromium-sandbox és az AppArmor zárja (B6), nem a `cfa4-sandbox` | | | |
| Letöltő (`fetch`) | ír: csak a letöltés célja | csak kimenő 443-as port | nincs | tiltólista + `execve` |
| Bővítmény (`pluginHost`) | olvas: a bővítmény saját fájljai | nincs | nincs | tiltólista + `execve` + `socket(AF_INET*)` |
| Modell-átalakító (`modelConverter`) | olvas: forrásmodell; ír: a kimeneti fájl | nincs | nincs | tiltólista + `execve` + `socket(AF_INET*)` |

A seccomp-tiltólista (minden folyamatban, `EPERM`): `ptrace`, `process_vm_readv/writev`, kernelmodulok,
`mount` és az új mount-API, `chroot`, `pivot_root`, névterek (`unshare`, `setns`; a stúdió kivételével),
kulcstartók, órák állítása, `io_uring`, `bpf`, `perf_event_open`, `userfaultfd`,
`open_by_handle_at`. Engedélylistára (csak a szükséges hívások) a profilok a csomagolt alkalmazással
végzett tesztek (B8) után válthatnak; a GPU-illesztők miatt a GPU-s folyamatok szűrője marad
engedékenyebb (terv VIII.).

**D4 ellenőrzése:** a támogatott rendszeren (Ubuntu 26.04, 7.0-s rendszermag) a Landlock 8-as
ABI-ja, a seccomp-szűrő és az AppArmor gyárilag megvan; a `security.status` minden folyamatra
`canStart: true`-t ad. Ha bármelyik hiányzik, a `cfa4-sandbox` szigorú módja hibát ad a hiányzó
védelem nevével, és a hozzá tartozó kockázatos rész nem indul. A B1 tesztje ezt egy 6.18-as
rendszermagon (Landlock 8-as ABI) valódi korlátozással ellenőrizte: a médiaszolgáltató profiljával
futó folyamat nem olvassa a `~/.ssh`-t, nem indít programot, nem nyit IPv4/IPv6 socketet.

## 5. Fázisok és maradó kockázat

| Fázis | Mit zár le a fenti táblákból | Maradó kockázat utána |
|---|---|---|
| B1 (kész) | a profilok és a `security.status`; tesztfolyamat a profiljával | a motor és a GUI még nem korlátozza magát |
| B2 | P1, P2, P8 elemzőhibái: `cargo fuzz` cél minden L0 elemzőre és a projekt-/cserefájlokra, éjszakai CI | a fuzzer által el nem ért kódutak |
| B3 | P1, P2, P5, P8 a motorban: Landlock + seccomp a `cfa4-cli serve`-ben | a C dekóder hibája még a motort viszi (B4-ig) |
| B4 | P1 C kódja külön folyamatban (D2) | a saját Rust dekóderek logikai hibái a motorban (a fuzzing és a §0 kezeli) |
| B5 | P2: korlátok, útvonal-ellenőrzés, figyelmeztetés (D6) | – |
| B6 | P3, P4 a stúdióban | a Chromium javítatlan hibái a Chromium-sandboxon belül |
| B7 | P6, P7 (D7) | a valós idejű hangbővítmény késleltetése (mérendő) |
| B8, B9 | csomagolás, AppArmor-profilok, SBOM, aláírás; „mit nem érhet el” tesztek minden folyamatra a CI-ban | – |

## 6. Mérőkészlet a referenciagépen

A D2 5%-os célja a `cfa4-media-host`-ra vonatkozik (B4-ben mérjük, ha a folyamat létezik). A B0 a
mechanizmusok saját költségét méri, a legrosszabb esetre: rendszerhívás-sűrű ciklus (megnyitás,
olvasás, lezárás, `stat`, könyvtárlistázás) kis fájlokon, korlátozás nélkül és a `pluginHost`, illetve
az `engine` profillal, minden eset új gyermekfolyamatban, váltakozó sorrendben.

```sh
scripts/dev/security-overhead.sh            # 0-7. mag, 2000 fájl, 20 kör
scripts/dev/security-overhead.sh 0-7 5000 50
```

A szkript a `reference-machine.md` szerint rögzíti az `lscpu` kimenetét, a rögzített magokat és a
terhelést, és ha van `numactl`, a 0. NUMA-csomóponton fut. A jelentés: `target/audit/security-overhead/<idő>/`
(`overhead.json`, `record.txt`, `lscpu.txt`); `overheadPct` az esetenkénti eltérés a korlátozás
nélküli mediántól, `applyUs` a profil alkalmazásának ideje.

**A B0 mérése (2026-11-12, felhőben):** a tulajdonos döntése szerint (2026-11-12: „a felhőben megy a
fejlesztés”) a B0 mérése a felhős fejlesztői környezetben fut, nem a referenciagépen. Környezet:
4 vCPU (Intel Xeon @ 2,10 GHz, megosztott virtuális gép), 6.18-as rendszermag, Landlock 8-as ABI,
seccomp; más terhelés nélkül (`scripts/dev/security-overhead.sh 0-3`, négy futás, 2000–5000 fájl,
30–40 kör).

| Eset | Profil alkalmazása | Fájlonként (megnyitás, olvasás, `stat`) | Többlet |
|---|---|---|---|
| korlátozás nélkül | – | 2,7–3,5 µs | – |
| `pluginHost` | 0,55–0,62 ms | +0,4–0,5 µs | +8–16% |
| `engine` | 0,49–0,62 ms | +0,4–1,2 µs | +17–35% (egy kiugró futás) |

**Értelmezés:** a többlet a Landlock útvonal-ellenőrzése minden fájlmegnyitáskor (néhány tized
mikroszekundum) és a seccomp-szűrő minden rendszerhíváson; a megosztott virtuális gép miatt a
szórás nagy. Ez a legrosszabb eset: a ciklus csak rendszerhívásokat végez. A lejátszás és az export
másodpercenként néhány fájlt nyit meg és sokszor nagyobb blokkokat olvas, így ott a többlet ennek
töredéke; a profil alkalmazása folyamatonként egyszeri, kb. 0,6 ms. A D2 5%-os célját a B4 a
valódi lejátszáson és exporton méri (ugyanezzel a módszerrel, a felhőben). **Ezzel a B0 lezárult.**

## 7. Jóváhagyás

**A tulajdonos (`ubuntuokos`) 2026-11-12-én jóváhagyta** a dokumentumot:

1. Elfogadva: a 2. pont határai (mi nem cél).
2. Elfogadva: a 3. pont megállapításai és helyük a fázisokban (a modelletöltés a B3 előtt a
   `cfa4-fetch`-be költözik).
3. Kész: a mérés a tulajdonos döntése szerint a felhős fejlesztői környezetben futott (2026-11-12),
   az eredmény a 6. pontban. Ezzel a B0 lezárult.
