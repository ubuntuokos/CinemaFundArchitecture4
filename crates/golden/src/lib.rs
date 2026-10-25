//! Golden-image tests for CFA4's renderer (test-only crate; the library is empty).
//!
//! `tests/golden.rs` builds small procedural projects (no media files), renders one frame of each
//! through the CPU reference compositor (`cfa4-render`) and compares it with a committed PNG
//! in `goldens/` using `cfa4_testkit::golden`. The same scenes are composited on the GPU
//! (`cfa4-gpu`) when an adapter is available and compared with the CPU result.
//!
//! Regenerate references with `CFA4_BLESS=1 cargo test -p cfa4-golden`.
