//! Downstream-style coverage for qualified external generated-component paths (Issue #191).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::rc::Rc;

use elwindui::component;
use elwindui::core::ui::{ContentControlExt, UIElementExt as _};
use elwindui_external_component_fixture::{ExternalProbeItemExt, ExternalProbeTabsExt};

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
