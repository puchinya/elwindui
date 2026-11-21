//! The WinUI 3 peers behind `elwindui::ui::RadioButton` (Core-owned Components, Issue #313).

use super::control::HandlePeer;
use crate::inner::InnerRadioButton;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::native::{NativePeer, RadioButtonPeer};
use std::any::Any;

/// [`RadioButtonPeer`] over the native radio button. Grouping is Core's. Created by [`super::WinUi3PeerFactory`].
pub(crate) struct WinUi3RadioButtonPeer {
    base: HandlePeer,
    inner: InnerRadioButton,
}

impl WinUi3RadioButtonPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerRadioButton::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for WinUi3RadioButtonPeer {
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
    /// Replaces every Core callback with a no-op, so no native event reaches Core afterwards.
    fn dispose(&self) {
        self.inner.set_on_click(Box::new(|| {}));
    }
}

impl RadioButtonPeer for WinUi3RadioButtonPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_checked(&self, checked: bool) {
        self.inner.set_checked(checked);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>) {
        self.inner
            .set_on_click(callback.unwrap_or_else(|| Box::new(|| {})));
    }
}
