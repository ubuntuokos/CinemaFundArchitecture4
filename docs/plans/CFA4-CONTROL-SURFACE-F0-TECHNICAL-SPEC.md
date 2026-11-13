# CFA4-CONTROL-SURFACE-001 – F0 műszaki specifikáció

**Dátum:** 2026-11-13 · **Állapot:** elfogadva (a tulajdonos, 2026-11-13) · **ADR:** [ADR 0012](../adr/ADR-0012-control-surface.md) ·
**Alap:** `ubuntuokos/CinemaFundArchitecture4` main `92e29e4`; referenciagép a repó
`docs/development/reference-machine.md` fájlja szerint (Ubuntu Studio 26.04, kernel 7.0.0-43-generic, KDE Plasma 6.6.6,
Wayland, Qt 6.10.2) · **Lefedett elfogadási pontok:** F0-AC-02, 04, 05, 10, 11, 12, 13

Ez a dokumentum a fejlesztőknek átadható szerződés. A „kötelező” szó az F1a/F1/F2 megvalósításra
vonatkozik; ami a WP3 mérésétől függ, azt **[mérendő]** jelöli.

## 1. Rétegek és felelősség

| Réteg | Hely | Felel | Nem felel |
|---|---|---|---|
| L1 | `crates/surface-model` (`cfa4-surface-model`) | Adattípusok (2. fejezet), MIDI-bájtfolyam → `SurfaceInput` parser, profilséma-típusok, `serde` | I/O, idő lekérdezése, parancsok |
| L4 | `crates/engine/src/surface/` | Profilok, leképezés, bank, skálázás, pickup, Learn, ütközés, szerkesztési munkamenet, undo/redo, audit, feedback-előállítás, `surface.*` parancsok | OS-I/O, szálkezelés |
| L5 | `crates/surface` (`cfa4-surface`) | Felderítés, megnyitás/zárás, hot-plug, nyers olvasás, időbélyeg, normalizálás, feedback-kódolás és -küldés, eszközállapot, eszközönkénti izoláció | Parancsvégrehajtás, profilszerkesztés, Learn-cél választása, Qt |
| L6 | `apps/cfa4-cli/src/serve.rs` + új `serve_surface.rs` | L5-sorok ürítése, L4 hívása, események kiírása, `surface.*` kiszolgálása, pánikvédelem | Eszköz-I/O a főciklusban |
| Qt6 | `python/cfa4_qt/` | Preferences ▸ Control Surfaces, profilszerkesztő, Learn-UI, eszközlista, MIDI Monitor, `surfaceAction` végrehajtása, `surface.clock` küldése | Leképezési logika, eszköz-I/O |

Rétegellenőrzés (`xtask`): `LAYERS += ("surface-model", 1), ("surface", 5)`. Új tiltólista L5 alatt:
`alsa`, `alsa-sys`, `evdev`, `udev`, `inotify`, `libudev-sys`. Az `engine` függhet a `cfa4-surface-model`-től,
a `cfa4-surface` is; a `cfa4-surface` nem függ az `engine`-től.

## 2. Közös adatmodell (F0-AC-05)

```rust
// cfa4-surface-model (L1)
pub struct DeviceId(pub String);           // stabil: "hid:0b33:0030:<sysfs-út hash>" | "midi:<kliensnév>:<portnév>"
pub struct MonoNs(pub u64);                // CLOCK_MONOTONIC ns, az L5 adja olvasáskor

pub enum SurfaceProtocol { Hid, Midi, Osc }

pub struct SurfaceEvent {
    pub device: DeviceId,
    pub at: MonoNs,
    pub protocol: SurfaceProtocol,
    pub input: SurfaceInput,
    pub seq: u64,                          // eszközönként szigorúan növő; hézag = vesztés (mérhető)
    pub raw: RawBytes,                     // a nyers üzenet/riport (Monitor, hibakeresés), legfeljebb 64 bájt inline
}

pub enum SurfaceInput {
    Button   { control: ControlId, pressed: bool },          // lenyomás és felengedés külön esemény
    Absolute { control: ControlId, value: u16, bits: u8 },   // 7 vagy 14 bit, a nyers érték megmarad
    Relative { control: ControlId, delta: i32, encoding: RelEncoding },
    Jog      { control: ControlId, delta: i32 },             // lépés (ShuttlePRO: tárcsa-kattanás)
    Shuttle  { control: ControlId, position: i8, range: i8 },// -range..=range, 0 = középállás
    Touch    { control: ControlId, touched: bool },
    ProgramChange { channel: u8, program: u8 },
    PitchBend     { channel: u8, value: u16 },               // 14 bit, 8192 = közép
    SysEx    { len: u32 },                                   // a tartalom a `raw`-ban; csak triggerként képezhető le
    Raw,                                                      // ismeretlen: csak Monitor és Learn látja
}

pub enum ControlId {
    Midi { channel: u8, kind: MidiKind, number: u16 },      // kind: Cc, Note, Rpn, Nrpn, Cc14 (MSB szám)
    Hid  { usage_page: u16, usage: u16, index: u16 },        // a riportleíró szerinti eredeti kód
    Evdev { ev_type: u16, code: u16 },                       // tartalék út: az eredeti evdev-kód
    Osc  { path: String },
}

pub enum RelEncoding { TwosComplement, SignMagnitude, Offset64, Binary } // = encoder-2 / -left / -right / -binary

pub struct SurfaceFeedback {
    pub device: DeviceId,
    pub target: FeedbackTarget,   // Led{control}, Fader{control}, Ring{control}, Display{index}, Timecode, Meter{index}, DeviceState
    pub value: FeedbackValue,     // Bool, U14, Text(String), Level(f32 dBFS), Tick(i64)
    pub priority: FeedbackPriority, // Immediate | Coalesce
    pub at: MonoNs,
}
```

Szabályok: a normalizálás nem dob el csatornát, vezérlőszámot, üzenettípust, nyers értéket,
felbontást, lenyomás/felengedés irányt, relatív előjelet vagy HID/evdev-kódot. A konkrét eszközprotokollra
alakítás (pl. MIDI Note On sebesség a LED-hez) kizárólag az L5-ben történik.

## 3. Sorok, sorrend, összevonás

```
L5 olvasószál (eszközönként) ──trigger──▶ sync_channel(256)  ─┐
                            └─folyamatos─▶ LastValue[ControlId] ┤──▶ serve főciklus ──▶ L4
                                                                └─ ébresztő (mpsc<()>)
```

1. Trigger = `Button`, `ProgramChange`, `SysEx`, `Jog`, `Shuttle`, `Touch`: nem vonható össze, nem dobható el.
   Teli sornál az olvasószál blokkol (az eszköz kernelpuffere vár), és a `stalls` számláló nő.
2. Folyamatos = `Absolute`, `Relative` (a delták összeadódnak), `PitchBend`: eszközönként vezérlőnként
   az utolsó érték (relatívnál az összeg) marad; a felülírások száma `coalesced` számlálóba kerül.
3. Eszközönként a sorrend megmarad (`seq`); eszközök között nincs sorrendi ígéret.
4. HID és MIDI külön olvasószálon, de közös L6-bemeneten fut (a 19. kérdés 8. pontjára adott válasz).
5. A kapacitás (256) kezdőérték; F1 előtt mérés alapján véglegesítendő **[mérendő]**.
6. Számlálók (`surface.state` adja vissza): `received`, `dispatched`, `coalesced`, `stalls`, `gaps`, `errors` eszközönként.
7. Hibás vagy hiányos üzenet `Raw`-ként a Monitorba megy, motorhibát nem okoz.

## 4. ShuttlePRO esemény- és vezérlési szerződés (F0-AC-04)

Eszközök: `0b33:0010` ShuttlePRO v1 (13 gomb), `0b33:0020` ShuttleXpress (5 gomb), `0b33:0030` ShuttlePRO v2 (15 gomb).

| Elem | hidraw (elsődleges) | evdev (tartalék) | `SurfaceInput` |
|---|---|---|---|
| Shuttle-gyűrű | riport 0. bájt, előjeles −7..+7, középállás 0 is jön **[mérendő]** | `EV_REL REL_WHEEL` (8), −7..+7; a 0 nem jön, a hiányából kell kikövetkeztetni | `Shuttle { position, range: 7 }` |
| Jog-tárcsa | riport 1. bájt, 0–255 körbeforduló számláló | `EV_REL REL_DIAL` (7), ugyanígy | `Jog { delta }` (a körbefordulás kezelve: delta = ((új − régi + 128) mod 256) − 128) |
| Gombok | riport 3–4. bájt bitmaszk **[mérendő]** | `EV_KEY` `BTN_*` | `Button { pressed }` mindkét élre |

Az evdev középállás-hibája (a kernel nem küld 0 értékű relatív eseményt, és 0-s jog-számlálónál a
kikövetkeztetés is hibázik) három független forrás szerint valós (Kdenlive, shuttle-go, Shuttle Lander);
ezért elsődleges a hidraw. A kernel 7.0.0-43-generic alatti viselkedést a WP3 méri.

**Alapleképezés (beépített „CFA4 ShuttlePRO” profil, Kdenlive-viselkedés szerint):**

- Shuttle-pozíció → `playback.shuttleSpeed {speed}`; 0 → `playback.stop` (megállás, a Kdenlive `monitor_pause` megfelelője). Az ellenkező irány negatív sebesség.
- Sebességtábla (profilban felülírható, ±7 elem, |érték| ≤ 8):

  | Pozíció | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
  |---|---|---|---|---|---|---|---|
  | „Ardour-szerű” (alapértelmezett) | 0,5× | 0,75× | 1× | 1,5× | 2× | 5× | 8× (Ardour: 10×) |
  | „Kdenlive-szerű” | 1× | 2× | 4× | 5× | 8× | 8× (16×) | 8× (60×) |

- Jog → `playhead.stepForward` / `playhead.stepBack` |delta|-szor (forrásmonitoron `src.stepFwd` / `src.stepBack`),
  1 kattanás = 1 képkocka (Kdenlive szerint); a lassú jog
  (shuttle-go: 200 ms küszöb) a profilban kapcsolható, alapból ki.
- Gombok → tetszőleges parancs-id a regisztrált listából (alap: J/K/L, In/Out, Add Edit, Undo stb.).
- **Trim mód:** ha a Trim Monitor aktív (`panels/trim_monitor.py`, `context: "trim"`), a jog a meglévő
  `trim.nudge {frames: delta}` parancsot hívja, a shuttle a meglévő `trim.shuttle {direction, slow?, clock}` és
  `trim.shuttleStop` parancsot (dinamikus trim). Új vágási vagy lejátszási állapot nem keletkezik.
- **Hang:** a Qt `play(speed)` útját használja, így a hang a meglévő `SHUTTLE_SOUND = (0.5, 4.0)` szabályt
  követi (előre 0,5–4× hallható, egyébként néma). Ez eltér a specifikáció „shuttle közben nincs hang” mondatától;
  a tulajdonos 2026-11-13-án a Qt meglévő szabályát választotta (N-01); a profil `shuttle.sound: "mute"` beállítása némít. Külön hangmotor nem készül.
- Egyetlen új UI-parancs kell: `playback.shuttleSpeed {speed}` a Qt `commands.py`-ban (a `MonitorPanel.play(speed)`
  hívása, |speed| ≤ `MAX_SPEED`, 0 = `playback.stop`). A többi parancs már létezik (`playback.forward`, `playback.reverse`,
  `playback.toggle`, `playhead.step*`, `trim.*`). A `surfaceAction` ezeket ugyanúgy hívja, mint egy billentyű.

## 5. MIDI és ALSA

- Kliens: `alsa::seq::Seq::open(None, Some(Direction::Duplex), true)`, név „CFA4”, egy bemeneti és egy kimeneti port
  eszközönként; a felderítés `ClientIter`/`PortIter`, a csatlakozás `PortSubscribe` az eszközportra.
- Hot-plug: feliratkozás a System Announce portra (0:1); `PortStart` → felderítés, megnyitás, aktív profil
  visszatöltése; `PortExit`/`ClientExit` → feedback leállítása, port lezárása, eszköz „offline” állapotba,
  a profil megmarad; újracsatlakozáskor a teljes feedback-állapot újraküldése.
- ALSA hiányában (`/dev/snd/seq` nincs, vagy a megnyitás hibázik): üres eszközlista, egy `surfaceError`
  (`code: "alsa_unavailable"`), a CFA4 normálisan indul; a HID működik.
- PipeWire: a WirePlumber ALSA-szekvenszer hídja a hardveres MIDI-t és a PipeWire-MIDI-t is szekvenszerportként
  mutatja; JACK nem kell. **[mérendő]** a referenciagépen: `aconnect -l` kimenete a WP3 jegyzőkönyvben.
- Jogosultság: a `/dev/snd/seq` az `audio` csoporté, a logind `uaccess` a helyi munkamenetnek adja; root nem kell.
- MIDI órajel (0xF8), Active Sensing (0xFE) és MTC (0xF1 quarter-frame, `F0 7F … 01 01` full-frame) a Learnből és a
  Monitor alapnézetéből kiszűrve; az MTC a `cfa4-tcsync-io` dolga.
- A `cfa4-tcsync-io` (L5, külső időkód-szinkron) már saját szekvenszer-klienst nyit (`alsa_seq.rs`). A `cfa4-surface`
  ugyanazt az `alsa` verziót használja, saját klienssel; ugyanarra a hardverportra mindkettő feliratkozhat. A tcsync-io
  ma nem kezel hot-plugot: ha a surface announce-kezelője elkészül, a közös rész kiemelése a két projekt közös döntése.

## 6. HID-hozzáférés és jogosultság

- Felderítés: `/sys/class/hidraw/hidraw*/device/uevent` (`HID_ID`, `HID_NAME`, `HID_PHYS`), hot-plug `inotify`-jal a `/dev`-en
  (a lockfile-ban már meglévő `rustix` 1.1.6 `fs` funkciójával); `libudev` nem kell. Olvasás `std::fs::File`-lal, `unsafe` nélkül
  (részletek: [`CFA4-CONTROL-SURFACE-HID-EVALUATION.md`](CFA4-CONTROL-SURFACE-HID-EVALUATION.md)).
- Jogosultság: csomagolt udev-szabály (`/usr/lib/udev/rules.d/70-cfa4-surfaces.rules`):
  `SUBSYSTEM=="hidraw", ATTRS{idVendor}=="0b33", TAG+="uaccess"` (és ugyanez `input` alrendszerre a tartalék úthoz).
  Nem `MODE="0644"`/`0666`: az bármely felhasználónak olvashatóvá tenné az eszközt.
- Hiányzó jogosultság: `surfaceError {code: "permission_denied", device}` és a Preferences-lapon útmutatás; a CFA4 fut tovább.

## 7. MIDI Learn állapotgép (F0-AC-10)

```
Idle ─learn(target)─▶ Learning ─üzenet─▶ Capturing ─1,5 s csend─▶ Captured ─▶ PendingMapping ─confirm─▶ Confirmed ─▶ Idle
                        │                    │                         │                └─reject──▶ Rejected ──▶ Idle
                        ├─7 s üzenet nélkül─▶ Timeout ─▶ Idle         └─ütközés─▶ Conflict ─(felülír / mégse)─▶ PendingMapping | Idle
                        └─learnCancel───────▶ Cancelled ─▶ Idle
```

- Indítás csak a profilszerkesztő Learn gombjáról (`surface.learn {target}`); a cél parancs-id vagy GUI-elem automation-id.
- Típusfelismerés a rögzített üzenetekből: azonos CC monoton változással → Absolute; 0x01/0x7F körüli értékek → Relative
  (kódolás a mintából); Note On/Off párok → Button; CC MSB+LSB (n, n+32) → 14 bit.
- 10 üzenet után folyamatjelző (`surfaceLearn {state: "capturing", count}`).
- Az ideiglenes leképezés a `PendingMapping` alatt él (a felhasználó kipróbálhatja), véglegesítés csak `confirm`-ra.
- 7 s és 1,5 s kezdőérték, a Preferencesben állítható (F1-ben ellenőrizendő).
- A Learn globális, nem projektszintű; egyszerre egy Learn futhat.

## 8. Profilok

Séma: [`CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json`](CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json). Rétegek és prioritás (a 19. fejezet 5. kérdésére adott javaslat):

1. **beépített** (csak olvasható, a binárisban), 2. **felhasználói** (`$XDG_DATA_HOME/cfa4/Control Surfaces/<név>.json`),
3. **projektprofil** (a projektfájlban, opcionális).

Egy eszközhöz egy aktív profil tartozik (`active-<device>.json`, a `DeviceId` fájlnévbiztos alakjával).
Ha a projekt tartalmaz az eszközre profilt, az a projekt nyitva tartásáig felülírja a felhasználói aktívat,
de csak azokat a vezérlőket, amelyeket maga is leképez (vezérlőnkénti felülírás, nem teljes csere).
A beépítettből szerkesztés másolatot készít (mint a `shortcuts.rs` `modified` jelzője).

Írás: `cfa4_format::atomic_write` (ideiglenes fájl → írás → `fsync` → átnevezés), előtte sémaellenőrzés.
Sérült fájl: figyelmeztetés az eseménynaplóba, a beépített profil töltődik. Magasabb `version`: csak olvasható.

## 9. Hibakezelés és izoláció (F0-AC-11)

1. Eszközönként saját olvasószál; pánik vagy I/O-hiba csak azt az eszközt állítja „error” állapotba.
2. Az L5 felügyelője újraindítja a szálat visszalépő időzítéssel (0,5 s, 1 s, 2 s … legfeljebb 30 s);
   öt egymást követő hiba után „disabled”, amíg a felhasználó újra nem engedélyezi (`surface.enable`).
3. A `serve` kéréskezelése már `catch_unwind` alatt fut; a surface-feldolgozás külön `catch_unwind` alatt,
   hiba esetén az esemény eldobva, `surfaceError` és naplóbejegyzés.
4. Hibás profil nem okoz leállást (8. fejezet); ismeretlen parancsra mutató kötés `surface.audit` figyelmeztetés.
5. ALSA nélkül, HID nélkül, OSC-modul nélkül a többi rész működik.
6. Leválasztáskor a port lezárva, a lenyomva maradt gombokhoz szintetikus felengedés megy az L4-be
   (különben „beragadt” momentary művelet maradna).
7. Feedback-visszhang: a feedbackből az eszköz által visszaküldött azonos érték 50 ms-on belül elnyelve;
   motoros fadernél érintés (`Touch`) alatt a motor nem küld pozíciót (feedback-harc ellen).
8. Túlterhelés és vesztés a 3. fejezet számlálóival mérhető.

## 10. Időzítés és teljesítmény

- `surface.clock {time: ticks, playing, speed, monitor: "program"|"source"|"trim"}`: a Qt küldi állapotváltozáskor és
  lejátszás közben legfeljebb 10 Hz-en; az L4 a monoton órával extrapolál (`time + (now − at) × speed`), 500 ms-nál
  régebbi órát érvénytelennek tekint (feedback-időkód leáll).
- Ideiglenes cél: motoroldali feldolgozás (L5 időbélyeg → `surfaceAction` kiírása) p95 ≤ 10 ms a referenciagépen;
  a GUI-akcióig mért határ F1 mérése után. Az Ardour-ritmusok (10/15/55/100/200 ms) feedback-frissítési alapértékek.

## 11. OSC bővíthetőség

Az OSC-modul (F-OSC) egy további L5-forrás, ugyanazt a `SurfaceEvent`/`SurfaceFeedback` modellt használja,
saját leképezőmotor nélkül; hiánya semmit nem tilt le. Alapértelmezett kötés `127.0.0.1`, LAN csak külön
engedéllyel (javaslat a 9. nyitott kérdésre; az F-OSC előtt dönt a tulajdonos).

## 12. Fázisbelépési feltételek (F0-AC-12)

| Fázis | Feltétel |
|---|---|
| F1a Rust-rész (L1, L5) | Tulajdonosi döntés 2026-11-13: a Qt-kapu előtt indulhat; a HID-mérés (M1–M10) eredménye a riportformátumot pontosítja |
| F1a bekötés (L4, L6, Qt) | Ez az ADR és a HID-döntés jóváhagyva; WP3 mérési jegyzőkönyv a referenciagépen; L5 interfész (2. fejezet) jóváhagyva; `qt-parity.md` 1.9, 1.10, 4.2, 5.2–5.5 ✅ |
| F1 | F1a elfogadva; ALSA-függőség (`alsa 0.9.1`) és protokollszerződés jóváhagyva; sorkapacitás mérve |
| F2 | Profilséma és leképezési szabályok jóváhagyva; F1 elfogadása fizikai eszközön igazolva |

## 13. Nyitott kérdések (F0-AC-13)

| # | Kérdés | Javaslat | Ki dönt, mikor |
|---|---|---|---|
| N-01 | Shuttle közbeni hang: a spec szerint néma, a Qt ma 0,5–4× előre hallható | A Qt meglévő szabálya, ki-be kapcsolható profilbeállítással | **Eldöntve 2026-11-13:** a Qt meglévő szabálya, profilban némítható |
| N-02 | Elsődleges HID-út | hidraw, tartalék evdev | WP3 mérése után, F1a előtt |
| N-03 | ShuttlePRO hidraw riportformátum | 5 bájt (shuttle, jog, –, gombok) | WP3 mérés |
| N-04 | Pontos udev-szabály és csomagolás (`.deb`, 1.10 pont) | `uaccess` szabály a csomagban | F1a előtt, a 1.10-zel együtt |
| N-05 | ALSA-crate verzió | a meglévő `alsa 0.9.1`, nincs második verzió | Jóváhagyás F1 előtt |
| N-06 | Profilrétegek prioritása | 8. fejezet (vezérlőnkénti felülírás) | F2 előtt |
| N-07 | `surface.clock` | tick, 10 Hz, 500 ms érvényesség | F1 előtt |
| N-08 | Sorstruktúra és kapacitás | 3. fejezet, 256 | F1 mérése |
| N-09 | OSC alapkötés | localhost, LAN külön engedéllyel | F-OSC előtt |
| N-10 | Első F2 beépített profilok | Generic MIDI, Behringer X-Touch Mini, Korg nanoKONTROL2 (saját adat, Ardour-viselkedés szerint) | F2 előtt |
| N-11 | A Kdenlive jogshuttle-kód portolása vagy csak viselkedés | Viselkedés + a sebességtábla és az evdev-logika portolása a licencfolyamattal | F1a előtt |
| N-12 | Az 1.10 (csomagolás) maradjon-e F1a-kapu | Igen, mert az udev-szabály a csomagba kerül | **Eldöntve 2026-11-13:** marad F1a-kapu |

## 14. Dokumentációs változások (F0-AC-14)

| Dokumentum | Változás | Mikor |
|---|---|---|
| `docs/adr/ADR-0012-control-surface.md` | az ADR | F0 lezárásakor (ez a PR) |
| `docs/architecture.md` §1 | `surface-model` L1, `surface` L5 sor, xtask-szabály | F1a első PR-jével |
| `docs/control-protocol.md`, `docs/qt-gui.md` | `surface.*`, események, `features`, hiba-`code` | F1 |
| `docs/hardware-parity.md` | ShuttlePRO, Generic MIDI sor, állapot: F0-ban specifikált | F0 lezárásakor (ez a PR) |
| `docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md` | jog-shuttle sor (a `kdenlive-parity.md` elavult) | a Kdenlive-terv projektje dönt (§2.8) |
| `docs/qt-parity.md` | Control Surface GUI sorok | F1 |
| `docs/licensing.md`, `NOTICE`, `ATTRIBUTION.md` | `evdev`/`udev`/`inotify` crate, Kdenlive-port | az első függőséget hozó PR-ral |
| `deny.toml` | csak ha új licenc kell (nem várható) | – |

A parity-sorok állapotai: F0-ban specifikált → fejlesztés alatt → implementált → tesztelt → fizikai eszközön igazolt.
