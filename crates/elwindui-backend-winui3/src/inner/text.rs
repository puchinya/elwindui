//! The three text controls: `TextBox` (multiline = `TextArea`), single-line `TextBox`, and
//! `PasswordBox`.

use crate::bindings;
use crate::bindings::Microsoft::UI::Xaml::Controls::TextChangedEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    PasswordBox as XamlPasswordBox, TextBox as XamlTextBox,
};
use crate::bindings::Microsoft::UI::Xaml::Input::KeyEventHandler;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use crate::render::xaml_text_alignment;
use std::cell::RefCell;
use std::rc::Rc;
use windows::System::VirtualKey;
use windows::core::HSTRING;

/// Raw `TextBox` (multi-line configured — `SetAcceptsReturn(true)`/`SetTextWrapping(Wrap)`, unlike
/// `InnerTextBox`'s single-line configuration of the exact same underlying XAML class below) +
/// change-notification wiring — composed by `native_ui::TextArea`.
pub(crate) struct InnerTextArea {
    handle: AnyView,
    text_box: XamlTextBox,
    on_change: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
}

impl InnerTextArea {
    pub(crate) fn new() -> Self {
        let text_box = XamlTextBox::new().expect("TextBox::new");
        let _ = text_box.SetAcceptsReturn(true);
        let _ = text_box.SetTextWrapping(bindings::Microsoft::UI::Xaml::TextWrapping::Wrap);
        let handle = AnyView::from(text_box.clone());
        let this = Self {
            handle,
            text_box,
            on_change: Rc::new(RefCell::new(None)),
        };
        {
            let callback = this.on_change.clone();
            let text_box_for_handler = this.text_box.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                if let Some(callback) = callback.borrow().as_ref() {
                    let text = text_box_for_handler
                        .Text()
                        .map(|value| value.to_string_lossy())
                        .unwrap_or_default();
                    callback(text);
                }
            }));
            let _ = this
                .text_box
                .TextChanged(&TextChangedEventHandler::new(move |_, _| {
                    invoke_ui_event_callback(callback_id);
                    Ok(())
                }));
        }
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    /// `TextBox.Text` assigned programmatically resets the caret/selection to the start, even when
    /// the text given is identical to what's already there — same issue as AppKit's
    /// `NSTextView.setString:` (see that backend's own `InnerTextArea::set_text` doc comment for
    /// the full rationale). The two-way `#[two_way] text` binding re-syncs *every* bound field on
    /// *every* model change, including the one this exact edit just caused, so without this guard
    /// typing a single character would immediately call this with that same character already
    /// applied, yanking the caret away mid-keystroke.
    pub(crate) fn set_text(&self, text: &str) {
        let current = self
            .text_box
            .Text()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        if current == text {
            return;
        }
        let _ = self.text_box.SetText(&HSTRING::from(text));
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }
}

/// Raw single-line `TextBox` — the exact same underlying XAML class `InnerTextArea` wraps above,
/// just without `SetAcceptsReturn(true)`/`SetTextWrapping(Wrap)` (see that struct's own doc
/// comment) — composed by `native_ui::TextBox`. Structurally mirrors
/// `elwindui-backend-appkit::inner::InnerTextBox`; unverified on this machine (no Windows
/// environment — see `docs/status/control_status.md`).
pub(crate) struct InnerTextBox {
    handle: AnyView,
    text_box: XamlTextBox,
    on_change: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
}

impl InnerTextBox {
    pub(crate) fn new() -> Self {
        let text_box = XamlTextBox::new().expect("TextBox::new");
        let handle = AnyView::from(text_box.clone());
        let this = Self {
            handle,
            text_box,
            on_change: Rc::new(RefCell::new(None)),
        };
        {
            let callback = this.on_change.clone();
            let text_box_for_handler = this.text_box.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                if let Some(callback) = callback.borrow().as_ref() {
                    let text = text_box_for_handler
                        .Text()
                        .map(|value| value.to_string_lossy())
                        .unwrap_or_default();
                    callback(text);
                }
            }));
            let _ = this
                .text_box
                .TextChanged(&TextChangedEventHandler::new(move |_, _| {
                    invoke_ui_event_callback(callback_id);
                    Ok(())
                }));
        }
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    /// Same value-compare guard as `InnerTextArea::set_text` — see that method's own doc comment.
    pub(crate) fn set_text(&self, text: &str) {
        let current = self
            .text_box
            .Text()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        if current == text {
            return;
        }
        let _ = self.text_box.SetText(&HSTRING::from(text));
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }

    pub(crate) fn set_placeholder(&self, text: &str) {
        let _ = self.text_box.SetPlaceholderText(&HSTRING::from(text));
    }

    pub(crate) fn set_read_only(&self, read_only: bool) {
        let _ = self.text_box.SetIsReadOnly(read_only);
    }

    /// `TextBox.MaxLength` is a native WinUI3 property (`0` = unlimited) — a direct improvement
    /// over AppKit's manual delegate-based truncation
    /// (`elwindui-backend-appkit::inner::NativeTextFieldDelegate`'s own doc comment) — this
    /// AppKit/WinUI3 asymmetry is recorded in `docs/status/control_status.md`.
    pub(crate) fn set_max_length(&self, max_length: Option<u32>) {
        let _ = self.text_box.SetMaxLength(max_length.unwrap_or(0) as i32);
    }

    pub(crate) fn set_text_alignment(&self, alignment: elwindui_core::ui::TextAlignment) {
        let _ = self
            .text_box
            .SetTextAlignment(xaml_text_alignment(alignment));
    }

    /// Submit-on-Enter — unlike AppKit (which needs
    /// `elwindui-backend-appkit::inner::InnerTextBox::set_on_submit`'s own
    /// `control:textView:doCommandBySelector:` workaround, see that method's own doc comment for
    /// why), WinUI3's own `TextBox.KeyDown` fires natively regardless of focus and needs no
    /// special-casing.
    pub(crate) fn set_on_submit(&self, callback: Box<dyn Fn()>) {
        let callback_id = register_ui_event_callback(Rc::new(move || callback()));
        let _ = self
            .text_box
            .KeyDown(&KeyEventHandler::new(move |_sender, args| {
                let Some(args) = args.cloned() else {
                    return Ok(());
                };
                let Ok(virtual_key) = args.Key() else {
                    return Ok(());
                };
                if virtual_key == VirtualKey::Enter {
                    invoke_ui_event_callback(callback_id);
                }
                Ok(())
            }));
    }
}

/// Raw `PasswordBox` + change-notification wiring — composed by `native_ui::PasswordBox`.
/// Structurally mirrors `elwindui-backend-appkit::inner::InnerPasswordBox`; unverified on this
/// machine (no Windows environment — see `docs/status/control_status.md`). Unlike
/// `InnerTextBox`/`InnerTextArea`, `PasswordBox` is a genuinely distinct XAML class (not the same
/// class configured differently), so there's no bare-name import collision to rename here — the
/// `PasswordBox as XamlPasswordBox` alias at the top of this file was chosen from the start.
pub(crate) struct InnerPasswordBox {
    handle: AnyView,
    password_box: XamlPasswordBox,
    on_change: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
}

impl InnerPasswordBox {
    pub(crate) fn new() -> Self {
        let password_box = XamlPasswordBox::new().expect("PasswordBox::new");
        let handle = AnyView::from(password_box.clone());
        let this = Self {
            handle,
            password_box,
            on_change: Rc::new(RefCell::new(None)),
        };
        {
            let callback = this.on_change.clone();
            let password_box_for_handler = this.password_box.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                if let Some(callback) = callback.borrow().as_ref() {
                    let password = password_box_for_handler
                        .Password()
                        .map(|value| value.to_string_lossy())
                        .unwrap_or_default();
                    callback(password);
                }
            }));
            // `PasswordChanged`'s event type is the plain `RoutedEventHandler` `Button.Click`
            // already uses (see `build.rs`'s own comment on this allow-list entry) — not
            // `TextChangedEventHandler`.
            let _ = this
                .password_box
                .PasswordChanged(&RoutedEventHandler::new(move |_, _| {
                    invoke_ui_event_callback(callback_id);
                    Ok(())
                }));
        }
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    /// Same value-compare guard as `InnerTextBox::set_text`/`InnerTextArea::set_text` — see those
    /// methods' own doc comments.
    pub(crate) fn set_password(&self, password: &str) {
        let current = self
            .password_box
            .Password()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        if current == password {
            return;
        }
        let _ = self.password_box.SetPassword(&HSTRING::from(password));
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }

    pub(crate) fn set_placeholder(&self, text: &str) {
        let _ = self.password_box.SetPlaceholderText(&HSTRING::from(text));
    }

    pub(crate) fn set_max_length(&self, max_length: Option<u32>) {
        let _ = self
            .password_box
            .SetMaxLength(max_length.unwrap_or(0) as i32);
    }

    /// `PasswordBox.PasswordRevealMode` is native (`Peek`/`Hidden`) — the full-support side of the
    /// asymmetry `elwindui-backend-appkit::inner::InnerPasswordBox::set_reveal_enabled`'s own doc
    /// comment describes (AppKit's `NSSecureTextField` has no equivalent and is a documented no-op
    /// there).
    pub(crate) fn set_reveal_enabled(&self, enabled: bool) {
        let mode = if enabled {
            bindings::Microsoft::UI::Xaml::Controls::PasswordRevealMode::Peek
        } else {
            bindings::Microsoft::UI::Xaml::Controls::PasswordRevealMode::Hidden
        };
        let _ = self.password_box.SetPasswordRevealMode(mode);
    }
}
