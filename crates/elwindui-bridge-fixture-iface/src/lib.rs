//! Cross-crate test fixture for Issue #128: `#[overridable]`/`#[overrides]` propagation along an
//! ordinary class chain that crosses real crate boundaries.
//!
//! Declares the base classes (crate A). `elwindui-bridge-fixture-backend` declares descendants of
//! them in a *different* crate (crate B), and `crates/elwindui`'s own integration tests declare
//! further `#[overrides]` descendants in a *third* crate (crate C) — together proving the
//! inherit/override machinery reaches across real crate boundaries, rather than only the same-crate
//! fixtures in `elwindui-core::ui::testsupport`. This is the shape of a native control's chain
//! (`elwindui-core`'s `NativeControl` → Core's `Button` → a generated component in a user crate).
//! Not published; workspace-internal only — see `elwindui-environment-key-fixture` for the same
//! dev-dependency pattern applied to Issue #129.

// See `crates/elwindui-macros/src/class.rs`'s own doc comment on `inherit_macro_self_ref_path` —
// every crate using `#[class]` needs this.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

/// A root-mode base class. `elwindui-bridge-fixture-backend::BridgeFixtureConcrete` inherits it from
/// another crate, and `as_ui_element` must still reach this storage from a third one.
#[elwindui_macros::class]
pub struct BridgeFixtureRoot {
    value: i32,
}

#[elwindui_macros::class]
impl BridgeFixtureRoot {
    #[overridable]
    fn value(&self) -> i32 {
        self.value
    }
    fn construct() -> Self {
        Self { value: 1 }
    }
}

/// T8's own base: a *generic root-mode* class (no `inherits` of its own). This surfaced two
/// genuine, pre-existing bugs in `#[class]`'s own root-mode codegen: (1) the root-mode
/// `as_ui_element` accessor's declared/impl'd return type was missing `#ty_generics` (`&Foo`
/// instead of `&Foo<T>`); (2) `build_dyn_default_methods`'s shared default-method body called
/// through a bare `#ext_ty::#name(..)` path expression, which parses as a chained comparison
/// ("comparison operators cannot be chained") once `#ext_ty` carries its own generic arguments
/// (`FooExt<T>`) — fixed via `turbofish_ext_path` (`FooExt::<T>`), not the `<dyn FooExt<T>>::` form
/// (that form reintroduces the E0034 ambiguity `build_dyn_default_methods`'s own doc comment
/// documents avoiding).
#[elwindui_macros::class]
pub struct BridgeFixtureGenericOrdinaryBase<T: Clone + 'static> {
    value: T,
}

#[elwindui_macros::class]
impl<T: Clone + 'static> BridgeFixtureGenericOrdinaryBase<T> {
    #[overridable]
    fn value(&self) -> T {
        self.value.clone()
    }
    fn construct(value: T) -> Self {
        Self { value }
    }
}
