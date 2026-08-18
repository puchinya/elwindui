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

/// PR #164 final remediation round, T7 (finding A5): a *non-generic* interface — deliberately kept
/// this way (the point of T7 is proving a `struct_only` implementor's *own* generic parameters never
/// leak onto this interface, cross-crate, with no same-crate registry available to prevent it).
#[elwindui_macros::class(trait_only)]
pub trait BridgeFixtureGenericInterface {
    #[overridable]
    fn value(&self) -> i32;
}

/// PR #164 final remediation round, T10 (finding C2): a root-mode interface — `elwindui-bridge-fixture-backend`
/// declares a matching `struct_only = ..Ext, inherits = ..` implementor of it in a *different* crate,
/// proving C2's root bridge works across a real crate boundary, not just same-crate.
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

/// T8's own base: a *generic root-mode* class (no `inherits`/`struct_only` of its own). This
/// surfaced two genuine, pre-existing bugs in `#[class]`'s own root-mode codegen, unrelated to A5/C2
/// but blocking T8's own fixture from compiling, so fixed as independent valid work alongside this
/// remediation round: (1) the root-mode `as_ui_element` accessor's declared/impl'd return type was
/// missing `#ty_generics` (`&Foo` instead of `&Foo<T>`), confirmed pre-existing via `git stash`
/// against the base commit in an isolated reproduction crate; (2) `build_dyn_default_methods`'s
/// shared default-method body called through a bare `#ext_ty::#name(..)` path expression, which
/// parses as a chained comparison ("comparison operators cannot be chained") once `#ext_ty` carries
/// its own generic arguments (`FooExt<T>`) — fixed via `turbofish_ext_path` (`FooExt::<T>`), not the
/// `<dyn FooExt<T>>::` form tried first and reverted (that form reintroduces the exact E0034
/// ambiguity `build_dyn_default_methods`'s own doc comment already documents avoiding).
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
