# ADR 0013 – H.266/VVC bekötése: VVdeC külön folyamatban, kódolás FFmpeg `libvvenc`-kel

**Dátum:** 2026-11-14 · **Állapot:** javaslat (VVC-01, a tulajdonos jóváhagyására vár; G1 kapu) ·
**Kapcsolódik:** [VVC terv v1.2](../plans/CFA4-VVC-PLAN-001-2026-11-07.md) 3., 4. és 7. fejezet,
[VVC-00 jelentés](../plans/CFA4-VVC-00-REPORT-2026-11-14.md),
[ADR 0001](0001-platform-ffi.md) (FFI, 2026-11-07-i kiegészítés), [ADR 0004](ADR-0004-compute-routing.md) (`cfa4-route`),
[ADR 0005](ADR-0005-media-backends.md) (kiegészítő hátterek), [ADR 0010](ADR-0010-av2-external-decoder.md)
(külső AV2-dekóder, ennek a mintája), [`docs/licensing.md`](../licensing.md) §7

## Háttér

A VVC terv VVC-01 munkacsomagja ADR-t kér a VVdeC bekötésének módjáról (könyvtárhívás vagy külön
folyamat), a binding helyéről és rétegéről, a `routing.toml` sorokról és a `cfa4-restricted-extras`
észleléséről. A VVC-00 mérései (felhős gép, 4 szál; a referenciagépen a VVC-02 ismétli):

| Út | 1080p dekódolás | 2160p dekódolás | Kimenet |
|---|---|---|---|
| VVdeC v3.2.1 | 150,6 fps | 41,9 fps | bitre azonos az FFmpeg-gel |
| FFmpeg 9.0.2 natív `vvc` | 90,0 fps | 18,0 fps | bitre azonos a VVdeC-cel |

További tények a VVC-00-ból és a repóból:

- Az FFmpeg 9.0.2-ben **nincs** `libvvdec` wrapper, ezért a VVdeC-hez saját kötés kell. A `libvvenc`
  wrapper van; a VVenC-et ehhez megosztott könyvtárként kell fordítani.
- A `libvvenc` MKV-ba és MPEG-TS-be hibás DTS-t ír; MP4-be helyesen.
- A repóban már működik egy külön folyamatos dekódermintázat: az AV2 `cfa4-av2-decode` segédprogramja
  (`packaging/av2-decode/`) és a tiszta Rust adapter (`crates/codecs/src/av2_external.rs`, frame
  protocol v1). Ez kodekfüggetlen; csak a hello-azonosító és a minták (sample) feldarabolása
  kodekspecifikus.
- A „soha nem crash-el” szabály (AGENTS.md 0.) miatt egy C++ dekóder folyamaton belüli összeomlása
  a CFA4-et is leállítaná, és `catch_unwind`-dal nem fogható meg.
- A CFA4-ben ma nincs VVC-nyom (`crates/isobmff` `CodecConfig`-ban sincs `Vvc`).

## Döntés (javaslat)

1. **A VVdeC külön folyamatban fut, az ADR 0010 mintájára.** Új segédprogram: `cfa4-vvc-decode`
   (C, GPL-3.0-or-later, a VVdeC C API-ját, a `vvdec/vvdec.h`-t hívja), helye
   `packaging/vvc-decode/`. Ugyanazt a **frame protocol v1**-et beszéli, mint a `cfa4-av2-decode`,
   a hello-azonosító `"CFA4VVCD"`. Egy `DATA` üzenet egy hozzáférési egység (access unit, Annex B
   NAL-ok, a hossz-előtagos MP4-mintákat az adapter Annex B-re alakítja a `vvcC` paraméterhalmazaival).
   - **Közös adapter:** az `av2_external.rs` protokoll- és folyamatkezelő része kodekfüggetlen modulba
     kerül (munkanév `crates/codecs/src/external_decoder.rs`); az AV2 és a VVC csak a hello-azonosítót,
     a minta-darabolást és a véletlen hozzáférés felismerését adja hozzá. Az AV2 viselkedése nem
     változik (a meglévő `av2_external` tesztek kapuként futnak).
   - **Nincs új binding crate és nincs új `unsafe`** a CFA4 crate-jeiben; a terv 4. fejezetében
     javasolt `crates/vvc-native` nem kell. Az ADR 0001 nem változik.
   - Összeomlás, időtúllépés vagy hibás kimenet a motorban `CodecError` (a forrás „nem dekódolható”),
     nem crash. Időkorlát, üzenetméret-korlát és a fejléc ellenőrzése minden képkockán kötelező (mint
     az AV2-nél).
2. **Rust oldali VVC-elemző a VVC-02-ben, L0 crate-ben** (`crates/vvc`, csak `cfa4-bitstream`
   függőséggel): NAL-fejléc, hozzáférésiegység-határok, IRAP/GDR felismerés (`is_random_access`),
   nem referencia képek (`is_disposable`), SPS-ből méret, bitmélység, kroma és VUI-színadatok. Ez a
   terv B vonalának (saját Rust-dekóder) első lépése is.
3. **Konténerek:** `crates/isobmff` új `CodecConfig::Vvc` változatot kap (`vvc1`/`vvi1`, `vvcC`),
   a `crates/matroska` a `V_MPEGI/ISO/VVC`, a `crates/mpegts` a `0x33` stream type-ot ismeri fel.
   Olvasás a VVC-02-ben, írás a VVC-03-ban; a PTS/DTS-t a CFA4 számolja, nem a kódoló.
4. **Útválasztás** (`crates/route/routing.toml`, ADR 0004):

   ```toml
   [compute.task."video.decode.vvc"]
   order = ["vvdec", "ffmpeg", "vaapi"]

   [compute.task."video.encode.vvc"]
   order = ["ffmpeg-libvvenc"]
   ```

   A `vvdec` út csak akkor kerül a listába a futásidejű vizsgálatnál, ha a segédprogram megvan és a
   hello-üzenete érvényes; különben a router kihagyja, és az FFmpeg natív `vvc` dekódere veszi át. A
   `vaapi` csak futásidejű teszt után kerülhet előre (referenciagépen NOT SUPPORTED: NVIDIA, és a
   Battlemage sem dekódol VVC-t). Téves dekóder (például HEVC) sosem fut VVC helyett.
5. **Kódolás FFmpeg `libvvenc`-kel**, a meglévő `cfa4-ffmpeg` háttéren át. A `RESTRICTED` listába
   (`crates/ffmpeg/src/lib.rs`) új sor kerül: `libvvenc` – „H.266/VVC software encoding (VVenC)”.
   A kódoló csomagokat ad, a konténerbe írás a CFA4 muxereivel történik (3. pont). Amíg a saját
   muxer nem ír VVC-t, a VVC-export csak MP4 (VVC-FR-5). Exportprofilok: Gyors = VVenC `faster`,
   Általános = `medium`, Magas = `slow`, Archiválási = `slower` (a pontos presetek a VVC-03 mérése
   alapján); az uvg266 nem exportút (tulajdonos, 2026-11-14).
6. **Csomagolás és észlelés:** a `cfa4-vvc-decode`, a VVdeC és a VVenC megosztott könyvtárai, valamint
   a `libvvenc`-es FFmpeg-részek a **`cfa4-restricted-extras`** csomagba kerülnek (licensing.md §7,
   tulajdonosi döntés 2026-11-07), a CFA4 saját prefixébe (`/opt/cfa4`), rendszerkönyvtár felülírása
   nélkül. A segédprogram helye: `/opt/cfa4/libexec/cfa4-vvc-decode`, fejlesztéskor a `CFA4_VVC_DECODER`
   környezeti változó. A `crates/media` háttérjelentése (`BackendReport.restricted`) új tételként
   mutatja („VVdeC decoder”), így a Beállítások ▸ Médiahátterek megmondja, ha a csomag hiányzik.
7. **Rögzített források:** `packaging/vvc-decode/vvdec.json` (VVdeC v3.2.1, `cb1a3ef`) és
   `vvenc.json` (VVenC v1.14.0, `9428ea8`), a `dav2d.json` mintájára. Verziót csak új
   konformanciafuttatással együtt lehet váltani. Build: `scripts/dev/vvc-tools.py build` (CMake,
   `-DBUILD_SHARED_LIBS=ON`), a `target/vvc-tools/` alá.
8. **Fokozatos bekapcsolás:** a VVC-út a `vvc-external` cargo feature és a `CFA4_VVC_DECODER` mögött
   indul (VVC-LAB szint). A routerbe regisztrálás és az alapértelmezett bekapcsolás csak **G2 PASS**
   után: VTM-orákulum és JVET konformancia (a G0 feltételes elfogadása szerint ez a G2 belépő
   feltétele), plusz fuzzing a segédprogram bemenetén.
9. **Folyamaton belüli kötés a mérés után** (ADR 0010 5. pontjával azonos feltételek): ha a
   referenciagépen mérve a cső lassítja a lejátszást vagy a seeket (1080p 10 bit 4:2:0 kb.
   6,2 MB/kép, 150 fps-nél kb. 0,9 GB/s), és a VVdeC a korpuszon és legalább 24 órás fuzzingon sem
   omlik össze, saját kötő crate jöhet az ADR 0001 kiegészítése szerint. Ehhez új ADR nem kell, csak
   mérés és teszt a PR-ban; a külön folyamatos út tartalék marad.

## Interfészterv (VVC-01 második pontja)

| CFA4-felület | VVC-hez szükséges változás | Munkacsomag |
|---|---|---|
| `VideoDecoder` (`crates/codecs/src/video.rs`) | `ExternalVvcDecoder` a közös külső adapterre; `is_random_access` és `is_disposable` a `crates/vvc` elemzőből | VVC-02 |
| `MediaSource` (`crates/media`) | VVC-forrás MP4/MKV/TS-ből (konténer-crate-ek) és nyers `.266`-ból (Annex B indexelés, mint az `Av2AnnexBSource`) | VVC-02 |
| Dekóderregiszter (`crates/codecs/src/lib.rs`) | `CodecConfig::Vvc` → a router `video.decode.vvc` sora; hiányzó út esetén egyértelmű `Unsupported` üzenet a csomagnévvel | VVC-02 |
| `ColorInfo` (`crates/color`) | A `FRME` fejléc mátrix/átvitel/alapszín/tartomány mezői; konténer `colr` elsőbbsége a VUI előtt, mint a HEVC-nél; HDR (PQ/HLG) metaadat `mdcv`/`clli`-ből | VVC-02, VVC-05 |
| `ExportQueue` (`crates/engine/src/export_tools.rs`) | VVC-presetek (5. pont), csak MP4 konténer engedélyezve a saját muxerig; hiányzó `libvvenc` esetén a preset letiltva, a csomagnévvel | VVC-03 |
| Proxy (`crates/engine/src/proxies.rs`) | VVC-forrásnál ajánlott proxy, ha a dekódolás nem valós idejű (4K-nál a VVC-00 szerint 4 szálon nem az) | VVC-04 |
| Háttérjelentés (`crates/media/src/backends.rs`) | „VVdeC decoder” és „VVenC encoder” korlátozott tételek | VVC-02, VVC-03 |

## Következmények

- **Előny:** a VVdeC hibái elszigeteltek; nincs új `unsafe`; az AV2 és a VVC egy kipróbált
  protokollon és adapteren osztozik; a dekóder cseréje (VVdeC → saját Rust `crates/vvc`) a hívókat
  nem érinti.
- **Hátrány:** a képkockák csövön mennek át (lásd 9. pont); ezt a VVC-02 a referenciagépen méri.
  A közös adapter kiemelése az AV2-kódot is érinti, ezért az AV2 tesztjei kötelező kapuk abban a PR-ban.
- **Csomagolás:** a VVdeC és a VVenC licencfájljai (BSD-3-Clause-Clear, amely kifejezetten nem ad
  szabadalmi engedélyt) a `cfa4-restricted-extras` notices-anyagába kerülnek.
- **Tesztek:** a segédprogram és az adapter a VTM-orákulum hash-eivel (G2), hibás bemenetekkel
  (csonka, sérült, túlméretes), a crash-elszigetelés tesztjével és a VVdeC–FFmpeg bitazonossággal fut;
  a `libvvenc` MKV/TS DTS-hibájára regressziós teszt kerül (VVC-TEST-004/005).

## Nyitott kérdés a tulajdonosnak

Nincs új döntési pont a tervhez képest: a külön folyamat és a `cfa4-restricted-extras` csomagolás a
terv és a korábbi döntések következménye. Jóváhagyásra vár: ez az ADR egészében (G1).
