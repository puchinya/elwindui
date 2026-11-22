//! Issue #128 acceptance criterion (§9.9): a genuine 3-crate fixture proving `#[overridable]`/
//! `#[overrides]` propagation along an ordinary class chain crosses real crate boundaries, not
//! just module boundaries within one crate (unlike `elwindui-core::ui::testsupport`'s own
//! same-crate fixtures, which exercise the mechanism itself but not this specific concern).
//!
//! Three separate compilation units:
//! - `elwindui-bridge-fixture-iface`: the base classes (crate A).
//! - `elwindui-bridge-fixture-backend`: descendants of them, in a *different* crate (crate B).
//! - this file (part of `crates/elwindui`'s own test binary, a *third* crate again): further
//!   `#[overrides]` descendants (crate C).
//!
//! This is the shape of a native control's chain (`elwindui-core`'s `NativeControl` → Core's
//! `Button` → a generated component in a user crate), reproduced as a minimal, backend-independent
//! fixture that needs no native window construction.

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` with a same-crate `inherits` chain needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui_bridge_fixture_backend::BridgeFixtureConcrete;
use elwindui_bridge_fixture_iface::{BridgeFixtureGenericOrdinaryBaseExt, BridgeFixtureRootExt};

#[elwindui::class(inherits = elwindui_bridge_fixture_backend::BridgeFixtureConcrete)]
struct BridgeFixtureDerived {}

#[elwindui::class]
impl BridgeFixtureDerived {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 100
    }
    fn construct() -> Self {
        Self {
            base: BridgeFixtureConcrete::construct(),
        }
    }
}

#[test]
fn cross_crate_override_dispatches_and_reaches_the_root_storage() {
    let derived = BridgeFixtureDerived::new();
    // `101`: `BridgeFixtureDerived::value` (crate C) overrides and adds 100 to `self.base.value()`,
    // which passes through `BridgeFixtureConcrete` (crate B, no override) to crate A's own
    // `BridgeFixtureRoot::value` returning `1`. Dispatched through crate A's own `BridgeFixtureRootExt`.
    assert_eq!(BridgeFixtureRootExt::value(&*derived), 101);
    // `as_ui_element` reaches crate A's root storage from crate C, through crate B.
    let root_ref: &elwindui_bridge_fixture_iface::BridgeFixtureRoot =
        BridgeFixtureRootExt::as_ui_element(&*derived);
    assert_eq!(root_ref.value(), 1);
}

/// PR #164 final remediation round, T8 (finding A5): a *generic* ordinary (root-mode) ancestor,
/// declared in crate A, inherited directly here (crate C) with a consumer-chosen concrete generic
/// argument (`i32`) — its own generated `BridgeFixtureGenericOrdinaryBaseExt<i32>` *does* need that
/// argument reattached, cross-crate, proving the opposite direction from T7 (no global drop of
/// generic arguments either).
#[elwindui::class(
    inherits = elwindui_bridge_fixture_iface::BridgeFixtureGenericOrdinaryBase<i32>
)]
struct GenericOrdinaryDerived {}

#[elwindui::class]
impl GenericOrdinaryDerived {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 100
    }
    fn construct() -> Self {
        Self {
            base: elwindui_bridge_fixture_iface::BridgeFixtureGenericOrdinaryBase::construct(1),
        }
    }
}

#[test]
fn cross_crate_generic_ordinary_ancestor_keeps_its_own_generic_argument() {
    let derived = GenericOrdinaryDerived::new();
    assert_eq!(BridgeFixtureGenericOrdinaryBaseExt::value(&*derived), 101);
}

/// PR #164 final remediation round, T20 (finding A6): a *second* ordinary hop below
/// `GenericOrdinaryDerived` — proves the opposite direction from T19 recursively too: a generic
/// ordinary/root ancestor's own generic argument must still be *kept* through recursive metadata.
#[elwindui::class(inherits = crate::GenericOrdinaryDerived)]
struct GenericOrdinaryDeepDerived {}

#[elwindui::class]
impl GenericOrdinaryDeepDerived {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 1000
    }
    fn construct() -> Self {
        Self {
            base: GenericOrdinaryDerived::construct(),
        }
    }
}

#[test]
fn cross_crate_generic_ordinary_ancestor_deep_descendant_keeps_its_own_generic_argument() {
    let deep = GenericOrdinaryDeepDerived::new();
    // 1101 = 1 (BridgeFixtureGenericOrdinaryBase) + 100 (GenericOrdinaryDerived) + 1000 (deep).
    assert_eq!(BridgeFixtureGenericOrdinaryBaseExt::value(&*deep), 1101);
}
