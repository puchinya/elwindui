//! The XAML `ToggleSwitch` and its `Toggled` event.

use super::{CallbackSlot, invoke_slot, new_callback_slot};
use crate::bindings::Microsoft::UI::Xaml::Controls::ToggleSwitch as XamlToggleSwitch;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, UiCallbackRegistryOwner, UiEventGate, invoke_ui_event_callback};
use std::rc::Rc;

/// Raw `XamlToggleSwitch` + change wiring — composed by `native_ui::ToggleSwitch`.
pub(crate) struct InnerToggleSwitch {
    handle: AnyView,
    xaml: XamlToggleSwitch,
    on_change: CallbackSlot<bool>,
    events: UiEventGate,
    /// Owns every registry entry this widget registered; dropping the widget removes them.
    callbacks: UiCallbackRegistryOwner,
}

impl InnerToggleSwitch {
    pub(crate) fn new() -> Self {
        let xaml = XamlToggleSwitch::new().expect("ToggleSwitch::new");
        let handle = AnyView::from(xaml.clone());
        let events = UiEventGate::default();
        let this = Self {
            handle,
            xaml,
            on_change: new_callback_slot(),
            events,
            callbacks: UiCallbackRegistryOwner::default(),
        };
        let callback = this.on_change.clone();
        let xaml_for_read = this.xaml.clone();
        let events = this.events.clone();
        let callback_id = this.callbacks.register_event(Rc::new(move || {
            if events.is_suppressed() {
                return;
            }
            invoke_slot(&callback, xaml_for_read.IsOn().unwrap_or(false));
        }));
        let _ = this.xaml.Toggled(&RoutedEventHandler::new(move |_, _| {
            invoke_ui_event_callback(callback_id);
            Ok(())
        }));
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    pub(crate) fn set_is_on(&self, is_on: bool) {
        self.events.suppress(|| {
            let _ = self.xaml.SetIsOn(is_on);
        });
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }
}
