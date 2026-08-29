//! Downstream-style coverage for qualified external generated-component paths (Issue #191).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::rc::Rc;

use elwindui::component;
use elwindui::core::ui::{ContentControlExt, UIElementExt as _, WindowExt};
use elwindui_external_component_fixture::{
    ExternalProbeItem, ExternalProbeItemExt, ExternalProbeTabs, ExternalProbeTabsExt,
    RequiredExternalCardExt,
};

#[component(inherits VerticalLayout)]
struct LocalNewProbe {
    #[param]
    required: String,
    #[param]
    optional_param: Option<String>,
    #[param(default = 3)]
    fixed: usize,
    #[prop]
    optional: Option<String>,
    #[prop(default = String::from("default"))]
    label: String,
    body: view! {
        TextBlock { text: required }
    },
}

#[component]
impl LocalNewProbe {}

#[component(inherits VerticalLayout)]
struct LocalInheritedNewBase {
    #[param]
    id: String,
    #[param(default = String::from("base-default"))]
    base_mode: String,
    #[prop]
    title: String,
    body: view! {
        #[id("rendered")]
        let rendered = TextBlock { text: format!("{id}:{title}") };

        rendered
    },
}

#[component]
impl LocalInheritedNewBase {}

#[component(inherits crate::LocalInheritedNewBase)]
struct LocalInheritedNewDerived {
    #[param(default = false)]
    compact: bool,
    #[prop]
    subtitle: String,
}

#[component]
impl LocalInheritedNewDerived {}

#[elwindui::viewmodel]
mod local_inherited_new_model {
    struct LocalInheritedNewModel {
        #[observable(default = String::new())]
        value: String,
    }
}

#[component(inherits VerticalLayout)]
struct LocalInheritedBindableBase {
    #[bindable]
    model: Rc<LocalInheritedNewModel>,
    body: view! {
        TextBlock { text: model.value }
    },
}

#[component]
impl LocalInheritedBindableBase {}

#[component(inherits crate::LocalInheritedBindableBase)]
struct LocalInheritedBindableDerived {}

#[component]
impl LocalInheritedBindableDerived {}

#[test]
fn new_macro_constructs_local_component_with_own_fields() {
    let value = elwindui::new!(LocalNewProbe(
        required: "required",
        optional_param: Some("optional-param"),
        fixed: 9,
        optional: None,
        label: "label",
    ));

    assert_eq!(LocalNewProbeExt::required(&*value), "required");
    assert_eq!(
        LocalNewProbeExt::optional_param(&*value),
        Some(String::from("optional-param"))
    );
    assert_eq!(LocalNewProbeExt::fixed(&*value), 9);
    assert_eq!(LocalNewProbeExt::optional(&*value), None);
    assert_eq!(LocalNewProbeExt::label(&*value), "label");
}

#[test]
fn new_macro_constructs_local_component_with_effective_inherited_shape() {
    let value = elwindui::new!(LocalInheritedNewDerived(
        subtitle: "sub",
        title: "title",
        compact: true,
        id: "id",
    ));

    assert_eq!(LocalInheritedNewBaseExt::id(&value.base), "id");
    assert_eq!(
        LocalInheritedNewBaseExt::base_mode(&value.base),
        "base-default"
    );
    assert_eq!(LocalInheritedNewBaseExt::title(&value.base), "title");
    assert!(LocalInheritedNewDerivedExt::compact(&*value));
    assert_eq!(LocalInheritedNewDerivedExt::subtitle(&*value), "sub");
    assert_eq!(value.base.rendered().text.borrow().as_str(), "id:title");

    LocalInheritedNewBaseExt::set_title(&value.base, String::from("updated"));
    assert_eq!(LocalInheritedNewBaseExt::title(&value.base), "updated");
}

#[test]
fn new_macro_accepts_inherited_required_bindable() {
    let model = LocalInheritedNewModel::new();
    let value = elwindui::new!(LocalInheritedBindableDerived(model: Rc::clone(&model)));

    assert!(Rc::ptr_eq(
        &LocalInheritedBindableBaseExt::model(&value.base),
        &model
    ));
}

// Compile/type-shape coverage only. AppKit Window creation is not executed from an ordinary Rust
// test worker; the codegen semantic expansion test and Windows main-thread test own behavior proof.
#[allow(dead_code)]
fn new_macro_constructs_builtin_window_with_named_property() {
    let _window = elwindui::new!(Window(title: "Text"));
}

/// This is intentionally a normal consumer shape: `elwindui` and the external generated
/// component crate are separate dependencies, and the DSL uses the external crate-qualified path
/// directly. There is no crate alias or local facade module here.
#[component(inherits VerticalLayout)]
struct ExternalControlsHost {
    #[prop(default = String::from("Document"))]
    consumer_title: String,
    #[prop(default = 0)]
    selected: usize,
    body: view! {
        #[id("item")]
        let item = elwindui_external_component_fixture::ExternalProbeItem {
            title: consumer_title
            closable: false
            TextBlock { text: "Page body" }
        };

        #[id("tabs")]
        let tabs = elwindui_external_component_fixture::ExternalProbeTabs {
            selected_index <=> selected
            item
        };

        tabs
    },
}

#[component]
impl ExternalControlsHost {}

#[component(inherits Control)]
struct ExternalDynamicIfTemplateHost {
    #[prop(default = true)]
    show_first: bool,
    template: template_view! {
        elwindui_external_component_fixture::ExternalProbeTabs {
            if show_first {
                elwindui_external_component_fixture::ExternalProbeItem {
                    title: "A"
                }
            } else {
                elwindui_external_component_fixture::ExternalProbeItem {
                    title: "B"
                }
            }
        }
    },
}

#[component]
impl ExternalDynamicIfTemplateHost {}

#[component(inherits VerticalLayout)]
struct ExternalNestedModuleHost {
    body: view! {
        elwindui_external_component_fixture::nested::NestedExternalProbe {
            label: "nested"
        }
    },
}

#[component]
impl ExternalNestedModuleHost {}

#[component(inherits VerticalLayout)]
struct ExternalAliasHost {
    body: view! {
        external_component_fixture_alias::AliasedExternalProbe {
            label: "alias"
        }
    },
}

#[component]
impl ExternalAliasHost {}

#[component(inherits Control)]
struct ExternalDynamicForTemplateHost {
    #[prop(default = Vec::new())]
    labels: Vec<String>,
    template: template_view! {
        elwindui_external_component_fixture::ExternalProbeTabs {
            for label in labels {
                elwindui_external_component_fixture::ExternalProbeItem {
                    title: label
                }
            }
        }
    },
}

#[component]
impl ExternalDynamicForTemplateHost {}

#[component(inherits VerticalLayout)]
struct ExternalShapeHost {
    #[prop(default = Some(String::from("optional")))]
    optional_value: Option<String>,
    #[prop(default = Some(String::from("deferred")))]
    deferred_value: Option<String>,
    body: view! {
        elwindui_external_component_fixture::ExternalShapeProbe {
            count: 7
            optional: optional_value
            deferred: deferred_value
        }
    },
}

#[component]
impl ExternalShapeHost {}

#[test]
fn new_macro_constructs_external_generated_component_with_named_fields() {
    let title_calls = Rc::new(std::cell::Cell::new(0));
    let fallback_calls = Rc::new(std::cell::Cell::new(0));
    let title_calls_for_expr = Rc::clone(&title_calls);
    let fallback_calls_for_expr = Rc::clone(&fallback_calls);
    let card = elwindui::new!(elwindui_external_component_fixture::RequiredExternalCard(
        mutable_label: "mutable",
        optional_fallback: {
            fallback_calls_for_expr.set(fallback_calls_for_expr.get() + 1);
            String::from("fallback")
        },
        optional: Some("optional"),
        count: 7,
        fixed: 9,
        defaulted_optional: Some("defaulted"),
        title: {
            title_calls_for_expr.set(title_calls_for_expr.get() + 1);
            String::from("title")
        },
    ));

    assert_eq!(title_calls.get(), 1);
    assert_eq!(fallback_calls.get(), 1);
    assert_eq!(RequiredExternalCardExt::title(&*card), "title");
    assert_eq!(RequiredExternalCardExt::count(&*card), 7);
    assert_eq!(RequiredExternalCardExt::fixed(&*card), 9);
    assert_eq!(
        RequiredExternalCardExt::defaulted_optional(&*card),
        Some(String::from("defaulted"))
    );
    assert_eq!(
        RequiredExternalCardExt::optional(&*card),
        Some(String::from("optional"))
    );
    assert_eq!(
        RequiredExternalCardExt::optional_fallback(&*card),
        "fallback"
    );
    assert_eq!(RequiredExternalCardExt::mutable_label(&*card), "mutable");

    let defaulted_card = elwindui::new!(elwindui_external_component_fixture::RequiredExternalCard(
        title: "defaulted",
        count: 1,
        optional: None,
    ));
    assert_eq!(RequiredExternalCardExt::fixed(&*defaulted_card), 5);
    assert_eq!(
        RequiredExternalCardExt::defaulted_optional(&*defaulted_card),
        None
    );
}

#[test]
fn new_macro_constructs_same_crate_generated_component_and_mounts_it() {
    let probe = elwindui::new!(LocalNewProbe(
        label: "label",
        optional: Some(String::from("optional")),
        optional_param: Some("parameter"),
        fixed: 9,
        required: "required",
    ));

    assert_eq!(LocalNewProbeExt::required(&*probe), "required");
    assert_eq!(
        LocalNewProbeExt::optional_param(&*probe),
        Some(String::from("parameter"))
    );
    assert_eq!(LocalNewProbeExt::fixed(&*probe), 9);
    assert_eq!(
        LocalNewProbeExt::optional(&*probe),
        Some(String::from("optional"))
    );
    assert_eq!(LocalNewProbeExt::label(&*probe), "label");
    assert_eq!(probe.visual_children().len(), 1);
}

#[test]
fn qualified_external_components_preserve_properties_content_and_resync() {
    let host = ExternalControlsHost::new();
    let authored_item = host.item();
    let tabs = host.tabs();

    assert_eq!(ExternalProbeItemExt::title(&*authored_item), "Document");
    assert!(!ExternalProbeItemExt::closable(&*authored_item));
    let content = ContentControlExt::content(&*authored_item);
    assert_eq!(
        content
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("external item content is TextBlock")
            .text
            .borrow()
            .as_str(),
        "Page body"
    );

    let stored_items = ExternalProbeTabsExt::children(&*tabs);
    assert_eq!(stored_items.len(), 1);
    assert!(Rc::ptr_eq(&authored_item, &stored_items[0]));

    // The external own property assignment is reactive, not construction-only. The external
    // item's generated template reads `title`, so its visual TextBlock must resync as well.
    host.set_consumer_title(String::from("Renamed"));
    assert_eq!(ExternalProbeItemExt::title(&*authored_item), "Renamed");
    let item_root = authored_item
        .visual_children()
        .into_iter()
        .next()
        .expect("external item template root is attached");
    assert_eq!(
        item_root
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("external item template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "Renamed"
    );

    // The external generated two-way property is wired through its exported shape, too.
    assert_eq!(ExternalProbeTabsExt::selected_index(&*tabs), 0);
    tabs.select_index(1);
    assert_eq!(host.selected(), 1);
}

#[test]
fn qualified_external_template_dynamic_if_replaces_collection_item() {
    let host = ExternalDynamicIfTemplateHost::new();
    let root = host
        .visual_children()
        .into_iter()
        .next()
        .expect("external dynamic template host has one root");
    let tabs = root
        .as_any()
        .downcast_ref::<ExternalProbeTabs>()
        .expect("template root is the external collection host");

    let initial = ExternalProbeTabsExt::children(tabs);
    assert_eq!(initial.len(), 1);
    assert_eq!(ExternalProbeItemExt::title(&*initial[0]), "A");
    let old_item = initial[0].clone();

    host.set_show_first(false);

    let replacement = ExternalProbeTabsExt::children(tabs);
    assert_eq!(replacement.len(), 1);
    assert_eq!(ExternalProbeItemExt::title(&*replacement[0]), "B");
    assert!(!Rc::ptr_eq(&old_item, &replacement[0]));
    assert!(replacement.iter().all(|item| !Rc::ptr_eq(item, &old_item)));
}

#[test]
fn qualified_external_nested_module_uses_root_props_macro() {
    let host = ExternalNestedModuleHost::new();
    let nested = host
        .visual_children()
        .into_iter()
        .next()
        .expect("nested external component is attached");
    let nested = nested
        .as_any()
        .downcast_ref::<elwindui_external_component_fixture::nested::NestedExternalProbe>()
        .expect("nested external component retains its authored module path");
    let text = nested
        .visual_children()
        .into_iter()
        .next()
        .expect("nested component template root is attached");
    assert_eq!(
        text.as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("nested component template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "nested"
    );
}

#[test]
fn cargo_alias_external_component_uses_defining_crate_shape() {
    let host = ExternalAliasHost::new();
    let item = host
        .visual_children()
        .into_iter()
        .next()
        .expect("aliased external component is attached");
    let item = item
        .as_any()
        .downcast_ref::<external_component_fixture_alias::AliasedExternalProbe>()
        .expect("aliased path constructs the external component type");
    assert_eq!(
        item.visual_children()
            .into_iter()
            .next()
            .expect("aliased component template root is attached")
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("aliased component template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "alias"
    );
}

#[test]
fn new_macro_constructs_cargo_aliased_external_component() {
    let item = elwindui::new!(external_component_fixture_alias::AliasedExternalProbe(
        label: "new alias"
    ));
    let text = item
        .visual_children()
        .into_iter()
        .next()
        .expect("aliased component template root is attached")
        .as_any()
        .downcast_ref::<elwindui::core::ui::TextBlock>()
        .expect("aliased component template root is TextBlock")
        .text
        .borrow()
        .to_string();
    assert_eq!(text, "new alias");
}

#[test]
fn qualified_external_template_dynamic_for_replaces_collection_items() {
    let host = ExternalDynamicForTemplateHost::new();
    let root = host
        .visual_children()
        .into_iter()
        .next()
        .expect("external dynamic template host has one root");
    let tabs = root
        .as_any()
        .downcast_ref::<ExternalProbeTabs>()
        .expect("template root is the external collection host");
    assert!(ExternalProbeTabsExt::children(tabs).is_empty());

    host.set_labels(vec![String::from("A"), String::from("B")]);
    let initial = ExternalProbeTabsExt::children(tabs);
    assert_eq!(initial.len(), 2);
    assert_eq!(ExternalProbeItemExt::title(&*initial[0]), "A");
    assert_eq!(ExternalProbeItemExt::title(&*initial[1]), "B");
    let old = initial[0].clone();

    host.set_labels(vec![String::from("C")]);
    let replacement = ExternalProbeTabsExt::children(tabs);
    assert_eq!(replacement.len(), 1);
    assert_eq!(ExternalProbeItemExt::title(&*replacement[0]), "C");
    assert!(!Rc::ptr_eq(&old, &replacement[0]));
    assert!(replacement.iter().all(|item| !Rc::ptr_eq(item, &old)));
}

#[test]
fn qualified_external_shape_preserves_scalar_and_option_props() {
    let host = ExternalShapeHost::new();
    let probe_root = host
        .visual_children()
        .into_iter()
        .next()
        .expect("external shape probe is attached");
    let probe = probe_root
        .as_any()
        .downcast_ref::<elwindui_external_component_fixture::ExternalShapeProbe>()
        .expect("external shape path constructs the fixture component");

    assert_eq!(
        elwindui_external_component_fixture::ExternalShapeProbeExt::count(probe),
        7
    );
    assert_eq!(
        elwindui_external_component_fixture::ExternalShapeProbeExt::optional(probe),
        Some(String::from("optional"))
    );
    assert_eq!(
        elwindui_external_component_fixture::ExternalShapeProbeExt::deferred(probe),
        Some(String::from("deferred"))
    );
    assert_eq!(
        elwindui_external_component_fixture::ExternalShapeProbeExt::computed_value(probe),
        7
    );
}

#[test]
fn external_dynamic_reconciliation_publishes_one_children_commit_and_updates_dependents() {
    let host = ExternalDynamicForTemplateHost::new();
    let root = host
        .visual_children()
        .into_iter()
        .next()
        .expect("external dynamic template host has one root");
    let tabs = root
        .as_any()
        .downcast_ref::<ExternalProbeTabs>()
        .expect("template root is the external collection host");

    let children_commits = Rc::new(std::cell::Cell::new(0));
    let commits = Rc::clone(&children_commits);
    let _subscription = tabs.subscribe_property_changed(move |property| {
        if property == elwindui_external_component_fixture::ExternalProbeTabsProperty::children {
            commits.set(commits.get() + 1);
        }
    });

    assert_eq!(ExternalProbeTabsExt::child_count(tabs), 0);
    assert_eq!(
        tabs.visual_children()
            .into_iter()
            .next()
            .expect("external tabs template root is attached")
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("external tabs template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "0"
    );

    // This reconciliation performs two raw inserts but publishes one completed children update.
    host.set_labels(vec![String::from("A"), String::from("B")]);
    assert_eq!(children_commits.get(), 1);
    assert_eq!(ExternalProbeTabsExt::child_count(tabs), 2);
    assert_eq!(
        tabs.visual_children()
            .into_iter()
            .next()
            .expect("external tabs template root is attached")
            .as_any()
            .downcast_ref::<elwindui::core::ui::TextBlock>()
            .expect("external tabs template root is TextBlock")
            .text
            .borrow()
            .as_str(),
        "2"
    );

    host.set_labels(vec![String::from("C")]);
    assert_eq!(children_commits.get(), 2);
    assert_eq!(ExternalProbeTabsExt::child_count(tabs), 1);
}
