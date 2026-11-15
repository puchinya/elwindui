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

#![allow(
    macro_expanded_macro_exports_accessed_by_absolute_paths,
    non_snake_case
)]

use std::rc::Rc;

use elwindui::core::theme::BrushStyle;
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

#[elwindui::component(inherits VerticalLayout)]
struct RequiredDefaultsBase {
    #[param]
    title: String,
    #[prop(default = false)]
    flag: bool,
    #[param(default = 1)]
    step: i32,

    body: view! {
        TextBlock { text: format!("{title}:{flag}:{step}") }
    },
}

#[elwindui::component]
impl RequiredDefaultsBase {}

/// Sets the base's ordinary Prop and defaulted Param on its root while `title` is auto-forwarded.
#[elwindui::component(inherits crate::RequiredDefaultsBase)]
struct RootDefaultsDerived {
    #[param]
    extra: i32,

    body: view! {
        flag: true
        step: extra
        TextBlock { text: "defaults" }
    },
}

#[elwindui::component]
impl RootDefaultsDerived {}

#[test]
fn derived_root_sets_the_base_prop_and_defaulted_param() {
    let derived = RootDefaultsDerived::new("d".to_string(), 5);
    assert_eq!(texts(&*derived), ["d:true:5", "defaults"]);
}

/// A middle level whose own view references neither `flag` nor `step`; a leaf's root values for
/// them must still reach `RequiredDefaultsBase`.
#[elwindui::component(inherits crate::RequiredDefaultsBase)]
struct DefaultsMiddle {
    body: view! {
        TextBlock { text: "middle" }
    },
}

#[elwindui::component]
impl DefaultsMiddle {}

#[elwindui::component(inherits crate::DefaultsMiddle)]
struct DefaultsLeaf {
    body: view! {
        flag: true
        step: 5
        TextBlock { text: "leaf" }
    },
}

#[elwindui::component]
impl DefaultsLeaf {}

#[test]
fn leaf_root_sets_a_base_prop_and_defaulted_param_two_hops_up() {
    let leaf = DefaultsLeaf::new("l".to_string());
    assert_eq!(texts(&*leaf), ["l:true:5", "middle", "leaf"]);
}

/// A plain chain over `VerticalLayout` whose leaf root sets builtin properties: `spacing`
/// (declared by `VerticalLayout`) and `margin` (declared by `UIElement`). Neither middle view
/// references them, so both are set on the hand-written builtin through its public setters.
#[elwindui::component(inherits VerticalLayout)]
struct LayoutBase {
    body: view! {
        TextBlock { text: "layout base" }
    },
}

#[elwindui::component]
impl LayoutBase {}

#[elwindui::component(inherits crate::LayoutBase)]
struct LayoutMiddle {
    body: view! {
        TextBlock { text: "layout middle" }
    },
}

#[elwindui::component]
impl LayoutMiddle {}

#[elwindui::component(inherits crate::LayoutMiddle)]
struct SpacedLayoutLeaf {
    body: view! {
        spacing: 8.0
        margin: 18.0
        TextBlock { text: "layout leaf" }
    },
}

#[elwindui::component]
impl SpacedLayoutLeaf {}

#[elwindui::component(inherits crate::LayoutMiddle)]
struct PlainLayoutLeaf {
    body: view! {
        TextBlock { text: "layout leaf" }
    },
}

#[elwindui::component]
impl PlainLayoutLeaf {}

/// The same builtin properties set one hop over the shape base.
#[elwindui::component(inherits crate::LayoutBase)]
struct SpacedLayoutDerived {
    body: view! {
        spacing: 8.0
        margin: 18.0
        TextBlock { text: "layout derived" }
    },
}

#[elwindui::component]
impl SpacedLayoutDerived {}

fn measured_height(root: &dyn UIElementExt) -> f32 {
    root.measure(elwindui::core::base::Size {
        width: 400.0,
        height: f32::INFINITY,
    });
    root.measured_size().unwrap_or_default().height
}

#[test]
fn leaf_root_sets_builtin_spacing_and_margin_two_hops_up() {
    let spaced = SpacedLayoutLeaf::new();
    let plain = PlainLayoutLeaf::new();
    assert_eq!(
        texts(&*spaced),
        ["layout base", "layout middle", "layout leaf"]
    );
    assert_eq!(spaced.margin(), 18.0);
    assert_eq!(plain.margin(), 0.0);
    // Three stacked children: two spacing gaps, plus the margin on both sides.
    assert_eq!(
        measured_height(&*spaced) - measured_height(&*plain),
        2.0 * 8.0 + 2.0 * 18.0
    );
}

#[test]
fn derived_root_sets_builtin_spacing_and_margin_one_hop_up() {
    let spaced = SpacedLayoutDerived::new();
    let plain = LayoutMiddle::new();
    assert_eq!(spaced.margin(), 18.0);
    // Two stacked children (as in `LayoutMiddle`): one spacing gap, plus the margin on both sides.
    assert_eq!(
        measured_height(&*spaced) - measured_height(&*plain),
        8.0 + 2.0 * 18.0
    );
}

#[elwindui::component(inherits VerticalLayout)]
struct PayloadBase {
    #[prop(default = Rc::new(0))]
    payload: Rc<i32>,

    body: view! {
        TextBlock { text: format!("payload {payload}") }
    },
}

#[elwindui::component]
impl PayloadBase {}

/// Sets the base's non-`Copy` Prop from its own Param, which it also keeps.
#[elwindui::component(inherits crate::PayloadBase)]
struct PayloadDerived {
    #[param]
    source: Rc<i32>,

    body: view! {
        payload: source
        TextBlock { text: format!("source {source}") }
    },
}

#[elwindui::component]
impl PayloadDerived {}

#[test]
fn a_non_copy_own_param_set_on_the_root_is_also_kept() {
    let derived = PayloadDerived::new(Rc::new(7));
    assert_eq!(texts(&*derived), ["payload 7", "source 7"]);
}

/// Defaulted `Option<String>` and `String` Params set from a derived root with plain literals: the
/// value is converted for the declared type, as in a constructor call.
#[elwindui::component(inherits VerticalLayout)]
struct OptionalBase {
    #[param(default = None)]
    label: Option<String>,
    #[param(default = String::from("none"))]
    caption: String,

    body: view! {
        TextBlock { text: format!("{label:?}:{caption}") }
    },
}

#[elwindui::component]
impl OptionalBase {}

#[elwindui::component(inherits crate::OptionalBase)]
struct OptionalDerived {
    body: view! {
        label: "x"
        caption: "c"
        TextBlock { text: "optional derived" }
    },
}

#[elwindui::component]
impl OptionalDerived {}

#[test]
fn defaulted_params_set_on_the_root_are_converted_for_their_type() {
    let derived = OptionalDerived::new();
    assert_eq!(texts(&*derived), ["Some(\"x\"):c", "optional derived"]);
}

/// A semantic brush set on a derived root two hops above (`background`, declared by `Layout`)
/// resolves against the environment the Component is mounted in, here an `EnvironmentScope`.
#[elwindui::component(inherits crate::LayoutMiddle)]
struct BrushedLayoutLeaf {
    body: view! {
        background: BrushStyle::Primary
        TextBlock { text: "brushed" }
    },
}

#[elwindui::component]
impl BrushedLayoutLeaf {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsMiddle)]
struct CrossCrateBrushedLeaf {
    body: view! {
        background: BrushStyle::Primary
        TextBlock { text: "cross brushed" }
    },
}

#[elwindui::component]
impl CrossCrateBrushedLeaf {}

#[elwindui::component(inherits VerticalLayout)]
struct BrushScopeHost {
    #[prop(default = BrushStyle::Value(scoped_brush()))]
    scoped_primary: BrushStyle,

    body: view! {
        EnvironmentScope {
            primary: scoped_primary,
            BrushedLayoutLeaf {}
            CrossCrateBrushedLeaf { title: "b" }
        }
    },
}

#[elwindui::component]
impl BrushScopeHost {}

fn scoped_brush() -> elwindui::core::graphics::Brush {
    elwindui::core::graphics::Brush::Solid(elwindui::core::graphics::Color::rgb(1, 2, 3))
}

#[test]
fn a_root_semantic_brush_resolves_against_the_mount_environment() {
    use elwindui::core::ui::LayoutExt as _;
    let host = BrushScopeHost::new();
    let leaf = elwindui::core::visual_tree::find_all::<BrushedLayoutLeaf>(host.as_ref())
        .into_iter()
        .next()
        .expect("brushed leaf");
    let leaf = leaf.as_any().downcast_ref::<BrushedLayoutLeaf>().unwrap();
    assert_eq!(leaf.base.base.base.background(), Some(scoped_brush()));
    let cross = elwindui::core::visual_tree::find_all::<CrossCrateBrushedLeaf>(host.as_ref())
        .into_iter()
        .next()
        .expect("cross-crate brushed leaf");
    let cross = cross
        .as_any()
        .downcast_ref::<CrossCrateBrushedLeaf>()
        .unwrap();
    assert_eq!(cross.base.base.base.background(), Some(scoped_brush()));
}

/// A required input type less visible than the (always `pub`) generated Component. An inheritable
/// base must still compile with it (no E0446), and a same-crate derived still auto-forwards it.
struct PrivateInput(String);

impl std::fmt::Display for PrivateInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[elwindui::component(inherits VerticalLayout)]
struct PrivateInputBase {
    #[param]
    input: Rc<PrivateInput>,

    body: view! {
        TextBlock { text: format!("{input}") }
    },
}

#[elwindui::component]
impl PrivateInputBase {}

#[elwindui::component(inherits crate::PrivateInputBase)]
struct PrivateInputDerived {
    body: view! {
        TextBlock { text: "private derived" }
    },
}

#[elwindui::component]
impl PrivateInputDerived {}

#[test]
fn a_less_visible_required_input_type_still_compiles_and_forwards() {
    let base = PrivateInputBase::new(Rc::new(PrivateInput("p".to_string())));
    assert_eq!(texts(&*base), ["p"]);
    let derived = PrivateInputDerived::new(Rc::new(PrivateInput("q".to_string())));
    assert_eq!(texts(&*derived), ["q", "private derived"]);
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

/// Sets the base's ordinary Prop and defaulted Param on its root while `title` is auto-forwarded.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsBase)]
struct CrossCrateRootDefaults {
    body: view! {
        flag: true
        step: 5
        TextBlock { text: "cross defaults" }
    },
}

#[elwindui::component]
impl CrossCrateRootDefaults {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalOptionalBase)]
struct CrossCrateOptionalDerived {
    body: view! {
        label: "x"
        caption: "c"
        TextBlock { text: "cross optional" }
    },
}

#[elwindui::component]
impl CrossCrateOptionalDerived {}

#[test]
fn cross_crate_defaulted_params_set_on_the_root_are_converted_for_their_type() {
    let derived = CrossCrateOptionalDerived::new();
    assert_eq!(texts(&*derived), ["Some(\"x\"):c", "cross optional"]);
}

#[test]
fn cross_crate_root_sets_the_base_prop_and_defaulted_param() {
    let derived = CrossCrateRootDefaults::new("d".to_string());
    assert_eq!(texts(&*derived), ["d:true:5", "cross defaults"]);
}

/// Sets the base's non-`Copy` Prop from its own Param, which it also keeps.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsBase)]
struct CrossCrateNoteFromParam {
    #[param]
    memo: String,

    body: view! {
        note: memo
        TextBlock { text: format!("memo {memo}") }
    },
}

#[elwindui::component]
impl CrossCrateNoteFromParam {}

#[test]
fn cross_crate_non_copy_own_param_set_on_the_root_is_also_kept() {
    let derived = CrossCrateNoteFromParam::new("d".to_string(), "!".to_string());
    assert_eq!(texts(&*derived), ["d:false:1!", "memo !"]);
}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsMiddle)]
struct CrossCrateDefaultsLeaf {
    body: view! {
        flag: true
        step: 5
        TextBlock { text: "cross leaf defaults" }
    },
}

#[elwindui::component]
impl CrossCrateDefaultsLeaf {}

#[test]
fn cross_crate_leaf_root_sets_a_base_prop_and_defaulted_param_two_hops_up() {
    let leaf = CrossCrateDefaultsLeaf::new("d".to_string());
    assert_eq!(
        texts(&*leaf),
        ["d:true:5", "defaults middle", "cross leaf defaults"]
    );
}

/// Builtin properties (`spacing` from `VerticalLayout`, `margin` from `UIElement`) set on the
/// root two generated levels above, in another crate, next to a base Prop.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsMiddle)]
struct CrossCrateSpacedLeaf {
    body: view! {
        spacing: 8.0
        margin: 18.0
        flag: true
        TextBlock { text: "cross leaf defaults" }
    },
}

#[elwindui::component]
impl CrossCrateSpacedLeaf {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsMiddle)]
struct CrossCratePlainLeaf {
    body: view! {
        flag: true
        TextBlock { text: "cross leaf defaults" }
    },
}

#[elwindui::component]
impl CrossCratePlainLeaf {}

/// The same builtin properties set one hop over the other crate's shape base.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalDefaultsBase)]
struct CrossCrateSpacedDerived {
    body: view! {
        spacing: 8.0
        margin: 18.0
        TextBlock { text: "cross defaults" }
    },
}

#[elwindui::component]
impl CrossCrateSpacedDerived {}

#[test]
fn cross_crate_root_sets_builtin_spacing_and_margin() {
    let spaced = CrossCrateSpacedLeaf::new("d".to_string());
    let plain = CrossCratePlainLeaf::new("d".to_string());
    assert_eq!(
        texts(&*spaced),
        ["d:true:1", "defaults middle", "cross leaf defaults"]
    );
    assert_eq!(spaced.margin(), 18.0);
    assert_eq!(plain.margin(), 0.0);
    assert_eq!(
        measured_height(&*spaced) - measured_height(&*plain),
        2.0 * 8.0 + 2.0 * 18.0
    );

    let derived = CrossCrateSpacedDerived::new("d".to_string());
    let plain = CrossCrateRootDefaults::new("d".to_string());
    assert_eq!(derived.margin(), 18.0);
    assert_eq!(
        measured_height(&*derived) - measured_height(&*plain),
        8.0 + 2.0 * 18.0
    );
}

/// A cross-crate base whose type and required input names both contain `__`.
#[elwindui::component(inherits elwindui_external_component_fixture::Odd__Name)]
struct CrossCrateOddName {
    body: view! {
        TextBlock { text: format!("derived {item__id}") }
    },
}

#[elwindui::component]
impl CrossCrateOddName {}

/// Constructs that derived Component from a view, so its constructor ABI converts `item__id`
/// through the base's (`@__ctor_arg_item__id`, found from the required-input type alias).
#[elwindui::component(inherits VerticalLayout)]
struct CrossCrateOddNameUser {
    body: view! {
        CrossCrateOddName { item__id: "used" }
    },
}

#[elwindui::component]
impl CrossCrateOddNameUser {}

#[test]
fn cross_crate_names_containing_double_underscores_are_forwarded() {
    let derived = CrossCrateOddName::new("i".to_string());
    assert_eq!(texts(&*derived), ["odd i", "derived i"]);

    let user = CrossCrateOddNameUser::new();
    let children = user.visual_children();
    assert_eq!(children.len(), 1);
    let inner = children[0]
        .as_any()
        .downcast_ref::<CrossCrateOddName>()
        .expect("constructed derived component");
    assert_eq!(texts(inner), ["odd used", "derived used"]);
}

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
