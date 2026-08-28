//! Downstream-style coverage for qualified external generated-component paths (Issue #191).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::rc::Rc;

use elwindui::component;
use elwindui::core::ui::{ContentControlExt, UIElementExt as _};
use elwindui_external_component_fixture::{
    ExternalProbeItem, ExternalProbeItemExt, ExternalProbeTabs, ExternalProbeTabsExt,
};

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
