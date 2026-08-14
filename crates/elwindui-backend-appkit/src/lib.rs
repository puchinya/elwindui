//! AppKit backend — the concrete widget surface `elwindui-codegen` targets on macOS.
//! See docs/design/backends/appkit_backend_design.md.
//!
//! Layering — dependencies run one way only, `native_ui -> inner -> host -> render -> ffi`:
//!
//! | module      | owns |
//! |-------------|------|
//! | `native_ui` | the public façade: one `#[class]` per builtin, implementing the matching
//! |             | `elwindui_core::ui` `*Ext` trait by delegating to its `inner` twin |
//! | `inner`     | raw per-control plumbing, `Inner`-prefixed |
//! | `host`      | the tree host view: layout/render driving, native event -> core input |
//! | `render`    | drawing only — knows nothing about `UIElement`, focus or any control |
//! | `ffi`       | the toolkit seam: the erased native handle (`AnyView`) |
//! | `app`       | dispatcher, app delegate, event-loop entry |
//! | `platform`  | OS services that are not UI elements (file dialogs) |
//!
//! `elwindui-backend-winui3` mirrors this file-for-file; keep the two in step.

#![cfg(target_os = "macos")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/specs/macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

/// Performs process-wide AppKit setup required before creating views.
///
/// AppKit performs this lazily when the application object is created, so this is intentionally
/// idempotent and has one piece of eager work: registering this crate's
/// `elwindui_core::graphics::TextBackend` so `TextBlock::measure_override` (and every
/// `NativeControl::sync_text_style` call) gets real font metrics instead of the core-only
/// deterministic fallback (`DummyTextBackend`). Runs on the main thread — this same guarantee is
/// what every subsequent `measure_text`/`default_text_style` call (always during a main-thread
/// layout pass) relies on to call `NSFont`/`NSFontDescriptor` APIs without its own `mtm()` check.
pub fn init() -> Result<(), std::convert::Infallible> {
    elwindui_core::graphics::set_text_backend(std::rc::Rc::new(render::AppKitTextBackend));
    Ok(())
}

mod app;
#[cfg(feature = "render-stats")]
pub mod diagnostics;
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
