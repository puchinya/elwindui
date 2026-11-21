//! The XAML `CheckBox` and its `Checked`/`Unchecked`/`Indeterminate` events.

use super::{CallbackSlot, invoke_slot, new_callback_slot};
use crate::bindings::Microsoft::UI::Xaml::Controls::CheckBox as XamlCheckBox;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, UiCallbackRegistryOwner, UiEventGate, invoke_ui_event_callback};
use elwindui_core::ui::CheckState;
use std::rc::Rc;
use windows::Foundation::{IReference, PropertyValue};
use windows::core::{HSTRING, Interface};

/// Raw `XamlCheckBox` + change wiring — composed by `native_ui::CheckBox`.
pub(crate) struct InnerCheckBox {
    handle: AnyView,
    xaml: XamlCheckBox,
    on_change: CallbackSlot<CheckState>,
    events: UiEventGate,
    /// Owns every registry entry this widget registered; dropping the widget removes them.
    callbacks: UiCallbackRegistryOwner,
}

impl InnerCheckBox {
    pub(crate) fn new() -> Self {
        let xaml = XamlCheckBox::new().expect("CheckBox::new");
        // `IsThreeState` is enabled only while a programmatic `Indeterminate` is displayed (see
        // `set_checked`). Leaving it enabled permanently makes XAML's user cycle advance from
        // Checked to Indeterminate on every subsequent click; coercing that event back to Checked
        // would then leave the control stuck. Ordinary Checked/Unchecked model updates restore
        // XAML's native two-state user cycle.
        let handle = AnyView::from(xaml.clone());
        let events = UiEventGate::default();
        let this = Self {
            handle,
            xaml,
            on_change: new_callback_slot(),
            events,
            callbacks: UiCallbackRegistryOwner::default(),
        };
        for (event, state) in [
            ("Checked", CheckState::Checked),
            ("Unchecked", CheckState::Unchecked),
        ] {
            let callback = this.on_change.clone();
            let events = this.events.clone();
            let callback_id = this.callbacks.register_event(Rc::new(move || {
                if events.is_suppressed() {
                    return;
                }
                invoke_slot(&callback, state);
            }));
            let handler = RoutedEventHandler::new(move |_, _| {
                invoke_ui_event_callback(callback_id);
                Ok(())
            });
            let _ = match event {
                "Checked" => this.xaml.Checked(&handler),
                _ => this.xaml.Unchecked(&handler),
            };
        }
        // `IsThreeState(true)` lets XAML's own click-tracking cycle Unchecked -> Checked ->
        // Indeterminate -> Unchecked across repeated clicks, firing this event when it lands on the
        // third state. Coerce back to `Checked` here, synchronously, so the state a real user click
        // can ever land on stays exactly `Unchecked`/`Checked` — mirrors the AppKit backend's own
        // `set_on_change` coercion exactly.
        {
            let xaml_for_coerce = this.xaml.clone();
            let callback = this.on_change.clone();
            let events = this.events.clone();
            let callback_id = this.callbacks.register_event(Rc::new(move || {
                if events.is_suppressed() {
                    return;
                }
                events.suppress(|| {
                    let value = PropertyValue::CreateBoolean(true)
                        .ok()
                        .and_then(|v| v.cast::<IReference<bool>>().ok());
                    let _ = xaml_for_coerce.SetIsChecked(value.as_ref());
                });
                invoke_slot(&callback, CheckState::Checked);
            }));
            let handler = RoutedEventHandler::new(move |_, _| {
                invoke_ui_event_callback(callback_id);
                Ok(())
            });
            let _ = this.xaml.Indeterminate(&handler);
        }
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_text(&self, text: &str) {
        if let Ok(value) = PropertyValue::CreateString(&HSTRING::from(text)) {
            let _ = self.xaml.SetContent(&value);
        }
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    /// `CheckBox.IsChecked` is `Windows.Foundation.IReference<bool>` — nullable, with `null`
    /// meaning indeterminate. Three-state mode is enabled for that programmatic value and disabled
    /// again for either Boolean value, preserving both the dash glyph and a native two-state user
    /// click cycle. The `Indeterminate` event handler remains a defensive guard for a user action
    /// that reaches the third state before a model refresh restores two-state mode.
    pub(crate) fn set_checked(&self, checked: CheckState) {
        let value = match checked {
            CheckState::Unchecked => PropertyValue::CreateBoolean(false)
                .ok()
                .and_then(|v| v.cast::<IReference<bool>>().ok()),
            CheckState::Checked => PropertyValue::CreateBoolean(true)
                .ok()
                .and_then(|v| v.cast::<IReference<bool>>().ok()),
            CheckState::Indeterminate => None,
        };
        self.events.suppress(|| {
            let _ = self
                .xaml
                .SetIsThreeState(matches!(checked, CheckState::Indeterminate));
            let _ = self.xaml.SetIsChecked(value.as_ref());
        });
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(CheckState)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }
}
