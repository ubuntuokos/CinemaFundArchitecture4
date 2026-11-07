//! Official JVT conformance bitstreams (ITU-T H.264.1) for interlaced coding, decoded and compared
//! bit-exactly with their reference YUV. The streams are not part of the repository: point
//! `CFA4_H264_JVT_DIR` at a directory holding one folder per stream (as extracted from the JVT
//! zips); the tests skip when it is unset or a stream is missing.

mod common;

use std::path::PathBuf;

/// (folder, bitstream, reference yuv)
const STREAMS: &[(&str, &str, &str)] = &[
    // MBAFF
    ("CVMA1_Sony_D", "CVMA1_Sony_D.jsv", "CVMA1_Sony_D.yuv"),
    ("CAMA1_Sony_C", "CAMA1_Sony_C.jsv", "CAMA1_Sony_C.yuv"),
    ("CAMA3_Sand_E", "CAMA3_Sand_E.264", "CAMA3_Sand_E.yuv"),
    ("MR6_BT_B", "MR6_BT_B.h264", "MR6_BT_B.yuv"),
    ("MR8_BT_B", "MR8_BT_B.h264", "MR8_BT_B.yuv"),
    ("HPCAMAPALQ_BRCM_B", "HPCAMAPALQ_BRCM_B.264", "HPCAMAPALQ_BRCM_B.yuv"),
    // field pictures (PAFF)
    ("CVPA1_TOSHIBA_B", "CVPA1_TOSHIBA_B.264", "CVPA1_TOSHIBA_B_dec.yuv"),
    ("CAPA1_TOSHIBA_B", "CAPA1_TOSHIBA_B.264", "CAPA1_TOSHIBA_B_dec.yuv"),
    ("CVFI1_Sony_D", "CVFI1_Sony_D.jsv", "CVFI1_Sony_D.yuv"),
    ("CVFI2_Sony_H", "CVFI2_Sony_H.jsv", "CVFI2_Sony_H.yuv"),
    ("CAFI1_SVA_C", "CAFI1_SVA_C.264", "CAFI1_SVA_C_rec.yuv"),
    ("Sharp_MP_Field_1_B", "Sharp_MP_Field_1_B.jvt", "Sharp_MP_Field_1_rec_B.yuv"),
    ("Sharp_MP_Field_2_B", "Sharp_MP_Field_2_B.jvt", "Sharp_MP_Field_2_rec_B.yuv"),
    ("Sharp_MP_Field_3_B", "Sharp_MP_Field_3_B.jvt", "Sharp_MP_Field_3_rec_B.yuv"),
    ("CVSEFDFT3_Sony_E", "CVSEFDFT3_Sony_E.jsv", "CVSEFDFT3_Sony_E-norpt.yuv"),
    ("CVMP_MOT_FLD_L30_B", "CVMP_MOT_FLD_L30_B.26l", "CVMP_MOT_FLD_L30_B_rec.yuv"),
    ("HPCAFL_BRCM_C", "HPCAFL_BRCM_C.264", "HPCAFL_BRCM_C.yuv"),
    // picture-adaptive frame/field with MBAFF frames
    ("CAPAMA3_Sand_F", "CAPAMA3_Sand_F.264", "CAPAMA3_Sand_F.yuv"),
    ("CVMAPAQP3_Sony_E", "CVMAPAQP3_Sony_E.jsv", "CVMAPAQP3_Sony_E.yuv"),
    ("MR9_BT_B", "MR9_BT_B.h264", "MR9_BT_B.yuv"),
];

fn check(name: &str) -> Result<(), String> {
    let Some(dir) = std::env::var_os("CFA4_H264_JVT_DIR").map(PathBuf::from) else {
        eprintln!("{name}: CFA4_H264_JVT_DIR unset, skipped");
        return Ok(());
    };
    let Some(&(folder, bits, yuv)) = STREAMS.iter().find(|s| s.0 == name) else {
        return Err(format!("{name}: unknown stream"));
    };
    let (bits, yuv) = (dir.join(folder).join(bits), dir.join(folder).join(yuv));
    let (Ok(data), Ok(reference)) = (std::fs::read(&bits), std::fs::read(&yuv)) else {
        eprintln!("{name}: stream or reference missing, skipped");
        return Ok(());
    };
    let threads = std::env::var("CFA4_H264_JVT_THREADS").ok().and_then(|t| t.parse().ok()).unwrap_or(1);
    let mut dec = cfa4_h264::Decoder::with_threads(threads);
    let mut out = Vec::new();
    for (i, au) in common::split_access_units(&data).into_iter().enumerate() {
        out.extend(dec.decode(au, i as i64).map_err(|e| format!("{name}: access unit {i}: {e}"))?);
    }
    out.extend(dec.flush());
    let first = out.first().ok_or_else(|| format!("{name}: no pictures"))?;
    let (w, h) = (first.width as usize, first.height as usize);
    common::compare(&out, &reference, w, h).map_err(|e| format!("{name}: {e}"))
}

macro_rules! jvt {
    ($($test:ident => $name:literal),* $(,)?) => {$(
        #[test]
        fn $test() {
            if let Err(e) = check($name) {
                panic!("{e}");
            }
        }
    )*};
}

jvt! {
    jvt_cvma1_sony_d => "CVMA1_Sony_D",
    jvt_cama1_sony_c => "CAMA1_Sony_C",
    jvt_cama3_sand_e => "CAMA3_Sand_E",
    jvt_mr6_bt_b => "MR6_BT_B",
    jvt_mr8_bt_b => "MR8_BT_B",
    jvt_hpcamapalq_brcm_b => "HPCAMAPALQ_BRCM_B",
    jvt_cvpa1_toshiba_b => "CVPA1_TOSHIBA_B",
    jvt_capa1_toshiba_b => "CAPA1_TOSHIBA_B",
    jvt_cvfi1_sony_d => "CVFI1_Sony_D",
    jvt_cvfi2_sony_h => "CVFI2_Sony_H",
    jvt_cafi1_sva_c => "CAFI1_SVA_C",
    jvt_sharp_mp_field_1_b => "Sharp_MP_Field_1_B",
    jvt_sharp_mp_field_2_b => "Sharp_MP_Field_2_B",
    jvt_sharp_mp_field_3_b => "Sharp_MP_Field_3_B",
    jvt_cvsefdft3_sony_e => "CVSEFDFT3_Sony_E",
    jvt_cvmp_mot_fld_l30_b => "CVMP_MOT_FLD_L30_B",
    jvt_hpcafl_brcm_c => "HPCAFL_BRCM_C",
    jvt_capama3_sand_f => "CAPAMA3_Sand_F",
    jvt_cvmapaqp3_sony_e => "CVMAPAQP3_Sony_E",
    jvt_mr9_bt_b => "MR9_BT_B",
}
