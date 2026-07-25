//! Test-only support and the two golden-image suites.
//!
//! These live in their own files rather than inline at the bottom of the module they exercise:
//! together they are ~1500 lines, which used to be 30% of `inner.rs`. They stay *inside* the
//! crate (rather than moving to `tests/`) so they can keep reaching crate-private items.

mod bitmap;
mod golden;
mod svg_golden;
