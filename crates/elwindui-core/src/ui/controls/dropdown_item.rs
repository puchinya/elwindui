//! `elwindui::ui::DropdownItem` — one entry in a `Dropdown`.

use super::*;

/// `elwindui::ui::DropdownItem`, one entry of a [`Dropdown`]. Declared once here as a Core-owned
/// Component (Issue #313). It has no native view of its own: the `Dropdown` pushes every item's
/// `text` to its peer whenever its item list changes.
#[elwindui_macros::component(sealed)]
#[prop(text: String)]
pub struct DropdownItem {
    text: RefCell<String>,
}

#[elwindui_macros::component]
impl DropdownItem {
    fn set_text(&self, text: &str) {
        *self.as_ui_element().text.borrow_mut() = text.to_string();
    }

    fn construct() -> Self {
        Self {
            text: RefCell::new(String::new()),
        }
    }
}

impl DropdownItem {
    /// The entry's caption.
    pub fn text(&self) -> String {
        self.text.borrow().clone()
    }
}
