//! The XAML `Slider` and its `ValueChanged` event.

use super::{CallbackSlot, invoke_slot, new_callback_slot};
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBaseValueChangedEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Controls::Slider as XamlSlider;
use crate::ffi::{AnyView, UiCallbackRegistryOwner, UiEventGate, invoke_ui_f32_event_callback};
use std::cell::Cell;
use std::rc::Rc;

/// Raw `XamlSlider` + change wiring — composed by `native_ui::Slider`.
pub(crate) struct InnerSlider {
    handle: AnyView,
    xaml: XamlSlider,
    on_change: CallbackSlot<f32>,
    events: UiEventGate,
    /// The last pushed range and normalized step, from which `StepFrequency`/`SmallChange` are
    /// derived (`apply_step`).
    range: Cell<(f32, f32)>,
    step: Cell<Option<f32>>,
    /// Owns every registry entry this widget registered; dropping the widget removes them. Held
    /// only for that `Drop`.
    _callbacks: UiCallbackRegistryOwner,
}

impl InnerSlider {
    pub(crate) fn new() -> Self {
        let xaml = XamlSlider::new().expect("Slider::new");
        let handle = AnyView::from(xaml.clone());
        let on_change: CallbackSlot<f32> = new_callback_slot();
        let callbacks = UiCallbackRegistryOwner::default();
        let events = UiEventGate::default();
        let callback = on_change.clone();
        let callback_events = events.clone();
        let callback_id = callbacks.register_f32_event(Rc::new(move |value| {
            if callback_events.is_suppressed() {
                return;
            }
            invoke_slot(&callback, value);
        }));
        let _ = xaml.ValueChanged(&RangeBaseValueChangedEventHandler::new(
            move |_sender, args| {
                if let Some(args) = args.cloned() {
                    if let Ok(new_value) = args.NewValue() {
                        invoke_ui_f32_event_callback(callback_id, new_value as f32);
                    }
                }
                Ok(())
            },
        ));
        Self {
            handle,
            xaml,
            on_change,
            events,
            range: Cell::new((0.0, 1.0)),
            step: Cell::new(None),
            _callbacks: callbacks,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    pub(crate) fn set_value(&self, value: f32) {
        self.events.suppress(|| {
            let _ = self.xaml.SetValue2(value as f64);
        });
    }

    pub(crate) fn set_min(&self, min: f32) {
        self.events.suppress(|| {
            let _ = self.xaml.SetMinimum(min as f64);
        });
        self.range.set((min, self.range.get().1));
        self.apply_step();
    }

    pub(crate) fn set_max(&self, max: f32) {
        self.events.suppress(|| {
            let _ = self.xaml.SetMaximum(max as f64);
        });
        self.range.set((self.range.get().0, max));
        self.apply_step();
    }

    /// Core owns the snapping rule; this only tunes the native feel.
    pub(crate) fn set_step(&self, step: Option<f32>) {
        self.step.set(step);
        self.apply_step();
    }

    /// With a step, dragging and the arrow keys move by it. Without one the slider is continuous:
    /// `StepFrequency` defaults to 1, which would snap every `0..1` slider to its ends, so it is set
    /// to 1/10000 of the range, while `SmallChange` stays 1 like the accessibility `Increment`.
    fn apply_step(&self) {
        let (frequency, small_change) = match self.step.get() {
            Some(step) => (step as f64, step as f64),
            None => {
                let (min, max) = self.range.get();
                let range = (max as f64 - min as f64).abs();
                let frequency = if range.is_finite() && range > 0.0 {
                    range / 10_000.0
                } else {
                    1e-4
                };
                (frequency, 1.0)
            }
        };
        self.events.suppress(|| {
            let _ = self.xaml.SetStepFrequency(frequency);
            let _ = self.xaml.SetSmallChange(small_change);
        });
    }

    /// `NSSlider`'s AppKit counterpart fires continuously while dragging by default
    /// (`isContinuous`); `Slider.ValueChanged` already behaves the same way, so no extra setup is
    /// needed here to match.
    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }
}
