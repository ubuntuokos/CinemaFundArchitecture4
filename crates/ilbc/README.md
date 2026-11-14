# cfa4-ilbc

Pure-Rust iLBC speech decoder (RFC 3951, 8 kHz, 20 ms / 15.2 kbit/s and 30 ms / 13.33 kbit/s
frames), ported from WebRTC's fixed-point decoder as packaged by
[libilbc](https://github.com/TimothyGu/libilbc) v3.0.4 (BSD-3-Clause with the WebRTC patent grant,
Copyright (c) 2011-2013 The WebRTC project authors; `LICENSE.webrtc`, `PATENTS.webrtc`).
Codec backlog item CB-12.1.

- The enhancer is off, as in ffmpeg's `ilbc` decoder (itself derived from the WebRTC code), so
  the output matches ffmpeg sample for sample once both decoders have seen the first frame.
- `Decoder::decode` takes one frame (38 or 50 bytes, or `None` for a lost frame) and appends 160
  or 240 samples; lost frames, frames of the wrong size and frames with a detected bit error are
  concealed as WebRTC does.
- `decode_file` reads the RFC 3952 storage format (`#!iLBC20\n` or `#!iLBC30\n`, then frames);
  without a header it assumes 30 ms frames.

## Tests

- `src/oracle_tests.rs`: bit-exact against ffmpeg's `ilbc` decoder on synthetic 20 ms and 30 ms
  streams of valid frames (ffmpeg neither encodes iLBC nor conceals bad frames). ffmpeg starts
  with a zeroed LSF history where WebRTC and RFC 3951 start from the mean LSF vector, so this
  test starts our decoder from ffmpeg's state and checks that `decode_file` converges to it.
- `crates/codecs/tests/ilbc_file.rs`: a `.lbc` file through the media source.
