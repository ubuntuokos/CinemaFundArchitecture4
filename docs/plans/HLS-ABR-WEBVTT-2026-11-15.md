# CFA4 HLS ABR-létra, WebVTT feliratok és alternatív hangsávok – megvalósítási terv

**Dátum:** 2026-11-15  
**Státusz:** a terv tulajdonos által jóváhagyva (2026-11-15), D1–D7 a javaslatok szerint  
**Issue-k:** #230 (ABR-létra, állítható szegmenshossz), #233 (WebVTT feliratok, külön hangsávok)  
**Előzmény:** [`CHAPTERS-DASH-2026-10-30.md`](CHAPTERS-DASH-2026-10-30.md) (D1–D5 kész, utolsó PR #968)  
**Sorrend (tulajdonos, 2026-11-15):** először az A rész (#230), utána a B rész (#233).

## Cél

A HLS export ma egyetlen változatot ír: egy H.264-es képsávot a sorozat méretében, egy AAC-hangsávot
és a fölöttük álló multivariant playlistet, kérésre DASH manifesttel. A terv két lépésben bővíti ezt:

- **A rész (#230):** egy exportból több képváltozat (például 1080p, 720p, 480p és 360p), közös
  hangsávval és egybeeső szegmenshatárokkal, állítható szegmenshosszal, „HLS ABR-létra”
  előbeállítással. A DASH manifest ugyanezt több `Representation`-ként írja le.
- **B rész (#233):** a sorozat feliratsávjai WebVTT-ként (`EXT-X-MEDIA TYPE=SUBTITLES`), és további
  hangváltozatok, például másik nyelv vagy kommentár (`EXT-X-MEDIA TYPE=AUDIO`, ugyanabban a
  csoportban). A DASH manifest ezeket is tartalmazza.

## Kiinduló állapot

| Terület | Ami megvan | Ami hiányzik |
| --- | --- | --- |
| `crates/export/src/hls_out.rs` | `HlsMux`: egy képváltozat és egy hang, CMAF fMP4 (egy sáv fájlonként), vágás az első olyan kulcskockánál, amely után a szegmens elérte a `SEGMENT_SECONDS` = 6 értéket; a fájlnevekben már benne a magasság (`<stem>_<magasság>p_…`) | több képváltozat, állítható szegmenshossz, feliratok, több hangváltozat |
| `crates/export/src/dash_out.rs` | `dash_mpd`: típusonként egy AdaptationSet, abban bármennyi `Representation`, `SegmentTimeline` | szöveges (`text/vtt`) AdaptationSet, nyelv (`lang`) és szerep (`Role`) |
| `crates/export/src/job.rs` | egy renderelés és egy képkódoló (`venc`); a `pipeline::resample` már tud kicsinyíteni | képváltozatonként külön kódoló, a kicsinyített képek továbbadása |
| `cfa4-h264enc` | rögzített `keyint`, `scenecut` alapból bekapcsolva | (a létrához a jelenetváltási kulcskockát ki kell kapcsolni, hogy minden változat ugyanott vágjon) |
| `cfa4-captions` | `vtt::write`, `Document` | szegmensekre bontott WebVTT `X-TIMESTAMP-MAP`-pel |
| Projekt | `CaptionTrack { name, language, … }`; hangsávok és submixek a keverőben | a hangsávoknak nincs nyelvük, és nincs „külön hangváltozat” fogalom |
| Felület | Qt exportpanel: HLS formátum és DASH-kapcsoló (`export.video.dashManifest`), CLI `export --dash` | létra, szegmenshossz, felirat és hangváltozat beállítása |

## A rész: ABR-létra (#230)

### Beállítások (`ExportSettings`)

| Mező | Jelentés | Alapérték |
| --- | --- | --- |
| `hls_segment_seconds: Option<u32>` | célszegmenshossz 1–30 másodperc | `None` = 6 (a mai viselkedés) |
| `hls_ladder: Vec<HlsRung>` | a képváltozatok, `HlsRung { height, bitrate_kbps }`; a szélesség a sorozat képarányából jön, páros számra kerekítve | üres = egy változat a sorozat méretében (a mai viselkedés) |

Szabályok:

- A sorozatnál nagyobb változat kimarad (nincs felskálázás). Ha emiatt egy sem marad, a
  legnagyobb a sorozat méretében készül. Azonos magasság kétszer nem szerepelhet: ezt a `validate()`
  hibaként jelzi, nem pánikkal.
- A kulcskocka-távolság legfeljebb a szegmenshossz. Ha a szegmenshossz nem többszöröse, a
  kulcskocka-távolság a szegmenshosszal lesz egyenlő. Így minden szegmens pontosan a célhosszú,
  kivéve az utolsót.
- Létrás exportban a jelenetváltási kulcskocka ki van kapcsolva (`scenecut = false`): a vágási
  pontok csak a képkocka sorszámától függenek, ezért minden változatban ugyanott vannak.
- Kétmenetes VBR létrával egyelőre nem támogatott. A `validate()` ezt érthető üzenettel jelzi
  („Two-pass VBR is not available with an HLS ladder yet”), és az egymenetes VBR, a CBR és a CRF
  működik.

### Előbeállítás: „HLS ABR-létra”

H.264 High, egymenetes VBR, AAC 160 kbit/s, 6 másodperces szegmensek. A bitráták 30 fps-re szólnak,
30 fps fölött 1,5-szeresek:

| Változat | Méret (16:9) | Célbitráta | Max. bitráta |
| --- | --- | --- | --- |
| 1080p | 1920×1080 | 6000 kbit/s | 9000 kbit/s |
| 720p | 1280×720 | 3000 kbit/s | 4500 kbit/s |
| 480p | 854×480 → 852×480 (páros) | 1500 kbit/s | 2250 kbit/s |
| 360p | 640×360 | 800 kbit/s | 1200 kbit/s |

### Renderelés és kódolás

A sorozat egyszer renderelődik, a legnagyobb változat méretében. A kisebb változatok képét a meglévő
`pipeline::resample` készíti (premultiplikált kép, háromszögszűrő), a kódoló színátalakítása előtt.
Minden változat saját kódolót kap a meglévő `video_factories()` útján, így a hardveres kódoló
ugyanúgy használható. A kódolás változatonként párhuzamosan fut a Rayon-készletben. A becsült
többletidő a négylépcsős létránál körülbelül 1,8-szoros a kódolásban, a renderelés nem nő. Ezt a
referenciagépen mérjük, és a PR-ban közöljük.

### Mux (`HlsMux`)

- `HlsMux` N képváltozatot kezel. A vágási pontokat a legnagyobb változat kulcskockái adják; a
  többi változatnál ellenőrizzük, hogy ugyanott kulcskocka van. Ha nem, az belső hiba, és
  `ExportError` lesz belőle, nem pánik.
- A hang egyszer kódolódik, és minden változat ugyanazt az `EXT-X-MEDIA` hangcsoportot használja.
- A fájlnevek a mai sémát követik: `<stem>_<magasság>p.m3u8`, `…_init.mp4`, `…_00000.m4s`.
  Egyváltozatos exportnál a kimenet bájtra azonos a maival (regressziós teszt).
- A multivariant playlistben változatonként egy `EXT-X-STREAM-INF` van, csökkenő sávszélesség
  szerint, `BANDWIDTH` és `AVERAGE-BANDWIDTH` értékkel (kép és hang együtt), valamint `CODECS`,
  `RESOLUTION` és `FRAME-RATE` mezővel.

### DASH

A meglévő `dash_mpd` már egy AdaptationSetbe teszi az összes videós `Representation`-t. A
`HlsMux::dash_manifest` változatonként egy `Representation`-t ad át (`id="video_<magasság>p"`).
A `segmentAlignment="true"` állítás így igaz is marad.

### Felület, CLI, MCP

- Motor és vezérlőcsatorna: `hlsLadder` (magasságok és bitráták listája, vagy `"auto"` az
  előbeállítás létrájához) és `hlsSegmentSeconds`.
- CLI: `export --hls-ladder auto|1080,720,480` és `--segment-seconds N`.
- Qt exportpanel (csak Qt-ban, az egui be van fagyasztva): HLS formátumnál „Változatok” sor
  jelölőnégyzetekkel (1080p, 720p, 480p, 360p; a sorozatnál nagyobbak tiltva) és „Szegmenshossz”
  számmező. Automatizálási azonosítók: `export.video.hlsLadder.<magasság>`,
  `export.video.hlsSegmentSeconds`. Magyar fordítás az `i18n`-ben, teszt a `test_qt_gui.py`-ban.

## B rész: WebVTT feliratok és alternatív hangsávok (#233)

### Feliratok

- Új beállítás: `hls_subtitles: bool`, alapból bekapcsolva, ahogy a fejezeteknél. A sorozat minden
  bekapcsolt (szem ikonos) feliratsávja külön `TYPE=SUBTITLES` változat lesz a `subs` csoportban.
  `NAME` = a sáv neve, `LANGUAGE` = a sáv `language` mezője (ha üres, kimarad), `DEFAULT=YES` csak
  az elsőnél, `AUTOSELECT=YES`. Ha egyik sávot sem lehet felhasználni, a csoport kimarad.
- A feliratok a képpel azonos szegmenshatárokon szegmensekre bomlanak:
  `<stem>_sub_<n>_00000.vtt`, és mindegyik szegmens elején `X-TIMESTAMP-MAP` áll. A határon
  átnyúló felirat mindkét szegmensbe bekerül, ahogy az RFC 8216 §3.5 engedi. A pontos
  `MPEGTS`/`LOCAL` értéket fMP4 mellett az ffmpeg `hls` demuxerével és a feliratidők
  visszaolvasásával igazoljuk.
- A CEA-608/708 sávok szövege is átkerül, a stílus nélkül. A WebVTT STYLE/REGION blokkok minden
  szegmensbe bekerülnek.
- DASH: sávonként egy `contentType="text" mimeType="text/vtt"` AdaptationSet `lang` attribútummal,
  egyetlen teljes `.vtt` fájllal (`<stem>_sub_<n>.vtt`, `BaseURL`). Ezt a dash.js és az ffmpeg is
  olvassa.

### Alternatív hangsávok

- Új beállítás: `hls_audio_renditions: Vec<HlsAudioRendition>`, ahol
  `HlsAudioRendition { name, language, tracks: Vec<TrackId>, default: bool }`.
  A fő hang a teljes keverés marad (`NAME` = „Main”, nyelve `hls_audio_language`). Minden további
  változat csak a kiválasztott hangsávokat keveri, a sávok saját effektjeivel, submix-útvonalával és
  automatizálásával, a többi sáv némítva. Erre az `AudioOut` sávmaszkot kap.
- Mindegyik változat külön AAC-kódolót és külön fMP4-sorozatot kap ugyanazokon a vágási
  pontokon (`<stem>_audio_<n>…`), az `audio` csoportban `DEFAULT=NO` és `AUTOSELECT=YES`
  beállítással. A `STREAM-INF` `BANDWIDTH` értéke a csoport legnagyobb hangváltozatával számol
  (RFC 8216 §4.3.4.2).
- DASH: nyelvenként külön hang-AdaptationSet `lang` attribútummal és
  `Role` elemmel (`main`, `alternate` vagy `commentary`).
- Felület: Qt exportpanel, „Hangváltozatok” táblázat (név, nyelv, sávok), CLI
  `--hls-audio "Commentary:en:A3,A4"`, motorparaméter `hlsAudioRenditions`.

## A CFA4 szabályai

- Nincs pánik: minden új bemenet (létra, nyelvkód, sávazonosító, szegmenshossz) a `validate()`-ben
  hibát ad, és szélsőséges értéknél (0 vagy 10 000 másodperc, 0 magasság, nem létező sáv) sem esik szét.
- Az idő `Tick`-ben és egész időskálákban számolódik. A feliratok szegmenshatárai a kép
  időskálájából jönnek, nem `f64`-ből.
- A rétegek nem változnak: minden a `cfa4-export`-ban, a `cfa4-captions`-ben és a motorban marad.
  A `cfa4-isobmff` nem változik.
- Közös fájlok (`i18n/*.tsv`, `docs/*-parity.md`, `docs/audit/README.md`): csak saját sorok kerülnek
  bele.
- Új külső függőség nincs.

## Tesztelés

| Szint | Mit ellenőriz |
| --- | --- |
| Unit (`hls_out`, `dash_out`) | létra kiszámítása (felskálázás kimarad, páros szélesség, duplikált magasság hiba); multivariant és MPD szövege kézzel írt mintával; WebVTT-szegmensek: határon átnyúló felirat, üres szegmens, `X-TIMESTAMP-MAP` |
| Regresszió | egyváltozatos, felirat és extra hang nélküli export bájtra azonos a mai kimenettel |
| Export + ffprobe | minden változat dekódolható, a méretük helyes; a szegmenshatárok (`tfdt`, `EXTINF`) változatonként azonosak; a `hls` és a `dash` demuxer N videó-, M hang- és K feliratfolyamot lát |
| Feliratidők | az ffmpeg az exportált HLS-ből a feliratokat ±1 képkockán belül a sorozatbeli időkön adja vissza |
| Hangváltozatok | a kommentár-változat csak a kiválasztott sávot szólaltatja meg (a némított sáv tesztjele nincs benne, az FFT-ben nincs csúcs a frekvenciáján) |
| Proptest | véletlen létra, szegmenshossz és feliratidők: nincs pánik, a vágások monotonok |
| Qt | `test_qt_gui.py`: a létra és a szegmenshossz a motorig eljut, és a beállítások mentése és betöltése megőrzi |
| Mérés | a négylépcsős létra exportideje a referenciagépen (a PR-ban) |

## Feladatok

Minden feladat külön PR, friss `main`-ből, a quality gate-ek zöldek, és a végén „Gazda:” issue készül.

| Id | Feladat | Becslés |
| --- | --- | --- |
| H0 | Ez a terv a `main`-re, sor a `docs/audit/README.md`-ben | – |
| A1 | `hls_segment_seconds` és a kulcskocka-igazítás, motor- és CLI-paraméter, tesztek | 2–3 óra |
| A2 | `HlsMux` N képváltozattal és közös hanggal, multivariant playlist, DASH több `Representation`; szintetikus csomagokkal tesztelve | 4–6 óra |
| A3 | Exporter: `hls_ladder`, renderelés egyszer, kicsinyítés és változatonkénti kódoló, `scenecut` kikapcsolva, kétmenetes VBR hibaüzenete; ffprobe-tesztek, mérés | 6–10 óra |
| A4 | „HLS ABR-létra” előbeállítás, CLI `--hls-ladder`, Qt exportpanel és magyar fordítás, dokumentáció (`file-format-parity.md`, `gaps.md` G13) | 4–6 óra |
| B1 | WebVTT szegmentáló, `TYPE=SUBTITLES` csoport, `hls_subtitles`, DASH text AdaptationSet; ffmpeg-orákulum | 5–8 óra |
| B2 | `AudioOut` sávmaszk, `hls_audio_renditions`, több AAC-kódoló, hangcsoport és DASH `lang`/`Role` | 6–10 óra |
| B3 | CLI- és motorparaméterek, Qt felirat-kapcsoló és hangváltozat-táblázat, magyar fordítás, dokumentáció (#233 lezárása) | 4–6 óra |

## Döntések (jóváhagyva 2026-11-15)

| Kérdés | Javaslat |
| --- | --- |
| D1. Az előbeállítás létrája | 1080p, 720p, 480p, 360p a fenti bitrátákkal; a sorozatnál nagyobbak kimaradnak |
| D2. A kisebb változatok képe | egyszeri renderelés és kicsinyítés (változatonkénti renderelés helyett): gyorsabb, és a kép ugyanaz |
| D3. Kétmenetes VBR létrával | egyelőre nem; érthető hibaüzenet, később külön feladat |
| D4. Honnan jön egy alternatív hangváltozat | a sorozat kiválasztott hangsávjaiból, a többi sáv némításával (nem külön sorozatból és nem submixből) |
| D5. A feliratok alapértéke | bekapcsolva: minden látható feliratsáv WebVTT-ként |
| D6. DASH-feliratok | feliratsávonként egy teljes `.vtt` fájl (nem szegmentálva) |
| D7. A változatok sorrendje a multivariant playlistben | csökkenő sávszélesség |
