//! The `index` task type of Media Intelligence (CFA4-MEDIA-INTELLIGENCE-001 F1, DATA-PG D-6).
//!
//! Building or refreshing the index of one media asset is a queue task like any other: it runs in
//! class P2 (speculative background work that the editing session may pause), or in P1 when the
//! user asks for the index of a clip now. Nothing here touches a database or a media file: this
//! module decides what the task *is* (its id, its class, its resources, its stages and how a
//! checkpoint maps to the next stage); `cfa4-store` keeps the rows (`mi` schema) and the indexer
//! does the work.
//!
//! - **Id:** `mi-index-<asset uuid>-<generation>`. One task per asset and index generation, so
//!   submitting the same job twice is the same id (the queue refuses a duplicate) and the id alone
//!   says which asset and generation a checkpoint row belongs to ([`IndexJob::parse_task_id`]).
//! - **Stages:** [`IndexStage`] in a fixed order. The queue's checkpoint is the number of the last
//!   finished stage; [`IndexStage::after_checkpoint`] gives the next one, so a task that lost its
//!   lease continues from there and never starts over by itself.
//! - **Resumable** always; **start mode** "when resources allow" (it waits quietly, never nags the
//!   user); a missed start simply runs late.

use serde::{Deserialize, Serialize};

use crate::manager::{Class, Request};
use crate::queue::{MissedStart, NewTask, Priority, QueueError, StartMode, Task};

/// Prefix of every index task id.
pub const ID_PREFIX: &str = "mi-index-";
/// The most RAM an index task asks for by default (512 MiB): the whole-file stages stream.
pub const DEFAULT_RAM_BYTES: u64 = 512 << 20;
/// One CPU (thousandths), the default: the indexer must not take the editor's cores.
pub const DEFAULT_CPU_MILLI: u64 = 1000;

/// The stages of one index task, in the order they run. The number is what the queue stores as the
/// task's checkpoint, so existing numbers never change; a new stage gets a new number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexStage {
    /// The sampled content hash (`sha256s1`); until it is done the asset is `pending_hash`.
    Hash,
    /// Duration and stream facts from the container.
    Metadata,
    /// Shot boundaries from the scene detector.
    Shots,
    /// Speech and imported subtitle text.
    Transcript,
}

impl IndexStage {
    /// Every stage, in running order.
    pub const ALL: [IndexStage; 4] = [IndexStage::Hash, IndexStage::Metadata, IndexStage::Shots, IndexStage::Transcript];

    /// The queue checkpoint number of this stage.
    pub fn number(self) -> u64 {
        match self {
            Self::Hash => 0,
            Self::Metadata => 1,
            Self::Shots => 2,
            Self::Transcript => 3,
        }
    }

    pub fn from_number(number: u64) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.number() == number)
    }

    /// The stage to run after the queue's last checkpoint: the first stage when there is none,
    /// `None` when the last stage is done (or the checkpoint is not one of ours).
    pub fn after_checkpoint(checkpoint: Option<u64>) -> Option<Self> {
        match checkpoint {
            None => Self::ALL.first().copied(),
            Some(n) => Self::from_number(n.checked_add(1)?),
        }
    }

    /// The name stored with the stage (`mi.job_checkpoint.detail`, logs).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hash => "hash",
            Self::Metadata => "metadata",
            Self::Shots => "shots",
            Self::Transcript => "transcript",
        }
    }
}

/// What to index: one asset, one index generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexJob {
    /// The `mi.asset` id (a UUID, lower case).
    pub asset_id: String,
    /// The generation being built (`mi.index_generation`, at least 1).
    pub generation: u64,
    /// The user asked for this index now: class P1 instead of P2.
    pub on_demand: bool,
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_digit() || (b'a'..=b'f').contains(&b) })
}

impl IndexJob {
    /// A job; the asset id must be a lower-case UUID and the generation at least 1.
    pub fn new(asset_id: &str, generation: u64, on_demand: bool) -> Result<Self, QueueError> {
        if !is_uuid(asset_id) {
            return Err(QueueError::Invalid { reason: "index job: the asset id must be a lower-case UUID".into() });
        }
        if generation == 0 {
            return Err(QueueError::Invalid { reason: "index job: the generation starts at 1".into() });
        }
        Ok(Self { asset_id: asset_id.to_owned(), generation, on_demand })
    }

    /// The queue task id of this job.
    pub fn task_id(&self) -> String {
        format!("{ID_PREFIX}{}-{}", self.asset_id, self.generation)
    }

    /// The asset id and generation named by an index task id; `None` for any other text.
    pub fn parse_task_id(id: &str) -> Option<(String, u64)> {
        let rest = id.strip_prefix(ID_PREFIX)?;
        let (asset, generation) = rest.split_at_checked(36)?;
        let digits = generation.strip_prefix('-')?;
        let generation: u64 = digits.parse().ok()?;
        (is_uuid(asset) && generation >= 1 && generation.to_string() == digits).then(|| (asset.to_owned(), generation))
    }

    /// The task class: P1 when asked for now, P2 otherwise.
    pub fn class(&self) -> Class {
        if self.on_demand { Class::Background } else { Class::Speculative }
    }

    /// The queue submission for this job with the default resources.
    pub fn new_task(&self) -> NewTask {
        self.new_task_with(DEFAULT_RAM_BYTES, DEFAULT_CPU_MILLI)
    }

    /// The queue submission with an explicit RAM and CPU request (thousandths of a CPU).
    pub fn new_task_with(&self, ram_bytes: u64, cpu_milli: u64) -> NewTask {
        NewTask {
            id: self.task_id(),
            request: Request { class: self.class(), ram_bytes, cpu_milli, gpu: None },
            priority: if self.on_demand { Priority::Normal } else { Priority::Low },
            start: StartMode::WhenResources,
            missed: MissedStart::RunLate,
            resumable: true,
            time_zone: None,
        }
    }
}

/// The stage an index task continues with, from the checkpoint the queue kept for it.
pub fn resume_stage(task: &Task) -> Option<IndexStage> {
    IndexStage::after_checkpoint(task.checkpoint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{Queue, QueueConfig, State};

    const ASSET: &str = "0190a000-0000-7000-8000-000000000001";

    #[test]
    fn stage_numbers_are_frozen_and_checkpoints_continue_in_order() {
        assert_eq!(IndexStage::ALL.map(IndexStage::number), [0, 1, 2, 3]);
        for s in IndexStage::ALL {
            assert_eq!(IndexStage::from_number(s.number()), Some(s));
        }
        assert_eq!(IndexStage::after_checkpoint(None), Some(IndexStage::Hash));
        assert_eq!(IndexStage::after_checkpoint(Some(0)), Some(IndexStage::Metadata));
        assert_eq!(IndexStage::after_checkpoint(Some(2)), Some(IndexStage::Transcript));
        assert_eq!(IndexStage::after_checkpoint(Some(3)), None, "all stages done");
        assert_eq!(IndexStage::after_checkpoint(Some(u64::MAX)), None, "a hostile checkpoint is not a stage");
        assert_eq!(IndexStage::from_number(99), None);
    }

    #[test]
    fn the_task_id_names_the_asset_and_the_generation() {
        let job = IndexJob::new(ASSET, 7, false).unwrap();
        assert_eq!(job.task_id(), format!("mi-index-{ASSET}-7"));
        assert_eq!(IndexJob::parse_task_id(&job.task_id()), Some((ASSET.to_owned(), 7)));
        for bad in [
            "",
            "export-1",
            "mi-index-",
            &format!("mi-index-{ASSET}"),
            &format!("mi-index-{ASSET}-"),
            &format!("mi-index-{ASSET}-0"),
            &format!("mi-index-{ASSET}-07"),
            &format!("mi-index-{ASSET}-+7"),
            &format!("mi-index-{ASSET}-7x"),
            &format!("mi-index-{ASSET}-99999999999999999999999"),
            &format!("mi-index-{}-1", ASSET.to_uppercase()),
            "mi-index-é0190a000-0000-7000-8000-00000000000-1",
        ] {
            assert_eq!(IndexJob::parse_task_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_hostile_job_is_an_error_not_a_task() {
        for asset in ["", "nope", &ASSET.to_uppercase(), &format!("{ASSET}0"), "0190a000-0000-7000-8000-00000000000g"] {
            assert!(IndexJob::new(asset, 1, false).is_err(), "{asset:?}");
        }
        assert!(IndexJob::new(ASSET, 0, false).is_err());
    }

    #[test]
    fn speculative_by_default_and_background_when_asked_for() {
        let idle = IndexJob::new(ASSET, 1, false).unwrap().new_task();
        assert_eq!((idle.request.class, idle.priority), (Class::Speculative, Priority::Low));
        let now = IndexJob::new(ASSET, 1, true).unwrap().new_task();
        assert_eq!((now.request.class, now.priority), (Class::Background, Priority::Normal));
        for t in [idle, now] {
            assert!(t.resumable && t.request.gpu.is_none());
            assert_eq!(t.start, StartMode::WhenResources);
            assert_eq!((t.request.ram_bytes, t.request.cpu_milli), (DEFAULT_RAM_BYTES, DEFAULT_CPU_MILLI));
        }
    }

    #[test]
    fn the_queue_accepts_the_job_once() {
        let mut queue = Queue::new(QueueConfig::default());
        let job = IndexJob::new(ASSET, 1, false).unwrap();
        let task = queue.submit(job.new_task(), 1_000).unwrap().clone();
        assert_eq!(task.state, State::Queued);
        assert_eq!(resume_stage(&task), Some(IndexStage::Hash));
        assert!(queue.submit(job.new_task(), 2_000).is_err(), "the same asset and generation is the same task");
        assert!(queue.submit(IndexJob::new(ASSET, 2, false).unwrap().new_task(), 3_000).is_ok(), "a new generation is a new task");
    }
}
