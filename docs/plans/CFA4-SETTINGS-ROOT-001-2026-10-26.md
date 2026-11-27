# CFA4-SETTINGS-ROOT-001 – CFA4 Hardverbeállítások – root menü terve

> **Átvétel a repóba (2026-11-27, `docs/coordination.md` §2.11):** ez a terv 2026-10-26-án Claude-dokumentumként
> készült (<https://claude.ai/artifact/GcFzoqUGJK7KkmFq8UQLYd>). A tartalma itt változatlan, csak Markdownra
> alakítva. Ugyanazon a napon a közös menüterv ([`CFA4-SETTINGS-MENU-001.md`](CFA4-SETTINGS-MENU-001.md))
> váltotta fel; a root-rész mostani terve: [`CFA4-SETTINGS-HW-003.md`](CFA4-SETTINGS-HW-003.md). A lenti
> „ADR-0004” szám azóta foglalt (`docs/adr/ADR-0004-compute-routing.md`), és a „Mit találtam a kódban” rész
> a 2026-10-26-i állapotot írja le.

2026-10-26 · ubuntuokos

A Beállítások ablak új „Hardver” kategóriát kap, alatta behúzott „Rendszerszintű beállítások (root)” aloldallal. A GPU Unlock itt opcionális, utólag telepíthető, minden root-lépésnél megerősítést kérő komponens lesz; a vgpu-unlock-blackwell csak figyelőlistás hivatkozás. A kártyafelismerés a már meglévő `cfa4-accel` PCI-leltárra épül.

## Mit találtam a kódban

A Beállítások oldalai egy közös sémából épülnek, így az új oldal kis módosítással beilleszthető, és a Qt6-felület később ugyanazt a sémát használhatja.

- **Séma:** `crates/engine/src/settings.rs`, `CATEGORIES` lista. Minden kategória `Category { id, title, rows }`; a sorok mezők, csoportok, megjegyzések, parancsgombok (`Row::Button`) vagy saját rajzolású részek (`Row::Custom`). A kategórialista ma egyszintű, almenü nincs.
- **egui-ablak:** `crates/ui-egui/src/panels/settings.rs`. Bal oldalon a kategórialista, jobb oldalon az oldal, alul Súgó / Visszaállítás / Mégse / OK. Megnyitás: `app.settings.<kategória>` parancs; automatizálási azonosítók: `settings.category.<id>`, `settings.<kulcs>`.
- **Hardverhez kötött beállítások ma szétszórva:** `audioHardware` (hangeszközök), `playback.hardwareDecoding` (Auto/Ki), `memory`.
- **GPU-felismerés már van:** a `cfa4-accel` crate (`probe.rs`) a `/sys/class/drm/renderD*` és `/sys/class/accel/*` alól kiolvassa minden GPU és NPU PCI vendor- és eszközazonosítóját és a kernelmeghajtót. Ma még semmi nem használja a felületen; erre épülhet a kártyánkénti ellenőrzés.
- **Qt6-shell:** `python/cfa4_devtools/qt_app.py` még csak prototípus, Beállítások ablaka nincs.
- **Root-művelet ma sehol nincs** a programban (nincs pkexec, polkit, initramfs-kezelés); ez teljesen új rész lesz.

## Menüszerkezet

A kategórialista kap egy szülő–gyermek kapcsolatot (`Category.parent`), így a root rész külön, behúzott aloldal lesz, saját figyelmeztető fejléccel. A többi kategória változatlan marad.

- **Hardver** (`hardware`, felhasználói szint, root nélkül)
  - Felismert eszközök: GPU-k és NPU-k típusa, PCI-azonosítója (`10de:xxxx`), meghajtója, a `cfa4-accel` leltárból.
  - Gyorsítás: a `cfa4-accel` beállításai (gyártóhoz kötött hátterek engedélyezése, munkatípusonkénti preferencia). A `playback.hardwareDecoding` a helyén marad, innen csak hivatkozás mutat rá.
- **Rendszerszintű beállítások (root)** (`hardwareSystem`, a Hardver alatt behúzva)
  - Fejléc: „Az itt lévő műveletek rendszergazdai jelszót kérnek, és a CFA4-en kívül, a rendszerindítást is módosítják.”
  - Komponenslista (`Row::Custom("systemComponents")`): komponensenként név, leírás, a saját kártya státusza, forrásrepó és rögzített verzió, ellenőrzés dátuma, állapot (nincs telepítve / telepítve / bekapcsolva, újraindításra vár / aktív), és a megengedett gombok.
  - Figyelőlista: nem telepíthető kutatási projektek csak hivatkozással (vgpu-unlock-blackwell).

Minden gomb parancs lesz (`hardware.system.<komponens>.check | install | enable | disable | uninstall`), így a CLI, a vezérlőcsatorna és az MCP is eléri, a CFA4 szabálya szerint. A root-műveleteket a vezérlőcsatornáról is csak a felhasználó jelszó-megerősítésével lehet végrehajtani.

## Komponensek és kompatibilitási státuszok

A státuszt mindig a pontos PCI-azonosító dönti el; ami nincs a listán, az „nem támogatott”. Így az RTX A1000 (és minden RTX A- és adatközponti kártya) nem telepíthető.

| Státusz | Jelentése | Mit enged a menü |
|---|---|---|
| Mért | Az upstream azonos kártyán végzett stock/unlocked mérést közöl | Telepítés felajánlva, normál megerősítéssel |
| Jelentett | Csak felhasználói jelentés van | Telepíthető, figyelmeztetés + külön megerősítés |
| Kísérleti | Az upstream csak kísérleti kapcsolóval, PCI-címhez kötve engedi | Telepíthető, erős figyelmeztetés + a kártya nevének begépelésével megerősítve |
| Teszteletlen | A modul elfogadja az azonosítót, de nincs bizonyíték | Mint a kísérleti |
| Nem támogatott | Kizárt vagy ismeretlen azonosító (pl. RTX A1000) | Csak indoklás, gomb nincs |
| Hardveresen blokkolt | Figyelőlista, ma nem működik | Csak hivatkozás és leírás |

**Komponensek az első körben**

- **GPU Unlock** (Zagitta/gpu-unlock Ampere-re, JustAResearcher/gpu-unlock-ada Adára): FP16/BF16/TF32 tensor-sebesség GeForce kártyákon. Telepíthető komponens.
- **vgpu-unlock-blackwell** (bird): vGPU RTX 5090-en; „hardveresen blokkolt”, csak figyelőlista.

**Adatforrás:** a kártyatábla a `cfa4-accel` új `compat` moduljába kerül, egy rögzített upstream committól származó pillanatképként: a kernelmodul PCI-listája és a README bizonyítékszintjei, származási commit és ellenőrzési dátum soronként. A weboldal listája nem forrás, mert lemarad a repó mögött. Frissítés: egy fejlesztői szkript (`python/cfa4_devtools/`) újragenerálja a táblát; a program futás közben nem tölt le listát. A menüben a „mért” felirat „upstream mérés”-ként jelenik meg, mert nem CFA4-es mérés.

## Telepítési folyamat

Három külön megerősített root-lépés van: telepítés, bekapcsolás, és az alapértelmezett indítás átállítása a sikeres próba után. Mindegyik előtt a párbeszédablak pontosan felsorolja, mely fájlok és beállítások változnak.

1. **Ellenőrzés (root nélkül):** PCI-azonosító → státusz; NVIDIA-meghajtó verziója; kernelverzió és fejlécfájlok; Secure Boot állapota; initramfs-eszköz (Ubuntu 26.04-en dracut, régebbin initramfs-tools); szabad hely a /boot-on. Bármelyik hiba esetén a telepítés gomb inaktív, indoklással.
2. **Telepítés (1. megerősítés, root):** a rögzített upstream commit letöltése ellenőrzött hash-sel, fordítás DKMS-sel a felhasználó gépén. A modul ekkor még nem töltődik be, a rendszerindítás változatlan. Bekapcsolt Secure Bootnál a MOK-kulcs regisztrálását külön lépésként mutatja.
3. **Bekapcsolás (2. megerősítés, root):** initramfs-konfiguráció és egy **külön** GRUB-bejegyzés „CFA4 GPU Unlock” néven; az eredeti bejegyzés marad az alapértelmezett. A következő indítás egyszer erre a bejegyzésre megy (`grub-reboot`), és hidegindítást kérünk, ahogy az upstream javasolja.
4. **Ellenőrzés újraindítás után (root nélkül):** a CFA4 induláskor megnézi, betöltődött-e a modul, és látja-e az NVIDIA-meghajtó a GPU-t. Siker esetén „aktív”, hiba esetén a rendszer a következő indításkor magától az eredeti bejegyzésre tér vissza.
5. **Alapértelmezetté tétel (3. megerősítés, root, opcionális):** csak sikeres próba után ajánljuk fel. Az eredeti bejegyzés ekkor is megmarad.
6. **Kikapcsolás / eltávolítás (megerősítéssel, root):** a 3–5. lépés visszavonása, majd a DKMS-modul törlése. Minden módosított fájlról a telepítéskor biztonsági másolat készül, az eltávolítás ezeket állítja vissza.

A pontos kernelparamétert, modulnevet és initramfs-hookot a rögzített upstream commit README-jéből és csomagolásából veszem át a megvalósításkor; ezeket még nem ellenőriztem a forrásban.

## Root-jogosultság és biztonság

A CFA4 maga soha nem fut rootként. A rendszerszintű lépéseket egy külön, kicsi segédprogram végzi, amelyet `pkexec` indít, így a rendszer saját jelszókérő ablaka jelenik meg minden lépésnél.

- **Segédprogram:** `cfa4-system-helper`, zárt igélistával (`check`, `install`, `enable`, `make-default`, `disable`, `uninstall`, `status`), komponens-azonosítóval. Tetszőleges parancsot nem fogad el.
- **Próbafutás:** minden igének van `--dry-run` módja, amely kilistázza a tervezett változtatásokat; ezt mutatja a megerősítő ablak.
- **Napló:** minden módosítás a rendszernaplóba és egy CFA4-es állapotfájlba kerül, ebből dolgozik a kikapcsolás és az eltávolítás.
- **Letöltés:** csak a rögzített commit, ellenőrzött hash-sel; eltérésnél a telepítés leáll.
- **Hibák:** a CFA4 „soha ne omoljon össze” szabálya szerint minden hiba (megtagadott jelszó, sikertelen fordítás, hálózati hiba) üzenetként jelenik meg az oldalon, és a félbemaradt lépés visszagördül.
- **Tesztek:** a segédprogram egy hamis gyökérkönyvtáron (`--root <dir>`) fut a tesztekben, ugyanúgy, ahogy a `cfa4-accel` probe-tesztjei hamis sysfs-t használnak.

## Licenc és ütközések a VISION.md-vel

Három pont érinti a repo szabályait; a VISION.md szerint ezeket jeleznem kell, nem csendben feloldani.

- **Licenc:** a GPU Unlock GPLv2-es, a CFA4 GPL-3.0-or-later. Az `AGENTS.md` tiltja a GPL-2.0-only kód átvételét, de megengedi külső programként hívni. Ezért a CFA4 sem a modult, sem a firmware-t nem tartalmazza és nem terjeszti: a felhasználó gépe tölti le és fordítja az upstreamből. A firmware/payload eredete így nem a CFA4 terjesztési kérdése, de a menü kiírja a forrást és a licencet.
- **Gyártófüggetlenség (VISION 2/4. elv):** a GPU Unlock csak NVIDIA-n működik. Javaslatom: a root-aloldal általános keret (bármely gyártó rendszerkomponensét fogadhatja), a GPU Unlock egyetlen opcionális elem rajta, és semmilyen CFA4-funkció nem függ tőle. Ezt egy új ADR-ben (ADR-0004) rögzíteném, ahogy a GStreamer-kivételt is.
- **Bizonyíték (VISION 2/5. elv):** a te gépeden (RTX A1000) csak a „nem támogatott” ág igazolható. A telepítési ág valódi gépes ellenőrzéséhez Ampere vagy Ada GeForce kártya kell; addig a funkció „kísérleti” jelöléssel kerül be.

## Megvalósítás PR-onként

Négy kisebb PR-ban haladnék, hogy minden lépés külön átnézhető legyen. Az első három semmilyen root-műveletet nem tartalmaz.

1. **Terv a main ágon:** ADR-0004 (rendszerkomponensek, licenc- és gyártó-kivétel), `docs/hardware-parity.md` kiegészítése.
2. **Kompatibilitási adatok:** `cfa4-accel::compat` modul, a GPU Unlock és a vgpu-unlock-blackwell pillanatképe, tesztek (RTX A1000 → nem támogatott; ismeretlen azonosító → nem támogatott; RTX 4090 `10de:2684` → mért; kísérleti Ada-kártyák → kísérleti), valamint a frissítő szkript.
3. **Menü, csak megjelenítés:** `Category.parent`, a Hardver és a Rendszerszintű (root) oldal az egui-ablakban, felismert eszközök, státuszok, figyelőlista, magyar fordítás. Telepítés gomb még nincs.
4. **Root-műveletek:** `cfa4-system-helper` + `pkexec`, a parancsok, a megerősítő ablakok, az újraindítás utáni ellenőrzés, hamis gyökérkönyvtáras tesztek.

A Qt6-felület ugyanazt a sémát és parancsokat használja majd, amikor elkészül a Beállítások ablaka. A teljes CI-t (`cfa4-full-validation` címke) PR-onként lefuttatom.

## Döntések, amiket kérek

- **Jóváhagyod a tervet?** Ha igen, az 1–3. PR-ral kezdek; a 4. (root-műveletek) előtt még egyszer szólok.
- **Almenü formája:** behúzott, külön „Rendszerszintű (root)” oldal a Hardver alatt (ajánlott), vagy csak egy csoport a Hardver oldalon?
- **Gyártófüggetlenség:** rendben van-e, hogy a GPU Unlock NVIDIA-only opcionális elemként, ADR-kivétellel kerül be?
