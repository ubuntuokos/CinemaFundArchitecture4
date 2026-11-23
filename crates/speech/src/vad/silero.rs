//! Speech detection with the Silero VAD model (V1-B, `docs/venturi-parity.md` V1).
//!
//! The model (Silero VAD, the build without `If` nodes (`silero_vad_op18_ifless.onnx`), MIT, about 2.8 MB) is **not** part of CFA4 and is never committed or
//! embedded: the user downloads it on first use, after the licence has been shown and accepted
//! once (the owner's rule: CFA4 distributes applications, never models). It lives in the model
//! store like the Whisper models (`<models dir>/silero-vad/`), pinned to a release tag and checked
//! against its SHA-256 before every load. It runs on the CPU through `tract` (pure Rust), 512
//! samples (32 ms) at a time at 16 kHz, with the model's recurrent state carried from window to
//! window, so a clip of any length is read in pieces ([`Stream`]).
//!
//! Silero VAD: <https://github.com/snakers4/silero-vad>, MIT licence, © 2020-present Silero Team.

use std::path::Path;

use crate::models::{self, ModelFile, ModelInfo};

#[cfg(feature = "silero")]
pub use infer::{SileroVad, Stream, silence_regions};

/// Samples per model window at 16 kHz (32 ms).
pub const WINDOW: usize = 512;

const FILE: ModelFile = ModelFile {
    name: "silero_vad_op18_ifless.onnx",
    url: "https://github.com/snakers4/silero-vad/raw/v6.2.3/src/silero_vad/data/silero_vad_op18_ifless.onnx",
    sha256: "7671cd04b004e9076da0d4a7b1a5aec36adf161c39230c1cb94a4fd5db6bbd28",
    size: 2_845_718,
};

static FILES: [ModelFile; 1] = [FILE];

/// The Silero VAD model record: what the host shows (size, source, licence) before the one-time
/// acceptance and the download. The model manager (CFA4-MODEL-MANAGER-001) takes this record as
/// the model's catalogue entry; until it exists the store is `<data dir>/models`.
pub static MODEL: ModelInfo = ModelInfo {
    id: "silero-vad",
    name: "Silero VAD (without If nodes)",
    multilingual: true,
    description: "Voice activity detection: separates speech from clicks and room noise in any language.",
    license: "MIT (Silero VAD, model and code)",
    license_url: "https://github.com/snakers4/silero-vad/blob/master/LICENSE",
    author: "Silero Team",
    source: "https://github.com/snakers4/silero-vad",
    files: &FILES,
};

/// The model is downloaded and its file matches the pinned size and SHA-256.
pub fn installed(models_dir: &Path) -> bool {
    models::installed(models_dir, &MODEL)
}

/// Probability above which a window starts speech, and below which it ends (hysteresis).
const START: f32 = 0.5;
const END: f32 = 0.35;

/// Windows to voiced flags with hysteresis: speech starts at a probability of 0.5 and lasts while
/// it stays at 0.35 or above, so a dip inside a word does not cut it. NaN counts as no speech.
pub fn voiced(probabilities: &[f32]) -> Vec<bool> {
    let mut on = false;
    probabilities
        .iter()
        .map(|&p| {
            on = if on { p >= END } else { p >= START };
            on
        })
        .collect()
}

/// The model's inference (feature `silero`: tract).
#[cfg(feature = "silero")]
mod infer {
    use std::path::Path;

    use tract_onnx::prelude::*;

    use super::{FILE, MODEL, WINDOW, installed, voiced};
    use crate::{SpeechError, models};

    /// Samples of the previous window the model reads in front of each window.
    const CONTEXT: usize = 64;
    const STATE_LEN: usize = 2 * 128;

    type Plan = std::sync::Arc<TypedSimplePlan>;

    /// Replace the model's `If` node by its `then` branch (the 16 kHz network) and drop the `sr`
    /// input. The branch reads the outer graph's values directly, so its nodes move over as they
    /// are and an `Identity` per output keeps the `If`'s output names.
    fn inline_16k_branch(proto: &mut tract_onnx::pb::ModelProto) -> TractResult<()> {
        let fail = |what: &str| TractError::msg(format!("the model's graph is not the expected Silero VAD layout: {what}"));
        let graph = proto.graph.as_mut().ok_or_else(|| fail("no graph"))?;
        let at = graph.node.iter().position(|n| n.op_type == "If").ok_or_else(|| fail("no `If` node"))?;
        let node = graph.node.remove(at);
        let branch = node.attribute.iter().find(|a| a.name == "then_branch").and_then(|a| a.g.as_ref()).ok_or_else(|| fail("no `then_branch`"))?;
        if branch.output.len() != node.output.len() || !branch.input.is_empty() {
            return Err(fail("the branch's outputs or inputs"));
        }
        graph.initializer.extend(branch.initializer.iter().cloned());
        graph.node.extend(branch.node.iter().cloned());
        for (inner, outer) in branch.output.iter().zip(&node.output) {
            graph.node.push(tract_onnx::pb::NodeProto {
                op_type: "Identity".into(),
                name: format!("{outer}_16k"),
                input: vec![inner.name.clone()],
                output: vec![outer.clone()],
                ..Default::default()
            });
        }
        // what only computed the `If`'s condition from `sr` (Reshape, Gather, Equal) goes with it
        let mut dead = std::collections::HashSet::from(["sr".to_string()]);
        while let Some(i) = graph.node.iter().position(|n| n.input.iter().any(|x| dead.contains(x))) {
            let n = graph.node.remove(i);
            dead.extend(n.output);
        }
        graph.input.retain(|i| i.name != "sr");
        Ok(())
    }

    /// A loaded Silero VAD model.
    pub struct SileroVad {
        plan: Plan,
    }

    impl SileroVad {
        /// Load the installed model from `models_dir`. Refuses a missing or modified file: there is no
        /// fallback to another detector here (the caller says so to the user).
        pub fn load(models_dir: &Path) -> Result<Self, SpeechError> {
            if !installed(models_dir) {
                return Err(SpeechError::NotInstalled(MODEL.id.into()));
            }
            Self::from_file(&models::model_dir(models_dir, &MODEL).join(FILE.name))
        }

        fn from_file(path: &Path) -> Result<Self, SpeechError> {
            let err = |e: TractError| SpeechError::Model(format!("silero-vad: {e:#}"));
            // The model reads its sample rate as an input and picks the 16 kHz or the 8 kHz network
            // with one top-level `If` (`sr == 16000`). tract's shape inference walks both branches
            // and fails on the 8 kHz one while the rate is a runtime value, so the graph is cut down
            // to what CFA4 uses: the `then` (16 kHz) branch is inlined in place of the `If`, and the
            // rate input goes away. The plan has two inputs: the audio window and the state.
            let plan = (|| -> TractResult<Plan> {
                let onnx = tract_onnx::onnx();
                let mut proto = onnx.proto_model_for_path(path)?;
                inline_16k_branch(&mut proto)?;
                onnx.model_for_proto_model(&proto)?
                    .with_input_fact(0, f32::fact([1, CONTEXT + WINDOW]).into())?
                    .with_input_fact(1, f32::fact([2, 1, 128]).into())?
                    .into_optimized()?
                    .into_runnable()
            })()
            .map_err(err)?;
            Ok(Self { plan })
        }

        /// A fresh stream (zero state) over one clip.
        pub fn stream(&self) -> Stream<'_> {
            Stream { vad: self, state: vec![0.0; STATE_LEN], context: vec![0.0; CONTEXT], pending: Vec::new() }
        }

        /// Speech probability of every 32 ms window of `audio` (16 kHz mono); the last partial window
        /// is padded with zeros.
        pub fn probabilities(&self, audio: &[f32]) -> Result<Vec<f32>, SpeechError> {
            let mut s = self.stream();
            let mut p = s.push(audio)?;
            p.extend(s.finish()?);
            Ok(p)
        }

        /// One voiced flag per window of `audio` (see [`voiced`]).
        pub fn voiced(&self, audio: &[f32]) -> Result<Vec<bool>, SpeechError> {
            Ok(voiced(&self.probabilities(audio)?))
        }
    }

    /// A clip read in pieces: keeps the model state, the context samples and the part of a window that
    /// the last piece did not fill.
    pub struct Stream<'a> {
        vad: &'a SileroVad,
        state: Vec<f32>,
        context: Vec<f32>,
        pending: Vec<f32>,
    }

    impl Stream<'_> {
        /// Feed samples (16 kHz mono, any length); returns the probability of every window they
        /// complete.
        pub fn push(&mut self, audio: &[f32]) -> Result<Vec<f32>, SpeechError> {
            self.pending.extend(audio.iter().map(|v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }));
            let mut out = Vec::with_capacity(self.pending.len() / WINDOW);
            let mut used = 0;
            while let Some(w) = self.pending.get(used..used + WINDOW) {
                let w = w.to_vec();
                out.push(self.run(&w)?);
                used += WINDOW;
            }
            self.pending.drain(..used);
            Ok(out)
        }

        /// The probability of the last, partial window (zero padded); nothing when there is none.
        pub fn finish(&mut self) -> Result<Vec<f32>, SpeechError> {
            if self.pending.is_empty() {
                return Ok(Vec::new());
            }
            let mut w = std::mem::take(&mut self.pending);
            w.resize(WINDOW, 0.0);
            Ok(vec![self.run(&w)?])
        }

        fn run(&mut self, window: &[f32]) -> Result<f32, SpeechError> {
            let err = |e: TractError| SpeechError::Model(format!("silero-vad: {e:#}"));
            let mut input = Vec::with_capacity(CONTEXT + WINDOW);
            input.extend_from_slice(&self.context);
            input.extend_from_slice(window);
            let x = Tensor::from_shape(&[1, CONTEXT + WINDOW], &input).map_err(err)?;
            let st = Tensor::from_shape(&[2, 1, 128], &self.state).map_err(err)?;
            let out = self.vad.plan.run(tvec!(x.into(), st.into())).map_err(err)?;
            let prob = out.first().and_then(|t| t.to_plain_array_view::<f32>().ok()).and_then(|a| a.iter().next().copied());
            let state: Option<Vec<f32>> = out.get(1).and_then(|t| t.to_plain_array_view::<f32>().ok()).map(|a| a.iter().copied().collect());
            match (prob, state) {
                (Some(p), Some(s)) if s.len() == STATE_LEN => {
                    self.state = s;
                    self.context.clear();
                    self.context.extend_from_slice(window.get(WINDOW - CONTEXT..).unwrap_or_default());
                    Ok(if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 })
                }
                _ => Err(SpeechError::Model("silero-vad: the model's outputs are not (probability, state[2,1,128])".into())),
            }
        }
    }

    /// The silent regions of `audio` (16 kHz mono) found with the model: as [`super::super::silence_regions`]
    /// (same minimum length, padding and edge rules) on the model's 32 ms windows. `threshold_db` of
    /// the parameters is not used: the model decides what is speech.
    pub fn silence_regions(vad: &SileroVad, audio: &[f32], p: &super::super::SilenceParams) -> Result<Vec<(cfa4_time::Tick, cfa4_time::Tick)>, SpeechError> {
        Ok(super::super::silence_regions_from_voiced(&vad.voiced(audio)?, WINDOW, audio.len(), p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "silero")]
    use crate::SpeechError;

    #[test]
    fn record_is_pinned_and_not_embedded() {
        assert!(MODEL.files.iter().all(|f| f.url.starts_with("https://github.com/snakers4/silero-vad/raw/v") && f.sha256.len() == 64));
        assert!(MODEL.license.starts_with("MIT"));
        assert_eq!(MODEL.size(), 2_845_718);
    }

    #[cfg(feature = "silero")]
    #[test]
    fn missing_model_is_not_installed_and_does_not_load() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!installed(dir.path()));
        assert!(matches!(SileroVad::load(dir.path()), Err(SpeechError::NotInstalled(id)) if id == "silero-vad"));
    }

    #[cfg(feature = "silero")]
    #[test]
    fn modified_model_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let d = models::model_dir(dir.path(), &MODEL);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE.name), vec![0u8; FILE.size as usize]).unwrap();
        assert!(!installed(dir.path()));
        assert!(matches!(SileroVad::load(dir.path()), Err(SpeechError::NotInstalled(_))));
    }

    #[test]
    fn hysteresis_keeps_a_dip_inside_speech() {
        let v = voiced(&[0.1, 0.6, 0.4, 0.2, 0.4, 0.6, f32::NAN, 0.3]);
        assert_eq!(v, [false, true, true, false, false, true, false, false]);
    }
}
