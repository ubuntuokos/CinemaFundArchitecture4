# CFA4-SETTINGS-MENU-001 – CFA4 Beállítások és Hardver menüterv

> **Átvétel a repóba (2026-11-27, `docs/coordination.md` §2.11):** ez a terv 2026-10-26-án Claude-dokumentumként
> készült (<https://claude.ai/artifact/7tCP6EREnCdZ43owCSJPBC>). A tartalma itt változatlan, csak Markdownra
> alakítva; ettől kezdve ez a fájl az irányadó. Az 1. fázis (PR #122) és a 2. fázis (PR #169) a `main`-ben van; a
> következő lépés: [`CFA4-SETTINGS-HW-002.md`](CFA4-SETTINGS-HW-002.md). A szövegben hivatkozott „előző terv”
> (root menü, <https://claude.ai/artifact/GcFzoqUGJK7KkmFq8UQLYd>) ennek a tervnek az elődje; a repóban:
> [`CFA4-SETTINGS-ROOT-001-2026-10-26.md`](CFA4-SETTINGS-ROOT-001-2026-10-26.md), a root-rész mostani terve:
> [`CFA4-SETTINGS-HW-003.md`](CFA4-SETTINGS-HW-003.md).

2026-10-26 · ubuntuokos

A Beállítások ablak nyolc csoportba rendeződik, köztük egy új „Hardver és eszközök” csoporttal, amelynek utolsó aloldala a „Rendszerszintű beállítások (root)”. A terv három forrásra épül: a CFA3 Control Center képtervekre, a Final-Architecture-v3.0 Rendszerbeállítások vázlatára és a CFA4 jelenlegi Beállítások ablakára. A meglévő 16 CFA4-oldal megmarad, csak csoportba kerül.

## Alapelvek

A három forrás ugyanazokat az elveket képviseli; ezeket a CFA4 Beállítások ablakának minden oldalára alkalmazni kell.

1. **Felderítés csak olvasva.** A Hardver oldalak megmutatják, mit talált a rendszer, de semmit nem írnak át pusztán megnyitásra vagy frissítésre (FA3: „Read-only host discovery”).
2. **Program-beállítás, nem rendszer-beállítás.** Az alapértelmezett kamera, hangeszköz vagy gyorsító kiválasztása a CFA4 saját beállítása; az operációs rendszer alapértelmezését nem módosítja (FA3: Kamera / Print / Scan).
3. **Rendszerszintű változtatás csak tervezeten át.** A felület „változtatási tervezetet” készít, amelyet a felhasználó lát és jóváhagy, és egy külön segédprogram hajt végre. A felületről root-parancs nem futtatható (FA3: ChangeSet; CFA3: Biztonság és jóváhagyás).
4. **CPU az alap, gyártófüggetlen gyorsítás.** GPU vagy NPU nélkül is minden működik; egy gyártóhoz kötött út csak a gyártófüggetlen párja mellett lehet (CFA4 VISION 2/4; CFA3 README).
5. **Nincs rejtett csere és nincs automatikus hangolás.** Ha a kért gyorsító nem érhető el, a menü ezt kiírja; nem vált csendben másik eszközre. Nem biztonságos hardverhangolást a CFA4 magától soha nem kapcsol be (CFA3 README).
6. **Bizonyíték szerinti állapot.** Minden eszköz és komponens állapota látszik: észlelve, ellenőrizetlen, tesztelve, nem támogatott. Mintaadat soha nem jelenhet meg valós adatként (CFA3: „Mintanézet” jelölés).
7. **Minden vezérelhető.** Minden gomb CFA4-parancs, minden vezérlő automatizálási azonosítót kap, ahogy a CFA4 szabályai előírják.

## Menüfa

A bal oldali lista csoportfejléceket kap, az oldalak behúzva alattuk, a CFA3-képtervhez hasonlóan. A „Megvan” oszlop a mai CFA4-oldalt mutatja; az új oldalak a Ütemezés részben szereplő szakaszokban készülnek el.

| Csoport | Oldal | Tartalom | Megvan | Forrás |
|---|---|---|---|---|
| Általános | Általános | Indítás, nyelv, legutóbbi projektek, **felület módja: Egyszerű / Teljes** | részben (`general`) | CFA4, OpenShot |
| Általános | Helyek | Alapértelmezett projekt-, import-, export- és felvételi mappa, ideiglenes mappa | új | Kdenlive Environment, OpenShot Location |
| Általános | Automatikus mentés | Mentési időköz, verziók, helyreállítás, **visszavonási előzmények hossza** | részben (`autoSave`) | CFA4, OpenShot |
| Általános | Gyorsbillentyűk | Hivatkozás a meglévő Billentyűparancsok ablakra | ablakként igen | mind |
| Általános | Bővítmények | Bővítmények, fejlesztői mód | igen (`plugins`) | CFA4 |
| Megjelenés | Megjelenés | Téma, méretezés, világos/sötét | igen (`appearance`) | mind |
| Megjelenés | Címkék és jelölők | Címkeszínek, előbeállítások, **jelölőkategóriák** | részben (`labels`) | CFA4, Kdenlive Colors and Markers |
| Szerkesztés | Projekt-alapértelmezések | Alapértelmezett profil (felbontás, fps), sávok száma, hangcsatornák | új | Kdenlive Project Defaults, OpenShot General |
| Szerkesztés | Idővonal | Időtartamok, görgetés, **sávmagasság, bélyegkép-stílus, automatikus áttűnés** | részben (`timeline`) | mind |
| Szerkesztés | Vágás | Trim-beállítások | igen (`trim`) | CFA4 |
| Szerkesztés | Grafika | Szöveg- és grafika-alapértelmezések | igen (`graphics`) | CFA4 |
| Szerkesztés | Szín | Szín- és HDR-beállítások | igen (`color`) | CFA4 |
| Szerkesztés | Monitorok és mérőeszközök | Rács, biztonsági terület, waveform/vektorszkóp alapértelmezései | új | Kdenlive Playback, OpenShot Scopes |
| Média és lejátszás | Média | Import, időkód, skálázás, képsorozatok, Camera RAW | igen (`media`) | CFA4 |
| Média és lejátszás | Dekódolás és kódolás | Kodekenkénti háttér, hardveres dekódolás/kódolás, szálak (lásd: Kodekek) | új | kódexparítás, ADR 0003, Kdenlive, OpenShot |
| Média és lejátszás | Proxy és előnézet | Proxy engedélyezése, min. méret, proxyprofil, kamerák saját proxyjai, idővonal-előnézet profilja | részben (`media.enableProxies`) | Kdenlive Proxy Clips, OpenShot Optimize |
| Média és lejátszás | Átkódolás | Nem támogatott források átkódolási előbeállításai, eredeti cseréje | új | Kdenlive Transcode |
| Média és lejátszás | Média-gyorsítótár | Hely, tisztítás, **lemezes képkocka-gyorsítótár** | részben (`mediaCache`) | CFA4, OpenShot Cache |
| Média és lejátszás | Lejátszás | Preroll, vázlatdekódolás, képkocka-eldobás | igen (`playback`) | CFA4, Kdenlive |
| Média és lejátszás | Hang | Hangkeverés, mintázás | igen (`audio`) | CFA4 |
| Hardver és eszközök | Áttekintés | Minden felismert eszköz egy listában | új | CFA3 |
| Hardver és eszközök | Gyorsítók (GPU / NPU) | Eszközök, előnyben részesített gyorsító, dekódoló- és kódoló-GPU választása | új (`cfa4-accel`) | FA3, OpenShot Performance |
| Hardver és eszközök | Memória | RAM-tartalék, képkocka-gyorsítótár | igen (`memory`), átkerül | CFA4, OpenShot Cache |
| Hardver és eszközök | Hangeszközök | Bemenet, kimenet, puffer, hangháttér | igen (`audioHardware`), átkerül | CFA4, Kdenlive, OpenShot Preview |
| Hardver és eszközök | Kijelzők és külső kimenet | Monitorok, HDR, teljes képernyős monitor, **külső videókimenet (Blackmagic)** | új | CFA3, Kdenlive Playback |
| Hardver és eszközök | Tárolóeszközök | Lemezek, szabad hely, munkalemez | új | CFA3 |
| Hardver és eszközök | Felvétel: kamerák, kártyaolvasók, képernyő | USB/PTP, V4L2, Decklink, képernyőfelvétel, hangfelvétel | új | CFA3, Kdenlive Capture |
| Hardver és eszközök | Rajztáblák | Toll, nyomásgörbe, kijelző-hozzárendelés | új | CFA3 |
| Hardver és eszközök | Vezérlőeszközök | MIDI, vezérlőfelületek, **jog/shuttle** | új | FA3, Kdenlive JogShuttle |
| Hardver és eszközök | Rendszerszintű beállítások (root) | Jóváhagyáshoz kötött rendszerkomponensek | új | CFA3, FA3 |
| Helyi MI | MI általános | MI-funkciók be/ki, futtatási eszköz (CPU / GPU / NPU), kijelző-GPU védelme, memória-/VRAM-korlát, jóváhagyási szabályok | új | CFA3, FA3, VISION 5. |
| Helyi MI | Beszéd, felirat, fordítás | Whisper-átírás, nyelv, beszélők, fordítómodell, felolvasás (Kokoro), szinkron | részben (`mediaAnalysis`) | CFA4, Kdenlive Plugins |
| Helyi MI | Kép, videó és hang | Maszkolás, követés, jelenet- és csendfelismerés, színillesztés, zaj- és zeneleválasztás | új | VISION 5., Kdenlive K4, K26 |
| Helyi MI | Modellek | Telepített modellek, licenc, forrás, hash, méret, hely, eszköz, FP16 tiltása, letöltés jóváhagyással | új | Kdenlive Plugins, K18, K31 |
| Közzététel és szolgáltatók | Közzététel | Célplatformok, alapértelmezett kiadási előbeállítás, láthatóság, export utáni lépések | új | VISION 4., K11, K15 |
| Közzététel és szolgáltatók | Online források | Hugging Face, Freesound, Pixabay; letöltési mappa, licencszűrő | új | Kdenlive K18 |
| Közzététel és szolgáltatók | Fiókok és tokenek | Szolgáltatónkénti belépés és API-token, állapot, visszavonás | új | FA3 Token Control Center |
| Közzététel és szolgáltatók | Hálózat és adatvédelem | Offline mód, proxy, szolgáltatónkénti hálózati engedély, felhős MI (alapból ki) | új | FA3 External Providers |
| Rendszer | Külső programok | Blender, kép-, hang- és animációszerkesztő elérési útja | új | Kdenlive Environment, OpenShot General |
| Rendszer | Napló és diagnosztika | Naplószint, FPS-kijelzés, hibajelentés-csomag | új | FA3, OpenShot Advanced |
| Rendszer | Frissítések | Frissítési csatorna, komponensverziók | új | CFA3, FA3 |

A Hálózat és a külön Biztonság oldal az FA3-ban az MI-szolgáltatók miatt kellett; a CFA4-ben a helyi MI miatt egyelőre nem szükséges, a jóváhagyás a root-aloldalon belül marad.

## Hardver és eszközök

Minden eszközoldal ugyanazt a CFA3-mintát követi: bal oldalon az eszközök listája állapottal, jobb oldalon a kiválasztott eszköz részletei fülekre bontva. Az arányokat a mai 900 × 600 pontos Beállítások ablakhoz kell igazítani, ezért a lista keskenyebb, mint a képterven.

**Közös eszközoldal-minta**

- **Fejléc:** „Felismerés” (újraolvasás) és „Tesztelés” gomb, valamint az utolsó felismerés ideje.
- **Eszközlista:** név, kapcsolat (USB, PCI, Wi-Fi), állapot: csatlakoztatva / offline / nem vizsgált / nem támogatott.
- **Részletek fülei:** Adatok (típus, azonosító, meghajtó, firmware), Képességek (pipás lista: mit tud használni belőle a CFA4), Beállítások (csak CFA4-beállítások), Meghajtók (állapot: ellenőrizetlen / rendben / hiányzik, hivatkozás a root-aloldalra, ha telepítés kell), Diagnosztika (időbélyeges napló, „Kapcsolat rendben” jelzés).
- **Alsó sáv:** a meglévő Súgó / Visszaállítás / Mégse / OK gombok, ahogy ma.

**Oldalanként**

| Oldal | Felderítés forrása | CFA4-beállítások | Első körben |
|---|---|---|---|
| Áttekintés | Az összes alábbi | – (összesítő, figyelmeztetésekkel) | igen |
| Gyorsítók (GPU / NPU) | `cfa4-accel`: `/sys/class/drm`, `/sys/class/accel`, PCI-azonosító, meghajtó | Előnyben részesített gyorsító; munkatípusonként Auto / adott háttér / CPU; gyártóhoz kötött hátterek engedélyezése; a kijelzőt hajtó GPU MI-re használható-e | igen |
| Memória | Rendszermemória | RAM-tartalék, képkocka-gyorsítótár (mai mezők) | áthelyezés |
| Hangeszközök | Meglévő platform-hanglista | Mai `audioHardware` mezők | áthelyezés |
| Kijelzők | Wayland/X11 kijelzőlista | Teljes képernyős lejátszás kijelzője, HDR-megjelenítés | később |
| Tárolóeszközök | Csatolt fájlrendszerek | Munkalemez, gyorsítótár-lemez, figyelmeztetési küszöb | később |
| Kamerák és kártyaolvasók | USB/PTP, V4L2, cserélhető adathordozók | Import utáni műveletek, alapértelmezett webkamera | később |
| Rajztáblák | libinput / Wacom-eszközök | Nyomásgörbe, gombok, kijelző-hozzárendelés | később |
| MIDI és vezérlőfelületek | ALSA MIDI | Végpont, leképezési profil a Keverőhöz | később |

A GPU-ra hivatkozó MI-beállítás („kijelző-GPU védelme”) a CFA3-terv elvét veszi át: alapértelmezésben a kijelzőt hajtó GPU nem fut MI-feladatot, ha van másik gyorsító.

## Átvétel a Kdenlive-ból és az OpenShotból

A két program beállításainak képességeit vesszük át, a kódjukat nem; a fenti menüfában a „Forrás” oszlop mutatja, mi honnan jön. Forrás: a Kdenlive mai főágának beállítás-ablaka és `kdenlivesettings.kcfg` fájlja, valamint az OpenShot `_default.settings` fájlja.

| Program » oldal | Mit veszünk át | Hova kerül |
|---|---|---|
| Kdenlive » Misc | Alapértelmezett időtartamok, összeomlás utáni helyreállítás, gyorsítótár-tisztítás | meglévő Idővonal, Automatikus mentés, Média-gyorsítótár |
| Kdenlive » Project Defaults | Profil, sávszám, hangcsatornák | Projekt-alapértelmezések |
| Kdenlive » Proxy Clips | Proxy engedélyezése, min. méret, profil, képproxy, kamerák saját proxyjai (K12) | Proxy és előnézet |
| Kdenlive » Timeline | Sávmagasság, bélyegképek, hullámforma, automatikus előnézet-renderelés | Idővonal |
| Kdenlive » Environment | Feldolgozási, proxy- és kódolási szálak, ideiglenes mappa, alapértelmezett mappák, külső programok | Dekódolás és kódolás, Helyek, Külső programok |
| Kdenlive » Colors and Markers | Jelölőkategóriák és színek | Címkék és jelölők |
| Kdenlive » Plugins | Whisper/Vosk, modellmappa, eszközválasztás (CPU/GPU), FP16 tiltása, SAM-maszk | MI-elemzés, Modellek |
| Kdenlive » Playback | Hangháttér, teljes képernyős monitor, külső Blackmagic-kimenet, monitorrács | Hangeszközök, Kijelzők és külső kimenet, Monitorok és mérőeszközök |
| Kdenlive » Capture | Képernyő-, hang- és Decklink-felvétel | Felvétel |
| Kdenlive » JogShuttle | Jog/shuttle eszköz és gombkiosztás | Vezérlőeszközök |
| Kdenlive » Transcode | „Vágásbarát” átkódolási profilok | Átkódolás |
| Kdenlive » Project (rejtett) | VA-API / AMF / NVIDIA beállítások, hardveres dekódolás, támogatott hardveres kodekek | Dekódolás és kódolás (az AMF nem, ADR 0003 §5) |
| OpenShot » General | Egyszerű / Teljes felület (VISION: kezdőbarát mód), alapértelmezett profil, Blender útvonala | Általános, Projekt-alapértelmezések, Külső programok |
| OpenShot » Timeline | Sávméret, sávszám, automatikus áttűnés átfedésnél | Idővonal, Projekt-alapértelmezések |
| OpenShot » Optimize | Automatikus optimalizálás küszöbbel, párhuzamos feladatok, előnézeti felbontás | Proxy és előnézet |
| OpenShot » Cache | Gyorsítótár módja (memória / lemez), előtöltés, korlát | Memória, Média-gyorsítótár |
| OpenShot » Performance | Hardveres dekódolás módja, dekódoló- és kódoló-GPU, szálak, max. dekódolási felbontás | Gyorsítók, Dekódolás és kódolás |
| OpenShot » Autosave | Visszavonási előzmények hossza, helyreállítási másolatok száma | Automatikus mentés |
| OpenShot » Location | Import-, mentési és exportmappa (utolsó / rögzített) | Helyek |
| OpenShot » Scopes | Waveform mód és szín, IRE-vonalak, hisztogram, vektorszkóp | Monitorok és mérőeszközök |
| OpenShot » Advanced | Naplózás, lejátszási FPS kijelzése | Napló és diagnosztika |

**Nem vesszük át:** az OpenShot névtelen statisztikaküldését és a ComfyUI-címet (a VISION.md szerint az MI helyi, felhő csak külön döntéssel), az MLT/melt elérési utakat (az MLT csak tesztorákulum), és az FFmpeg-útvonalat, amíg az FFmpeg-háttér döntése (#62) nincs a main ágon.

## Kodekek a Beállításokban

A kodekes dokumentumokból (`docs/codec-parity.md`, ADR 0003 §5, `docs/kdenlive-parity.md` K11) egy új „Dekódolás és kódolás” oldal következik, plusz néhány mező más oldalakon. Ma a hardveres dekódolás két helyen is szerepel (`media.hardwareDecoding` és `playback.hardwareDecoding`); ezt egy mezőbe kell vonni.

**Dekódolás és kódolás oldal**

- **Ezen a gépen (csak olvasható tábla):** kodekenként dekódolás és kódolás, szoftveresen és hardveresen, a tényleges felmérés szerint. Például: H.264 10 bit / 4:2:2 csak hardverrel; HEVC- és AV1-kódolás szoftveresen nincs; MPEG-2-, MP3- és JPEG 2000-kódolás nincs.
- **Dekódolás:** hardveres dekódolás Auto / Ki (egy mező), kodekenkénti háttér ott, ahol több van (H.264, HEVC, VP9, AV1: saját szoftveres / VA-API / NVDEC / GStreamer), dekódoló-GPU, max. hardveres felbontás, vázlatdekódolás.
- **Kódolás:** gyártónkénti út az ADR 0003 §5 szerint (NVIDIA: NVENC; AMD és Intel: VA-API; Vulkan Video, ahol van; mindig saját szoftveres tartalék), kódoló-GPU, alapértelmezett GPU-s exportrenderelés (ma az exportban van: `gpuRendering`).
- **Szálak:** dekódolási, kódolási és proxykészítési szálak száma, párhuzamos renderelés.
- **Háttérmotorok:** GStreamer kiegészítő háttér be/ki (ha a build tartalmazza), állapottal és verzióval. Az FFmpeg-háttér csak a #62 elfogadása után.
- **Hiányzó gyártói meghajtó:** a tulajdonosi döntés szerint a felhasználó telepíti; a menü telepítési linket ad, illetve a root-aloldalon a VA-API csomagokat jóváhagyással telepíti.

**Más oldalakon**

- **Média:** Camera RAW / CinemaDNG alapértelmezések (fehéregyensúly, debayer minőség), mert a kódexparítás szerint ma nincs Raw-beállítás.
- **Proxy és előnézet:** proxykodek (ProRes, DNxHR, H.264) és idővonal-előnézet kodekje a saját kódolókból.
- **Átkódolás:** a nem támogatott források (DV, MPEG-4 Part 2, WMV, R3D stb.) átkódolási javaslata.
- **A kiadási előbeállítások** (K11, platformprofilok) az Export ablakban maradnak, nem a Beállításokban.
- **Ki****vesszük (jóváhagyva, 2026-10-26)****:** `media.proresHardwareEncoding` (csak macOS-en van ProRes-hardver, a CFA4 Linux-only).

## MI, közzététel és külső szolgáltatók

Két új csoport kezeli ezeket: a „Helyi MI” a gépen futó modelleket, a „Közzététel és szolgáltatók” mindazt, ami hálózatra megy. Az alapértelmezés mindig a helyi, hálózat nélküli működés (VISION.md 2/3).

**Helyi MI**

- **MI általános:** funkciók be/ki; futtatási eszköz munkatípusonként (Auto / CPU / adott GPU / NPU), a Gyorsítók oldallal összhangban; a kijelzőt hajtó GPU védelme; memória- és VRAM-korlát. Fix szabály, nem kapcsolható ki: az MI idővonal-módosítása csak előnézettel és emberi jóváhagyással fut.
- **Beszéd, felirat, fordítás:** a mai `mediaAnalysis` mezők, plusz fordítómodell (K6), felolvasás hangja (Kokoro), szinkron beállításai.
- **Kép, videó és hang:** maszkolás (K4), zaj- és zeneleválasztás (K26), jelenet- és csendfelismerés, színillesztés alapértelmezései.
- **Modellek:** listázza a telepített modelleket licenccel, forrással, hash-sel, mérettel és a futtató eszközzel. Nem kereskedelmi licencnél figyelmeztet (K28). Letöltés csak a felhasználó jóváhagyásával, előtte látszik a méret és a licenc (K18, K31).

**Közzététel és szolgáltatók**

- **Közzététel:** mely platformokra lehet feltölteni (a VISION.md 8. nyitott döntése), platformonkénti alapértelmezett kiadási előbeállítás (K11), alapértelmezett láthatóság (privát / nem listázott / nyilvános), export utáni lépések: borítókép, fejezetek, leírás (K15). Feltöltés mindig csak külön megerősítéssel.
- **Online források:** Hugging Face (modellek), Freesound és Pixabay (hang, kép, videó); letöltési mappa, licencszűrő (pl. csak CC0 / CC BY), forrás- és licencnapló minden letöltött elemhez (K18).
- **Fiókok és tokenek:** szolgáltatónként állapot (nincs / csatlakoztatva / lejárt), belépés, token megadása, visszavonás, utolsó használat. Az FA3-elv szerint a token értéke soha nem kerül a beállításfájlba, projektbe, naplóba vagy hibajelentésbe: a CFA4 titkosított titoktárában tárolódik (lásd lent), a felületen csak a vége látszik (pl. `hf_…x7Q2`).
- **Hálózat és adatvédelem:** offline mód egy kapcsolóval; proxy; szolgáltatónkénti hálózati engedély; mi hagyhatja el a gépet. Felhős MI-szolgáltató alapból ki, és csak a VISION.md 8. nyitott döntése után jelenik meg.

A tokenek és fiókok kezelése nem kér root-jogot, de ugyanúgy tervezet → megerősítés mintát követ, mint a root-aloldal.

**Jelszavak és tokenek tárolása (az FA3-megoldás alapján)**

A Final-Architecture-v3.0 részletes, már valódi gépen igazolt megoldást tartalmaz (`docs/security/FA3-SECRETS-FABRIC.md`, `src/fa3_secret_broker.py`, döntések: `FA3-DEC-SECRET-BROKER-ENCRYPTED-VAULT-2026-09-20`, `FA3-DEC-DESKTOP-SECRET-BACKEND-ADAPTER-2026-09-19`; licenc: Apache-2.0, a GPL-3-mal kompatibilis). Ezt vesszük át.

**Döntés (István, 2026-10-26):** az asztali titoktárak (GNOME Keyring, KWallet) könnyen visszafejthetők, ezért a CFA4 az FA3 Secret Broker mintáját használja, nem azokat.

| Rész | Működés | CFA4-ben |
|---|---|---|
| Titkosított tár | Külön LUKS2-képfájl általános, semmit el nem áruló névvel; feloldokulcs `systemd-creds`-szel, a táron kívül; TPM2/FIDO2 opcionális | A root-aloldal általános komponense („Titkosított titoktár”), jóváhagyással telepítve |
| Közvetítő (broker) | Saját, jogosultság nélküli rendszerfelhasználó, csak helyi Unix-socket, a hívót `SO_PEERCRED`-del azonosítja, titkonkénti szabály | A CFA4 csak hivatkozást (SecretRef) tárol, az értéket egyszerre egy titkot, fájlleírón kapja meg |
| Életciklus | systemd-target: megnyitás → csatolás → broker; kilépéskor fordítva, zárt állapotig ellenőrizve | Csak a szolgáltatót igénylő művelet idejére nyitva |
| Adminisztráció | Megadás, csere, visszavonás, metaadat-lista, zárt állapotú mentés és visszaállítás, kulcscsere | A „Fiókok és tokenek” oldal gombjai; mentés és kulcscsere a root-aloldalon |

Ha a titkosított tár nincs telepítve, a tokenhez kötött szolgáltatások nem működnek (Hugging Face-letöltés névtelenül továbbra is), és a menü a telepítéshez vezet. Kulcstár-tartalék nincs.

**Az FA3-szabályok, amelyeket változatlanul átveszünk:**

- A program csak hivatkozást (SecretRef) tárol a beállításokban; maga az érték soha nem kerül beállításfájlba, projektbe, környezeti változóba, naplóba vagy hibajelentésbe.
- Nincs egyszerű szöveges tartalék: ha a titoktár nem érhető el, az adott szolgáltató nem működik, és a menü kiírja, miért.
- Műveletek: megadás, csere (rotáció), visszavonás, csak metaadatos lista; tömeges export nincs.
- Az FA3-ban az azonos felhasználóként futó programok elleni védelem nem készült el; a CFA4-ben ezt az alábbi pontok szerint be kell fejezni.
- Ugyanazon felhasználó más programjai ellen nem ígérünk védelmet (az FA3 is így fogalmaz).

**A CFA4-ben befejezendő: védelem az azonos felhasználóként futó programok ellen** (az FA3-ban vázlat maradt)

1. **A token nem hagyja el a brokert.** A CFA4 nem kéri ki a tokent, hanem a brokertől kéri a műveletet (pl. „töltsd le ezt a modellt a Hugging Face-ről”, „töltsd fel ezt a fájlt”). A broker maga teszi hozzá a hitelesítést a hálózati kéréshez, és csak az előre engedélyezett címekre. Így a CFA4 folyamatának memóriájából sem olvasható ki.
2. **A hívó ellenőrzése:** `SO_PEERCRED` mellett a hívó futtatható fájljának útvonala és hash-e is egyezzen a telepített CFA4-ével.
3. **Megerősítés érzékeny műveletnél:** feltöltés, token cseréje vagy törlése előtt a broker saját megerősítést kér (jelszó vagy polkit), nem a CFA4 ablaka.
4. **Időkorlát:** a tár csak a művelet idejére van nyitva, tétlenség után zár.
5. **Napló:** minden kiadott művelet metaadata naplózva (ki, mit, mikor), érték nélkül; a „Fiókok és tokenek” oldalon látható.

A broker kódja az FA3 `fa3_secret_broker.py` módszerét követi; a CFA4-ben Rustban írjuk újra, vagy külön folyamatként vesszük át az eredeti Apache-2.0 jegyzékkel és a licencnyilvántartás bejegyzésével. Ugyanezt a tárat használhatja később a hálózati render (K14) is.

## Rendszerszintű beállítások (root)

Ez az egyetlen oldal, amely a CFA4-en kívül módosítja a rendszert; minden változtatás tervezetként indul, és csak jóváhagyás után hajtja végre egy külön segédprogram. A CFA4 maga soha nem fut rootként.

**Egy változtatás menete**

1. **Ellenőrzés (root nélkül):** a komponens kompatibilis-e ezzel a géppel (PCI-azonosító, kernel, meghajtó, Secure Boot, initramfs-eszköz, szabad hely).
2. **Tervezet:** a segédprogram próbafutása kilistázza, mely fájlok, csomagok, csoporttagságok vagy indítási beállítások változnak, és mi lesz a visszavonás módja.
3. **Jóváhagyás:** a felhasználó látja a tervezetet; kísérleti komponensnél külön figyelmeztetés és megerősítés is kell. Ezután a rendszer saját jelszókérő ablaka jelenik meg (`pkexec`).
4. **Végrehajtás és napló:** a `cfa4-system-helper` csak a zárt igélistáját fogadja el, minden lépést naplóz, és hiba esetén visszagördül.
5. **Utóellenőrzés:** a komponens állapota „telepítve”, „újraindításra vár” vagy „aktív” lesz; a Diagnosztika fülön látszik az eredmény.

Minden komponensnél elérhető a „Kikapcsolás” és az „Eltávolítás”, ugyanezen az úton.

**Komponensek**

| Komponens | Gyártó | Mit módosít | Státusz a menüben |
|---|---|---|---|
| GPU-hozzáférés (`render` / `video` csoport) | bármely | A felhasználó csoporttagsága | telepíthető, első körben |
| VA-API meghajtók | Intel, AMD | Ubuntu-csomagok (`intel-media-va-driver`, `mesa-va-drivers`) | telepíthető, első körben |
| NPU-meghajtó ellenőrzése | Intel, AMD | Kernelmodul és firmware megléte | első körben csak ellenőrzés és útmutató |
| GPU Unlock (tensor-sebesség) | NVIDIA GeForce | Kernelmodul, initramfs, külön GRUB-bejegyzés | adatlap: kártyánkénti státusz, figyelmeztetés, upstream útmutató; telepítő később |
| vgpu-unlock-blackwell | NVIDIA RTX 5090 | – | figyelőlista, „hardveresen blokkolt” |

A GPU Unlock kártyánkénti státuszai: mért (upstream mérés), jelentett, kísérleti, teszteletlen, nem támogatott. Az RTX A1000 és minden ismeretlen azonosító „nem támogatott”. A telepítő akkor készül el, ha a helyi MI már ténylegesen használ FP16/BF16 tensorszámítást, és van támogatott tesztkártya. A részletes telepítési lépések az előző tervben vannak: [CFA4 Hardverbeállítások – root menü terve](https://claude.ai/artifact/GcFzoqUGJK7KkmFq8UQLYd).

## Megvalósítás a CFA4 kódjában

A menü a motor közös beállítás-sémájára épül, így az egui-ablak most, a Qt6-felület később ugyanazt használja.

- **Séma** (`crates/engine/src/settings.rs`): a `Category` két új mezőt kap: `group` (csoportfejléc) és `parent` (aloldal). A meglévő oldalak azonosítói és kulcsai nem változnak, így a mentett beállítások és az `app.settings.<oldal>` parancsok tovább működnek.
- **Ablak** (`crates/ui-egui/src/panels/settings.rs`): csoportfejlécek és behúzott aloldalak a bal listában; az eszközoldalak új `Row::Custom` részekként (`deviceList`, `acceleratorDetails`, `systemComponents`).
- **Felderítés** (`crates/accel`): a meglévő `probe` adja a GPU/NPU-listát; új `compat` modul a komponensek kártyánkénti státuszához. A többi eszköztípus felderítése saját modult kap, ugyanilyen hamis-sysfs tesztekkel.
- **Parancsok** (`crates/engine`): `hardware.refresh`, `hardware.test.<eszköz>`, `hardware.system.<komponens>.check | plan | apply | disable | uninstall`. A `plan` adja a tervezetet; az `apply` csak jóváhagyott tervezetre fut.
- **Segédprogram:** `cfa4-system-helper`, `pkexec`-kel indítva, `--dry-run` és tesztekhez `--root <könyvtár>` móddal.
- **Fordítás:** az új feliratok angol forrásszöveggel és magyar fordítással kerülnek a meglévő i18n-rendszerbe.
- **Döntési dokumentum:** új ADR a rendszerszintű komponensekről (jóváhagyási modell, GPLv2-es és csak NVIDIA-s elemek kezelése).

## Ütemezés

Öt szakaszban haladnék, mindegyik külön PR; az 1–2. szakaszban még nincs root-művelet.

| Szakasz | Tartalom | Root | Tesztelhető a te gépeden |
|---|---|---|---|
| 1. Keret | Csoportok és aloldalak a Beállításokban, a meglévő oldalak átrendezése, ADR | nem | igen |
| 2. Gyorsítók | Áttekintés + Gyorsítók oldal a `cfa4-accel` alapján, eszközoldal-minta, Dekódolás és kódolás oldal (a dupla hardveres dekódolás-mező összevonásával) | nem | igen (RTX A1000, CPU) |
| 3. Root-keret | Segédprogram, tervezet → jóváhagyás → végrehajtás, első komponensek: csoporttagság, VA-API, NPU-ellenőrzés | igen | igen |
| 4. GPU Unlock adatlap | Kártyánkénti státusz, figyelőlista, vgpu-unlock-blackwell | nem | csak a „nem támogatott” ág |
| 5. További oldalak | Kdenlive/OpenShot-oldalak (Helyek, Projekt-alapértelmezések, Proxy és előnézet, Átkódolás, Külső programok, Egyszerű mód), kijelzők, tárolók, felvétel, rajztáblák, vezérlőeszközök, Modellek, Napló, Frissítések | részben | eszközfüggő |

A GPU Unlock telepítője az 5. szakasz után, a fenti feltételek teljesülésekor kerül sorra.

## Kiegészítendő és pontosítandó kérdések

- Megfelel-e a hét csoport és a sorrend, vagy legyen más a felső szint (például a CFA3 második képterve szerinti „Erőforrások”, „Biztonság és jóváhagyás” külön)?
- Hiányzik-e eszköztípus a Hardver és eszközök csoportból (például nyomtató és szkenner az FA3-ból)? Az OpenShot-féle „Egyszerű mód” az Általános oldalon jó helyen van-e, vagy legyen külön választás az első indításkor?
- Kell-e az FA3 Accelerator Guard (GPU/NPU-ütközés kezelése) a CFA4-ben, és ha igen, a Gyorsítók oldalon vagy külön?
- Mely platformokra legyen közzététel (YouTube, Vimeo, PeerTube, Facebook, TikTok, egyéb), és engedélyezzük-e kifejezett hozzájárulással a felhős MI-t? Ez a VISION.md 8. két nyitott döntése. Melyik „később” oldal fontos neked előbb, hogy az 5. szakasz sorrendjét ahhoz igazítsam?
- A menü neve az ablakban maradjon „Preferences” (a jelenlegi Premiere-hasonlóság miatt), vagy legyen „Beállítások” / „Settings”?
