//! `elwindui::ui::TextArea`/`TextBox`/`PasswordBox` — the text-entry `*Ext` implementations.

use super::{NativeControl, base_accessibility_semantics};
use crate::AnyView;
use crate::inner::{InnerPasswordBox, InnerTextArea, InnerTextBox};
use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityRole,
};
use elwindui_core::ui::UIElementExt;
use std::cell::RefCell;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextAreaExt, inherits = crate::NativeControl)]
pub struct TextArea {
    inner: InnerTextArea,
    on_change: RefCell<Option<Box<dyn Fn(String)>>>,
}

#[elwindui_macros::class]
impl TextArea {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::SetText(text) => {
                self.set_text(&text);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// `#[two_way] text` (`TextArea`'s `#[class]` declaration) — the change-back half of the binding;
    /// `elwindui_core::ui::TextArea::set_text` is the model→widget half.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::TextInput,
                    &[
                        AccessibilityActionKind::SetText,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        semantics.value = Some(text.to_string());
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(callback);
        self.install_change_trampoline();
    }

    fn construct() -> Self {
        let inner = InnerTextArea::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            on_change: RefCell::new(None),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = base_accessibility_semantics(
            AccessibilityRole::TextInput,
            &[
                AccessibilityActionKind::SetText,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.value = Some(String::new());
        self.base.set_intrinsic_accessibility_semantics(semantics);
        // WinUI3's `TextBox` is a tab stop by default — see
        // docs/design/runtime/input_focus_design.md.
        self.set_tab_stop(true);
        self.install_change_trampoline();
    }

    #[inherent]
    fn install_change_trampoline(&self) {
        let owner: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("TextArea must be Rc-constructed before installing its text callback");
        let weak = Rc::downgrade(&owner);
        self.inner.set_on_change(Box::new(move |text| {
            let Some(owner) = weak.upgrade() else { return };
            let this = owner
                .as_any()
                .downcast_ref::<TextArea>()
                .expect("TextArea owner must downcast to TextArea");
            this.set_text(&text);
            let callback = this.on_change.borrow_mut().take();
            if let Some(callback) = callback {
                callback(text);
                let mut slot = this.on_change.borrow_mut();
                if slot.is_none() {
                    *slot = Some(callback);
                }
            }
        }));
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextBoxExt, inherits = crate::NativeControl)]
pub struct TextBox {
    inner: InnerTextBox,
    on_change: RefCell<Option<Box<dyn Fn(String)>>>,
}

#[elwindui_macros::class]
impl TextBox {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::SetText(text) => {
                self.set_text(&text);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// `#[two_way] text` (`TextBox`'s `#[class]` declaration) — the change-back half of the binding;
    /// `elwindui_core::ui::TextBox::set_text` is the model→widget half. Mirrors
    /// `TextArea::set_on_text_change` above.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(|| {
                base_accessibility_semantics(
                    AccessibilityRole::TextInput,
                    &[
                        AccessibilityActionKind::SetText,
                        AccessibilityActionKind::Focus,
                    ],
                )
            });
        semantics.value = Some(text.to_string());
        self.base.set_intrinsic_accessibility_semantics(semantics);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(callback);
        self.install_change_trampoline();
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
            on_change: RefCell::new(None),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = base_accessibility_semantics(
            AccessibilityRole::TextInput,
            &[
                AccessibilityActionKind::SetText,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.value = Some(String::new());
        self.base.set_intrinsic_accessibility_semantics(semantics);
        // WinUI3's `TextBox` is a tab stop by default — see
        // docs/design/runtime/input_focus_design.md.
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
        self.install_change_trampoline();
    }

    #[inherent]
    fn install_change_trampoline(&self) {
        let owner: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("TextBox must be Rc-constructed before installing its text callback");
        let weak = Rc::downgrade(&owner);
        self.inner.set_on_change(Box::new(move |text| {
            let Some(owner) = weak.upgrade() else { return };
            let this = owner
                .as_any()
                .downcast_ref::<TextBox>()
                .expect("TextBox owner must downcast to TextBox");
            this.set_text(&text);
            let callback = this.on_change.borrow_mut().take();
            if let Some(callback) = callback {
                callback(text);
                let mut slot = this.on_change.borrow_mut();
                if slot.is_none() {
                    *slot = Some(callback);
                }
            }
        }));
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::PasswordBoxExt, inherits = crate::NativeControl)]
pub struct PasswordBox {
    inner: InnerPasswordBox,
}

#[elwindui_macros::class]
impl PasswordBox {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// `#[two_way] password` (`PasswordBox`'s `#[class]` declaration) — the change-back half of the
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
        self.base
            .set_intrinsic_accessibility_semantics(base_accessibility_semantics(
                AccessibilityRole::SecureTextInput,
                &[AccessibilityActionKind::Focus],
            ));
        // WinUI3's `PasswordBox` is a tab stop by default — see
        // docs/design/runtime/input_focus_design.md.
        self.set_tab_stop(true);
    }
}
