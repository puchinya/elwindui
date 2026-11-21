//! The XAML `Slider` and its `ValueChanged` event.

use super::{CallbackSlot, invoke_slot, new_callback_slot};
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBaseValueChangedEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Controls::Slider as XamlSlider;
use crate::ffi::{AnyView, UiCallbackRegistryOwner, UiEventGate, invoke_ui_f32_event_callback};
use std::rc::Rc;

/// Raw `XamlSlider` + change wiring — composed by `native_ui::Slider`.
pub(crate) struct InnerSlider {
    handle: AnyView,
    xaml: XamlSlider,
    on_change: CallbackSlot<f32>,
    events: UiEventGate,
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
    }

    pub(crate) fn set_max(&self, max: f32) {
        self.events.suppress(|| {
            let _ = self.xaml.SetMaximum(max as f64);
        });
    }

    /// `NSSlider`'s AppKit counterpart fires continuously while dragging by default
    /// (`isContinuous`); `Slider.ValueChanged` already behaves the same way, so no extra setup is
    /// needed here to match.
    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }
}
