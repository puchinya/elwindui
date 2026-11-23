//! `elwindui::ui::Menu` — a menu holding `MenuItem`s.

use super::*;
use crate::ui::native::{MenuPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::Menu`, an ordered list of [`MenuItem`]s used as a menu-bar submenu or a
/// context menu. Declared once here as a Core-owned Component (Issue #313,
/// `native_peer_design.md` §7c).
///
/// Core owns the item list. The native [`MenuPeer`] is realized when a backend first shows the
/// menu natively (a menu bar, a native context menu); every later list change is pushed to it as
/// the full ordered list of item peers, which the backend reconciles by identity.
#[elwindui_macros::component(sealed)]
#[content(items)]
#[prop(items: crate::ui::ListExt<dyn crate::ui::MenuItemExt>)]
pub struct Menu {
    /// This menu, as the owner recorded in its items' single-parent leases (Issue #323).
    self_weak: Weak<dyn MenuExt>,
    peer: OnceCell<Rc<dyn MenuPeer>>,
    items: ChildList<dyn MenuItemExt>,
}

#[elwindui_macros::component]
impl Menu {
    /// Appends `item` (Core's `MenuItem`).
    fn add_item(&self, item: &dyn MenuItemExt) {
        if let Some(item) = item.as_ui_element().self_rc() {
            ListExt::add(self.as_ui_element(), item);
        }
    }

    /// Removes `item` if present.
    fn remove_item(&self, item: &dyn MenuItemExt) {
        if let Some(item) = item.as_ui_element().self_rc() {
            ListExt::remove(self.as_ui_element(), &item);
        }
    }

    /// A live handle onto the menu's items. The DSL's `#[content(items)]` populates nested
    /// `MenuItem { .. }` children through it.
    fn items(&self) -> &dyn ListExt<dyn MenuItemExt> {
        self.as_ui_element()
    }

    fn construct() -> Self {
        Self {
            self_weak: __self_weak.clone(),
            peer: OnceCell::new(),
            items: ChildList::new(),
        }
    }
}

impl Menu {
    /// Returns this menu's native peer, realizing it (and every item's peer) on first use.
    #[doc(hidden)]
    pub fn __menu_peer(&self) -> Rc<dyn MenuPeer> {
        if let Some(peer) = self.peer.get() {
            return peer.clone();
        }
        let peer = create_native_peer("Menu", |factory| factory.create_menu());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a Menu's native peer was realized more than once");
        }
        self.push_items();
        peer
    }

    /// Pushes the current item list to a realized peer. The list is cloned out first: the peer
    /// may re-enter this menu.
    fn push_items(&self) {
        let Some(peer) = self.peer.get().cloned() else {
            return;
        };
        let items: Vec<Rc<dyn crate::ui::native::MenuItemPeer>> = self
            .items
            .to_vec()
            .iter()
            .map(|item| __menu_item_peer_of(&**item))
            .collect();
        peer.set_items(&items);
    }

    /// Rejects `item` before any change when it is already in this list or belongs to another
    /// live `Menu`; otherwise claims it for this menu.
    fn accept(&self, item: &Rc<dyn MenuItemExt>) {
        self.items.assert_absent(item, "Menu");
        let lease = item.as_ui_element().menu_lease();
        lease.check(&self.self_weak, "MenuItem", "Menu");
        lease.claim(&self.self_weak);
    }

    /// Releases the removed items still owned by this menu, after the push detached them.
    fn release(&self, removed: &[Rc<dyn MenuItemExt>]) {
        crate::ui::collections::release_removed_items(
            removed,
            &self.items.to_vec(),
            &self.self_weak,
            |item| item.as_ui_element().menu_lease(),
            |_| {},
        );
    }
}

impl ListExt<dyn MenuItemExt> for Menu {
    fn add(&self, item: Rc<dyn MenuItemExt>) {
        self.accept(&item);
        self.items.add(item);
        self.push_items();
    }
    fn insert(&self, index: usize, item: Rc<dyn MenuItemExt>) {
        self.accept(&item);
        self.items.insert(index, item);
        self.push_items();
    }
    fn remove(&self, item: &Rc<dyn MenuItemExt>) -> bool {
        let removed = self.items.remove(item);
        if removed {
            self.push_items();
            self.release(std::slice::from_ref(item));
        }
        removed
    }
    fn remove_at(&self, index: usize) -> Rc<dyn MenuItemExt> {
        let item = self.items.remove_at(index);
        self.push_items();
        self.release(std::slice::from_ref(&item));
        item
    }
    fn clear(&self) {
        // The snapshot keeps every removed item alive until its native entry is detached.
        let removed = self.items.to_vec();
        self.items.clear();
        self.push_items();
        self.release(&removed);
    }
    fn len(&self) -> usize {
        self.items.len()
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn MenuItemExt>> {
        self.items.to_vec()
    }
}

/// The realized native peer of a `Menu`, for backends presenting it natively (menu bars,
/// native context menus).
#[doc(hidden)]
pub fn __menu_peer_of(menu: &dyn MenuExt) -> Rc<dyn MenuPeer> {
    menu.as_ui_element().__menu_peer()
}
