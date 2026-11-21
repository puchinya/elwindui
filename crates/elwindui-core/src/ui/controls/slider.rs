//! `elwindui::ui::Slider` — a native continuous-value slider.

use super::*;
use crate::ui::native::{NativePeer, SliderPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::Slider` — a native continuous-value slider (AppKit: `NSSlider`; WinUI3: `Slider`).
///
/// **Has no `text` property**, the same as `ToggleSwitch` — pair it with an adjacent `TextBlock`
/// if a label is needed.
///
/// `min`/`max` are runtime-mutable `#[prop]`s (not `#[param]`), so a `component` can change a
/// `Slider`'s range reactively rather than only at construction time. `value` is clamped to
/// `min..=max` by the native widget itself, not by elwindui.
///
/// Declared once here as a Core-owned Component (Issue #313) over a lazily realized
/// [`SliderPeer`]. A user drag reaches Core through the peer's change callback (an echo of the
/// stored value is dropped); a programmatic `set_value` does not notify.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(two_way, value: f32)]
#[prop(min: f32)]
#[prop(max: f32)]
#[prop(enabled: Option<bool>)]
pub struct Slider {
    value: Cell<f32>,
    min: Cell<f32>,
    max: Cell<f32>,
    enabled: Cell<bool>,
    on_change: RefCell<Option<Rc<dyn Fn(f32)>>>,
    peer: OnceCell<Rc<dyn SliderPeer>>,
}

#[elwindui_macros::component]
impl Slider {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::SetValue(value) => {
                self.set_value(value as f32);
                notify_change(&self.on_change, value as f32);
                true
            }
            AccessibilityAction::Increment | AccessibilityAction::Decrement => {
                let value = if matches!(action, AccessibilityAction::Increment) {
                    self.value.get() + 1.0
                } else {
                    self.value.get() - 1.0
                };
                self.set_value(value);
                notify_change(&self.on_change, value);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("Slider", |factory| factory.create_slider());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a Slider's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: range before value, then the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        peer.set_min(self.min.get());
        peer.set_max(self.max.get());
        peer.set_value(self.value.get());
        peer.set_enabled(self.enabled.get());
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &Slider, value: f32| this.on_native_change(value),
        )));
    }

    /// Programmatic: updates the display without notifying `on_change`.
    fn set_value(&self, value: f32) {
        self.value.set(value);
        if let Some(peer) = self.peer.get() {
            peer.set_value(value);
        }
        self.sync_range_semantics();
    }

    fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }

    fn set_min(&self, min: f32) {
        self.min.set(min);
        if let Some(peer) = self.peer.get() {
            peer.set_min(min);
        }
        self.sync_range_semantics();
    }

    fn set_max(&self, max: f32) {
        self.max.set(max);
        if let Some(peer) = self.peer.get() {
            peer.set_max(max);
        }
        self.sync_range_semantics();
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
    pub fn set_on_value_change(&self, callback: Box<dyn Fn(f32)>) {
        self.set_on_change(callback);
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            value: Cell::new(0.0),
            min: Cell::new(0.0),
            max: Cell::new(1.0),
            enabled: Cell::new(true),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = native_semantics(
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
    }
}

impl Slider {
    /// A user change reported by the peer (§5 two-way rule).
    fn on_native_change(&self, value: f32) {
        if self.value.get() == value {
            return;
        }
        self.value.set(value);
        self.sync_range_semantics();
        notify_change(&self.on_change, value);
    }

    fn sync_range_semantics(&self) {
        let (value, min, max) = (self.value.get(), self.min.get(), self.max.get());
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            let range = semantics.state.value_range.get_or_insert(ValueRange {
                value: 0.0,
                min: 0.0,
                max: 1.0,
                step: None,
            });
            range.value = value as f64;
            range.min = min as f64;
            range.max = max as f64;
        });
    }
}
