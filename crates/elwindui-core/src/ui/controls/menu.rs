//! `builtin::Menu` — a menu holding `MenuItem`s.

use super::*;

#[elwindui_macros::class(trait_only)]
#[content(items)]
#[prop(items: crate::ui::ListExt<dyn crate::ui::MenuItemExt>)]
pub trait Menu {
    fn add_item(&self, item: &dyn MenuItemExt);
    fn remove_item(&self, item: &dyn MenuItemExt);
    /// A live handle onto the same backing collection `add_item`/`remove_item` mutate — added
    /// alongside them (not a replacement) so the DSL's `#[content(items)]` mechanism
    /// (`elwindui-core::ui`'s `Menu`, `docs/specs/ui_spec.md#menu`) can populate `Menu`'s
    /// nested `MenuItem { .. }` children through the same generic `ListExt`-typed
    /// content-field path every other multi-child builtin (`VerticalLayout`/`Grid`/`TabView`/...)
    /// already uses, instead of `elwindui-codegen` needing a `Menu`-specific construction branch.
    /// A borrow (mirroring `Layout::children`/`Control::children`), not an owned `Rc` — no backend
    /// needs to hand out an independently-owned handle here.
    fn items(&self) -> &dyn ListExt<dyn MenuItemExt>;
}
