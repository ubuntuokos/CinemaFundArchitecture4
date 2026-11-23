# cfa4-speech

Speech-to-text for CFA4's Text panel ▸ Transcript (L2). User-facing behaviour, commands and
model handling: [docs/transcripts.md](../../docs/transcripts.md).

- `Transcriber` trait (mono 16 kHz in, word-timed `cfa4_project::Transcript` out) and
  `FixedTranscriber` for tests and agents.
- `models`: Whisper model catalogue (pinned revisions, SHA-256, sizes, licences) and the check of
  installed files. Downloads run in the confined `cfa4-fetch` process the engine starts
  (CFA4-SECURITY-001 D5); this crate has no network code. Weights are never bundled.
- `whisper` (feature `whisper`): Whisper inference on candle (CPU, pure Rust).
- `vad::silero` (feature `silero`): Silero VAD speech detection on tract (CPU, pure Rust). The model
  (MIT, about 2.8 MB) is never bundled: the record `vad::silero::MODEL` (source, licence, pinned URL
  and SHA-256) is what the host shows before the user's one-time acceptance and the download; the
  file is checked against its SHA-256 on every load, and a missing model is an error, never a
  fallback to the energy detector. The record is the hand-over point for the model manager
  (CFA4-MODEL-MANAGER-001), which will own the store and the acceptance.
- `mel`, `vad`, `diarize`: log-mel front end, word-bound tightening, MFCC-clustering speaker labels.

The `whisper` and `silero` features are off by default and are never enabled for wasm (`cargo xtask wasm` checks the
crate without them).

## References

Implemented from the published description of the model: A. Radford et al., "Robust Speech
Recognition via Large-Scale Weak Supervision" (OpenAI, 2022), the model cards and configuration
files published with the weights (`config.json`, `generation_config.json`, `tokenizer.json`), and
the candle tensor library's public API. Weights: OpenAI Whisper, MIT licence.
