use crate::ffi;
use std::ffi::c_double;

/// Get a string containing a timestamp representation.
pub fn ts2str(ts: i64) -> String {
    if ts == ffi::AV_NOPTS_VALUE {
        "NOPTS".to_string()
    } else {
        ts.to_string()
    }
}

/// Get a string containing a timestamp time representation.
pub fn ts2timestr(ts: i64, tb: ffi::AVRational) -> String {
    if ts == ffi::AV_NOPTS_VALUE {
        "NOPTS".to_string()
    } else {
        format!("{:.6}", ffi::av_q2d(tb) * ts as c_double)
    }
}
