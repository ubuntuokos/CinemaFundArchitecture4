//! Stand-in for the `tracing` macros of the ported code: CFA4 does not log from inside the
//! decoder, so events and spans compile to nothing.

/// A span that does nothing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Span;

impl Span {
    pub(crate) fn entered(self) -> Self {
        self
    }
    pub(crate) fn enter(&self) -> Self {
        *self
    }
    pub(crate) fn in_scope<R>(&self, f: impl FnOnce() -> R) -> R {
        f()
    }
}

macro_rules! event {
    ($($t:tt)*) => {
        ()
    };
}
macro_rules! span {
    ($($t:tt)*) => {
        $crate::tracing::Span
    };
}
pub(crate) use event as debug;
pub(crate) use event as error;
pub(crate) use event as trace;
pub(crate) use event as warn;
pub(crate) use span;
pub(crate) use span as trace_span;
