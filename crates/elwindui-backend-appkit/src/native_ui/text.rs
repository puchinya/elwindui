//! `elwindui::ui::TextArea`/`TextBox`/`PasswordBox` — the text-entry `*Ext` implementations.

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
    /// Overrides `NativeControl::measure_override`'s generic `fittingSize()`-based measurement —
    /// see `InnerTextArea::measure`'s own doc comment for why `TextArea` specifically can't share
    /// that path (its handle is an `NSScrollView`, whose `fittingSize()` doesn't reflect the
    /// wrapped `NSTextView`'s natural size). Since this override bypasses `NativeControl`'s own
    /// `measure_override` entirely, it must call `sync_text_style()` itself first — every other
    /// `NativeControl` leaf (`Button`/`TextBox`/`PasswordBox`/`ScrollView`/`TabView`) gets it for
    /// free from the base.
    #[overrides]
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.base.sync_text_style();
        self.inner.refresh_default_size();
        self.inner.measure(available)
    }

    /// `#[two_way] text` (`TextArea`'s `#[class]` declaration) — the change-back half of the binding;
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
        // WinUI3's `TextBox`/AppKit's `NSTextField` are tab stops by default — see
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
    /// `#[two_way] text` (`TextBox`'s `#[class]` declaration) — the change-back half of the binding;
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
        // AppKit's `NSTextField`/WinUI3's `TextBox` are tab stops by default — see
        // docs/design/gui_framework_design.md §5.5.
        self.set_tab_stop(true);
        // Enter-key submit rides the ordinary inherited `on_key_down` (see
        // `elwindui_core::ui::TextBox`'s own doc comment on why this isn't a dedicated field) —
        // wired here, once, the same way `Button::on_constructed` wires `on_click`.
        // `InnerTextBox::set_on_submit` is the one narrowly-scoped AppKit addition that makes a
        // native `NSTextField`'s own Enter key actually reach this dispatch at all (AppKit doesn't
        // otherwise forward its own key handling into `on_key_down` — see
        // `docs/design/gui_framework_design.md` §5.5/§8.1's "known limitation" note).
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
    /// `#[two_way] password` (`PasswordBox`'s `#[class]` declaration) — the change-back half of the
    /// binding; `elwindui_core::ui::PasswordBox::set_password` is the model→widget half.
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
        // AppKit's `NSSecureTextField`/WinUI3's `PasswordBox` are tab stops by default — see
        // docs/design/gui_framework_design.md §5.5.
        self.set_tab_stop(true);
    }
}
