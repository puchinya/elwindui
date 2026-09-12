//! `elwindui::ui::ToggleSwitch` — the `ToggleSwitchExt` implementation.

use super::{NativeControl, base_accessibility_semantics, sync_intrinsic_enabled};
use crate::AnyView;
use crate::inner::InnerToggleSwitch;
use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityCheckState, AccessibilityRole,
};
use elwindui_core::ui::{ToggleSwitchExt, UIElementExt};
use std::cell::RefCell;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::ToggleSwitchExt, inherits = crate::NativeControl)]
pub struct ToggleSwitch {
    inner: InnerToggleSwitch,
    on_change: RefCell<Option<Box<dyn Fn(bool)>>>,
}

#[elwindui_macros::class]
impl ToggleSwitch {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Activate => {
                let is_on = self
                    .base
                    .intrinsic_accessibility_semantics()
                    .and_then(|semantics| semantics.state.checked)
                    .is_some_and(|state| state == AccessibilityCheckState::On);
                self.set_is_on(!is_on);
                self.notify_change(!is_on);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_is_on(&self, is_on: bool) {
        self.inner.set_is_on(is_on);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::Switch,
                    &[
                        AccessibilityActionKind::Activate,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        semantics.state.checked = Some(if is_on {
            AccessibilityCheckState::On
        } else {
            AccessibilityCheckState::Off
        });
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
        *self.on_change.borrow_mut() = Some(callback);
        self.install_change_trampoline();
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
        sync_intrinsic_enabled(self.base.as_ui_element(), enabled);
    }

    fn construct() -> Self {
        let inner = InnerToggleSwitch::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            on_change: RefCell::new(None),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = base_accessibility_semantics(
            AccessibilityRole::Switch,
            &[
                AccessibilityActionKind::Activate,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.state.checked = Some(AccessibilityCheckState::Off);
        self.base.set_intrinsic_accessibility_semantics(semantics);
        self.set_tab_stop(true);
        self.install_change_trampoline();
    }

    /// `#[two_way] is_on` — the change-back half of the binding; `elwindui_core::ui::ToggleSwitch::
    /// set_is_on` is the model→widget half. Mirrors `TextBox::set_on_text_change`'s own naming.
    #[inherent]
    pub fn set_on_is_on_change(&self, callback: Box<dyn Fn(bool)>) {
        self.set_on_change(callback);
    }

    #[inherent]
    fn install_change_trampoline(&self) {
        let owner: Rc<dyn UIElementExt> =
            self.as_ui_element().visual_collection.owner_rc().expect(
                "ToggleSwitch must be Rc-constructed before installing its change callback",
            );
        let weak = Rc::downgrade(&owner);
        self.inner.set_on_change(Box::new(move |is_on| {
            let Some(owner) = weak.upgrade() else { return };
            let this = owner
                .as_any()
                .downcast_ref::<ToggleSwitch>()
                .expect("ToggleSwitch owner must downcast to ToggleSwitch");
            this.set_is_on(is_on);
            this.notify_change(is_on);
        }));
    }

    #[inherent]
    fn notify_change(&self, is_on: bool) {
        let callback = self.on_change.borrow_mut().take();
        if let Some(callback) = callback {
            callback(is_on);
            let mut slot = self.on_change.borrow_mut();
            if slot.is_none() {
                *slot = Some(callback);
            }
        }
    }
}
