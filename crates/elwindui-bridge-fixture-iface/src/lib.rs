//! Cross-crate test fixture for Issue #128 (`#[overridable]`/`#[overrides]` propagation through a
//! `struct_only` bridge).
//!
//! Declares the `trait_only` interface half only. `elwindui-bridge-fixture-backend` declares a
//! `struct_only` implementor of it in a *different* crate, and `crates/elwindui`'s own integration
//! tests declare an ordinary `#[overrides]` descendant in a *third* crate again — together proving
//! the #128 fix reaches the exact `trait_only` (crate A) -> `struct_only` (crate B) -> ordinary
//! (crate C) shape that originally exposed the bug (`Window`'s own production chain), rather than
//! only the same-crate fixtures in `elwindui-core::ui::testsupport`. Not published; workspace-
//! internal only — see `elwindui-environment-key-fixture` for the same dev-dependency pattern
//! applied to Issue #129.

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui_macros::class(trait_only)]
pub trait BridgeFixtureInterface {
    #[overridable]
    fn value(&self) -> i32;
}
