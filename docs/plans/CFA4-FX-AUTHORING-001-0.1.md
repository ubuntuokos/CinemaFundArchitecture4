# CFA4-FX-AUTHORING-001 – effektszerkesztés és -láncok, 0.1

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-21 22:45, Europe/Budapest). **Kódolási engedélyt nem ad:** minden fázis külön tulajdonosi engedélyt igényel (coordination.md §2.10). **Dátum:** 2026-11-21 · **Kérte:** István (ubuntuokos).
> **Alap:** CFA4 `main` @ `97d7982f`; ez a dokumentum csak dokumentáció.
> **Értelmezés (a jóváhagyással elfogadva; változtatása tervmódosítás):** az „FX Authoring” = az **effektek összeállításának és újrahasználatának rétege**: effektláncok, felhasználói előbeállítások (preset), paraméter-kifejezések és sablonok, a meglévő effektrendszerre építve. Nem külön szerkesztő és nem új motor. A névről a `main`-ben semmi nincs, a tartalom a tulajdonos szövegéből (DNEG-terv §1.2 A. pont: „effektek és effektláncok, procedurális effektek, keyframe és expression rendszer”) és a kódból következtetett.

## 1. Cél

A felhasználó a meglévő effektekből **saját, paraméterezhető, menthető effektet** állíthat össze (lánc + kivezetett paraméterek + kulcskockák + opcionális kifejezések), amelyet projektek közt újrahasznál. A DNEG-terv AX/VDB-képességei később ide *egy csomópontként* kapcsolódnak, nem külön szerkesztőként.

## 2. Kiinduló állapot (forrással)

| Tény | Forrás |
|---|---|
| Egy klipen több effekt (`EffectInstance`) lánca van, paraméterei kulcskockázhatók | `crates/project` (`EffectInstance`, `ParamValue`); Effect Controls |
| Előbeállítások már vannak (pl. Lumetri-presetek) | `crates/engine/src/lumetri_presets.rs` |
| Grafikai sablonok saját formátumban (`.fcgt`), szerkeszthető kivezetett tulajdonságokkal | `crates/engine/src/graphic_templates.rs` |
| Hangeffekt-szerkesztő parancs (`audio.fxEditor`) – minta arra, hogyan ad a motor szerkesztő-állapotot a GUI-nak | `crates/engine/src/fx_editor.rs` |
| Térbeli (voxel/pont) kifejezés **nincs**; a kulcskocka/időfüggő paraméter van | DNEG-audit D3 |
| Minden művelet engine-parancs, ügynökvezérelhető, automation-id | CLAUDE.md |

## 3. Hatókör

1. **Effektlánc-preset:** egy klip effektláncának mentése névvel (sorrend, paraméterek, kulcskockák), alkalmazása más klipre; projekt-könyvtár + felhasználói mappa (a `.fcgt` mintájára, saját formátum, verziózott).
2. **Kivezetett paraméterek („macro”):** a lánc néhány paramétere egy közös vezérlőre köthető (egy csúszka több paramétert hajt), kulcskockázható.
3. **Paraméter-kifejezések (időfüggő, skalár):** korlátozott, **Rust-oldali, biztonságos** kifejezés-kiértékelő (`t`, `frame`, `clipTime`, alapfüggvények); nincs fájl-/hálózat-/shell-hozzáférés, korlátozott idő és méret. A *térbeli* kifejezés (AX) a DNEG-terv N3-ja, nem ennek része.
4. **Katalógus-integráció:** a preset a [VFX-katalógusban](CFA4-VFX-CATALOG-001-0.1.md) `own:preset` eredettel jelenik meg.
5. **Qt6 GUI:** mentés/betöltés/macro-szerkesztés a meglévő Effect Controls-ban és effektböngészőben; minden elem parancson át.

**Nincs benne:** csomópont-alapú kompozitor (Fusion-szerű; VISION §4, külön terv), plugin-host, térbeli kifejezés, külön idővonal/projektmodell.

## 4. Fázisok és kapuk

| Fázis | Tartalom | Kilépési feltétel |
|---|---|---|
| EC-0 | Formátum- és modell-tervezés (preset séma, verziózás) | séma jóváhagyva |
| EC-1 | Effektlánc-preset mentés/alkalmazás (engine-parancsok) | EC-A1…A3 zöld |
| EC-2 | Macro-paraméterek | EC-A4 zöld |
| EC-3 | Skalár kifejezés-kiértékelő (biztonságos) | EC-A5…A7 zöld |
| EC-4 | Qt6-felület | EC-A8 zöld |

## 5. Elfogadási feltételek

- **EC-A1:** preset mentés → alkalmazás másik klipen azonos kimenetet ad (pixelpontos CPU-n), a kulcskockák megmaradnak.
- **EC-A2:** hibás/csonka/hostile preset-fájl → `Err`, nincs összeomlás (mutation-fuzz, AGENTS §0.8).
- **EC-A3:** ismeretlen effektazonosítót tartalmazó preset: a hiányzó elemeket jelenti, a többit alkalmazza, nem veszít el csendben semmit.
- **EC-A4:** egy macro-csúszka N paramétert hajt; mentés/betöltés round-trip.
- **EC-A5:** a kifejezés determinisztikus ugyanarra az `(t, frame)` bemenetre.
- **EC-A6:** a kifejezés nem tud I/O-t végezni, memória- és lépéskorlátos; túllépés → hiba az UI-n, nem lefagyás.
- **EC-A7:** végtelen/rekurzív kifejezés megszakad a korláton.
- **EC-A8:** Qt6 GUI-teszt (automation-id), CPU-only működés, regressziókapu (idővonal, render, projektmentés/-betöltés változatlan).

## 6. Licenc, függőség, erőforrás

Új külső függőség nincs; saját kód (GPL-3.0-or-later). Mivel a kifejezés-kiértékelő Rust, nincs `unsafe` és nincs JIT. Erőforrás: nem érinti az ACS-t (olcsó, képkockánkénti számítás).

## 7. Nyitott döntések

1. Az „FX Authoring” értelmezése (fenti feltételezés). Ha a tulajdonos egy csomópontos/grafikus effektszerkesztőt értett, az új, nagyobb terv (Resolve-Fusion-szintű kompozitor, VISION §4).
2. A kifejezésnyelv szintaxisa (javaslat: a Rust-oldali egyszerű aritmetikai nyelv; AX-kompatibilitás **nem** cél).
3. A preset-formátum neve és kiterjesztése.
