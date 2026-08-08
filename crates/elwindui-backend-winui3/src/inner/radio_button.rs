//! The XAML `RadioButton` and its `Checked` event. Group exclusivity itself lives one layer up,
//! in `native_ui::RadioButton` — deliberately not delegated to `RadioButton.GroupName` (see that
//! type's own doc comment for why).

use crate::bindings::Microsoft::UI::Xaml::Controls::RadioButton as XamlRadioButton;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, UiEventGate, invoke_ui_event_callback, register_ui_event_callback};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::Foundation::{IReference, PropertyValue};
use windows::core::{HSTRING, Interface};

static NEXT_NATIVE_GROUP: AtomicUsize = AtomicUsize::new(1);

/// Raw `XamlRadioButton` + click wiring — composed by `native_ui::RadioButton`.
pub(crate) struct InnerRadioButton {
    handle: AnyView,
    xaml: XamlRadioButton,
    on_click: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    events: UiEventGate,
}

impl InnerRadioButton {
    pub(crate) fn new() -> Self {
        let xaml = XamlRadioButton::new().expect("RadioButton::new");
        // An unset `GroupName` makes WinUI implicitly group every RadioButton with the same visual
        // parent. Give each raw widget a unique native group so only elwindui's logical `group`
        // registry decides exclusivity, including when two logical groups share one TreeHostPanel.
        let native_group = format!(
            "elwindui-radio-{}",
            NEXT_NATIVE_GROUP.fetch_add(1, Ordering::Relaxed)
        );
        let _ = xaml.SetGroupName(&HSTRING::from(native_group));
        let handle = AnyView::from(xaml.clone());
        let events = UiEventGate::default();
        let this = Self {
            handle,
            xaml,
            on_click: Rc::new(RefCell::new(None)),
            events,
        };
        let callback = this.on_click.clone();
        let events = this.events.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            if events.is_suppressed() {
                return;
            }
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
        self.events.suppress(|| {
            let _ = self.xaml.SetIsChecked(value.as_ref());
        });
    }

    /// The raw click signal — see `InnerRadioButton`'s AppKit counterpart's own doc comment for
    /// why this reports no value of its own and leaves exclusivity to `native_ui::RadioButton`.
    pub(crate) fn set_on_click(&self, callback: Box<dyn Fn()>) {
        *self.on_click.borrow_mut() = Some(callback);
    }
}
