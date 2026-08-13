//! CI-7 of #80 (closes #100): `EnvironmentScope { key: value, ..; <children> }` — a codegen-time
//! construction/mount context boundary, not a real element. Overrides declared in one scope must
//! reach only the children lexically inside it, leaving a sibling outside the scope (in the same
//! parent) observing the un-overridden `application_environment()` value.
//!
//! CI-6's dedicated-key-per-test isolation convention applies here too (see `environment_field.rs`'s
//! own module doc comment) — this file's tests share one key deliberately (both read the *same*
//! override behavior from two different vantage points in a single component tree), but no other
//! test file in the workspace touches `EnvironmentScopeLocale`.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::RefCell;

thread_local! {
    static INSIDE_LOCALE: RefCell<String> = RefCell::new(String::new());
    static OUTSIDE_LOCALE: RefCell<String> = RefCell::new(String::new());
}

#[elwindui::environment_key(
    name = environment_scope_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentScopeLocale;

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeInsideChild {
    #[environment(environment_scope_locale)]
    locale: String,

    body: view! {
        on_mount {
            INSIDE_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentScopeInsideChild {}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeOutsideChild {
    #[environment(environment_scope_locale)]
    locale: String,

    body: view! {
        on_mount {
            OUTSIDE_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentScopeOutsideChild {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeParent {
    body: view! {
        EnvironmentScope {
            environment_scope_locale: "ja-JP",
            EnvironmentScopeInsideChild {}
        }
        EnvironmentScopeOutsideChild {}
    },
}

#[elwindui::component]
impl EnvironmentScopeParent {}

#[test]
fn override_reaches_only_children_inside_the_scope() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentScopeLocale>("en-US".to_string());
    INSIDE_LOCALE.with(|c| *c.borrow_mut() = String::new());
    OUTSIDE_LOCALE.with(|c| *c.borrow_mut() = String::new());

    let _parent = EnvironmentScopeParent::new();

    assert_eq!(
        INSIDE_LOCALE.with(|c| c.borrow().clone()),
        "ja-JP",
        "the child declared inside EnvironmentScope must observe its override"
    );
    assert_eq!(
        OUTSIDE_LOCALE.with(|c| c.borrow().clone()),
        "en-US",
        "a sibling outside the scope, in the same parent, must observe the un-overridden value"
    );
}

// CI-7 of #80: `EnvironmentScope` itself must produce no UIElement/Visual/Render/Layout node.
// `emit_environment_scope_construction` (crates/elwindui-codegen/src/codegen.rs) structurally
// cannot emit one — it only ever emits `let #binding = <expr>.derive(); #binding.set(..); ...;`,
// never a `Type::new()`/struct field/wiring/resync statement — but this test additionally proves
// the *count* of real children built is exactly right (no accidental extra or missing construction
// around the scope boundary): three `TextBlock`s (two inside the scope, one outside) each report
// their own construction via `on_mount`, and no fourth one ever fires.
thread_local! {
    static NO_EXTRA_NODE_MOUNT_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeCountingLeaf {
    body: view! {
        on_mount {
            NO_EXTRA_NODE_MOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "leaf" }
    },
}

#[elwindui::component]
impl EnvironmentScopeCountingLeaf {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeNoExtraNodeView {
    body: view! {
        EnvironmentScope {
            environment_scope_locale: "en-US",
            EnvironmentScopeCountingLeaf {}
            EnvironmentScopeCountingLeaf {}
        }
        EnvironmentScopeCountingLeaf {}
    },
}

#[elwindui::component]
impl EnvironmentScopeNoExtraNodeView {}

#[test]
fn environment_scope_produces_no_extra_visual_node() {
    NO_EXTRA_NODE_MOUNT_COUNT.with(|c| c.set(0));
    let _view = EnvironmentScopeNoExtraNodeView::new();
    assert_eq!(NO_EXTRA_NODE_MOUNT_COUNT.with(|c| c.get()), 3);
}

// CI-7 of #80: a nested `EnvironmentScope` must derive from its own *enclosing scope's* already-
// derived `EnvironmentContext` local variable, not directly from `self.__mount_environment` — the
// inner scope's override must be visible to its own children, and the inner scope's un-overridden
// keys must still see the outer scope's override (proving the derive chain, not just a flat
// re-derive from the component root).
thread_local! {
    static NESTED_INNER_TINT: RefCell<String> = RefCell::new(String::new());
    static NESTED_INNER_LOCALE: RefCell<String> = RefCell::new(String::new());
}

#[elwindui::environment_key(
    name = environment_scope_nested_tint,
    value = String,
    default = String::from("default-tint")
)]
pub struct EnvironmentScopeNestedTint;

#[elwindui::environment_key(
    name = environment_scope_nested_locale,
    value = String,
    default = String::from("en-US")
)]
pub struct EnvironmentScopeNestedLocale;

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeNestedInnerChild {
    #[environment(environment_scope_nested_tint)]
    tint: String,
    #[environment(environment_scope_nested_locale)]
    locale: String,

    body: view! {
        on_mount {
            NESTED_INNER_TINT.with(|c| *c.borrow_mut() = self.tint());
            NESTED_INNER_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    },
}

#[elwindui::component]
impl EnvironmentScopeNestedInnerChild {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeNestedParent {
    body: view! {
        EnvironmentScope {
            environment_scope_nested_tint: "outer-tint",
            EnvironmentScope {
                environment_scope_nested_locale: "ja-JP",
                EnvironmentScopeNestedInnerChild {}
            }
        }
    },
}

#[elwindui::component]
impl EnvironmentScopeNestedParent {}

#[test]
fn nested_environment_scope_derives_from_its_own_enclosing_scope() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentScopeNestedTint>("root-tint".to_string());
    NESTED_INNER_TINT.with(|c| *c.borrow_mut() = String::new());
    NESTED_INNER_LOCALE.with(|c| *c.borrow_mut() = String::new());

    let _parent = EnvironmentScopeNestedParent::new();

    // The inner scope only overrides `locale` — `tint` must still see the *outer* scope's
    // override ("outer-tint"), not the component root's ("root-tint"), proving the inner scope
    // derived from the outer scope's own context rather than re-deriving from
    // `self.__mount_environment` directly.
    assert_eq!(NESTED_INNER_TINT.with(|c| c.borrow().clone()), "outer-tint");
    assert_eq!(NESTED_INNER_LOCALE.with(|c| c.borrow().clone()), "ja-JP");
}
