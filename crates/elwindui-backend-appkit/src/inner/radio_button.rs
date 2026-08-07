//! `NSButton` set to `NSButtonType::Radio` — the same widget `InnerButton` wraps, in a different
//! button type, reusing its `ButtonTarget` click trampoline directly rather than duplicating it.
//! Group exclusivity itself lives one layer up, in `native_ui::RadioButton` — this type only knows
//! how to display and report its own two-state value.

use crate::ffi::{AnyView, mtm};
use crate::inner::button::ButtonTarget;
use objc2::rc::Retained;
use objc2_app_kit::{NSButton, NSButtonType, NSControlStateValueOff, NSControlStateValueOn};
use objc2_foundation::NSString;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct InnerRadioButton {
    pub(crate) handle: AnyView,
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl InnerRadioButton {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(""), None, None, m)
        };
        ns.setButtonType(NSButtonType::Radio);
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

    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    pub(crate) fn set_checked(&self, checked: bool) {
        self.ns.setState(if checked {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    /// The raw click signal, reporting only "the user just clicked this button" — unlike
    /// `InnerCheckBox::set_on_change`, it does not itself read back `checked`: a native radio
    /// click always lands on the checked state (there is no way to natively click a radio button
    /// *off*), and `native_ui::RadioButton` needs to run its own group-exclusivity logic before
    /// deciding what the reported new value is.
    pub(crate) fn set_on_click(&self, callback: Box<dyn Fn()>) {
        let target = ButtonTarget::attach(&self.ns, callback);
        *self.target_storage.borrow_mut() = Some(target);
    }
}
