//! Issue #166 (A2): a required base `#[param]` / `#[bindable]` (no default) reaches a derived
//! Component that has its own view.
//!
//! - Inputs the derived view leaves unset are auto-forwarded: they become the derived
//!   constructor's leading parameters (base order first, then the derived Component's own).
//! - A value the derived view sets itself (`title: "fixed"`) is used instead, and no parameter is
//!   added for it.
//! - An authored same-name forward (`title: title`) is the same forward, never a duplicate.
//! - This holds for a same-crate base and for a generated base in another crate. The other-crate
//!   case covers a view-less derived level, a two-level chain defined in the other crate, and
//!   same-crate code that inherits or constructs such a derived Component.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::rc::Rc;

use elwindui::core::ui::UIElementExt;

fn texts(root: &dyn UIElementExt) -> Vec<String> {
    root.visual_children()
        .iter()
        .filter_map(|child| {
            child
                .as_any()
                .downcast_ref::<elwindui::ui::TextBlock>()
                .map(|text| text.text.borrow().clone())
        })
        .collect()
}

#[elwindui::viewmodel]
mod required_labels {
    struct RequiredLabels {
        #[observable(default = "vm-a".to_string())]
        label: String,
    }
}

// ---------------------------------------------------------------------------
// Same-crate base.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits VerticalLayout)]
struct RequiredBase {
    #[param]
    title: String,
    #[bindable]
    vm: Rc<RequiredLabels>,

    body: view! {
        TextBlock { text: title }
        TextBlock { text: vm.label }
    },
}

#[elwindui::component]
impl RequiredBase {}

/// Leaves both base inputs unset and reads `title` in its own view.
#[elwindui::component(inherits crate::RequiredBase)]
struct AutoForwardDerived {
    #[param]
    extra: i32,

    body: view! {
        TextBlock { text: format!("derived {title} {extra}") }
    },
}

#[elwindui::component]
impl AutoForwardDerived {}

/// Writes the same-name forward explicitly.
#[elwindui::component(inherits crate::RequiredBase)]
struct ExplicitForwardDerived {
    body: view! {
        title: title
        TextBlock { text: "explicit" }
    },
}

#[elwindui::component]
impl ExplicitForwardDerived {}

/// Sets `title` itself, so only `vm` is forwarded.
#[elwindui::component(inherits crate::RequiredBase)]
struct FixedTitleDerived {
    body: view! {
        title: "fixed"
        TextBlock { text: "fixed" }
    },
}

#[elwindui::component]
impl FixedTitleDerived {}

/// Two levels with their own views.
#[elwindui::component(inherits crate::AutoForwardDerived)]
struct AutoForwardLeaf {
    body: view! {
        TextBlock { text: "leaf" }
    },
}

#[elwindui::component]
impl AutoForwardLeaf {}

#[test]
fn unset_required_base_inputs_become_leading_constructor_parameters() {
    let vm = RequiredLabels::new();
    let derived = AutoForwardDerived::new("t".to_string(), Rc::clone(&vm), 7);
    assert_eq!(texts(&*derived), ["t", "vm-a", "derived t 7"]);
    assert_eq!(derived.extra(), 7);

    // The base keeps its own `#[bindable]` subscription for the forwarded viewmodel.
    vm.set_label("vm-b".to_string());
    assert_eq!(texts(&*derived), ["t", "vm-b", "derived t 7"]);
}

#[test]
fn explicit_same_name_forward_is_the_same_single_parameter() {
    let vm = RequiredLabels::new();
    let derived = ExplicitForwardDerived::new("e".to_string(), vm);
    assert_eq!(texts(&*derived), ["e", "vm-a", "explicit"]);
}

#[test]
fn a_value_set_by_the_derived_view_adds_no_parameter() {
    let vm = RequiredLabels::new();
    let derived = FixedTitleDerived::new(vm);
    assert_eq!(texts(&*derived), ["fixed", "vm-a", "fixed"]);
}

#[test]
fn required_base_inputs_forward_through_two_levels_with_views() {
    let vm = RequiredLabels::new();
    let leaf = AutoForwardLeaf::new("l".to_string(), Rc::clone(&vm), 3);
    assert_eq!(texts(&*leaf), ["l", "vm-a", "derived l 3", "leaf"]);
    vm.set_label("vm-c".to_string());
    assert_eq!(texts(&*leaf), ["l", "vm-c", "derived l 3", "leaf"]);
}

// ---------------------------------------------------------------------------
// Generated base in another crate. No `use` items are needed for the base's traits.
// ---------------------------------------------------------------------------

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalRequiredBase)]
struct CrossCrateDerived {
    body: view! {
        TextBlock { text: format!("cross {title} {count}") }
    },
}

#[elwindui::component]
impl CrossCrateDerived {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalRequiredBase)]
struct CrossCrateFixedTitle {
    body: view! {
        title: "fixed"
        TextBlock { text: "cross fixed" }
    },
}

#[elwindui::component]
impl CrossCrateFixedTitle {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalRequiredBase)]
struct CrossCrateViewless {}

#[elwindui::component]
impl CrossCrateViewless {}

/// The defining crate's own two-level chain (`ExternalRequiredMiddle` auto-forwards
/// `ExternalRequiredBase`'s inputs and adds `suffix`).
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalRequiredMiddle)]
struct CrossCrateOverMiddle {
    body: view! {
        TextBlock { text: format!("over {suffix}") }
    },
}

#[elwindui::component]
impl CrossCrateOverMiddle {}

/// Same-crate inheritance of a Component whose constructor comes from another crate's base.
#[elwindui::component(inherits crate::CrossCrateDerived)]
struct CrossCrateLeaf {
    body: view! {
        TextBlock { text: "cross leaf" }
    },
}

#[elwindui::component]
impl CrossCrateLeaf {}

/// Same-crate construction of that Component from a view.
#[elwindui::component(inherits VerticalLayout)]
struct CrossCrateUser {
    body: view! {
        CrossCrateDerived { title: "used", count: 9 }
    },
}

#[elwindui::component]
impl CrossCrateUser {}

#[test]
fn cross_crate_required_base_inputs_are_auto_forwarded() {
    let derived = CrossCrateDerived::new("x".to_string(), 3);
    assert_eq!(texts(&*derived), ["x:3", "cross x 3"]);
}

#[test]
fn cross_crate_value_set_by_the_derived_view_adds_no_parameter() {
    let derived = CrossCrateFixedTitle::new(5);
    assert_eq!(texts(&*derived), ["fixed:5", "cross fixed"]);
}

#[test]
fn cross_crate_view_less_derived_forwards_every_required_input() {
    let derived = CrossCrateViewless::new("y".to_string(), 4);
    assert_eq!(texts(&*derived), ["y:4"]);
}

#[test]
fn cross_crate_chain_defined_in_the_other_crate_is_forwarded_in_order() {
    let derived = CrossCrateOverMiddle::new("m".to_string(), 2, "!".to_string());
    assert_eq!(texts(&*derived), ["m:2", "m!", "over !"]);
}

#[test]
fn same_crate_code_inherits_and_constructs_a_cross_crate_derived_component() {
    let leaf = CrossCrateLeaf::new("cl".to_string(), 2);
    assert_eq!(texts(&*leaf), ["cl:2", "cross cl 2", "cross leaf"]);

    let user = CrossCrateUser::new();
    let children = user.visual_children();
    assert_eq!(children.len(), 1);
    let inner = children[0]
        .as_any()
        .downcast_ref::<CrossCrateDerived>()
        .expect("the user view constructs the cross-crate derived Component");
    assert_eq!(texts(inner), ["used:9", "cross used 9"]);
}
