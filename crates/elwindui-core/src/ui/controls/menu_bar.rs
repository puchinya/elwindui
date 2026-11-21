//! `elwindui::ui::MenuBar` — the native application menu bar.

use super::*;
use crate::ui::native::{MenuBarItemPeer, MenuBarPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::MenuBar`, the window's native menu bar. Declared once here as a Core-owned
/// Component (Issue #313, `native_peer_design.md` §7c). Core owns the ordered
/// [`MenuBarItem`] list; the [`MenuBarPeer`] is realized when a realized `Window` installs it.
#[elwindui_macros::component(sealed)]
#[content(items)]
#[prop(items: crate::ui::ListExt<dyn crate::ui::MenuBarItemExt>)]
pub struct MenuBar {
    peer: OnceCell<Rc<dyn MenuBarPeer>>,
    items: ChildList<dyn MenuBarItemExt>,
}

#[elwindui_macros::component]
impl MenuBar {
    /// Appends `item` (Core's `MenuBarItem`).
    fn add_item(&self, item: &dyn MenuBarItemExt) {
        if let Some(item) = item.as_ui_element().self_rc() {
            ListExt::add(self.as_ui_element(), item);
        }
    }

    /// Removes `item` if present.
    fn remove_item(&self, item: &dyn MenuBarItemExt) {
        if let Some(item) = item.as_ui_element().self_rc() {
            ListExt::remove(self.as_ui_element(), &item);
        }
    }

    /// A live handle onto the top-level entries (see `Menu::items`).
    fn items(&self) -> &dyn ListExt<dyn MenuBarItemExt> {
        self.as_ui_element()
    }

    fn construct() -> Self {
        Self {
            peer: OnceCell::new(),
            items: ChildList::new(),
        }
    }
}

impl MenuBar {
    /// Returns this menu bar's native peer, realizing it (and every entry's) on first use.
    #[doc(hidden)]
    pub fn __menu_bar_peer(&self) -> Rc<dyn MenuBarPeer> {
        if let Some(peer) = self.peer.get() {
            return peer.clone();
        }
        let peer = create_native_peer("MenuBar", |factory| factory.create_menu_bar());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a MenuBar's native peer was realized more than once");
        }
        self.push_items();
        peer
    }

    fn push_items(&self) {
        let Some(peer) = self.peer.get().cloned() else {
            return;
        };
        let items: Vec<Rc<dyn MenuBarItemPeer>> = self
            .items
            .to_vec()
            .iter()
            .map(|item| item.as_ui_element().__menu_bar_item_peer())
            .collect();
        peer.set_items(&items);
    }
}

impl ListExt<dyn MenuBarItemExt> for MenuBar {
    fn add(&self, item: Rc<dyn MenuBarItemExt>) {
        self.items.add(item);
        self.push_items();
    }
    fn insert(&self, index: usize, item: Rc<dyn MenuBarItemExt>) {
        self.items.insert(index, item);
        self.push_items();
    }
    fn remove(&self, item: &Rc<dyn MenuBarItemExt>) -> bool {
        let removed = self.items.remove(item);
        if removed {
            self.push_items();
        }
        removed
    }
    fn remove_at(&self, index: usize) -> Rc<dyn MenuBarItemExt> {
        let item = self.items.remove_at(index);
        self.push_items();
        item
    }
    fn clear(&self) {
        self.items.clear();
        self.push_items();
    }
    fn len(&self) -> usize {
        self.items.len()
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn MenuBarItemExt>> {
        self.items.to_vec()
    }
}

/// The realized native peer of a `MenuBar`, for a realized Window installing it.
#[doc(hidden)]
pub fn __menu_bar_peer_of(menu_bar: &dyn MenuBarExt) -> Rc<dyn MenuBarPeer> {
    menu_bar.as_ui_element().__menu_bar_peer()
}
