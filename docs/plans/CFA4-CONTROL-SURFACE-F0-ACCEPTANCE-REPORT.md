# CFA4-CONTROL-SURFACE-001 – F0 átvételi jegyzőkönyv

**Dátum:** 2026-11-13 · **Alap:** `ubuntuokos/CinemaFundArchitecture4` main `92e29e4` · **Állapot:** elfogadva (a tulajdonos, 2026-11-13: „Haladj tovább logikusan!”, a nyitott döntések az ajánlás szerint). Az F0 a spec 17. fejezete szerint csak akkor zárul, ha minden sorhoz ellenőrizhető bizonyíték van
és a tulajdonos elfogadta.

Jelmagyarázat: **✅ kész** (bizonyíték megvan, jóváhagyásra vár) · **🟡 részben** (a hiányzó rész és a pótlás ideje megnevezve) · **⬜ nyitott**

## 1. Elfogadási feltételek

| ID | Feltétel | Bizonyíték | Állapot | Megjegyzés |
|---|---|---|---|---|
| F0-AC-01 | Architektúra és réteghatárok rögzítve | [ADR 0012](../adr/ADR-0012-control-surface.md) Döntés 1–2.; műszaki spec 1. fejezet | ✅ | Repóban: `docs/adr/ADR-0012-control-surface.md` |
| F0-AC-02 | Az L5 crate felelőssége dokumentált | Műszaki spec 1., 3., 6., 9. fejezet | ✅ | |
| F0-AC-03 | HID-meghajtó kiválasztása dokumentált | [`CFA4-CONTROL-SURFACE-HID-EVALUATION.md`](CFA4-CONTROL-SURFACE-HID-EVALUATION.md) | 🟡 | Asztali értékelés kész (elsődleges hidraw, tartalék evdev). A fizikai mérés (M1–M10) a referenciagépen és eszközzel az F1a első lépése; az F1a-kapu a mérési jegyzőkönyv. |
| F0-AC-04 | ShuttlePRO eseménymodell meghatározott | Műszaki spec 4. fejezet | ✅ | A hidraw riport bájtkiosztása mérendő (N-03) |
| F0-AC-05 | Közös SurfaceEvent és Feedback szerződés | Műszaki spec 2. fejezet | ✅ | |
| F0-AC-06 | MIDI/ALSA függőségek ellenőrzöttek | [`CFA4-CONTROL-SURFACE-DEPENDENCY-AND-LICENSE-REVIEW.md`](CFA4-CONTROL-SURFACE-DEPENDENCY-AND-LICENSE-REVIEW.md) 1–3. fejezet; `cargo tree -i alsa` | ✅ | Új crate nem kell; az `alsa-sys` `links = "alsa"` miatt egy verzió lehet |
| F0-AC-07 | Licencellenőrzés dokumentált | Ugyanott, 3–4. fejezet; `deny.toml` engedélylista | ✅ | Kdenlive-port a `licensing.md` §5 szerint |
| F0-AC-08 | Profilformátum alapjai rögzítettek | [`CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json`](CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json) (JSON Schema 2020-12) | ✅ | A séma és a példa `jsonschema`-val ellenőrizve (2026-11-13): a példa érvényes, a hibás minta (CC 200, 10× sebesség) elutasítva |
| F0-AC-09 | Qt6/serve protokollbővítés dokumentált | [`CFA4-CONTROL-SURFACE-PROTOCOL-CONTRACT.md`](CFA4-CONTROL-SURFACE-PROTOCOL-CONTRACT.md) | ✅ | A Qt-befejezés után a végleges protokollhoz igazítandó (WP5) |
| F0-AC-10 | MIDI Learn állapotgép meghatározott | Műszaki spec 7. fejezet; protokoll `surface.learn*`, `surfaceLearn` | ✅ | |
| F0-AC-11 | Hibakezelési és izolációs szabályok | Műszaki spec 9. fejezet | ✅ | |
| F0-AC-12 | F1a/F1/F2 belépési feltételek egyértelműek | Műszaki spec 12. fejezet | ✅ | |
| F0-AC-13 | Nyitott kérdések külön listában | Műszaki spec 13. fejezet (N-01…N-12) | ✅ | |
| F0-AC-14 | Dokumentációs változások ellenőrzöttek | Műszaki spec 14. fejezet (jegyzék) | 🟡 | A jegyzék kész. Az ADR, a hat F0-dokumentum és a `hardware-parity.md` sora egy PR-ban megy be (tulajdonosi döntés 2026-11-13); a többi sor (architektúra, protokoll, Qt GUI, licenc) a 14. fejezet szerinti fázisban. A Kdenlive-terv jog-shuttle sora a Kdenlive-projekt döntése (§2.8). |

## 2. Teszttervi ellenőrzések (spec 15. fejezet)

| Teszt | Eredmény |
|---|---|
| T0.1 Architektúra | L4/L5/L6 határ, OS-I/O csak L5, Qt-ban nincs leképezés, OSC külön modul, egy igazságforrás: ADR + spec 1. és 11. fejezet ✅ |
| T0.2 HID | Jelöltek, licenc, azonosítás, olvashatóság, hot-plug, jogosultság, elsődleges/tartalék: HID-értékelés ✅ asztali, mérés 🟡 |
| T0.3 ALSA/MIDI | Lockfile, verzióütközés (`links`), licenc, `/dev/snd/seq` jogosultság, PipeWire-út, hot-plug-út: függőségi jegyzőkönyv + spec 5. fejezet ✅; PipeWire-út a referenciagépen mérendő (`aconnect -l`) |
| T0.4 Protokoll | `surface.*`, események, hibakódok, `surface.clock`, `surface.inject`, MCP, additív bővítés: protokollszerződés ✅ |
| T0.5 Profil | JSON-séma, verziózás, ütközés (`surface.audit`, `binding_conflict`), XDG-út, atomikus írás, beépített/felhasználói/projekt réteg: séma + spec 8. fejezet ✅ |
| T0.6 Licenc és dokumentáció | `deny.toml`, `licensing.md`, `NOTICE`, architektúra, protokoll, Qt GUI, parity: jegyzék ✅, repóba írás 🟡 (engedélyre vár) |

## 3. A WP0 újraértékelés eredménye (2026-11-13)

A 2026-11-02-i terv döntései érvényesek. A Qt-kapuból nyitott: `qt-parity.md` 1.9, 1.10, 5.5; a
`docs/plans/CFA4-QT6-FINISH-001.md` (TERVEZET 1.1) jóváhagyásra vár. A `serve` hello-jában nincs `features`, a hibákban nincs `code`
(a protokollszerződés pótolja). Az ALSA-helyzet változatlan. A kernel a `reference-machine.md` szerint 7.0.0-43-generic.

Két új tény az újraértékelésből, amely a specifikációtól eltér:

1. **Hang shuttle közben:** a Qt ma 0,5–4× előre hallhatóan játszik (`SHUTTLE_SOUND`), a spec némát ír. Eldöntve 2026-11-13: a Qt meglévő szabálya marad, a profilban némítható (N-01).
2. **ALSA-verzió:** az `alsa-sys` `links = "alsa"` kulcsa miatt két verzió nem lehet a munkaterületen; a `cfa4-surface` a `cpal`
   ALSA-verzióját követi az egui eltávolításáig.

## 4. Átadás (spec 21. fejezet)

| Átadandó | Fájl |
|---|---|
| Architekturális döntések | [ADR 0012](../adr/ADR-0012-control-surface.md) |
| Műszaki specifikáció, adatmodell, Learn, hibakezelés, fázisbelépés, nyitott kérdések | [`CFA4-CONTROL-SURFACE-F0-TECHNICAL-SPEC.md`](CFA4-CONTROL-SURFACE-F0-TECHNICAL-SPEC.md) |
| HID-kiválasztás | [`CFA4-CONTROL-SURFACE-HID-EVALUATION.md`](CFA4-CONTROL-SURFACE-HID-EVALUATION.md) |
| Függőségi és licencellenőrzés | [`CFA4-CONTROL-SURFACE-DEPENDENCY-AND-LICENSE-REVIEW.md`](CFA4-CONTROL-SURFACE-DEPENDENCY-AND-LICENSE-REVIEW.md) |
| Protokoll | [`CFA4-CONTROL-SURFACE-PROTOCOL-CONTRACT.md`](CFA4-CONTROL-SURFACE-PROTOCOL-CONTRACT.md) |
| Profilformátum | [`CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json`](CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json) |
| Elfogadási jegyzőkönyv | ez a fájl |

## 5. Tulajdonosi döntések az F0 lezárásához

1. Az ADR és a hat dokumentum elfogadása.
2. ~~N-01: hang shuttle közben~~ eldöntve 2026-11-13: a Qt meglévő szabálya, profilban némítható.
3. ~~N-12~~ eldöntve 2026-11-13: az 1.10 (csomagolás) marad F1a-kapu.
4. ~~Repó~~ eldöntve 2026-11-13: igen, külön PR-ban.

| Szerep | Név | Dátum | Döntés |
|---|---|---|---|
| Tulajdonos | | | |
