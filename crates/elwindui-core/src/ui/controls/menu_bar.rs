//! `elwindui::ui::MenuBar` — the native application menu bar.

use super::*;

#[elwindui_macros::class(trait_only)]
#[content(items)]
#[prop(items: crate::ui::ListExt<dyn crate::ui::MenuBarItemExt>)]
pub trait MenuBar {
    fn add_item(&self, item: &dyn MenuBarItemExt);
    fn remove_item(&self, item: &dyn MenuBarItemExt);
    /// See `Menu::items`'s own doc comment — same rationale, one level up (`MenuBar`'s children are
    /// `MenuBarItem`s rather than `MenuItem`s).
    fn items(&self) -> &dyn ListExt<dyn MenuBarItemExt>;
}
