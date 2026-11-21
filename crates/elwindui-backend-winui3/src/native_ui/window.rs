//! The WinUI3 peer behind `elwindui::ui::Window` (a Core-owned Component, Issue #313).

use super::MenuBar;
use crate::inner::InnerWindow;
use elwindui_core::base::Rect;
use elwindui_core::ui::native::WindowPeer;
use elwindui_core::ui::{MenuBarExt, UIElementExt, WindowExt};
use std::rc::{Rc, Weak};

/// [`WindowPeer`] over a XAML `Window`/`AppWindow`. Created by [`super::WinUi3PeerFactory`].
///
/// `InnerWindow` keeps only the owner's `Weak` and hands it to `crate::app::retain_window` on the
/// first `show()`, so the application registry, not this peer, is the owner's strong lifetime
/// authority (Issue #254). The `framework_initiated_close` reentrancy guard stays in `InnerWindow`.
pub(crate) struct WinUi3WindowPeer {
    inner: InnerWindow,
}

impl WinUi3WindowPeer {
    pub(crate) fn new(owner: Weak<dyn WindowExt>) -> Self {
        Self {
            inner: InnerWindow::new(owner),
        }
    }
}

impl WindowPeer for WinUi3WindowPeer {
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
    /// One `AppWindow.MoveAndResize` instead of four separate edge updates.
    fn set_bounds(&self, bounds: Rect) {
        self.inner.set_bounds(bounds);
    }
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        self.inner.set_close_request_handler(handler);
    }
    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        self.inner.set_bounds_changed_handler(handler);
    }
    fn configure_floating_title(&self) -> Result<(), String> {
        self.inner
            .configure_floating_title()
            .map_err(|error| error.to_string())
    }
    /// Clears both Core handlers so no native close or bounds event reaches Core afterwards. The
    /// native registrations themselves are owned by `InnerWindow`'s `UiCallbackRegistryOwner` and
    /// are released when the peer drops.
    fn dispose(&self) {
        self.inner.set_close_request_handler(None);
        self.inner.set_bounds_changed_handler(None);
    }
}
