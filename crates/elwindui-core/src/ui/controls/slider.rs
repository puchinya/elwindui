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
#[prop(step: Option<f32>)]
#[prop(enabled: Option<bool>)]
pub struct Slider {
    value: Cell<f32>,
    min: Cell<f32>,
    max: Cell<f32>,
    /// The normalized step: `Some` only for a finite, positive value (`ui_spec.md` Slider `step`).
    step: Cell<Option<f32>>,
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
                self.accessibility_set_value(value as f32);
                true
            }
            AccessibilityAction::Increment => {
                self.accessibility_set_value(self.value.get() + self.increment());
                true
            }
            AccessibilityAction::Decrement => {
                self.accessibility_set_value(self.value.get() - self.increment());
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

    /// Realization phase 3: range, then step, then value, then the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        peer.set_min(self.min.get());
        peer.set_max(self.max.get());
        peer.set_step(self.step.get());
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

    /// Sets the step. A value that is not finite and positive makes the slider continuous. The
    /// current value is not re-snapped; only later user changes snap (`ui_spec.md`).
    fn set_step(&self, step: f32) {
        let step = (step.is_finite() && step > 0.0).then_some(step);
        self.step.set(step);
        if let Some(peer) = self.peer.get() {
            peer.set_step(step);
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
            step: Cell::new(None),
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
    /// A user change made through accessibility (§7b). The requested value is clamped to the
    /// current range first, exactly as the native widget would clamp a drag, so Core's logical
    /// value, the accessibility range and the notified value never leave `min..=max`. A request
    /// whose clamped value equals the stored value (`SetValue(current)`, `Increment` at `max`,
    /// `Decrement` at `min`) changes nothing and notifies nothing; otherwise the value is applied
    /// and `on_change` fires exactly once.
    fn accessibility_set_value(&self, requested: f32) {
        let value = self.snap(requested);
        if value == self.value.get() {
            return;
        }
        self.set_value(value);
        notify_change(&self.on_change, value);
    }

    /// The accessibility `Increment`/`Decrement` amount: the step, or 1 when continuous.
    fn increment(&self) -> f32 {
        self.step.get().unwrap_or(1.0)
    }

    /// Where a user change lands (`ui_spec.md` Slider `step`): inside `low..=high` and, with a
    /// step, on `low + k × step` or on `high`, whichever is nearer.
    fn snap(&self, requested: f32) -> f32 {
        snap_slider_value(requested, self.min.get(), self.max.get(), self.step.get())
    }

    /// A user change reported by the peer (§5 two-way rule). It is snapped first; a snapped value
    /// that differs from the report is pushed back to the widget (programmatic, no echo).
    fn on_native_change(&self, reported: f32) {
        let value = self.snap(reported);
        if value != reported {
            if let Some(peer) = self.peer.get() {
                peer.set_value(value);
            }
        }
        if self.value.get() == value {
            return;
        }
        self.value.set(value);
        self.sync_range_semantics();
        notify_change(&self.on_change, value);
    }

    fn sync_range_semantics(&self) {
        let Some(mut semantics) = self.intrinsic_accessibility_semantics() else {
            return;
        };
        let range = semantics.state.value_range.get_or_insert(ValueRange {
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: None,
        });
        range.value = f64::from(self.value.get());
        range.min = f64::from(self.min.get());
        range.max = f64::from(self.max.get());
        range.step = self.step.get().map(f64::from);
        self.set_intrinsic_accessibility_semantics(semantics);
    }
}

/// Snaps a user-requested slider value (`ui_spec.md` Slider `step`). `f32::max`/`min` (unlike
/// `clamp`) never panic, so a NaN request or an inverted range still lands inside the range. The
/// grid arithmetic runs in `f64`: every finite positive `f32` step is valid, and an `f32` quotient
/// `(value - low) / step` overflows to infinity for a tiny step (or a span wider than `f32::MAX`),
/// which would otherwise throw the value to `high`. The `f64` quotient of two finite `f32`s cannot.
pub(crate) fn snap_slider_value(requested: f32, min: f32, max: f32, step: Option<f32>) -> f32 {
    let (low, high) = (min.min(max), min.max(max));
    let clamped = requested.max(low).min(high);
    let Some(step) = step else {
        return clamped;
    };
    let (low64, high64, value64, step64) = (
        f64::from(low),
        f64::from(high),
        f64::from(clamped),
        f64::from(step),
    );
    let on_grid = (low64 + ((value64 - low64) / step64).round() * step64)
        .max(low64)
        .min(high64);
    let snapped = if (high64 - value64).abs() < (on_grid - value64).abs() {
        high64
    } else {
        on_grid
    };
    // Narrowing may round just past an end of the range; clamp again in `f32`.
    (snapped as f32).max(low).min(high)
}
