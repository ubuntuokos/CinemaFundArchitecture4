mod avutil;

#[allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    improper_ctypes,
    unnecessary_transmutes,
    // bindgen emits libc function declarations (malloc/bcmp/strlen, ...) for
    // symbols pulled in through FFmpeg headers, which trips this rustc lint
    // (warn-by-default since Rust 1.89) under `-D warnings`.
    suspicious_runtime_symbol_definitions,
    clippy::all
)]
pub mod ffi {
    // AVChannelLayout-based constants require FFmpeg 5.1+
    #[cfg(feature = "ffmpeg5_1")]
    pub use crate::avutil::channel_layout::*;
    pub use crate::avutil::{_avutil::*, common::*, error::*, pixfmt::*, rational::*};
    include!(concat!(env!("OUT_DIR"), "/binding.rs"));
}
