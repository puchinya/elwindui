//! Issue #128 acceptance criterion (§9.9): a genuine 3-crate fixture proving `#[overridable]`/
//! `#[overrides]` propagation through a `struct_only` bridge crosses real crate boundaries, not
//! just module boundaries within one crate (unlike `elwindui-core::ui::testsupport`'s own
//! same-crate fixtures, which exercise the mechanism itself but not this specific concern).
//!
//! Three separate compilation units:
//! - `elwindui-bridge-fixture-iface`: the `trait_only` interface.
//! - `elwindui-bridge-fixture-backend`: a `struct_only` implementor of it, a *different* crate.
//! - this file (part of `crates/elwindui`'s own test binary, a *third* crate again): an ordinary
//!   `#[overrides]` descendant of the `struct_only` implementor.
//!
//! This is the exact `trait_only` (crate A) -> `struct_only` (crate B) -> ordinary (crate C) shape
//! that originally exposed the bug in `Window` (`elwindui-core`'s `trait_only Window` -> each
//! backend's `struct_only Window` -> a generated host-composition component), reproduced as a
//! minimal, backend-independent fixture that needs no native window construction (and so is not
//! subject to the main-thread constraints `tests/window_mount_hide_close.rs` documents).

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` with a same-crate `inherits` chain needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui_bridge_fixture_backend::BridgeFixtureConcrete;
use elwindui_bridge_fixture_iface::{
    BridgeFixtureGenericOrdinaryBaseExt, BridgeFixtureInterfaceExt, BridgeFixtureRootExt,
};

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
fn cross_crate_struct_only_bridge_override_dispatches_and_reaches_backend() {
    let derived = BridgeFixtureDerived::new();
    // `101`: `BridgeFixtureDerived::value` (crate C) overrides and adds 100 to `self.base.value()`,
    // which reaches `BridgeFixtureConcrete::value` (crate B, the `struct_only` implementor of crate
    // A's `trait_only` interface) returning `1` — proving `base::` (here, raw `self.base.value()`)
    // crosses back into the backend implementation across the real crate boundary. Dispatched through
    // `BridgeFixtureInterfaceExt` directly (crate A's own real interface trait, no backend-side
    // `{ConcreteName}Ext` compatibility alias — Issue #128 remediation review finding A2).
    assert_eq!(BridgeFixtureInterfaceExt::value(&*derived), 101);
}

/// PR #164 final remediation round, T7 (finding A5): a *generic* `struct_only` concrete type
/// (`BridgeFixtureGenericConcrete<T>`, crate B), implementing a *non-generic* interface (crate A),
/// inherited here (crate C) with a consumer-local concrete generic argument
/// (`ConsumerSource`) — the real interface identity must stay `BridgeFixtureGenericInterfaceExt`
/// (never `BridgeFixtureGenericInterfaceExt<ConsumerSource>`), with no same-crate registry available
/// to prevent that across this real crate boundary.
struct ConsumerSource(i32);

impl elwindui_bridge_fixture_backend::BridgeFixtureGenericSource for ConsumerSource {
    fn value(&self) -> i32 {
        self.0
    }
}

#[elwindui::class(
    inherits = elwindui_bridge_fixture_backend::BridgeFixtureGenericConcrete<ConsumerSource>
)]
struct BridgeFixtureGenericDerived {}

#[elwindui::class]
impl BridgeFixtureGenericDerived {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 100
    }
    fn construct() -> Self {
        Self {
            base: elwindui_bridge_fixture_backend::BridgeFixtureGenericConcrete::construct(
                ConsumerSource(1),
            ),
        }
    }
}

#[test]
fn cross_crate_generic_struct_only_never_leaks_concrete_generics_onto_the_real_interface() {
    let derived = BridgeFixtureGenericDerived::new();
    assert_eq!(
        elwindui_bridge_fixture_iface::BridgeFixtureGenericInterfaceExt::value(&*derived),
        101
    );
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
    assert_eq!(
        elwindui_bridge_fixture_iface::BridgeFixtureGenericOrdinaryBaseExt::value(&*derived),
        101
    );
}

/// PR #164 final remediation round, T10 (finding C2): a root-mode interface (crate A), a matching
/// `struct_only = ..Ext, inherits = ..` implementor of it (crate B), and an ordinary descendant here
/// (crate C) — proves C2's root bridge (`as_ui_element` forwarding to the composed root storage, no
/// duplicate `impl`) works across real crate boundaries, not just same-crate (`BridgeRootDerived`,
/// `elwindui-core::ui::testsupport`).
#[elwindui::class(inherits = elwindui_bridge_fixture_backend::BridgeFixtureRootConcrete)]
struct BridgeFixtureRootDerived {}

#[elwindui::class]
impl BridgeFixtureRootDerived {
    #[overrides]
    fn value(&self) -> i32 {
        self.base.value() + 100
    }
    fn construct() -> Self {
        Self {
            base: elwindui_bridge_fixture_backend::BridgeFixtureRootConcrete::construct(),
        }
    }
}

#[test]
fn cross_crate_root_struct_only_bridge_dispatches_and_reaches_composed_root_storage() {
    let derived = BridgeFixtureRootDerived::new();
    assert_eq!(
        elwindui_bridge_fixture_iface::BridgeFixtureRootExt::value(&*derived),
        101
    );
    let root_ref: &elwindui_bridge_fixture_iface::BridgeFixtureRoot =
        elwindui_bridge_fixture_iface::BridgeFixtureRootExt::as_ui_element(&*derived);
    assert_eq!(root_ref.value(), 1);
}
