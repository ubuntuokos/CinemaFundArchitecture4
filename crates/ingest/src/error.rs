use thiserror::Error;

use crate::session::{State, Trigger};

/// Everything the recorder core can refuse. Never a panic.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IngestError {
    #[error("invalid recording job: {0}")]
    InvalidJob(String),
    #[error("invalid transition: {trigger:?} is not allowed in state {from:?}")]
    InvalidTransition { from: State, trigger: Trigger },
    #[error("a reason is required: {0}")]
    ReasonRequired(&'static str),
    #[error("the extent can only grow: {0}")]
    ExtentNotMonotonic(String),
    #[error("the extent is closed ({0})")]
    ExtentClosed(&'static str),
    #[error("unknown recording handle {0}")]
    UnknownHandle(u64),
    #[error("a recording is already active")]
    AlreadyActive,
    #[error("backend error: {0}")]
    Backend(String),
}
