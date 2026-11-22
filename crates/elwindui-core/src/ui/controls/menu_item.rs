//! `elwindui::ui::MenuItem` — one entry in a `Menu`.

use super::*;
use crate::graphics::IconSource;
use crate::ui::native::{MenuItemPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::MenuItem`, one entry in a [`Menu`]. Declared once here as a Core-owned
/// Component (Issue #313, `native_peer_design.md` §7c).
///
/// Core keeps the caption, icon, shortcut, enabled state and `on_select` handler. A native menu
/// that shows the item realizes its [`MenuItemPeer`] once, through the backend factory, and
/// shares it with every other native menu showing the same item. A custom-rendered context menu
/// reads the logical state directly and never realizes a peer.
#[elwindui_macros::component(sealed)]
#[prop(text: String)]
#[prop(icon: Option<crate::graphics::IconSource>)]
#[prop(shortcut: Option<String>)]
#[prop(enabled: Option<bool>)]
#[prop(on_select: fn())]
pub struct MenuItem {
    self_weak: Weak<dyn MenuItemExt>,
    peer: OnceCell<Rc<dyn MenuItemPeer>>,
    text: RefCell<String>,
    icon: RefCell<Option<IconSource>>,
    shortcut: RefCell<Option<String>>,
    enabled: Cell<bool>,
    on_select: RefCell<Option<Rc<dyn Fn()>>>,
}

#[elwindui_macros::component]
impl MenuItem {
    fn text(&self) -> String {
        self.as_ui_element().text.borrow().clone()
    }

    fn set_text(&self, text: &str) {
        *self.as_ui_element().text.borrow_mut() = text.to_string();
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_text(text);
        }
    }

    fn icon(&self) -> Option<IconSource> {
        self.as_ui_element().icon.borrow().clone()
    }

    fn set_icon(&self, icon: Option<IconSource>) {
        *self.as_ui_element().icon.borrow_mut() = icon.clone();
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_icon(icon);
        }
    }

    fn enabled(&self) -> bool {
        self.as_ui_element().enabled.get()
    }

    fn set_enabled(&self, enabled: bool) {
        self.as_ui_element().enabled.set(enabled);
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_enabled(enabled);
        }
    }

    fn shortcut(&self) -> Option<String> {
        self.as_ui_element().shortcut.borrow().clone()
    }

    /// Sets the keyboard shortcut; `""` clears it.
    fn set_shortcut(&self, key_equivalent: &str) {
        *self.as_ui_element().shortcut.borrow_mut() =
            (!key_equivalent.is_empty()).then(|| key_equivalent.to_string());
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_shortcut(key_equivalent);
        }
    }

    fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.as_ui_element().on_select.borrow_mut() = Some(Rc::from(callback));
    }

    /// Runs the `on_select` handler, exactly as a user activation does.
    fn select(&self) {
        let callback = self.as_ui_element().on_select.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }

    fn construct() -> Self {
        Self {
            self_weak: __self_weak.clone(),
            peer: OnceCell::new(),
            text: RefCell::new(String::new()),
            icon: RefCell::new(None),
            shortcut: RefCell::new(None),
            enabled: Cell::new(true),
            on_select: RefCell::new(None),
        }
    }
}

impl MenuItem {
    /// This item as an owned trait object, while it is alive.
    pub(crate) fn self_rc(&self) -> Option<Rc<dyn MenuItemExt>> {
        self.self_weak.upgrade()
    }

    /// Returns this item's native peer, realizing it on first use: create, push the logical
    /// state, then install the activation callback last (§6). The callback holds only a `Weak`
    /// owner.
    ///
    /// # Panics
    ///
    /// Panics when no native backend is registered on this thread, or the backend has no menus.
    #[doc(hidden)]
    pub fn __menu_item_peer(&self) -> Rc<dyn MenuItemPeer> {
        if let Some(peer) = self.peer.get() {
            return peer.clone();
        }
        let peer = create_native_peer("MenuItem", |factory| factory.create_menu_item());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a MenuItem's native peer was realized more than once");
        }
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        let icon = self.icon.borrow().clone();
        if icon.is_some() {
            peer.set_icon(icon);
        }
        let shortcut = self.shortcut.borrow().clone();
        if let Some(shortcut) = shortcut {
            peer.set_shortcut(&shortcut);
        }
        peer.set_enabled(self.enabled.get());
        let owner = self.self_weak.clone();
        peer.set_on_select(Some(Box::new(move || {
            // Upgrade first and keep the owner alive for the whole call (§6 in-flight callbacks).
            let owner: Option<Rc<dyn MenuItemExt>> = owner.upgrade();
            if let Some(owner) = owner {
                owner.select();
            }
        })));
        peer
    }
}

impl Drop for MenuItem {
    fn drop(&mut self) {
        if let Some(peer) = self.peer.get() {
            peer.dispose();
        }
    }
}

/// The realized native peer of a `MenuItem`.
///
/// # Panics
///
/// Panics if `item` is not Core's `MenuItem` (the class is sealed, so this cannot happen for
/// values built through the public API), or when no native backend is registered.
#[doc(hidden)]
pub fn __menu_item_peer_of(item: &dyn MenuItemExt) -> Rc<dyn MenuItemPeer> {
    item.as_ui_element().__menu_item_peer()
}
