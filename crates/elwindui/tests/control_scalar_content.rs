//! Runtime coverage for the metadata-driven scalar content path on `Control`.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::{TextBlock, UIElementExt};
use std::rc::Rc;

#[elwindui::component(inherits Control)]
struct ScalarControlProbe {
    #[prop(default = false)]
    alternate: bool,

    template: template_view! {
        if alternate {
            TextBlock { text: "A" }
        } else {
            TextBlock { text: "B" }
        }
    },
}

#[elwindui::component]
impl ScalarControlProbe {}

#[test]
fn scalar_control_content_replaces_one_visual_root_on_property_change() {
    let probe = ScalarControlProbe::new();
    let old_root = probe
        .visual_children()
        .into_iter()
        .next()
        .expect("initial scalar branch should attach one root");
    let old_text = old_root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("initial root should be TextBlock")
        .text
        .borrow()
        .clone();
    assert_eq!(old_text, "B");
    assert_eq!(probe.visual_children().len(), 1);

    probe.set_alternate(true);

    let new_root = probe
        .visual_children()
        .into_iter()
        .next()
        .expect("replacement scalar branch should attach one root");
    let new_text = new_root
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("replacement root should be TextBlock")
        .text
        .borrow()
        .clone();
    assert_eq!(new_text, "A");
    assert_eq!(probe.visual_children().len(), 1);
    assert!(old_root.visual_parent().is_none());
    let owner: Rc<dyn UIElementExt> = probe.clone();
    assert!(
        new_root
            .visual_parent()
            .is_some_and(|parent| Rc::ptr_eq(&parent, &owner))
    );
}
