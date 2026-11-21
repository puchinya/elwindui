//! `elwindui::ui::MenuBarItem` — one top-level entry in a `MenuBar`.

use super::*;
use crate::ui::native::{MenuBarItemPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::MenuBarItem`, one top-level entry of a [`MenuBar`] with its submenu. Declared
/// once here as a Core-owned Component (Issue #313, `native_peer_design.md` §7c).
#[elwindui_macros::component(sealed)]
#[prop(text: String)]
#[content(submenu)]
#[prop(submenu: std::rc::Rc<dyn crate::ui::MenuExt>)]
pub struct MenuBarItem {
    self_weak: Weak<dyn MenuBarItemExt>,
    peer: OnceCell<Rc<dyn MenuBarItemPeer>>,
    text: RefCell<String>,
    submenu: RefCell<Option<Rc<dyn MenuExt>>>,
}

#[elwindui_macros::component]
impl MenuBarItem {
    fn set_text(&self, text: &str) {
        *self.as_ui_element().text.borrow_mut() = text.to_string();
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_text(text);
        }
    }

    fn set_submenu(&self, submenu: Rc<dyn MenuExt>) {
        *self.as_ui_element().submenu.borrow_mut() = Some(submenu.clone());
        if let Some(peer) = self.as_ui_element().peer.get().cloned() {
            peer.set_submenu(__menu_peer_of(&*submenu));
        }
    }

    fn construct() -> Self {
        Self {
            self_weak: __self_weak.clone(),
            peer: OnceCell::new(),
            text: RefCell::new(String::new()),
            submenu: RefCell::new(None),
        }
    }
}

impl MenuBarItem {
    /// This item as an owned trait object, while it is alive.
    pub(crate) fn self_rc(&self) -> Option<Rc<dyn MenuBarItemExt>> {
        self.self_weak.upgrade()
    }

    /// The caption.
    pub fn text(&self) -> String {
        self.text.borrow().clone()
    }

    /// Returns this entry's native peer, realizing it (and its submenu's) on first use.
    #[doc(hidden)]
    pub fn __menu_bar_item_peer(&self) -> Rc<dyn MenuBarItemPeer> {
        if let Some(peer) = self.peer.get() {
            return peer.clone();
        }
        let peer = create_native_peer("MenuBarItem", |factory| factory.create_menu_bar_item());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a MenuBarItem's native peer was realized more than once");
        }
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        let submenu = self.submenu.borrow().clone();
        if let Some(submenu) = submenu {
            peer.set_submenu(__menu_peer_of(&*submenu));
        }
        peer
    }
}
