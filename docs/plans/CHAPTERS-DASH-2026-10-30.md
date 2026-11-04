# CFA4 MP4/MOV/Matroska fejezetjelölők és DASH manifest – megvalósítási terv

**Dátum:** 2026-10-30  
**Státusz:** a terv tulajdonos által jóváhagyva (2026-10-30)  
**Minta:** [muxide](https://github.com/Michael-A-Kuykendall/muxide) v0.5.0 (MIT OR Apache-2.0), csak ötletforrásként; kódot nem veszünk át, függőség nem lesz.

## Cél

A sorozat `MarkerKind::Chapter` markerei fejezetként kerülnek az MP4, MOV, HLS és Matroska/WebM exportba, és importáláskor visszaolvashatók. A HLS export mellé DASH manifest (`.mpd`) is készül ugyanazokból a CMAF-szegmensekből.

A muxide fejezetkódja a bájtformátumhoz nem jó minta. 100 ns-os egységek helyett ezredmásodpercet ír, a darabszám nála 32 bites a 8 bites helyett, és bejegyzésenként 4 felesleges bájtot tesz bele, ezért az ffprobe egyetlen fejezetet sem látna egy ilyen fájlban. A formátumot az ffprobe viselkedéséből vezetjük le, és orákulumként azzal is teszteljük.

## Kiinduló állapot

| Terület | Ami megvan | Ami hiányzik |
| --- | --- | --- |
| `cfa4-isobmff` mux | `Mp4Writer` (fast-start), `FragmentedWriter`, `WriterOptions::metadata` → `udta` ©xxx atomok | fejezet (`chpl`, QuickTime fejezetsáv) írása |
| `cfa4-isobmff` demux | `udta`, `meta/ilst`, `keys` olvasása | `chpl` és `tref/chap` olvasása |
| `cfa4-matroska` | `Chapters` olvasása a demuxban | `Chapters` írása a `MkvWriter`-ben |
| Projekt | `Marker` + `MarkerKind::Chapter` (`crates/project/src/lib.rs`), idő `Tick`-ben | exportba nem jut el |
| Export | `hls_out.rs`: H.264 + AAC CMAF-szegmensek, egy fájlban mindkét sáv, m3u8 lejátszási listák | DASH MPD |

Mellékesen: a `write_udta` (`crates/isobmff/src/mux.rs`) a szöveg hosszát `as u16`-tal csonkolja. Ez sérti az „ellenséges számok” szabályt, és az F1 feladat javítja.

## 1. rész: fejezetjelölők

### Nero `chpl` (a `moov/udta` alatt)

| Mező | Méret | Érték |
| --- | --- | --- |
| verzió + flags | 1 + 3 bájt | 1, 0 |
| fenntartott | 4 bájt | 0 |
| darabszám | 1 bájt | legfeljebb 255 |
| kezdőidő (bejegyzésenként) | 8 bájt | 100 ns-os egységben |
| cím hossza | 1 bájt | legfeljebb 255 |
| cím | változó | UTF-8, karakterhatáron vágva |

API a `cfa4-isobmff`-ben:

- Új típus: `Chapter { start_100ns: u64, title: String }`.
- `WriterOptions::chapters: Vec<Chapter>`. Az `Mp4Writer` és a `FragmentedWriter` is ugyanazzal a `write_udta` hívással írja ki (fragmentált kimenetnél az init szegmensbe).
- A 255-nél több fejezet és a túl hosszú cím nem pánik, hanem vágás. Az író visszaadja, hány fejezet maradt ki, és ezt az export figyelmeztetésként mutatja.
- Demux: `Metadata::chapters`. A `chpl` olvasása minden hosszt a box méretéhez mérve ellenőriz.

### QuickTime fejezetsáv (Apple lejátszók)

A QuickTime és az iOS a `chpl`-t nem olvassa, ezért QuickTime fejezetsáv is készül. Ez egy `text` szövegsáv, amelynek mintái a fejezetcímek, és a videósáv `tref/chap` hivatkozással mutat rá. A sáv le van tiltva (`tkhd` enabled = 0), hogy a lejátszók ne mutassák feliratként. API: `Mp4Writer::add_chapter_track(video, &chapters)`, a meglévő `add_timecode_track` mintájára.

### Matroska és WebM

A `cfa4-matroska` demux már olvassa a `Chapters` elemet, de a `MkvWriter` még nem írja. Ide `MuxOptions::chapters` kerül, amely egy `EditionEntry` alatt `ChapterAtom` elemeket ír (`ChapterTimeStart` ns-ban, `ChapterDisplay/ChapString`). A WebM export ugyanazt a kapcsolót használja.

### Export

- Az `ExportSettings` új kapcsolót kap: „Fejezetek írása a markerekből”, **alapból bekapcsolva**.
- `Tick` → 100 ns: `start × 10⁷ / TICKS_PER_SECOND`, i128-ban számolva, mert i64-ben a szorzat túlcsordulhat. A kiexportált tartományon kívüli markerek kimaradnak, a többi a tartomány elejéhez van eltolva.
- A névtelen marker címe „Fejezet N” (a felhasználói felület nyelvén).

## 2. rész: DASH manifest

A HLS export mellé egy `.mpd` fájl is készül, így ugyanaz a kimenet dash.js-ben és Shaka Playerben is lejátszható.

**Hova kerül:** új `crates/export/src/dash_out.rs`, a `hls_out.rs` mintájára. Az MPD csak szöveg, ezért nem a `cfa4-isobmff`-be (L0) való, hanem az exportba.

**Az MPD felépítése:**

- `type="static"`, profil: `urn:mpeg:dash:profile:isoff-live:2011`.
- `SegmentTemplate` + `SegmentTimeline`, mert a szegmensek kulcsképkockánál vágnak, így nem egyforma hosszúak. A sablon `$Number%05d$` és `startNumber="0"`, így illeszkedik a meglévő, nullától számozott `<n:05>.m4s` nevekre (a puszta `$Number$` `0`-t adna `00000` helyett).
- `codecs` a meglévő `video_codecs()` függvényből, valamint `bandwidth`, `width`, `height` és `frameRate`.
- Az XML-be kerülő fájlneveket escape-eljük (`&`, `<`, `"`).

**Döntés (B út):** a videó és a hang külön szegmensekbe kerül (két `FragmentedWriter`), és ezeket a HLS és a DASH is használja. A hangcsoportot (`EXT-X-MEDIA:TYPE=AUDIO`) a multivariant lejátszási lista deklarálja, a változat `EXT-X-STREAM-INF` sora `AUDIO` attribútummal hivatkozik rá, és a hangszegmensek külön média lejátszási listát kapnak. Média lejátszási listába ez a tag nem kerülhet (RFC 8216 4.3.4). Ez a CMAF gyakorlat, és minden DASH-lejátszó támogatja.

**Beállítás:** a `Format::Hls` kap egy „DASH manifest is” kapcsolót, új formátum helyett. A `hls_files()` mellé `dash_files()` kerül (`<stem>.mpd`). A CLI-ban az `export` ugyanezt a beállítást kapja meg.

## A CFA4 szabályai

- **Soha ne omoljon össze:** nincs `unwrap`, `expect`, `panic!`, és nincs `as`-szal csonkolás. Hosszt és darabszámot `u8::try_from`-mal vagy vágással kezelünk, minden olvasott szám ellenséges. A demux `get()`-tel indexel.
- **Rétegek:** a `cfa4-isobmff` továbbra is csak a `cfa4-bitstream`-től függ, és wasm32-re is fordul. A `cfa4_time::Tick` átváltása az exportban történik.
- **Clean-room és licenc:** sem muxide-, sem FFmpeg-kódot nem másolunk. A formátumot nyilvános leírásból és az ffprobe kimenetéből vesszük. Ha a muxide-ot forrásként megnevezzük, az `ATTRIBUTION.md`-be bejegyzés kerül.
- **Minden parancs:** a fejezet-kapcsoló az exportbeállítások közé kerül, serde-vel, így a CLI, a vezérlőcsatorna és az MCP is eléri.
- **Commitok:** commitonként egy feladat-azonosító, zöld `cargo xtask ci` mellett.

## Tesztelés

| Szint | Mit ellenőriz |
| --- | --- |
| Unit (`cfa4-isobmff`, `cfa4-matroska`) | `chpl` bájtjai egy kézzel írt mintával egyeznek; 256 fejezet és 300 bájtos cím vágása pánik nélkül; több bájtos UTF-8 karakter nem törik ketté |
| Roundtrip + proptest | írás → `open()` → ugyanazok a fejezetek; véletlen bájtok a `chpl`-ben nem okoznak pánikot (a `fuzz.rs` kiegészítése) |
| ffprobe orákulum (`oracle_mux.rs`) | `ffprobe -show_chapters` ugyanazokat a kezdőidőket és címeket adja (`chpl`, QuickTime fejezetsáv, MKV); ffmpeg-gel készített fejezetes fájlt a demux helyesen olvas |
| Export (`crates/export`) | Chapter markerek → fejezetek, a tartományon kívüliek kimaradnak; az MPD szegmensszáma és időtartamai egyeznek az m3u8-éval |
| DASH lejátszás | `ffprobe` megnyitja az `.mpd`-t; kézi próba dash.js-ben (CI-ban nem fut) |

## Feladatok

A két rész egymástól független, így külön PR-ban mehetnek. Az 1. résszel kezdünk. A becslés fejlesztési idő, review nélkül.

**1. rész: fejezetek (kb. 3,5 nap)**

- [ ] F1: `write_udta` hosszcsonkolás javítása, regressziós teszttel
- [ ] F2: `Chapter` típus + `WriterOptions::chapters` + `chpl` írása, unit tesztek
- [ ] F3: `chpl` olvasása a demuxban, roundtrip és fuzz
- [ ] F4: ffprobe orákulum teszt az `oracle_mux.rs`-ben
- [ ] F5: Chapter markerek → export (MP4, MOV, HLS init szegmens), alapból bekapcsolt kapcsoló, `Tick` átváltás
- [ ] F6: QuickTime fejezetsáv (`text` + `tref/chap`) írása és olvasása, ffprobe orákulummal
- [ ] F7: Matroska/WebM `Chapters` írása a `MkvWriter`-ben, roundtrip a meglévő demuxszal, WebM export
- [ ] F8: README a `cfa4-isobmff`-ben és a `cfa4-matroska`-ban, `docs/gaps.md` frissítése

**2. rész: DASH (kb. 2–3 nap)**

- [ ] D1: külön videó- és hangszegmensek a HLS exportban, külön hang média lejátszási listával, a hangcsoport (`EXT-X-MEDIA` + `AUDIO`) a multivariant listában
- [ ] D2: `dash_out.rs`: MPD-író `SegmentTimeline`-nal, unit tesztek
- [ ] D3: „DASH manifest is” kapcsoló, `dash_files()`, CLI
- [ ] D4: export teszt (MPD ↔ m3u8 egyezés) és ffprobe-megnyitás
- [ ] D5: dokumentáció, `docs/gaps.md`

## Döntések

A tulajdonos 2026-10-30-án döntött, nyitott kérdés nem maradt.

| Kérdés | Döntés |
| --- | --- |
| QuickTime fejezetsáv az Apple lejátszóknak | Igen (F6) |
| DASH-szegmensek | B út: külön videó- és hangszegmensek |
| A fejezet-kapcsoló alapértéke | Bekapcsolva |
| Matroska/WebM fejezetek | Igen (F7) |
