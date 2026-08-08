//! The XAML `Slider` and its `ValueChanged` event.

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBaseValueChangedEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Controls::Slider as XamlSlider;
use crate::ffi::{
    AnyView, UiEventGate, invoke_ui_f32_event_callback, register_ui_f32_event_callback,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Raw `XamlSlider` + change wiring — composed by `native_ui::Slider`.
pub(crate) struct InnerSlider {
    handle: AnyView,
    xaml: XamlSlider,
    on_change: Rc<RefCell<Option<Box<dyn Fn(f32)>>>>,
    events: UiEventGate,
}

impl InnerSlider {
    pub(crate) fn new() -> Self {
        let xaml = XamlSlider::new().expect("Slider::new");
        let handle = AnyView::from(xaml.clone());
        let on_change: Rc<RefCell<Option<Box<dyn Fn(f32)>>>> = Rc::new(RefCell::new(None));
        let events = UiEventGate::default();
        let callback = on_change.clone();
        let callback_events = events.clone();
        let callback_id = register_ui_f32_event_callback(Rc::new(move |value| {
            if callback_events.is_suppressed() {
                return;
            }
            if let Some(callback) = callback.borrow().as_ref() {
                callback(value);
            }
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
            let _ = self.xaml.SetValue(value as f64);
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
        *self.on_change.borrow_mut() = Some(callback);
    }
}
