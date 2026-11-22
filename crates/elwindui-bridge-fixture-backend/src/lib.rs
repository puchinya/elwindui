//! Cross-crate test fixture for Issue #128 — the middle crate (crate B).
//!
//! Inherits `elwindui-bridge-fixture-iface`'s root class from a *different* crate than the one
//! that declares it, the way Core's native controls are inherited by generated components in a
//! user crate. See that crate's own doc comment for the full picture. Not published;
//! workspace-internal only.

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

/// An ordinary descendant of crate A's root class that overrides nothing, so a further descendant
/// in crate C reaches crate A's own `value`.
#[elwindui_macros::class(inherits = elwindui_bridge_fixture_iface::BridgeFixtureRoot)]
pub struct BridgeFixtureConcrete {}

#[elwindui_macros::class]
impl BridgeFixtureConcrete {
    fn construct() -> Self {
        Self {
            base: elwindui_bridge_fixture_iface::BridgeFixtureRoot::construct(),
        }
    }
}
