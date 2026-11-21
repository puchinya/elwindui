//! `elwindui::ui::ToggleSwitch` — a native on/off switch.

use super::*;
use crate::ui::native::{NativePeer, ToggleSwitchPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::ToggleSwitch` — a native on/off switch (AppKit: `NSSwitch`, macOS 10.15+; WinUI3:
/// `ToggleSwitch`).
///
/// **Has no `text` property.** Neither `NSSwitch` nor a bare Fluent `ToggleSwitch` carries a
/// label the way `Button`/`CheckBox`/`RadioButton` do — pair it with an adjacent `TextBlock`
/// (`HorizontalLayout { ToggleSwitch { .. } TextBlock { text: ".." } }`) the same way a `Slider`
/// or any other unlabeled control would be.
///
/// Declared once here as a Core-owned Component (Issue #313) over a lazily realized
/// [`ToggleSwitchPeer`]. A user toggle reaches Core through the peer's change callback (an echo of
/// the stored state is dropped); a programmatic `set_is_on` does not notify.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(two_way, is_on: bool)]
#[prop(enabled: Option<bool>)]
pub struct ToggleSwitch {
    is_on: Cell<bool>,
    enabled: Cell<bool>,
    on_change: RefCell<Option<Rc<dyn Fn(bool)>>>,
    peer: OnceCell<Rc<dyn ToggleSwitchPeer>>,
}

#[elwindui_macros::component]
impl ToggleSwitch {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Activate => {
                let next = !self.is_on.get();
                self.set_is_on(next);
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
        let peer = create_native_peer("ToggleSwitch", |factory| factory.create_toggle_switch());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a ToggleSwitch's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: push the logical state, then install the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        peer.set_is_on(self.is_on.get());
        peer.set_enabled(self.enabled.get());
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &ToggleSwitch, is_on: bool| this.on_native_change(is_on),
        )));
    }

    /// Programmatic: updates the display without notifying `on_change`.
    fn set_is_on(&self, is_on: bool) {
        self.is_on.set(is_on);
        if let Some(peer) = self.peer.get() {
            peer.set_is_on(is_on);
        }
        self.sync_is_on_semantics();
    }

    fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
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
    pub fn set_on_is_on_change(&self, callback: Box<dyn Fn(bool)>) {
        self.set_on_change(callback);
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            is_on: Cell::new(false),
            enabled: Cell::new(true),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = native_semantics(
            AccessibilityRole::Switch,
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

impl ToggleSwitch {
    /// A user change reported by the peer (§5 two-way rule).
    fn on_native_change(&self, is_on: bool) {
        if self.is_on.get() == is_on {
            return;
        }
        self.is_on.set(is_on);
        self.sync_is_on_semantics();
        notify_change(&self.on_change, is_on);
    }

    fn sync_is_on_semantics(&self) {
        let is_on = self.is_on.get();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.state.checked = Some(if is_on {
                AccessibilityCheckState::On
            } else {
                AccessibilityCheckState::Off
            });
        });
    }
}
