# CFA4-CONTROL-SURFACE-001 – Protokollszerződés (Qt6 ↔ `cfa4-cli serve`)

**Dátum:** 2026-11-13 · **Alap:** `apps/cfa4-cli/src/serve.rs` (`PROTOCOL = 1`, main `92e29e4`) · **Lefedi:** F0-AC-09, T0.4 ·
A szerződés a Qt6 GUI befejezésekor (qt-parity 1.9, 1.10, 5.5) a végleges Qt–engine protokollhoz igazítandó (WP5).

## 1. Kiindulás és kompatibilitás

A mai protokoll: egy kérés soronként `{"id", "method", "params"}`; válasz `{"id", "ok": true, "result"}` vagy
`{"id", "ok": false, "error": "szöveg"}`; esemény `{"event": "...", ...}` `id` nélkül; bináris farok `binary: N` esetén.
Az engine-parancsok az `engine.execute {command, params}` metóduson mennek (a CLI `exec`, a vezérlőcsatorna
és az MCP ugyanígy éri el őket).

Bővítési szabályok (mind additív, a `PROTOCOL` szám **nem** nő):

1. `hello.result.features`: új tömb. A surface-képes szerver `["surface"]`-t ad; ha nincs (régi szerver),
   a Qt a Control Surfaces lapot letiltja.
2. Hibaválasz: új, opcionális `code` mező a meglévő `error` szöveg mellett:
   `{"id": 7, "ok": false, "error": "unknown device", "code": "device_not_found"}`. Régi kliens a `code`-ot figyelmen kívül hagyja.
3. Minden `surface.*` **engine-parancs** (`cmd!`/`query!` a `crates/engine/src/surface/commands.rs`-ben), így a `command.list`
   része, és a CLI, a vezérlőcsatorna és az MCP külön munka nélkül eléri. A Qt `engine.execute`-tal hívja.
4. Az L5 állapotát az L6 tükrözi a `Session`-be (`Session::surface`); a parancsok ezt olvassák. Az L5-nek szóló
   kéréseket (engedélyezés, feedback) az engine egy kimenő sorba teszi, amelyet az L6 ürít (mint az `Event`-eket).
5. Új események az `Event` enumban (`Event::Surface…`), az `event_json` képezi le őket.

## 2. Hibakódok

| `code` | Mikor |
|---|---|
| `invalid_params` | Hiányzó vagy rossz típusú paraméter |
| `device_not_found` | Ismeretlen `device` |
| `device_offline` | Az eszköz nincs csatlakoztatva |
| `permission_denied` | Az L5 nem tudja megnyitni (udev/uaccess) |
| `alsa_unavailable` | Nincs `/dev/snd/seq` vagy nem nyitható |
| `profile_not_found`, `profile_invalid`, `profile_readonly` | Profilkezelés (beépített, sérült, magasabb verzió) |
| `binding_conflict` | `surface.bind` ütközés `keepConflicts` nélkül |
| `unknown_command` | A kötés célja nincs a regisztrált listában |
| `learn_busy`, `learn_idle` | Learn már fut / nem fut |
| `not_editing` | `surface.end`/`cancelEditing` munkamenet nélkül |
| `overload` | A feldolgozás torlódik (csak `surfaceError`-ban) |
| `internal` | Elkapott pánik vagy váratlan hiba |

## 3. Parancsok

A paraméterek a `CommandSpec::params` jelölésével (a `?` opcionális). `device` = `DeviceId` szöveg.
Fázis: **F1a** a ShuttlePRO-hoz kell, **F1** a Generic MIDI-hez és a Learnhez, **F2** a profilszerkesztő teljes készletéhez.

| Parancs | Paraméterek | Eredmény | Napló (journal) | Fázis |
|---|---|---|---|---|
| `surface.devices` | `{}` | `[{device, name, protocol, vid?, pid?, port?, state: "online"\|"offline"\|"error"\|"disabled", enabled, profile, error?}]` | nem | F1a |
| `surface.enable` | `{device, enabled: bool}` | `{device, enabled}` | nem (beállítás) | F1a |
| `surface.state` | `{device?}` | eszközönként számlálók (`received`, `dispatched`, `coalesced`, `stalls`, `gaps`, `errors`), bank, Learn-állapot | nem | F1a |
| `surface.profiles` | `{device?}` | `[{name, builtin, driver, version, readonly}]` | nem | F1a |
| `surface.loadProfile` | `{device, name}` | `{device, name}` | nem | F1a |
| `surface.bindings` | `{device}` | az aktív profil `controls` tömbje + `conflicts` | nem | F1a |
| `surface.bind` | `{device, control, mode, command, params?, context?, bank?, options?, keepConflicts?}` | a módosított kötések | profilszerkesztés (saját undo) | F1 |
| `surface.unbind` | `{device, control, context?, bank?}` | `{removed: n}` | ugyanaz | F1 |
| `surface.learn` | `{target: {command, params?} \| {automationId}, device?}` | `{state: "learning"}` | nem | F1 |
| `surface.learnCancel` | `{}` | `{state: "cancelled"}` | nem | F1 |
| `surface.learnConfirm` | `{accept: bool}` | `{state: "confirmed"\|"rejected"}` | igen, ha elfogadva | F1 |
| `surface.monitor` | `{enabled: bool, device?, filter?: {clock?: bool, sense?: bool}}` | `{enabled}` | nem | F1 |
| `surface.inject` | `{device, input: SurfaceInput, raw?}` | `{seq}` | nem | F1 |
| `surface.clock` | `{time: ticks, playing, speed, monitor: "program"\|"source"\|"trim"}` | `{}` | nem | F1 |
| `surface.audit` | `{device?}` | `[{device, control, problem: "unknown_command"\|"conflict"\|"unreachable_bank", detail}]` | nem | F1 |
| `surface.saveProfile` | `{device, name}` | `{name}` | nem | F2 |
| `surface.deleteProfile` | `{name}` | `{deleted}` | nem | F2 |
| `surface.export` | `{name, path}` | `{path}` | nem | F2 |
| `surface.import` | `{path, name?}` | `{name}` | nem | F2 |
| `surface.undo`, `surface.redo` | `{device}` | a kötések | profil-undo | F2 |
| `surface.begin`, `surface.end`, `surface.cancelEditing` | `{device}` | `{editing}` | – | F2 |
| `surface.bank` | `{device, bank: n} \| {device, step: ±n}` | `{bank, banks}` | nem | F2 |

Megjegyzések:

- A `surface.learnConfirm` új a specifikáció listájához képest: a `PendingMapping → Confirmed/Rejected` átmenethez kell.
- A `surface.inject` ugyanazon az L6 → L4 úton megy, mint egy valódi eszköz eseménye (ugyanaz a validáció,
  sor és leképezés); csak az L5 olvasót kerüli meg. Ismeretlen vagy offline eszközre is működik, ha a `device`
  létező profilhoz tartozik (teszt és diagnosztika).
- A profilszerkesztés undo/redo-ja a `shortcuts.rs` mintájára külön verem, nem a projekt-undo.

## 4. Események

| Esemény | Mezők | Gyakoriság |
|---|---|---|
| `surfaceAction` | `{command, params, device, control, seq}` | eseményenként; a folyamatos értékek összevonva |
| `surfaceLearn` | `{state, target, count?, captured?: {control, mode, encoding?}, conflict?}` | állapotváltozáskor; `capturing` alatt ≤ 5 Hz |
| `surfaceDevices` | `{devices: [...]}` (mint a `surface.devices`) | csatlakozás, leválasztás, állapotváltozás |
| `surfaceMonitor` | `{messages: [{device, at, raw (hex), decoded}], dropped}` | **≤ 20 Hz**, kötegelve; csak ha be van kapcsolva |
| `surfaceError` | `{device?, code, error}` | hibánként, ugyanaz a hiba legfeljebb 1/s |

A Qt a `surfaceAction`-t a meglévő parancsúton hajtja végre: engine-parancsnál `engine.execute`, UI-parancsnál
(`playback.*`, `playhead.*`, `trim.*` a Qt-ban) a `commands.py` diszpécserén. MIDI/HID bemenet nem hív közvetlenül widgetet.

## 5. Óra

`surface.clock`: `time` egész tick (`cfa4_time::Tick`, 254 016 000 000/s), `speed` lejátszási szorzó
(negatív = hátra), `monitor` a lejátszó monitor. A Qt állapotváltozáskor és lejátszás közben ≤ 10 Hz-en küldi;
az L4 a monoton órával extrapolál, 500 ms után érvénytelennek tekinti.

## 6. Példák

```text
→ {"id":1,"method":"hello"}
← {"id":1,"ok":true,"result":{"protocol":1,"engine":"CFA4 CLI 0.10.0","pid":4242,"autosave":true,"recovery":[],"features":["surface"]}}
→ {"id":2,"method":"engine.execute","params":{"command":"surface.devices","params":{}}}
← {"id":2,"ok":true,"result":[{"device":"hid:0b33:0030:usb-0000:00:14.0-3","name":"Contour ShuttlePRO v2","protocol":"hid","vid":2867,"pid":48,"state":"online","enabled":true,"profile":"CFA4 ShuttlePRO"}]}
← {"event":"surfaceAction","command":"playback.shuttleSpeed","params":{"speed":1.5},"device":"hid:0b33:0030:usb-0000:00:14.0-3","control":{"hid":{"usagePage":1,"usage":55,"index":0}},"seq":118}
→ {"id":3,"method":"engine.execute","params":{"command":"surface.bind","params":{"device":"midi:nanoKONTROL2:0","control":{"midi":{"channel":0,"kind":"cc","number":16}},"mode":"absolute","command":"mixer.noSuchCommand"}}}
← {"id":3,"ok":false,"error":"`mixer.noSuchCommand` is not a registered command","code":"unknown_command"}
```

## 7. MCP

Mivel minden `surface.*` regisztrált engine-parancs, a `cfa4-cli mcp` eszközlistájában megjelenik. Headless
(`cfa4-cli mcp` híd nélkül) módban nincs L5: a `surface.devices` üres listát ad, a profil- és kötésparancsok
és a `surface.inject` működnek (tesztelhető automatizálás eszköz nélkül).
