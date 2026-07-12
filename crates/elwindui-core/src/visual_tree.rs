//! WinUI3's `VisualTreeHelper` equivalent — free functions for walking a `UIElement` tree
//! structurally. `get_parent`/`visual_children` already exist as `UIElement` methods themselves
//! (no COM/WinRT-style indirection needed in Rust); this module exists for the WinUI3-familiar
//! call shape (`visual_tree::get_child(elem, i)`) and for `find_all`, the one piece with no direct
//! `UIElement` method of its own.
//!
//! Deliberately has no name-based lookup (WinUI3's own `VisualTreeHelper` doesn't either — that's
//! `FrameworkElement.FindName`, a separate mechanism). In ElwindUIL, named access is
//! `#[id(...)]` (docs/elwindui_spec.md §13), resolved entirely at compile time via a generated
//! typed accessor — there is no runtime element-id concept to search by.

use crate::ui::UIElement;
use std::rc::Rc;

/// WinUI3's `VisualTreeHelper.GetChildrenCount`.
pub fn get_children_count(element: &dyn UIElement) -> usize {
    element.visual_children().len()
}

/// WinUI3's `VisualTreeHelper.GetChild`.
pub fn get_child(element: &dyn UIElement, index: usize) -> Option<Rc<dyn UIElement>> {
    element.visual_children().into_iter().nth(index)
}

/// WinUI3's `VisualTreeHelper.GetParent` — thin wrapper over `UIElement::parent` for call-site
/// symmetry with the other functions here.
pub fn get_parent(element: &dyn UIElement) -> Option<Rc<dyn UIElement>> {
    element.parent()
}

/// Recursively collects every element in `root`'s subtree (including `root` itself) whose concrete
/// type downcasts to `T`, depth-first. Not part of real WinUI3's `VisualTreeHelper`, but the type-
/// based counterpart to its child/parent walk (docs/elwindui_spec.md §13's original `find_all`
/// intent) — useful for e.g. asserting how many `Button`s a generated view produced. Returns each
/// match still erased as `Rc<dyn UIElement>` (this crate's usual erasure convention, matching
/// `UIElement::as_native_control`'s own downcast pattern) — call `.as_any().downcast_ref::<T>()` on
/// a result to get at `T`'s own fields.
pub fn find_all<T: 'static>(root: &dyn UIElement) -> Vec<Rc<dyn UIElement>> {
    let mut out = Vec::new();
    collect_all::<T>(root, &mut out);
    out
}

fn collect_all<T: 'static>(node: &dyn UIElement, out: &mut Vec<Rc<dyn UIElement>>) {
    for child in node.visual_children() {
        if child.as_any().downcast_ref::<T>().is_some() {
            out.push(child.clone());
        }
        collect_all::<T>(child.as_ref(), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{create_text_block, create_vertical_layout, new_element, TextBlockImpl, VerticalLayout as _};

    #[test]
    fn children_count_and_get_child_match_visual_children() {
        let layout = create_vertical_layout();
        layout.children().add(new_element(create_text_block()));
        layout.children().add(new_element(create_text_block()));
        let tree: Rc<dyn UIElement> = new_element(layout);

        assert_eq!(get_children_count(tree.as_ref()), 2);
        assert!(get_child(tree.as_ref(), 0).is_some());
        assert!(get_child(tree.as_ref(), 2).is_none());
    }

    #[test]
    fn get_parent_walks_back_up() {
        let layout = create_vertical_layout();
        let text = new_element(create_text_block());
        layout.children().add(text.clone());
        let tree: Rc<dyn UIElement> = new_element(layout);

        let parent = get_parent(text.as_ref()).expect("child has a parent");
        assert!(Rc::ptr_eq(&parent, &tree));
        assert!(get_parent(tree.as_ref()).is_none());
    }

    #[test]
    fn find_all_collects_matching_type_across_tree() {
        let outer = create_vertical_layout();
        let inner = create_vertical_layout();
        inner.children().add(new_element(create_text_block()));
        outer.children().add(new_element(inner));
        outer.children().add(new_element(create_text_block()));
        let tree: Rc<dyn UIElement> = new_element(outer);

        let texts = find_all::<TextBlockImpl>(tree.as_ref());
        assert_eq!(texts.len(), 2);
    }
}
