//! `ComboBox` — a native, non-editable selection control.

use crate::bindings::Microsoft::UI::Xaml::Controls::ComboBox as XamlComboBox;
use crate::bindings::Microsoft::UI::Xaml::Controls::SelectionChangedEventHandler;
use crate::ffi::{AnyView, invoke_ui_index_event_callback, register_ui_index_event_callback};
use std::cell::RefCell;
use std::rc::Rc;
use windows::Foundation::PropertyValue;
use windows::core::{HSTRING, Interface};

/// Raw `XamlComboBox` + change wiring — composed by `native_ui::Dropdown`.
pub(crate) struct InnerDropdown {
    handle: AnyView,
    xaml: XamlComboBox,
    on_change: Rc<RefCell<Option<Box<dyn Fn(usize)>>>>,
}

impl InnerDropdown {
    pub(crate) fn new() -> Self {
        let xaml = XamlComboBox::new().expect("ComboBox::new");
        let handle = AnyView::from(xaml.clone());
        let this = Self {
            handle,
            xaml,
            on_change: Rc::new(RefCell::new(None)),
        };
        let on_change = this.on_change.clone();
        let callback_id = register_ui_index_event_callback(Rc::new(move |index| {
            if let Some(callback) = on_change.borrow().as_ref() {
                callback(index);
            }
        }));
        let _ = this.xaml.SelectionChanged(&SelectionChangedEventHandler::new(
            move |sender, _| {
                if let Some(sender) = sender.cloned().and_then(|s| s.cast::<XamlComboBox>().ok())
                {
                    let index = sender.SelectedIndex().unwrap_or(-1);
                    if index >= 0 {
                        invoke_ui_index_event_callback(callback_id, index as usize);
                    }
                }
                Ok(())
            },
        ));
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    /// Full rebuild rather than incremental diffing against the previous item list — see
    /// `elwindui_backend_appkit::inner::InnerDropdown::rebuild_items`'s own doc comment (mirrored
    /// exactly here, unverified — no Windows environment).
    pub(crate) fn rebuild_items(&self, texts: &[String]) {
        if let Ok(items) = self.xaml.Items() {
            let _ = items.Clear();
            for text in texts {
                if let Ok(value) = PropertyValue::CreateString(&HSTRING::from(text)) {
                    let _ = items.Append(&value);
                }
            }
        }
    }

    pub(crate) fn set_selected_index(&self, index: usize) {
        let _ = self.xaml.SetSelectedIndex(index as i32);
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }
}
