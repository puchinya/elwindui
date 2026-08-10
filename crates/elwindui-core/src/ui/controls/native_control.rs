//! The abstract base every real OS-toolkit-backed leaf inherits — the marker that tells the tree a subtree is opaque native content, plus the `background`/`#[text_style]` props shared by all of them.

use super::*;

/// `Button`/`TextArea`/`TabView` — the only `UIElement`s with a real backend handle. Always a leaf as
/// far as this tree is concerned: whatever lives beneath it in its own backend-managed hierarchy
/// (e.g. `TabView`'s tab-switching) is opaque here. A pure marker trait (`trait_only` — no
/// `NativeControlImpl`/`<H>` here at all): measuring/placing a native handle is entirely
/// backend-specific (e.g. AppKit's `NSView.fittingSize()`/`setFrame:`), so instead of `elwindui-core`
/// owning a shared generic `NativeControlImpl<H>` that every backend's `H` would need to plug into,
/// each backend defines its own concrete, non-generic implementor (e.g.
/// `elwindui-backend-appkit::NativeControlImpl { handle: AnyView, .. }`, and its winui3 equivalent)
/// that `TextArea`/`Button`/`TabView` (that backend's own leaf widgets) inherit from — the same way
/// `VerticalLayout`/`Control`/`Grid` above each write their own `measure_override`, not through any
/// shared "MeasureNode" abstraction. `collect_render_items<H>` downcasts a leaf's
/// `try_as_native_control()` result directly to `H` (see that trait method's own doc comment) — no
/// wrapper struct type needs to be nameable from `elwindui-core` for this to work.
#[elwindui_macros::class(trait_only, inherits = crate::ui::UIElement, abstract_class)]
#[text_style]
#[prop(background: Option<crate::graphics::Brush>)]
#[prop(tooltip: Option<String>)]
pub trait NativeControl {
    /// Sets an explicit native-control background, or removes it (`None`) so the backend control
    /// theme can supply it again — matching `#[prop(background: Option<Brush>)]`'s own declared
    /// type exactly, unlike the virtual-builtin/native-leaf signature mismatch this used to have
    /// (`Layout::set_background` always took `Option<Brush>`; this took a bare `Brush`, relying on
    /// the separate `clear_background` below for the `None` case). `clear_background` stays a
    /// distinct method rather than folding away into `set_background(None)`'s body — `emit_resync`'s
    /// generic `clear_<name>()` convention calls it directly, unconditionally, wherever a DSL value
    /// transitions from `Some` to not-written across a re-render.
    fn set_background(&self, background: Option<Brush>);

    /// Removes an explicit background so the backend control theme can supply it again.
    fn clear_background(&self);

    /// Sets the hover-delayed explanatory text shown for this control.
    ///
    /// Declared here rather than on any individual leaf because `docs/specs/ui_spec.md#23-common-properties`
    /// specifies `tooltip` as an attribute any control may carry, and both toolkits
    /// expose it on their universal view type (AppKit `NSView.toolTip`, WinUI3
    /// `ToolTipService.ToolTip`) — so every native leaf inherits one working implementation from
    /// its backend's own `NativeControl` struct, exactly as `background` above already does.
    ///
    /// Passing an empty string removes the tooltip. There is no `clear_tooltip` counterpart:
    /// `elwindui-codegen` only emits `clear_<name>()` for properties that can carry a
    /// `theme!(..)` value and resolve to `PlatformDefault`, and a tooltip is plain text with no
    /// theme token — the same reason `TextBox::set_placeholder` has no `clear_placeholder`.
    fn set_tooltip(&self, tooltip: &str);
}
