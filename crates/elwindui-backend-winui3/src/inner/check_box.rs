//! The XAML `CheckBox` and its `Checked`/`Unchecked`/`Indeterminate` events.

use crate::bindings::Microsoft::UI::Xaml::Controls::CheckBox as XamlCheckBox;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use elwindui_core::ui::CheckState;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Foundation::{IReference, PropertyValue};
use windows::core::HSTRING;

/// Raw `XamlCheckBox` + change wiring — composed by `native_ui::CheckBox`.
pub(crate) struct InnerCheckBox {
    handle: AnyView,
    xaml: XamlCheckBox,
    on_change: Rc<RefCell<Option<Box<dyn Fn(CheckState)>>>>,
}

impl InnerCheckBox {
    pub(crate) fn new() -> Self {
        let xaml = XamlCheckBox::new().expect("CheckBox::new");
        // `IsThreeState(false)` (the default) is documented to make XAML ignore a programmatic
        // `IsChecked = null` and render Unchecked instead — the same silent-coercion behavior
        // confirmed empirically on the AppKit backend's `NSButton.allowsMixedState` (see
        // `elwindui-backend-appkit`'s `inner/check_box.rs::new`'s own doc comment; unverified here,
        // no Windows environment, but mirrored on the same reasoning). So three-state stays
        // *enabled* at the XAML level for a `component`'s programmatic `Indeterminate` to ever
        // actually render the dash glyph; the `Indeterminate` event handler below is what keeps a
        // real user click from ever landing on it instead (`CheckBox`'s own doc comment,
        // elwindui-core).
        let _ = xaml.SetIsThreeState(true);
        let handle = AnyView::from(xaml.clone());
        let this = Self {
            handle,
            xaml,
            on_change: Rc::new(RefCell::new(None)),
        };
        for (event, state) in [
            ("Checked", CheckState::Checked),
            ("Unchecked", CheckState::Unchecked),
        ] {
            let callback = this.on_change.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                if let Some(callback) = callback.borrow().as_ref() {
                    callback(state);
                }
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
            let callback_id = register_ui_event_callback(Rc::new(move || {
                let value = PropertyValue::CreateBoolean(true)
                    .ok()
                    .and_then(|v| v.cast::<IReference<bool>>().ok());
                let _ = xaml_for_coerce.SetIsChecked(value.as_ref());
                if let Some(callback) = callback.borrow().as_ref() {
                    callback(CheckState::Checked);
                }
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
    /// meaning indeterminate. `IsThreeState` is `true` (see `new`) so XAML actually renders this,
    /// but the `Indeterminate` event handler registered there coerces any real user click away from
    /// it — so this remains the only path that ever leaves `Indeterminate` in effect, matching
    /// `CheckState`'s own doc comment.
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
        let _ = self.xaml.SetIsChecked(value.as_ref());
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(CheckState)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }
}
