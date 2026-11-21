//! The AppKit peers behind `elwindui::ui::CheckBox` (Core-owned Components, Issue #313).

use super::control::HandlePeer;
use crate::inner::InnerCheckBox;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::CheckState;
use elwindui_core::ui::native::{CheckBoxPeer, NativePeer};
use std::any::Any;

/// [`CheckBoxPeer`] over the native check box. Created by [`super::AppKitPeerFactory`].
pub(crate) struct AppKitCheckBoxPeer {
    base: HandlePeer,
    inner: InnerCheckBox,
}

impl AppKitCheckBoxPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerCheckBox::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for AppKitCheckBoxPeer {
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
    fn refresh_text_style_on_measure(&self) -> bool {
        true
    }
    /// Replaces every Core callback with a no-op, so no native event reaches Core afterwards.
    fn dispose(&self) {
        self.inner.set_on_change(Box::new(|_| {}));
    }
}

impl CheckBoxPeer for AppKitCheckBoxPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_checked(&self, checked: CheckState) {
        self.inner.set_checked(checked);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(CheckState)>>) {
        self.inner
            .set_on_change(callback.unwrap_or_else(|| Box::new(|_| {})));
    }
}
