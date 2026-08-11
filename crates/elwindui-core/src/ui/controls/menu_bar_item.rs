//! `elwindui::ui::MenuBarItem` — one top-level entry in a `MenuBar`.

use super::*;

#[elwindui_macros::class(trait_only)]
#[prop(text: String)]
#[content(submenu)]
#[prop(submenu: std::rc::Rc<dyn crate::ui::MenuExt>)]
pub trait MenuBarItem {
    fn set_text(&self, text: &str);
    fn set_submenu(&self, submenu: Rc<dyn MenuExt>);
}
