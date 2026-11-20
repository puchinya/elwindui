//! The WinUI 3 peer behind `elwindui::ui::Button` (a Core-owned Component, Issue #313).

use super::control::HandlePeer;
use crate::inner::InnerButton;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::ButtonRole;
use elwindui_core::ui::native::{ButtonPeer, NativePeer};
use std::any::Any;

/// [`ButtonPeer`] over a XAML `Button`. Created by [`super::WinUi3PeerFactory`].
pub(crate) struct WinUi3ButtonPeer {
    base: HandlePeer,
    inner: InnerButton,
}

impl WinUi3ButtonPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerButton::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for WinUi3ButtonPeer {
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
    fn dispose(&self) {
        self.inner.dispose();
    }
}

impl ButtonPeer for WinUi3ButtonPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_role(&self, role: ButtonRole) {
        self.inner.set_role(role);
    }
    fn set_is_default(&self, is_default: bool) {
        self.inner.set_is_default(is_default);
    }
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>) {
        self.inner.set_on_click(callback);
    }
}
