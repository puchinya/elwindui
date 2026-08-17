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

use elwindui_bridge_fixture_backend::{BridgeFixtureConcrete, BridgeFixtureConcreteExt};

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
    // crosses back into the backend implementation across the real crate boundary.
    assert_eq!(BridgeFixtureConcreteExt::value(&*derived), 101);
}
