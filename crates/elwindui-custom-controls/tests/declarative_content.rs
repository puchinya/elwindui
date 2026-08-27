#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

extern crate elwindui_custom_controls as elwindui;

use elwindui::component;
use elwindui::core::base::Size;
use elwindui::core::graphics::{RenderCommand, RenderGroup, RenderTree};
use elwindui::core::ui::{ContentControlExt, TextBlock, UIElementExt, layout_root};
use elwindui::{CustomTabViewExt, CustomTabViewItemExt};
use std::rc::Rc;

#[component(inherits VerticalLayout)]
struct DeclarativeTabHost {
    body: view! {
        #[id("tabs")]
        let tabs = CustomTabView {
            CustomTabViewItem {
                header: "Document"
                TextBlock { text: "Page body" }
            }
        };

        tabs
    },
}

#[component]
impl DeclarativeTabHost {}

fn render_commands<'a>(group: &'a RenderGroup, out: &mut Vec<&'a RenderCommand>) {
    out.extend(group.commands.iter());
    for child in &group.children {
        render_commands(child, out);
    }
}

fn rendered_texts(tree: &RenderTree) -> Vec<String> {
    let mut commands = Vec::new();
    render_commands(&tree.root, &mut commands);
    commands
        .into_iter()
        .filter_map(|command| match command {
            RenderCommand::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn subtree_texts(node: &Rc<dyn UIElementExt>, out: &mut Vec<String>) {
    if let Some(text_block) = node.as_any().downcast_ref::<TextBlock>() {
        out.push(text_block.text.borrow().clone());
    }
    for child in node.visual_children() {
        subtree_texts(&child, out);
    }
}

fn contains_identity(root: &Rc<dyn UIElementExt>, target: &Rc<dyn UIElementExt>) -> bool {
    if Rc::ptr_eq(root, target) {
        return true;
    }
    root.visual_children()
        .iter()
        .any(|child| contains_identity(child, target))
}

#[test]
fn declarative_bare_children_preserve_content_and_template_ownership() {
    let host = DeclarativeTabHost::new();
    let tabs = host.tabs();

    let typed_items = <elwindui::CustomTabView as CustomTabViewExt>::children(&*tabs);
    let list_items = tabs.children().to_vec();
    assert_eq!(typed_items.len(), 1);
    assert_eq!(list_items.len(), 1);

    let authored_item = typed_items[0].clone();
    let authored_item_ext: Rc<dyn CustomTabViewItemExt> = authored_item.clone();
    assert!(Rc::ptr_eq(&list_items[0], &authored_item_ext));
    assert_eq!(authored_item.header(), "Document");

    let page = authored_item.content();
    let page_text = page
        .as_any()
        .downcast_ref::<TextBlock>()
        .expect("the authored page remains a TextBlock")
        .text
        .borrow()
        .clone();
    assert_eq!(page_text, "Page body");

    let item: Rc<dyn UIElementExt> = authored_item.clone();
    let logical_parent = page
        .parent()
        .expect("the page has a logical content parent");
    assert!(Rc::ptr_eq(&logical_parent, &item));

    let tabs_root: Rc<dyn UIElementExt> = tabs;
    layout_root(
        &tabs_root,
        Size {
            width: 360.0,
            height: 180.0,
        },
    );

    let visual_parent = page
        .visual_parent()
        .expect("the page is mounted into the content presenter");
    assert!(
        visual_parent
            .type_name()
            .contains("CustomTabContentPresenter")
    );

    let mut header_texts = Vec::new();
    subtree_texts(&item, &mut header_texts);
    assert!(header_texts.iter().any(|text| text == "Document"));
    assert!(!header_texts.iter().any(|text| text == "Page body"));

    assert!(!contains_identity(&item, &page));

    let tree = RenderTree::new::<()>(&tabs_root);
    let texts = rendered_texts(&tree);
    assert!(texts.iter().any(|text| text == "Document"));
    assert!(texts.iter().any(|text| text == "Page body"));
}
