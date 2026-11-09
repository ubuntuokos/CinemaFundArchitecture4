# CFA4 F9 – a kiadási kapuk állapota, 2026-11-08

**Ítélet: továbbra is HOLD.** Csomagot, GitHub-kiadást, APT-tárolót és frissítést továbbra sem
szabad közzétenni.

Ez a dokumentum a [2026-10-23-i F9 döntés](F9-RELEASE-DECISION-2026-10-23.md) és a
[`f9-release-readiness.json`](f9-release-readiness.json) kapuit nézi meg újra, a `main`
`a697fd70` állapotán (2026-11-08 10:13 UTC). Csak a bizonyítékokat gyűjti össze, kaput nem nyit
és nem zár. A gépi nyilvántartást (`f9-release-readiness.json`) szándékosan **nem** módosítja,
mert azt a nyitott #248 (LIC-03) is szerkeszti. A frissítést a #248 beolvasztása után külön
lépésben lehet elvégezni, ennek a dokumentumnak a 3. szakasza alapján.

Cél: Ubuntu 26.04 AMD64, négy csomagformátum (.deb, AppImage, Flatpak, tar.gz).

## 1. Összefoglaló

| Kapu (`id`) | JSON (2026-10-23) | Mostani állapot | Változás |
|---|---|---|---|
| `integration_ci` | PASS | PASS | a bizonyíték már a `main` tartalmára vonatkozik |
| `ubuntu_2604_amd64_policy` | PASS | PASS | nincs |
| `cfa4_release_ownership_source` | PASS | PASS | nincs |
| `four_artifact_structure_source` | PASS | PASS | nincs |
| `xdg_metadata_source` | PASS | PASS | nincs |
| `native_media_prior_physical_h264_hevc` | PASS | PASS | nincs (2026-10-22-i fizikai mérés) |
| `qt6_architecture_source` | PASS | PASS | nincs |
| `protected_main_integration` | OPEN | **PASS-ra javasolt** | a PR-lánc bekerült, a `main` védett, kötelező teljes CI-kapuval |
| `real_four_format_build` | OPEN | OPEN, **részben teljesül** | 2026-10-26: négy valódi csomag épült a saját futtatón, de nem a mostani `main`-ből |
| `full_qt6_nle_parity` | OPEN | OPEN, **nagyrészt teljesül** | `docs/qt-parity.md`: nincs ❌ sor, 4 sor 🟡 |
| `desktop_kde_gnome_cinnamon_runtime` | OPEN | OPEN, **részben teljesül** | valódi X11- és Wayland-szerveren fut a CI-ban, telepített csomaggal asztali környezetenként még nem |
| `av_content_sync_precision` | OPEN | OPEN, **részben teljesül** | a Qt-lejátszás a hanghoz igazítja a képet (#475), valódi kimeneti mérés nincs |
| `runtime_abi_dependencies` | OPEN | OPEN | a 2026-10-25-i mérés óta nincs újabb (azóta került a csomagba a Qt GUI) |
| `licensing_brand_attribution_sbom` | OPEN | OPEN | a #6 lezárult, a maradék a #220-ban és a nyitott #248-ban van |
| `clean_vm_install_uninstall` | OPEN | OPEN | nincs bizonyíték |
| `native_av1_nvdec_heif_icc_hdr` | OPEN | OPEN | a #18 nyitott, az NVDEC továbbra is csak H.264/HEVC |

Összesen: 8 PASS (ebből egy javasolt), 8 OPEN (ebből négy részben teljesül).

## 2. Kapunként

### `integration_ci`: PASS
A 2026-10-23-i bizonyíték egy integrációs ágra vonatkozott. Most a `main` minden beolvasztását
a kötelező `full-validation-gate` védi (`docs/coordination.md` §3.4).

- A `main` feje (`a697fd70`) a #581 beolvasztása. Fájltartalma megegyezik a PR fejével
  (`6ea2d1a1`, `git diff 6ea2d1a1 a697fd70` üres).
- Ezen a fejen a teljes validáció zöld volt:
  [42466677771](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/42466677771).
  Lefutott a Rust workspace, az FFmpeg-orákulumos tesztek, a Python és Qt 6, az architektúra- és
  eszközkapuk, a GStreamer, az OTIO-orákulum, a CLI-füsttesztek és a `full-validation-gate`.
- Figyelem: a `main`-re érkező push csak a szabályzati jobot futtatja (a többi job `skipped`).
  A push-futások zöld jele tehát önmagában nem bizonyíték. A `main`-en utoljára kézzel indított
  teljes futás: [41168720792](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/41168720792)
  (2026-10-31, zöld).

### `protected_main_integration`: PASS-ra javasolt
A JSON indoka („PR #17/#20 and stacked PR #23-#30 are still drafts”) elavult:
- A #20 2026-10-23-án bekerült. A #17 és a #23–#30 2026-10-24-én lezárult, a tartalmuk a #35-tel
  (`6889664`) került a `main`-be (`docs/audit/README.md`).
- 2026-10-31 óta minden beolvasztás feltétele a teljes CI zöld állapota azon a fejen
  (`full-validation-gate`), és csak a gazda olvaszt be, tulajdonosi jóváhagyással.
- A fenti `integration_ci` bizonyíték egyben beolvasztás utáni validáció a `main` tartalmán.

A PASS-ra állítás a tulajdonos döntése, a JSON-ban a #248 után.

### `real_four_format_build`: OPEN, részben teljesül
- A `cfa4-release-candidate.yml` kézi futása a tulajdonos saját futtatóján
  ([40363682631](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/40363682631),
  2026-10-26, zöld) mind a négy csomagot elkészítette: `cfa4-0.10.0-linux-x86_64.deb` (41 MB),
  `.AppImage` (37 MB, `.zsync`-kel), `.tar.gz` (40 MB) és a Flatpak-köteg. A `SHA256SUMS` és az
  F5-ös futásidejű ellenőrzés is lefutott, a csomagok a futtatón maradtak.
- Ez még nem elég, mert:
  1. az `adr0002/gstreamer-in-release` ágról épült, és a commitja (`6941cc86`) nincs a `main`-ben;
  2. azóta került a .deb és a tar.gz csomagba a Qt 6 GUI (#493, 2026-11-04);
  3. a nyitott #743 (ADR-0005-4) az FFmpeg 9.0.2 csomagolását változtatja meg (`/opt/cfa4`).
- Teendő: a #743 után egy kézi `build_four_packages: true` futás a `main` fejéről, és a
  `SHA256SUMS` rögzítése.

### `full_qt6_nle_parity`: OPEN, nagyrészt teljesül
A 2026-10-23-i indok („Qt6 is development shell”) már nem igaz. A
[`docs/qt-parity.md`](../qt-parity.md) szerint nincs el nem kezdett (❌) sor. 59 sor ✅,
4 sor 🟡:
- 1.9 Wayland és X11 minden Ubuntu 26.04 változaton;
- 1.10 csomagolás a Qt-futtatókörnyezettel;
- 4.2 exportmód;
- 5.5 GPU-képkockaút.

A nyitott M5.Qt PR-ok (#712, #716, #723, #727, #729, #731, összevonva #733) további sorokat
érintenek. A kapu akkor zárható, amikor a négy 🟡 sor ✅ lesz, és a tulajdonos fizikailag is
kipróbálta.

### `desktop_kde_gnome_cinnamon_runtime`: OPEN, részben teljesül
- A #503 (2026-11-03) óta a Qt GUI a CI-ban valódi X11-szerveren (Xvfb, `xcb`) és valódi
  Wayland-kompozitoron (weston headless) fut, és a vezérlőcsatorna vezérli.
- Hiányzik: telepített csomagból, formátumonként, KDE / GNOME / Cinnamon (és a többi 26.04
  változat) munkamenetében futtatva még nincs mérés.

### `av_content_sync_precision`: OPEN, részben teljesül
- A #475 (2026-11-04) óta a Qt-lejátszásban a kép a hangot követi. A Qt `perf.stats` megmutatja
  a megjelenített és eldobott képkockákat (#489).
- Hiányzik: exportált fájl tartalmi A/V-eltolásának mérése (például villanás és sípjel minta,
  orákulummal).

### `runtime_abi_dependencies`: OPEN
- A 2026-10-25-i mérés ([F2-01](F2-01-RUNTIME-DEPS-2026-10-25.md)) óta nincs újabb.
- Azóta új futásidejű függőségek jöttek: a Qt GUI a csomagban az Ubuntu PySide6-jával (#493),
  a PipeWire-őr (#505). Ezért a mérést a négy csomagon újra kell futtatni:
  `bash scripts/dev/audit-runtime-deps.sh --build`, a Flatpak-homokozóval együtt.

### `licensing_brand_attribution_sbom`: OPEN
- A #6 lezárult (2026-10-27). Bekerült a cargo-deny, az SBOM, a Python hash-zár (#66) és a
  licencfájlok a csomagokba (#213).
- Nyitott: a #220 (teljes harmadik féltől származó licenclista a csomagban, PySide6/QtWebEngine
  terjesztési licencterv, FFmpeg/HEIF szabadalmi felülvizsgálat, valódi csomagok tartalmi
  vizsgálata) és a #248 (LIC-03, szabadalmi vizsgálat és kiadási kapu).
- A régi „MIT OR Apache-2.0” projektlicenc-szöveg gyakorlatilag eltűnt (#358, lásd #745).

### `clean_vm_install_uninstall`: OPEN
Nincs bizonyíték tiszta Ubuntu 26.04 AMD64 rendszeren mind a négy formátum telepítésére és
eltávolítására. Ez a `real_four_format_build` után következik.

### `native_av1_nvdec_heif_icc_hdr`: OPEN
- A #18 nyitott. A `crates/platform/src/nvdec` továbbra is csak H.264/HEVC-t dekódol.
- A HEIF-integráció ([HEIF-HEIC-LINUX](HEIF-HEIC-LINUX-2026-10-22.md)) továbbra is az első
  állóképet kezeli. Többképes sorozat, teljes ICC/EXIF és 10/12 bites HDR nincs.

## 3. Javasolt JSON-módosítás (a #248 beolvasztása után, tulajdonosi döntéssel)

- `as_of`: a módosítás napja.
- `integration_ci.evidence` → a fenti 42466677771-es futás, az indokban a `main` fejével.
- `protected_main_integration` → `PASS`, indok: a PR-lánc a #35-tel bekerült, a `main`
  beolvasztását a `full-validation-gate` kényszeríti ki.
- A többi OPEN kapu indoka a 2. szakasz „Hiányzik” sorai szerint frissüljön, a lezárt
  #26–#30 PR-hivatkozások helyett élő bizonyítékkal vagy issue-val.
- `verdict`: marad `HOLD`, `release_publication_authorized`: marad `false`.

## 4. A következő lépések sorrendje

1. #743 (FFmpeg a csomagokban), utána négyformátumú build a `main` fejéről
   (`real_four_format_build`).
2. Futásidejű függőségek újramérése a négy csomagon (`runtime_abi_dependencies`).
3. Tiszta VM-telepítés és -eltávolítás, majd asztali környezetenkénti futtatás
   (`clean_vm_install_uninstall`, `desktop_kde_gnome_cinnamon_runtime`). Ehhez a tulajdonos
   gépe vagy egy saját futtató kell.
4. #220 és #248 (licenc és szabadalom).
5. A négy 🟡 Qt-paritássor, A/V tartalmi mérés, AV1 NVDEC (#18) és HEIF.
