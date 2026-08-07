//! `NSPopUpButton` — a native, non-editable selection control. `NSPopUpButton` is itself an
//! `NSButton` subclass, so it reuses `inner/button.rs`'s `ButtonTarget` click trampoline directly
//! rather than duplicating it (same reasoning as `CheckBox`/`RadioButton`, `inner/check_box.rs`'s
//! own doc comment).

use crate::ffi::{AnyView, mtm};
use crate::inner::button::ButtonTarget;
use objc2::rc::Retained;
use objc2_app_kit::NSPopUpButton;
use objc2_foundation::NSString;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct InnerDropdown {
    pub(crate) handle: AnyView,
    ns: Retained<NSPopUpButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl InnerDropdown {
    pub(crate) fn new() -> Self {
        let ns = NSPopUpButton::new(mtm());
        let handle = AnyView::from(ns.clone());
        Self {
            handle,
            ns,
            target_storage: Rc::new(RefCell::new(None)),
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// Full rebuild rather than incremental diffing against the previous item list — unlike a
    /// `TabView` tab's own content subtree, a `DropdownItem` carries no live native editing state
    /// worth preserving across an update (`docs/status/nativecontrol_status.md` §2, `Dropdown`).
    pub(crate) fn rebuild_items(&self, texts: &[String]) {
        self.ns.removeAllItems();
        for text in texts {
            self.ns.addItemWithTitle(&NSString::from_str(text));
        }
    }

    pub(crate) fn set_selected_index(&self, index: usize) {
        self.ns.selectItemAtIndex(index as isize);
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(usize)>) {
        let ns = self.ns.clone();
        let target = ButtonTarget::attach(
            &self.ns,
            Box::new(move || {
                let index = ns.indexOfSelectedItem();
                if index >= 0 {
                    callback(index as usize);
                }
            }),
        );
        *self.target_storage.borrow_mut() = Some(target);
    }
}
