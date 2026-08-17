//! Cross-crate test fixture for Issue #128 — the `struct_only` half.
//!
//! Implements `elwindui-bridge-fixture-iface`'s `trait_only` interface from a *different* crate than
//! the one that declares it, matching `Window`'s own production shape (`elwindui-core`'s `trait_only
//! Window`, each backend crate's own `struct_only Window`). See that crate's own doc comment for the
//! full picture. Not published; workspace-internal only.

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui_macros::class(struct_only = elwindui_bridge_fixture_iface::BridgeFixtureInterfaceExt)]
pub struct BridgeFixtureConcrete {}

#[elwindui_macros::class]
impl BridgeFixtureConcrete {
    fn value(&self) -> i32 {
        1
    }
    fn construct() -> Self {
        Self {}
    }
}
