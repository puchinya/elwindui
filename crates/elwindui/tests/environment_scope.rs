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

use elwindui::core::ui::UIElementExt as _;

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

    template: template_view!(|templated_parent: Self| {
        on_mount {
            INSIDE_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    }),
}

#[elwindui::component]
impl EnvironmentScopeInsideChild {}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeOutsideChild {
    #[environment(environment_scope_locale)]
    locale: String,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            OUTSIDE_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    }),
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
    template: template_view!(|templated_parent: Self| {
        on_mount {
            NO_EXTRA_NODE_MOUNT_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "leaf" }
    }),
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

    template: template_view!(|templated_parent: Self| {
        on_mount {
            NESTED_INNER_TINT.with(|c| *c.borrow_mut() = self.tint());
            NESTED_INNER_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    }),
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

// CI-7 follow-up: an `if` directly inside an `EnvironmentScope` must be scope-aware — the residual
// gap noted in PR #121 ("Known limitation: only a bare literal element... an if/match/for... falls
// back to the ordinary, non-scoped path"). Both branches here are single childless leaves — the
// exact shape that would otherwise qualify for lazy-once materialization (`lazy_branch_plan`) —
// deliberately, so this test also exercises scoped Environment propagation after the parent
// template is explicitly applied.
thread_local! {
    static IF_IN_SCOPE_LOCALE: RefCell<String> = RefCell::new(String::new());
}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeIfChild {
    #[environment(environment_scope_locale)]
    locale: String,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            IF_IN_SCOPE_LOCALE.with(|c| *c.borrow_mut() = self.locale());
        }
        TextBlock { text: locale }
    }),
}

#[elwindui::component]
impl EnvironmentScopeIfChild {}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeIfParent {
    #[param]
    show_child: bool,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            EnvironmentScope {
                environment_scope_locale: "fr-FR",
                if show_child {
                    EnvironmentScopeIfChild {}
                } else {
                    TextBlock { text: "placeholder" }
                }
            }
        }
    }),
}

#[elwindui::component]
impl EnvironmentScopeIfParent {}

#[test]
fn an_if_directly_inside_environment_scope_is_scope_aware() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentScopeLocale>("en-US".to_string());
    IF_IN_SCOPE_LOCALE.with(|c| *c.borrow_mut() = String::new());

    let parent = elwindui::new!(EnvironmentScopeIfParent(show_child: true));
    assert!(parent.apply_template());

    assert_eq!(
        IF_IN_SCOPE_LOCALE.with(|c| c.borrow().clone()),
        "fr-FR",
        "a literal element inside an `if` branch that is itself inside an EnvironmentScope must \
         observe the scope's override, not the un-overridden application_environment() value"
    );
}

// Issue #127: a `for` renderer outlives `__build_view()`, so it must read the live retained scope
// context when a later collection refresh creates an item.  The nested scope in the item body
// also proves that the item was mounted against the effective outer scope rather than the
// application environment.
thread_local! {
    static SCOPED_FOR_RECORDS: RefCell<Vec<(String, String, String)>> = RefCell::new(Vec::new());
    static TEMPLATE_SCOPED_FOR_RECORDS: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());
}

#[elwindui::environment_key(
    name = environment_scope_for_outer,
    value = String,
    default = String::from("root-outer")
)]
pub struct EnvironmentScopeForOuter;

#[elwindui::environment_key(
    name = environment_scope_for_inner,
    value = String,
    default = String::from("root-inner")
)]
pub struct EnvironmentScopeForInner;

#[elwindui::viewmodel]
mod environment_scope_for_collection_item {
    struct EnvironmentScopeForCollectionItem {
        #[observable(default = String::new())]
        label: String,
    }
}

#[elwindui::viewmodel]
mod environment_scope_for_items_view_model {
    struct EnvironmentScopeForItemsViewModel {
        #[observable(default = Vec::new())]
        items: Vec<EnvironmentScopeForCollectionItem>,
    }
}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeForItemProbe {
    #[param]
    label: String,
    #[environment(environment_scope_for_outer)]
    outer: String,
    #[environment(environment_scope_for_inner)]
    inner: String,

    body: view! {
        on_mount {
            SCOPED_FOR_RECORDS.with(|records| {
                records.borrow_mut().push((
                    self.label(),
                    self.outer(),
                    self.inner(),
                ));
            });
        }
        TextBlock { text: label }
    },
}

#[elwindui::component]
impl EnvironmentScopeForItemProbe {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeForItem {
    #[param]
    label: String,

    body: view! {
        EnvironmentScope {
            environment_scope_for_inner: "inner-scope",
            EnvironmentScopeForItemProbe { label: label }
        }
    },
}

#[elwindui::component]
impl EnvironmentScopeForItem {}

#[elwindui::component(inherits VerticalLayout)]
struct EnvironmentScopeForParent {
    #[bindable]
    vm: std::rc::Rc<EnvironmentScopeForItemsViewModel>,
    #[prop(default = String::from("scope-a"))]
    outer_value: String,

    body: view! {
        EnvironmentScope {
            environment_scope_for_outer: outer_value,
            for item in vm.items {
                EnvironmentScopeForItem { label: item.label }
            }
        }
    },
}

#[elwindui::component]
impl EnvironmentScopeForParent {}

#[test]
fn scoped_for_late_item_uses_live_scope_and_preserves_rc_identity() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentScopeForOuter>("root-outer".to_string());
    elwindui::core::environment::application_environment()
        .set::<EnvironmentScopeForInner>("root-inner".to_string());
    SCOPED_FOR_RECORDS.with(|records| records.borrow_mut().clear());

    let first = EnvironmentScopeForCollectionItem::new();
    first.set_label("first".to_string());
    let second = EnvironmentScopeForCollectionItem::new();
    second.set_label("second".to_string());
    let vm = EnvironmentScopeForItemsViewModel::new();
    let parent = elwindui::new!(EnvironmentScopeForParent(vm: vm.clone()));

    vm.items_push(first.clone());
    assert_eq!(
        SCOPED_FOR_RECORDS.with(|records| records.borrow().clone()),
        vec![(
            "first".to_string(),
            "scope-a".to_string(),
            "inner-scope".to_string()
        )]
    );

    parent.set_outer_value("scope-b".to_string());
    SCOPED_FOR_RECORDS.with(|records| records.borrow_mut().clear());
    vm.items_push(second);
    assert_eq!(
        SCOPED_FOR_RECORDS.with(|records| records.borrow().clone()),
        vec![(
            "second".to_string(),
            "scope-b".to_string(),
            "inner-scope".to_string()
        )]
    );
}

#[elwindui::environment_key(
    name = environment_scope_template_for,
    value = String,
    default = String::from("template-root")
)]
pub struct EnvironmentScopeTemplateFor;

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeTemplateForItem {
    #[param]
    label: String,
    #[environment(environment_scope_template_for)]
    value: String,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            TEMPLATE_SCOPED_FOR_RECORDS.with(|records| {
                records
                    .borrow_mut()
                    .push((self.label(), self.value()));
            });
        }
        TextBlock { text: templated_parent.label }
    }),
}

#[elwindui::component]
impl EnvironmentScopeTemplateForItem {}

#[elwindui::component(inherits ContentControl)]
struct EnvironmentScopeTemplateForParent {
    #[prop(default = Vec::new())]
    items: Vec<String>,

    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            EnvironmentScope {
                environment_scope_template_for: "template-scope",
                for item in templated_parent.items {
                    EnvironmentScopeTemplateForItem { label: item }
                }
            }
        }
    }),
}

#[elwindui::component]
impl EnvironmentScopeTemplateForParent {}

#[test]
fn template_scoped_for_late_item_uses_lexical_scope_storage() {
    elwindui::core::environment::application_environment()
        .set::<EnvironmentScopeTemplateFor>("template-root".to_string());
    TEMPLATE_SCOPED_FOR_RECORDS.with(|records| records.borrow_mut().clear());

    let parent = EnvironmentScopeTemplateForParent::new();
    assert!(parent.apply_template());

    parent.set_items(vec!["late-template-item".to_string()]);
    assert_eq!(
        TEMPLATE_SCOPED_FOR_RECORDS.with(|records| records.borrow().clone()),
        vec![(
            "late-template-item".to_string(),
            "template-scope".to_string()
        )]
    );
}
