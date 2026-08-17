//! Cross-crate test fixture for Issue #128 — the `struct_only` half.
//!
//! Implements `elwindui-bridge-fixture-iface`'s `trait_only` interface from a *different* crate than
//! the one that declares it, matching `Window`'s own production shape (`elwindui-core`'s `trait_only
//! Window`, each backend crate's own `struct_only Window`). See that crate's own doc comment for the
//! full picture. Not published; workspace-internal only.

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

// `BridgeFixtureConcreteExt` is a plain re-export alias for the trait this struct implements, not
// a distinct interface — needed because `elwindui_macros::class`'s own same-crate/cross-crate
// ancestor-trait naming-convention fallback (`ancestor_own_trait`, consulted by any *further*
// descendant declared in a crate that hasn't registered `BridgeFixtureConcrete` in its own
// same-crate registry — true for any third crate, by construction) guesses `{bare_name}Ext` from
// the *struct's own* bare name. `Window`/`NativeControl` avoid needing this explicitly because
// their `struct_only` implementor happens to share its bare name with the interface it implements;
// this fixture's names deliberately don't, so the alias makes that naming-convention guess resolve
// correctly for any descendant reached from a third crate.
pub use elwindui_bridge_fixture_iface::BridgeFixtureInterfaceExt as BridgeFixtureConcreteExt;

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
