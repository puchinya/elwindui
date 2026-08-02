//! `builtin::TextArea`/`TextBox`/`PasswordBox` — the text-entry `*Ext` implementations.

use super::NativeControl;
use crate::AnyView;
use crate::inner::{
    InnerPasswordBox, InnerTextArea, InnerTextBox,
};
use elwindui_core::ui::UIElementExt;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextAreaExt, inherits = crate::NativeControl)]
pub struct TextArea {
    inner: InnerTextArea,
}

#[elwindui_macros::class]
impl TextArea {
    /// `#[two_way] text` (`TextArea` in `builtins.elwind`) — the change-back half of the binding;
    /// `elwindui_core::ui::TextArea::set_text` is the model→widget half.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    fn construct() -> Self {
        let inner = InnerTextArea::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `TextBox` is a tab stop by default — see
        // docs/design/gui_framework_design.md §5.5.
        self.set_tab_stop(true);
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextBoxExt, inherits = crate::NativeControl)]
pub struct TextBox {
    inner: InnerTextBox,
}

#[elwindui_macros::class]
impl TextBox {
    /// `#[two_way] text` (`TextBox` in `builtins.elwind`) — the change-back half of the binding;
    /// `elwindui_core::ui::TextBox::set_text` is the model→widget half. Mirrors
    /// `TextArea::set_on_text_change` above.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }
    fn set_placeholder(&self, text: &str) {
        self.inner.set_placeholder(text);
    }
    fn set_read_only(&self, read_only: bool) {
        self.inner.set_read_only(read_only);
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.inner.set_max_length(max_length);
    }
    fn set_text_alignment(&self, alignment: elwindui_core::ui::TextAlignment) {
        self.inner.set_text_alignment(alignment);
    }

    fn construct() -> Self {
        let inner = InnerTextBox::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `TextBox` is a tab stop by default — see
        // docs/design/gui_framework_design.md §5.5.
        self.set_tab_stop(true);
        // Enter-key submit rides the ordinary inherited `on_key_down` — see
        // `elwindui_core::ui::TextBox`'s own doc comment on why this isn't a dedicated field, and
        // `InnerTextBox::set_on_submit`'s own doc comment on why WinUI3 (unlike AppKit) needs no
        // special-casing to make a focused `TextBox`'s own Enter key reach this at all.
        let node: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("TextBox::on_constructed: object must already be Rc-constructed");
        self.inner.set_on_submit(Box::new(move || {
            let args = elwindui_core::input::RoutedEventArgs::default();
            let key_args = elwindui_core::input::KeyEventArgs {
                key: elwindui_core::input::Key::Enter,
                modifiers: elwindui_core::input::KeyModifiers::default(),
                is_repeat: false,
            };
            elwindui_core::ui::dispatch_routed(&node, "on_key_down", &key_args, &args);
        }));
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::PasswordBoxExt, inherits = crate::NativeControl)]
pub struct PasswordBox {
    inner: InnerPasswordBox,
}

#[elwindui_macros::class]
impl PasswordBox {
    /// `#[two_way] password` (`PasswordBox` in `builtins.elwind`) — the change-back half of the
    /// binding; `elwindui_core::ui::PasswordBox::set_password` is the model→widget half. Mirrors
    /// `TextBox::set_on_text_change` above.
    #[inherent]
    pub fn set_on_password_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_password(&self, password: &str) {
        self.inner.set_password(password);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }
    fn set_placeholder(&self, text: &str) {
        self.inner.set_placeholder(text);
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.inner.set_max_length(max_length);
    }
    fn set_reveal_enabled(&self, enabled: bool) {
        self.inner.set_reveal_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerPasswordBox::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `PasswordBox` is a tab stop by default — see
        // docs/design/gui_framework_design.md §5.5.
        self.set_tab_stop(true);
    }
}
