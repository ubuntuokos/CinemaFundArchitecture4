# CFA4 – Binaurális (HRTF) előhallgatás objektum- és csatornaalapú hanghoz

**Specifikáció:** `CFA4-BINAURAL-MONITOR-001`
**Dátum:** 2026-11-27
**Verzió:** 1.0 (végleges)
**Állapot:** **Jóváhagyva 2026-11-27 (István, a projektszálban).** A 9. fejezet döntései a javaslatok szerint. A munka a B-00-val indul, a 8. fejezet ütközési szabályaival.
**Módosítva:** 2026-11-27 este: a Maul Audio (MIT) bevonva portolási forrásként (3.1, 4.2, 5., 7.), az OpenSpatialDelay (GPL-3.0) ötletforrásként (4.2), a 9. fejezet döntései rögzítve.
**Kapcsolódik:** `CFA4-SMART-CUT-RENDER-R2` (párhuzamos, ütközés nélkül: 8. fejezet), hiánylista `docs/gaps.md` (#123, #124, #125, #126, #130).

---

## 0. Röviden

Cél: a Qt6 szerkesztőben **fejhallgatón** térben lehessen hallani az immerzív hangot, hangszórórendszer nélkül. Ez érvényes az objektumalapú (Dolby Atmos TrueHD, IAB a DCP-ből, később DTS:X és E-AC-3 JOC objektumok) és a csatornaalapú (5.1, 7.1, 7.1.2, 7.1.4, Auro-3D 9.1–13.1) hangra is. A megoldás **HRTF-alapú binaurális renderelés**, a saját Rust motorban, külső program nélkül. Az előhallgatás a monitor kimenetén fut, az exportot nem változtatja meg. A binaurális sztereó export külön, kifejezetten választható lehetőség.

Ötletforrás a ThreeDeeJay/DolbyAtmosBinauralizer. Ez egy Windows `.cmd` lánc (ffmpeg → truehdd → Cavernize → OpenAL Soft allafplay HRTF → FLAC). Licence nincs, ezért **kódot nem veszünk át**, a láncot sem futtatjuk. Minden lépésének van már saját CFA4-megfelelője, kivéve egyet, a HRTF-renderelést. Ezt pótolja ez a terv.

## 1. Mi van már meg (kódolvasás, 2026-11-27, `main`)

| Elem | Hol | Állapot |
|---|---|---|
| TrueHD/MLP dekódolás, OAMD (objektumpozíció, méret, zóna, snap, rámpák) | `crates/truehd` (truehdd 0.7.2 port, Apache-2.0) | kész. `ObjectDecoder`: objektumonként egy mono jel és `render::Event`-ek, renderelés nélkül |
| OAMD fejhallgató-elem (`oa_element_id` 3, objektumonként binaural render mode) | `crates/truehd/src/structs/oamd.rs` `HeadphoneElement` | beolvasva, de semmi nem használja |
| Atmos → hangszóró renderer (távolságalapú panner, 2.0…7.1.4) | `crates/truehd/src/render.rs` | kész. A forrás kimondja: „binaural nincs modellezve” (#125) |
| IAB dekódolás, renderer (2.0…7.1.4), író | `crates/iab` | kész. A DCP Atmos export (`export/src/dcp_atmos.rs`) is ezt használja |
| DTS-HD MA, DTS:X alap (7.1 bed) és a DTS:X privát metaadat (objektumpozíciók, a bedbe keverés mátrixa) | `crates/dts`, `dcadec/xmeta.rs` | a bed szól. Az objektumok és a magasságok saját helyükön még nem (#130) |
| Auro-3D: magassági csatornák visszaállítása (8.0…13.1) | `crates/auro` | kész, bitpontos hordozónál |
| E-AC-3 + JOC (Atmos) | `crates/ac3` | csak az 5.1/7.1 bed szól. A JOC-ot nem dekódoljuk (#126) |
| Csatornaelrendezések, downmix | `crates/audio-dsp/src/channels.rs` | Mono, 2.0, 5.1, 7.1, 7.1.2. **7.1.4 nincs** a szekvencia-keverőben |
| Partícionált konvolúció (overlap-save), FFT | `audio-dsp/src/effects/reverbs.rs`, `fft.rs` | megvan, a konvolúciós zengetőben |
| „Binauralizer” effekt | `audio-dsp/src/effects/special.rs` | csak gömbfej-modell (Woodworth ITD + Brown–Duda), ±szög sztereóhoz, **HRTF nélkül** |
| Lejátszás a Qt-ban | `python/cfa4_qt/panels/monitor.py` | `audio.render` → interleaved float32 → `QAudioSink`, **mindig sztereó** |

**Következmény:** ma a lejátszás a TrueHD-nak csak a csatornás (7.1) bemutatását dekódolja (`codecs/src/audio.rs`, `Decoder::new()`), az objektumokat nem. A szekvencia keverője legfeljebb 7.1.2-es, a monitor pedig sztereóra hajtogat (BS.775). Térbeli hallgatás jelenleg sehol sincs.

## 2. Hatókör

**Benne van:**

1. HRTF-adatkészlet kezelése: egy beépített alapkészlet, opcionálisan letölthető további készletek, és a felhasználó saját SOFA-fájlja (AES69).
2. Binaurális renderer két módban:
   - **Virtuális hangszórók**: bármely csatornaelrendezés (2.0…7.1.4, Auro 13.1, 22.2) minden csatornája egy HRIR-pár az adott irányból.
   - **Pontos objektum**: minden objektum a saját, időben változó irányából, interpolált HRIR-rel.
3. Közös térbeli jelenetmodell (bed + objektumok + események), illesztőkkel: TrueHD Atmos, IAB, csatornás források, Auro-3D. Később DTS:X objektumok és E-AC-3 JOC.
4. Engine: a monitor kimenetének „Hangszórók / Fejhallgató (binaurális)” módja. A forrásmonitorban az objektumok közvetlenül szólnak, a szekvenciában a végső keverés virtuális hangszórókon keresztül.
5. Qt6: beállítások, monitorkapcsoló, HRTF-választó, SOFA betöltése, mérők, fordítások (`i18n/`).
6. Binaurális sztereó export (WAV/FLAC/AAC) választható kimenetként, és `cfa4-cli` parancsként. Ez a DolbyAtmosBinauralizer saját megfelelője.
7. Validáció valódi mintákon (az öt Altitude16 fájl) és mérés a referenciagépen.

**Nincs benne:**

- A Dolby, DTS vagy Auro saját fejhallgató-rendererének utánzása. Ezek zártak. Az OAMD fejhallgató-elemét csak irányjelzésnek használjuk (near/mid/far/off), saját, dokumentált értelmezéssel.
- Atmos, DTS:X, Auro kódolás. Az objektumhang nem kerül vissza bitstreambe.
- Objektumbusz a keverőben: objektumsáv, objektum-automatizálás, panner a sávon (Fairlight-szerű Atmos-keverés). Ez külön terv lesz, ha kell.
- Fejkövetés (head tracking): nyitott döntés (9. fejezet), alapból nincs.
- Személyre szabott HRTF fényképből vagy MI-vel: később, az MI-réteggel.
- OpenAL Soft, Steam Audio, libspatialaudio vagy SAF a CFA4-folyamatban: lásd 4.3.

## 3. Felépítés

```
forrás (TrueHD / IAB / DTS:X / Auro / PCM)                    szekvencia keverés (2.0…7.1.2, később 7.1.4)
        │ illesztő                                                       │
        ▼                                                                ▼
  SpatialScene: bed csatornák (címke + irány) + objektumok (mono + események)
        │
        ├── Virtuális hangszórók: csatornánként HRIR-pár (fix, olcsó)
        └── Pontos objektum: irányonként interpolált HRIR, átúsztatás mozgáskor
        ▼
  BinauralRenderer → sztereó (L/R) → monitor (QAudioSink, változatlanul sztereó)
                                   → opcionális export (binaurális sztereó fájl)
```

### 3.1 Új crate: `cfa4-hrtf` (L0, tiszta Rust, `unsafe` nélkül)

- **Forrás: a Maul Audio (Rawframe-Project/maul-audio, MIT, C) spatial részének Rust-portja** (4.2). Ugyanezt a feladatot oldja meg, és ugyanazokkal az elvekkel, amelyeket ez a terv eredetileg magától választott: minimumfázisú HRIR plusz külön késleltetés, kimenet-átúsztatás irányváltáskor, allokációmentes audioszál. A releváns C-kód kb. 1700 sor: `binaural*.c`, `hrtf_lookup.h`, `hrtf_resample.h`, `near_field*.h`, `magls.h`, `speaker_panner.c`, `speaker_decoder.c`. A portot fájlonként végezzük, az MIT-fejléc megtartásával és nyilvántartási sorral. C-t nem linkelünk (FFI és `unsafe` kellene), az eszközkezelő részét (PipeWire, ALSA) nem vesszük át, mert a kimenet a Qt-é.
- `HrirSet`: HRIR-párok és késleltetések **5°-os elevációs gyűrűkön**, gyűrűnként egyenletes azimutokkal (a Maul `.maudhrtf` formátuma: 128 tap 48 kHz-en, dokumentált, MIT). Ezt a formátumot olvassuk, nem találunk ki újat. Ha a felhasználó SOFA-fájlt ad, azt betöltéskor erre a rácsra mintavételezzük át.
- **SOFA olvasása:** a `sofar` crate (MIT OR Apache-2.0, a BSD-3 libmysofa tiszta Rust portja, C-függőség nélkül). Ha a `cargo deny` vagy az érettség miatt nem felel meg, a SOFA → `.cfa4hrir` átalakítás `xtask`-ba kerül, a futó program pedig csak a saját formátumot olvassa.
- Előfeldolgozás betöltéskor: mintavétel-átalakítás a kimeneti rátára (`audio-dsp/resample.rs`), **minimumfázisú HRIR + külön ITD-késleltetés**. Így az interpoláció nem fésűsít, és egy irányváltás csak finom késleltetés-változás. Normalizálás: a 0°/0° irány szélessávú energiája 0 dB.
- Irányinterpoláció: a két szomszédos gyűrű két-két azimutja, bilineáris súlyokkal (a Maul `hrtf_lookup` módszere). Keresés nincs, csak osztás, így olcsó és determinisztikus.
- **Közeltér** (Maul `near_field`: merev gömb fejmodell, Duda–Martens 1998): ha egy Atmos-objektum a szoba belsejében, a fejhez közel van, a közeli fül mélyei erősödnek, a távoli fül árnyékoltabb. Ez az OAMD `near` módjának természetes megvalósítása.
- Konvolúció: egyenletesen partícionált overlap-save (a `reverbs.rs` magja kiemelve közös modulba), 128 mintás blokk a lejátszáshoz, nagyobb blokk exporthoz.
- `VirtualSpeakers::new(layout, &HrirSet)`: elrendezésenként fix HRIR-párok (az ITU-R BS.2051 névleges irányai, Auro-3D szerinti magasságok).
- `ObjectBinauralizer`: N objektum, objektumonként interpolált szűrőpár, blokkonkénti átúsztatás (két szűrő párhuzamosan, cos² ablak), a késleltetés simítva.
- **LFE:** mindkét fülre, 120 Hz-es aluláteresztővel, állítható szinten (alapból −10 dB, mert fejhallgatón a +10 dB-es mozihelyzet túl erős). A választás dokumentált CFA4-döntés.
- **Méret / szórás (size, spread):** az objektum a középpont körüli több irány között oszlik meg, dekorrelált ágakkal. Ha a méret 0, pontforrás.
- **Távolság és fejhallgató-mód (OAMD `binaural_render_mode`):** `off` = sima sztereó pan, HRTF nélkül (a keverő így kérte); `near` = száraz; `mid` / `far` = egyre több rövid, szintetikus korai visszaverődés és diffúz utózengés (a meglévő zengető paramétereivel). A szintek dokumentáltak és kikapcsolhatók („Szoba: ki”).

### 3.2 Közös jelenetmodell: `cfa4-spatial` (vagy a `cfa4-hrtf` modulja)

- `SpatialScene { beds: [(ChannelLabel, Direction)], objects: [ObjectTrack], events: [Event] }`. Az irány egységes: egységnyi szobakocka (ahogy a `truehd::render` és az `iab::render` is használja) → gömbi irány a hallgatótól, egy dokumentált leképezéssel (a kocka közepe a hallgató, a fal ±1).
- Illesztők:
  - **TrueHD Atmos:** `ObjectDecoder` → bed + objektumok + `render::Event` + `HeadphoneElement`.
  - **IAB:** `Frame` BedDefinition / ObjectDefinition (pozíció, spread, zónák, snap) → jelenet. Az aktivációs szabályok (UseCase, BedRemap) változatlanul az `iab` crate-ből jönnek.
  - **Csatornás források:** a csatornacímkék → BS.2051 irányok. Auro-3D: a visszaállított 9.1…13.1 csatornák saját magasságukon.
  - **DTS:X objektumok (B-06):** az `xmeta` mátrixával a bedből kivonjuk a magasságok és objektumok hozzájárulását, ezek utána a saját irányukból szólnak (#130). Amíg ez nincs kész, a 7.1 bed virtuális hangszórókon szól.
  - **E-AC-3 JOC (B-07):** lásd 6. fejezet. Addig az 5.1/7.1 bed virtuális hangszórókon.
- A közös modell a hangszórós renderelésnek is jó alap. A `truehd::render` és az `iab::render` panner-kódja most kétszer van meg: az egyesítés nem cél, de ha megtörténik, a jelenetmodellen keresztül történjen.

### 3.3 Engine és lejátszás

- **Forrásmonitor (első lépés):** ha a megnyitott klip TrueHD Atmos vagy IAB, és a monitor mód „Fejhallgató”, akkor a dekóder az objektum-bemutatást adja (`ObjectDecoder`, pre-roll a major sync miatt: `truehd_preroll`), és a binaurális renderer közvetlenül szól. Más forrás a saját csatornáival, virtuális hangszórókon.
- **Programmonitor (szekvencia):** a végső keverés (2.0 / 5.1 / 7.1 / 7.1.2) a monitor utolsó lépéseként virtuális hangszórókon szól. Ma a 7.1-es és magassági anyag sztereóra hajtogatva szól. Ezzel térben szólna, a keverő megváltoztatása nélkül.
- **Opcionális (B-03b):** 7.1.4-es szekvencia-elrendezés a keverőben (`audio-dsp` `Layout::Surround714`, `render/audio.rs`). Ekkor az Atmos és IAB klipek a meglévő rendererrel 7.1.4-re renderelődnek, és a szekvenciában is hallhatók a magasságok. Az objektumok pontos iránya itt elvész (7.1.4 a felbontás). Ez elfogadott egyszerűsítés az objektumbusz nélkül.
- Protokoll: `audio.render` új mezője: `monitor: {"mode": "speakers" | "binaural", "hrtf": "<id>", "room": 0..1, "lfe_db": -10}`. A kimenet továbbra is interleaved float32 sztereó, így a `QAudioSink` és a mérők útja nem változik.
- Valós idejű szabályok: a render szálon nincs allokáció, a HRIR-készlet betöltése és átalakítása háttérben fut, a csere egy lépésben történik. Egy hibás SOFA-fájl hibaüzenet, nem összeomlás (AGENTS §0). Ilyenkor a beépített készlet marad.
- A renderelt hangelőnézet (`render_audio` WAV-gyorsítótár) **hangszórós** keverést tárol. A binaurális lépés utána fut, így a monitor módjának váltása nem teszi érvénytelenné az előnézeteket.

### 3.4 Qt6 felület (`python/cfa4_qt/`)

- **Beállítások ▸ Audio Hardware:** „Előhallgatás: Hangszórók / Fejhallgató (binaurális)”, HRTF-készlet (beépített, letöltött, „SOFA-fájl betöltése…”), LFE-szint, Szoba (ki / kevés / közepes), „Fejhallgató-mód metaadat követése (Atmos)” jelölő.
- **Monitorok:** fejhallgató-ikon a transzportsorban (gyors váltás), és ugyanez a csavarkulcs-menüben. Ha a forrás objektumalapú, a monitor feliratban jelzi: „Atmos · 16 objektum · binaurális”.
- **Audio Meters:** a ténylegesen hallott binaurális L/R-t mutatja. Mellette a hangszórós mérés választható.
- **Opcionális (B-08) objektumnézet panel:** felülnézet és oldalnézet a szobáról, az objektumok pontként a lejátszófej idején. Egy objektumra kattintva szóló/némítás csak az előhallgatásban.
- A külön indítható részek elve (VISION 7.): a Hangszerkesztő munkaterület is ugyanezt a monitorbeállítást használja.
- Fordítás: új kulcsok csak az `i18n/<nyelv>.tsv`-be. A magyar és az angol kész a WP-vel, a többi nyelvhez gépi előfordítás jelöléssel, a meglévő gyakorlat szerint.

### 3.5 Binaurális export (B-05)

- Export ablak ▸ Hang ▸ Csatornák: „Binaurális sztereó (fejhallgatóhoz)”. Ugyanaz a renderer, a „Pontos objektum” mód és nagyobb konvolúciós blokk. A kimeneti fájl metaadatában (WAV `LIST/INFO`, MP4 udta) szerepel: „binaural, HRTF: <készlet>”.
- `cfa4-cli`: `binaural --hrtf <id|fájl.sofa> --room <0..1> in.mkv out.flac`. Ez fedi le a DolbyAtmosBinauralizer teljes láncát, egyetlen saját programban.
- A Smart Cut / Smart Export (R2) bitstream-másolását ez nem érinti. A binaurális hang mindig újrakódolt PCM/FLAC/AAC sáv, és csak ha a felhasználó kifejezetten kéri.

## 4. HRTF-adatkészletek és licencek (ellenőrizve 2026-11-27)

### 4.1 Adatkészletek

| Készlet | Licenc (forrás) | Szerep a tervben |
|---|---|---|
| **SADIE II** (York, KU100 műfej és 18 alany, SOFA, 44,1/48/96 kHz) | **Apache-2.0** (Zenodo 12092466, v2-2) | **beépített alapkészlet** (9. fejezet, 1. döntés): KU100, 48 kHz, 5°-os gyűrűk, 128 tap (a Maul `.maudhrtf`-je, 854 KB) |
| MIT KEMAR (Gardner & Martin, 1994) | „provided free with no restrictions on use, provided the authors are cited”, kereskedelmi célra is (libmysofa copyright-fájl) | második beépített vagy letölthető készlet, hivatkozással |
| HUTUBS (TU Berlin, 96 alany) | CC BY 4.0 | letölthető. Később „hozzád illő HRTF” választás antropometria alapján |
| Neumann KU100 far-field (TH Köln, Bernschütz) | CC BY 3.0 | letölthető, hivatkozással |
| CIPIC (UC Davis) | bármilyen célra, a szerzői jogi megjegyzés megtartásával. Kereskedelmi felhasználónál írásos visszajelzést kérnek | csak a felhasználó saját SOFA-jaként, nem csomagoljuk |
| LISTEN (IRCAM) | „public and available for any use”, köszönetnyilvánítást kérnek | letölthető lehet |
| ARI (ÖAW), FHK L2354 (FH Köln) | CC BY-SA 3.0 | **nem csomagoljuk** (a share-alike 3.0 nem egyértelműen GPLv3-kompatibilis). A felhasználó maga betöltheti |

Szabály: a csomagba csak Apache-2.0, MIT-KEMAR-típusú vagy CC BY 4.0 adat kerül. Minden készlet bekerül a `docs/licensing.md` nyilvántartásába és az `ATTRIBUTION.md`-be, a kért hivatkozással. A letöltés csak a felhasználó jóváhagyásával, rögzített SHA-256-tal és forrás-URL-lel történik (a modelljegyzék mintájára). A letölthető készletek többsége Zenodón van (SADIE II, KU100, HUTUBS). Ezeket a Zenodo nyilvános REST API-ján (`/api/records/<id>`) érjük el, rögzített rekordazonosítóval és verzióval. A Zenodo csak MD5-öt ad a fájlokhoz (`checksum: "md5:…"`, developers.zenodo.org). Ezt is ellenőrizzük, de a döntő a mi rögzített SHA-256-unk. A letöltő betartja a Zenodo kéréskorlátait, és egyszerre egy fájlt tölt le. A Zenodo szerverszoftverére (zenodo-rdm, InvenioRDM) nincs szükség, csak az API-ra. **B-00 feladata:** a SADIE II csomag `LICENSE.txt`-jének elolvasása, és annak ellenőrzése, hogy a KU100 almappa is Apache-2.0.

### 4.2 Kód

| Projekt | Licenc | Mire jó |
|---|---|---|
| **Maul Audio** (Rawframe-Project/maul-audio, commit `aeb3834`, 0.6.0) | **MIT** (Copyright 2026 Sirac Ozmen). A mellékelt SADIE II KU100 48 kHz `.maudhrtf` Apache-2.0, a York-i forrásmegjelöléssel | **fő portolási forrás** a `cfa4-hrtf`-hez: binaurális effekt, HRTF-gyűrűk és -keresés, közeltér, MagLS binaurális ambiszonikus dekóder, VBAP panner, AllRAD hangszóró-dekóder. A golden renderek (`data/golden/`) tesztorákulumok. A `tools/sofa_to_maudhrtf.py` (MIT, numpy + h5py, csak fejlesztői eszköz) SOFA → `.maudhrtf` átalakító |
| OpenSpatialDelay (Spatial-Media-Lab, commit `14947a0`) | GPL-3.0 (a LICENSE nem írja, hogy „or later”, fájlfejléc nincs). JUCE-alapú C++ plugin, libmysofa | **ötletforrás, kódot nem portolunk**: a kód a JUCE típusaira épül, a JUCE 8 pedig AGPL vagy kereskedelmi. Átvehető ötletek: a binaurális út 9.1.6-os virtuális hangszórórácsa plusz zenit (ez sűrűbb, mint a 7.1.4, ezért jobb a „Virtuális hangszórók” módhoz); az MDAP az objektum méretéhez (spread); a 9.1.6 és 13.1 kimeneti elrendezések; az ADM-OSC pozícióvezérlés későbbi, objektumbuszos tervhez. A mellékelt SOFA-készletei (MIT KEMAR, SADIE D2, CIPIC 003, HUTUBS PP2, Bernschütz KU100) ugyanazok, mint a 4.1 táblázatban |
| `sofar` (crates.io 0.3.0) | MIT OR Apache-2.0, a libmysofa (BSD-3) Rust portja | a felhasználó saját SOFA-fájljának olvasása futás közben. Függőségként (a `cargo deny` ellenőrzésével) |
| EBU ADM Renderer (`ear`, Python) | BSD-3-Clause-Clear | az ITU-R BS.2127 objektum-panner referenciája: **tesztorákulum** a jelenet → irány leképezéshez, nem termékkód |
| `libear` (C++) / BEAR (binaurális EAR, BBC) | Apache-2.0 (BEAR alapadatának licence nincs megadva) | ötlet és összehasonlítás. A BEAR adatfájlja a licenc hiánya miatt nem használható |
| harletty-bridge `eac3` crate | Apache-2.0 (de több fájlja FFmpeg-et említ: fájlonkénti fejléc-ellenőrzés kell) | az E-AC-3 JOC objektum-dekódolás (`ObjectPcmDecoder`) portjának jelöltje (B-07) |
| harletty-bridge `dca` | LGPL-2.1-or-later (a 0.9.0 Apache-2.0 jelölése hibás volt, javították) | a `cfa4-dts` már ebből készült, és a nyilvántartás már LGPL-2.1-or-later-ként vezeti. Rendben |
| oxideav-dts | MIT, tisztaszobás (ETSI TS 102 114) | független DTS-dekóder. DTS:X objektumokat nem renderel. Összehasonlító orákulumnak jó lehet, a tervhez nem szükséges |
| Omniphony (`liborender`) | GPL-3.0-or-later | objektumrenderer a harletty-bridge mögött. Kompatibilis, de a mi rendererünk egyszerűbb és saját. Csak ötletnek |

### 4.3 Miért nem OpenAL Soft / Steam Audio / libspatialaudio / SAF

Licencük megengedné (LGPL-2.0+, Apache-2.0, LGPL-2.1+, ISC/GPL keverék). Mind C/C++, a CFA4-folyamatban viszont FFI és `unsafe` kellene, külön folyamatban pedig nem valós idejű. A Maul Audio is C, de kicsi, jól dokumentált és MIT, ezért nem linkeljük, hanem Rustra portoljuk (AGENTS: „port to Rust by default”). Ez a saját Rust út, rövidebb úton.

**A Maul Audio kockázata:** fiatal projekt (2026, egy szerző, 1.0 előtti), az API és a fájlformátum még változhat. Ezért rögzített commitból portolunk, és a `.maudhrtf` olvasónk a saját, verziózott másolatunk. A felső verzió változása nem töri el a CFA4-et.

## 5. Munkacsomagok

Sorrend és függőségek. Minden WP egy PR, friss `main`-ről (`docs/coordination.md`).

| WP | Tartalom | Függ | Kész, ha |
|---|---|---|---|
| **B-00** | Licencek véglegesítése (SADIE II `LICENSE.txt`, Maul fájlfejlécek), `sofar` `cargo deny` próba, a nyilvántartás sorai. A beépített készlet a Maul `sadie2-ku100-48k.maudhrtf` (854 KB) a notice-szal, vagy ugyanez a Maul átalakítójával újragenerálva (SHA-256 egyezés) | – | a nyilvántartás frissült, a csomagolt fájl reprodukálható |
| **B-01** | `cfa4-hrtf`: a Maul binaurális magjának portja (`.maudhrtf` olvasás, gyűrűs keresés, minimumfázis + tört késleltetés, átúsztatás, közeltér), `VirtualSpeakers` 2.0…7.1.4, Auro 13.1, 22.2 elrendezésekkel, LFE | B-00 | egység- és orákulumtesztek zöldek, a Maul golden HRTF-sweep −50 dB-en belül (7. fejezet) |
| **B-02** | `SpatialScene` + illesztők: TrueHD, IAB, csatornás, Auro. `ObjectBinauralizer` (pontos mód, átúsztatás, méret, OAMD fejhallgató-mód, szoba) | B-01 | szintetikus mozgó objektum kattanás nélkül, a valódi Atmos minta objektumai megjelennek a jelenetben |
| **B-03** | Engine: `audio.render` `monitor` mező. Forrásmonitor objektum-útja (TrueHD/IAB), programmonitor virtuális hangszórókkal. Háttérbetöltés, hibakezelés | B-02 | engine-tesztek. A seek és a pre-roll pontos, a lejátszás képkocka-ideje nem nő (referenciagép) |
| **B-03b** | 7.1.4-es szekvencia-elrendezés a keverőben (9. fejezet, 4. döntés: benne van) | B-03 | a keverő 12 csatornán fut, export 7.1.4 WAV-ba |
| **B-04** | Qt6: beállítások, monitorkapcsoló, HRTF-választó, SOFA betöltése, mérők, `i18n/` | B-03 | Qt offscreen tesztek, kézi próba Waylanden és X11-en |
| **B-05** | Binaurális sztereó export (Export ablak + `cfa4-cli binaural`) | B-02 | export-teszt: a fájl egyezik a monitor útjával (azonos beállítás mellett bitpontosan) |
| B-06 | DTS:X objektumok és magasságok saját helyükön az `xmeta` alapján (#130) | B-02 | a DTS-X minta objektumai irányból szólnak, a bed kivonás után nem szól duplán (mérés) |
| B-07 | E-AC-3 JOC objektum-dekódolás (#126): **külön tervbe kerül** (9. fejezet, 3. döntés). Ebben a tervben nincs | B-02 | az Atmos DD minta objektumai megjelennek, összevetve a harletty kimenetével |
| B-08 | Objektumnézet panel (opcionális) | B-04 | Qt teszt, kézi próba |
| **B-09** | Validációs jelentés: az öt Altitude16 minta, mérések, hallgatási ellenőrzőlista | B-04, B-05 | jelentés a `docs/audit/`-ban, a tulajdonos meghallgatta |

A vastagon szedett WP-k adják a minimális, használható kört: fejhallgatón szóló Atmos (TrueHD), IAB, Auro és minden csatornás anyag a Qt6 monitorban, valamint binaurális export.

## 6. Kockázatok

- **E-AC-3 JOC:** nagy munka (QMF-tartomány, JOC-mátrixok). A harletty-bridge `eac3` crate-je rövidít, de egyes fájljai FFmpeg-et említenek. Ha FFmpeg-származékok, LGPL-2.1-or-later alatt vehetők át (mint a `dca`), de ehhez fájlonkénti ellenőrzés kell. Javaslat: a B-07 külön döntés legyen.
- **DTS:X objektumok:** a privát metaadat értelmezése korpusz alapján történt (`xmeta.rs` fejléce). Kevés mintánál téves irány vagy dupla szólás lehet. Mérés kell, és tartalék: a bed marad.
- **Auro-3D:** csak bitpontos hordozónál működik. A DTS-HD MA dekóder bitpontos, így a mintánál várhatóan rendben van.
- **A HRTF egyéni:** a műfej-HRTF (KU100) sok hallgatónál elöl-hátul tévesztést okoz. Ezért választható a készlet, és saját SOFA is betölthető. Ez nem hiba, a dokumentációban leírjuk.
- **Valós idejű terhelés:** 16 objektum + 10 bed pontos módban kb. 26 × 2 konvolúció (256 mintás HRIR, 128-as blokk). Becslés: egy CPU-mag néhány százaléka. A B-03 méri a referenciagépen. Ha sok, a programmonitor virtuális hangszórós módban marad.

## 7. Bizonyítás (VISION 2.5)

1. **Egységtesztek:** szimmetria (0°/0°: L = R ±0,1 dB); +90° azimut: ILD 4 kHz-en legalább 10 dB, ITD keresztkorrelációval 0,55–0,75 ms; az interpoláció folytonos (egy körbe mozgó objektum mintáról mintára ugrása egy küszöb alatt, kattanásmentes); energiamegtartás virtuális hangszórókon; LFE szűrő és szint.
2. **Saját konvolúció ↔ közvetlen időtartományú konvolúció** ugyanazzal a HRIR-rel: eltérés legfeljebb −100 dBFS.
2a. **Maul Audio orákulum:** a C könyvtár (csak a tesztgépen fordítva, nem a termékben) és a `data/golden/` HRTF-sweep render ugyanarra a jelenetre. A Rust-port eltérése blokkonként legfeljebb −50 dB, a Maul saját platformközi tűrése.
3. **FFmpeg `sofalizer` orákulum** (libmysofa, ugyanaz a SOFA, `interpolate=0`, `minphase=0`): a 7.1 virtuális hangszórós kimenetünk tűrésen belül egyezik vele. Előfeltétel: a rögzített FFmpeg-build `--enable-libmysofa`-val fusson. Ha nem, a B-00 rögzíti, és a teszt kimarad, indoklással.
4. **EBU ADM Renderer (`ear`) orákulum** a jelenet → irány leképezéshez szintetikus objektumpályákon.
5. **Valódi minták** (a projekt mappájából, nem kerülnek a repóba): Dolby Atmos 16-9 és 2.35 (TrueHD, objektumok), DTS-X (bed, majd B-06), Auro-3D (13.1 virtuális hangszórók), Atmos DD MP4 (5.1 bed, majd B-07). A DCP Atmos export IAB kimenete körbefuttatva.
6. **Mérés a referenciagépen:** a binaurális lépés CPU-ideje és késleltetése. A lejátszás és a seek ideje bekapcsolt fejhallgató-móddal sem romlik.
7. **Hallgatási ellenőrzőlista** a tulajdonosnak: elöl/hátul, fent/lent, mozgó objektum, LFE, szoba be/ki. A B-09 jelentésébe kerül.

## 8. Viszony a Smart Cut R2-höz (ütközés elkerülése)

- Az R2 fájljai: `matroska`, `isobmff`, `engine/src/smart_cut.rs`, `export/src/audio_copy.rs`, Smart Export UI. Ez a terv ezekhez **nem nyúl**.
- Közös crate-ek: `cfa4-truehd`, `cfa4-dts`, `cfa4-ac3`. Az R2 ezeket keretelemzésre használja, ez a terv dekódolásra és metaadatra. Szabály: itt **csak bővítő** (additív) változás lehet, az R2 által használt nyilvános API nem változik. A B-06 (`dts`) és a B-07 (`ac3`/`eac3`) csak az R2 hang-WP-i (R2-00…R2-02) beolvasztása után indul.
- A B-00…B-05 az R2-vel párhuzamosan haladhat.

## 9. Döntések (elfogadva 2026-11-27, mind a javaslat szerint)

1. **Beépített HRTF:** SADIE II KU100, Apache-2.0 (a Maul Audio kész `.maudhrtf` fájlja vagy annak reprodukciója). Az MIT KEMAR letölthető.
2. **Sorrend:** előbb a forrásmonitor objektum-útja és a binaurális export (B-00…B-05), utána a B-03b.
3. **E-AC-3 JOC (B-07):** külön tervben, a harletty-bridge `eac3` fejléc-ellenőrzésével együtt. Itt csak a bed szól virtuális hangszórókon.
4. **7.1.4-es szekvencia-elrendezés (B-03b):** ebben a tervben.
5. **Fejkövetés:** most nincs.
6. **Binaural render mode:** a szoba-értelmezés (near/mid/far) alapból bekapcsolva, kikapcsolható.

## 10. Források

- ThreeDeeJay/DolbyAtmosBinauralizer: csak ötlet, licenc nélkül (koordinátori vizsgálat, 2026-11-27).
- SADIE II: <https://zenodo.org/records/12092466> · KU100 TH Köln: <https://zenodo.org/records/3928297> · MIT KEMAR / CIPIC / LISTEN licencek: libmysofa copyright-fájl, <https://launchpad.net/ubuntu/jammy/+source/libmysofa/+copyright> · HUTUBS: DepositOnce (CC BY 4.0) · ARI: <https://www.kfs.oeaw.ac.at/research/experimental_audiology/hrtf/database/hrtfItESOFAb.html>
- Maul Audio: <https://github.com/Rawframe-Project/maul-audio> (MIT; `docs/internals/spatial-rendering.md`, `docs/internals/file-formats.md`)
- `sofar`: <https://docs.rs/crate/sofar/0.3.0> · EAR: <https://pypi.org/project/ear/> · libear: <https://github.com/ebu/libear> · BEAR: <https://github.com/ebu/bear>
- harletty-bridge (licenc szakasz, `eac3` crate): <https://github.com/harletty/harletty-bridge> · oxideav-dts: <https://github.com/OxideAV/oxideav-dts> (MIT)
- AES69 (SOFA), ITU-R BS.2051 (elrendezések), ITU-R BS.2127 (ADM renderer), SMPTE ST 2098-2 (IAB).
