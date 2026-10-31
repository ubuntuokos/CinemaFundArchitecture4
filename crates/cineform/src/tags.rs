//! CineForm tag numbers (`CODEC_TAG_*` of the SDK's `codec.h`). A negative tag on the wire marks
//! an optional tag; these are the absolute values.

pub const SAMPLE: u16 = 1;
pub const INDEX: u16 = 2;
pub const MARKER: u16 = 4;
pub const TRANSFORM_TYPE: u16 = 10;
pub const NUM_CHANNELS: u16 = 12;
pub const NUM_WAVELETS: u16 = 13;
pub const FRAME_WIDTH: u16 = 20;
pub const FRAME_HEIGHT: u16 = 21;
pub const LOWPASS_WIDTH: u16 = 27;
pub const LOWPASS_HEIGHT: u16 = 28;
pub const PIXEL_OFFSET: u16 = 33;
pub const QUANTIZATION: u16 = 34;
pub const PIXEL_DEPTH: u16 = 35;
pub const BAND_WIDTH: u16 = 49;
pub const BAND_HEIGHT: u16 = 50;
pub const BAND_SUBBAND: u16 = 51;
pub const BAND_ENCODING: u16 = 52;
pub const BAND_QUANTIZATION: u16 = 53;
pub const BAND_HEADER: u16 = 55;
pub const CHANNEL: u16 = 62;
pub const PRECISION: u16 = 70;
pub const BAND_CODING_FLAGS: u16 = 72;
pub const PEAK_LEVEL: u16 = 74;
pub const PEAK_TABLE_OFFSET_L: u16 = 75;
pub const PEAK_TABLE_OFFSET_H: u16 = 76;
pub const VERSION: u16 = 79;
pub const PRESCALE_TABLE: u16 = 83;
pub const ENCODED_FORMAT: u16 = 84;
pub const FRAME_DISPLAY_HEIGHT: u16 = 85;

/// Tags at or below this carry a plain value; above it the value is a chunk size.
pub const LAST_NON_SIZED: u16 = 0xff;
/// `0x2000..0x3fff`: size of the following subband / level / sample (informational).
pub const CHUNK24BIT: u16 = 0x2000;
/// `0x2300`: an uncompressed sample.
pub const UNCOMPRESS: u16 = 0x2300;
/// `0x4000..`: a skippable chunk (`0x6000..`: 24-bit size).
pub const CHUNK: u16 = 0x4000;

/// Marker value (`CODEC_COEFFICIENT_START_CODE`) that precedes the lowpass coefficients.
pub const COEFFICIENT_START_CODE: u16 = 0x0f0f;

/// `SAMPLE` types.
pub const SAMPLE_TYPE_GROUP: u16 = 2;

/// `TRANSFORM_TYPE` values.
pub const TRANSFORM_TYPE_SPATIAL: u16 = 0;

/// `BAND_ENCODING` values.
pub const BAND_ENCODING_RUNLENGTHS: u16 = 3;
pub const BAND_ENCODING_16BIT: u16 = 4;
