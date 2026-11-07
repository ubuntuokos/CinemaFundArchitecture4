# CFA4-AV2-PLAN-001 – AV2 kodekbevezetési és fejlesztési terv

**Verzió:** 1.2 (a források 2026-11-07-i újraellenőrzése és az AV2-00 mérések alapján; a v1.1 a 2026-10-29-i review javított változata volt)
**Dátum:** 2026-11-07 (v1.2); 2026-10-29 (v1.1)
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) jóváhagyta: v1.1 2026-10-29, v1.2 2026-11-07; a fejlesztés indítása külön döntéshez kötött
**Végrehajtás:** jóváhagyás után, külön fejlesztésindítási döntéssel (lásd XII.)
**Kapcsolódik:** [ADR 0001](../adr/0001-platform-ffi.md) (FFI), [ADR 0004](../adr/ADR-0004-compute-routing.md) (`cfa4-route`),
[ADR 0005](../adr/ADR-0005-media-backends.md) (kiegészítő médiahátterek), [`docs/licensing.md`](../licensing.md),
[`docs/coordination.md`](../coordination.md)

A v1.1-hez képesti változásokat a XIV.1, a v1.0-hoz képestieket a XIV.2 fejezet sorolja fel. A tényállás 2026-11-07-i; a forrásokat a XV. fejezet adja meg.

A v1.2 csak a tényállást és a belőle következő pontosításokat frissíti; a XII. fejezet stratégiai döntéseit nem változtatja. A tulajdonos a v1.1-et 2026-10-29-én, a v1.2-t 2026-11-07-én hagyta jóvá. A fejlesztés indítása továbbra is külön döntéshez kötött.

---

## I. A fejlesztés célja

A CFA4 kapjon teljes értékű, szabványos AV2 videokodek-támogatást, amely alkalmas a következőkre:

1. AV2-videók felismerése és importálása.
2. AV2-videók CPU-alapú dekódolása.
3. AV2-anyagok lejátszása a Qt6 szerkesztőben.
4. Képkockapontos vágás, seek, frame-step és timeline-műveletek.
5. Proxyállományok létrehozása és kezelése (a proxy a CFA4 meglévő proxykodekjével készül, nem AV2-vel).
6. AV2-videók natív kódolása.
7. AV2-export támogatott, szabványos konténerekbe.
8. HDR- és színmetaadatok helyes kezelése.
9. Többszálú dekódolás és kódolás.
10. A v1.0.0 profiljain túli képességek (többrétegű, többnézetes videó, későbbi toolsetek), amikor a szabvány és a konformancia lehetővé teszi.
11. Opcionális, igazolt hardveres gyorsítás.
12. Konformancia és interoperabilitás más AV2-implementációkkal.

**Alapkövetelmény:** a fejlesztés nem okozhat regressziót a CFA4 meglévő AV1, H.264, HEVC, VP9 és egyéb kodekjeiben.

Az AV2 elsődleges célja a CFA4 saját szerkesztő- és exportképességének bővítése, nem a YouTube webes lejátszójának másolása. YouTube-kompatibilis exportot kizárólag a platform által ténylegesen elfogadott AV2-bemenet igazolása után lehet hirdetni (a YouTube webes AV2 legkorábban 2027 elején várható).

## II. Tényállás és a szabvány kötelező elemei

### II.1 Ökoszisztéma (újraellenőrizve 2026-11-07)

A „v1.2” jelölés azt mutatja, ahol a 2026-10-29-i állapothoz képest változás van.

| Elem | Ellenőrzött állapot | Következmény |
|---|---|---|
| AV2 specifikáció | v1.0.0, 2026-05-28 (PDF `20260528_38f28e7`); a v13 munkavázlat elavult. **v1.2:** az `AOMediaCodec/av2-spec` repóban (az av2.aomedia.org forrása) 2026-05-30 óta nincs új commit (`016ffa9`): v1.0.1, errata vagy újabb nyilvános változat nincs. Az AVM `main` ága már egy belső specifikációs vázlatot követ, amely 12 bites profilt (`Main_444C_12_IP2`, `seq_profile_idc = 5`) vezet be (AVM #5112, 2026-08-04) | A v1.0.0 marad a rögzített alap. A 12 bites profil nem része a v1.0.0-nak; csak nyilvánosan kiadott specifikációmódosítás után kerülhet be (AV2-09) |
| AVM | v1.0.0 tag (`966a7d7`); `avmenc`/`avmdec`; IVF kimenet `AV02` fourcc-vel; nagyon lassú, egyszálú. **v1.2:** továbbra is a v1.0.0 az egyetlen kiadási tag. Az `av2-normative` ág (`14f43e4`, 2026-10-08) = v1.0.0 + 9 commit: bitfolyam-konformancia ellenőrzések (CWG-G089 OrderHint, multistream atlasz-OBU-k, `is_first_tile_group`) és robusztussági javítások (leb128, túlcsordulás). A `main` 431 committal jár a v1.0.0 előtt (2026-11-05: `7a0c956`), benne enkóderváltozások, a 12 bites profil és a switch frame konformancia (VCWG-G144) | Orákulum a v1.0.0 tag, kereszt-ellenőrzés az `av2-normative` ág rögzített commitjával (II.3); a `main` nem orákulum |
| dav2d (VideoLAN) | 0.0.1 preview (2026 április–május, `0430370`), AVM v15-alapú; BSD-2-Clause (ellenőrizve). **v1.2:** új tag nincs; HEAD `1ff0679` (2026-11-06). A CFA4 AV2-00 mérése (PR #635, 2026-11-05): HEAD `326a357` 7/8 bitpontos, a 4:4:4 vektoron SIGSEGV; 0.0.1 2/8. Azóta upstream: „Fix 4:2:2 and 4:4:4 block syntax parsing” (`2f7c315`, 2026-11-02) és 4:2:2/4:4:4 CfL C-implementáció (`c8da122`). Ubuntu-csomag (`libdav2d1`, 0.0.1) csak a 26.10-ben (stonking) és a 27.04 fejlesztői ágban (tenacious) van, a 26.04 LTS-ben nincs | A legerősebb A-vonalas jelölt, de ma nem engedélyezhető; a 4:4:4 hibajavítás után újramérés kell (AV2-00). Az Ubuntu 0.0.1-es csomag elavult (AVM v15), rendszercsomagként nem használható |
| Nem hivatalos dav2d fork (`afen261/vlc-av2`) | v1.0.0 CCSO-eltérések javítása; egy 10 bites MP4-en bitpontos az AVM-mel | Jel arra, hogy a upstream dav2d nem teljesen v1.0.0-konform |
| rav2d (`stukenov/rav2d`) | Rust port a dav2d-ből, BSD-2-Clause, nagyrészt MI-eszközökkel készült. **v1.2:** 0.4.2 (2026-11-02; a CFA4 0.3.0-t mért: 3/8). Saját közlése szerint: 34/39 AVM-mel kódolt vektora egyezik az `avmdec` md5-tel (az `av2-normative` ágon); a több csempés folyamokat már dekódolja; 4:0:0, 4:2:2, 4:4:4 és resize még függőben; a `disable_loopfilters_across_tiles`-t figyelmen kívül hagyja; a 12 bit nincs ellenőrizve; a többszálúság csak a kimeneti másolásra és a film grainre terjed ki. A workspace egy `rav2d-sys` crate-et is tartalmaz (FFI-orákulum a dav2d-hez), aarch64-en vendorolt NEON-assemblyt hív FFI-n; x86-on tiszta Rust | Továbbra is FFI nélküli A-vonalas jelölt (x86-on) és B-vonalas referencia, de csak a II.3 szerinti v1.0.0-konformancia után; a 0.4.2-t újra kell mérni. Függőségként csak a `rav2d` crate jöhet szóba, a `rav2d-sys` nem |
| SVT-AV2 | Intel bemutatta (IBC 2026); nyilvános kiadást, licencet nem találtunk. **v1.2:** változatlan | Feltételes, később értékelendő |
| FFmpeg | A CFA4 rögzített minimuma 9.0.2 (ADR 0005). **v1.2:** a legújabb kiadás továbbra is a 9.0.2 (fejlesztői ág: `n9.1-dev`); a `master` 2026-10-27-ig (`0706d0a`) nem tartalmaz AV2-dekódert, -enkódert vagy `libdav2d` wrappert | Az FFmpeg AV2-útja csak futásidejű képességvizsgálattal regisztrálható; az 1. bekötési lehetőség (III.) továbbra sem áll rendelkezésre |
| GStreamer | AV2-elem (dav2d/AVM) nem ismert (GStreamer main, 2026-10-29). **v1.2:** a gitlab.freedesktop.org ebből a környezetből nem érhető el, nem ellenőrizhető újra | Csak képességvizsgálat |
| AV2-ISOBMFF (MP4) | Munkacsoport-vázlat (WGD), kiadás és tag nélkül: `av02` sample entry, `av2C`. **v1.2:** 2026 novemberében intenzíven változik: az `av2C` minden réteget leír (#95), egyszerűsödött a codec-paraméter string (#103), egy TU-ból több kép is kimenet lehet (#111), külön fejezet a réteg- és operating point választásról (#109); legutóbbi commit `6a52b93` (2026-11-06) | Olvasás kísérleti jelöléssel, rögzített commit alapján; írás a G6 mögött. A vázlat változásai miatt az MP4-olvasás csak AV2-02-ben, a beolvasáskori legfrissebb commitra rögzítve készülhet |
| Matroska | draft-ietf-cellar-codec-20 (2026-08): nincs `V_AV2`. **v1.2:** az újabb (2026-10-i) revízió ebből a környezetből nem volt letölthető; `V_AV2` regisztrációjáról nem találtunk forrást | MKV-export nem szabványosítható |
| `AOMediaCodec/av2-tools` | `av2_obu_tool` (elemző, JSON), `av2_mux`/`av2_demux` (MP4), BSD-3-Clause-Clear. **v1.2:** HEAD `d36be5a` (2026-11-04): több CVS-es (coded video sequence) mux/demux mintabejegyzésenként saját `av2C`-vel, `OBU_FILM_GRAIN_MODEL` értelmezés; licenc változatlan | Független kereszt-ellenőrző a parserhez és a G6-hoz, rögzített committal |
| Konformancia-tesztbitfolyamok | **v1.2:** nyilvános AOM-tesztkészletet továbbra sem találtunk; az Allegro DVT kereskedelmi AV2-megfelelőségi bitfolyamokat (Sirius) árul | Az AVM-generált korpusz marad az alap (II.3); kereskedelmi készlet csak tulajdonosi döntéssel, a repón kívül tárolva |
| Hardver | Csak IP-magok bejelentve (Allegro DVT, Chips&Media, VeriSilicon) | Hardveres gyorsítás P3 |
| Szabadalmak | AOM jogdíjmentes licenc (AOM Patent License 1.0); a Sisvel szabadalmi poolt jelentett be. **v1.2:** az Access Advance 2026-08-25-én feltáró szakaszt indított egy AV1/AV2 eszköz- és szoftverpoolhoz, és az AV2-t 2027-ben a Video Distribution poolba is fel kívánja venni | A LIC-03 szabadalmi vizsgálat mindkét poolt kezelje |

### II.2 A v1.0.0 specifikáció kötelező elemei

| Téma | A specifikáció szerint | Következmény a tervre |
|---|---|---|
| Profilok (A.2, Table A.1) | `Main_420_10_IP0/IP1/IP2`, `Main_422_10_IP1`, `Main_444_10_IP1`, `Configurable` (31); mind 8 vagy 10 bit; egyetlen toolset (Main), amely minden coding toolt tartalmaz | 12 bit a v1.0.0-ban nincs; részleges eszközkészletű profil nincs, konform dekóderhez minden coding tool kell |
| Interoperability point (Table A.3) | Az IP0 is 1–4 extended layert (multistream) enged; embedded rétegek az IP1-től | A multistream az IP0-konformanciához nem halasztható |
| Kimeneti konformancia (7.1, A.5) | Film grain nélkül bitpontos kimenet; grainnel vagy bitpontos a 7.21.7 szintézissel, vagy bitpontos köztes kép (7.21.2) + perceptuálisan egyező grain; a level X.Y dekóder minden ilyen szintű bitfolyamot dekódol | A CFA4 az első opciót választja; két hash képkockánként |
| Hibatűrés (Annex C) | Nem kulcsképből indítás informatív ajánlás | A seek kulcsképre épül; az Annex C opcionális |
| Raw formátum (Annex B) | Length-delimited OBU-folyam (leb128 + OBU) | „Raw AV2” = Annex B; az IVF csak fejlesztői formátum |
| HDR-metaadat (6.16.6) | A mastering display szemantikája eltér az AV1-től (SMPTE ST 2086 egész egységek) | Az AV1-es `ColorInfo` konverzió nem vehető át változatlanul |

A specifikáció konténerleképezést és konformancia-tesztbitfolyamokat nem tartalmaz; a hivatalos tesztkészlet megléte az AV2-00 nyitott kérdése.

### II.3 A konformancia definíciója (G1, G4, G5)

- **Orákulum:** AVM v1.0.0 `avmdec` (`966a7d7`), rögzített forráscommittal, a CFA4 saját prefixébe fordítva.
- **Kereszt-ellenőrzés (v1.2):** a korpuszt az AVM `av2-normative` ágának rögzített commitjával (jelenleg `14f43e4`) is dekódolni kell. Az ág a v1.0.0 utáni konformancia-ellenőrzéseket és robusztussági javításokat gyűjti. Ha a két `avmdec` kimenete egy konform vektoron eltér, az eltérést dokumentálni kell, és a vektor addig nem számít bele a „konform” eredménybe. Ha az ág egy vektort nem konform bitfolyamként elutasít, a vektor kikerül a korpuszból. Az AVM `main` ága nem orákulum, mert v1.0.0-n túli képességeket (pl. 12 bites profil) is tartalmaz.
- **Korpusz:** rögzített, manifestben nyilvántartott bitfolyamok (az av2.aomedia.org tesztstreamjei, ha elérhetők, valamint AVM-mel generált vektorok), coding tool lefedettségi mátrixszal (melyik tool melyik vektorban szerepel).
- **Összevetés:** képkockánként két hash: a köztes kép (7.21.2) és a film grain szintézis utáni kimenet (7.21.7), valamint a kimeneti sorrend.
- **Hatókör:** profil, interoperability point és szint (`seq_level_idx`) szerint rögzítve. „Konform” címke csak a teljes vállalt profil/IP/szint kombinációra adható.
- **Egystreamű első cél:** az első szállított szint a `Main_420_10_IP0` egystreamű része. Multistream vagy nem támogatott operating point esetén a dekóder operating point kiválasztást végez (Annex F), vagy egyértelmű `unsupported` hibát ad; ilyenkor „konform” címke nem adható.
- A kimeneti hash-ek az AV1-es konformanciatesztek mintájára a `crates/testkit` segítségével készülnek; médiafájl nem kerül a repóba (`target/fixtures/`).

### II.4 AV2-00 mérési eredmények (v1.2)

Az AV2-00 audit ([`docs/audit/AV2-00-AUDIT-2026-10-30.md`](../audit/AV2-00-AUDIT-2026-10-30.md), PR #341) és a dav2d-mérés (PR #635, 2026-11-05; a beolvasztása még nyitott) eredménye a `scripts/dev/av2-00-audit.py` harness 8 vektoros korpuszán, AVM v1.0.0 (`966a7d7`) orákulummal, Ubuntu 24.04-en:

| Dekóder | Bitpontos vektorok | Hibák | Állapot 2026-11-07-én |
|---|---|---|---|
| dav2d HEAD `326a357` (2026-10-30) | 7/8 | 4:4:4: SIGSEGV, `--threads 1 --cpumask 0` mellett is | Upstream javítás a 4:2:2/4:4:4 szintaxisra (`2f7c315`); újramérés kell (HEAD `1ff0679`) |
| dav2d 0.0.1 `0430370` | 2/8 | 4:4:4 és 4:0:0 nem értelmezhető (AVM v15-alapú) | Elavult, nem jelölt |
| rav2d 0.3.0 | 3/8 | intra-only, 2×2 csempe, 4:4:4, 4:0:0: „invalid or corrupt bitstream data”; páratlan méret: minden képkocka eltér | Azóta 0.4.2 (csempék, intra-javítások a saját közlése szerint); újramérés kell |
| FFmpeg 9.0.2 és `master` | – | AV2 nincs a forrásban | Változatlan |
| GStreamer `main` | – | AV2-elem nincs | Nem ellenőrizhető újra |

**Következtetés:** ma egyik jelölt sem engedélyezhető A-vonalas backendként. A dav2d a legerősebb jelölt, de egy reprodukálható crash miatt a folyamaton belüli bekötés (III., 2. és 4. lehetőség) csak a javítás igazolása és fuzzing után jöhet szóba. Addig, ha a fejlesztés elindul, a dav2d csak külön folyamatként (III., 3. lehetőség) használható fejlesztői célra. A G0 nyitott marad: hiányzik az Ubuntu 26.04-es futtatás, a LIC-03 értékelés és a dav2d HEAD / rav2d 0.4.2 újramérése.

**A korpusz bővítése:** a 8 vektor nem fedi le a v1.0.0 minden eszközét. Az AV2-00 lezárásához a harness kapjon 4:2:2 (`Main_422_10_IP1`), 10 bites 4:4:4, resize/skálázott referencia, `disable_loopfilters_across_tiles`, switch frame és multistream (IP0, 1–4 extended layer) vektort is, coding tool lefedettségi mátrixszal.

## III. Megvalósítási stratégia

### A vonal – Korai AV2-használhatóság

**Cél:** működő dekódolási és szerkesztési lehetőség a saját Rust-dekóder teljes elkészülése előtt.

| Technológia | Funkció | Tervezett szerep |
|---|---|---|
| AVM v1.0.0 | Referencia-enkóder és -dekóder | Kötelező tesztorákulum, fejlesztői prototípus |
| dav2d | Optimalizált CPU-dekóder | Elsődleges jelölt a korai lejátszáshoz, sikeres konformancia után (II.4: ma 7/8, 4:4:4 crash) |
| rav2d | Rust CPU-dekóder (dav2d-port) | FFI nélküli jelölt, sikeres konformancia után (II.4: a 0.3.0 3/8, a 0.4.2 újramérendő) |
| SVT-AV2 | Optimalizált szoftveres enkóder | Feltételes exportbackend, ha nyilvános és licencelhető |
| FFmpeg 9.0.2+ (CFA4 build, ADR 0005) | Kiegészítő médiamotor | Csak futásidejű vizsgálattal igazolt AV2-képességgel |
| GStreamer | Kiegészítő multimédia-backend | Csak elérhető és ellenőrzött AV2-útvonallal |

**A dav2d bekötési módja – architekturális döntés szükséges (ADR).** A dav2d C-könyvtár; a CFA4 szabályai szerint `unsafe` csak a `crates/platform`-ban megengedett (ADR 0001), kiegészítő háttér pedig csak az ADR 0005 szerint. A lehetőségek:

1. **FFmpeg-en keresztül:** ha a CFA4 saját FFmpeg-buildje (`/opt/cfa4`) `libdav2d` wrappert kap, a dav2d az ADR 0005 keretében, új FFI nélkül érhető el. Ehhez az FFmpeg-be kerülő AV2-támogatás kell, ami ma nincs (az FFmpeg `master` 2026-10-27-én sem tartalmazza).
2. **Saját FFI a dav2d-hez:** az ADR 0001 kivétele csak az operációs rendszer hardveres médiaAPI-jaira szól, a szoftveres kodek FFI-je ezért **nem fér bele**. Ez az út csak az ADR 0001 tulajdonos által jóváhagyott módosításával vagy egy új ADR-rel engedélyezhető, vendorolt, commitra rögzített dav2d-vel és ASan/UBSan builddel; a többi lehetőség ezt a policyváltozást nem igényli.
3. **Külön folyamat:** a dekóder önálló segédprogramként fut, a motor pipe-on kap képkockákat. A hibák elszigetelése a legjobb, a teljesítmény a leggyengébb. Fejlesztői AVM-útnak ez az alapértelmezés, és a II.4 szerinti dav2d-crash miatt a dav2d-nek is ez az egyetlen út, amíg a javítás nincs igazolva.
4. **Rust crate-függőség (rav2d):** BSD-2-Clause, GPL-3-mal kompatibilis, x86-on C-FFI nem kell (aarch64-en vendorolt NEON-assemblyt hív FFI-n, ami a CFA4 amd64-célját nem érinti; a `rav2d-sys` crate csak a rav2d saját tesztorákuluma, függőségként nem vehető fel). Az `unsafe` kódja miatt a crate csak külső függőségként, `cargo deny` alatt jöhet szóba, a CFA4 saját crate-jeibe nem másolható `unsafe` résszel. Feltétel: a II.3 szerinti konformancia az AVM v1.0.0 tagen, rögzített commit, és fuzzing a CFA4 korpuszán. Ha ez teljesül, ez a legkisebb architekturális kockázatú A-vonalas út.

Az ADR az AV2-00 kimenete; a XII. fejezet szerint jóváhagyandó.

**A korai AV2-dekódolás bevezetési feltételei:**

- A kiválasztott implementáció a II.3 szerint konform a vállalt profil/IP/szint kombinációra.
- Pontosan rögzített verzió és forráscommit.
- GPL-3.0-kompatibilis licenc (`cargo deny`, `docs/licensing.md`), szerzői jogi és a LIC-03 szerinti szabadalmi ellenőrzés.
- Ubuntu 26.04/amd64 build és működő dekódolási teszt.
- A CFA4 meglévő `MediaSource` (`crates/media`) és `VideoDecoder` (`crates/codecs`) interfészeivel való kompatibilitás.
- Az ADR szerinti elkülönített, biztonságos natív illesztés.
- A `cfa4-route` által kiválasztott backend; a hívó kód közvetlenül nem hívja.
- Nem ír felül rendszerszintű FFmpeg-, GStreamer- vagy más könyvtárat.

Ha a dav2d nem felel meg, a tesztek kizárólag fejlesztői környezetben, az AVM referencia-dekóderrel folytatódnak, és a kiadási szint AV2-LAB marad. A dav2d-ben talált eltéréseket a CFA4 upstream hibajegyként jelzi. Hibás vagy részlegesen kompatibilis dekóder nem kerülhet felhasználói kiadásba.

### B vonal – Végleges natív Rust AV2

**Cél:** saját CFA4 AV2-dekóder és -enkóder az új `crates/av2/` (`cfa4-av2`) crate-ben, L0 rétegben (csak `cfa4-bitstream` függőség), `unsafe` nélkül.

A fejlesztés alapja az AV2 v1.0.0 nyilvános szabványa. A `cfa4-av1`-ből elsősorban az infrastruktúra vehető át (bitolvasó, frame-pufferek, szálkezelés, tesztkeret); a normatív algoritmusokat a v1.0.0 szintaxisa és szemantikája szerint kell megvalósítani, mert az AV2 új eszközöket hoz (például CCSO). AGPL-licencű AV2-implementációból (például `framefinery-engine`) kód nem vehető át; ezek csak összehasonlító referenciaként használhatók, külső folyamatként. A BSD-2-Clause licencű rav2d kódja a `docs/licensing.md` szerint (fejléc és reused-code register) átvehető, de csak `unsafe` nélkül átírva, és minden átvett részt a v1.0.0 AVM-orákulummal újra kell ellenőrizni, mert a rav2d a dav2d-t és az `av2-normative` ágat követi.

A B vonal nem függhet a dav2d elérhetőségétől. A két vonal közös CFA4-interfészeket, de önálló teszteket kap.

**Felülvizsgálati pont:** a parser, az entrópiadekódolás és az intra-only dekódolás konformanciája után a tulajdonos dönt a B vonal folytatásáról, ütemezéséről és erőforrásáról.

### A két vonal összekapcsolásának szabálya

A korai backend és a natív Rust AV2 nem versengő projektek. A kezdeti útválasztás a ténylegesen használható, konform dekódert részesíti előnyben. A natív Rust-dekóder elsődlegessége csak akkor aktiválható, ha konformanciája, stabilitása és teljesítménye mérés alapján bizonyított (ADR 0005: sorrendváltozás csak teszttel és méréssel).

Bármelyik backend hiányakor a routernek másik alkalmas implementációt kell keresnie, vagy egyértelmű `unsupported` hibát kell adnia. Nem használhat téves dekódert AV2 helyett (különösen AV1-dekódert).

---

## IV. CFA4 modularchitektúra

| CFA4 modul | AV2-fejlesztési feladat |
|---|---|
| `crates/av2/` | Natív OBU-parser, dekóder, enkóder (L0) |
| `crates/bitstream/` | Közös ellenőrzött bitfolyam-műveletek (leb128, OBU-keretezés) |
| `crates/codecs/` | AV2 decoder factory és regisztráció |
| `crates/media/` | AV2 médiaforrás, felismerés (IVF `AV02`, Annex B, MP4 `av02`), cache |
| `crates/frame/` | Bitmélység, képkockák, időbélyegek |
| `crates/color/` | SDR, HDR, színjellemzők; az AV2-es ST 2086 egységek |
| `crates/route/` | Backendválasztás és visszaállás (`routing.toml` sorok) |
| `crates/platform/` | Natív/FFI-illesztés, csak az ADR szerinti döntés esetén |
| `crates/engine/` | Média-, szerkesztési és parancsintegráció |
| `crates/export/` | AV2 `VideoEncoder` és export (Annex B, majd konténer) |
| `crates/isobmff/` | AV2-ISOBMFF olvasás (kísérleti), később írás |
| `crates/matroska/` | Matroska AV2-leképezés, csak `V_AV2` regisztráció után |
| `crates/project/` | Kodekazonosító a projektfájlban, sémaverzió, „hiányzó backend” állapot |
| `python/cfa4_qt/` | Import, monitor, timeline és export GUI |
| `crates/testkit/` | Orákulum-harness, korpuszmanifest, hash-összevetés |

Az FFmpeg és a GStreamer kiegészítő backend marad. Az AV2 nem indokolhat különálló új médiamotort.

---

## V. Fejlesztési munkacsomagok

### AV2-00 – Specifikáció, függőségek, tesztkörnyezet

**Feladatok**

- AV2 v1.0.0 szabvány és referenciaforrás rögzítése (spec PDF azonosító, AVM commit SHA).
- AVM v1.0.0 build Ubuntu 26.04-en saját prefixbe; `avmenc` → `avmdec` kör; 1080p sebességmérés.
- dav2d (HEAD és 0.0.1) futtatása a korpuszon: bitpontos vektorok aránya, külön a CCSO- és 10 bites esetekre; API és szálkezelés vizsgálata. **Elvégezve (II.4, PR #635);** a 4:4:4 javítás (`2f7c315`) után a dav2d HEAD és a rav2d 0.4.2 újramérése szükséges.
- Kereszt-ellenőrzés az AVM `av2-normative` ágával (II.3) és a korpusz bővítése (II.4).
- Az av2.aomedia.org tesztstreamjeinek és az `av2-tools` eszközeinek felderítése.
- SVT-AV2 elérhetőségi és licencvizsgálat.
- FFmpeg 9.0.x és GStreamer tényleges AV2-képessége (`ffmpeg -decoders`, `gst-inspect-1.0`), rögzített eredménnyel.
- Profil/IP/szint tábla és coding tool lista a specifikációból.
- Rögzített tesztvektor-katalógus (manifest, hash-ek).
- Kodek- és konténerprofil-mátrix.
- Licencjegyzőkönyv: AVM `LICENSE`/`PATENTS`, dav2d `COPYING`, `av2-tools`; az AOM szabadalmi licenc és a GPL-3 viszonyának írásos értékelése; a Sisvel- és az Access Advance-pool követése (LIC-03).
- ADR-javaslat a dav2d bekötési módjáról (III. fejezet).

**Kimenet:** ellenőrzött technológiai alap, konformanciamátrix, licencjegyzőkönyv, ADR-javaslat.

**Kapu:** G0 – referenciaeszközök, forrásverziók és licencadatok VERIFIED.

### AV2-01 – Korai dekódolóbackend

**Feladatok**

- Az ADR szerinti dekóderadapter prototípusa (AVM fejlesztői adapter, majd dav2d).
- Rust motor és külső dekóder közti adatátadás (`VideoFrame`, PTS, `ColorInfo`).
- CPU-s dekódolási útvonal, 8/10 bit, a `Main_420_10_IP0` egystreamű részére.
- Operating point kiválasztás; nem támogatott multistream/réteg esetén `unsupported`.
- Sérült bitfolyamok kezelése; fuzzing és ASan/UBSan a natív illesztésre.
- Többszálú futtatás és megszakíthatóság.
- Alap teljesítménymérés (1080p/4K dekódolási fps, seek latencia).
- Backendregisztráció a routerben, csak G1 PASS után.

**Kimenet:** működő, ellenőrzött AV2-dekódolás a CFA4 motorján keresztül.

**Kapu:** G1 – dekódolás, hibatűrés, alapmérés és a II.3 szerinti konformancia PASS.

### AV2-02 – AV2-import és médiafeldolgozás

**Feladatok**

- Nyers AV2-bitfolyam támogatása: Annex B (normatív) és IVF (fejlesztői), „nem szabványos konténer” jelöléssel.
- MP4 (`av02`/`av2C`) olvasás kísérleti jelöléssel, az AV2-ISOBMFF vázlat rögzített commitja alapján.
- Profil, IP, szint, bitmélység és felbontás felismerése (a natív parserrel, backend nélkül is).
- Képkockasorrend és időbélyegek kezelése.
- Kulcsképre épülő véletlen hozzáférés.
- Cache és memóriahatárok.
- Médiaazonosítás a projektben; projektfájl-séma és visszafelé kompatibilis olvasás.
- Hibás vagy nem támogatott bitfolyam jelentése.

**Kimenet:** AV2 médiaforrás kezelése a CFA4 meglévő motorjában.

**Kapu:** G2 – import, dekódolás, seek, projektadat-kezelés PASS.

### AV2-03 – Qt6 előnézet és szerkesztési integráció

**Feladatok**

- AV2 felismerése a Media Browserben; forrásadatok és támogatottsági állapot (konform / kísérleti / nem támogatott).
- Source Monitor, Program Monitor, lejátszás, seek, frame-step, kulcskockára ugrás.
- A Qt6 GUI-ban már meglévő, kodekfüggetlen timeline- és szerkesztőfunkciók AV2-anyaggal (az AV2 nem hoz új szerkesztőfunkciót).
- Projektmentés, újranyitás.
- Proxykészítés a meglévő proxykodekkel és proxy/full-quality váltás; a G1 mérése alapján kötelező, ha a dekódolás nem valós idejű.

**Kimenet:** szerkeszthető AV2-média a CFA4 Qt6 GUI-jában.

**Kapu:** G3 – Qt6 funkcionális és regressziós tesztek PASS.

Ez az első kiemelt, felhasználói szempontból jelentős mérföldkő.

### AV2-04 – Natív Rust bitfolyam és dekóder

**Alrendszerek** (mindegyik után hash-összevetés az AVM-mel az adott eszközt használó vektorokon)

1. OBU-parser, sequence és frame header, operating point.
2. Szimbólum- és entrópiadekódolás.
3. Intra predikció.
4. Inter predikció.
5. Referenciaképek és mozgásvektorok.
6. Inverz kvantálás.
7. Inverz transzformációk.
8. Rekonstrukció.
9. Normatív képszűrők (beleértve a CCSO-t).
10. Film grain szintézis (7.21.7).
11. Szín- és időzítési metaadatok.
12. Multistream és operating point kezelés az IP0-hoz.
13. Többszálú dekódolás.

**Kimenet:** önálló Rust AV2-dekóder a vállalt profil/IP/szint kombinációra.

**Kapu:** G4 – a II.3 szerinti képkocka- és mintapontos összehasonlítás az AVM-mel.

### AV2-05 – Natív Rust AV2-enkóder és headless export

**Alapváltozat**

- Konformáns intra kódolás, majd alapvető inter kódolás.
- Referenciaképek, kvantálás, transzformációk.
- Annex B kimenet; `VideoEncoder` bekötés; CLI export.
- Determinisztikus encode/decode ellenőrzés az AVM-mel.

**Használható exportváltozat**

- 8/10 bites kódolás a vállalt profilokban.
- CRF/minőségvezérelt működés; CBR/VBR, amikor megvalósult és igazolt.
- Sebesség/minőség presetek, párhuzamos feldolgozás.
- HDR-metaadatok (ST 2086 egységek).
- Progress/cancel, hibák biztonságos kezelése.

**Optimalizált változat**

- Fejlettebb mozgáskeresés, rate-distortion optimalizáció, adaptív kvantálás.
- Párhuzamos tile/képkockafeldolgozás, ahol szabványosan megengedett.
- SIMD és memóriakezelési optimalizáció; NUMA-tudatos skálázás, ha van tesztgép.

**Korai exportút:** az AVM-enkóder túl lassú felhasználói exporthoz. Amíg a natív enkóder nem éri el a használható szintet, felhasználói AV2-export csak akkor lehetséges, ha az SVT-AV2 nyilvános, licencelhető és G5-konform; ennek hiányában az AV2-export a natív enkóderre vár.

**Kimenet:** saját Rust AV2-enkóder a CFA4 `VideoEncoder` rendszeréhez kapcsolva.

**Kapu:** G5 – AVM v1.0.0-val dekódolható kimenet, eredményösszehasonlítás és stabil export.

### AV2-06 – Konténeres kompatibilitás

**Kötelező vizsgálatok**

- AV2-ISOBMFF állapota (`av02`, `av2C`, az in-band `av2I` javaslat sorsa).
- Matroska `V_AV2` codec ID és CodecPrivate állapota.
- Konfigurációs rekordok, időbélyegek, képkockasorrend.
- Szín- és HDR-metaadatok.
- Multitrack és hangsáv-együttműködés.
- Seek és random-access információk.
- Fájlcsere más implementációkkal (`av2_mux`/`av2_demux`, VLC-AV2, később FFmpeg).

**Szabály:** AV1 sample entry vagy codec ID AV2-höz nem használható, és nem szabványos konténer nem tüntethető fel szabványos MP4/MKV exportként. Az MP4 mux az AV2-ISOBMFF kiadásáig csak kísérleti jelöléssel érhető el, az MKV mux csak a `V_AV2` regisztrációja után.

Az Annex B raw export korábban elkészülhet, mint a konténeres export.

**Kapu:** G6 – független eszközzel ellenőrzött mux/demux/round-trip PASS.

### AV2-07 – Qt6 exportfelület

**Felhasználói paraméterek**

- AV2 videokodek kiválasztása; elérhető profil és bitmélység.
- Felbontás és képkockasebesség.
- CRF/minőség; bitráta, ha támogatott; sebességpreset.
- Színmélység és HDR.
- Szabványos, ellenőrzött célkonténer (vagy Annex B, jelölten).
- Enkóderbackend és futtatási állapot.
- Exportelőrehaladás, megszakítás, hibaüzenet.

Nem támogatott kombinációk nem válhatnak választhatóvá.

**Kapu:** G7 – GUI és export végponttól végpontig PASS.

### AV2-08 – Teljesítményoptimalizálás

Mérési kategóriák: 720p, 1080p, 4K; 24/25/30/50/60 fps; 8/10 bit; SDR és HDR; 1/4/8/16/32 szál; memória; dekódolási késleltetés; random seek latency; első megjeleníthető képkocka ideje; encode FPS; bitráta/minőség összevetés AV1-gyel; hosszú videó.

A valós idejű képesség feltétele: az adott hardver és felbontás mellett a dekódolási sebesség tartósan érje el a forrás képkockasebességét, megfelelő késleltetési és memóriatartalékkal. Ha nem teljesül, kötelező a proxy- vagy csökkentett előnézeti útvonal (az alapmérés már a G1 része).

**Kapu:** G8 – reprodukálható teljesítménymérés, dokumentált korlátokkal.

### AV2-09 – Fejlettebb profilok és jövőbeli képességek

- `Main_422_10_IP1`, `Main_444_10_IP1`, `Configurable` profil.
- IP1/IP2: többrétegű (embedded) kódolás, többnézetes és atlasz-alapú videó (Annex D, G).
- Multistream kompozíció a megjelenítéshez.
- AR/VR és immerzív képanyagok.
- Nagy képfrissítési sebesség, képernyőtartalomra optimalizált kódolás.
- Valós idejű kódolás, ha műszakilag megvalósítható.
- Hardveres dekóder- és enkóderadapterek, gyártófüggetlenül.
- Későbbi toolsetek és bitmélységek, ha a szabvány bevezeti őket (a v1.0.0-ban 10 bit a maximum). Az AVM `main` már egy 12 bites profilt (`Main_444C_12_IP2`) tartalmaz egy még nem nyilvános specifikációs vázlat alapján; a CFA4 csak a nyilvánosan kiadott specifikációmódosítás és a hozzá tartozó AVM-tag után foglalkozik vele.

Ezek nem automatikus részei az első stabil AV2-kiadásnak. Minden új profilt külön konformanciateszt és funkcionális státusz követ.

### AV2-10 – Kiadás és csomagolás

Támogatott rendszer: Ubuntu 26.04 LTS / amd64, a CFA4 által célzott grafikus Ubuntu-változatokon.

Csomagok: `.deb`, Flatpak, AppImage, `tar.gz` fejlesztői/tesztcélra.

Kötelező:

- Backendek és függőségek elkülönítése; a dav2d (ha bekerül) vendorolt, commitra rögzített forrásból, `/opt/cfa4` alá.
- Licencfájlok, forrásazonosítók és notices.
- Rendszerkönyvtárak változatlanul hagyása.
- CPU-only indítás és működés; hiányzó backend biztonságos kezelése.
- Qt6 Wayland/X11 ellenőrzések.
- Telepítés, frissítés és eltávolítás tesztelése.

**Kapu:** G9 – teljes CFA4 kiadási validáció PASS.

---

## VI. Függőségek és párhuzamosítás

Az AV2 a meglévő kodekarchitektúrára épül, és használja a közös képkocka-, bitfolyam-, export- és tesztinfrastruktúrát. A Qt6 továbbra is elsődleges projektprioritás (VISION.md); az AV2 nem hoz létre új egui-funkciót, és nem késlelteti a Qt6 paritást. A dekóder, az enkóder, a konformanciatesztek és a CLI headless módon, párhuzamosan haladhatnak.

```text
AV2-00 Szabvány, technológiai audit, ADR (G0)
                 |
     Közös alap: CodecId, felismerés, cfa4-av2 parser
                 |
       +---------+------------------+
       |                            |
   A VONAL                       B VONAL
   AV2-01 korai backend (G1)     AV2-04 natív dekóder (G4)
       |                            |   [felülvizsgálati pont: intra-only után]
   AV2-02 import + MP4 olvasás   AV2-05 natív enkóder + Annex B export (G5)
       |                            |
   AV2-03 Qt6 előnézet (G3)         |
       |                            |
       +---------+------------------+
                 |
         AV2-06 konténeres export (G6)
                 |
         AV2-07 Qt6 export (G7)
                 |
         AV2-08 optimalizálás (G8)
                 |
         AV2-10 kiadási kapu (G9)

AV2-09: külön fejlettprofil-ág
```

A gráf technikai függőségeket jelent. A PR-ok nem épülhetnek más, még nyitott PR-ágakra; a tényleges sorrendnél az előfeltételeket előbb be kell olvasztani (`docs/coordination.md`).

## VII. Prioritások

| Prioritás | Fejlesztés | Besorolás |
|---|---|---|
| P0 | AV2 v1.0.0 referencia, orákulum és konformanciatesztek | Kötelező |
| P0 | Korai, konform CPU AV2-dekódolás (`Main_420_10_IP0`, egystreamű) | Kötelező (dav2d hiányában AV2-LAB) |
| P0 | Médiaimport, timeline és Qt6 előnézet | Kötelező |
| P1 | Saját natív Rust AV2-dekóder | Kötelező, felülvizsgálati ponttal |
| P1 | Proxy, seek és HDR-kompatibilitás | Kötelező |
| P1 | Multistream/operating point kezelés az IP0-hoz | Kötelező a „konform” címkéhez |
| P1 | Saját natív Rust AV2-enkóder | Kötelező |
| P1 | Konformáns Annex B export | Kötelező |
| P1 | Szabványos konténeres export | Feltételes szabványfüggőség |
| P2 | Kódolási minőség és sebesség optimalizálása | Kötelező a megfelelő kiadási szinthez |
| P2 | SVT-AV2 opcionális integráció | Feltételes |
| P3 | További profilok és interoperability pointok | Későbbi bővítés |
| P3 | Hardveres AV2-gyorsítás | Hardver-/API-függő |

**Az elsődleges cél nem a teljes szabvány egyszerre történő implementálása, hanem egy mielőbb használható, bizonyítottan helyes AV2-dekódolási és szerkesztési lánc.**

## VIII. Tesztelési és konformanciamátrix

| Terület | Előírt ellenőrzés |
|---|---|
| Parser | Valamennyi támogatott OBU-típus; kereszt-ellenőrzés az `av2_obu_tool` kimenetével |
| Dekóder | II.3 szerinti hash-összevetés az AVM-mel |
| Enkóder | AVM által dekódolható, determinisztikus kimenet |
| Film grain | Köztes és kimeneti kép hash-e |
| Bitmélység | 8/10 bit |
| Profil/IP/szint | A vállalt kombinációk; nem támogatottnál `unsupported` |
| HDR | Transfer, primaries, matrix, range, ST 2086 metaadatok |
| Időzítés | CFR, VFR, PTS/DTS és reorder |
| Seek | Kulcsképre épülő pontos véletlen hozzáférés |
| Konténer | Interoperábilis mux/demux független eszközzel |
| Timeline | Vágás, seek, effekt, proxy |
| GUI | Qt6 automatizált és manuális tesztek |
| Stabilitás | Sérült, csonka, túlméretes bemenet; `cargo-fuzz` a parserre; ASan/UBSan a natív illesztésre |
| Memória | Korlátok, cache, hosszú futás |
| Regresszió | Korábbi kodekek és exportok |
| Csomagolás | Négy Linux csomagformátum |
| Licenc | `cargo deny`, függőségek, forrásmegjelölések, reused-code register |
| Kiadás | Teljes CI (`cfa4-full-validation`) az aktuális PR-fejen |

**A tesztbizonyíték tartalma:** git commit SHA; compiler, Qt6 és Rust verziók; Ubuntu-verzió; AV2 implementáció verziója és commitja; tesztvektor vagy fixture azonosítója; parancs, elvárt és tényleges eredmény; PASS / FAIL / SKIPPED / BLOCKED státusz; hardveres mérésnél CPU/GPU-konfiguráció.

`SKIPPED` és `BLOCKED` eredmény nem számítható PASS-ként. A bizonyítékokat a CFA4 meglévő audit- és CI-rendszerében kell tárolni. A korpuszt az AVM lassúsága miatt előre kell generálni és hash-ekkel nyilvántartani; médiafájl nem kerül a repóba.

**Spec-követés:** egy kijelölt felelős havonta ellenőrzi a v1.0.x erratákat (`AOMediaCodec/av2-spec`), az AVM-tageket és az `av2-normative` ágat, a dav2d/rav2d kiadásokat, az `av2-tools`-t és az AV2-ISOBMFF/Matroska változásait, valamint a szabadalmi poolokat. Verzióemelés csak külön PR-ral és a korpusz újrafuttatásával történhet. A v1.2 az első ilyen ellenőrzés eredménye (2026-11-07).

## IX. Kiadási szintek

| Szint | Képesség | Kiadási feltétel |
|---|---|---|
| AV2-LAB | Fejlesztői parser és referenciaeszközök | G0 |
| AV2-DECODE | Ellenőrzött dekódolás | G0–G1 |
| AV2-EDIT | Import, timeline, Qt6 előnézet | G0–G3 |
| AV2-NATIVE | Saját Rust-dekóder | G4 |
| AV2-ENCODE | Natív AV2-kódolás (Annex B) | G5 |
| AV2-CONTAINER | Interoperábilis konténerexport | G6 |
| AV2-STABLE | A meghirdetett funkcióprofilhoz tartozó release | A meghirdetett funkciókhoz tartozó kapuk G0–G9 közül |
| AV2-COMPLETE | A XIII. fejezet szerinti teljes integráció | A XIII. fejezet minden pontja |
| AV2-ADVANCED | További profilok, multistream-kompozíció, hardver | Külön validáció |

Az `AV2-EDIT` és az `AV2-ENCODE` külön is elkészülhet. A teljes felhasználói AV2-exporthoz mindkettő megfelelő integrációja szükséges.

Az `AV2-STABLE` nem jelent minden AV2-profilra és minden konténerre kiterjedő támogatást. A kiadás funkcionális profilját (profil, IP, szint, konténerek) és korlátait pontosan dokumentálni kell.

## X. GitHub fejlesztési végrehajtás

A végrehajtás a [`docs/coordination.md`](../coordination.md) szabályai szerint történik:

1. Új ág friss `main` állapotból.
2. Egy PR egy témát tartalmaz.
3. Nyitott PR-ra nincs ráépítés.
4. Közös fájlok módosítása minimális diff-fel, csak a saját sor hozzáadásával.
5. Minden PR-hoz saját tesztbizonyíték.
6. Teljes CI az aktuális PR fejcommitján.
7. A merge-t kizárólag a kijelölt beolvasztási gazda végzi.
8. A tulajdonos jóváhagyása szükséges a projekt szabályai szerint.
9. Fejlesztési feladat indítása előtt ellenőrizni kell a nyitott PR-ok számát és ütközéseit (20 felett csak a tulajdonos kérésére).
10. A jóváhagyott terv ezen a fájlon (`docs/plans/AV2-CODEC-PLAN-2026-10-29.md`) keresztül a `main` ágon marad meg.

A munkacsomagok AV2-00–AV2-10 azonosítói nem GitHub issue- vagy PR-számok.

**Javasolt első PR-ok a fejlesztésindítási döntés után** (mindegyik friss `main`-ről, egymásra nem építve, a sorszám a beolvasztási sorrend):

| # | Téma | Tartalom | Előfeltétel |
|---|---|---|---|
| 1 | AV2-00 auditjelentés | Verziók, commit SHA-k, licencjegyzőkönyv, konformanciamátrix, profil/IP/szint tábla | – (**kész:** PR #341 beolvasztva; a dav2d-mérés PR #635 nyitott; G0 OPEN) |
| 2 | ADR: AV2 külső dekóder bekötése | rav2d crate, FFmpeg-út, külön folyamat, vagy (ADR 0001 módosításával) saját FFI | 1 |
| 3 | Orákulum-harness | `crates/testkit`: AVM build saját prefixbe, korpuszmanifest; CI-futás csak `workflow_dispatch` vagy `cfa4-full-validation` címke esetén (AGENTS.md), ütemezett futás nélkül | 1 |
| 4 | Kodekazonosító és felismerés | AV2 kodekazonosító, IVF/Annex B/MP4 `av02` sniffing, router `unsupported` válasz, regressziós tesztek | – |
| 5 | `cfa4-av2` váz | OBU- és sequence header parser, operating point, fuzz target | 4 |
| 6 | Korai dekóderadapter | Az ADR szerint, feature flag mögött, hash-egyezés az AVM-mel | 2, 3, 5 |
| 7 | Router-regisztráció | `routing.toml` sor, csak G1 PASS esetén | 6 |

## XI. Kockázatkezelés és megállítási feltételek

| Kockázat | Kezelés |
|---|---|
| A dav2d nem felel meg a v1.0.0-nak | II.3 szerinti konformanciateszt; sikertelenség esetén nincs production backend, AV2-LAB marad, eltérések upstream jelentése |
| A dav2d bekötése új FFI-t igényel | ADR az AV2-00-ban; előnyben a rav2d (Rust crate), az FFmpeg-út vagy a külön folyamat |
| A saját Rust-dekóder túl nagy ráfordítás | Alrendszeres megvalósítás, felülvizsgálati pont az intra-only után |
| Nincs korai exportút | SVT-AV2 feltételes bevonása; addig csak natív enkóder |
| Elégtelen dekódolási teljesítmény | Alapmérés a G1-ben, proxy, szál- és memóriaoptimalizálás |
| A konténeres leképezések nem véglegesek | Annex B kimenet; MP4 kísérleti jelöléssel; MKV csak `V_AV2` után |
| A multistream a vártnál korábban kell | Operating point kiválasztás vagy `unsupported`; konform címke csak teljes IP0 után |
| Hivatalos konformanciakészlet hiánya | AVM-generált korpusz, tool-lefedettségi mátrix; kereskedelmi készlet (Allegro DVT) csak tulajdonosi döntéssel |
| Szabadalmi igény (Sisvel- és Access Advance-pool) | LIC-03 szerinti vizsgálat és kiadási kapu |
| A jelölt dekóder crash-el konform bitfolyamon (dav2d 4:4:4, II.4) | Csak külön folyamatban; folyamaton belül csak javítás, újramérés és fuzzing után |
| Az AVM `main` és a v1.0.0 eltávolodik (431 commit, 12 bites profil) | Orákulum a v1.0.0 tag, kereszt-ellenőrzés az `av2-normative` ággal; a `main` nem orákulum |
| AV2 implementáció vagy hardverdekóder hiánya | CPU-only alapútvonal; nem támogatott hardveres backend nem regisztrálható |
| Qt6 GUI fejlesztési konfliktus | Headless kodek- és tesztfejlesztés; új AV2-funkció csak Qt6 GUI-ra |
| Visszamenőleges kodekregresszió | Állandó regressziós tesztek, módosítatlan publikus API-k, ahol lehetséges |

**Megállítási feltétel:** kritikus konformanciahiba, adatvesztés, reprodukálható crash, megoldatlan licencprobléma vagy sikertelen kötelező kiadási ellenőrzés esetén az érintett funkció nem léphet a következő kiadási szintre.

## XII. Jóváhagyandó döntések

A terv jóváhagyása az alábbi stratégiai döntések elfogadását jelenti:

1. Az AV2 kiemelt CFA4 kodekfejlesztési prioritás.
2. A bevezetés két vonalon történik: korai használhatóság és natív Rust.
3. Az AVM v1.0.0 az elsődleges referencia; a konformancia a II.3 szerint értendő.
4. A dav2d korai dekóderbackendként vizsgálandó, de csak sikeres konformancia- és licencvizsgálat után engedélyezhető; a bekötés módjáról külön ADR dönt.
5. Az első szállított szint a `Main_420_10_IP0` egystreamű része; a „konform” címke a teljes IP0 után jár.
6. A saját Rust AV2-dekóder és -enkóder a végső fejlesztési cél, a B vonalon felülvizsgálati ponttal.
7. A Qt6 marad a GUI fejlesztés elsődleges prioritása.
8. Az AV2-import és előnézet korábban kiadható, mint az AV2-export.
9. Felhasználói AV2-export a natív enkóder vagy egy igazolt SVT-AV2 út elkészültéig nincs.
10. Konténeres export csak szabványos és igazolt interoperabilitással engedélyezhető; MP4 olvasás kísérleti jelöléssel előbb megengedett.
11. A CPU-only támogatás kötelező, a GPU-gyorsítás feltételes.
12. A végrehajtás a CFA4 jelenlegi PR-, licenc-, architektúra- és kiadási szabályait követi.

**A terv jóváhagyása önmagában nem jogosít fel kódmódosításra, ágnyitásra, issue-létrehozásra vagy PR-indításra; kivétel ennek a tervfájlnak a PR-ja, amely a jóváhagyott szöveget a `main` ágon rögzíti. A fejlesztés külön indítási döntéshez kötött.**

Nyitott tulajdonosi kérdések: a licenc- és szabadalmi értékelés felelőse; a spec-követés felelőse; a B vonal felülvizsgálati pontjának időzítése.

## XIII. Végső elfogadási kritérium (AV2-COMPLETE)

A CFA4 AV2-integráció teljesnek akkor tekinthető, ha:

- AV2 média felismerhető és importálható.
- A támogatott bitfolyamok a II.3 szerint konformánsan dekódolhatók.
- Qt6 alatt működik az előnézet, seek, frame-step és szerkesztés.
- A proxy-, HDR- és színkezelési tesztek sikeresek.
- A saját Rust-dekóder konformanciája igazolt.
- A saját Rust-enkóder szabványos kimenetet hoz létre.
- A konténeres export bizonyítottan interoperábilis.
- Az AV1, H.264, HEVC és egyéb meglévő kodekek regressziós tesztjei sikeresek.
- Minden kiadott funkcióhoz megvan a teljes CI- és funkcionális bizonyíték.
- A csomagok telepíthetők és megfelelően működnek Ubuntu 26.04 LTS/amd64 környezetben.
- A felhasználói dokumentáció pontosan ismerteti a támogatott AV2-profilokat, IP-ket, szinteket és korlátokat.

Az AV2-STABLE kiadás ennél szűkebb, meghirdetett funkcióprofillal is megjelenhet (IX. fejezet).

**Végső cél:** a CFA4 önálló, natív Rust-alapú, szabványos AV2 videokodek-támogatása, amely a már működő külső implementációkkal is képes együttműködni, de nem függ kizárólag tőlük.

## XIV. Változásnapló

### XIV.1 Változások a v1.1-hez képest (v1.2, 2026-11-07)

A források újraellenőrzése a VIII. fejezet szerinti spec-követés első köre. A stratégiai döntések (XII.) nem változtak.

| Terület | v1.1 (2026-10-29) | v1.2 (2026-11-07) | Miért |
|---|---|---|---|
| AV2 specifikáció | v1.0.0 | Változatlanul v1.0.0; nincs nyilvános errata vagy v1.0.1 | Az `av2-spec` repóban 2026-05-30 óta nincs commit |
| 12 bites profil | „A v1.0.0-ban 10 bit a maximum” | Új tény: az AVM `main` `Main_444C_12_IP2` profilt tartalmaz egy belső vázlat alapján; AV2-09-ben követendő, csak nyilvános kiadás után | AVM #5112 (2026-08-04) |
| Orákulum | AVM v1.0.0 | v1.0.0 marad; új kereszt-ellenőrzés az `av2-normative` ág rögzített commitjával; a `main` kifejezetten nem orákulum | Az `av2-normative` 9 konformancia-/robusztussági commitot kapott, a `main` 431 committal eltávolodott |
| dav2d | „0.0.1, konformancia folyamatban, v1.0.0-egyezés nincs igazolva” | Mért eredmény: HEAD 7/8 + 4:4:4 crash, 0.0.1 2/8; upstream 4:2:2/4:4:4 javítás után újramérés; a dav2d-nek ma csak a külön folyamat az útja | PR #635 mérése, dav2d `2f7c315` |
| Ubuntu dav2d-csomag | „Ubuntu 26.04-ben nincs csomag” | 0.0.1 a 26.10-ben és a 27.04 fejlesztői ágban van, a 26.04-ben nincs; elavult (AVM v15) | packages.ubuntu.com |
| rav2d | 0.3.0; hiányzik 4:2:2/4:4:4, resize, tiles | 0.4.2: a csempéket már kezeli, 34/39 saját vektor egyezik az `avmdec`-kel; 4:0:0/4:2:2/4:4:4/resize függőben; a `rav2d-sys` és az aarch64 NEON-FFI tisztázva | rav2d README és kiadások (0.4.0–0.4.2, 2026-11-01/02) |
| AV2-00 mérések | Nem szerepeltek | Új II.4 fejezet: eredménytábla, következtetés, korpuszbővítés | Az AV2-00 audit (PR #341) és a dav2d-mérés (PR #635) |
| FFmpeg | 9.0-ban nincs AV2 | 9.0.2 a legújabb kiadás, a `master` 2026-10-27-én sem tartalmaz AV2-t vagy `libdav2d`-t | FFmpeg git |
| GStreamer | Nincs AV2-elem | Változatlan; ebből a környezetből nem volt újraellenőrizhető | gitlab.freedesktop.org nem érhető el |
| AV2-ISOBMFF | Vázlat, `av2I` javaslat | Továbbra is WGD; novemberi változások: `av2C` minden réteget leír, egyszerűbb codec-paraméter string, több kimeneti kép egy TU-ból, operating point fejezet | av2-isobmff #95, #103, #109, #111 |
| `av2-tools` | Elemző, mux/demux | Több CVS-es mux/demux, `OBU_FILM_GRAIN_MODEL`; licenc változatlan | av2-tools #9, `4f9fe46` |
| Konformanciakészlet | „Megléte nyitott kérdés” | Nyilvános AOM-készlet nincs; kereskedelmi (Allegro DVT) létezik, csak tulajdonosi döntéssel | Allegro DVT Sirius |
| Szabadalmak | Sisvel-pool | + Access Advance AV1/AV2 pool feltáró szakasz (2026-08-25), AV2 a VDP poolban 2027-ben | Access Advance közlemény |
| Kockázatok | – | Új sorok: crash konform bitfolyamon; AVM `main` eltávolodása | A fenti tények |
| Spec-követés | Errata, AVM-tagek, konténerek | + `av2-normative` ág, dav2d/rav2d kiadások, `av2-tools`, szabadalmi poolok | A v1.2 tapasztalata |
| Első PR-ok (X.) | 1. AV2-00 auditjelentés | Az 1. pont kész (PR #341), a dav2d-mérés PR #635 nyitott, G0 OPEN | A `main` állapota |

### XIV.2 Változások a v1.0-hoz képest (v1.1, 2026-10-29)

| Terület | v1.0 | v1.1 |
|---|---|---|
| Tényállás, szabvány | Nem szerepelt | II. fejezet: ökoszisztéma, a v1.0.0 kötelező elemei, konformancia-definíció |
| FFmpeg | „FFmpeg ≥ 9.0.2” mint AV2-jelölt | A 9.0.2 a CFA4 rögzített minimuma; AV2-képesség csak futásidejű vizsgálattal |
| Profilok | „8/10 bit, igazolt profilokhoz kötve” | Megnevezett profilok, IP-k, szintek; első cél `Main_420_10_IP0` egystreamű |
| Multistream | P3 | Az IP0-konformanciához P1; addig operating point kiválasztás vagy `unsupported` |
| Film grain | „ahol a profil megköveteli” | Bitpontos összevetés a szintézissel, két hash |
| Raw formátum | „raw bitstream” | Annex B normatív, IVF fejlesztői |
| dav2d bekötése | „elkülönített natív illesztés” | ADR szükséges (FFmpeg-út, külön folyamat, ADR 0001 módosítását igénylő saját FFI vagy a Rust rav2d crate) |
| Konténer | AV2-06 a két vonal után | MP4 olvasás előbb, kísérleti jelöléssel; írás a G6 mögött |
| Export | Nem volt korai út | Kimondva: natív enkóder vagy igazolt SVT-AV2 |
| Teljesítmény | Csak a G8-ban | Alapmérés már a G1-ben |
| Proxy | Kodek nem volt megnevezve | Meglévő proxykodek, nem AV2 |
| Kiadási szintek | AV2-STABLE és XIII. ellentmondott | Új AV2-COMPLETE szint a XIII.-hoz |
| Tervfájl PR | Ellentmondott a IX/10 és a XII. | A tervfájl PR-ja kivétel |
| Stabilitás | Sérült bemenet | Fuzzing, sanitizerek |
| B vonal | Nem volt döntési pont | Felülvizsgálati pont az intra-only után |
| Licenc | Általános | `av2-tools` BSD-3-Clause-Clear; AGPL-implementációk (pl. `framefinery-engine`) kódja tiltott |

## XV. Források

- AV2 specifikáció v1.0.0: <https://av2.aomedia.org/>, <https://github.com/AOMediaCodec/av2-spec>
- AVM v1.0.0: <https://github.com/AOMediaCodec/avm/tree/v1.0.0>
- AV2-ISOBMFF: <https://github.com/AOMediaCodec/av2-isobmff>, in-band javaslat: <https://github.com/AOMediaCodec/av2-isobmff/issues/100>
- AV2 eszközök: <https://github.com/AOMediaCodec/av2-tools>
- Matroska codec draft: <https://www.ietf.org/archive/id/draft-ietf-cellar-codec-20.html>
- dav2d bejelentés: <https://jbkempf.com/blog/2026/dav2d/>
- dav2d 0.0.1: <https://linuxiac.com/videolan-releases-dav2d-0-0-1-as-early-preview-av2-decoder/>
- Nem hivatalos VLC/dav2d fork: <https://github.com/afen261/vlc-av2>
- AOMedia sajtóközlemény (2026-10-22): <https://aomedia.org/press%20releases/AOMedia-Rapid-AV2-Adoption-Across-the-Video-Ecosystem/>
- FFmpeg 9.0: <https://jbkempf.com/blog/2026/ffmpeg-9.0/>
- AV2 az FFmpegben (2026-10-14-i állapot): <https://www.forasoft.com/learn/video-encoding/articles/future-codecs-av2-neural-end-to-end>
- rav2d (Rust AV2-dekóder, BSD-2-Clause): <https://github.com/stukenov/rav2d>
- Rust AV2-enkóder demók (csak referencia): <https://github.com/rafaelcaricio/av2_demo>, <https://github.com/framefinery/framefinery-engine> (AGPL-3.0)
- Áttekintés: <https://en.wikipedia.org/wiki/AV2>

**A v1.2 újraellenőrzésének forrásai (2026-11-07):**

- AVM `av2-normative` ág (`14f43e4`) és `main` (`7a0c956`), 12 bites profil: <https://github.com/AOMediaCodec/avm/pull/5112>
- dav2d forrás (HEAD `1ff0679`, tagek 0.0.0 és 0.0.1): <https://code.videolan.org/videolan/dav2d>
- dav2d Ubuntu-csomagok: <https://packages.ubuntu.com/search?keywords=dav2d&searchon=names&suite=all&section=all>
- rav2d 0.4.2: <https://github.com/stukenov/rav2d>
- AV2-ISOBMFF (`6a52b93`): <https://github.com/AOMediaCodec/av2-isobmff>
- `av2-tools` (`d36be5a`): <https://github.com/AOMediaCodec/av2-tools>
- FFmpeg forrás (`master` `0706d0a`, tagek `n9.0.2`, `n9.1-dev`): <https://github.com/FFmpeg/FFmpeg>
- Access Advance AV1/AV2 pool: <https://accessadvance.com/2026/08/25/access-advance-launches-exploration-phase-for-an-av1-av2-device-and-software-patent-pool-invites-participation-of-potential-licensors/>
- Allegro DVT AV2 megfelelőségi bitfolyamok: <https://www.allegrodvt.com/?p=7947>
- CFA4 mérések: [`docs/audit/AV2-00-AUDIT-2026-10-30.md`](../audit/AV2-00-AUDIT-2026-10-30.md), PR #635

Ebből a környezetből nem volt elérhető: av2.aomedia.org, datatracker.ietf.org (Matroska), gitlab.freedesktop.org (GStreamer). Ezeknél a v1.1 állapota maradt, a táblázat ezt jelzi.
