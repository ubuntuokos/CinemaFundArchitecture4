//! Synthetic failures at the third-party API boundary; no codec implementation is copied.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use symphonia::core::audio::{GenericAudioBufferRef, Position};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::codecs::audio::FinalizeResult;
use symphonia::core::codecs::audio::well_known::CODEC_ID_PCM_S16LE;
use symphonia::core::errors::{Error as SymphoniaError, Result as SymphoniaResult};
use symphonia::core::packet::PacketRef;

struct FaultDecoder {
    inner: Box<dyn AudioDecoder>,
    calls: Arc<AtomicUsize>,
    resets: Arc<AtomicUsize>,
    panic_at: usize,
    error_at: usize,
    panic_reset: bool,
}

impl AudioDecoder for FaultDecoder {
    fn codec_info(&self) -> &CodecInfo {
        self.inner.codec_info()
    }

    fn codec_params(&self) -> &AudioCodecParameters {
        self.inner.codec_params()
    }

    fn reset(&mut self) {
        self.resets.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic_reset, "synthetic decoder reset panic");
        self.inner.reset();
    }

    fn decode_ref(&mut self, packet: &PacketRef<'_>) -> SymphoniaResult<GenericAudioBufferRef<'_>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        assert_ne!(call, self.panic_at, "synthetic decoder panic after state mutation");
        if call == self.error_at {
            return Err(SymphoniaError::DecodeError("synthetic corrupt packet"));
        }
        self.inner.decode_ref(packet)
    }

    fn finalize(&mut self) -> FinalizeResult {
        self.inner.finalize()
    }

    fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
        self.inner.last_decoded()
    }
}

fn injected(params: &AudioCodecParameters, panic_at: usize, error_at: usize, panic_reset: bool) -> (SymphoniaDecoder, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let resets = Arc::new(AtomicUsize::new(0));
    let inner = symphonia::default::get_codecs().make_audio_decoder(params, &AudioDecoderOptions::default()).unwrap();
    let decoder = Box::new(FaultDecoder { inner, calls: calls.clone(), resets: resets.clone(), panic_at, error_at, panic_reset });
    (SymphoniaDecoder { decoder, failed: false }, calls, resets)
}

fn pcm_params() -> AudioCodecParameters {
    let mut params = AudioCodecParameters::new();
    params
        .for_codec(CODEC_ID_PCM_S16LE)
        .with_sample_rate(48_000)
        .with_channels((Position::FRONT_LEFT | Position::FRONT_RIGHT).into())
        .with_bits_per_sample(16)
        .with_max_frames_per_packet(1024);
    params
}

fn assert_panic_error<T>(result: Result<T>) {
    match result {
        Err(CodecError::Decode(message)) => {
            assert!(message.contains("panic"), "{message}");
            assert!(message.contains("reopen"), "{message}");
        }
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("a decoder panic must not produce successful audio"),
    }
}

#[test]
fn packet_panic_is_an_error_and_decoder_is_not_reused() {
    let (inner, calls, resets) = injected(&pcm_params(), 2, 0, false);
    let mut decoder = PacketDecoder { inner: Inner::Symphonia(inner), channels: 0 };
    let pcm = [0x00, 0x20, 0x00, 0xc0]; // left +0.25, right -0.5
    assert_eq!(decoder.decode(&pcm, 0).unwrap(), vec![vec![0.25], vec![-0.5]]);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decoder.decode(&pcm, 1)));
    assert_panic_error(outcome.expect("packet decoder panic escaped"));
    decoder.reset();
    assert_panic_error(decoder.decode(&pcm, 2));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "damaged decoder must never be called again");
    assert_eq!(resets.load(Ordering::SeqCst), 0, "reset must not touch damaged state");
    assert_eq!(decoder.channels, 2);
    let mut fresh = PacketDecoder { inner: Inner::Symphonia(SymphoniaDecoder::new(&pcm_params(), false).unwrap()), channels: 0 };
    assert_eq!(fresh.decode(&pcm, 0).unwrap(), vec![vec![0.25], vec![-0.5]]);
}

#[test]
fn reset_panic_is_contained_and_reported_on_next_decode() {
    let (inner, calls, resets) = injected(&pcm_params(), 0, 0, true);
    let mut decoder = PacketDecoder { inner: Inner::Symphonia(inner), channels: 0 };
    decoder.reset();
    decoder.reset();
    assert_panic_error(decoder.decode(&[0; 4], 0));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(resets.load(Ordering::SeqCst), 1);
}

#[test]
fn ordinary_packet_error_does_not_quarantine_decoder() {
    let (inner, calls, resets) = injected(&pcm_params(), 0, 1, false);
    let mut decoder = PacketDecoder { inner: Inner::Symphonia(inner), channels: 0 };
    let expected = SymphoniaError::DecodeError("synthetic corrupt packet").to_string();
    assert!(matches!(decoder.decode(&[0; 4], 0), Err(CodecError::Decode(message)) if message == expected));
    decoder.reset();
    assert_eq!(decoder.decode(&[0; 4], 1).unwrap(), vec![vec![0.0], vec![0.0]]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(resets.load(Ordering::SeqCst), 1);
}

fn wav() -> Arc<[u8]> {
    cfa4_media::wav::write_wav16(&[0.25; 32_768], 2, 48_000).into()
}

#[test]
fn standalone_panic_after_good_audio_rejects_partial_source() {
    let result = AudioFileSource::decode_with("test.wav", wav(), |params, _| Ok(injected(params, 2, 0, false).0));
    assert_panic_error(result);
    let source = AudioFileSource::decode("test.wav", wav()).unwrap();
    assert_eq!(source.samples.len(), 2);
    assert_eq!(source.samples[0].len(), 16_384);
    assert!(source.samples.iter().flatten().all(|sample| (*sample - 0.25).abs() < 1e-4));
}

#[test]
fn standalone_constructor_panic_is_an_error() {
    assert_panic_error(AudioFileSource::decode_with("test.wav", wav(), |_, _| panic!("synthetic constructor panic")));
}

#[test]
fn standalone_ordinary_corrupt_packet_keeps_existing_recovery() {
    let source = AudioFileSource::decode_with("test.wav", wav(), |params, _| Ok(injected(params, 0, 2, false).0)).unwrap();
    assert!(!source.samples[0].is_empty());
    assert!(source.samples[0].len() < 16_384);
    assert_eq!(source.samples[0].len(), source.samples[1].len());
}

#[test]
fn standalone_mutated_headers_never_unwind() {
    let seed = cfa4_media::wav::write_wav16(&[0.25; 64], 2, 48_000);
    let check = |bytes: Vec<u8>| {
        assert!(std::panic::catch_unwind(|| AudioFileSource::decode("hostile.wav", bytes.into())).is_ok());
    };
    for end in 0..seed.len() {
        check(seed[..end].to_vec());
    }
    for offset in 0..44 {
        for mask in [1, 0x80, 0xff] {
            let mut bytes = seed.clone();
            bytes[offset] ^= mask;
            check(bytes);
        }
    }
}
