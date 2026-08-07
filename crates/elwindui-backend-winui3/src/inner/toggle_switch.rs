//! The XAML `ToggleSwitch` and its `Toggled` event.

use crate::bindings::Microsoft::UI::Xaml::Controls::ToggleSwitch as XamlToggleSwitch;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use std::cell::RefCell;
use std::rc::Rc;

/// Raw `XamlToggleSwitch` + change wiring — composed by `native_ui::ToggleSwitch`.
pub(crate) struct InnerToggleSwitch {
    handle: AnyView,
    xaml: XamlToggleSwitch,
    on_change: Rc<RefCell<Option<Box<dyn Fn(bool)>>>>,
}

impl InnerToggleSwitch {
    pub(crate) fn new() -> Self {
        let xaml = XamlToggleSwitch::new().expect("ToggleSwitch::new");
        let handle = AnyView::from(xaml.clone());
        let this = Self {
            handle,
            xaml,
            on_change: Rc::new(RefCell::new(None)),
        };
        let callback = this.on_change.clone();
        let xaml_for_read = this.xaml.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            if let Some(callback) = callback.borrow().as_ref() {
                callback(xaml_for_read.IsOn().unwrap_or(false));
            }
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
        let _ = self.xaml.SetIsOn(is_on);
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }
}
