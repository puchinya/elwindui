//! `elwindui::ui::Window` — the `WindowExt` implementation.

use super::MenuBar;
use crate::inner::InnerWindow;
use std::cell::RefCell;
use std::rc::Rc;

/// `component X inherits Window` ("host composition", docs/design/runtime/ui_tree_design.md) is what
/// actually inherits this — hence `struct_only`'s target being `elwindui_core::ui::WindowExt`
/// itself. `Window` is deliberately *not* a `UIElement` (no `inherits` here at all) — like AppKit's
/// `Window`, it's a separate top-level concept, not embeddable as a child.
#[elwindui_macros::class(struct_only = elwindui_core::ui::WindowExt)]
pub struct Window {
    inner: InnerWindow,
    content: RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>,
}

#[elwindui_macros::class]
impl Window {
    // The bare (not `Rc`-wrapped) value `#[class]`'s auto-generated `new` wraps — this is also what
    // lets a `component X inherits Window` (host composition) embed a real `Window` directly as its
    // own `base` field.
    fn construct() -> Self {
        Self {
            inner: InnerWindow::new(),
            content: RefCell::new(None),
        }
    }

    fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    fn set_menu_bar(&self, menu_bar: Rc<dyn elwindui_core::ui::MenuBarExt>) {
        let menu_bar = menu_bar
            .as_any()
            .downcast_ref::<MenuBar>()
            .expect("WindowExt::set_menu_bar: menu_bar must be this backend's MenuBar");
        self.inner.set_menu_bar(&menu_bar.inner);
    }

    fn content_element(&self) -> Option<Rc<dyn elwindui_core::ui::UIElementExt>> {
        self.content.borrow().clone()
    }

    fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.inner.set_content(content.clone());
        *self.content.borrow_mut() = Some(content);
    }

    fn set_transparent(&self, transparent: bool) {
        self.inner.set_transparent(transparent);
    }

    fn set_always_on_top(&self, always_on_top: bool) {
        self.inner.set_always_on_top(always_on_top);
    }

    fn show(&self) {
        self.inner.show();
    }

    fn hide(&self) {
        self.inner.hide();
    }

    fn close(&self) {
        self.inner.close();
    }

    fn left(&self) -> f32 {
        self.inner.left()
    }

    fn set_left(&self, left: f32) {
        self.inner.set_left(left);
    }

    fn top(&self) -> f32 {
        self.inner.top()
    }

    fn set_top(&self, top: f32) {
        self.inner.set_top(top);
    }

    fn width(&self) -> f32 {
        self.inner.width()
    }

    fn set_width(&self, width: f32) {
        self.inner.set_width(width);
    }

    fn height(&self) -> f32 {
        self.inner.height()
    }

    fn set_height(&self, height: f32) {
        self.inner.set_height(height);
    }
}
