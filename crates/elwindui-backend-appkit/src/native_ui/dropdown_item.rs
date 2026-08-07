//! `builtin::DropdownItem` — the `DropdownItemExt` implementation. No native view of its own;
//! `native_ui::Dropdown` rebuilds its `NSPopUpButton` item list directly from each item's own
//! `text()` (see that file's own doc comment).

use crate::inner::InnerDropdownItem;

#[elwindui_macros::class(struct_only = elwindui_core::ui::DropdownItemExt)]
pub struct DropdownItem {
    inner: InnerDropdownItem,
}

#[elwindui_macros::class]
impl DropdownItem {
    fn construct() -> Self {
        Self {
            inner: InnerDropdownItem::new(),
        }
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    /// Backend-local accessor, not part of `DropdownItemExt` — `native_ui::Dropdown` downcasts
    /// each item to this concrete type to read it back when rebuilding its own native item list,
    /// mirroring `native_ui::menu`'s own `downcast_menu_bar_item` pattern.
    #[inherent]
    pub(crate) fn text(&self) -> String {
        self.inner.text()
    }
}
