//! Just the text storage for a `DropdownItem` — no native handle of its own. `Dropdown` reads
//! each item's `text()` when rebuilding its own native item list (`inner/dropdown.rs`), the same
//! way `Dropdown`'s AppKit peer never needs an independent `AnyView` per item.

use std::cell::RefCell;

pub(crate) struct InnerDropdownItem {
    text: RefCell<String>,
}

impl InnerDropdownItem {
    pub(crate) fn new() -> Self {
        Self {
            text: RefCell::new(String::new()),
        }
    }

    pub(crate) fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
    }

    pub(crate) fn text(&self) -> String {
        self.text.borrow().clone()
    }
}
