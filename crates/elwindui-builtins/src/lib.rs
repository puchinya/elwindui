//! Hand-written Rust implementations of every `builtin::` DSL element (`Window`/`Column`/`Row`/
//! `TextArea`/`Button`/`Text`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabView`), paired with
//! shape-only `.elwind` declarations under `src/shapes/` (embedded into `elwindui-codegen` via
//! `include_str!` so any `.elwind` file can resolve/validate against them without a `use`).
//!
//! `elwindui-codegen`'s `emit_construction`/`emit_wiring`/`emit_resync` no longer know these types
//! by name — every one of them is constructed/wired/resynced through the exact same generic
//! conventions a plain user `component`/`view` pair is. This crate is where the widget-specific
//! logic that used to live inside the compiler now lives instead.
//!
//! `backend-appkit` is the real, verified implementation. `backend-winui3` also has an
//! implementation, written best-effort without a Windows machine to build/test it against — see
//! `elwindui-backend-winui3`'s crate-level doc comment. Every other backend feature is reserved
//! for when its corresponding `elwindui-backend-*` crate actually has one.

#[cfg(feature = "backend-appkit")]
mod appkit;
#[cfg(feature = "backend-appkit")]
pub use appkit::*;

#[cfg(feature = "backend-winui3")]
mod winui3;
#[cfg(feature = "backend-winui3")]
pub use winui3::*;
