//! `TestRecorder`: a deterministic synthetic backend. It writes nothing (no I/O in IN-1): it counts
//! frames and advances the readable extent exactly as a real backend must, and it can inject faults,
//! so the whole core is testable without hardware.

use std::collections::BTreeMap;

use cfa4_time::{FrameRate, Tick};

use crate::contract::{MetadataKind, RecordHandle, RecordJob, RecordReport, Recorder};
use crate::error::IngestError;
use crate::events::IngestEvent;
use crate::extent::{Extent, ExtentView};
use crate::session::{Session, State, Trigger};

pub struct TestRecorder {
    next_id: u64,
    active: Option<Active>,
    done: BTreeMap<u64, Active>,
    events: Vec<IngestEvent>,
}

struct Active {
    id: u64,
    job: RecordJob,
    rate: FrameRate,
    session: Session,
    extent: Extent,
    frames: u64,
    dropped: u64,
    markers: Vec<(Tick, String)>,
}

impl Default for TestRecorder {
    fn default() -> Self {
        TestRecorder { next_id: 1, active: None, done: BTreeMap::new(), events: Vec::new() }
    }
}

impl TestRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything published so far, oldest first.
    pub fn events(&self) -> &[IngestEvent] {
        &self.events
    }

    pub fn state(&self, h: RecordHandle) -> Option<State> {
        self.get(h.0).map(|a| a.session.state())
    }

    pub fn session(&self, h: RecordHandle) -> Option<&Session> {
        self.get(h.0).map(|a| &a.session)
    }

    fn get(&self, id: u64) -> Option<&Active> {
        self.active.as_ref().filter(|a| a.id == id).or_else(|| self.done.get(&id))
    }

    fn active_mut(&mut self, h: RecordHandle) -> Result<&mut Active, IngestError> {
        self.active.as_mut().filter(|a| a.id == h.0).ok_or(IngestError::UnknownHandle(h.0))
    }

    /// The source delivers `frames` good frames and `dropped` lost ones. Only good frames make the
    /// extent grow.
    pub fn push_frames(&mut self, h: RecordHandle, frames: u64, dropped: u64) -> Result<(), IngestError> {
        let a = self.active_mut(h)?;
        if a.session.state() != State::Recording {
            return Err(IngestError::InvalidTransition { from: a.session.state(), trigger: Trigger::Started });
        }
        // Validate everything before changing anything: a refused push leaves the recording as it was.
        let total = a.frames.checked_add(frames).ok_or_else(|| IngestError::Backend("frame count overflow".into()))?;
        let end = a.rate.tick_of(i64::try_from(total).map_err(|_| IngestError::Backend("frame count too large".into()))?);
        a.extent.advance(frames, end)?;
        a.frames = total;
        a.dropped = a.dropped.saturating_add(dropped);
        let ev = IngestEvent {
            session: a.session.id(),
            seq: a.session.journal().len() as u64,
            at: end,
            kind: crate::events::EventKind::GrowingMediaUpdated { frames: a.frames, available: end },
        };
        self.events.push(ev);
        Ok(())
    }

    /// A manual transition of a session (quarantine, release, ...): it is journaled and published like
    /// any other. `reason` is required where the state machine says so.
    pub fn transition(&mut self, h: RecordHandle, trigger: Trigger, reason: &str) -> Result<State, IngestError> {
        let a = match self.active.as_mut().filter(|a| a.id == h.0) {
            Some(a) => a,
            None => self.done.get_mut(&h.0).ok_or(IngestError::UnknownHandle(h.0))?,
        };
        let at = a.extent.view().available;
        let ev = a.session.apply(trigger, at, reason)?;
        let state = a.session.state();
        self.events.push(ev);
        Ok(state)
    }

    /// Inject a fault: the recording is interrupted, its readable part is kept (`Recoverable`).
    pub fn inject_fault(&mut self, h: RecordHandle, reason: &str) -> Result<(), IngestError> {
        let a = self.active_mut(h)?;
        let at = a.extent.view().available;
        let ev = a.session.apply(Trigger::Fault, at, reason)?;
        a.extent.interrupt()?;
        let ev2 = a.session.apply(Trigger::Preserve, at, "partial recording preserved")?;
        let id = a.id;
        self.events.push(ev);
        self.events.push(ev2);
        if let Some(a) = self.active.take_if(|a| a.id == id) {
            self.done.insert(id, a);
        }
        Ok(())
    }
}

impl Recorder for TestRecorder {
    fn start(&mut self, job: RecordJob) -> Result<RecordHandle, IngestError> {
        job.validate()?;
        if self.active.is_some() {
            return Err(IngestError::AlreadyActive);
        }
        let rate = job.video.first().map(|v| v.frame_rate).unwrap_or(FrameRate::FPS_25).sane();
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut session = Session::new(id);
        let at = job.start_timecode.unwrap_or(Tick::ZERO);
        let e1 = session.apply(Trigger::Begin, at, "")?;
        let e2 = session.apply(Trigger::Started, at, "")?;
        self.events.push(e1);
        self.events.push(e2);
        self.active = Some(Active { id, job, rate, session, extent: Extent::default(), frames: 0, dropped: 0, markers: Vec::new() });
        Ok(RecordHandle(id))
    }

    fn marker(&mut self, handle: RecordHandle, at: Tick, label: &str) -> Result<(), IngestError> {
        let a = self.active_mut(handle)?;
        if at < Tick::ZERO {
            return Err(IngestError::InvalidJob("negative marker time".into()));
        }
        a.markers.push((at, label.to_string()));
        Ok(())
    }

    fn stop(&mut self, handle: RecordHandle) -> Result<RecordReport, IngestError> {
        let a = self.active_mut(handle)?;
        let at = a.extent.view().available;
        let e1 = a.session.apply(Trigger::Stop, at, "")?;
        a.extent.finalize()?;
        let e2 = a.session.apply(Trigger::Stopped, at, "")?;
        let report = RecordReport {
            files: vec![a.job.target.path.to_string_lossy().into_owned()],
            frames: a.frames,
            dropped: a.dropped,
            preserved: {
                let mut p = Vec::new();
                if a.job.start_timecode.is_some() {
                    p.push(MetadataKind::Timecode);
                }
                if !a.markers.is_empty() {
                    p.push(MetadataKind::Markers);
                }
                p
            },
            lost: Vec::new(),
        };
        let id = a.id;
        self.events.push(e1);
        self.events.push(e2);
        if let Some(a) = self.active.take_if(|a| a.id == id) {
            self.done.insert(id, a);
        }
        Ok(report)
    }

    fn extent(&self, handle: RecordHandle) -> Result<ExtentView, IngestError> {
        self.get(handle.0).map(|a| a.extent.view()).ok_or(IngestError::UnknownHandle(handle.0))
    }
}
