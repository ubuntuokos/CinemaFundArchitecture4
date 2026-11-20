# DCPVAL-LIBDCP-02 – A CFA4 DCP-exportja a libdcp `dcpverify`-án, a javítások után (ellenőrző mérés)

**Terv:** [CFA4-DCPVAL-LIBDCP-001](../plans/CFA4-DCPVAL-LIBDCP-001-FINAL-1.0.md) és [1.1-es kiegészítése](../plans/CFA4-DCPVAL-LIBDCP-001-FINAL-1.1-KIEGESZITES.md), 7. fázis · **Dátum:** 2026-11-14 · **Kód nem változott.**

Előzmény: [DCPVAL-LIBDCP-01](DCPVAL-LIBDCP-01-MEASUREMENT-2026-11-14.md). Ott a `dcpverify` minden képkockán MCT-hibát jelzett, `MainMarkers`-hibát, 2K-ban pedig guard bit-hibát. A rövid, 10 képkockás és `feature` jelölésű tesztminta több mellékhatást is hozott.

## 1. Mit mértem

- **Ellenőrző:** ugyanaz, mint a 01-es mérésben: libdcp v1.10.64 (`891facc8`) `dcpverify`, asdcplib v1.0.10 és libcxml v0.17.17 mellett, a [DCPOM-01 fordítószkript](DCPOM-01-build.sh) libdcp-részével, Ubuntu 24.04.5-ön. Alapbeállítás: a képkocka-ellenőrzés (`check_picture_details`) és a hash-ellenőrzés be van kapcsolva.
- **Kód:** a 3–6. fázis együtt, vagyis a `main` `cda07ae7`-en (benne a 3. fázis #1225 és az 5. fázis #1238) a 4. fázis (#1243, `ae7c1b31`) és a 6. fázis (#1244, `4f6bfbaa`). A mért fej: `4f6bfbaa`.
- **Minta:** öt csomag, mindegyik 2 másodperces, `test` tartalomtípusú (`ContentKind`), SMPTE, kép és 5.1 hang (440 Hz-es szinusz). A képforrás 1920×1080-as CFA4-generátor, hogy a kódolónak valódi tartalma legyen:

| Csomag | Felbontás, képarány | Kép/s, képkocka | Képforrás |
|---|---|---|---|
| `2k-flat-white` | 2K Flat (1998×1080) | 25, 50 | fehér matt (mint a 01-es mérésben) |
| `2k-flat-bars` | 2K Flat (1998×1080) | 25, 50 | színsávok (`BarsAndTone`) |
| `2k-scope-leader` | 2K Scope (2048×858) | 25, 50 | visszaszámláló (`CountingLeader`) |
| `4k-flat-bars` | 4K Flat (3996×2160) | 24, 48 | színsávok |
| `4k-scope-leader` | 4K Scope (4096×1716) | 24, 48 | visszaszámláló |

A 4K-csomagok 24 kép/s-osak, mert a Bv2.1 4K-ban csak 24 és 48 kép/s-ot enged. Egy első, 25 kép/s-os 4K-futás egyetlen üzenete ez volt: `Bv2.1 error: The frame rate 25/1 of picture asset … is not allowed for 4K DCPs`. Ez a minta választása, nem az export hibája, a CFA4 exportpanelje pedig a felhasználó dolga.

- **Előállítás:** ideiglenes, nem commitolt teszt a `crates/export/src/dcp_tests.rs`-ben, a meglévő `project()` mintájára (generátor a V1-en, hang az A1-en), `export()` hívással. A kódja az 5. pontban van. Utána `dcpverify -q <mappa>`.
- **A CFA4 saját ellenőrzője:** ugyanerre az öt csomagra a natív backend `smpte-cinema` profilja, egy ideiglenes tesztből (`NativeValidator`).

## 2. Eredmény

### 2.1 `dcpverify`

| Csomag | rc | Üzenet (`-q`) | Összegzés (`-q` nélkül) |
|---|---|---|---|
| `2k-flat-white` | 0 | nincs | `DCP verified OK.` |
| `2k-flat-bars` | 0 | nincs | `DCP verified OK.` |
| `2k-scope-leader` | 0 | nincs | `DCP verified OK.` |
| `4k-flat-bars` | 0 | nincs | `DCP verified OK.` |
| `4k-scope-leader` | 0 | nincs | `DCP verified OK.` |

Nincs ERROR, nincs Bv2.1 error, és WARNING sincs.

### 2.2 A CFA4 ellenőrzője (`smpte-cinema`)

Mind az öt csomag ítélete **Pass**, 24 vizsgálattal. Ezek nem Pass állapotúak:

| Szabály | Állapot | Csomagok | Ok |
|---|---|---|---|
| `DCP.PIC.001` | WARNING | a három 2K-s | „25 fps 2K is in the Smpte standard but not every cinema server plays it”. A minta 25 kép/s-os, ez szándékos figyelmeztetés. |
| `DCP.IAB.001` | NotApplicable | mind | nincs immerzív hangsáv |
| `DCP.SUB.001`, `DCP.NAME.001` | Unchecked | mind | a natív backend még nem valósítja meg (DCP-08, DCP-09) |

`DCP.J2K.001` (DCI-kodekprofil, benne a 2K guard bit-szám) és `DCP.MARK.001` (jelölők) mind az öt csomagon Pass.

## 3. Következtetések

1. **A D9 elfogadása teljesül:** a `dcpverify` ERROR és Bv2.1 error nélkül fut le a CFA4 2K- és 4K-exportján. A v1.0 4. fázisának elfogadása így a 3–6. fázis együttes kódjával teljesül.
2. **A 01-es mérés három eltérése megszűnt:**
   - MCT-flag (4. fázis): a `dcpverify` egyetlen képkockán sem jelez `invalid multi-component transform flag`-et.
   - `MainMarkers` `EntryPoint`/`Duration` (5. fázis): nincs ilyen üzenet.
   - 2K guard bitek (6. fázis): nincs `uses 2 guard bits in a 2K image` üzenet.
3. **A 01-es mérés minta-mellékhatásai** (1 másodpercnél rövidebb asset, `feature` jelölés FFEC/FFMC nélkül, LFOC) a hosszabb, `test` jelölésű mintán nem jelentkeznek.
4. **Feltétel:** az eredmény a #1243 és a #1244 beolvadásával lesz igaz a `main`-en. Ha valamelyik változik a beolvadás előtt, a mérést meg kell ismételni.

## 4. Mit nem mértem

- Interop csomagot (a terv SMPTE-re szól), titkosított csomagot, feliratot és immerzív hangot.
- 48 kép/s-os vagy HFR 2K-t, 3D-t.
- Hosszú (perces) anyagot: a kodekprofil képkockánként azonos, a 2 másodperc a rövid asset-hibák elkerülésére elég.

## 5. A minta előállítása (ideiglenes teszt, nincs commitolva)

A `crates/export/src/dcp_tests.rs` végére írva, `DCPVAL_OUT=<mappa> cargo test -p cfa4-export --release --lib -- --ignored zz_dcpval_probe` paranccsal futtatva, majd a fájlt visszaállítva:

```rust
#[test]
#[ignore]
fn zz_dcpval_probe() {
    let out = std::env::var("DCPVAL_OUT").unwrap();
    let r25 = FrameRate::new(25, 1);
    let mk = |g_: Generator| {
        let mut p = Project::new("dcp");
        let pic = GeneratorSource::new(g_, 1920, 1080, r25, Tick(2 * TICKS_PER_SECOND));
        let tone = GeneratorSource::new(Generator::Tone { hz: 440.0, db: -6.0 }, 320, 180, r25, Tick(2 * TICKS_PER_SECOND));
        let add = |p: &mut Project, g: &GeneratorSource| {
            let info = g.info().clone();
            p.add_item(&info.name.clone(), Label::Iris, ItemKind::Media(MediaClip { media: MediaRef::Generator(g.generator.clone()), info, interpret: Default::default(), mark_in: None, mark_out: None, markers: vec![], offline: false, proxy: None, identity: None }), None)
        };
        let v = add(&mut p, &pic);
        let a = add(&mut p, &tone);
        let seq = p.new_sequence("Probe", SequenceSettings { width: 1920, height: 1080, frame_rate: r25, ..Default::default() }, 1, 1, None);
        let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, r25.tick_of(50)), r25).unwrap();
        let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, r25.tick_of(50)), r25).unwrap();
        let q = p.sequence_mut(seq).unwrap();
        q.video_tracks[0].items.push(vi);
        q.audio_tracks[0].items.push(ai);
        let mut m = SourceMap::default();
        m.0.insert(v, Arc::new(pic));
        m.0.insert(a, Arc::new(tone));
        (Arc::new(p), seq, m)
    };
    let cases: Vec<(&str, Generator, &str, &str)> = vec![
        ("2k-flat-white", Generator::ColorMatte { color: [1.0, 1.0, 1.0, 1.0] }, "2k", "flat"),
        ("2k-flat-bars", Generator::BarsAndTone, "2k", "flat"),
        ("2k-scope-leader", Generator::CountingLeader, "2k", "scope"),
        ("4k-flat-bars", Generator::BarsAndTone, "4k", "flat"),
        ("4k-scope-leader", Generator::CountingLeader, "4k", "scope"),
    ];
    for (name, g_, res, aspect) in cases {
        let (p, seq, m) = mk(g_);
        let dir = format!("{out}/{name}");
        let _ = std::fs::remove_dir_all(&dir);
        let s = ExportSettings {
            format: Format::Dcp,
            path: dir.clone(),
            range: Some(TimeRange::new(Tick::ZERO, r25.tick_of(50))),
            dcp_resolution: res.into(),
            dcp_aspect: aspect.into(),
            dcp_content_kind: "test".into(),
            dcp_language: "hu-HU".into(),
            frame_rate: if res == "4k" { Some(FrameRate::new(24, 1)) } else { None },
            ..Default::default()
        };
        let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
        eprintln!("{name}: {} frames", r.frames);
    }
}
```
