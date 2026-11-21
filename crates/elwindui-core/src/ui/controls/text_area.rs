//! `elwindui::ui::TextArea` — multi-line text entry.

use super::*;
use crate::ui::native::{NativePeer, TextAreaPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::TextArea` — multi-line text input.
///
/// Declared once here as a Core-owned Component (Issue #313) over a lazily realized
/// [`TextAreaPeer`]. A user edit reaches Core through the peer's change callback (an echo of the
/// stored text is dropped); a programmatic `set_text` does not notify.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(two_way, text: String)]
pub struct TextArea {
    text: RefCell<String>,
    on_change: RefCell<Option<Rc<dyn Fn(String)>>>,
    peer: OnceCell<Rc<dyn TextAreaPeer>>,
}

#[elwindui_macros::component]
impl TextArea {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            // An assistive-technology edit is a user change: store it, push it to the widget,
            // and notify once if it changed the text.
            AccessibilityAction::SetText(text) => {
                let changed = *self.text.borrow() != text;
                self.set_text(&text);
                if changed {
                    notify_change(&self.on_change, text);
                }
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("TextArea", |factory| factory.create_text_area());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a TextArea's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: push the text, then install the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &TextArea, text: String| this.on_native_change(text),
        )));
    }

    /// Programmatic: updates the display without notifying `on_change`.
    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_text(text);
        }
        self.sync_value_semantics();
    }

    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.set_on_change(callback);
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            text: RefCell::new(String::new()),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = native_semantics(
            AccessibilityRole::TextInput,
            &[
                AccessibilityActionKind::SetText,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.value = Some(String::new());
        self.base.set_intrinsic_accessibility_semantics(semantics);
        self.set_tab_stop(true);
    }
}

impl TextArea {
    /// A user edit reported by the peer (§5 two-way rule).
    fn on_native_change(&self, text: String) {
        if *self.text.borrow() == text {
            return;
        }
        *self.text.borrow_mut() = text.clone();
        self.sync_value_semantics();
        notify_change(&self.on_change, text);
    }

    fn sync_value_semantics(&self) {
        let text = self.text.borrow().clone();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.value = Some(text);
        });
    }
}
