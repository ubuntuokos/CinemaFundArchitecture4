//! External oracle: GStreamer's RFC 4175 payloader and depayloader (`rtpvrawpay`,
//! `rtpvrawdepay`, gst-plugins-good), in both directions. The RTP packets go through files with
//! RFC 4571 framing (`rtpstreampay` / `rtpstreamdepay`: a 16-bit length before each packet), so
//! the comparison is deterministic and needs no network. GStreamer is used only here, as a test
//! oracle (plan D8). Skips without `gst-launch-1.0`; fails with `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_st2110::video::{Colorimetry, Depth, FrameRate, Sampling};
use cfa4_st2110::{Depacketizer, Packetizer, VideoFormat};
use cfa4_testkit::oracle;

const W: u32 = 320;
const H: u32 = 240;
/// GStreamer's UYVP is the 4:2:2 10-bit pgroup layout, 800 bytes per row at this width.
const FRAME: usize = (W as usize * 5 / 2) * H as usize;
const RTP_CAPS: &str = "application/x-rtp,media=video,clock-rate=90000,encoding-name=RAW,sampling=YCbCr-4:2:2,depth=(string)10,width=(string)320,height=(string)240,colorimetry=BT709,payload=98";

fn gst() -> Option<PathBuf> {
    let g = oracle::find_tool("gst-launch-1.0");
    if g.is_none() {
        oracle::skip("st2110 gst oracle", "gst-launch-1.0 not found (gstreamer1.0-tools, -plugins-base, -plugins-good)");
    }
    g
}

fn format() -> VideoFormat {
    VideoFormat::progressive(Sampling::YCbCr422, Depth::D10, W, H, FrameRate::new(25, 1).unwrap(), Colorimetry::Bt709)
}

fn run(gst: &Path, pipeline: &str) {
    let out = Command::new(gst).arg("-q").args(pipeline.split_whitespace()).output().expect("run gst-launch-1.0");
    assert!(out.status.success(), "gst-launch-1.0 failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// GStreamer packs, we reassemble: every frame must equal the raw frame GStreamer wrote.
#[test]
fn gstreamer_packets_reassemble_bit_exact() {
    let Some(gst) = gst() else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("st2110-gst-pay");
    let (raw, rtp) = (dir.join("frames.uyvp"), dir.join("frames.rtp"));
    let frames = 3;
    run(
        &gst,
        &format!(
            "videotestsrc num-buffers={frames} pattern=smpte ! video/x-raw,format=UYVP,width={W},height={H},framerate=25/1 ! tee name=t \
             t. ! queue ! filesink location={} \
             t. ! queue ! rtpvrawpay mtu=1452 pt=98 ! rtpstreampay ! filesink location={}",
            raw.display(),
            rtp.display()
        ),
    );
    let stream = std::fs::read(&rtp).unwrap();
    let mut d = Depacketizer::new(format()).unwrap();
    let mut at = 0;
    while at + 2 <= stream.len() {
        let n = u16::from_be_bytes([stream[at], stream[at + 1]]) as usize;
        d.push(&stream[at + 2..at + 2 + n]).expect("a well-formed RFC 4175 packet");
        at += 2 + n;
    }
    assert_eq!(at, stream.len());
    let raw = std::fs::read(&raw).unwrap();
    assert_eq!(raw.len(), frames * FRAME);
    for i in 0..frames {
        let img = d.pop().unwrap_or_else(|| panic!("frame {i} missing"));
        assert!(img.complete, "frame {i} incomplete");
        assert!(img.data == raw[i * FRAME..(i + 1) * FRAME], "frame {i} differs");
    }
    assert_eq!(d.lost_packets(), 0);
}

/// We pack, GStreamer reassembles: its frames must equal ours.
#[test]
fn our_packets_depayload_in_gstreamer() {
    let Some(gst) = gst() else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("st2110-gst-depay");
    let (rtp, out) = (dir.join("ours.rtp"), dir.join("out.uyvp"));
    let mut p = Packetizer::new(format(), 98, 0x1234_5678, 0xFFF0).unwrap();
    let frames: Vec<Vec<u8>> = (0..2).map(|k| (0..p.image_len(false)).map(|i| ((i * 7 + k * 13) % 251) as u8).collect()).collect();
    let mut stream = Vec::new();
    for (k, f) in frames.iter().enumerate() {
        p.packetize(f, 3600 * k as u32, false, |pkt| {
            stream.extend_from_slice(&(pkt.len() as u16).to_be_bytes());
            stream.extend_from_slice(pkt);
        })
        .unwrap();
    }
    std::fs::write(&rtp, &stream).unwrap();
    run(
        &gst,
        &format!(
            "filesrc location={} ! application/x-rtp-stream ! rtpstreamdepay ! {RTP_CAPS} ! rtpvrawdepay ! filesink location={}",
            rtp.display(),
            out.display()
        ),
    );
    let got = std::fs::read(&out).unwrap();
    assert_eq!(got.len(), frames.len() * FRAME);
    for (k, f) in got.chunks(FRAME).enumerate() {
        assert!(f == frames[k].as_slice(), "GStreamer's frame {k} differs from ours");
    }
}
