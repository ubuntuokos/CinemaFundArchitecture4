# CFA4-VFX-CATALOG-001 – a VFX-katalógus terve, 0.1

> **[MÓD-1] MÓDOSÍTÁS JÓVÁHAGYÁSRA VÁR** (egyeztetés: [`CFA4-DNEG-VFX-EGYEZTETES-001`](CFA4-DNEG-VFX-EGYEZTETES-001-0.1.md) K-1, K-6, D-1). A jóváhagyásig a lenti 0.1 szöveg érvényes; jóváhagyás után az alábbi szűkítés **felülírja** az érintett pontokat.
> A `CFA4-VFX-001` v1.0 (X-3, jóváhagyva 2026-11-22) szerint **egy** katalógus van: a `docs/audit/render-f0v/F0-capability-gap-matrix.csv` (`G-xx`). Ezért:
> - **Törlődik** a külön katalógus: a §3 1. pont (generált manifeszt), a `docs/vfx-catalog.md`, a VC-A1 és VC-A4 külön-manifeszt része, a VC-1 és VC-4 fázis (az effektböngésző a meglévő effektregiszterből épül, külön manifeszt nélkül).
> - **Marad** (a VFX-1 részeként): (a) a regisztrációs szabály: minden új effekt (OFX, VDB, kifejezés, saját) csak mátrixsorral együtt kerülhet be (VC-A5); (b) egy `cargo xtask` **konzisztencia-ellenőrzés**: a mátrix `EXISTING` effektsorai és a regisztrált `EffectDef`-ek száma/azonosítója egyezik, nincs azonos `id` vagy azonos nevű, eltérő `id` (VC-A1, VC-A2, hálózat nélkül, determinisztikusan); (c) az eredet/licenc oszlopok a VFX-001 §6 mezői (`source_project`, `license_status`), nem külön oszlopok.
> - **Jelölés:** a lenti „G18/G11” a `gaps.md` tételeit jelenti (effektmélység / plugin-hosting), **nem** a mátrix G-18/G-11 sorát (OpenFX-host / alfa és ProRes 4444).

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-21 22:45, Europe/Budapest). **Kódolási engedélyt nem ad:** minden fázis külön tulajdonosi engedélyt igényel (coordination.md §2.10).
> **Dátum:** 2026-11-21 · **Kérte:** István (ubuntuokos); a [DNEG-adaptációs terv](CFA4-DNEG-TECHNOLOGY-ADAPTATION-001-1.0.md) hivatkozott rá, de a `main`-ben nem volt ilyen terv.
> **Alap:** CFA4 `main` @ `97d7982f`. Ez a dokumentum csak dokumentáció.
> **Értelmezés (a jóváhagyással elfogadva; változtatása tervmódosítás):** a „VFX Catalog” = a CFA4 összes videóeffektjének **egyetlen, gépileg olvasható leltára és szabálya** (mi van, hol fut, honnan származik, mi teszteli). Nem új effektmotor.

## 1. Cél

Egy hely, amely megmondja, mi a CFA4 effektkészlete, és amely minden új képességnek (OpenFX, OpenVDB-alapú, AX, saját) kötelező regisztrációs pontja. Ezzel nincs párhuzamos effektlista, és az ADR-0003/G11/G18 jelöltjei és a DNEG-terv új képességei **duplikáció nélkül** illeszthetők be.

## 2. Kiinduló állapot (a `main`-ből, forrással)

| Tény | Forrás |
|---|---|
| Az effektek definíciója Rust-adat: `EffectDef { id, name, kind, category, params, intrinsic, accelerated, float32, yuv }` | `crates/project/src/effect.rs` (`EffectDef`, ~255. sor) |
| A Premiere 26 katalógus (M5.11) CPU-s megvalósítása: blur, color, distort, glitch, stylize, temporal, lights, denoise, pixelate, text, immersive, auto | `crates/render/src/vfx/*` (14 modul), `crates/project/src/effect/vfx.rs` |
| CFA4-saját effektek is vannak (pl. `particles`, `databend`, `glint`) | `crates/project/src/effect/vfx.rs:509` és szomszédai |
| GPU-gyorsítás van egyes effektekre (`accelerated`), wgpu/WGSL | `crates/gpu/src/fx.wgsl`, `gpufx.rs` |
| Plugin-hosting nincs (G11); OpenFX-host terv: ADR-0003 A4 (külön folyamat) | `docs/gaps.md` G11, ADR-0003 §9 |
| A mélység-hiányok listája: G18 (Warp Stabilizer 2-D, Morph Cut face-tracking nélkül, …) | `docs/gaps.md` G18 |

## 3. Hatókör

**Benne:**
1. **Katalógus-manifeszt:** az effektdefiníciókból *generált* táblázat/JSON (id, név, kategória, kind, paraméterek, kulcskockázható?, CPU/GPU/YUV/float32, eredet, licenc, tesztlefedettség, állapot). Forrás: `EffectDef`; nem kézzel írt másolat.
2. **Eredet- és licencoszlop:** `own` (CFA4-saját), `parity` (Premiere-viselkedés megfigyelés alapján, clean-room), `ofx:<csomag>` (külső OFX, GPL-3-kompatibilis), `vdb:<…>` (DNEG-terv N2/N3 kimenete). Adobe-eredetű eszköz tilos (AGENTS §1).
3. **Hiánylista** a katalógus és a G18/ADR-0003 jelöltjei közt (kulcsolók, követés, retime…), csak *forrásolt* hiányokkal; elfogadott funkciót nem vesz fel újra.
4. **Regisztrációs szabály:** minden új effekt (OFX-ból, VDB-ből, AX-ből) `EffectDef`-ként jelenik meg; külön lista nincs.
5. **Qt6:** az effektböngésző ebből a manifesztből épül (kategória, keresés); egyetlen UI.

**Nincs benne:** új effektek megvalósítása (azok saját terveikben), render-motor csere, plugin-host (A4), új idővonal.

## 4. Fázisok és kapuk

| Fázis | Tartalom | Kilépési feltétel |
|---|---|---|
| VC-0 | Leltár: az összes `EffectDef` kinyerése és a manifeszt-séma | a manifeszt-séma jóváhagyva; **0** kézi másolat |
| VC-1 | Generátor (`cargo xtask` al-parancs) + `docs/vfx-catalog.md` generálva | generált fájl egyezik a kóddal (CI-ellenőrzés: eltérés esetén piros) |
| VC-2 | Eredet/licenc/teszt-oszlopok kitöltése | minden sorhoz eredet; a licenc nélküli sor tiltott |
| VC-3 | Hiánylista a G18/ADR-0003 alapján | minden hiánysorhoz forrás; elfogadott funkció nincs duplán |
| VC-4 | Qt6 effektböngésző bekötése a manifesztre | UI-teszt automation-id-vel |

Kapu: VC-n csak az előző zöld, és külön tulajdonosi engedéllyel indul.

## 5. Elfogadási feltételek

- **VC-A1:** a manifeszt sorainak száma = a regisztrált `EffectDef`-ek száma (teszt).
- **VC-A2:** nincs két azonos `id`; nincs azonos nevű, eltérő `id`-jű duplikátum (teszt).
- **VC-A3:** minden sorhoz van eredet és licenc; Adobe-eredetű jelölés esetén a build elbukik.
- **VC-A4:** a katalógus-generálás hálózat nélkül, determinisztikusan fut.
- **VC-A5:** új OFX/VDB effekt csak manifeszt-sorral együtt kerülhet be (CI-szabály).

## 6. Licenc, függőség, erőforrás

Új külső függőség nincs (csak Rust-oldali generálás). Erőforrás-kezelés: nem érinti. Never-crash: a manifeszt-olvasó hibás bemenetre `Err`.

## 7. Nyitott döntések

1. A „VFX Catalog” fenti értelmezése helyes-e (leltár+szabály)? Ha a tulajdonos tartalmilag mást értett (pl. külső, letölthető effektgyűjtemény), a terv módosul.
2. A manifeszt formátuma: JSON (gépi) + generált Markdown (emberi) – javasolt.
