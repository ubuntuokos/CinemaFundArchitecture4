//! JPEG 2000 against ffmpeg: ffmpeg's `jpeg2000` encoder (and `libopenjpeg` for the DCI cinema
//! profile) write codestreams from generated test patterns. Reversible 5/3 streams must decode
//! bit-exact to the encoder's input; irreversible 9/7 streams to within ±1 of ffmpeg's decode
//! (float rounding), with the same PSNR against the input.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Spec {
    name: &'static str,
    /// Frame size before the odd crop (3 columns, 5 rows off).
    size: &'static str,
    pix_fmt: &'static str,
    /// Encoder and its options.
    enc: &'static [&'static str],
    /// Bit-exact to the input (reversible, all layers).
    lossless: bool,
    /// File extension (j2k codestream or jp2 file).
    ext: &'static str,
}

const SPECS: &[Spec] = &[
    Spec { name: "gray_53", size: "96x64", pix_fmt: "gray", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec { name: "rgb_53_rct", size: "320x240", pix_fmt: "rgb24", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec { name: "yuv420_53", size: "320x240", pix_fmt: "yuv420p", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec { name: "gray16_53", size: "200x150", pix_fmt: "gray16le", enc: &["-c:v", "jpeg2000", "-pred", "dwt53"], lossless: true, ext: "j2k" },
    Spec {
        name: "rgb_53_tiles",
        size: "640x480",
        pix_fmt: "rgb24",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-tile_width", "64", "-tile_height", "48"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "lrcp",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "lrcp", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "rlcp",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "rlcp", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "rpcl",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "rpcl", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "pcrl",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "pcrl", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "cprl",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-prog", "cprl", "-tile_width", "128", "-tile_height", "128"],
        lossless: true,
        ext: "j2k",
    },
    Spec {
        name: "sop_eph",
        size: "320x240",
        pix_fmt: "rgb24",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-sop", "1", "-eph", "1"],
        lossless: true,
        ext: "j2k",
    },
    Spec { name: "jp2", size: "200x100", pix_fmt: "rgb24", enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-format", "jp2"], lossless: true, ext: "jp2" },
    Spec { name: "yuv420_97", size: "320x240", pix_fmt: "yuv420p", enc: &["-c:v", "jpeg2000", "-pred", "dwt97int"], lossless: false, ext: "j2k" },
    Spec { name: "rgb_97_ict", size: "320x240", pix_fmt: "rgb24", enc: &["-c:v", "jpeg2000", "-pred", "dwt97int"], lossless: false, ext: "j2k" },
    Spec {
        name: "yuv420_97_q10",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt97int", "-q:v", "10"],
        lossless: false,
        ext: "j2k",
    },
    // quality layers, lossy rates: the full stream against ffmpeg
    Spec {
        name: "layers",
        size: "320x240",
        pix_fmt: "yuv420p",
        enc: &["-c:v", "jpeg2000", "-pred", "dwt53", "-layer_rates", "100,10,1"],
        lossless: false,
        ext: "j2k",
    },
    // DCI 2K (SMPTE 429-4): 12-bit X'Y'Z', 9/7, CPRL, a tile-part per component
    Spec {
        name: "dci_2k",
        size: "2051x1085",
        pix_fmt: "xyz12le",
        enc: &["-c:v", "libopenjpeg", "-profile:v", "cinema2k", "-cinema_mode", "2k_24"],
        lossless: false,
        ext: "j2k",
    },
];

fn input(s: &Spec) -> String {
    format!("testsrc2=s={},crop=iw-3:ih-5:1:2", s.size)
}

fn make(ff: &Path, s: &Spec) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("j2k").join(format!("{}.{}", s.name, s.ext));
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let st = Command::new(ff)
            .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", &input(s), "-frames:v", "1", "-pix_fmt", s.pix_fmt])
            .args(s.enc)
            .args(["-f", "image2"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status();
        matches!(st, Ok(x) if x.success())
    })
}

/// Samples of each component from ffmpeg raw output in `pix_fmt` (planar or packed).
fn raw(ff: &Path, args: &[&str], pix_fmt: &str, comps: &[(usize, u8)]) -> Vec<Vec<i32>> {
    let o = Command::new(ff).args(["-v", "error"]).args(args).args(["-pix_fmt", pix_fmt, "-f", "rawvideo", "-"]).output().unwrap();
    let d = o.stdout;
    let n = comps.len();
    match pix_fmt {
        "rgb24" => (0..n).map(|c| (0..comps[c].0).map(|i| i32::from(d[i * n + c])).collect()).collect(),
        "xyz12le" => {
            (0..n).map(|c| (0..comps[c].0).map(|i| i32::from(u16::from_le_bytes([d[2 * (i * n + c)], d[2 * (i * n + c) + 1]]) >> 4)).collect()).collect()
        }
        "gray16le" => vec![(0..comps[0].0).map(|i| i32::from(u16::from_le_bytes([d[2 * i], d[2 * i + 1]]))).collect()],
        _ => {
            let mut off = 0;
            comps
                .iter()
                .map(|&(len, _)| {
                    let v = d[off..off + len].iter().map(|&b| i32::from(b)).collect();
                    off += len;
                    v
                })
                .collect()
        }
    }
}

/// Whether `ff` has the encoder a spec names (the pinned CI ffmpeg is built without
/// `libopenjpeg`, so the DCI cinema stream comes from a local ffmpeg only).
fn has_encoder(ff: &Path, s: &Spec) -> bool {
    let Some(name) = s.enc.iter().skip_while(|a| **a != "-c:v").nth(1) else { return true };
    Command::new(ff)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some(name)))
        .unwrap_or(false)
}

#[test]
fn ffmpeg_encoded_codestreams_decode() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for s in SPECS {
        if !has_encoder(&ff, s) {
            eprintln!("{}: skipped, ffmpeg has no {:?} encoder", s.name, s.enc);
            continue;
        }
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        let img = cfa4_j2k::decode(&data).unwrap_or_else(|e| panic!("{}: {e}", s.name));
        let comps: Vec<(usize, u8)> = img.components.iter().map(|p| (p.data.len(), p.precision)).collect();
        let src = raw(&ff, &["-f", "lavfi", "-i", &input(s), "-frames:v", "1"], s.pix_fmt, &comps);
        let theirs = raw(&ff, &["-i", f.to_str().unwrap()], s.pix_fmt, &comps);
        let mut report = String::new();
        for (c, p) in img.components.iter().enumerate() {
            let peak = f64::from((1u32 << p.precision) - 1);
            let psnr = |want: &[i32]| {
                let se: f64 = p.data.iter().zip(want).map(|(&a, &b)| f64::from(a - b).powi(2)).sum();
                10.0 * (peak * peak / (se / p.data.len() as f64).max(1e-12)).log10()
            };
            let worst = p.data.iter().zip(&theirs[c]).map(|(&a, &b)| (a - b).abs()).max().unwrap_or(0);
            let (ps, pf) = (psnr(&src[c]), psnr(&theirs[c]));
            report += &format!(" c{c} {}x{}: {ps:.1} dB vs input, ±{worst} vs ffmpeg;", p.width, p.height);
            if s.lossless {
                assert!(p.data == src[c], "{}: component {c} is not lossless", s.name);
            } else {
                assert!(worst <= 1, "{}: component {c} differs from ffmpeg by {worst}", s.name);
                assert!(pf > 80.0, "{}: component {c} only {pf:.1} dB against ffmpeg", s.name);
            }
        }
        println!("{}:{report}", s.name);
    }
}

/// OpenJPEG's command-line tools, when installed (`libopenjp2-tools`): the code-block styles
/// ffmpeg cannot write.
fn opj(tool: &str) -> Option<PathBuf> {
    let ok = Command::new(tool).arg("-h").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
    ok.then(|| PathBuf::from(tool))
}

/// R, G, B planes of a binary PPM (P6, 8-bit).
fn ppm(data: &[u8]) -> Vec<Vec<i32>> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        loop {
            while data.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            if data.get(at) != Some(&b'#') {
                break;
            }
            while data.get(at).is_some_and(|&b| b != b'\n') {
                at += 1;
            }
        }
        let start = at;
        while data.get(at).is_some_and(|b| !b.is_ascii_whitespace()) {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&data[start..at]).to_string());
    }
    assert_eq!(fields[0], "P6");
    let n = fields[1].parse::<usize>().unwrap() * fields[2].parse::<usize>().unwrap();
    let px = &data[at + 1..at + 1 + 3 * n];
    (0..3).map(|c| (0..n).map(|i| i32::from(px[3 * i + c])).collect()).collect()
}

/// What OpenJPEG writes and ffmpeg cannot: every code-block style (selective arithmetic coding
/// bypass, RESET, TERMALL, VSC, predictable termination, segmentation symbols, and combinations),
/// progression order changes and regions of interest. 5/3 bit-exact to the input, 9/7 within ±1
/// of OpenJPEG's own decode. Skipped without `opj_compress`.
#[test]
fn openjpeg_codestreams() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (Some(enc), Some(dec)) = (opj("opj_compress"), opj("opj_decompress")) else {
        eprintln!("skipped: OpenJPEG tools (libopenjp2-tools) not installed");
        return;
    };
    let dir = cfa4_testkit::fixtures_dir("j2k").join("opj");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.ppm");
    let st =
        Command::new(&ff).args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=203x117", "-frames:v", "1", "-pix_fmt", "rgb24"]).arg(&input).status();
    assert!(matches!(st, Ok(s) if s.success()));
    let src = ppm(&std::fs::read(&input).unwrap());
    // bypass 1, RESET 2, TERMALL 4, VSC 8, predictable termination 16, segmentation symbols 32
    // (name, options, compared with: None the input exactly, Some(tolerance) OpenJPEG's decode)
    let mut cases: Vec<(String, Vec<String>, Option<i32>)> = Vec::new();
    for mode in [1, 2, 4, 8, 16, 32, 62, 5, 9, 63] {
        cases.push((format!("m{mode}"), vec!["-M".into(), mode.to_string(), "-b".into(), "32,32".into()], None));
        cases.push((format!("m{mode}_97"), vec!["-M".into(), mode.to_string(), "-b".into(), "32,32".into(), "-I".into(), "-r".into(), "20".into()], Some(1)));
    }
    let args = |a: &str| a.split(' ').map(String::from).collect::<Vec<_>>();
    // progression order changes (POC), in the main header and per tile with tile-parts
    cases.push(("poc".into(), args("-r 20,10,1 -POC T0=0,0,2,3,3,RLCP/T0=0,0,3,6,3,CPRL"), None));
    // OpenJPEG writes this one with packets missing (its own decode is not lossless either), so
    // it is compared with OpenJPEG's decode
    cases.push(("poc_tiles".into(), args("-r 20,10,1 -t 64,64 -POC T0=0,0,1,6,3,LRCP/T1=0,0,2,4,2,PCRL/T1=0,0,3,6,3,RLCP"), Some(0)));
    // region of interest (RGN Maxshift on one component)
    cases.push(("roi".into(), args("-ROI c=0,U=4"), None));
    cases.push(("roi_97".into(), args("-ROI c=1,U=6 -I -r 15"), Some(1)));
    for (name, opts, against) in cases {
        let (cs, out) = (dir.join(format!("{name}.j2k")), dir.join(format!("{name}.ppm")));
        let st = Command::new(&enc).arg("-i").arg(&input).arg("-o").arg(&cs).args(&opts).stdout(Stdio::null()).status();
        assert!(st.is_ok_and(|s| s.success()), "{name}: opj_compress");
        let img = cfa4_j2k::decode(&std::fs::read(&cs).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let want = if against.is_some() {
            let st = Command::new(&dec).arg("-i").arg(&cs).arg("-o").arg(&out).stdout(Stdio::null()).status();
            assert!(st.is_ok_and(|s| s.success()), "{name}: opj_decompress");
            ppm(&std::fs::read(&out).unwrap())
        } else {
            src.clone()
        };
        for (c, p) in img.components.iter().enumerate() {
            let worst = p.data.iter().zip(&want[c]).map(|(&a, &b)| (a - b).abs()).max().unwrap_or(0);
            assert!(worst <= against.unwrap_or(0), "{name}: component {c} differs by {worst}");
        }
    }
}

/// HTJ2K (T.814) codestreams written by Grok (`grokj2k-tools`, HT code-block style `-M 64`):
/// 5/3 bit-exact to the input, 9/7 within ±1 of OpenJPEG's decode (OpenJPEG 2.5 decodes HT).
/// Skipped without `grk_compress` / `opj_decompress`.
#[test]
fn htj2k_codestreams() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (Some(enc), Some(dec)) = (opj("grk_compress"), opj("opj_decompress")) else {
        eprintln!("skipped: Grok (grokj2k-tools) or OpenJPEG (libopenjp2-tools) not installed");
        return;
    };
    let dir = cfa4_testkit::fixtures_dir("j2k").join("ht");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.ppm");
    let st =
        Command::new(&ff).args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=203x117", "-frames:v", "1", "-pix_fmt", "rgb24"]).arg(&input).status();
    assert!(matches!(st, Ok(s) if s.success()));
    let src = ppm(&std::fs::read(&input).unwrap());
    let args = |a: &str| a.split(' ').map(String::from).collect::<Vec<_>>();
    // (name, options, compared with: None the input exactly, Some(tolerance) OpenJPEG's decode)
    let cases: Vec<(&str, Vec<String>, Option<i32>)> = vec![
        ("ht", args("-M 64"), None),
        ("ht_cb32x16", args("-M 64 -b 32,16"), None),
        ("ht_cb4x4", args("-M 64 -b 4,4 -n 3"), None),
        ("ht_tiles_rpcl", args("-M 64 -t 64,48 -p RPCL"), None),
        ("ht_precincts", args("-M 64 -c [64,64],[32,32] -b 16,16"), None),
        ("ht_vsc", args("-M 72"), None),
        ("ht_97", args("-M 64 -I"), Some(1)),
        ("ht_97_r20", args("-M 64 -I -r 20"), Some(1)),
        ("ht_97_r60", args("-M 64 -I -r 60 -b 32,32"), Some(1)),
        ("ht_53_r10", args("-M 64 -r 10"), Some(0)),
    ];
    for (name, opts, against) in cases {
        let (cs, out) = (dir.join(format!("{name}.j2k")), dir.join(format!("{name}.ppm")));
        let st = Command::new(&enc).arg("-i").arg(&input).arg("-o").arg(&cs).args(&opts).stdout(Stdio::null()).stderr(Stdio::null()).status();
        assert!(st.is_ok_and(|s| s.success()), "{name}: grk_compress");
        let img = cfa4_j2k::decode(&std::fs::read(&cs).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let want = if against.is_some() {
            let st = Command::new(&dec).arg("-i").arg(&cs).arg("-o").arg(&out).stdout(Stdio::null()).status();
            assert!(st.is_ok_and(|s| s.success()), "{name}: opj_decompress");
            ppm(&std::fs::read(&out).unwrap())
        } else {
            src.clone()
        };
        for (c, p) in img.components.iter().enumerate() {
            let worst = p.data.iter().zip(&want[c]).map(|(&a, &b)| (a - b).abs()).max().unwrap_or(0);
            assert!(worst <= against.unwrap_or(0), "{name}: component {c} differs by {worst}");
        }
        // damaged HT code-blocks give errors or a picture, never a panic
        let data = std::fs::read(&cs).unwrap();
        let mut x = 0x5EED_0042u32 ^ data.len() as u32;
        for k in 0..FUZZ {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let mut m = data.clone();
            if k % 2 == 0 {
                let bit = x as usize % (m.len() * 8);
                m[bit / 8] ^= 0x80 >> (bit % 8);
            } else {
                let at = x as usize % m.len();
                for (i, b) in m.iter_mut().skip(at).take(1 + (x >> 24) as usize % 32).enumerate() {
                    *b = (x >> (i % 24)) as u8;
                }
            }
            let _ = cfa4_j2k::decode(&m);
        }
        println!("{name}: ok");
    }
}

/// HTJ2K codestreams from other encoders, dropped into `target/fixtures/j2k/ht-samples/` (never
/// committed): Grok writes cleanup passes only, so the significance propagation and magnitude
/// refinement passes and placeholder passes are checked here, against OpenJPEG's decode (±1).
/// OpenJPH's `subprojects/js/html/test.j2c` (Kakadu-style, with refinement passes) is one such
/// stream. 8-bit RGB streams only; skipped without samples or `opj_decompress`.
#[test]
fn htj2k_sample_codestreams() {
    let Some(dec) = opj("opj_decompress") else {
        eprintln!("skipped: OpenJPEG (libopenjp2-tools) not installed");
        return;
    };
    let dir = cfa4_testkit::fixtures_dir("j2k").join("ht-samples");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: no HTJ2K samples in {}", dir.display());
        return;
    };
    let out_dir = cfa4_testkit::fixtures_dir("j2k").join("ht-samples-out");
    std::fs::create_dir_all(&out_dir).unwrap();
    for e in entries.flatten() {
        let cs = e.path();
        if !matches!(cs.extension().and_then(|x| x.to_str()), Some("j2c" | "j2k" | "jph" | "jhc")) {
            continue;
        }
        let name = cs.file_name().unwrap().to_string_lossy().to_string();
        let img = cfa4_j2k::decode(&std::fs::read(&cs).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        if img.components.len() != 3 || img.components.iter().any(|p| p.precision != 8) {
            eprintln!("{name}: skipped (not 8-bit RGB)");
            continue;
        }
        let out = out_dir.join(format!("{name}.ppm"));
        let st = Command::new(&dec).arg("-i").arg(&cs).arg("-o").arg(&out).stdout(Stdio::null()).status();
        assert!(st.is_ok_and(|s| s.success()), "{name}: opj_decompress");
        let want = ppm(&std::fs::read(&out).unwrap());
        for (c, p) in img.components.iter().enumerate() {
            let worst = p.data.iter().zip(&want[c]).map(|(&a, &b)| (a - b).abs()).max().unwrap_or(0);
            assert!(worst <= 1, "{name}: component {c} differs by {worst}");
        }
        println!("{name}: ok");
    }
}

/// Damaged variants per stream.
const FUZZ: u32 = 60;

/// Damaged codestreams (bit flips, truncation, scrambled bytes) give errors or a picture, never a
/// panic.
#[test]
fn damaged_codestreams_never_panic() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut x = 0x00C0_FFEEu32;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    for s in SPECS.iter().filter(|s| s.size != "2051x1085") {
        let f = make(&ff, s).unwrap_or_else(|| panic!("could not generate {}", s.name));
        let data = std::fs::read(&f).unwrap();
        for k in 0..FUZZ {
            let mut m = data.clone();
            match k % 3 {
                0 => {
                    for _ in 0..1 + rnd() % 16 {
                        let bit = rnd() as usize % (m.len() * 8);
                        m[bit / 8] ^= 0x80 >> (bit % 8);
                    }
                }
                1 => m.truncate(rnd() as usize % m.len()),
                _ => {
                    let at = rnd() as usize % m.len();
                    for b in m.iter_mut().skip(at).take(1 + rnd() as usize % 32) {
                        *b = rnd() as u8;
                    }
                }
            }
            let _ = cfa4_j2k::decode(&m);
        }
    }
}

#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("j2k fixtures") else { return };
    for s in SPECS {
        let out = cfa4_testkit::fixtures_dir("j2k").join(format!("{}.{}", s.name, s.ext));
        cfa4_testkit::fixtures::generate_and_report(&format!("j2k/{}", s.name), &[out], || make(&ff, s));
    }
}

/// Planes of an `opj_decompress -o x.rawl` file: each component in turn, little-endian, one byte
/// per sample up to 8 bits and two above.
fn rawl(data: &[u8], planes: &[(usize, u8, bool)]) -> Vec<Vec<i32>> {
    let mut at = 0;
    planes
        .iter()
        .map(|&(n, precision, signed)| {
            let size = if precision > 8 { 2 } else { 1 };
            let bytes = &data[at..at + n * size];
            at += n * size;
            bytes
                .chunks(size)
                .map(|b| match (size, signed) {
                    (1, false) => i32::from(b[0]),
                    (1, true) => i32::from(b[0] as i8),
                    (_, false) => i32::from(u16::from_le_bytes([b[0], b[1]])),
                    _ => i32::from(i16::from_le_bytes([b[0], b[1]])),
                })
                .collect()
        })
        .collect()
}

/// Our encoder's codestreams in OpenJPEG and ffmpeg: lossless ones decode to the input exactly
/// there too, lossy ones (rate controlled, DCI 2K / 4K) within ±1 of our own decode. Skipped
/// without the tools.
#[test]
fn encoded_codestreams_decode_in_openjpeg_and_ffmpeg() {
    use cfa4_j2k::{EncodeOptions, Image, Plane, Progression};
    let ff = cfa4_testkit::require_ffmpeg!();
    let Some(dec) = opj("opj_decompress") else {
        eprintln!("skipped: OpenJPEG tools (libopenjp2-tools) not installed");
        return;
    };
    let dir = cfa4_testkit::fixtures_dir("j2k").join("encoder");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.ppm");
    let st = Command::new(&ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=206x118", "-frames:v", "1", "-vf", "format=rgb24,crop=203:117:0:0", "-pix_fmt", "rgb24"])
        .arg(&input)
        .status();
    assert!(matches!(st, Ok(s) if s.success()));
    let rgb = ppm(&std::fs::read(&input).unwrap());
    let plane = |data: Vec<i32>, precision: u8| Plane { width: 203, height: 117, precision, signed: false, dx: 1, dy: 1, data };
    let rgb8 = Image { width: 203, height: 117, components: rgb.iter().cloned().map(|d| plane(d, 8)).collect(), mct: false };
    // 12-bit for the cinema profiles (the 8-bit pattern scaled up, plus a ramp)
    let rgb12 = Image {
        width: 203,
        height: 117,
        components: rgb.iter().enumerate().map(|(c, d)| plane(d.iter().enumerate().map(|(i, &v)| v * 16 + ((i + c) % 16) as i32).collect(), 12)).collect(),
        mct: false,
    };
    let cases: Vec<(&str, &Image, EncodeOptions, bool)> = vec![
        ("lossless", &rgb8, EncodeOptions::lossless(), true),
        (
            "lossless_cprl",
            &rgb8,
            EncodeOptions {
                progression: Progression::Cprl,
                precincts: vec![(5, 5), (6, 6)],
                code_block: (5, 5),
                tile_part_per_component: true,
                tlm: true,
                ..EncodeOptions::lossless()
            },
            true,
        ),
        ("lossless_rpcl_nomct", &rgb8, EncodeOptions { progression: Progression::Rpcl, mct: false, levels: 3, ..EncodeOptions::lossless() }, true),
        ("lossy", &rgb8, EncodeOptions::lossy(2.0), false),
        ("lossy_budget", &rgb8, EncodeOptions { max_bytes: Some(6_000), ..EncodeOptions::lossy(0.5) }, false),
        ("lossless_budget", &rgb8, EncodeOptions { max_bytes: Some(9_000), ..EncodeOptions::lossless() }, false),
        ("dci2k", &rgb12, EncodeOptions::cinema_2k(24), false),
        ("dci2k_budget", &rgb12, EncodeOptions { max_bytes: Some(12_000), max_component_bytes: Some(5_000), ..EncodeOptions::cinema_2k(24) }, false),
        ("dci4k", &rgb12, EncodeOptions::cinema_4k(24), false),
    ];
    for (name, img, opts, lossless) in cases {
        let data = cfa4_j2k::encode(img, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
        let ours = cfa4_j2k::decode(&data).unwrap();
        if lossless {
            assert_eq!(&ours, img, "{name}: our decode");
        }
        let (cs, out) = (dir.join(format!("{name}.j2c")), dir.join(format!("{name}.rawl")));
        std::fs::write(&cs, &data).unwrap();
        let st = Command::new(&dec).arg("-i").arg(&cs).arg("-o").arg(&out).stdout(Stdio::null()).status();
        assert!(st.is_ok_and(|s| s.success()), "{name}: opj_decompress");
        let shape: Vec<_> = img.components.iter().map(|p| (p.data.len(), p.precision, p.signed)).collect();
        let theirs = rawl(&std::fs::read(&out).unwrap(), &shape);
        for (c, p) in ours.components.iter().enumerate() {
            let worst = p.data.iter().zip(&theirs[c]).map(|(&a, &b)| (a - b).abs()).max().unwrap_or(0);
            assert!(worst <= i32::from(!lossless), "{name}: OpenJPEG component {c} differs by {worst}");
        }
        // ffmpeg's own decoder, for the 8-bit ones
        if img.components[0].precision == 8 {
            let o =
                Command::new(&ff).args(["-v", "error", "-c:v", "jpeg2000", "-i"]).arg(&cs).args(["-pix_fmt", "rgb24", "-f", "rawvideo", "-"]).output().unwrap();
            assert!(o.status.success(), "{name}: ffmpeg");
            for (c, p) in ours.components.iter().enumerate() {
                let worst = p.data.iter().enumerate().map(|(i, &a)| (a - i32::from(o.stdout[3 * i + c])).abs()).max().unwrap_or(0);
                assert!(worst <= i32::from(!lossless), "{name}: ffmpeg component {c} differs by {worst}");
            }
        }
    }
}
