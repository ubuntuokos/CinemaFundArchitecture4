# CFA4-DVD-MENU-001 — VÉGLEGES 1.0 — DVD-Video menük, feliratok (subpicture), több cím, pulldown (#1812, #244 3. rész)

Állapot: **jóváhagyva** (István, 2026-11-27: minden javaslat elfogadva; a D1 indulása a #1841 beolvasztásától függ, lásd 4a.). Régi állapot: jóváhagyásig nem készült kód (`docs/coordination.md` §2.10). A terv nem PR, hanem külön .md (állandó szabály); jóváhagyás után kerül a `docs/plans/` mappába a megvalósító PR-ral.
Dátum: 2026-11-27. Utána jön a Blu-ray (#245), külön terv.

## Rövidítések (első említéskor kifejtve)

| Rövidítés | Jelentése |
|---|---|
| DVD | Digital Versatile Disc |
| IFO / BUP | Information file (`VIDEO_TS.IFO`, `VTS_01_0.IFO`) és annak tartalék másolata (Backup) |
| VOB | Video Object, a DVD videofájl |
| VOBU | Video Object Unit, a VOB legkisebb lejátszási egysége (egy GOP) |
| VMG / VMGM | Video Manager / Video Manager Menu, a lemez főmenüje |
| VTS / VTSM | Video Title Set / Video Title Set Menu, a címhalmaz és menüje |
| PGC | Program Chain, a lejátszási lánc |
| NAV | Navigation pack, navigációs csomag (PCI + DSI) |
| PCI / DSI | Presentation Control Information / Data Search Information |
| HLI | Highlight Information, a gombkiemelés adatai a PCI-ban |
| SPU | Sub-Picture Unit, a DVD bitképes felirat- és gombgrafika |
| VM | Virtual Machine, a lejátszó parancsvégrehajtója |
| CLUT | Color Look-Up Table, színtábla |
| GOP | Group Of Pictures |
| T-STD | Transport stream System Target Decoder (MPEG-2 dekódermodell) |
| ADR | Architecture Decision Record |


## 1. Kiindulás (a main ágon ellenőrizve, 2026-11-27)

- `crates/dvd` (main, #244 1. rész): egy cím, menük nélkül. `author()` kitölti a NAV-csomagok PCI/DSI-jét, `build_vts` / `build_vmg` írja az IFO-t, `write_disc` adja a `VIDEO_TS` mappát. Egy PGC, a fejezetek programok/cellák.
- `cfa4-mpegts` `Muxer::new_dvd`: 2048 bájtos csomagok, üres NAV-csomag minden GOP elején.
- Az export-bekötés (preset, `mpeg2Container: "dvd"`, Qt6 vezérlő) a **#1841** PR-ban van, **még nincs beolvasztva**; a T-STD (#1753/#1857) szintén nyitott. A tervezett munka `crates/dvd`-t érinti elsősorban, az exportnál csak `mpeg_out.rs` / `job.rs` kerül hozzá, ott ütközés várható #1841-gyel és #1857-tel: ezért **a bekötési rész csak #1841 beolvasztása után** indul (nem építek rá).
- Oracle: `lsdvd`, `dvdauthor` (GPL-2, csak viselkedés, kódot nem veszünk át), libdvdread/libdvdnav hiányában a `lsdvd -x`. Referencia: mpucoder dvdinfo oldalai (dvd.sourceforge.net/dvdinfo: ifo, pgc, pci_pkt, dsi_pkt, vmi, spu).

## 2. Cél és határ

A #1812 lista hat pontja négy feladatra bontva, **egy PR = egy feladat**, mind friss `main`-ből, egymásra nem építve (ahol függ, az előzőt megvárja):

| Fázis | Tartalom | Függ |
|---|---|---|
| **D1 Menük** | VMGM (főmenü) és fejezetmenü; menü-VOB; gombok (HLI a PCI-ban); VM-parancsok; PGC-parancstábla | #1841 (export-bekötés) |
| **D2 Feliratok** | subpicture sáv (SPU, 2 bites RLE), `sp_synca`, PGC_SPST_CTL, `VTS` subpicture-attribútumok | D1 SPU-kódolója (a gombok is SPU) |
| **D3 Több cím** | több VTS, több PGC, szög (angle), ILVU-interleaving, szülői szint, régiókód | D1 |
| **D4 Pulldown, időkód** | repeat_first_field a VOBU-időtartamban, STC-megszakadás a cellakategóriában, DTS/MPEG hang-attribútumok, `vob_a_stp_ptm` hanghézag | #277 |

A D1 az, amit István kért („DVD menus export”); a D2–D4 ugyanennek az issue-nak a folytatása, külön PR-okban, a jóváhagyott sorrendben.

## 3. D1 — menük: részletes terv

### 3.1 Menümodell (`crates/dvd`, serde)
`DiscMenu { background: MenuBackground, buttons: Vec<MenuButton>, audio: Option<..> }`
- `MenuBackground`: állókép (egy I-kép, a `cfa4-mpeg2v` kódolóval 720×576/480 MPEG-2 I-képként; az idő legalább 1 s, a menü ismétlődik a VM-parancs `Link`-jével) — mozgó háttér később.
- `MenuButton { id, rect, label, action }`; `action`: `PlayTitle(n)`, `PlayChapter(title, ptt)`, `ShowMenu(m)`, `Resume` (a VM-nyelven lefordítva: `JumpTT`, `JumpVTS_PTT`, `LinkPGCN`, `CallSS`).
- Fejezetmenü automatikusan a sorozat fejezetjelzőiből (a #1841 már fejezetekké alakítja).

### 3.2 Menü-VOB
Egy VOBU: NAV + videó (I-kép) + csendes hang (ha a PGC-ben van hangsáv) + SPU. A menü a `VIDEO_TS.VOB` (VMGM) és a `VTS_01_0.VOB` (VTSM) fájlban, `VMGM_VOBU_ADMAP`, `VMGM_C_ADT`, `VTSM_VOBU_ADMAP` címtáblákkal.

### 3.3 Gombok, kiemelés
- PCI HLI: `hli_gi` (kezdő/záró PTM, gombszám, `btn_ofn`), 3 szín-/kontrasztkészlet (kijelölt, aktivált), gombok: rect, szomszédok (up/down/left/right), `auto_action`, gomb-parancs (8 bájt VM).
- A gombkép 4 színű SPU-bitmap (háttér, szöveg, kijelölés, aktiválás) RLE-kódolással; a kiemelés a paletta-átképezés (CLUT), nem újrarajzolás — így a szabvány szerint működik.
- A gombfelirat/szöveg renderelése: a meglévő `cfa4-text` és címrenderelő (nem új betűkészlet-függőség), egyetlen kép, majd 4 színre kvantálva.

### 3.4 VM-parancsok, PGC
- 8 bájtos parancsok (`Nop`, `Goto`, `Break`, `SetSTN`, `Link`, `Jump`, `Call`, `Resume`, `SetHL_BTNN`). Csak a fent felsorolt szűk készlet, minden bájtmintára saját teszt.
- PGC-parancstábla: pre-command (`SetHL_BTNN` az első gomb, `Link` ismétlés), post-command (`LinkPGCN` önmagára, hogy a menü ne érjen véget), cell-command.
- A `VIDEO_TS.IFO` kapja: `FP_PGC` (első lejátszás: `JumpSS VMGM menu`), `TT_SRPT`, `VMGM_PGCI_UT` (nyelvi egység, menü-PGC), `PTL_MAIT` (üres), `VMGM_VOBU_ADMAP`.
- A `VTS_01_0.IFO` kapja: `VTSM_PGCI_UT` (fejezetmenü), `VTS_PTT_SRPT`, `VTS_PGCIT`, címtáblák.

### 3.5 Bekötés (a #1841 beolvasztása után)
- Export-paraméter: `dvdMenu` (JSON, `DiscMenu`), és a CLI/Qt6 vezérlő (`python/cfa4_qt/panels/export.py`: „DVD menü” kapcsoló + gomblista, Qt-teszt, hu/es/ja szöveg a szabály szerint). Az egui nem változik.
- A kimenet továbbra is a `VIDEO_TS` mappa; menü nélkül a mai viselkedés **bájtra azonos** marad.

### 3.6 Bizonyíték
- `lsdvd -x` olvassa: a menü létezik, a címek és fejezetek száma helyes.
- Bájtszintű tesztek: PCI/HLI mezők, VM-parancsok, IFO-táblák (offset és méret), menü-VOB csomagrend (2048 bájt, pack/PES hossz).
- ffprobe a menü-VOB-ra (lejátszható MPEG-2 + SPU-folyam).
- Navigáció-ellenőrzés: nincs libdvdnav telepítve → saját, egyszerű VM-szimulátor a tesztben (a gomb parancsa a várt PGC-re/címre ugrik); ha István szerint kell valódi libdvdnav/lejátszó (pl. VLC), az **oracle-ként**, nem kódként kerül szóba (előbb megkérdezem, külön jóváhagyás).
- Nem állítom, hogy valódi lejátszón nézve működik, amíg nincs mérés; ezt a PR leírása kimondja.

## 4. Kockázatok és nyitott pontok

- Valódi DVD-lejátszóval nem tesztelhetünk; a szabvány pontjai miatt a VM-szimulátor és az `lsdvd`/`dvdauthor` összevetés a legjobb elérhető ellenőrzés.
- A gombkiemelés színkészlete és az SPU-kvantálás látványa csak képernyőképpel ítélhető meg: képet csatolok a PR-hoz (egy menü-PNG a kimenetből).
- A `Muxer::new_dvd` és #1841/#1857 fájljai (`mpeg_out.rs`, `job.rs`) ütközhetnek: a D1 kódmagja (`crates/dvd`) önálló, a bekötés külön commit/PR.
- A pulldown (D4) a T-STD (#1753/#1857) után jobb, mert a VOBU-időtartam és a szabványos puffermodell egymásra hat.

## 4a. István döntése (2026-11-27): a függőségeknek teljesülniük kell

Egy fázis csak akkor indul, ha a függőségei **be vannak olvasztva a `main`-be**: D1 a #1841-re (DVD export-bekötés), D4 a #1753/#1857-re (T-STD) és #277-re vár. Menümag előre, bekötés nélkül **nem** készül. Amíg a #1841 nincs beolvasztva, a munka áll; a beolvasztás a gazda dolga (Gazda #1842), én nem merge-elek.

## 5. Döntést kér István (az 2. pontra a 4a. válaszol)

1. **Sorrend és felbontás:** egyetért a D1 → D2 → D3 → D4 négy PR-ral (a D1: menü; a D2 a feliratok)? *(ajánlott: igen)*
2. **A D1 bekötése:** várjam meg a #1841 beolvasztását (ajánlott), vagy a `crates/dvd`-menümagot (bekötés nélkül, CLI-tesztekkel) addig is elkészítsem, és a bekötést külön PR-ba tegyem? *(ajánlott: a magot most, a bekötést a #1841 után)*
3. **Menühatter:** csak állókép (D1), a mozgó háttér és a hangos menü később külön issue-ban? *(ajánlott: igen)*
4. **Lejátszó-oracle:** kérhetem később, hogy egy valódi lejátszóval (VLC/libdvdnav) nézd meg a kimenetet? (Kód nem kerül át; csak ellenőrzés.)

## 6. Nem tartozik ide
Blu-ray (#245, utána külön terv), mozgó menühatter, DVD-Audio, másolásvédelem/CSS, ISO-lemezkép-írás (külön issue, ha kell).
