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

/// PR #164 final remediation round, T7 (finding A5): a *generic* `struct_only` concrete type,
/// implementing `elwindui-bridge-fixture-iface`'s *non-generic* `BridgeFixtureGenericInterfaceExt`,
/// from a different crate than both the interface and (later) the consumer's own generic argument.
pub trait BridgeFixtureGenericSource: 'static {
    fn value(&self) -> i32;
}

#[elwindui_macros::class(
    struct_only = elwindui_bridge_fixture_iface::BridgeFixtureGenericInterfaceExt
)]
pub struct BridgeFixtureGenericConcrete<T: BridgeFixtureGenericSource> {
    source: T,
}

#[elwindui_macros::class]
impl<T: BridgeFixtureGenericSource> BridgeFixtureGenericConcrete<T> {
    fn value(&self) -> i32 {
        self.source.value()
    }
    fn construct(source: T) -> Self {
        Self { source }
    }
}

/// PR #164 final remediation round, T10 (finding C2): a `struct_only` implementor of
/// `elwindui-bridge-fixture-iface`'s root-mode `BridgeFixtureRootExt`, composing the exact same root
/// storage via a matching `inherits = ..` — from a different crate than both the root class and
/// (later) the ordinary descendant.
#[elwindui_macros::class(
    struct_only = elwindui_bridge_fixture_iface::BridgeFixtureRootExt,
    inherits = elwindui_bridge_fixture_iface::BridgeFixtureRoot
)]
pub struct BridgeFixtureRootConcrete {}

#[elwindui_macros::class]
impl BridgeFixtureRootConcrete {
    fn value(&self) -> i32 {
        1
    }
    fn construct() -> Self {
        Self {
            base: elwindui_bridge_fixture_iface::BridgeFixtureRoot::construct(),
        }
    }
}
