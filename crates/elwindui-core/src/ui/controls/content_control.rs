//! `elwindui::ui::ContentControl` — a `Control` holding a single content slot.

use super::*;
use crate::reactive::Subscription;

/// Callback shape used by generated `ContentPresenter` wiring.
#[doc(hidden)]
pub type ContentChangedHandler = Rc<dyn Fn(Option<Rc<dyn UIElementExt>>)>;

/// `elwindui::ui::ContentControl`(docs/design/runtime/ui_tree_design.md)— 単一の子(`content`)を持つ
/// `Control`の薄いラッパー。二重管理を避けるため、バックエンド非依存な合成 builtin としてここに直接手書きする。
/// Content is a single Visual child managed directly by this type.
#[elwindui_macros::class(inherits = crate::ui::Control)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
pub struct ContentControl {
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    template_presentation: Cell<bool>,
    content_changed_handlers: Rc<RefCell<Vec<ContentChangedHandler>>>,
}

#[elwindui_macros::class]
impl ContentControl {
    #[overrides]
    fn __prepare_template_presentation(&self) {
        self.__enable_template_presentation();
    }
    fn content(&self) -> Rc<dyn UIElementExt> {
        self.content
            .borrow()
            .clone()
            .expect("ContentControl has no content")
    }
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        if self
            .content
            .borrow()
            .as_ref()
            .is_some_and(|old| Rc::ptr_eq(old, &content))
        {
            return;
        }

        let old = self.content.borrow_mut().replace(content.clone());
        if let Some(old) = old {
            *old.as_ui_element().parent.borrow_mut() = None;
            if !self.template_presentation.get() {
                self.as_ui_element().visual_collection.remove(&old);
            }
            unmount_subtree(&old);
        }
        // `visual_collection.add` (below) is what routed-event bubbling (`dispatch_routed`) actually
        // relies on now — it walks `visual_parent`. Setting `content`'s Logical `parent` here too is
        // no longer needed for routing; it's kept purely so the Logical tree (a receptacle for a
        // future template/accessibility tree, see `UIElementCollection`'s own doc comment) stays
        // consistent — mirrors what `UIElementCollection::add` already does for `Layout::children`.
        if let Some(owner) = self.as_ui_element().visual_collection.owner_rc() {
            *content.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        if self.template_presentation.get() {
            self.__notify_content_changed(Some(content));
        } else {
            self.as_ui_element().visual_collection.add(content);
        }
    }
    /// Returns the optional logical content without the public `content()` panic contract.
    #[doc(hidden)]
    fn __content_opt(&self) -> Option<Rc<dyn UIElementExt>> {
        self.content.borrow().clone()
    }
    /// Switches a generated template-enabled descendant from direct to presenter-based display.
    #[doc(hidden)]
    fn __enable_template_presentation(&self) {
        if self.template_presentation.replace(true) {
            return;
        }
        if let Some(content) = self.content.borrow().clone() {
            self.as_ui_element().visual_collection.remove(&content);
        }
    }
    /// Subscribes a `ContentPresenter` to logical content replacement.
    #[doc(hidden)]
    fn __subscribe_content_changed(&self, handler: ContentChangedHandler) -> Subscription {
        self.content_changed_handlers
            .borrow_mut()
            .push(handler.clone());
        let weak_handlers = Rc::downgrade(&self.content_changed_handlers);
        Subscription::new(move || {
            let Some(handlers) = weak_handlers.upgrade() else {
                return;
            };
            handlers
                .borrow_mut()
                .retain(|candidate| !Rc::ptr_eq(candidate, &handler));
        })
    }
    fn __notify_content_changed(&self, content: Option<Rc<dyn UIElementExt>>) {
        let handlers = self.content_changed_handlers.borrow().clone();
        for handler in handlers {
            handler(content.clone());
        }
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
            template_presentation: Cell::new(false),
            content_changed_handlers: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_control_replaces_its_visual_child() {
        let first = TextBlock::new();
        let content_control = ContentControl::new();
        content_control.set_content(first.clone());
        let control: Rc<dyn UIElementExt> = content_control.clone();
        assert!(Rc::ptr_eq(
            &first.visual_parent().expect("initial visual parent"),
            &control
        ));

        let second = TextBlock::new();
        content_control.set_content(second.clone());
        assert!(first.visual_parent().is_none());
        assert!(Rc::ptr_eq(
            &second.visual_parent().expect("replacement visual parent"),
            &control
        ));
        assert_eq!(content_control.visual_children().len(), 1);
    }

    #[test]
    fn content_presenter_keeps_logical_parent_on_content_control() {
        let content_control = ContentControl::new();
        let first = TextBlock::new();
        content_control.set_content(first.clone());
        content_control.__enable_template_presentation();

        let presenter = ContentPresenter::new();
        ContentPresenter::__bind_templated_parent(&presenter, &content_control);

        let owner: Rc<dyn UIElementExt> = content_control.clone();
        let presenter_node: Rc<dyn UIElementExt> = presenter.clone();
        assert!(
            first
                .as_ui_element()
                .parent
                .borrow()
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|parent| Rc::ptr_eq(&parent, &owner))
        );
        assert!(
            first
                .visual_parent()
                .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
        );

        let second = TextBlock::new();
        content_control.set_content(second.clone());
        assert!(first.visual_parent().is_none());
        assert!(
            second
                .visual_parent()
                .is_some_and(|parent| Rc::ptr_eq(&parent, &presenter_node))
        );
    }

    #[test]
    fn empty_and_same_content_presentations_do_not_duplicate_visual_children() {
        let content_control = ContentControl::new();
        content_control.__enable_template_presentation();
        let presenter = ContentPresenter::new();
        ContentPresenter::__bind_templated_parent(&presenter, &content_control);
        assert!(presenter.visual_children().is_empty());

        let content = TextBlock::new();
        content_control.set_content(content.clone());
        content_control.set_content(content.clone());
        assert_eq!(presenter.visual_children().len(), 1);

        let weak_presenter = Rc::downgrade(&presenter);
        drop(presenter);
        assert!(weak_presenter.upgrade().is_none());
        assert!(content.visual_parent().is_none());
    }
}
