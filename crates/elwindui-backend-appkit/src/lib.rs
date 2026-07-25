//! AppKit implementation of the widget surface `elwindui-codegen` targets for the `notepad`
//! example. See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! Split into `inner` (private — raw AppKit plumbing, `Inner`-prefixed types) and `native_ui`
//! (public, re-exported here — implements every `elwindui_core::ui` builtin trait this backend
//! provides by composing the matching `inner` type). See each module's own doc comment.

#![cfg(target_os = "macos")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

/// Performs process-wide AppKit setup required before creating views.
///
/// AppKit performs this lazily when the application object is created, so this is intentionally
/// idempotent and currently has no eager work. It exists to keep the facade's `elwindui::init()`
/// contract uniform across native backends.
pub fn init() -> Result<(), std::convert::Infallible> {
    Ok(())
}

mod app;
mod ffi;
mod host;
mod inner;
mod native_ui;
pub mod platform;
mod render;

#[cfg(test)]
mod testsupport;

pub use native_ui::*;

// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly (see
// `inner::AnyView`'s own doc comment), so it needs to stay reachable at this crate's own root even
// though the rest of `inner` is private.
pub use ffi::AnyView;


/// Re-exported so `elwindui`'s own facade can expose `application::run` uniformly across
/// backends. See `app`'s module doc.
pub mod application {
    pub use crate::app::run;
}
