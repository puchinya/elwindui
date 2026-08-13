//! `elwindui::ui::Window` — the top-level native window.

use super::*;

/// `Window`'s own class trait (docs/design/runtime/ui_tree_design.md) — also the `component X inherits
/// Window` (host-composition) bare name every backend's own `WindowImpl` implements.
/// `set_menu_bar`'s `Rc<dyn MenuBar>` follows the same trait-object-argument convention as
/// `Menu`/`MenuBar`/`MenuBarItem` just above (see this module's own doc comment on that group) —
/// `impl Window for WindowImpl` downcasts it back to its own concrete `MenuBarImpl` internally.
#[elwindui_macros::class(trait_only)]
#[prop(title: String)]
#[prop(menu_bar: Option<std::rc::Rc<dyn crate::ui::MenuBarExt>>)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(onetime, left: Option<f32>)]
#[prop(onetime, top: Option<f32>)]
#[prop(onetime, width: Option<f32>)]
#[prop(onetime, height: Option<f32>)]
pub trait Window {
    fn set_title(&self, title: &str);
    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBarExt>);
    fn set_content(&self, content: Rc<dyn UIElementExt>);
    fn show(&self);
    fn left(&self) -> f32;
    fn set_left(&self, left: f32);
    fn top(&self) -> f32;
    fn set_top(&self, top: f32);
    fn width(&self) -> f32;
    fn set_width(&self, width: f32);
    fn height(&self) -> f32;
    fn set_height(&self, height: f32);
}
