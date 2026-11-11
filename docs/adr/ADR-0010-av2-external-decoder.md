# ADR 0010 – AV2 külső dekóder bekötése (A vonal)

**Dátum:** 2026-11-11 · **Állapot:** javaslat, a tulajdonos (`ubuntuokos`) jóváhagyására vár ·
**Kapcsolódik:** [AV2 terv v1.2](../plans/AV2-CODEC-PLAN-2026-10-29.md) III. fejezet és X. fejezet 2. PR,
[AV2-00 audit](../audit/AV2-00-AUDIT-2026-10-30.md),
[ADR 0001](0001-platform-ffi.md) (FFI), [ADR 0004](ADR-0004-compute-routing.md) (`cfa4-route`),
[ADR 0005](ADR-0005-media-backends.md) (kiegészítő médiahátterek)

## Háttér

Az AV2 terv A vonala korai, konform CPU-s AV2-dekódolást kér a saját Rust-dekóder (B vonal)
elkészülte előtt. A terv III. fejezete négy bekötési módot sorol fel, és az AV2-00 kimeneteként
ADR-t ír elő. Az AV2-00 mérései (AVM v1.0.0 `966a7d7` orákulum, 8 vektoros korpusz):

| Jelölt | Mérés | Bitpontos | Megjegyzés |
|---|---|---|---|
| dav2d HEAD `18641bb` (C, BSD-2-Clause) | 2026-11-11 | 7/8 | Nincs összeomlás; a 4:4:4 inter képkockák eltérnek. Minden 4:2:0 és 4:0:0 vektor bitpontos |
| dav2d HEAD `326a357` | 2026-11-05 | 7/8 | A 4:4:4 vektoron SIGSEGV (azóta javítva) |
| dav2d 0.0.1 (egyetlen kiadás) | 2026-11-05 | 2/8 | AVM v15-alapú, elavult |
| rav2d 0.4.2 (Rust, BSD-2-Clause) | 2026-11-11 | 4/8 | Intra-only, páratlan méret, 4:4:4, 4:0:0 hibás |
| FFmpeg 9.0.2 / `master` | 2026-11-07 | – | Nincs AV2, nincs `libdav2d` wrapper |
| GStreamer | 2026-10-29 | – | Nincs AV2-elem |

Tanulság: a legjobb jelölt (dav2d) C-kód, kiadása nincs, és egy hete még összeomlott konform
bitfolyamon.

**Szabályváltozás a terv v1.2 óta:** a tulajdonos 2026-11-07-i döntése (AGENTS.md CFA4-szakasz,
[ADR 0001 kiegészítés](0001-platform-ffi.md#addendum-2026-11-07-non-rust-components-owner-decision))
szerint nem Rust komponens (például a C nyelvű dav2d) csak akkor ütközik a CFA4 szabályaival, ha
lassítja a CFA4-et; kötése az ADR 0001 elszigetelési szabályait követi, és a külön folyamat
egyenrangú választás. A terv III. fejezetének 2. lehetőségéhez (saját FFI) ezért **már nem kell
ADR 0001-módosítás**. A „soha nem crash-el” szabály (AGENTS.md 0.) változatlan: egy C-dekóder
folyamaton belüli összeomlása a CFA4-et is leállítaná, és ez `catch_unwind`-dal nem fogható meg.

## Döntés (javaslat)

1. **Az A vonal külső dekódere külön folyamatban fut** (terv III., 3. lehetőség). A CFA4 egy
   segédprogramot indít (munkanév: `cfa4-av2-decode`), amely a bitfolyamot stdin-en vagy
   fájlútvonalon kapja, a dekódolt képkockákat egy egyszerű, verziózott keretformátumban
   (fejléc: szélesség, magasság, kroma, bitmélység, PTS; utána a síkok) írja a stdout-ra. A
   segédprogram belül a dav2d-t (vagy fejlesztői módban az AVM `avmdec`-et) hívja.
   - Új FFI és `unsafe` nem kerül a CFA4 crate-jeibe; az ADR 0001 nem változik.
   - A motor oldali adapter tiszta Rust (`std::process`), a `VideoDecoder` felületet valósítja
     meg, és a `cfa4-route`-ba új háttérként, saját `routing.toml` sorral kerül.
   - A segédprogram összeomlása, időtúllépése vagy hibás kimenete a motorban `Result` hiba lesz
     (a forrás „nem dekódolható” állapotba kerül), nem crash. Időkorlát, kimeneti méretkorlát és
     a fejléc ellenőrzése a beolvasott minden képkockára kötelező.
2. **A dav2d rögzített commitról, vendorolt forrásból épül**, a CFA4 saját prefixébe
   (`/opt/cfa4`), rendszerkönyvtár felülírása nélkül (terv AV2-10). Az Ubuntu `libdav2d1`
   csomag (0.0.1) nem használható.
3. **Regisztráció csak G1 PASS után.** Addig a háttér fejlesztői kapcsoló (feature flag és
   környezeti változó) mögött marad, AV2-LAB szinten. A G1-hez a terv II.3 szerinti
   konformancia kell a vállalt `Main_420_10_IP0` egystreamű körre, a bővített korpuszon
   (terv II.4), és fuzzing a segédprogram bemenetére.
4. **A rav2d (4. lehetőség) a folyamaton belüli utód-jelölt.** Ha a II.3 szerinti konformanciát
   eléri, `cargo deny` alatt, rögzített verzióval függőségként vehető fel (csak a `rav2d` crate,
   a `rav2d-sys` nem), és ugyanazt a `VideoDecoder` felületet valósítja meg. A sorrendváltás a
   routerben az ADR 0005 szerint méréssel és teszttel történik.
5. **Folyamaton belüli dav2d-kötés (2. lehetőség) a mérés után.** Az ADR 0001 kiegészítése
   megengedi (saját kötő crate, `unsafe` csak az FFI-modulokban, `// SAFETY:` minden blokkon,
   biztonságos `Result` API). Akkor váltunk rá, ha (a) a referenciagépen mérve a külön folyamat
   lassítja a lejátszást vagy a seeket (terv G1/G8, ADR 0001 kiegészítés 5. pont), és (b) a
   dav2d a bővített korpuszon és legalább 24 órás fuzzingon sem omlik össze. Ehhez új ADR nem
   kell, csak mérés és teszt a PR-ban; a külön folyamatos út tartalékként megmarad.
   A dav2d Rust-ra cserélése (rav2d vagy a saját `cfa4-av2`, B vonal) a terv saját feladata
   (ADR 0001 kiegészítés utolsó bekezdése).
6. **Az FFmpeg-út (1. lehetőség)** automatikusan elérhetővé válik, ha a CFA4 FFmpeg-buildje
   AV2-t kap; addig nincs teendő. A router a futásidejű képességvizsgálat alapján veszi fel.

## Következmények

- **Előny:** a dekóder hibái elszigeteltek; a CFA4 nem kap új `unsafe` kódot; a dekóder cseréje
  (dav2d → rav2d → saját `cfa4-av2`) a hívókat nem érinti.
- **Hátrány:** a képkockák másolása a csövön keresztül többletköltség (1080p 4:2:0 8 bit
  képkockánként kb. 3 MB). A G1 alapmérésének (terv AV2-01) ezt is mérnie kell, és ha nem
  valós idejű, a proxyút kötelező (terv AV2-03).
- **Csomagolás:** a segédprogram és a vendorolt dav2d licencfájljai (BSD-2-Clause, AOM Patent
  License 1.0 szövege) a csomag notices-anyagába kerülnek (AV2-00 audit 8. pont).
- **Tesztek:** a segédprogram és az adapter tesztjei az AVM-orákulum hash-eivel futnak
  (`crates/testkit`, terv X. 3. PR); a hibás bemenetek (csonka, sérült, túlméretes) tesztjei
  és a crash-elszigetelés tesztje (a segédprogram szándékos leállítása) kötelező.

## Nyitott kérdések a tulajdonosnak

- Jóváhagyja-e a külön folyamatos utat A-vonalas kiindulásként (1–3. pont), és a folyamaton belüli
  kötésre váltás 5. pont szerinti mérési feltételeit?
- A segédprogram a `cfa4` csomag része legyen, vagy külön, opcionális csomag (pl. `cfa4-av2-dav2d`)?
- A dav2d upstream felé jelezzük-e a 4:4:4 inter eltérést (AV2-00 audit 4. pont)?
