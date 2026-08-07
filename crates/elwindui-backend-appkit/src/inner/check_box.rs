//! `NSButton` set to `NSButtonType::Switch` — the same widget `InnerButton` wraps, in a different
//! button type, reusing its `ButtonTarget` click trampoline directly rather than duplicating it.

use crate::ffi::{AnyView, mtm};
use crate::inner::button::ButtonTarget;
use elwindui_core::ui::CheckState;
use objc2::rc::Retained;
use objc2_app_kit::{NSButton, NSButtonType, NSControlStateValueMixed, NSControlStateValueOff, NSControlStateValueOn};
use objc2_foundation::NSString;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct InnerCheckBox {
    pub(crate) handle: AnyView,
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl InnerCheckBox {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(""), None, None, m)
        };
        ns.setButtonType(NSButtonType::Switch);
        // `allowsMixedState(false)` (the default) does more than gate the user-click cycle on this
        // AppKit version: it also makes `setState(.mixed)` a silent no-op that renders as `.on`
        // instead of the dash glyph — confirmed empirically via `tools/macos-ui-driver` (a
        // programmatic `checked: CheckState::Indeterminate` produced no visible change from
        // `Checked`). So mixed state must stay *allowed* at the AppKit level for a `component` to
        // ever actually see the dash; `set_on_change`'s own click callback below is what keeps a
        // real user click from ever landing on it instead (`CheckBox`'s own doc comment,
        // elwindui-core).
        ns.setAllowsMixedState(true);
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

    pub(crate) fn set_checked(&self, checked: CheckState) {
        self.ns.setState(check_state_to_state(checked));
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(CheckState)>) {
        let ns = self.ns.clone();
        let target = ButtonTarget::attach(
            &self.ns,
            Box::new(move || {
                // `allowsMixedState(true)` (see `new`) lets AppKit's own click-tracking cycle
                // Off -> On -> Mixed -> Off across repeated clicks; coerce a click that landed on
                // `Mixed` back to `Checked` here, synchronously, before this closure returns, so
                // the state a real user click can ever land on stays exactly `Unchecked`/`Checked`
                // (`CheckBox`'s own doc comment, elwindui-core) with no visible mixed-state frame.
                let mut state = state_to_check_state(ns.state());
                if state == CheckState::Indeterminate {
                    state = CheckState::Checked;
                    ns.setState(check_state_to_state(state));
                }
                callback(state);
            }),
        );
        *self.target_storage.borrow_mut() = Some(target);
    }
}

fn check_state_to_state(state: CheckState) -> objc2_app_kit::NSControlStateValue {
    match state {
        CheckState::Unchecked => NSControlStateValueOff,
        CheckState::Checked => NSControlStateValueOn,
        CheckState::Indeterminate => NSControlStateValueMixed,
    }
}

fn state_to_check_state(state: objc2_app_kit::NSControlStateValue) -> CheckState {
    if state == NSControlStateValueOn {
        CheckState::Checked
    } else if state == NSControlStateValueMixed {
        CheckState::Indeterminate
    } else {
        CheckState::Unchecked
    }
}
