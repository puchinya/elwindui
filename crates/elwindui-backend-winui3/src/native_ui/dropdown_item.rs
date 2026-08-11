//! `elwindui::ui::DropdownItem` — the `DropdownItemExt` implementation. No native view of its own;
//! `native_ui::Dropdown` rebuilds its `ComboBox` item list directly from each item's own `text()`
//! (see that file's own doc comment).

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

    /// Backend-local accessor, not part of `DropdownItemExt` — mirrors
    /// `elwindui_backend_appkit::native_ui::DropdownItem::text`'s own doc comment exactly.
    #[inherent]
    pub(crate) fn text(&self) -> String {
        self.inner.text()
    }
}
