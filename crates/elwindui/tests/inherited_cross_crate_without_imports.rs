//! Issue #166, cross-crate: a minimal downstream crate whose derived Components read and write
//! properties declared by a generated Component in *another* crate
//! (`elwindui-external-component-fixture`).
//!
//! This file deliberately has **no `use` items at all**: the DSL author must not need any
//! generated-trait import (`use ..Ext as _`) for the generated code — view construction,
//! handlers, per-property `__resync_inherited_<name>` methods and the base subscription — to
//! resolve. The test-side checks call ancestor accessors through fully qualified UFCS, so they
//! never put a trait into scope for the generated code either.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

fn text_of(block: &elwindui::ui::TextBlock) -> String {
    block.text.borrow().clone()
}

fn tap(target: std::rc::Rc<dyn elwindui::core::ui::UIElementExt>) {
    let routed_args = elwindui::core::input::RoutedEventArgs::default();
    elwindui::core::ui::dispatch_routed(
        &target,
        "on_tapped",
        &elwindui::core::input::TappedEventArgs {
            position: elwindui::core::base::Point { x: 1.0, y: 1.0 },
            modifiers: elwindui::core::input::KeyModifiers::default(),
        },
        &routed_args,
    );
}

fn external_heading(base: &elwindui_external_component_fixture::ExternalInheritBase) -> String {
    <elwindui_external_component_fixture::ExternalInheritBase as elwindui_external_component_fixture::ExternalInheritBaseExt>::heading(base)
}

fn set_external_heading(
    base: &elwindui_external_component_fixture::ExternalInheritBase,
    value: &str,
) {
    <elwindui_external_component_fixture::ExternalInheritBase as elwindui_external_component_fixture::ExternalInheritBaseExt>::set_heading(base, value.to_string())
}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritBase)]
struct CrossCrateDerived {
    body: view! {
        #[id("cross_label")]
        let cross_label = TextBlock {
            text: heading,
            on_tapped: |_event| {
                heading = format!("{heading}+tap");
            },
        };

        cross_label
    },
}

#[elwindui::component]
impl CrossCrateDerived {}

#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritMiddle)]
struct CrossCrateLeaf {
    body: view! {
        #[id("leaf_cross_label")]
        let leaf_cross_label = TextBlock {
            text: format!("[{heading}]"),
            on_tapped: |_event| {
                heading = "leaf".to_string();
            },
        };

        leaf_cross_label
    },
}

#[elwindui::component]
impl CrossCrateLeaf {}

#[test]
fn cross_crate_derived_reads_and_writes_external_ancestor_prop() {
    let derived = CrossCrateDerived::new();
    assert_eq!(text_of(&derived.cross_label()), "external-before");
    tap(derived.cross_label());
    assert_eq!(external_heading(&derived.base), "external-before+tap");
    assert_eq!(text_of(&derived.cross_label()), "external-before+tap");
}

#[test]
fn cross_crate_external_write_resyncs_derived_view() {
    let derived = CrossCrateDerived::new();
    set_external_heading(&derived.base, "from-outside");
    assert_eq!(text_of(&derived.cross_label()), "from-outside");
}

#[test]
fn cross_crate_multi_hop_through_external_middle() {
    let leaf = CrossCrateLeaf::new();
    assert_eq!(text_of(&leaf.leaf_cross_label()), "[external-before]");
    tap(leaf.leaf_cross_label());
    assert_eq!(external_heading(&leaf.base.base), "leaf");
    assert_eq!(text_of(&leaf.leaf_cross_label()), "[leaf]");
    set_external_heading(&leaf.base.base, "outside");
    assert_eq!(text_of(&leaf.leaf_cross_label()), "[outside]");
}

// View-less local middle over a qualified external generated base: it is itself an inheritable
// composition hop, and `heading` (declared in the external crate) stays reachable two hops down.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritBase)]
struct LocalViewlessMiddle {
    #[prop(default = 5)]
    local_only: i32,
}

#[elwindui::component]
impl LocalViewlessMiddle {}

#[elwindui::component(inherits crate::LocalViewlessMiddle)]
struct LocalViewlessLeaf {
    body: view! {
        #[id("viewless_leaf_label")]
        let viewless_leaf_label = TextBlock {
            text: format!("{heading}/{local_only}"),
            on_tapped: |_event| {
                heading = "viewless".to_string();
                local_only += 1;
            },
        };

        viewless_leaf_label
    },
}

#[elwindui::component]
impl LocalViewlessLeaf {}

#[test]
fn cross_crate_multi_hop_through_local_viewless_middle() {
    let middle = LocalViewlessMiddle::new();
    assert_eq!(external_heading(&middle.base), "external-before");
    assert_eq!(middle.local_only(), 5);

    let leaf = LocalViewlessLeaf::new();
    assert_eq!(text_of(&leaf.viewless_leaf_label()), "external-before/5");
    tap(leaf.viewless_leaf_label());
    assert_eq!(external_heading(&leaf.base.base), "viewless");
    assert_eq!(leaf.base.local_only(), 6);
    assert_eq!(text_of(&leaf.viewless_leaf_label()), "viewless/6");
}

fn visual_texts(root: &dyn elwindui::core::ui::UIElementExt) -> Vec<String> {
    elwindui::core::ui::UIElementExt::visual_children(root)
        .iter()
        .filter_map(|child| {
            child
                .as_any()
                .downcast_ref::<elwindui::ui::TextBlock>()
                .map(text_of)
        })
        .collect()
}

/// The external base's own view keeps re-syncing while it is embedded as a downstream derived
/// Component's `base`, and the derived root region lands after the base-owned child.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInheritBase)]
struct CrossCrateRegionDerived {
    #[prop(default = false)]
    show_extra: bool,

    body: view! {
        TextBlock { text: "own" }
        if show_extra {
            TextBlock { text: "extra" }
        }
    },
}

#[elwindui::component]
impl CrossCrateRegionDerived {}

#[test]
fn cross_crate_embedded_base_view_and_root_region_order() {
    let derived = CrossCrateRegionDerived::new();
    assert_eq!(
        visual_texts(&*derived),
        vec!["external-before:7".to_string(), "own".to_string()]
    );
    set_external_heading(&derived.base, "changed");
    derived.set_show_extra(true);
    assert_eq!(
        visual_texts(&*derived),
        vec![
            "changed:7".to_string(),
            "own".to_string(),
            "extra".to_string()
        ]
    );
}
