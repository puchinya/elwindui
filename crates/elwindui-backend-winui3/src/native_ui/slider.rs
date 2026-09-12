//! `elwindui::ui::Slider` — the Windows-tested `SliderExt` implementation. Mirrors
//! `elwindui_backend_appkit::native_ui::slider`'s own structure exactly.

use super::{NativeControl, base_accessibility_semantics};
use crate::AnyView;
use crate::inner::InnerSlider;
use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityRole, ValueRange,
};
use elwindui_core::ui::{SliderExt, UIElementExt};
use std::cell::RefCell;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::SliderExt, inherits = crate::NativeControl)]
pub struct Slider {
    inner: InnerSlider,
    on_change: RefCell<Option<Box<dyn Fn(f32)>>>,
}

#[elwindui_macros::class]
impl Slider {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::SetValue(value) => {
                self.set_value(value as f32);
                self.notify_change(value as f32);
                true
            }
            AccessibilityAction::Increment | AccessibilityAction::Decrement => {
                let Some(range) = self
                    .base
                    .intrinsic_accessibility_semantics()
                    .and_then(|semantics| semantics.state.value_range)
                else {
                    return false;
                };
                let step = range.step.unwrap_or(1.0);
                let value = if matches!(action, AccessibilityAction::Increment) {
                    range.value + step
                } else {
                    range.value - step
                };
                self.set_value(value as f32);
                self.notify_change(value as f32);
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

    fn set_value(&self, value: f32) {
        self.inner.set_value(value);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::Slider,
                    &[
                        AccessibilityActionKind::SetValue,
                        AccessibilityActionKind::Increment,
                        AccessibilityActionKind::Decrement,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        let range = semantics.state.value_range.get_or_insert(ValueRange {
            value: value as f64,
            min: 0.0,
            max: 1.0,
            step: None,
        });
        range.value = value as f64;
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        *self.on_change.borrow_mut() = Some(callback);
        self.install_change_trampoline();
    }
    fn set_min(&self, min: f32) {
        self.inner.set_min(min);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::Slider,
                    &[
                        AccessibilityActionKind::SetValue,
                        AccessibilityActionKind::Increment,
                        AccessibilityActionKind::Decrement,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        let range = semantics.state.value_range.get_or_insert(ValueRange {
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: None,
        });
        range.min = min as f64;
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_max(&self, max: f32) {
        self.inner.set_max(max);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::Slider,
                    &[
                        AccessibilityActionKind::SetValue,
                        AccessibilityActionKind::Increment,
                        AccessibilityActionKind::Decrement,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        let range = semantics.state.value_range.get_or_insert(ValueRange {
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: None,
        });
        range.max = max as f64;
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerSlider::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            on_change: RefCell::new(None),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = base_accessibility_semantics(
            AccessibilityRole::Slider,
            &[
                AccessibilityActionKind::SetValue,
                AccessibilityActionKind::Increment,
                AccessibilityActionKind::Decrement,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.state.value_range = Some(ValueRange {
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: None,
        });
        self.base.set_intrinsic_accessibility_semantics(semantics);
        self.set_tab_stop(true);
        self.install_change_trampoline();
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_value_change(&self, callback: Box<dyn Fn(f32)>) {
        self.set_on_change(callback);
    }

    #[inherent]
    fn install_change_trampoline(&self) {
        let owner: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("Slider must be Rc-constructed before installing its value callback");
        let weak = Rc::downgrade(&owner);
        self.inner.set_on_change(Box::new(move |value| {
            let Some(owner) = weak.upgrade() else { return };
            let this = owner
                .as_any()
                .downcast_ref::<Slider>()
                .expect("Slider owner must downcast to Slider");
            this.set_value(value);
            this.notify_change(value);
        }));
    }

    #[inherent]
    fn notify_change(&self, value: f32) {
        let callback = self.on_change.borrow_mut().take();
        if let Some(callback) = callback {
            callback(value);
            let mut slot = self.on_change.borrow_mut();
            if slot.is_none() {
                *slot = Some(callback);
            }
        }
    }
}
