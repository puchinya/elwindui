//! The AppKit peer behind `elwindui::ui::ScrollView` (a Core-owned Component, Issue #313).

use super::control::HandlePeer;
use crate::inner::InnerScrollView;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::UIElementExt;
use elwindui_core::ui::native::{NativePeer, ScrollViewPeer};
use std::any::Any;
use std::rc::Rc;

/// [`ScrollViewPeer`] over an `NSScrollView` whose document view is a nested `TreeHost`.
/// Cross-checked only; macOS verification is tracked by #316.
pub(crate) struct AppKitScrollViewPeer {
    base: HandlePeer,
    inner: InnerScrollView,
}

impl AppKitScrollViewPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerScrollView::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for AppKitScrollViewPeer {
    fn handle(&self) -> &dyn Any {
        self.base.handle()
    }
    fn measure(&self, available: Size) -> Size {
        self.base.measure(available)
    }
    fn apply_background(&self, background: Option<&Brush>) -> bool {
        self.base.apply_background(background)
    }
    fn set_tooltip(&self, tooltip: Option<&str>) {
        self.base.set_tooltip(tooltip);
    }
    fn supports_text_style(&self) -> bool {
        self.base.supports_text_style()
    }
    fn apply_text_style(&self, style: &CascadedTextStyle) -> bool {
        self.base.apply_text_style(style)
    }
}

impl ScrollViewPeer for AppKitScrollViewPeer {
    fn set_content(&self, content: Rc<dyn UIElementExt>, owner: Option<Rc<dyn UIElementExt>>) {
        // The AppKit nested host has no logical-parent link or retirement protocol yet
        // (unchanged by #313); `owner` is unused here.
        let _ = owner;
        self.inner.set_content(content);
    }
    fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        self.inner.set_horizontal_scroll_enabled(enabled);
    }
    fn set_vertical_scroll_enabled(&self, enabled: bool) {
        self.inner.set_vertical_scroll_enabled(enabled);
    }
    /// The AppKit nested host has no hosted-content retirement yet (unchanged by #313).
    fn unmount_content(&self) {}
    fn as_any(&self) -> &dyn Any {
        self
    }
}
