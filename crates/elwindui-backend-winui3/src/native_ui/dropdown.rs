//! The WinUI 3 peer behind `elwindui::ui::Dropdown` (a Core-owned Component, Issue #313).

use super::control::HandlePeer;
use crate::inner::InnerDropdown;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::native::{DropdownPeer, NativePeer};
use std::any::Any;

/// [`DropdownPeer`] over a XAML `ComboBox`. Created by [`super::WinUi3PeerFactory`].
pub(crate) struct WinUi3DropdownPeer {
    base: HandlePeer,
    inner: InnerDropdown,
}

impl WinUi3DropdownPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerDropdown::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for WinUi3DropdownPeer {
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
        self.inner.set_on_change(None);
    }
}

impl DropdownPeer for WinUi3DropdownPeer {
    fn set_items(&self, texts: &[String]) {
        self.inner.rebuild_items(texts);
    }
    fn set_selected_index(&self, index: usize) {
        self.inner.set_selected_index(index);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(usize)>>) {
        self.inner.set_on_change(callback);
    }
}
