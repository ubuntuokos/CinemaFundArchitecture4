# ADR 0017 – Rendszerszintű komponensek: a `cfa4-system-helper` és a jóváhagyási út

**Dátum:** 2026-11-28 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-28;
[CFA4-SETTINGS-HW-003](../plans/CFA4-SETTINGS-HW-003.md) D1–D6) · **Érintett rész:** Beállítások ▸ Hardver
és eszközök ▸ Rendszerszintű beállítások (root); `apps/cfa4-system-helper`; a `.deb` csomag polkit-szabálya.

## Háttér

A gyorsítók használatához néha rendszerszintű változtatás kell: a felhasználónak a `render` vagy `video`
csoportba kell kerülnie, hogy megnyithassa a `/dev/dri/renderD*` csomópontot, vagy hiányzik a GPU
VA-API-meghajtója. Eddig a CFA4 egyetlen root-műveletet ismert, a `cfa4-db-setup`-ot
(`crates/store/src/setup.rs`), amelyet a `postinst` vagy első indításkor a `pkexec` futtat. Általános,
jóváhagyáshoz kötött rendszer-segéd nem volt. A korábbi root-terv
([CFA4-SETTINGS-ROOT-001](../plans/CFA4-SETTINGS-ROOT-001-2026-10-26.md)) ADR-0004 számon tervezte ezt, de
az a szám foglalt ([ADR 0004](ADR-0004-compute-routing.md)), ezért ez az ADR 0017.

## Döntés

1. **A CFA4 soha nem fut rootként.** A GUI, a `cfa4-cli serve`, a vezérlőcsatorna és az MCP nem futtat
   tetszőleges root-parancsot.
2. **Egy út van:** ellenőrzés (root nélkül) → tervezet (root nélkül) → jóváhagyás a CFA4 ablakában →
   `pkexec` jelszókérés → végrehajtás a `cfa4-system-helper`-ben → utóellenőrzés.
3. **Zárt listák.** A segéd igéi: `status`, `check`, `plan`, `apply`, `disable`, `uninstall`. Komponensei:
   `gpu-access` (csoporttagság: `render`, `video`), `vaapi-drivers` (csomagok: `intel-media-va-driver`,
   `mesa-va-drivers`), `npu-check` (csak ellenőrzés). Más igét, komponenst, csoportot vagy csomagot nem
   fogad el, a hamisított tervezetben sem. A felhasználót rootként a `PKEXEC_UID`-ból számolja, nem a
   hívótól veszi át. A programokat abszolút úton, üres környezettel (`PATH` rögzítve, `LC_ALL=C`) indítja.
4. **Jóváhagyás nélkül semmi nem fut.** Ügynök (vezérlőcsatorna, MCP) is csak tervezetet kérhet; a
   végrehajtást a `pkexec` jelszókérése mindig a felhasználóhoz köti. A jelszót minden műveletnél kéri
   (`auth_admin`, D4). Az `apply` a jóváhagyott tervezet SHA-256 kivonatát kapja, rootként újra elkészíti
   a tervezetet, és csak egyezés esetén hajtja végre: a jóváhagyás után megváltozott rendszeren a régi
   tervezet nem fut le.
5. **Minden komponens visszavonható,** és a segéd csak azt vonja vissza, amit maga csinált: az
   állapotfájl (`/var/lib/cfa4/system-helper/state.json`) a még érvényes lépéseket tartja. Hibánál a
   félbemaradt `apply` korábbi lépései fordított sorrendben visszagördülnek. Sérült állapotfájl mellett a
   segéd nem változtat semmit. Minden lépés a rendszernaplóba kerül (`logger`, `cfa4-system-helper` címke).
6. **Gyártófüggetlenség.** A keret bármely gyártó komponensét fogadja. Gyártóhoz kötött komponens csak
   opcionális lehet, és semmilyen CFA4-funkció nem függhet tőle. NVIDIA saját meghajtóján a VA-API
   „nem kell” (ott az NVDEC a hardveres út); a gyártói GPU-meghajtót a CFA4 nem telepíti.
7. **Licenc.** GPL-2.0-only komponenst (például a GPU Unlock) a CFA4 nem tartalmaz és nem terjeszt. A GPU
   Unlock ebből a keretből kimarad (D6), rá külön terv jöhet.
8. **Csomagolás.** A segéd a `.deb`-ben a `/usr/libexec/cfa4/cfa4-system-helper` úton van, a polkit-szabály
   (`io.github.ubuntuokos.cfa4.system-helper`) csak ezt az egy programot engedi `pkexec`-kel. A tarball, az
   AppImage és a Flatpak nem tartalmazza: ott a root-oldal jelzi, hogy ez a telepítés nem kínálja.

## Következmények

- Új L6 bináris (`apps/cfa4-system-helper`), függőségei: `cfa4-route` (csak a sysfs-leltár), `serde`,
  `serde_json`, `sha2`, `chrono`. GUI-hoz, motorhoz és hálózathoz nem kötődik.
- A CI minden igét hamis gyökéren (`--root`) futtat; ott csak `--dry-run` változtathat, a parancsokat
  rögzítő futtató kapja. Valódi `gpasswd`/`apt-get` a CI-ben nem fut.
- A valódi gépen végzett `apply`/visszavonás a tulajdonos külön, helyben adott megerősítéséhez kötött; ha
  elmarad, a [`deferred-physical-tests.md`](../development/deferred-physical-tests.md) listába kerül.
- A motor (`system.*` serve-metódusok) és a Qt-oldal a HW-003 R2–R3 lépésében jön.
