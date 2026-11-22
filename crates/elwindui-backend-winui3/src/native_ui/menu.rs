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
