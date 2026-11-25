# BacklotHub 11a. lépcső – DCC Integration Host: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-14 · **Terv:** CFA4-BACKLOTHUB-001 F5 · **Előzmény:**
[G1 DCC Integration Host](CFA4-BACKLOTHUB-G1-11A-DCC-INTEGRATION-HOST-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-09 (Workfiles), G2-10 (Publisher), G2-11 (Loader), CFA4-APP-LAUNCHER-001
(jóváhagyva 2026-11-14, ADR 0014) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-21) · implementáció nincs

Egy új döntési kérdés van (#57, Pyblish a DCC-ben). Új függőség nincs, ha a #57 a javaslat szerint dől el.

## 1. Hatókör ebben a lépcsőben

- **Integrációs host:** a BacklotHub-szolgáltatás helyi socketje a DCC-adaptereknek.
- **Adapterkeretrendszer:** közös Python-csomag, amely a DCC saját Python-értelmezőjében fut.
- **Első adapter: Blender.** A Krita és a Natron adaptere ugyanerre a keretre épül, külön allépcsőként
  (11a-2, 11a-3), mert mindkettőnek más a bővítménybetöltése (3.4 pont).
- **Nem része:** OpenAssetIO (csak kereskedelmi DCC-hez éri meg, 9. pont), értesítés a DCC-be („új verzió
  jött”; a 12. lépcsőtől), és az alkalmazás indítása (Launcher).

## 2. Integrációs határok

```
 Launcher (cfa4-cli serve, launcher.launch)
     │  CFA4_CONTEXT (JSON), CFA4_HOST_SOCKET, adapter betöltése ({launcher.addons})
     ▼
 DCC (Blender) ── cfa4_dcc adapter ──► Unix-socket ──► BacklotHub DCC Host ──► backlot.* parancsok
                                     JSON-sorok,        (engedélyezett lista)
                                     verzióval
```

| Határ | Irány | Szerződés |
|---|---|---|
| Launcher (AL-1, AL-2) | ← | Indítás a változat környezeti szabályaival; `{host.socket}` és `{launcher.addons}` helyőrző; `CFA4_CONTEXT` (produkció, feladat, munkafájl, gyökerek) |
| Workfiles (9.) | → | `backlot.workfile.context`, `backlot.workfile.create`, `backlot.workfile.register` (mentés új verzióként) |
| Publisher (10.) | → | `backlot.publish.validate`, `backlot.publish.run`, `backlot.publish.status`, `backlot.publish.cancel`; a DCC a Collect és az Extract fázist adja (5. pont) |
| Loader (11.) | → | `backlot.load.candidates`, `.check`, `.run`, `.inventory`, `.update`, `.remove`; a célalkalmazás itt a DCC |
| Search (7.) | → | `backlot.search.query` csak olvasásra (a betöltési párbeszédhez) |
| File Services | → | A helyi utat a BacklotHub oldja fel (`files.resolve`); az adapter nem ismeri a gyökerek szerkezetét |
| Notification (12.) | később | „Új verzió jött” jelzés a DCC-be; addig az adapter lekérdezéskor kérdezi (`backlot.load.inventory`) |

## 3. Integrációs host

### 3.1 Socket és hozzáférés

- **Hely:** `$XDG_RUNTIME_DIR/cfa4/backlothub.sock`; a könyvtár `0700`, a socket `0600`. Hálózaton nem érhető
  el (SEC-02). A BacklotHub-szolgáltatás nyitja, leálláskor törli; elárvult socketet induláskor felismer és
  lecserél.
- **Partnerellenőrzés:** minden kapcsolatnál `SO_PEERCRED`; csak a szolgáltatással azonos UID fogadható el,
  másnál a kapcsolat azonnal bezárul (`DCC_PEER_REJECTED` a naplóban).
- **Token nincs.** A vezérlőcsatornától (G2-11) eltérően itt nem kell: a socketet csak a felhasználó érheti el,
  és a DCC-host parancsai ugyanazok, amelyeket a felhasználó a BacklotHub felületén is futtathat, a saját
  nevében. Így a kézzel indított DCC is csatlakozhat (3.3).
- **Flatpak:** a sandbox alapból nem látja a `$XDG_RUNTIME_DIR/cfa4` könyvtárat. A Launcher indítás előtti
  ellenőrzése (ADR 0014, 7. pont) ezt is vizsgálja; hiányzó engedélynél `LAUNCH_SANDBOX_NO_ACCESS`, és a felület
  a szükséges `flatpak override --filesystem=xdg-run/cfa4` parancsot mutatja. Engedélyt a rendszer magától nem
  módosít.

### 3.2 Protokoll

- A CFA4 vezérlőcsatorna JSON-sor formátuma (`docs/control-protocol.md`): egy kérés, egy válasz soronként,
  kérésazonosítóval.
- **Kézfogás** a kapcsolat elején: `{"cmd": "dcc.hello", "protocol": "cfa4.dcc_host/1", "adapter": "blender",
  "adapter_version": "…", "app_version": "4.5.1", "launch_id": "…"}`. Ismeretlen fő protokollverziónál
  `DCC_PROTOCOL_MISMATCH`, és a kapcsolat bezárul; a felület megmondja, melyik oldal régebbi.
- **Engedélyezett parancslista:** csak a 2. pont táblázatának `backlot.*` parancsai, valamint `dcc.hello`,
  `dcc.context` és `dcc.ping`. Minden más `DCC_COMMAND_NOT_ALLOWED`. Adminisztratív parancs (produkció,
  folyamatdefiníció, profil szerkesztése) a DCC-ből nem hívható.
- A parancsok a BacklotHub közös regiszterén futnak (#51), ugyanazzal a jogosultság- és auditútvonallal, mint
  a felületről. Az audit eseménye megkapja a forrást: `{"via": "dcc", "adapter": "blender", "launch_id": …}`.
- Méretkorlát soronként (1 MiB) és kapcsolatonként egyszerre egy futó hosszú parancs; a publikálás és a
  betöltés állapotát az adapter `backlot.publish.status` hívással kérdezi, nem tartja nyitva a kérést.

### 3.3 Kontextus

- **Launcherrel indítva:** a `CFA4_CONTEXT` környezeti változó (JSON) adja a produkciót, a feladatot és a
  munkafájlt; az adapter a `dcc.context` paranccsal ellenőrzi, hogy még érvényes-e (pl. a feladatot közben nem
  zárták le).
- **Kézzel indítva:** nincs `CFA4_CONTEXT`. Ha a megnyitott fájl egy regisztrált munkafájl útja, a host ebből
  adja vissza a kontextust (`backlot.workfile.context`); különben az adapter párbeszédben kérdezi meg a
  felhasználót (produkció, feladat), és amíg nincs kontextus, a menüpontok tiltottak, a DCC rendesen működik.
- **Socket hiányában** (a BacklotHub nem fut): `DCC_HOST_UNAVAILABLE`, az adapter menüje ezt jelzi, és
  felajánlja az újrapróbálást. A DCC munkája nem áll meg.

### 3.4 Az adapter betöltése (AL-2-vel együtt)

- A felhasználó saját beállításait és bővítménykönyvtárát a betöltés **nem írhatja felül és nem módosíthatja**.
- **Blender:** a változat argumentumlistája `--python {launcher.addons}/blender/cfa4_bootstrap.py`; a
  betöltőszkript a csomagot a saját útjáról regisztrálja. A `BLENDER_USER_SCRIPTS` felülírását nem használjuk,
  mert az a felhasználó saját szkriptkönyvtárát kiváltaná.
- **Natron (11a-3):** `NATRON_PLUGIN_PATH` elé fűzve (`prepend`), ez nem írja felül a meglévőt.
- **Krita (11a-2):** a Python-bővítményeket a Krita a saját erőforráskönyvtárából tölti, és kézi engedélyezést
  kér. A betöltés módja a 11a-2 tervében dől el (ellenőrzés: van-e felülírás nélküli út); addig Krita-adapter
  nincs.
- Kézzel indított DCC-hez a felület egyszeri telepítési útmutatót ad (Blenderben bővítményként telepíthető
  csomag); automatikus telepítés nincs.

## 4. Adapterkeretrendszer (`cfa4_dcc`)

- **Csak a Python standard könyvtára**, mert a DCC-k saját, eltérő verziójú Pythont hoznak (Blender 4.x/5.x:
  3.11+). Legalacsonyabb támogatott verzió: Python 3.10; a csomag mindkét Blender-sorozattal tesztelve.
- **Rétegek:**
  1. **Kliens:** socket, kézfogás, kérés–válasz, időkorlát, újracsatlakozás.
  2. **Közös felület (`DccHost` absztrakt osztály):** aktuális fájl útja; mentés adott útra; megnyitás;
     „container” létrehozása, listázása, frissítése és eltávolítása; menü regisztrálása; üzenet a
     felhasználónak.
  3. **Közös munkafolyamatok** (a felületre épülve, minden DCC-ben ugyanazok): munkafájl mentése új verzióként,
     publikálás, betöltés, „van újabb verzió” lista, frissítés.
  4. **DCC-specifikus rész:** a `DccHost` megvalósítása (Blenderben `bpy`).
- **Never crash a DCC-ben:**
  - minden adapterbelépési pont kivételkezelővel fut; hiba esetén üzenet és napló, a DCC nem áll le;
  - a socket-hívás nem blokkolja a DCC felületét: háttérszál, az eredmény a DCC fő szálán alkalmazódik
    (Blenderben `bpy.app.timers`), mert a `bpy` csak a fő szálról hívható biztonságosan;
  - időkorlát minden hívásra (alapból 10 s, a hosszú műveleteknél állapotlekérdezés);
  - az adapter napló a `$XDG_STATE_HOME/cfa4/dcc/<adapter>.log` fájlba megy, rotálva.
- **Container a DCC-ben:** a betöltött elemhez az adapter a jelenetben tárolja a `load_record` azonosítóját
  (Blenderben egyedi tulajdonság a betöltött gyűjteményen vagy adatblokkon). Ez az AYON container-mintája
  (Apache-2.0), saját megvalósításban.

## 5. Publikálás a DCC-ből (#57)

A Publisher (G2-10) négy fázisa megoszlik:

| Fázis | Hol fut | Mit csinál |
|---|---|---|
| **Collect** | DCC (adapter) | A jelenetből publikálható példányok (pl. modell, animáció, render), a felhasználó választásával |
| **Validate (DCC-oldali)** | DCC (adapter) | Csak olyan ellenőrzés, amelyhez a jelenet kell (pl. hiányzó textúra, nem alkalmazott transzformáció); az adapter saját, zárt ellenőrzőkészlete, eredménye a futás része |
| **Extract** | DCC (adapter) | Exportálás a `staging` területre, a BacklotHubtól kapott útra (útsablon, G2-09) |
| **Validate + Integrate** | BacklotHub | A G2-10 zárt ellenőrzőregisztere a kimenetre, majd a File Services lezárás és egy tranzakció (G2-10, változatlan) |

- **Javaslat (#57): Pyblish nélkül.** A DCC-oldali Collect, Validate és Extract az adapterkeret saját, kis
  bővítményfelülete (osztály `id`-vel, paraméterekkel, eredménnyel), mint a BacklotHub-oldali futtató (G2-10).
  Indok: egy futási modell van mindkét oldalon; a DCC-k Pythonjába nem kell külső csomagot telepíteni (Blender
  4.2-től a bővítményeknek a függőségeket csomagolniuk kell); és a Pyblish legnagyobb előnye, a tetszőleges
  bővítménykód, nálunk tiltott (G2-10: tetszőleges kód nem fut). A Pyblish (LGPL-3.0) a fázisok mintája marad.
- A publikálás eredménye és hibája ugyanaz, mint a felületről indítottnál; a DCC-oldali ellenőrzés hibája a
  futás `rejected` állapota, a jelenet változatlan.
- A publikálás előtt az adapter a munkafájlt új verzióként menti (`backlot.workfile.register`), így a
  `publish_run` a mentett munkafájl-verzióra hivatkozik, és a `version_input` a munkafájl aktív betöltéseiből
  áll elő (G2-10, G2-11).

## 6. Betöltés a DCC-be

- Ugyanaz a Loader-szerződés, mint a CFA4 szerkesztőnél (G2-11), más céllal: `load_record.app = 'blender'`,
  `target_ref = {"workfile": "<id>", "container": "<a jelenetbeli azonosító>"}`.
- A betöltési profil (`load_profile.app`) alkalmazásonként külön; a Blender alapprofilja: `alembic`, `usd`,
  `fbx`, `blend` (hivatkozás vagy hozzáfűzés), képsorozat textúraként.
- **Félig betöltött állapot nem marad** (G2-11, 6. pont): ha N elemből egy elbukik, az adapter eltávolítja a
  már betöltötteket, és a host nem ír `load_record`-ot.
- **Leltár-egyeztetés:** a munkafájl megnyitásakor az adapter elküldi a jelenetben talált containerek listáját;
  a host összeveti az aktív `load_record`-okkal. A jelenetből kézzel törölt elemnél a felület felajánlja a
  rekord `removed` állapotba tételét; magától nem írja át.

## 7. Adatmodell

**Új tábla nincs.** Ami kell, a meglévő táblákban van:

- `load_profile.app`, `load_record.app` és `target_ref` (G2-11) a DCC-re is;
- `publish_profile` a gazdaalkalmazás szerinti szűrővel (G2-10);
- az audit `via` mezője a forrást (felület, DCC, CLI) jelzi;
- az indítás diagnosztikai adata a Launcher `LaunchRecord`-jában (ADR 0014), nem az adatbázisban.

A csatlakozott adapterek listája (melyik DCC, melyik munkafájl, mióta) a host memóriájában van, a
`backlot.dcc.sessions` paranccsal kérdezhető (felület, hibakeresés); nem üzleti adat, nem tárolódik.

## 8. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `dcc.hello`, `dcc.ping` | Kézfogás és élő-e (csak a socketen) |
| `dcc.context` | Az érvényes kontextus visszaadása vagy ellenőrzése |
| `backlot.dcc.sessions` | A csatlakozott adapterek (a felületnek) |

Hibakódok: `DCC_HOST_UNAVAILABLE`, `DCC_PEER_REJECTED`, `DCC_PROTOCOL_MISMATCH`, `DCC_COMMAND_NOT_ALLOWED`,
`DCC_CONTEXT_MISSING`, `DCC_CONTEXT_STALE` (a feladat vagy munkafájl közben megváltozott), `DCC_ADAPTER_ERROR`,
valamint a `LOAD_*`, `PUBLISH_*`, `WORKFILE_*`, `FILES_*` és `LAUNCH_*` kódok változatlanul.

## 9. OpenAssetIO

**Javaslat: M1-ben nem.** A G1 szerint főleg kereskedelmi programok (Nuke, Katana) használják; Blenderhez csak
demószkript van, Krita és Natron nem szerepel. Újravizsgálat: ha kereskedelmi DCC támogatása igény lesz. Akkor a
BacklotHub egy OpenAssetIO manager-bővítményként csatlakozhat ugyanerre a hostra, a fenti parancsokkal.

## 10. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Host és keret | 9., 10., 11. lépcső megvalósítása; Launcher AL-1 |
| Blender-adapter | AL-2 (`{launcher.addons}`, telepítés ellenőrzése) |
| Krita (11a-2), Natron (11a-3) | A Blender-adapter G5-je; Krita betöltési módja |
| Értesítés a DCC-be | 12. lépcső (Notification Center, EF-R1) |

## 11. Tesztek (G5-re előre)

- **Protokoll-szerződésteszt** egy hamis adapterrel (tiszta Python, DCC nélkül): kézfogás, verzióeltérés,
  nem engedélyezett parancs, méretkorlát, félbeszakadt sor, a host újraindítása kapcsolat közben.
- **Hozzáférés:** a socket és a könyvtár jogai; más UID-ú kapcsolat elutasítva (tesztben külön felhasználóval,
  ha a CI engedi, különben a `SO_PEERCRED` ellenőrző függvény egységtesztje).
- **Fuzz:** véletlen és csonkolt JSON-sorok a hostnak; összeomlás nincs, a többi kapcsolat működik.
- **Blender, fej nélkül** (`blender -b --python …`), a Blender 4.x és 5.x sorozattal; ha nincs telepítve,
  a teszt kihagyva, nem elbukva: munkafájl mentése, publikálás (Collect–Extract–Integrate), betöltés,
  frissítés, leltár-egyeztetés.
- **Never crash:** a BacklotHub leáll publikálás közben; az adapter hibát dob a fő szálon kívül; a DCC tovább
  fut, a jelenet változatlan, az adatbázisban nincs félkész rekord.
- **Kézzel indított DCC:** kontextus a munkafájl útjából, illetve párbeszédből.
- **Flatpak:** hiányzó `xdg-run/cfa4` engedély → `LAUNCH_SANDBOX_NO_ACCESS` és az ajánlott parancs.

## 12. Licenc (G3-ra előre)

- **Új függőség nincs** (a #57 javaslata szerint). A keret csak a Python standard könyvtárát használja.
- A Blender-adapter a CFA4 része, GPL-3.0-or-later, ami a Blender-bővítmények GPL-követelményének megfelel.
- AYON Blender és AYON core (Apache-2.0): host-interfész, container, menü mint minta; kódátvételnél
  attribúcióval (`NOTICE`). Pyblish (LGPL-3.0): a fázisok mintája, kód nem. OpenAssetIO: nem használjuk.

## 13. Nyitott kérdés

| # | Kérdés | Javaslat |
|---|---|---|
| 57 | Használja-e a DCC-oldali publikálás a Pyblisht? | Nem: az adapterkeret saját, zárt bővítményfelülete, ugyanaz a modell, mint a BacklotHub-oldali futtatóé; nincs külső csomag a DCC-k Pythonjában |

## 14. G2-eredmény

**Javaslat: PASS**, a #57 eldöntésével. A megvalósítás a 9–11. lépcső és a Launcher AL-1, AL-2 fázisa után
indulhat.
