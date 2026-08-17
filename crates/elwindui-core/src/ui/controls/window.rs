//! `elwindui::ui::Window` — the top-level native window.

use super::*;

/// `Window`'s own class trait (docs/design/runtime/ui_tree_design.md) — also the `component X inherits
/// Window` (host-composition) bare name every backend's own `WindowImpl` implements.
/// `set_menu_bar`'s `Rc<dyn MenuBar>` follows the same trait-object-argument convention as
/// `Menu`/`MenuBar`/`MenuBarItem` just above (see this module's own doc comment on that group) —
/// `impl Window for WindowImpl` downcasts it back to its own concrete `MenuBarImpl` internally.
///
/// `show`/`hide`/`close` are `#[overridable]` (Issue #128 restored normal `#[overridable]`/
/// `#[overrides]` propagation across the `trait_only` (this trait) -> `struct_only` (each backend's
/// concrete `Window`) -> ordinary (a generated host-composition component) chain — previously (CI-8
/// of #80) that propagation didn't work, and `generate_view`'s host-composition codegen worked
/// around it with a plain inherent `show`/`hide`/`close` reached via UFCS instead of `#[overrides]`;
/// #128 removed that workaround). A host-composition generated component now emits ordinary
/// `#[overrides]` methods calling `self.base.show()`/etc. directly, exactly like any other ancestor
/// call at the `#[class]` layer.
#[elwindui_macros::class(trait_only)]
#[prop(title: String)]
#[prop(menu_bar: std::rc::Rc<dyn crate::ui::MenuBarExt>)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(transparent: bool)]
#[prop(always_on_top: bool)]
#[prop(onetime, left: Option<f32>)]
#[prop(onetime, top: Option<f32>)]
#[prop(onetime, width: Option<f32>)]
#[prop(onetime, height: Option<f32>)]
pub trait Window {
    fn set_title(&self, title: &str);
    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBarExt>);
    fn content_element(&self) -> Option<Rc<dyn UIElementExt>>;
    fn set_content(&self, content: Rc<dyn UIElementExt>);
    /// Enables or disables an alpha-capable client surface.
    ///
    /// Transparent pixels reveal windows behind this one; native window decorations are unchanged.
    fn set_transparent(&self, transparent: bool);
    /// Pins this window above normal application windows, or restores normal Z-order when false.
    fn set_always_on_top(&self, always_on_top: bool);
    #[overridable]
    fn show(&self);
    /// Visibility only: the mounted subtree, Environment subscriptions, and state all remain alive.
    /// A subsequent `show()` makes the window visible again without remounting/rebuilding
    /// (docs/specs/dsl_spec.md's Window contract; CI-8 of #80).
    #[overridable]
    fn hide(&self);
    /// Ends this Window's mount lifetime: releases the native window and (for a host-composition
    /// generated component's own `#[overrides]` — see this trait's own doc comment) its own
    /// Environment subscriptions. See docs/design/runtime/component_lifecycle_design.md §4g for
    /// exactly what today's implementation does and does not clean up.
    #[overridable]
    fn close(&self);
    fn left(&self) -> f32;
    fn set_left(&self, left: f32);
    fn top(&self) -> f32;
    fn set_top(&self, top: f32);
    fn width(&self) -> f32;
    fn set_width(&self, width: f32);
    fn height(&self) -> f32;
    fn set_height(&self, height: f32);
}
