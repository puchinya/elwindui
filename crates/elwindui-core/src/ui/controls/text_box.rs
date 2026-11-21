//! `elwindui::ui::TextBox` — single-line text entry.

use crate::ui::native::{NativePeer, TextBoxPeer, create_native_peer};
use std::cell::OnceCell;

use super::*;

/// Single-line text input — see `docs/status/control_status.md` for the wider
/// NativeControl expansion this is part of. Deliberately narrower than the original spec's
/// `TextBox` sketch: `selection_start`/`selection_length` are not included (AppKit's `NSTextField`
/// selection lives on its *field editor*, which only exists while actively being edited — a shared
/// method here would either be AppKit-only-half-working or need a `NativeControl`-wide "is there an
/// active editor right now" concept this codebase doesn't have yet; revisit once a second consumer
/// needs it). `submit`-on-Enter is likewise not a dedicated trait method — `UIElement`'s existing
/// `on_key_down` (`#[routed]`) already covers it the same way any other element's own key handling
/// would (see `TextBox`'s `#[class]` declaration and `native_ui::TextBox::on_constructed`'s own doc
/// comment on why AppKit needs one narrow, TextBox-specific addition to make that work in practice).
///
/// Declared once here as a Core-owned Component (Issue #313) over a lazily realized
/// [`TextBoxPeer`]. A user edit reaches Core through the peer's change callback (an echo of the
/// stored text is dropped); a programmatic `set_text` does not notify. Enter in the native field
/// dispatches the routed `on_key_down` (`Key::Enter`) from this node.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(two_way, text: String)]
#[prop(placeholder: Option<String>)]
#[prop(read_only: Option<bool>)]
#[prop(max_length: Option<u32>)]
#[prop(text_alignment: Option<crate::ui::TextAlignment>)]
pub struct TextBox {
    text: RefCell<String>,
    placeholder: RefCell<String>,
    read_only: Cell<bool>,
    max_length: Cell<Option<u32>>,
    /// `None` until set: realization then keeps the platform's default alignment.
    text_alignment: Cell<Option<TextAlignment>>,
    on_change: RefCell<Option<Rc<dyn Fn(String)>>>,
    peer: OnceCell<Rc<dyn TextBoxPeer>>,
}

#[elwindui_macros::component]
impl TextBox {
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
        let peer = create_native_peer("TextBox", |factory| factory.create_text_box());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a TextBox's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: push the logical state, then install the callbacks last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        let placeholder = self.placeholder.borrow().clone();
        peer.set_placeholder(&placeholder);
        peer.set_read_only(self.read_only.get());
        peer.set_max_length(self.max_length.get());
        if let Some(alignment) = self.text_alignment.get() {
            peer.set_text_alignment(alignment);
        }
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &TextBox, text: String| this.on_native_change(text),
        )));
        let submit = weak_native_callback(self.as_ui_element(), |this: &TextBox, ()| {
            this.dispatch_submit();
        });
        peer.set_on_submit(Some(Box::new(move || submit(()))));
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

    fn set_placeholder(&self, text: &str) {
        *self.placeholder.borrow_mut() = text.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_placeholder(text);
        }
    }

    fn set_read_only(&self, read_only: bool) {
        self.read_only.set(read_only);
        if let Some(peer) = self.peer.get() {
            peer.set_read_only(read_only);
        }
    }

    fn set_max_length(&self, max_length: Option<u32>) {
        self.max_length.set(max_length);
        if let Some(peer) = self.peer.get() {
            peer.set_max_length(max_length);
        }
    }

    fn set_text_alignment(&self, alignment: TextAlignment) {
        self.text_alignment.set(Some(alignment));
        if let Some(peer) = self.peer.get() {
            peer.set_text_alignment(alignment);
        }
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
            placeholder: RefCell::new(String::new()),
            read_only: Cell::new(false),
            max_length: Cell::new(None),
            text_alignment: Cell::new(None),
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

impl TextBox {
    /// A user edit reported by the peer (§5 two-way rule).
    fn on_native_change(&self, text: String) {
        if *self.text.borrow() == text {
            return;
        }
        *self.text.borrow_mut() = text.clone();
        self.sync_value_semantics();
        notify_change(&self.on_change, text);
    }

    /// Enter in the native field: the routed `on_key_down` with `Key::Enter`, from this node.
    fn dispatch_submit(&self) {
        let Some(node) = self.as_ui_element().visual_collection.owner_rc() else {
            return;
        };
        let args = crate::input::RoutedEventArgs::default();
        let key_args = crate::input::KeyEventArgs {
            key: crate::input::Key::Enter,
            modifiers: crate::input::KeyModifiers::default(),
            is_repeat: false,
        };
        crate::ui::dispatch_routed(&node, "on_key_down", &key_args, &args);
    }

    fn sync_value_semantics(&self) {
        let text = self.text.borrow().clone();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.value = Some(text);
        });
    }
}
