//! `elwindui::ui::CheckBox` — the `CheckBoxExt` implementation.

use super::{NativeControl, base_accessibility_semantics};
use crate::AnyView;
use crate::inner::InnerCheckBox;
use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityCheckState, AccessibilityRole,
};
use elwindui_core::ui::{CheckBoxExt, CheckState, UIElementExt};
use std::cell::RefCell;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::CheckBoxExt, inherits = crate::NativeControl)]
pub struct CheckBox {
    inner: InnerCheckBox,
    on_change: RefCell<Option<Box<dyn Fn(CheckState)>>>,
}

#[elwindui_macros::class]
impl CheckBox {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Activate => {
                let next = match self
                    .base
                    .intrinsic_accessibility_semantics()
                    .and_then(|semantics| semantics.state.checked)
                {
                    Some(AccessibilityCheckState::On) => CheckState::Unchecked,
                    _ => CheckState::Checked,
                };
                self.set_checked(next);
                self.notify_change(next);
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

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::CheckBox,
                    &[
                        AccessibilityActionKind::Activate,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        semantics.label = Some(text.to_string());
        self.base.set_intrinsic_accessibility_semantics(semantics);
        self.base.reapply_text_style();
    }
    fn set_checked(&self, checked: CheckState) {
        self.inner.set_checked(checked);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::CheckBox,
                    &[
                        AccessibilityActionKind::Activate,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        semantics.state.checked = Some(match checked {
            CheckState::Unchecked => AccessibilityCheckState::Off,
            CheckState::Checked => AccessibilityCheckState::On,
            CheckState::Indeterminate => AccessibilityCheckState::Mixed,
        });
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(CheckState)>) {
        *self.on_change.borrow_mut() = Some(callback);
        self.install_change_trampoline();
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerCheckBox::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            on_change: RefCell::new(None),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = base_accessibility_semantics(
            AccessibilityRole::CheckBox,
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

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_checked_change(&self, callback: Box<dyn Fn(CheckState)>) {
        self.set_on_change(callback);
    }

    #[inherent]
    fn install_change_trampoline(&self) {
        let owner: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("CheckBox must be Rc-constructed before installing its change callback");
        let weak = Rc::downgrade(&owner);
        self.inner.set_on_change(Box::new(move |state| {
            let Some(owner) = weak.upgrade() else { return };
            let this = owner
                .as_any()
                .downcast_ref::<CheckBox>()
                .expect("CheckBox owner must downcast to CheckBox");
            this.set_checked(state);
            this.notify_change(state);
        }));
    }

    #[inherent]
    fn notify_change(&self, state: CheckState) {
        let callback = self.on_change.borrow_mut().take();
        if let Some(callback) = callback {
            callback(state);
            let mut slot = self.on_change.borrow_mut();
            if slot.is_none() {
                *slot = Some(callback);
            }
        }
    }
}
