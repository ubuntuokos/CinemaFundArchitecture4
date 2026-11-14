# DCPVAL-LIBDCP-01 – A CFA4 DCP-exportja a libdcp `dcpverify`-án (mérés)

**Terv:** [CFA4-DCPVAL-LIBDCP-001](../plans/CFA4-DCPVAL-LIBDCP-001-FINAL-1.0.md), 1. fázis · **Dátum:** 2026-11-14 · **Kód nem változott.**

## 1. Mit mértem

- **Ellenőrző:** libdcp v1.10.64 (`891facc8`) `dcpverify`, asdcplib v1.0.10 és libcxml v0.17.17 mellett, a [DCPOM-01 fordítószkript](DCPOM-01-build.sh) libdcp-részével, Ubuntu 24.04.5 rendszer-OpenJPEG 2.5.0-val. Alapbeállítás: a képkocka-ellenőrzés (`check_picture_details`) be van kapcsolva.
- **Minta:** a CFA4 `main` (`89107cf8`) DCP-exportja, a `cfa4-export` két meglévő tesztjének kimenete:
  - `dcp_tests::two_k_flat_package`: 2K Flat SMPTE DCP, 10 képkocka, 25 kép/s, kép + 5.1 hang;
  - `dcp_tests::four_k_scope_without_sound`: 4K Scope SMPTE DCP, hang nélkül.
- **Parancs:** `cargo test -p cfa4-export --release --lib -- dcp_tests::two_k_flat_package dcp_tests::four_k_scope_without_sound`, majd `dcpverify <mappa>`.

## 2. Eredmény

Mindkét csomagra `dcpverify` rc = 1. Az azonos üzeneteket összevontam (`N` = képkocka):

| Üzenet (libdcp) | Típus | 2K Flat | 4K Scope | Mi az oka |
|---|---|---|---|---|
| `Frame N has an invalid JPEG2000 codestream (invalid multi-component transform flag 0)` | ERROR | 10 / 10 képkocka | 1 | **A terv 2. eltérése.** A CFA4 MCT = 0-val kódol (`crates/export/src/dcp_out.rs:336`). |
| `There is an <EntryPoint> node inside a <MainMarkers>` | ERROR | 1 | 1 | **Új eltérés, a tervben nem szerepel** (3. pont). |
| `There is an <Duration> node inside a <MainMarkers>` | ERROR | 1 | 1 | Ugyanaz. |
| `The (intrinsic) duration of the asset … is less than 1 second` | ERROR | 4 | 2 | A tesztminta rövid (10 képkocka). Nem az export hibája. |
| `The DCP is marked as a Feature but there is no FFEC / FFMC marker` | Bv2.1 error | – | 2 | A teszt `feature` tartalomtípust ad jelölők nélkül. Nem az export hibája. |
| `The LFOC marker is 00:00:00:01 instead of 1 less than the duration of the last reel` | WARNING | – | 1 | A rövid minta mellékhatása. |

A teljes nyers kimenet (`dcpverify -q`) a 4. pontban van.

## 3. Következtetések

1. **A D3 feltétele teljesül:** a CFA4 mai DCP-exportja minden képkockán elbukik a `dcpverify` MCT-ellenőrzésén, ERROR súllyal. A terv 4. fázisa (`mct: true`) ezért indul.
2. **Új eltérés: `MainMarkers` `EntryPoint` és `Duration`.** A CFA4 a jelölő-assetbe (`MainMarkers`) `EntryPoint`-ot és `Duration`-t ír. A libdcp ezt ERROR-nak veszi (`src/verify.cc`, `UNEXPECTED_ENTRY_POINT` / `UNEXPECTED_DURATION`). A libdcp forrása nem nevezi meg, melyik szabványpontra épül; valószínűleg a SMPTE Bv2.1 alkalmazási profilra (RDD 52), de ezt nem ellenőriztem. A jóváhagyott terv ezt nem fedi le. Nem javítom, külön tulajdonosi döntést kér: a terv kiegészítése vagy új terv.
3. A többi üzenet a 10 képkockás tesztmintából ered, a valódi exportot nem minősíti.

### 3.1 Ellenőrzés az MCT-javítással (a 4. fázis helyi ága, 2026-11-14)

Ugyanez a két teszt a 4. fázis kódjával (`cfa4-j2k` mozis beállítás: `mct: true`), ugyanazzal a `dcpverify`-jal:

| Üzenet (libdcp) | Típus | 2K Flat | 4K Scope |
|---|---|---|---|
| `invalid multi-component transform flag 0` | ERROR | **eltűnt** | **eltűnt** |
| `The JPEG2000 codestream uses 2 guard bits in a 2K image instead of 1` | Bv2.1 error | 10 / 10 képkocka | – |
| `<EntryPoint>` / `<Duration>` a `MainMarkers`-ben | ERROR | 1 + 1 | 1 + 1 |
| rövid asset, Feature-jelölők, LFOC | ERROR / Bv2.1 / WARNING | mint fent | mint fent |

**Második új eltérés: 2K guard bitek.** Az MCT-hiba eddig elfedte, mert a libdcp az első kódfolyam-hibánál abbahagyja a képkocka elemzését. A `cfa4-j2k` mindig legalább 2 guard bitet ír (`crates/j2k/src/encode/mod.rs`, „guard bits: enough for the largest index of every band”). A 2K mozis profil 1-et ír elő. Ezt a CFA4 saját ellenőrzője is tudja: a `DCP.J2K.001` a CFA4 2K-exportját ezen elbuktatja (`crates/dcp-validate/src/tests.rs`, `codestreams_follow_the_dci_profile`: „as cfa4-j2k writes it: 2 guard bits”). A javítás a kódoló kvantálását érinti (1 guard bitnél a lépésközt kell úgy választani, hogy a legnagyobb index is elférjen), ezért ez sem fér a jóváhagyott tervbe.

## 4. Nyers kimenet (`dcpverify -q`)

### 2K Flat (`two_k_flat_package`)

```
Error: The duration of the asset ff99ac70-10e5-44f6-b7a9-d23d3b760852 is less than 1 second.
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┗━ Reel number 0
Error: The intrinsic duration of the asset ff99ac70-10e5-44f6-b7a9-d23d3b760852 is less than 1 second.
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┗━ Reel number 0
Error: The duration of the asset cb2ea247-2e5c-4c6f-8b42-d22347ec0b20 is less than 1 second.
  ┣━ Asset ID cb2ea247-2e5c-4c6f-8b42-d22347ec0b20
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┗━ Reel number 0
Error: The intrinsic duration of the asset cb2ea247-2e5c-4c6f-8b42-d22347ec0b20 is less than 1 second.
  ┣━ Asset ID cb2ea247-2e5c-4c6f-8b42-d22347ec0b20
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┗━ Reel number 0
Error: Frame 0 (timecode 00:00:00:00) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 0
  ┗━ Reel number 0
Error: Frame 1 (timecode 00:00:00:01) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 1
  ┗━ Reel number 0
Error: Frame 2 (timecode 00:00:00:02) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 2
  ┗━ Reel number 0
Error: Frame 3 (timecode 00:00:00:03) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 3
  ┗━ Reel number 0
Error: Frame 4 (timecode 00:00:00:04) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 4
  ┗━ Reel number 0
Error: Frame 5 (timecode 00:00:00:05) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 5
  ┗━ Reel number 0
Error: Frame 6 (timecode 00:00:00:06) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 6
  ┗━ Reel number 0
Error: Frame 7 (timecode 00:00:00:07) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 7
  ┗━ Reel number 0
Error: Frame 8 (timecode 00:00:00:08) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 8
  ┗━ Reel number 0
Error: Frame 9 (timecode 00:00:00:09) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID ff99ac70-10e5-44f6-b7a9-d23d3b760852
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┣━ Frame 9
  ┗━ Reel number 0
Error: There is an <EntryPoint> node inside a <MainMarkers>.
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┗━ Reel number 0
Error: There is an <Duration> node inside a <MainMarkers>.
  ┣━ CPL ID 7f0a9682-85db-4117-a2da-42a78139d00f
  ┗━ Reel number 0
```

### 4K Scope (`four_k_scope_without_sound`)

```
Error: The duration of the asset 2d9a10e3-b430-4815-83f0-29e4c02d267b is less than 1 second.
  ┣━ Asset ID 2d9a10e3-b430-4815-83f0-29e4c02d267b
  ┣━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
  ┗━ Reel number 0
Error: The intrinsic duration of the asset 2d9a10e3-b430-4815-83f0-29e4c02d267b is less than 1 second.
  ┣━ Asset ID 2d9a10e3-b430-4815-83f0-29e4c02d267b
  ┣━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
  ┗━ Reel number 0
Error: Frame 0 (timecode 00:00:00:00) has an invalid JPEG2000 codestream (invalid multi-component transform flag 0).
  ┣━ Asset ID 2d9a10e3-b430-4815-83f0-29e4c02d267b
  ┣━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
  ┣━ Frame 0
  ┗━ Reel number 0
Error: There is an <EntryPoint> node inside a <MainMarkers>.
  ┣━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
  ┗━ Reel number 0
Error: There is an <Duration> node inside a <MainMarkers>.
  ┣━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
  ┗━ Reel number 0
Bv2.1 error: The DCP is marked as a Feature but there is no FFEC (first frame of end credits) marker.
  ┗━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
Bv2.1 error: The DCP is marked as a Feature but there is no FFMC (first frame of moving credits) marker.
  ┗━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
Warning: The LFOC marker is 00:00:00:01 instead of 1 less than the duration of the last reel.
  ┗━ CPL ID dc54842f-b69e-42c7-b191-f1c1bf6c9af7
```
