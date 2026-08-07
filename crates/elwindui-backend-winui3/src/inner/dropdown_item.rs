//! Just the text storage for a `DropdownItem` — see
//! `elwindui_backend_appkit::inner::InnerDropdownItem`'s own doc comment for why it carries no
//! native handle of its own.

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
