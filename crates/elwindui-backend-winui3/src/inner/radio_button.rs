//! The XAML `RadioButton` and its `Checked` event. Group exclusivity itself lives one layer up,
//! in `native_ui::RadioButton` — deliberately not delegated to `RadioButton.GroupName` (see that
//! type's own doc comment for why).

use crate::bindings::Microsoft::UI::Xaml::Controls::RadioButton as XamlRadioButton;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use std::cell::RefCell;
use std::rc::Rc;
use windows::Foundation::{IReference, PropertyValue};
use windows::core::HSTRING;

/// Raw `XamlRadioButton` + click wiring — composed by `native_ui::RadioButton`.
pub(crate) struct InnerRadioButton {
    handle: AnyView,
    xaml: XamlRadioButton,
    on_click: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl InnerRadioButton {
    pub(crate) fn new() -> Self {
        let xaml = XamlRadioButton::new().expect("RadioButton::new");
        // Deliberately empty: an empty `GroupName` still participates in WinUI 3's own
        // visual-parent-based automatic grouping, the exact ambiguity `RadioButton`'s own doc
        // comment explains elwindui avoids by managing groups itself. Nothing to set here.
        let handle = AnyView::from(xaml.clone());
        let this = Self {
            handle,
            xaml,
            on_click: Rc::new(RefCell::new(None)),
        };
        let callback = this.on_click.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            if let Some(callback) = callback.borrow().as_ref() {
                callback();
            }
        }));
        let _ = this.xaml.Checked(&RoutedEventHandler::new(move |_, _| {
            invoke_ui_event_callback(callback_id);
            Ok(())
        }));
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

    pub(crate) fn set_checked(&self, checked: bool) {
        let value = PropertyValue::CreateBoolean(checked)
            .ok()
            .and_then(|v| v.cast::<IReference<bool>>().ok());
        let _ = self.xaml.SetIsChecked(value.as_ref());
    }

    /// The raw click signal — see `InnerRadioButton`'s AppKit counterpart's own doc comment for
    /// why this reports no value of its own and leaves exclusivity to `native_ui::RadioButton`.
    pub(crate) fn set_on_click(&self, callback: Box<dyn Fn()>) {
        *self.on_click.borrow_mut() = Some(callback);
    }
}
