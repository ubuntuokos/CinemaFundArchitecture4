//! Opened Smart Cut sources kept for one batch of ranges (CFA4-LOSSLESS-MEDIA-TOOLS-001 K1): a
//! batch plans every range and then cuts it, and without this every step would read and index the
//! same source again. Outside a batch nothing is kept, so a single command always reads the file
//! as it is now.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use crate::Result;

type Entry = (String, Rc<dyn Any>);

thread_local! {
    static BATCH: RefCell<Option<Vec<Entry>>> = const { RefCell::new(None) };
}

/// Keeps the sources opened until it is dropped (the end of the batch, also on an error).
pub(crate) struct Batch {
    outer: bool,
}

/// Start keeping opened sources; a batch inside a batch shares the outer one.
pub(crate) fn batch() -> Batch {
    BATCH.with(|b| {
        let mut b = b.borrow_mut();
        let outer = b.is_none();
        if outer {
            *b = Some(Vec::new());
        }
        Batch { outer }
    })
}

impl Drop for Batch {
    fn drop(&mut self) {
        if self.outer {
            BATCH.with(|b| *b.borrow_mut() = None);
        }
    }
}

/// The source opened for `path` in this batch, or `open()`'s (kept when a batch runs).
pub(crate) fn opened<T: 'static>(path: &str, open: impl FnOnce() -> Result<T>) -> Result<Rc<T>> {
    let hit = BATCH.with(|b| b.borrow().as_ref().and_then(|v| v.iter().find(|(p, _)| p == path).and_then(|(_, x)| Rc::clone(x).downcast::<T>().ok())));
    if let Some(x) = hit {
        return Ok(x);
    }
    let x = Rc::new(open()?);
    BATCH.with(|b| {
        if let Some(v) = b.borrow_mut().as_mut() {
            v.push((path.to_string(), Rc::clone(&x) as Rc<dyn Any>));
        }
    });
    Ok(x)
}
