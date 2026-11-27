# CFA4-SETTINGS-HW-003 – Rendszerszintű beállítások (root), Áttekintés oldal, a kísérleti héj Beállítások ablaka

**Állapot:** JAVASLAT 0.1 – a tulajdonos jóváhagyására vár. Fejlesztés csak jóváhagyás után indul
([`docs/coordination.md`](../coordination.md) §2.10).
**Dátum:** 2026-11-27 · **Tulajdonos:** ubuntuokos
**Projekt:** „CFA4 hardver beállítása”.
**Előzmény:** [`CFA4-SETTINGS-HW-002.md`](CFA4-SETTINGS-HW-002.md) 6. pontja (a tulajdonos 2026-11-27-én kérte
ennek a külön tervnek az elkészítését); menüterv: [`CFA4-SETTINGS-MENU-001.md`](CFA4-SETTINGS-MENU-001.md)
„Rendszerszintű beállítások (root)” és „Ütemezés” 3. szakasz; a korábbi root-terv:
[`CFA4-SETTINGS-ROOT-001-2026-10-26.md`](CFA4-SETTINGS-ROOT-001-2026-10-26.md).
**Feltétel:** a HW-002 PR-ja előbb kerüljön be, mert ez a terv a `cfa4-qt` szerkesztő Beállítások
ablakának csoportos listájára épül.

## 1. Kiinduló állapot

- Root-műveletet a CFA4 ma egy helyen kínál: a `cfa4-db-setup` (`crates/store/src/setup.rs`), amelyet a
  `.deb` `postinst`, vagy első indításkor `pkexec` futtat. Általános, jóváhagyáshoz kötött rendszer-segéd
  nincs.
- A gyorsító-leltár (`cfa4_route::probe`) látja a GPU-kat és NPU-kat (gyártó, meghajtó, PCI-azonosító),
  azt viszont nem, hogy a felhasználó hozzáfér-e a `/dev/dri/renderD*` csomóponthoz, és hogy van-e VA-API
  meghajtó. Ezt ma a Dekódolás és kódolás oldal csak közvetve, a hardveres dekóder hiányából mutatja.
- Az ADR-0004 száma foglalt (`ADR-0004-compute-routing.md`); a régi root-tervben szereplő ADR ezért új
  számot kap: **ADR-0017**.
- A Secret Broker (titoktár) a menütervben a root-aloldal komponense, de a security-rész külön projekt:
  **ebben a tervben nincs benne**.

## 2. Hatókör

**Benne van:**

- R0: ADR-0017 a rendszerszintű komponensekről.
- R1: `cfa4-system-helper` és az első három, gyártófüggetlen komponens.
- R2: a motor oldala (serve-metódusok).
- R3: a „Rendszerszintű beállítások (root)” oldal a `cfa4-qt`-ban.
- R4: az Áttekintés oldal és a közös eszközoldal-minta.
- R5: a kísérleti héj Beállítások ablakának megszüntetése (D5 szerint).

**Nincs benne:** GPU Unlock (sem adatlap, sem telepítő: D6), vgpu-unlock-blackwell, titoktár és
tokenek (security-projekt), a többi eszközoldal (kijelzők, tárolók, kamerák, rajztáblák), az egui.

## 3. R0 – ADR-0017: rendszerszintű komponensek

Az ADR ezeket rögzíti:

1. A CFA4 (GUI, `cfa4-cli serve`, vezérlőcsatorna, MCP) **soha nem fut rootként**, és nem futtat tetszőleges
   root-parancsot.
2. Minden rendszerszintű változtatás egy úton megy: **ellenőrzés (root nélkül) → tervezet (root nélkül)
   → jóváhagyás a CFA4 ablakában → `pkexec` jelszókérés → végrehajtás a segédprogramban → utóellenőrzés**.
3. A segédprogram zárt igélistát és zárt komponenslistát fogad el. Paramétert csak a komponens saját,
   ellenőrzött mezőjeként kap (például a felhasználónevet a `pkexec` által adott `PKEXEC_UID`-ból
   számolja, nem a hívótól veszi át).
4. Jóváhagyás nélkül semmi nem fut: ügynök (vezérlőcsatorna, MCP) is csak tervezetet kérhet. A végrehajtást
   a `pkexec` jelszókérése mindig a felhasználóhoz köti, ügynök ezt nem kerülheti meg.
5. Minden komponensnek van visszavonása (kikapcsolás, eltávolítás), és a segédprogram csak azt vonja
   vissza, amit maga csinált (állapotfájl).
6. Gyártófüggetlenség: a keret bármely gyártó komponensét fogadja. Gyártóhoz kötött komponens (például a
   későbbi GPU Unlock) csak opcionális lehet, és semmilyen CFA4-funkció nem függhet tőle.
7. Licenc: GPL-2.0-only komponenst (például a GPU Unlock) a CFA4 nem tartalmaz és nem terjeszt, legfeljebb
   a felhasználó gépe tölti le az upstreamből, külön döntéssel (D6).

## 4. R1 – `cfa4-system-helper`

**Hely:** új Rust bináris, `apps/cfa4-system-helper` (Rust first). Kevés függőség: `serde_json`, és a
leltárhoz a `cfa4-route` (csak sysfs-olvasás). A GUI-hoz, a motorhoz és a hálózathoz nem kötődik.
Telepítési helye: `/usr/libexec/cfa4/cfa4-system-helper`.

**Igék:** `status`, `check`, `plan`, `apply <tervezetfájl>`, `disable`, `uninstall`; mindegyik
`--component <id>` paraméterrel, JSON kimenettel. Tesztekhez `--root <könyvtár>` (hamis gyökér) és
`--dry-run`.

**Jogosultság:** a polkit-szabály (`packaging/linux/…/org.cfa4.system-helper.policy`) csak ezt az egy
programot engedi `pkexec`-kel, `auth_admin` hitelesítéssel (D4). Root nélkül csak a `status`, `check` és
`plan` fut. Az `apply` a tervezet kivonatát (SHA-256) is megkapja, rootként újra elkészíti a tervezetet,
és csak egyezés esetén hajtja végre, így a jóváhagyás és a végrehajtás között módosult tervezet nem fut le.

**Napló és állapot:** minden lépés a rendszernaplóba kerül (`journald`, `cfa4-system-helper` azonosítóval)
és a `/var/lib/cfa4/system-helper/state.json` állapotfájlba (mit, mikor, milyen visszavonással). Hibánál
a félbemaradt lépés visszagördül; a segédprogram a hibát JSON-ban adja vissza, nem omlik össze
(`AGENTS.md` §0).

**Az első komponensek (mind gyártófüggetlen):**

| Komponens | `check` (root nélkül) | `apply` | Visszavonás |
|---|---|---|---|
| `gpu-access` – GPU-hozzáférés | olvasható-e a felhasználónak minden `/dev/dri/renderD*` és `/dev/accel/accel*`; melyik csoport kell hozzá (`render`, `video`); tagja-e a felhasználó | a felhasználó felvétele a hiányzó csoportba (`gpasswd -a`); utána **ki- és bejelentkezés kell**, ezt a menü kiírja (D3) | `gpasswd -d`, csak ha a segéd vette fel |
| `vaapi-drivers` – VA-API meghajtók | a leltár gyártói szerint melyik Ubuntu-csomag kell (Intel: `intel-media-va-driver`, AMD: `mesa-va-drivers`); telepítve van-e; működik-e (`/usr/lib/x86_64-linux-gnu/dri/*_drv_video.so`) | a hiányzó csomag telepítése `apt-get install`-lal, a tervezetben a pontos csomaglistával (D2) | `apt-get remove`, csak a segéd által telepített csomagra |
| `npu-check` – NPU-meghajtó | NPU esetén betöltődött-e a kernelmodul (`intel_vpu`, `amdxdna`) és megvan-e a firmware a `/lib/firmware` alatt | **nincs**: csak ellenőrzés és útmutató (melyik csomag vagy kernel kell) | – |

NVIDIA kártyán a VA-API komponens „nem kell” állapotú (ott az NVDEC a hardveres út, ADR 0003 §5); a
gyártói meghajtót a CFA4 nem telepíti (tulajdonosi döntés, `docs/settings-menu.md`).

## 5. R2 – a motor oldala

A HW-002 D2 döntésével azonosan `cfa4-cli serve`-metódusok (`apps/cfa4-cli/src/serve.rs`):

- `system.components` – minden komponens `status` + `check` eredménye (root nélkül futtatja a segédet).
- `system.plan` – egy komponens tervezete és kivonata.
- `system.apply` – `pkexec /usr/libexec/cfa4/cfa4-system-helper apply …` a jóváhagyott tervezettel; a
  válasz a segéd JSON-ja. Ha a `pkexec` vagy a polkit-szabály hiányzik (fejlesztői build), a válasz ezt
  mondja, és semmi nem fut.
- `system.disable`, `system.uninstall` – ugyanígy.

Mindegyik aszinkron fut a serve-ben (külön szálon), hogy a jelszóablak alatt a szerkesztő ne álljon meg.

## 6. R3 – „Rendszerszintű beállítások (root)” oldal

- Helye: a *Hardver és eszközök* csoport utolsó oldala (`systemComponents`, Qt-only oldal, mint a Gyorsítók).
- Fejléc: „Az itt lévő műveletek rendszergazdai jelszót kérnek, és a CFA4-en kívül módosítják a rendszert.”
- Komponensenként: név, leírás, állapot (*nem kell* / *rendben* / *hiányzik* / *kijelentkezésre vár* /
  *nem vizsgált*), gombok: **Ellenőrzés**, **Tervezet…** (megmutatja a pontos változtatásokat és a
  visszavonás módját), **Alkalmazás…** (csak a megnézett tervezetre, utána `pkexec`), **Visszavonás…**.
- Napló fül: a segéd állapotfájljának és válaszainak időbélyeges listája.
- Automatizálási azonosítók: `settings.system.components`, `settings.system.<id>.check|plan|apply|undo`,
  `settings.system.log`.
- A Gyorsítók és a Dekódolás és kódolás oldal hiányzó hozzáférésnél vagy meghajtónál ide mutató linket kap.

## 7. R4 – Áttekintés oldal és eszközoldal-minta

- **Áttekintés** a *Hardver és eszközök* csoport első oldala: egy listában a gyorsítók, a hangeszközök, a
  vezérlőfelületek, a memória és a rendszerkomponensek állapota, figyelmeztetésekkel (például „a GPU nem
  érhető el: nincs `render` csoporttagság”), mindegyik a saját oldalára visz. Csak olvas, adatot a már
  meglévő hívásokból kap (`hardware.inventory`, `codecs.capabilities`, `system.components`,
  `surface.devices`, a hangeszköz-lista).
- **Eszközoldal-minta** (a menüterv „Közös eszközoldal-minta” része): bal oldalon eszközlista állapottal,
  jobb oldalon fülek (Adatok, Képességek, Meghajtók, Diagnosztika). Újrahasznosítható Qt-elem
  (`python/cfa4_qt/panels/device_page.py`); elsőként a Gyorsítók oldal kapja meg, a tábláit megtartva a
  Képességek fülön. A későbbi eszközoldalak (kijelzők, tárolók, kamerák, rajztáblák) erre épülnek, külön
  tervvel.

## 8. R5 – a kísérleti héj Beállítások ablaka

A HW-002 után a Beállítások két helyen lesz, eltérő tartalommal: a szerkesztőben és a kísérleti héjban
(`python/cfa4_devtools/qt_app.py`, `qt_settings.py`). Javaslat (D5): a HW-002 beolvasztása után a kísérleti
héj Beállítások ablakát (`qt_settings.py` és az Edit ▸ Settings… menüpont) törlöm. A szerkesztő saját adatmodellt kapott
(`python/cfa4_qt/hardware_report.py`), így a `settings_backend.py` megfelelő része is törölhető, ha más
nem használja. Előtte megnézem, hivatkozik-e
rá csomagolás, CI vagy dokumentáció, és a talált hivatkozásokat ugyanabban a PR-ban javítom. Törlés csak a
tulajdonos kifejezett jóváhagyásával (D5).

## 9. Sorrend és PR-ok

| PR | Tartalom | Root-művelet |
|---|---|---|
| A | R0 + R1 (ADR-0017, segédprogram, polkit-szabály, hamis gyökeres tesztek) | igen, de csak tesztben, hamis gyökéren |
| B | R2 + R3 (serve-metódusok, root-oldal) | igen, `pkexec`-en át |
| C | R4 (Áttekintés, eszközoldal-minta) | nem |
| D | R5 (kísérleti héj Beállítások ablaka), ha D5 = igen | nem |

Az A és a B PR kockázatos a §2.9 szerint (biztonsági jellegű, csomagolás), ezért CodeRabbit-ellenőrzést kap.

## 10. Bizonyíték és mérés

- CI: a segéd minden igéje hamis gyökéren (`--root`) fut, valódi `gpasswd`/`apt-get` nélkül (a parancsokat
  egy tesztben cserélhető futtató kapja). A tervezet és a kivonat-ellenőrzés egységtesztet kap (módosított
  tervezet → elutasítva).
- Valódi gépen (referenciagép, [`docs/development/reference-machine.md`](../development/reference-machine.md)):
  a `check` futása root nélkül (NVIDIA-n a VA-API „nem kell”, a `gpu-access` állapota), és egy valódi
  `apply`/visszavonás a `gpu-access` komponensre, **csak a tulajdonos külön, ott helyben adott
  megerősítésével**. Ha ez elmarad, sort kap a
  [`docs/development/deferred-physical-tests.md`](../development/deferred-physical-tests.md) listában.
- AMD/Intel GPU-s és NPU-s ág: a referenciagépen nincs ilyen hardver, ezért ezek csak hamis sysfs-szel
  tesztelhetők; a valódi mérés a deferred listába kerül.

## 11. Döntések a tulajdonosnak

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Jóváhagyod az ADR-0017 alapelveit (3. pont) és a segédprogram zárt ige- és komponenslistáját? | Igen. |
| D2 | A VA-API csomagot a segéd telepítse `apt-get`-tel (jóváhagyás + jelszó után), vagy a menü csak a parancsot mutassa másolásra, ahogy a Media Backends oldal a korlátozott kodekeknél? | A segéd telepítse: ez a root-oldal célja, és a visszavonás is így követhető. |
| D3 | A csoporttagság után elég, ha a menü kiírja, hogy ki- és bejelentkezés kell, vagy ajánlja fel a kijelentkezést is? | Csak kiírja; kijelentkeztetni nem kell. |
| D4 | A jelszót minden műveletnél kérje (`auth_admin`), vagy a polkit pár percig megjegyezheti (`auth_admin_keep`)? | Mindig kérje. |
| D5 | Törölhető-e a kísérleti héj Beállítások ablaka a HW-002 beolvasztása után? | Igen (8. pont). |
| D6 | A GPU Unlock (adatlap és telepítő) maradjon-e ki ebből a tervből, és külön terv jöjjön-e rá később? | Igen: a referenciagép kártyáit (RTX A1000, RTX A4000) nem támogatja, NVIDIA-only és GPLv2. |
