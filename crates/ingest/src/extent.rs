//! How far a growing recording may be read safely (TV-01).
//!
//! The recorder owns an [`Extent`] and advances it only to what is durably written. The editor gets
//! the read-only [`ExtentView`]. A seek at or past the end is [`SeekCheck::NotYetWritten`]: the
//! caller shows "not yet recorded", never a wrong picture, and never blocks.

use cfa4_time::Tick;
use serde::{Deserialize, Serialize};

use crate::error::IngestError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtentState {
    Growing,
    Finalized,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtentView {
    /// Readable duration from the start of the recording (end exclusive).
    pub available: Tick,
    pub frames: u64,
    pub state: ExtentState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekCheck {
    Available,
    NotYetWritten {
        available: Tick,
    },
    /// A negative position.
    BeforeStart,
}

impl ExtentView {
    pub fn check_seek(&self, t: Tick) -> SeekCheck {
        if t < Tick::ZERO {
            SeekCheck::BeforeStart
        } else if t < self.available {
            SeekCheck::Available
        } else {
            SeekCheck::NotYetWritten { available: self.available }
        }
    }
}

/// The recorder's side. Monotonic: it only grows, and only while `Growing`.
#[derive(Clone, Copy, Debug)]
pub struct Extent {
    view: ExtentView,
}

impl Default for Extent {
    fn default() -> Self {
        Extent { view: ExtentView { available: Tick::ZERO, frames: 0, state: ExtentState::Growing } }
    }
}

impl Extent {
    pub fn view(&self) -> ExtentView {
        self.view
    }

    /// Declare `frames` more frames readable, ending at `available` (durably written).
    pub fn advance(&mut self, frames: u64, available: Tick) -> Result<(), IngestError> {
        if self.view.state != ExtentState::Growing {
            return Err(IngestError::ExtentClosed("not growing"));
        }
        if available < self.view.available {
            return Err(IngestError::ExtentNotMonotonic(format!("{} ticks < {} ticks", available.0, self.view.available.0)));
        }
        self.view.frames = self.view.frames.checked_add(frames).ok_or_else(|| IngestError::ExtentNotMonotonic("frame count overflow".into()))?;
        self.view.available = available;
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), IngestError> {
        self.close(ExtentState::Finalized)
    }

    pub fn interrupt(&mut self) -> Result<(), IngestError> {
        self.close(ExtentState::Interrupted)
    }

    fn close(&mut self, to: ExtentState) -> Result<(), IngestError> {
        if self.view.state != ExtentState::Growing {
            return Err(IngestError::ExtentClosed("already closed"));
        }
        self.view.state = to;
        Ok(())
    }
}
