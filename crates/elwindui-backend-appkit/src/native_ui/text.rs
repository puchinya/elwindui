//! The AppKit peers behind `elwindui::ui::{TextBox, TextArea, PasswordBox}` (Core-owned Components, Issue #313).

use super::control::HandlePeer;
use crate::inner::{InnerPasswordBox, InnerTextArea, InnerTextBox};
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::TextAlignment;
use elwindui_core::ui::native::{NativePeer, PasswordBoxPeer, TextAreaPeer, TextBoxPeer};
use std::any::Any;

/// [`TextBoxPeer`] over the native single-line text field. Created by [`super::AppKitPeerFactory`].
pub(crate) struct AppKitTextBoxPeer {
    base: HandlePeer,
    inner: InnerTextBox,
}

impl AppKitTextBoxPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerTextBox::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for AppKitTextBoxPeer {
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
        self.inner.set_on_submit(Box::new(|| {}));
    }
}

impl TextBoxPeer for AppKitTextBoxPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_placeholder(&self, text: &str) {
        self.inner.set_placeholder(text);
    }
    fn set_read_only(&self, read_only: bool) {
        self.inner.set_read_only(read_only);
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.inner.set_max_length(max_length);
    }
    fn set_text_alignment(&self, alignment: TextAlignment) {
        self.inner.set_text_alignment(alignment);
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>) {
        self.inner
            .set_on_change(callback.unwrap_or_else(|| Box::new(|_| {})));
    }
    fn set_on_submit(&self, callback: Option<Box<dyn Fn()>>) {
        self.inner
            .set_on_submit(callback.unwrap_or_else(|| Box::new(|| {})));
    }
}

/// [`TextAreaPeer`] over the native multi-line text view. Created by [`super::AppKitPeerFactory`].
pub(crate) struct AppKitTextAreaPeer {
    base: HandlePeer,
    inner: InnerTextArea,
}

impl AppKitTextAreaPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerTextArea::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for AppKitTextAreaPeer {
    fn handle(&self) -> &dyn Any {
        self.base.handle()
    }
    fn measure(&self, available: Size) -> Size {
        self.inner.refresh_default_size();
        self.inner.measure(available)
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

impl TextAreaPeer for AppKitTextAreaPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>) {
        self.inner
            .set_on_change(callback.unwrap_or_else(|| Box::new(|_| {})));
    }
}

/// [`PasswordBoxPeer`] over the native secure field. Never logs the value. Created by [`super::AppKitPeerFactory`].
pub(crate) struct AppKitPasswordBoxPeer {
    base: HandlePeer,
    inner: InnerPasswordBox,
}

impl AppKitPasswordBoxPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerPasswordBox::new();
        Self {
            base: HandlePeer::new(inner.handle()),
            inner,
        }
    }
}

impl NativePeer for AppKitPasswordBoxPeer {
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

impl PasswordBoxPeer for AppKitPasswordBoxPeer {
    fn set_password(&self, password: &str) {
        self.inner.set_password(password);
    }
    fn set_placeholder(&self, text: &str) {
        self.inner.set_placeholder(text);
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.inner.set_max_length(max_length);
    }
    fn set_reveal_enabled(&self, enabled: bool) {
        self.inner.set_reveal_enabled(enabled);
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>) {
        self.inner
            .set_on_change(callback.unwrap_or_else(|| Box::new(|_| {})));
    }
}
