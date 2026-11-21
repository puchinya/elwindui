//! `elwindui::ui::CheckBox` — a native tri-state checkbox.

use super::*;
use crate::ui::native::{CheckBoxPeer, NativePeer, create_native_peer};
use std::cell::OnceCell;

/// A [`CheckBox`]'s check state.
///
/// Three states exist because `NSButton`/`Checkbox` genuinely support a mixed/indeterminate
/// display (e.g. "some but not all items in this group are selected"), but only two of them are
/// reachable by a user click — see [`CheckBox`]'s own doc comment for why `Indeterminate` is
/// program-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckState {
    #[default]
    Unchecked,
    Checked,
    /// A mixed/partial state. Never produced by a user click — only ever reached by setting
    /// `checked: CheckState::Indeterminate` from a `component`.
    Indeterminate,
}

/// `elwindui::ui::CheckBox` — a native tri-state checkbox (AppKit: `NSButton` with
/// `NSButtonType::Switch`; WinUI3: `CheckBox`).
///
/// **User interaction only ever toggles between `Unchecked` and `Checked`.** Both backends
/// disable native mixed-state cycling (AppKit `setAllowsMixedState(false)`) so a click can never
/// land on `Indeterminate` — that third state exists purely so a `component` can *display* "some
/// but not all" (e.g. a header checkbox reflecting a partially-selected list) without a user
/// being able to get stuck in it by clicking. Setting `checked: CheckState::Indeterminate` from
/// Rust still works. A user click on a box showing `Indeterminate` always moves it to `Checked`
/// (the common "select all" convention); Core enforces this for every backend. From `Checked` or
/// `Unchecked`, a click toggles as usual.
///
/// Declared once here as a Core-owned Component (Issue #313). The OS widget is a
/// [`CheckBoxPeer`](crate::ui::native::CheckBoxPeer) created lazily by the active backend (see
/// [`NativeControl`]). A programmatic `set_checked` updates Core's state and the peer without
/// notifying; a user click reaches Core through the peer's change callback, which stores the new
/// state and notifies once (an echo of the stored state is dropped).
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(two_way, checked: crate::ui::CheckState)]
#[prop(enabled: Option<bool>)]
pub struct CheckBox {
    text: RefCell<String>,
    checked: Cell<CheckState>,
    enabled: Cell<bool>,
    on_change: RefCell<Option<Rc<dyn Fn(CheckState)>>>,
    peer: OnceCell<Rc<dyn CheckBoxPeer>>,
}

#[elwindui_macros::component]
impl CheckBox {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Activate => {
                let next = match self.checked.get() {
                    CheckState::Checked => CheckState::Unchecked,
                    _ => CheckState::Checked,
                };
                self.set_checked(next);
                notify_change(&self.on_change, next);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("CheckBox", |factory| factory.create_check_box());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a CheckBox's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: push the logical state, then install the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        peer.set_checked(self.checked.get());
        peer.set_enabled(self.enabled.get());
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &CheckBox, state: CheckState| this.on_native_change(state),
        )));
    }

    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_text(text);
        }
        let latest = self.text.borrow().clone();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.label = Some(latest);
        });
        // Some toolkits drop the attributed title when the caption changes (AppKit `NSButton`),
        // and the caption changes the fitting size.
        self.base.reapply_text_style();
        self.base.invalidate_measure();
    }

    /// Programmatic: updates the display without notifying `on_change`.
    fn set_checked(&self, checked: crate::ui::CheckState) {
        self.checked.set(checked);
        if let Some(peer) = self.peer.get() {
            peer.set_checked(checked);
        }
        self.sync_checked_semantics();
    }

    fn set_on_change(&self, callback: Box<dyn Fn(crate::ui::CheckState)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        if let Some(peer) = self.peer.get() {
            peer.set_enabled(enabled);
        }
        sync_native_enabled(self.as_ui_element(), self.enabled.get());
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_checked_change(&self, callback: Box<dyn Fn(CheckState)>) {
        self.set_on_change(callback);
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            text: RefCell::new(String::new()),
            checked: Cell::new(CheckState::Unchecked),
            enabled: Cell::new(true),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = native_semantics(
            AccessibilityRole::CheckBox,
            &[
                AccessibilityActionKind::Activate,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.state.checked = Some(AccessibilityCheckState::Off);
        self.base.set_intrinsic_accessibility_semantics(semantics);
        self.set_tab_stop(true);
    }
}

impl CheckBox {
    /// A user change reported by the peer (§5 two-way rule): an echo of the stored state is
    /// dropped; otherwise Core stores it and notifies once, without pushing it back.
    ///
    /// A user click on a box showing `Indeterminate` always lands on `Checked` (the common
    /// "select all" convention, and the AppKit behavior). Toolkits whose own cycle goes elsewhere
    /// (WinUI3 three-state: `null` -> `false`) are corrected here, once, for every backend: only a
    /// corrected state is pushed back to the widget.
    fn on_native_change(&self, reported: CheckState) {
        let previous = self.checked.get();
        let state = if previous == CheckState::Indeterminate {
            CheckState::Checked
        } else {
            reported
        };
        if previous == state {
            return;
        }
        self.checked.set(state);
        // Only a corrected state needs to reach the widget; a reported one is already shown.
        if state != reported {
            if let Some(peer) = self.peer.get() {
                peer.set_checked(state);
            }
        }
        self.sync_checked_semantics();
        notify_change(&self.on_change, state);
    }

    fn sync_checked_semantics(&self) {
        let checked = self.checked.get();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.state.checked = Some(match checked {
                CheckState::Unchecked => AccessibilityCheckState::Off,
                CheckState::Checked => AccessibilityCheckState::On,
                CheckState::Indeterminate => AccessibilityCheckState::Mixed,
            });
        });
    }
}
