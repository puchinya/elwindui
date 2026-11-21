//! The AppKit peer behind `elwindui::ui::Window` (a Core-owned Component, Issue #313).

use super::MenuBar;
use crate::inner::InnerWindow;
use elwindui_core::base::Rect;
use elwindui_core::ui::native::WindowPeer;
use elwindui_core::ui::{MenuBarExt, UIElementExt, WindowExt};
use std::rc::{Rc, Weak};

/// [`WindowPeer`] over an `NSWindow`. Created by [`super::AppKitPeerFactory`].
///
/// `InnerWindow` keeps only the owner's `Weak` and hands it to `crate::app::retain_window` on the
/// first `show()` (Issue #254). The `windowShouldClose:`/`windowWillClose:` split stays in
/// `InnerWindow`. Not compiled or run on a macOS host yet: verification is tracked by #316.
pub(crate) struct AppKitWindowPeer {
    inner: InnerWindow,
}

impl AppKitWindowPeer {
    pub(crate) fn new(owner: Weak<dyn WindowExt>) -> Self {
        Self {
            inner: InnerWindow::new(owner),
        }
    }
}

impl WindowPeer for AppKitWindowPeer {
    fn show(&self) {
        self.inner.show();
    }
    fn hide(&self) {
        self.inner.hide();
    }
    fn close(&self) {
        self.inner.close();
    }
    fn activate(&self) {
        self.inner.activate();
    }
    fn close_active_popup(&self) {
        self.inner.close_active_popup();
    }
    fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }
    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBarExt>) {
        let menu_bar = menu_bar
            .as_any()
            .downcast_ref::<MenuBar>()
            .expect("Window::set_menu_bar: menu_bar must be this backend's MenuBar");
        self.inner.set_menu_bar(&menu_bar.inner);
    }
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.inner.set_content(content);
    }
    fn set_transparent(&self, transparent: bool) {
        self.inner.set_transparent(transparent);
    }
    fn set_always_on_top(&self, always_on_top: bool) {
        self.inner.set_always_on_top(always_on_top);
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
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        self.inner.set_close_request_handler(handler);
    }
    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        self.inner.set_bounds_changed_handler(handler);
    }
    /// Clears both Core handlers so no native close or bounds event reaches Core afterwards.
    fn dispose(&self) {
        self.inner.set_close_request_handler(None);
        self.inner.set_bounds_changed_handler(None);
    }
}
