//! Speex files from the reference `speexenc` decoded here and by the reference `speexdec`
//! (libspeex 1.2.1): every 16-bit sample must be equal. Covers every narrowband quality
//! (submodes 1–8), wideband and ultra-wideband qualities, VBR, ABR, VAD + DTX, several frames
//! per packet and in-band stereo. Skips when ffmpeg (the source generator) or the Speex tools
//! (`speexenc`, `speexdec` on the PATH, or `CFA4_SPEEXENC` / `CFA4_SPEEXDEC`) are missing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_speex::{Decoder, Header};

/// An Ogg page: granule position, completed packets, end-of-stream flag.
struct Page {
    granule: i64,
    packets: Vec<Vec<u8>>,
    eos: bool,
}

fn pages(b: &[u8]) -> Vec<Page> {
    let mut out = Vec::new();
    let mut partial = Vec::new();
    let mut at = 0;
    while at + 27 <= b.len() && &b[at..at + 4] == b"OggS" {
        let flags = b[at + 5];
        let granule = i64::from_le_bytes(b[at + 6..at + 14].try_into().unwrap());
        let nseg = b[at + 26] as usize;
        let lacing = &b[at + 27..at + 27 + nseg];
        let mut data = at + 27 + nseg;
        let mut packets = Vec::new();
        for &l in lacing {
            partial.extend_from_slice(&b[data..data + l as usize]);
            data += l as usize;
            if l < 255 {
                packets.push(std::mem::take(&mut partial));
            }
        }
        out.push(Page { granule, packets, eos: flags & 4 != 0 });
        at = data;
    }
    out
}

/// Decode an Ogg Speex file and trim it exactly as `speexdec` (1.2.1) does.
fn speexdec_like(file: &[u8]) -> (Header, Vec<i16>) {
    let pages = pages(file);
    let header = Header::parse(&pages[0].packets[0]).unwrap();
    let mut dec = Decoder::new(&header);
    let (fs, la, ch) = (dec.frame_size() as i64, dec.lookahead() as i64, header.channels as usize);
    let nframes = i64::from(header.frames_per_packet);
    let mut out = Vec::new();
    let mut packet_count = 0u32;
    let mut last_granule = 0i64;
    for page in &pages {
        let n = page.packets.len() as i64;
        let mut skip = 0i64;
        if page.granule > 0 {
            skip = fs * (n * fs * nframes - (page.granule - last_granule)) / fs;
            if page.eos {
                skip = -skip;
            }
        }
        last_granule = page.granule;
        let mut packet_no = 0;
        for p in &page.packets {
            packet_count += 1;
            if packet_count <= 2 + header.extra_headers {
                continue;
            }
            packet_no += 1;
            let pcm = dec.decode_packet(p);
            for (j, frame) in pcm.chunks(fs as usize * ch).enumerate() {
                let j = j as i64;
                let (mut offset, mut size) = (0i64, fs);
                if packet_no == 1 && j == 0 && skip > 0 {
                    size -= skip + la;
                    offset = skip + la;
                }
                if packet_no == n && skip < 0 {
                    let len = nframes * fs + skip + la;
                    size = (len - j * fs).clamp(0, fs);
                }
                if size > 0 {
                    out.extend_from_slice(&frame[offset as usize * ch..(offset + size) as usize * ch]);
                }
            }
        }
    }
    (header, out)
}

fn tool(name: &str) -> Option<PathBuf> {
    cfa4_testkit::oracle::find_tool(name)
}

fn run(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// A source with speech-like structure: voiced harmonics, a glide, noise, silence, a click.
fn source(ff: &Path, dir: &Path, rate: u32, channels: u32) -> Option<PathBuf> {
    let wav = dir.join(format!("speex_src_{rate}_{channels}.wav"));
    let expr = "if(lt(t,1),0.3*sin(2*PI*140*t)*(1+0.5*sin(2*PI*900*t))*sin(2*PI*3*t),if(lt(t,2),0.4*sin(2*PI*(200+800*t)*t),if(lt(t,2.6),(random(0)-0.5)*0.5,if(lt(t,3.4),0,0.6*sin(2*PI*2500*t)*exp(-8*(t-3.4))))))";
    let exprs = if channels == 2 { format!("{expr}|0.7*({expr})") } else { expr.to_string() };
    cfa4_testkit::fixtures::generate(&wav, |tmp| {
        run(Command::new(ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("aevalsrc=exprs='{exprs}':s={rate}:d=4"))
            .args(["-c:a", "pcm_s16le", "-f", "wav"])
            .arg(tmp))
    })
}

#[test]
fn reference_encoded_files_match_speexdec() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (Some(enc), Some(dec)) = (tool("speexenc"), tool("speexdec")) else {
        eprintln!("SKIPPED (speex oracle): speexenc / speexdec are not installed");
        return;
    };
    let dir = cfa4_testkit::fixtures::fixtures_dir("speex");
    let mut cases: Vec<(String, u32, u32, Vec<String>)> = Vec::new();
    for q in 0..=10 {
        cases.push((format!("nb_q{q}"), 8000, 1, vec!["-n".into(), "--quality".into(), q.to_string()]));
    }
    for q in [0, 1, 2, 3, 4, 6, 8, 10] {
        cases.push((format!("wb_q{q}"), 16000, 1, vec!["-w".into(), "--quality".into(), q.to_string()]));
    }
    for q in [0, 3, 6, 10] {
        cases.push((format!("uwb_q{q}"), 32000, 1, vec!["-u".into(), "--quality".into(), q.to_string()]));
    }
    let more: [(&str, u32, u32, &[&str]); 9] = [
        ("nb_vbr", 8000, 1, &["-n", "--vbr", "--quality", "6"]),
        ("nb_abr", 8000, 1, &["-n", "--abr", "9000"]),
        ("nb_dtx", 8000, 1, &["-n", "--vad", "--dtx", "--vbr", "--quality", "4"]),
        ("nb_nframes", 8000, 1, &["-n", "--nframes", "3", "--quality", "5"]),
        ("wb_vbr_dtx", 16000, 1, &["-w", "--vbr", "--vad", "--dtx"]),
        ("wb_nframes", 16000, 1, &["-w", "--nframes", "4"]),
        ("uwb_vbr", 32000, 1, &["-u", "--vbr", "--quality", "7"]),
        ("nb_stereo", 8000, 2, &["-n", "--quality", "5"]),
        ("wb_stereo", 16000, 2, &["-w", "--quality", "8", "--nframes", "2"]),
    ];
    for (name, rate, ch, args) in more {
        cases.push((name.into(), rate, ch, args.iter().map(|s| s.to_string()).collect()));
    }
    let mut failed = Vec::new();
    for (name, rate, ch, args) in &cases {
        let Some(src) = source(&ff, &dir, *rate, *ch) else { panic!("{name}: ffmpeg could not write the source") };
        let spx = dir.join(format!("{name}.spx"));
        let spx = cfa4_testkit::fixtures::generate(&spx, |tmp| run(Command::new(&enc).args(args).arg(&src).arg(tmp)))
            .unwrap_or_else(|| panic!("{name}: speexenc failed"));
        let raw = dir.join(format!("{name}.raw"));
        let raw = cfa4_testkit::fixtures::generate(&raw, |tmp| {
            let t = tmp.with_extension("raw");
            run(Command::new(&dec).arg(&spx).arg(&t)) && std::fs::rename(&t, tmp).is_ok()
        })
        .unwrap_or_else(|| panic!("{name}: speexdec failed"));
        let want: Vec<i16> = std::fs::read(&raw).unwrap().as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        let (header, got) = speexdec_like(&std::fs::read(&spx).unwrap());
        assert_eq!(header.channels, *ch, "{name}");
        if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            let lo = i.saturating_sub(3);
            failed.push(format!("{name}: sample {i} differs (ours {:?}, speexdec {:?})", &got[lo..(i + 4).min(got.len())], &want[lo..(i + 4).min(want.len())]));
        } else if got.len() != want.len() {
            failed.push(format!("{name}: {} samples, speexdec {}", got.len(), want.len()));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
