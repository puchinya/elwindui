//! The AppKit peers behind `elwindui::ui::MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`
//! (Core-owned Components, Issue #313).
//!
//! Core owns every menu's logical state and item list. These peers wrap the existing
//! `crate::inner` menu objects and reconcile a pushed item list by peer identity, the same way
//! the backend types did before: an entry present in both lists stays, one only in the old list
//! is removed, one only in the new list is appended.

use crate::inner::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem};
use elwindui_core::graphics::IconSource;
use elwindui_core::ui::native::{MenuBarItemPeer, MenuBarPeer, MenuItemPeer, MenuPeer};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// [`MenuItemPeer`] over an `NSMenuItem`. Its target is retained by the peer, so the Core
/// `MenuItem` that owns the peer keeps the native action alive.
pub(crate) struct AppKitMenuItemPeer {
    pub(crate) inner: InnerMenuItem,
}

impl AppKitMenuItemPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenuItem::new(),
        }
    }
}

impl MenuItemPeer for AppKitMenuItemPeer {
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
        .downcast_ref::<AppKitMenuItemPeer>()
        .expect("an AppKit menu holds AppKit menu item peers")
        .inner
}

fn same_peer<T: ?Sized>(a: &Rc<T>, b: &Rc<T>) -> bool {
    Rc::as_ptr(a) as *const () == Rc::as_ptr(b) as *const ()
}

/// [`MenuPeer`]: a submenu's items, also the source of native context-menu flyouts.
pub(crate) struct AppKitMenuPeer {
    inner: InnerMenu,
    current: RefCell<Vec<Rc<dyn MenuItemPeer>>>,
}

impl AppKitMenuPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenu::new(),
            current: RefCell::new(Vec::new()),
        }
    }

    /// The native `NSMenu`, returned to AppKit for a native context menu.
    pub(crate) fn ns(&self) -> objc2::rc::Retained<objc2_app_kit::NSMenu> {
        self.inner.ns()
    }
}

impl MenuPeer for AppKitMenuPeer {
    fn set_items(&self, items: &[Rc<dyn MenuItemPeer>]) {
        let old = self.current.replace(items.to_vec());
        for removed in old
            .iter()
            .filter(|old| !items.iter().any(|new| same_peer(old, new)))
        {
            self.inner.remove_item(menu_item_inner(removed));
        }
        for added in items
            .iter()
            .filter(|new| !old.iter().any(|old| same_peer(old, new)))
        {
            self.inner.add_item(menu_item_inner(added));
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// [`MenuBarItemPeer`] over a top-level `NSMenuItem`.
pub(crate) struct AppKitMenuBarItemPeer {
    inner: InnerMenuBarItem,
}

impl AppKitMenuBarItemPeer {
    pub(crate) fn new() -> Self {
        Self {
            inner: InnerMenuBarItem::new(),
        }
    }
}

impl MenuBarItemPeer for AppKitMenuBarItemPeer {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_submenu(&self, submenu: Rc<dyn MenuPeer>) {
        let submenu = submenu
            .as_any()
            .downcast_ref::<AppKitMenuPeer>()
            .expect("an AppKit menu bar item takes an AppKit menu peer");
        self.inner.set_submenu(&submenu.inner);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// [`MenuBarPeer`] over the application `NSMenu`, installed by the Window peer.
pub(crate) struct AppKitMenuBarPeer {
    pub(crate) inner: InnerMenuBar,
    current: RefCell<Vec<Rc<dyn MenuBarItemPeer>>>,
}

impl AppKitMenuBarPeer {
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
        .downcast_ref::<AppKitMenuBarItemPeer>()
        .expect("an AppKit menu bar holds AppKit menu bar item peers")
        .inner
}

impl MenuBarPeer for AppKitMenuBarPeer {
    fn set_items(&self, items: &[Rc<dyn MenuBarItemPeer>]) {
        let old = self.current.replace(items.to_vec());
        for removed in old
            .iter()
            .filter(|old| !items.iter().any(|new| same_peer(old, new)))
        {
            self.inner.remove_item(menu_bar_item_inner(removed));
        }
        for added in items
            .iter()
            .filter(|new| !old.iter().any(|old| same_peer(old, new)))
        {
            self.inner.add_item(menu_bar_item_inner(added));
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
