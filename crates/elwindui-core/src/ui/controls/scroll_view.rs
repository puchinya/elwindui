//! `elwindui::ui::ScrollView` — hosts an elwindui subtree inside a native scrolling container.

use super::*;

/// Hosts arbitrary elwindui content inside a native scrolling container —
/// `ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content`
/// (`docs/status/control_status.md`). Unlike every other `NativeControl` leaf so
/// far (`Button`/`TextArea`/`TextBox`/`PasswordBox`/`TabView`, all self-contained native widgets),
/// `ScrollView`'s own content is a full elwindui subtree with its own layout/paint/hit-test/focus —
/// each backend's `ElwinduiContentRoot` is a second, nested instance of that same backend's own
/// "reflect an `Rc<dyn UIElement>` into real native views" host (AppKit's `TreeHostView`, WinUI3's
/// `TreeHostPanel`), the same pattern `TabView`'s own per-tab content host already establishes —
/// not a one-off special case. The one genuinely new piece is that this nested host's own Measure
/// must run *unconstrained* on the scrolling axis/axes (so the content reports/gets arranged at its
/// true natural size, letting the native container's own scroll physics do the rest) rather than
/// "fill exactly this frame", which is every other host's contract today.
///
/// Scroll-position get/set and a `scroll_changed` event are deliberately not part of this trait —
/// same "ship the minimal, honestly-scoped surface, document the gap" call as `TextBox`/
/// `PasswordBox`'s deferred selection APIs; a real, understood follow-up, not a silent omission.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(horizontal_scroll_enabled: Option<bool>)]
#[prop(vertical_scroll_enabled: Option<bool>)]
pub trait ScrollView {
    fn set_content(&self, content: Rc<dyn UIElementExt>);
    fn set_horizontal_scroll_enabled(&self, enabled: bool);
    fn set_vertical_scroll_enabled(&self, enabled: bool);
}
