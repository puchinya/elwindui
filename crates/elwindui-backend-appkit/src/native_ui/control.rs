//! The native-peer side of `elwindui_core::ui::NativeControl` (Issue #313).
//!
//! `NativeControl` itself is a Core-owned Component now. This module provides the
//! [`NativePeer`] that wraps one `NSView`, shared by every native leaf in this backend: the
//! background/tooltip/text-style/measure plumbing is identical for all of them.

use crate::AnyView;
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::native::NativePeer;
use std::any::Any;
use std::rc::Rc;

/// A [`NativePeer`] over one `NSView`. Controls not yet migrated to Core-owned Components create
/// one eagerly and pass it to `NativeControl::construct(Some(..))`.
pub(crate) struct HandlePeer {
    handle: AnyView,
}

impl HandlePeer {
    pub(crate) fn new(handle: AnyView) -> Self {
        Self { handle }
    }

    pub(crate) fn any_view(&self) -> &AnyView {
        &self.handle
    }
}

/// Wraps `handle` as the eagerly created peer of a native control's `NativeControl` base.
pub(crate) fn handle_peer(handle: AnyView) -> Option<Rc<dyn NativePeer>> {
    Some(Rc::new(HandlePeer::new(handle)))
}

impl NativePeer for HandlePeer {
    fn handle(&self) -> &dyn Any {
        &self.handle
    }
    fn measure(&self, available: Size) -> Size {
        self.handle.measure(available)
    }
    fn apply_background(&self, background: Option<&Brush>) -> bool {
        self.handle.apply_background(background);
        true
    }
    fn set_tooltip(&self, tooltip: Option<&str>) {
        self.handle.set_tooltip(tooltip);
    }
    fn supports_text_style(&self) -> bool {
        self.handle.supports_text_style()
    }
    fn apply_text_style(&self, style: &CascadedTextStyle) -> bool {
        self.handle.apply_text_style(style);
        true
    }
}
