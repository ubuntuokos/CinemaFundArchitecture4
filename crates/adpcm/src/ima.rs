//! IMA ADPCM (Interactive Multimedia Association, "Recommended Practices for Enhancing Digital
//! Audio Compatibility in Multimedia Systems", 1992): 4-bit codes, an 89-step size table and an
//! index adaptation table. Two packings: the Microsoft / IMA WAV block (`WAVE_FORMAT_IMA_ADPCM`,
//! 0x0011) and Apple's QuickTime `ima4` packet.

use crate::Error;

/// IMA step sizes.
const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230,
    253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327,
    3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];
/// Step index change per code magnitude.
const INDEX: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// One channel's predictor and step index.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    predictor: i32,
    index: i32,
}

impl Channel {
    fn new(predictor: i32, index: i32) -> Self {
        Self { predictor, index: index.clamp(0, 88) }
    }

    /// Decode one 4-bit code. `exact` computes the difference as `(magnitude + ½)·step / 4` in one
    /// product; otherwise it is built from the step by shifts and adds (the reference practice's
    /// "step/8 + step/4·b0 + step/2·b1 + step·b2", which rounds each term down).
    fn decode(&mut self, code: u8, exact: bool) -> i16 {
        let step = STEPS.get(self.index as usize).copied().unwrap_or(7);
        let diff = if exact {
            ((2 * i32::from(code & 7) + 1) * step) >> 3
        } else {
            let mut diff = step >> 3;
            if code & 4 != 0 {
                diff += step;
            }
            if code & 2 != 0 {
                diff += step >> 1;
            }
            if code & 1 != 0 {
                diff += step >> 2;
            }
            diff
        };
        self.predictor = if code & 8 != 0 { self.predictor - diff } else { self.predictor + diff }.clamp(-32768, 32767);
        self.index = (self.index + INDEX.get(usize::from(code & 7)).copied().unwrap_or(-1)).clamp(0, 88);
        self.predictor as i16
    }
}

/// ffmpeg-compatible difference rounding: the WAV packing computes it exactly, QuickTime with
/// shifts and adds (fitted black-box against ffmpeg's decoders).
const WAV_EXACT: bool = true;
const QT_EXACT: bool = false;

/// Sample frames in an IMA WAV block of `len` bytes (a short final block holds fewer).
pub fn wav_block_frames(len: usize, channels: usize) -> usize {
    let head = 4 * channels;
    if channels == 0 || len < head {
        return 0;
    }
    (len - head) / head * 8 + 1
}

/// Decode one IMA WAV block: per channel a header (predictor i16, step index u8, reserved u8),
/// then for each channel in turn 4 bytes holding 8 codes, low nibble first. The header predictor
/// is the block's first sample.
pub fn decode_wav_block(block: &[u8], channels: usize, out: &mut [Vec<i16>]) -> Result<(), Error> {
    let head = 4usize.checked_mul(channels).ok_or(Error::Invalid("channel count"))?;
    let hdr = block.get(..head).ok_or(Error::Truncated("IMA ADPCM block header"))?;
    if out.len() != channels {
        return Err(Error::Invalid("channel count"));
    }
    let mut state: Vec<Channel> = hdr.chunks_exact(4).map(|h| Channel::new(i32::from(i16::from_le_bytes([h[0], h[1]])), i32::from(h[2]))).collect();
    for (o, s) in out.iter_mut().zip(&state) {
        o.push(s.predictor as i16);
    }
    let body = block.get(head..).unwrap_or_default();
    for group in body.chunks_exact(head) {
        for ((words, st), o) in group.chunks_exact(4).zip(state.iter_mut()).zip(out.iter_mut()) {
            for &b in words {
                o.push(st.decode(b & 15, WAV_EXACT));
                o.push(st.decode(b >> 4, WAV_EXACT));
            }
        }
    }
    Ok(())
}

/// Bytes of one QuickTime `ima4` packet per channel: a 2-byte header and 32 bytes of codes.
pub const QT_PACKET: usize = 34;
/// Sample frames per QuickTime `ima4` packet.
pub const QT_FRAMES: usize = 64;

/// Decode QuickTime `ima4` data: 34-byte packets per channel in turn, each a big-endian header
/// (predictor in the top 9 bits, step index in the low 7) and 64 codes, low nibble first. The
/// header predictor is not a sample. When it equals the top 9 bits of the channel's last decoded
/// sample (`last`, carried from packet to packet), decoding continues from that full-precision
/// sample instead, as an encoder that does not restart at packet boundaries intends.
pub fn decode_qt(data: &[u8], channels: usize, last: &mut [Option<i16>], out: &mut [Vec<i16>]) -> Result<(), Error> {
    let frame = QT_PACKET.checked_mul(channels).ok_or(Error::Invalid("channel count"))?;
    if channels == 0 || out.len() != channels || last.len() != channels {
        return Err(Error::Invalid("channel count"));
    }
    for group in data.chunks_exact(frame) {
        for ((p, o), prev) in group.chunks_exact(QT_PACKET).zip(out.iter_mut()).zip(last.iter_mut()) {
            let h = u16::from_be_bytes([p[0], p[1]]);
            let head = (h & 0xFF80) as i16;
            let predictor = match *prev {
                Some(s) if (s as u16 & 0xFF80) as i16 == head => s,
                _ => head,
            };
            let mut st = Channel::new(i32::from(predictor), i32::from(h & 0x7F));
            for &b in p.get(2..).unwrap_or_default() {
                o.push(st.decode(b & 15, QT_EXACT));
                o.push(st.decode(b >> 4, QT_EXACT));
            }
            *prev = Some(st.predictor as i16);
        }
    }
    Ok(())
}
