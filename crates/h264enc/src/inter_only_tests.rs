//! `EncoderConfig::inter_only`: P pictures code no intra macroblocks.

use super::*;
use crate::mbinfo::MbKind;

const W: usize = 64;
const H: usize = 48;

/// A smooth gradient, then a picture with nothing in common with it (high-frequency noise), so
/// the normal mode decision picks intra blocks for the second picture.
fn pictures() -> [(Vec<u8>, Vec<u8>, Vec<u8>); 2] {
    let grad = (0..W * H).map(|i| ((i % W) * 3 + (i / W)) as u8).collect();
    let mut s = 0x2545_f491_u32;
    let noise = (0..W * H)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 24) as u8
        })
        .collect();
    let c = vec![128u8; W * H / 4];
    [(grad, c.clone(), c.clone()), (noise, c.clone(), c)]
}

/// The macroblock kinds of the P picture that follows the IDR.
fn p_kinds(preset: Preset, inter_only: bool) -> Vec<MbKind> {
    let mut cfg = EncoderConfig::new(W as u32, H as u32, 25, 1);
    cfg.preset = preset;
    cfg.rate = RateControl::Qp(26);
    cfg.bframes = 0;
    cfg.scenecut = false;
    cfg.threads = 1;
    cfg.slices = 1;
    cfg.inter_only = inter_only;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut types = Vec::new();
    for (i, (y, u, v)) in pictures().iter().enumerate() {
        let f = YuvFrame { y, u, v, y_stride: W, uv_stride: W / 2 };
        types.extend(enc.try_encode(&f, i as i64).unwrap().iter().map(|p| p.frame_type));
    }
    types.extend(enc.flush().iter().map(|p| p.frame_type));
    assert_eq!(types, [FrameType::Idr, FrameType::P]);
    let last = enc.dpb.iter().max_by_key(|r| r.poc).unwrap();
    last.mbs.iter().map(|m| m.kind).collect()
}

#[test]
fn p_pictures_code_intra_blocks_by_default() {
    for preset in [Preset::Speed, Preset::Quality] {
        assert!(p_kinds(preset, false).iter().any(|k| k.is_intra()), "{preset:?}: no intra block in the P picture");
    }
}

#[test]
fn inter_only_codes_no_intra_block_in_p_pictures() {
    for preset in [Preset::Speed, Preset::Balanced, Preset::Quality] {
        let kinds = p_kinds(preset, true);
        assert_eq!(kinds.len(), (W / 16) * (H / 16));
        assert!(kinds.iter().all(|k| matches!(k, MbKind::PInter | MbKind::PSkip)), "{preset:?}: {kinds:?}");
    }
}
