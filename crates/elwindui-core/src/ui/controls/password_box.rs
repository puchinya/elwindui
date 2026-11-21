//! `elwindui::ui::PasswordBox` — masked single-line entry.

use super::*;
use crate::ui::native::{NativePeer, PasswordBoxPeer, create_native_peer};
use std::cell::OnceCell;

/// Secure single-line text input (masked entry). Deliberately narrower than `TextBox`: no
/// `read_only`/`text_alignment` (neither platform's password widget meaningfully supports them the
/// way a text field does, and adding dead setters would misrepresent what's actually usable), and
/// no `selection_start`/`selection_length` (same rationale as `TextBox`, doubly so here — selection
/// semantics on obscured text are rarely product-relevant). The field/method is named `password`,
/// not `text`, everywhere (trait, `#[class]` declaration, backend structs) — a deliberate naming
/// divergence from `TextBox` so nothing can accidentally get routed through a code path that
/// assumes plaintext display is fine. See `docs/status/control_status.md` for the
/// `reveal_enabled` AppKit/WinUI3 asymmetry this control has (WinUI3's `PasswordRevealMode` is
/// native; AppKit's `NSSecureTextField` has no equivalent, so `true` is a documented no-op there).
///
/// Declared once here as a Core-owned Component (Issue #313) over a lazily realized
/// [`PasswordBoxPeer`]. The password is never exposed through accessibility semantics, `Debug`
/// or logs. A user edit reaches Core through the peer's change callback (an echo of the stored
/// password is dropped); a programmatic `set_password` does not notify.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(two_way, password: String)]
#[prop(placeholder: Option<String>)]
#[prop(max_length: Option<u32>)]
#[prop(reveal_enabled: Option<bool>)]
pub struct PasswordBox {
    password: RefCell<String>,
    placeholder: RefCell<String>,
    max_length: Cell<Option<u32>>,
    /// `None` until set: realization then keeps the platform's default reveal behavior.
    reveal_enabled: Cell<Option<bool>>,
    on_change: RefCell<Option<Rc<dyn Fn(String)>>>,
    peer: OnceCell<Rc<dyn PasswordBoxPeer>>,
}

#[elwindui_macros::component]
impl PasswordBox {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("PasswordBox", |factory| factory.create_password_box());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a PasswordBox's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: push the logical state, then install the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        let placeholder = self.placeholder.borrow().clone();
        peer.set_placeholder(&placeholder);
        peer.set_max_length(self.max_length.get());
        if let Some(enabled) = self.reveal_enabled.get() {
            peer.set_reveal_enabled(enabled);
        }
        let password = self.password.borrow().clone();
        peer.set_password(&password);
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &PasswordBox, password: String| this.on_native_change(password),
        )));
    }

    /// Programmatic: updates the field without notifying `on_change`.
    fn set_password(&self, password: &str) {
        *self.password.borrow_mut() = password.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_password(password);
        }
    }

    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }

    fn set_placeholder(&self, text: &str) {
        *self.placeholder.borrow_mut() = text.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_placeholder(text);
        }
    }

    fn set_max_length(&self, max_length: Option<u32>) {
        self.max_length.set(max_length);
        if let Some(peer) = self.peer.get() {
            peer.set_max_length(max_length);
        }
    }

    fn set_reveal_enabled(&self, enabled: bool) {
        self.reveal_enabled.set(Some(enabled));
        if let Some(peer) = self.peer.get() {
            peer.set_reveal_enabled(enabled);
        }
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_password_change(&self, callback: Box<dyn Fn(String)>) {
        self.set_on_change(callback);
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            password: RefCell::new(String::new()),
            placeholder: RefCell::new(String::new()),
            max_length: Cell::new(None),
            reveal_enabled: Cell::new(None),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        self.base
            .set_intrinsic_accessibility_semantics(native_semantics(
                AccessibilityRole::SecureTextInput,
                &[AccessibilityActionKind::Focus],
            ));
        self.set_tab_stop(true);
    }
}

impl PasswordBox {
    /// A user edit reported by the peer (§5 two-way rule).
    fn on_native_change(&self, password: String) {
        if *self.password.borrow() == password {
            return;
        }
        *self.password.borrow_mut() = password.clone();
        notify_change(&self.on_change, password);
    }
}
