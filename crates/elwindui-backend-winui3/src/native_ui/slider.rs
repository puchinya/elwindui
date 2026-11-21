//! The WinUI 3 peers behind `elwindui::ui::Slider` (Core-owned Components, Issue #313).

use super::control::HandlePeer;
use crate::inner::InnerSlider;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::native::{NativePeer, SliderPeer};
use std::any::Any;

/// [`SliderPeer`] over the native slider. Created by [`super::WinUi3PeerFactory`].
pub(crate) struct WinUi3SliderPeer {
    base: HandlePeer,
    inner: InnerSlider,
}

impl WinUi3SliderPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerSlider::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for WinUi3SliderPeer {
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
        self.inner.set_on_change(Box::new(|_| {}));
    }
}

impl SliderPeer for WinUi3SliderPeer {
    fn set_value(&self, value: f32) {
        self.inner.set_value(value);
    }
    fn set_min(&self, min: f32) {
        self.inner.set_min(min);
    }
    fn set_max(&self, max: f32) {
        self.inner.set_max(max);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(f32)>>) {
        self.inner
            .set_on_change(callback.unwrap_or_else(|| Box::new(|_| {})));
    }
}
