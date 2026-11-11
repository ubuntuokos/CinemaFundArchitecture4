# Külső időkód-szinkron: LTC és MTC (chase és generate)

**Állapot:** tervezet, tulajdonosi jóváhagyásra vár · **Dátum:** 2026-11-11 · **Tulajdonos:** ubuntuokos

## 1. Cél

A CFA4 lejátszása külső eszközökkel együtt fusson SMPTE-időkód alapján, két irányban:

- **Generate (küldés):** lejátszás közben a CFA4 a programidővonal időkódját kiadja
  **LTC**-ként (hangjel egy kiválasztott kimeneti csatornán) és/vagy **MTC**-ként (MIDI).
  Erre szinkronizálhat egy DAW, fénypult, másik lejátszó vagy felvevő.
- **Chase (követés):** a CFA4 egy beérkező **LTC** (hangbemenet) vagy **MTC** (MIDI-bemenet)
  időkódot követ: odaugrik, elindul, megáll, és együtt fut a külső mesterrel.

Ez a Hardver-paritás „LTC timecode input” sorát (0%) és a hiányzó MTC-t fedi le
([`docs/hardware-parity.md`](../hardware-parity.md), Devices and I/O). A DV-fájlok saját
(beágyazott) időkódja **nem** része ennek a tervnek; az a DV-terv része marad.

Referenciák (a tulajdonos küldte): Ableton SMPTE FAQ, Arcane Wizards SMPTE-leírás, ETC SMPTE
QuickGuide, Phil Rees: „Timecode” cikk. Az Ableton FAQ szerint a Live **nem** követ és **nem** generál
SMPTE-t natívan, csak előre renderelt LTC-wav-val vagy Max for Live eszközzel. A CFA4 ennél többet
ad: natív generate és chase. Az Arcane Wizards SMPTE-összefoglalója a vevőoldali buktatókat írja
le: az időkód folyamatos jel, nincs „megállt” üzenet (a megállást időtúllépésből kell kikövetkeztetni),
a sebesség nincs átküldve (átlagolt mérésből becsülhető), kiesésnél „lendkerék” (freewheel) kell,
és a képkockasebességet mindkét oldalon kézzel kell egyeztetni.

## 2. Ami már megvan

| Terület | Hol | Mit ad |
|---|---|---|
| Pontos idő | `crates/time` (`cfa4-time`, L0) | `Tick`, `FrameRate` (NTSC is), drop/non-drop SMPTE `frames_to_fields` / `fields_to_frames`, formázás és értelmezés. Az LTC/MTC mezők közvetlenül erre képezhetők. |
| Klip kezdő időkódja | `MediaInfo::start_timecode`, `engine/src/clip_ops.rs`, `sync.rs` | Fájlidőkód, multicam-szinkron időkód alapján. |
| Hangkimenet, lejátszási mesteróra | `apps/cfa4/src/audio.rs` (cpal, egui-ág); Qt: `python/cfa4_qt/panels/monitor.py` (`QAudioSink`) | A lejátszás órája a hangkimenet. |
| Hangbemenet | `apps/cfa4/src/audio_in.rs` (cpal, voice-over) | Eszköz- és hostválasztás mintája, saját szálon. |
| MIDI | — | **Nincs** MIDI-réteg. Az `alsa` 0.9.1 crate (MIT/Apache-2.0) már a lockfile-ban van a cpal miatt; ennek `alsa::seq` modulja ALSA szekvenszer-hozzáférést ad, új licenc nélkül. |
| LTC/MTC kód | — | **Nincs** LTC-kódoló/dekódoló és MTC-üzenetkezelés. |

A Qt6-felület a Rust motort a `cfa4-cli serve` gyermekfolyamatként használja (`python/cfa4_qt/engine.py`,
`docs/qt-gui.md`), a hang a Qt `QAudioSink`-en szól. Az egui befagyasztott (VISION 2.1): **új
funkció csak a Qt6-felületre kerül**, az eszközkezelés ezért a `serve` folyamatban fut.

## 3. Felépítés

```
L1  crates/tcsync  (cfa4-tcsync)   tiszta Rust, I/O nélkül, csak cfa4-time-tól függ
      ltc:   80 bites LTC-keret <-> mezők; biphase-mark kódoló (f32 minták) és dekódoló
      mtc:   quarter-frame és full-frame SysEx kódolás/értelmezés
      chase: lendkerék + zárolási állapotgép, eltolás, 24 órás átfordulás
L4  crates/engine                  beállítások (serde), parancsok, SyncSource/SyncSink trait
L5  crates/tcsync-io (cfa4-tcsync-io) cpal (LTC be/ki), alsa::seq (MTC be/ki); saját szálak
L6  apps/cfa4-cli serve            összeköti; események a Qt felé
Qt  python/cfa4_qt                 beállítópanel, állapotjelző (LOCK / FREEWHEEL / NO SIGNAL)
```

A rétegszabály teljesül: a tiszta logika L1-ben (az L0 kodekcrate-ekre vonatkozó „csak
`cfa4-bitstream`” szabály itt nem érintett), eszközök (cpal, alsa) csak L5-től (`cargo xtask layers`).

### 3.1 LTC (`cfa4-tcsync::ltc`)

- 80 bites keret: BCD óra/perc/mp/képkocka, drop-frame és color-frame jelző, 8×4 bit user bits,
  BGF-jelzők, polaritásjavító bit, 16 bites szinkronszó (`0011 1111 1111 1101`). A 25 fps-es
  bitkiosztás eltérését (BGF/polaritás helye) külön kezeljük.
- **Kódoló:** biphase-mark (0: egy átmenet a cella elején, 1: plusz egy a cella közepén;
  30 fps-nél 2400 bit/s), beállítható szint (alapból −18 dBFS; a fénypultok bemenete széles
  tartományt fogad, ETC: 0…+14 dBu) és felfutási idő (alapból
  ~25 µs, a szabványos 25±5 µs tartományban), bármely mintavételezési frekvencián (44,1–192 kHz).
- **Dekódoló:** nullátmenet-alapú, adaptív bitidővel; előre és hátra is (a szinkronszó iránya
  adja); sebességbecslés kb. 0,1×–10× között; zajtűrés, polaritásfüggetlen. Kimenet:
  `(mezők, user bits, keret kezdőmintája, irány, becsült sebesség)`.
- Olvasáskor a 27., 43., 58. és 59. bitet maszkoljuk, íráskor nullázzuk (Phil Rees). A
  drop-frame (10.) és color-frame (11.) jelzőt egyes generátorok nem töltik ki (ETC), ezért a ráta
  automatikus felismerése a mért bitsebességre és a képkockaszám-sorozatra épül, nem csak a jelzőre.
- Minden bemenet ellenséges (AGENTS.md §0.4): korlátos pufferek, `get()`, nincs `unwrap`.

### 3.2 MTC (`cfa4-tcsync::mtc`)

- **Quarter-frame** (`F1 0nnn dddd`): 8 üzenet két képkockán; a sebességkód a 7. üzenetben
  (24 / 25 / 29,97 DF / 30). Összerakás csak teljes, sorrendhelyes 8-asból; az irányt az
  üzenetsorrend adja. Az MTC a 8 üzenet végén +2 képkockás késést hordoz, ezt kompenzáljuk.
- **Full-frame SysEx** (`F0 7F 7F 01 01 hh mm ss ff F7`): ugrás (locate) jelzése, megállt
  állapotban is.
- 23,976 és 59,94 nincs az MTC-ben: ezeknél a CFA4 a legközelebbi MTC-kódot küldi és
  figyelmeztet (ugyanígy kezeli a többi szoftver is).

### 3.3 Chase (`cfa4-tcsync::chase`)

Tiszta, determinisztikus állapotgép, szimulált órával tesztelve:

`NoSignal → Locating → Locked ↔ Freewheel → Stopped`

- **Eltolás:** `szekvenciaidő = külső időkód + eltolás` (alapból a szekvencia kezdő időkódja
  szerint 0), 24 órás átfordulás kezelése.
- **Locate:** a szükséges előtekerés (pre-roll) után ugrik és indít, ha N egymást követő
  érvényes keret (alapból 3) egybehangzó.
- **Lock és drift:** v1-ben **újraugrás** (re-seek), ha az eltérés > 1 képkocka egy beállítható
  ideig; v2-ben opcionális **varispeed** (finom lejátszási sebességkorrekció).
- **Freewheel:** jelkiesésnél beállítható ideig (alapból 10 képkocka) tovább fut a becsült
  sebességgel, utána Stopped. Hátrafelé futó vagy nem 1× sebességű jelnél: követés locate-tel,
  hang nélkül (scrub), mint a legtöbb NLE-ben.
- **Vevőszabályok a fénypultok mintájára (ETC):** ha két képkockán belül nem jön új keret, vagy
  ugyanaz a keret ismétlődik, lendkerékre vált; az utolsó érvényes időhöz képest ±2 mp-en belüli
  keretet elfogad (hátrafelé is), azon kívül újra-locate.
- **Késleltetés:** a külső eszközök tipikusan 3–5 képkocka késéssel reagálnak (ETC); a generate
  oldalon beállítható előretolás (ms vagy képkocka) kompenzálja.
- **Képkockasebesség-eltérés** (pl. 25 fps-es LTC 24 fps-es szekvencián): figyelmeztetés az
  állapotsávban, a követés időalapon (Tick) helyes marad.

### 3.4 Motor, parancsok, beállítások (`crates/engine`)

„Minden parancs” szabály szerint, a vezérlőcsatornán és MCP-n is:

- `timecode_sync.chase` (be/ki), `timecode_sync.source` (`ltc` / `mtc`),
  `timecode_sync.generate_ltc`, `timecode_sync.generate_mtc` (be/ki),
  `timecode_sync.offset` (időkód), `timecode_sync.rate` (auto / kézi), eszközválasztás
  (hangbemenet és csatorna, hangkimenet és csatorna, MIDI-port), késleltetés-kompenzáció (ms).
- Állapotlekérdezés: `timecode_sync.status` (állapot, utolsó időkód, becsült sebesség, eltérés).
- Beállítások serde-vel, a meglévő `settings.rs` mintájára; ellenséges-paraméter tesztek.

### 3.5 Eszköz I/O (`crates/tcsync-io`, L5)

- **LTC be:** cpal bemeneti stream, a választott csatorna kiemelése, a dekódoló saját szálon,
  `catch_unwind` alatt; hiba → állapotüzenet, soha nem összeomlás.
- **LTC ki:** külön cpal kimeneti stream a választott eszközön/csatornán. Az időt a lejátszás
  órája adja (Qt: a `QAudioSink` feldolgozott ideje, amit a GUI a motornak jelent) lendkerékkel,
  és a kimeneti késleltetéssel előretolva. Cél: ±1 képkocka a lejátszott képhez.
- **MTC be/ki:** `alsa::seq` (már függőség, nincs új licenc): saját CFA4-port a szekvenszeren,
  így bármely hardveres vagy szoftveres MIDI-eszköz (PipeWire-en át is) bekötheti.
- PipeWire alatt a cpal az ALSA-rétegen át éri el az eszközöket (mint ma).

### 3.6 Qt6-felület

Új „Timecode Sync” beállítóoldal (Settings ▸ Audio Hardware mellé) és állapotjelző a Program
monitoron (LOCK / FREEWHEEL / NO SIGNAL, beérkező időkód). Minden vezérlő automation id-vel
(ügynökkel vezérelhető), a `docs/qt-parity.md` szabályai szerint.

## 4. Licenc és clean-room

- Saját, clean-room Rust-megvalósítás nyilvános leírásokból: az LTC bitkiosztása és a
  biphase-mark kódolás széles körben dokumentált (pl. Wikipedia „Linear timecode”); az MTC a
  MIDI 1.0 specifikáció része (a MIDI Association ingyenesen letölthetővé teszi). A fizetős
  SMPTE ST 12-1 megvásárlása **nem szükséges** a v1-hez.
- Kódot nem veszünk át. A `libltc` (LGPL-3.0-or-later) kompatibilis lenne, de C-könyvtár, és a
  feladat kicsi, ezért nem kell. Ugyanez áll elődjére, a `libltcsmpte`-re (ltcsmpte.sourceforge.net, a tulajdonos küldte);
  ha valaha kód kerülne át belőlük, előbb a fájlfejlécek licencét ellenőrizzük (AGENTS.md, docs/licensing.md §5).
  A böngészős LTC-generátor (elteesee.pehrhovey.net) kézi tesztjelforrásnak jelölt. Az Arcane Wizards TypeScript csomagjai csak referenciák.
- **Tesztorákulum:** az Ubuntu `ltc-tools` csomag (`ltcgen`, `ltcdump`, libltc alapú) külső
  programként, csak tesztben, mint az ffmpeg CLI: LTC-wav generálása a dekódolónk ellen, és a
  mi kimenetünk visszaolvasása. Termékbe nem kerül. Hiánya esetén a teszt kihagyja magát.
- Új crate-függőség nincs (cpal és alsa már bent van); `cargo deny check licenses` változatlan.

## 5. Fázisok

| # | Tartalom | Bizonyíték |
|---|---|---|
| TC-0 | Ez a terv a `main`-ben | — |
| TC-1 | `cfa4-tcsync::ltc` kódoló + dekódoló | körbejárási tesztek minden rátán (24, 25, 29,97 DF/NDF, 30), zaj, sebesség 0,5×–2×, hátrafelé, mutációs fuzz; `ltc-tools` orákulum |
| TC-2 | `cfa4-tcsync::mtc` | QF/SysEx körbejárás, sorrendhiba, csonkolt üzenetek, fuzz |
| TC-3 | `cfa4-tcsync::chase` | szimulált órás tesztek: locate, lock, drift → re-seek, kiesés → freewheel → stop, 24 órás átfordulás, rátaeltérés |
| TC-4 | Motor: beállítások, parancsok, státusz | parancstesztek, ellenséges paraméterek |
| TC-5 | `cfa4-tcsync-io` + `serve` bekötés | ALSA loopback (`snd-aloop`) LTC-re, virtuális MIDI-port MTC-re: CFA4 → CFA4 körbejárás CI-n kívül, majd a referenciagépen |
| TC-6 | Qt6 beállítóoldal és állapotjelző | Qt-tesztek, képernyőkép |
| TC-7 | Offline: LTC-sáv dekódolása klip hangjából → kezdő időkód / multicam-szinkron | teszt generált LTC-wav fixture-rel |
| TC-8 (később) | Varispeed chase, MTC/LTC user bits megjelenítése | — |

Egy fázis = egy PR, friss `main`-ből ([`docs/coordination.md`](../coordination.md)).

## 6. Kockázatok

- **Valódi hardver:** a referenciagépen nincs külön hangkártya vagy MIDI-eszköz leírva; a
  végső ellenőrzés a tulajdonos eszközein (vagy egy USB-s hangkártya + kábel hurok) történik.
  Addig az `snd-aloop` és virtuális MIDI ad bizonyítékot.
- **Qt hangóra pontossága:** a `QAudioSink` időjelentése durvább lehet a cpal-nál; ha ±1
  képkocka nem tartható, a LTC-kimenet az engine által renderelt hangfolyamba kerül (TC-5-ben mérjük).
- **88 nyitott PR:** a 30-as határ fölött vagyunk; ezt a munkát a tulajdonos kérte (2026-11-11).

## 7. Nyitott tulajdonosi döntések

- **D1 – Chase v1 módja:** újraugrás (re-seek) driftnél, varispeed később. *Javaslat: igen.*
- **D2 – LTC-kimenet helye:** külön választható kimeneti eszköz/csatorna (javasolt), vagy a
  program hangjába keverve egy plusz csatornán.
- **D3 – Offline LTC-dekód (TC-7)** a kamerák hangsávján rögzített LTC-ből: benne legyen-e ebben
  a projektben. *Javaslat: igen, kevés munka, és a multicam-szinkronnak segít.*
