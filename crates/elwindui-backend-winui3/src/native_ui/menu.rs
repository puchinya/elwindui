//! The WinUI 3 peers behind `elwindui::ui::MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`
//! (Core-owned Components, Issue #313).
//!
//! Core owns every menu's logical state and item list. These peers wrap the existing
//! `crate::inner` menu objects and reconcile a pushed item list by peer identity with
//! `reconcile_ordered_peers`: the native entries always end in the pushed order, and kept entries
//! are left untouched whenever new ones only need appending.

use crate::inner::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuFlyout, InnerMenuItem};
use elwindui_core::graphics::IconSource;
use elwindui_core::ui::native::{
    MenuBarItemPeer, MenuBarPeer, MenuItemPeer, MenuPeer, reconcile_ordered_peers,
};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// [`MenuItemPeer`] over a live `MenuFlyoutItem`.
pub(crate) struct WinUi3MenuItemPeer {
    pub(crate) inner: InnerMenuItem,
}

impl WinUi3MenuItemPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenuItem::new(),
        }
    }
}

impl MenuItemPeer for WinUi3MenuItemPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_icon(&self, icon: Option<IconSource>) {
        self.inner.set_icon(icon);
    }
    fn set_shortcut(&self, shortcut: &str) {
        self.inner.set_shortcut(shortcut);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_select(&self, callback: Option<Box<dyn Fn()>>) {
        self.inner.set_on_select(callback);
    }
    fn dispose(&self) {
        self.inner.set_on_select(None);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn menu_item_inner(peer: &Rc<dyn MenuItemPeer>) -> &InnerMenuItem {
    &peer
        .as_any()
        .downcast_ref::<WinUi3MenuItemPeer>()
        .expect("a WinUI 3 menu holds WinUI 3 menu item peers")
        .inner
}

/// [`MenuPeer`]: a submenu's items, also the source of native context-menu flyouts.
pub(crate) struct WinUi3MenuPeer {
    inner: InnerMenu,
    current: RefCell<Vec<Rc<dyn MenuItemPeer>>>,
}

impl WinUi3MenuPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenu::new(),
            current: RefCell::new(Vec::new()),
        }
    }

    /// A fresh context-menu realization of the current items.
    pub(crate) fn create_flyout(&self) -> Result<InnerMenuFlyout, windows::core::Error> {
        self.inner.create_flyout()
    }
}

impl MenuPeer for WinUi3MenuPeer {
    fn set_items(&self, items: &[Rc<dyn MenuItemPeer>]) {
        let old = self.current.replace(items.to_vec());
        reconcile_ordered_peers(
            &old,
            items,
            |removed| self.inner.remove_item(menu_item_inner(removed)),
            |added| self.inner.add_item(menu_item_inner(added)),
        );
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// [`MenuBarItemPeer`] over a XAML `MenuBarItem`.
pub(crate) struct WinUi3MenuBarItemPeer {
    inner: InnerMenuBarItem,
}

impl WinUi3MenuBarItemPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenuBarItem::new(),
        }
    }
}

impl MenuBarItemPeer for WinUi3MenuBarItemPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_submenu(&self, submenu: Rc<dyn MenuPeer>) {
        let submenu = submenu
            .as_any()
            .downcast_ref::<WinUi3MenuPeer>()
            .expect("a WinUI 3 menu bar item takes a WinUI 3 menu peer");
        self.inner.set_submenu(&submenu.inner);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// [`MenuBarPeer`] over the per-window XAML `MenuBar`, installed by the Window peer.
pub(crate) struct WinUi3MenuBarPeer {
    pub(crate) inner: InnerMenuBar,
    current: RefCell<Vec<Rc<dyn MenuBarItemPeer>>>,
}

impl WinUi3MenuBarPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenuBar::new(),
            current: RefCell::new(Vec::new()),
        }
    }
}

fn menu_bar_item_inner(peer: &Rc<dyn MenuBarItemPeer>) -> &InnerMenuBarItem {
    &peer
        .as_any()
        .downcast_ref::<WinUi3MenuBarItemPeer>()
        .expect("a WinUI 3 menu bar holds WinUI 3 menu bar item peers")
        .inner
}

impl MenuBarPeer for WinUi3MenuBarPeer {
    fn set_items(&self, items: &[Rc<dyn MenuBarItemPeer>]) {
        let old = self.current.replace(items.to_vec());
        reconcile_ordered_peers(
            &old,
            items,
            |removed| self.inner.remove_item(menu_bar_item_inner(removed)),
            |added| self.inner.add_item(menu_bar_item_inner(added)),
        );
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Live WinUI 3 host checks for the single-parent rule (Issue #323), run from the hosted XAML
/// regression test with the WinUI 3 peer factory registered.
#[cfg(test)]
pub(crate) mod hosted_ownership_tests {
    use super::*;
    use crate::bindings::Microsoft::UI::Xaml::Controls::MenuFlyoutItemBase;
    use elwindui_core::ui::{
        __menu_bar_peer_of, __menu_item_peer_of, __menu_peer_of, ListExt, Menu, MenuBar,
        MenuBarItem, MenuBarItemExt, MenuItem, MenuItemExt,
    };
    use std::cell::Cell;

    fn bar_item_inner(item: &Rc<MenuBarItem>) -> InnerMenuBarItem {
        let peer = item.as_ui_element().__menu_bar_item_peer();
        menu_bar_item_inner(&peer).clone()
    }

    fn native_entries(item: &Rc<MenuBarItem>) -> Vec<MenuFlyoutItemBase> {
        let items = bar_item_inner(item).items_for_test();
        (0..items.Size().expect("Items.Size"))
            .map(|index| items.GetAt(index).expect("Items.GetAt"))
            .collect()
    }

    fn rejection(action: impl FnOnce()) -> String {
        let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
            .expect_err("adding an item owned by another live Menu must be rejected");
        error
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    /// Two realized native submenus A and B: adding A's item to B panics in Core before any
    /// native change (both XAML `MenuBarItem.Items` collections and the item's one peer are
    /// unchanged), and after `remove` from A the same native entry moves to B and still works.
    pub(crate) fn a_menu_item_in_another_native_menu_is_rejected_before_any_xaml_change() {
        let menu_a = Menu::new();
        let menu_b = Menu::new();
        let item = MenuItem::new();
        item.set_text("Shared");
        let selected = Rc::new(Cell::new(0));
        let selected_for_callback = Rc::clone(&selected);
        item.set_on_select(Box::new(move || {
            selected_for_callback.set(selected_for_callback.get() + 1)
        }));
        menu_a.add(item.clone());

        let bar = MenuBar::new();
        let bar_a = MenuBarItem::new();
        bar_a.set_text("A");
        bar_a.set_submenu(menu_a.clone());
        let bar_b = MenuBarItem::new();
        bar_b.set_text("B");
        bar_b.set_submenu(menu_b.clone());
        bar.add(bar_a.clone());
        bar.add(bar_b.clone());
        let _bar_peer = __menu_bar_peer_of(&*bar);

        let item_peer = __menu_item_peer_of(&*item);
        let native_item = menu_item_inner(&item_peer).xaml_base_for_test();
        assert_eq!(native_entries(&bar_a), vec![native_item.clone()]);
        assert!(native_entries(&bar_b).is_empty());

        // add and insert into B are both rejected by Core with the diagnostic.
        for message in [
            rejection(|| menu_b.add(item.clone())),
            rejection(|| ListExt::insert(&*menu_b, 0, item.clone())),
        ] {
            assert!(
                message.contains("this `MenuItem` already belongs to another `Menu`")
                    && message.contains("`remove`, `remove_at` or `clear`"),
                "Core diagnostic: {message}"
            );
        }
        assert_eq!((menu_a.len(), menu_b.len()), (1, 0));
        assert_eq!(
            native_entries(&bar_a),
            vec![native_item.clone()],
            "A's native entries are unchanged"
        );
        assert!(
            native_entries(&bar_b).is_empty(),
            "nothing was appended to B natively"
        );
        assert!(
            Rc::ptr_eq(&item_peer, &__menu_item_peer_of(&*item)),
            "the item keeps its one peer"
        );

        // Normal operation continues: activation reaches the item once.
        menu_item_inner(&item_peer).select();
        assert_eq!(selected.get(), 1);

        // Explicit move: A detaches the native entry, B attaches the same one.
        let item_dyn: Rc<dyn MenuItemExt> = item.clone();
        assert!(ListExt::remove(&*menu_a, &item_dyn));
        menu_b.add(item.clone());
        assert!(native_entries(&bar_a).is_empty());
        assert_eq!(native_entries(&bar_b), vec![native_item.clone()]);
        assert!(Rc::ptr_eq(&item_peer, &__menu_item_peer_of(&*item)));
        menu_item_inner(&item_peer).select();
        assert_eq!(selected.get(), 2);

        // A native context menu of B is still realized per open from the current items.
        let menu_b_peer = __menu_peer_of(&*menu_b);
        let _flyout = menu_b_peer
            .as_any()
            .downcast_ref::<WinUi3MenuPeer>()
            .expect("WinUI 3 menu peer")
            .create_flyout()
            .expect("context flyout realized from B's items");
    }
}
