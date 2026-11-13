# CFA4-CONTROL-SURFACE-001 – Függőségi és licencjegyzőkönyv

**Dátum:** 2026-11-13 · **Alap:** `ubuntuokos/CinemaFundArchitecture4` main `92e29e4`, újraellenőrizve `5baa2fe`-n (`Cargo.lock`, `deny.toml`,
`docs/licensing.md`), crates.io adatai 2026-11-13-án · **Lefedi:** F0-AC-06, F0-AC-07, T0.3, T0.6

## 1. Eredmény

Az F1a és az F1 **új külső crate nélkül** megvalósítható: minden szükséges crate már a lockfile-ban van,
és a licencük a `deny.toml` engedélyezett listáján szerepel. A `deny.toml` módosítása nem szükséges.

| Crate | Verzió a lockban | Licenc | Mire kell | Ki húzza be ma |
|---|---|---|---|---|
| `alsa` | 0.9.1 | Apache-2.0 / MIT | ALSA Sequencer (`alsa::seq`): MIDI be/ki, announce-port | `cpal` 0.16.0 (`apps/cfa4`, `cfa4-tcsync-io`) és közvetlenül a `cfa4-tcsync-io` (`alsa = "0.9"`, MTC) |
| `alsa-sys` | 0.3.1 | MIT | `libasound2` FFI (az `alsa` alatt) | ugyanaz |
| `rustix` | 1.1.6 (`fs` funkció már be van kapcsolva) | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | inotify a hot-plughoz | több meglévő crate |
| `libc` | 0.2.189 | MIT OR Apache-2.0 | (közvetve) | sok crate |
| `serde`, `serde_json` | meglévő | MIT OR Apache-2.0 | `cfa4-surface-model`, profilok | sok crate |

Rendszerfüggőség: `libasound2` (futáskor, az Ubuntu alaprendszer része; már ma is kell a `cpal` miatt) és
fejlesztéskor `libasound2-dev` (már ma is kell). Új rendszerfüggőség nincs.

## 2. Az ALSA-verzió kérdése (19. fejezet, 4. kérdés)

**Kötelező korlát:** az `alsa-sys` `Cargo.toml`-jában `links = "alsa"` áll. A Cargo egy build-gráfban egy
`links` kulcsot csak egy csomagnak enged, ezért **két `alsa-sys`-verzió nem élhet egymás mellett** a
munkaterületen, akkor sem, ha a `deny.toml` `multiple-versions = "allow"`. A `cfa4-surface` tehát
ugyanazt az `alsa`/`alsa-sys` verziót kell használja, mint a `cpal` és a `cfa4-tcsync-io`.

| Időszak | Javaslat |
|---|---|
| Most | `alsa = "0.9"` (a `cpal` 0.16-tal és a `cfa4-tcsync-io`-val egyező, a lockban 0.9.1) |
| Ha a `cpal` frissül (crates.io-n 0.18.2 a legújabb) | az `alsa` vele együtt lép; az `alsa::seq` API a 0.9–0.12 közt kompatibilis **[F1 előtt ellenőrizendő]** |
| Frissítés (pl. `alsa` 0.12.1, `alsa-sys` 0.6.1) | csak együtt: a `cpal`, a `cfa4-tcsync-io` és a `cfa4-surface` egy PR-ban lép (a `cpal` az egui eltávolítása után is marad a `cfa4-tcsync-io` LTC-útján) |

`cargo tree -i alsa -e normal` (main `5baa2fe`, 2026-11-13): `alsa v0.9.1` ← `cfa4-tcsync-io` (← `cfa4-cli`) és ← `cpal v0.16.0` (← `cfa4`, `cfa4-tcsync-io`).

## 3. A specifikáció jelöltjei

| Függőség | Verzió (crates.io) | Licenc | Döntés | Indok |
|---|---|---|---|---|
| `midir` | 0.11.1 (2026-10-10) | MIT | **nem kell** | Nem ad hot-plug eseményt; az `alsa` `>=0.9.0, <0.13`-at kér, így használható lenne, de csak egy réteg az `alsa::seq` fölött, plusz `parking_lot`, `log` |
| `alsa` | 0.9.1 (lock) / 0.12.1 (legújabb) | Apache-2.0 / MIT | **kell** | 2. fejezet |
| `alsa-sys` | 0.3.1 / 0.6.1 | MIT | **kell** (közvetve) | 2. fejezet |
| `wmidi` | 4.0.11 | MIT | **nem kell** | A MIDI-parser kicsi, a `cfa4-surface-model`-ben saját (L1, wasm-tiszta, tesztelhető) |
| `midly` | 0.5.3 | Unlicense | **nem kell a termékben**; dev-dependency lehet SMF-tesztadatokhoz | A Learn- és parser-tesztekhez elég saját bájtsorozat |
| `evdev` | 0.13.2 (2025-09-15) | Apache-2.0 OR MIT | **csak ha `EVIOCGRAB` kell** (HID-értékelés 3. fejezet) | `nix` ^0.29 (a lockban 0.31.3 van → második verzió) és `bitvec` lánc (`funty`, `radium`, `tap`, `wyz`) jönne |
| `udev` | 0.9.3 | MIT | **nem kell** | `libudev-sys` (C `libudev`) kellene; az inotify + sysfs elég |
| `inotify` | 0.11.5 | ISC | **nem kell** | A `rustix` ugyanezt adja, és már a lockban van |
| hidapi, `rusb` | – | C-könyvtár (BSD/GPL-3 ill. LGPL-2.1+) | **elutasítva** | HID-értékelés |

## 4. Kódátvétel és licenc

| Forrás | Licenc | Kompatibilis? | Mit veszünk át | Teendő |
|---|---|---|---|---|
| Kdenlive `src/jogshuttle/` | GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL | igen (GPL-3.0-only, `licensing.md` §3) | A sebességtábla (0, 1, 2, 4, 5, 8, 16, 60), a ±7 tartomány, az evdev-középállás kezelése, Rustra portolva | Minden átvett fájl fejlécének ellenőrzése (SPDX), sor a `licensing.md` §5 nyilvántartásba, `ATTRIBUTION.md`, commitban forrás és revízió |
| shuttle-go | MIT | igen | Csak viselkedés (lassú jog 200 ms küszöb); ha kód, akkor §5 sor | – |
| Shuttle Lander | GPL-3.0 | igen | Csak viselkedés (a középállás-hiba leírása) | – |
| Ardour | GPL-2.0-or-later; a MIDI-map fájlok licence nem tisztázott | – | **Semmi**: csak viselkedési referencia; a beépített profilok saját adatok | – |
| Mixxx | GPL-2.0-or-later | – | **Semmi**: csak viselkedési referencia (Learn-időzítések) | – |

## 5. Dokumentációs teendők

| Fájl | Változás | Mikor |
|---|---|---|
| `docs/licensing.md` §5 | Kdenlive-port sora | az első portolt kóddal |
| `ATTRIBUTION.md` | Kdenlive jogshuttle | ugyanakkor |
| `NOTICE` | nem változik (nincs új Apache-2.0 NOTICE-köteles forrás) | – |
| `deny.toml` | nem változik | – |
| `xtask` `LAYERS` + tiltólista | `surface-model` L1, `surface` L5; `alsa`, `alsa-sys`, `evdev`, `udev`, `inotify` L5 alatt tilos | az első crate-tel |

## 6. Ellenőrzés F1 előtt (T0.3)

1. `cargo tree -i alsa` és `cargo tree -i alsa-sys`: egy verzió.
2. `cargo deny check licenses bans advisories`: zöld.
3. `cargo xtask layers`: a két új crate a helyén, L5 alatt nincs ALSA.
4. Ha közben a `cpal` frissült: az `alsa::seq` API-t használó kód fordul a új verzióval.
