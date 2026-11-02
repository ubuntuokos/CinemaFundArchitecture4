//! 9-, 12- and 14-bit decoding (#164), bit-exact against ffmpeg's decoder. libx264 encodes only 8
//! and 10 bits, so these streams are libx264 fixtures with their SPS rewritten to a deeper bit depth
//! (see `common::DepthVariant`); skipped when ffmpeg is absent.

mod common;

macro_rules! depth_tests {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                match common::check_fixture(stringify!($name)) {
                    Ok(true) => {}
                    Ok(false) => eprintln!("skipped {}", stringify!($name)),
                    Err(e) => panic!("{e}"),
                }
            }
        )*
    };
}

depth_tests!(
    d9_weightp2,
    d9_constrained_intra,
    d9_high422_cavlc,
    d9_high444_intra,
    d9_mbaff_cabac,
    d9_lossless_420,
    d12_cabac_b,
    d12_cavlc,
    d12_422,
    d12_444_cavlc,
    d12_mbaff,
    d12_lossless_444,
    d14_intra,
    d14_qp_low,
    d14_qp_high,
    d14_444,
    d14_cabac_b,
    d14_lossless_420,
);

/// Every variant really carries its bit depth (and the lossless ones their lowest QP).
#[test]
fn variants_have_the_requested_depth() {
    for v in common::DEPTH_VARIANTS {
        let Some((h264, _)) = common::ensure_depth(v) else { return };
        let pics = common::decode_file(&h264).unwrap_or_else(|(au, e, _)| panic!("{}: error at {au}: {e}", v.name));
        assert!(pics.iter().all(|p| u32::from(p.bit_depth) == v.depth), "{}", v.name);
    }
}
