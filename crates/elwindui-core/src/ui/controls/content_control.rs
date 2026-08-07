//! `builtin::ContentControl` — a `Control` holding a single content slot.

use super::*;

/// `builtin::ContentControl`(docs/design/gui_framework_design.md §5.1)— 単一の子(`content`)を持つ
/// `Control`の薄いラッパー。二重管理を避けるため、バックエンド非依存な合成 builtin としてここに直接手書きする。
/// Content is a single Visual child managed directly by this type.
#[elwindui_macros::class(inherits = crate::ui::Control)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
pub struct ContentControl {
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
}

#[elwindui_macros::class]
impl ContentControl {
    fn content(&self) -> Rc<dyn UIElementExt> {
        self.content
            .borrow()
            .clone()
            .expect("ContentControl has no content")
    }
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        let old = self.content.borrow_mut().replace(content.clone());
        if let Some(old) = old {
            *old.as_ui_element().parent.borrow_mut() = None;
            self.as_ui_element().visual_collection.remove(&old);
        }
        // `visual_collection.add` (below) is what routed-event bubbling (`dispatch_routed`) actually
        // relies on now — it walks `visual_parent`. Setting `content`'s Logical `parent` here too is
        // no longer needed for routing; it's kept purely so the Logical tree (a receptacle for a
        // future template/accessibility tree, see `UIElementCollection`'s own doc comment) stays
        // consistent — mirrors what `UIElementCollection::add` already does for `Layout::children`.
        if let Some(owner) = self.as_ui_element().visual_collection.owner_rc() {
            *content.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        self.as_ui_element().visual_collection.add(content);
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // The bare value is embedded as the base of generated subclasses. Content is attached only
    // after that outer `Rc` exists, through `set_content`, so collection mutation owns the Visual
    // parent wiring.
    fn construct() -> Self {
        Self {
            base: Control::construct(),
            content: RefCell::new(None),
        }
    }
}
