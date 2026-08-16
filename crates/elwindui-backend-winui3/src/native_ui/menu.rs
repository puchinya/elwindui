//! `elwindui::ui::MenuBar`/`MenuBarItem`/`Menu`/`MenuItem` and their `ListExt` collections.

use crate::inner::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem};
use std::cell::RefCell;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuBarExt)]
pub struct MenuBar {
    pub(crate) inner: InnerMenuBar,
    /// The currently-installed children, in display order — the "before" side of `set_children`'s
    /// diff against its own new `children` argument (the "after" side), mirroring `TabView`'s own
    /// `entries`/reconciliation pattern. Also `items()`'s own backing storage (`ListExt` impl
    /// below) — trait-object-typed, mirroring `elwindui_backend_appkit::native_ui::MenuBar`'s own
    /// shape (see its `children` field's own doc comment).
    children: RefCell<Vec<Rc<dyn elwindui_core::ui::MenuBarItemExt>>>,
}

#[elwindui_macros::class]
impl MenuBar {
    fn construct() -> Self {
        Self {
            inner: InnerMenuBar::new(),
            children: RefCell::new(Vec::new()),
        }
    }

    /// Reconciles the native menu bar's installed items against `children` by `Rc` pointer
    /// identity (matching `TabView`'s own reconciliation convention) — an item present in both the
    /// old and new list is left alone; one only in the old list is removed; one only in the new
    /// list is added.
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<MenuBarItem>>) {
        let mut current = self.children.borrow_mut();
        current.retain(|old| {
            let keep = children.iter().any(|new| {
                Rc::ptr_eq(
                    old,
                    &(Rc::clone(new) as Rc<dyn elwindui_core::ui::MenuBarItemExt>),
                )
            });
            if !keep {
                self.inner
                    .remove_item(&downcast_menu_bar_item(&**old).inner);
            }
            keep
        });
        for item in &children {
            let item_ext = Rc::clone(item) as Rc<dyn elwindui_core::ui::MenuBarItemExt>;
            if !current.iter().any(|old| Rc::ptr_eq(old, &item_ext)) {
                self.inner.add_item(&item.inner);
                current.push(item_ext);
            }
        }
    }

    fn add_item(&self, item: &dyn elwindui_core::ui::MenuBarItemExt) {
        self.inner.add_item(&downcast_menu_bar_item(item).inner);
    }
    fn remove_item(&self, item: &dyn elwindui_core::ui::MenuBarItemExt) {
        self.inner.remove_item(&downcast_menu_bar_item(item).inner);
    }
    /// See `elwindui_core::ui::MenuBar::items`'s own doc comment.
    fn items(&self) -> &dyn elwindui_core::ui::ListExt<dyn elwindui_core::ui::MenuBarItemExt> {
        self
    }
}

pub(crate) fn downcast_menu_bar_item(item: &dyn elwindui_core::ui::MenuBarItemExt) -> &MenuBarItem {
    item.as_any()
        .downcast_ref::<MenuBarItem>()
        .expect("MenuBarExt: item must be this backend's MenuBarItem")
}

impl elwindui_core::ui::ListExt<dyn elwindui_core::ui::MenuBarItemExt> for MenuBar {
    fn add(&self, item: Rc<dyn elwindui_core::ui::MenuBarItemExt>) {
        self.inner.add_item(&downcast_menu_bar_item(&*item).inner);
        self.children.borrow_mut().push(item);
    }
    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::MenuBarItemExt>) {
        // WinUI3's `InnerMenuBar` has no positional insert exposed here — appended, then
        // reconciled into logical position via a fresh `set_children` pass (matching
        // `set_children`'s own reconciliation, not a real native reorder).
        self.inner.add_item(&downcast_menu_bar_item(&*item).inner);
        let mut children = self.children.borrow_mut();
        let index = index.min(children.len());
        children.insert(index, item);
    }
    fn remove(&self, item: &Rc<dyn elwindui_core::ui::MenuBarItemExt>) -> bool {
        let mut children = self.children.borrow_mut();
        let Some(pos) = children.iter().position(|old| Rc::ptr_eq(old, item)) else {
            return false;
        };
        self.inner
            .remove_item(&downcast_menu_bar_item(&*children[pos]).inner);
        children.remove(pos);
        true
    }
    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::MenuBarItemExt> {
        let mut children = self.children.borrow_mut();
        let item = children.remove(index);
        self.inner
            .remove_item(&downcast_menu_bar_item(&*item).inner);
        item
    }
    fn clear(&self) {
        let mut children = self.children.borrow_mut();
        for item in children.drain(..) {
            self.inner
                .remove_item(&downcast_menu_bar_item(&*item).inner);
        }
    }
    fn len(&self) -> usize {
        self.children.borrow().len()
    }
    fn is_empty(&self) -> bool {
        self.children.borrow().is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::MenuBarItemExt>> {
        self.children.borrow().clone()
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuBarItemExt)]
pub struct MenuBarItem {
    inner: InnerMenuBarItem,
}

#[elwindui_macros::class]
impl MenuBarItem {
    fn construct() -> Self {
        Self {
            inner: InnerMenuBarItem::new(),
        }
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    // `submenu.as_any()` — NOT `AsAny::as_any(&submenu)` or anything routed through a directly-
    // imported `AsAny` — is required here. See this file's own top-level `use` block comment and
    // `elwindui_backend_appkit::native_ui::MenuBarItem::set_submenu`'s doc comment for the full
    // "as-any hack" rationale.
    fn set_submenu(&self, submenu: Rc<dyn elwindui_core::ui::MenuExt>) {
        // `submenu` itself is dropped at the end of this call — the underlying native menu stays
        // alive regardless (retained by whatever it gets installed into).
        let submenu = submenu
            .as_any()
            .downcast_ref::<Menu>()
            .expect("MenuBarItemExt::set_submenu: submenu must be this backend's Menu");
        self.inner.set_submenu(&submenu.inner);
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuExt)]
pub struct Menu {
    inner: InnerMenu,
    /// See `MenuBar::children`'s doc comment — same reconciliation pattern and same
    /// trait-object-typed storage rationale (also `items()`'s backing storage, `ListExt` impl
    /// below).
    children: RefCell<Vec<Rc<dyn elwindui_core::ui::MenuItemExt>>>,
}

#[elwindui_macros::class]
impl Menu {
    fn construct() -> Self {
        Self {
            inner: InnerMenu::new(),
            children: RefCell::new(Vec::new()),
        }
    }

    /// See `MenuBar::set_children`'s doc comment — same reconciliation pattern.
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<MenuItem>>) {
        let mut current = self.children.borrow_mut();
        current.retain(|old| {
            let keep = children.iter().any(|new| {
                Rc::ptr_eq(
                    old,
                    &(Rc::clone(new) as Rc<dyn elwindui_core::ui::MenuItemExt>),
                )
            });
            if !keep {
                self.inner.remove_item(&downcast_menu_item(&**old).inner);
            }
            keep
        });
        for item in &children {
            let item_ext = Rc::clone(item) as Rc<dyn elwindui_core::ui::MenuItemExt>;
            if !current.iter().any(|old| Rc::ptr_eq(old, &item_ext)) {
                self.inner.add_item(&item.inner);
                current.push(item_ext);
            }
        }
    }

    fn add_item(&self, item: &dyn elwindui_core::ui::MenuItemExt) {
        self.inner.add_item(&downcast_menu_item(item).inner);
    }
    fn remove_item(&self, item: &dyn elwindui_core::ui::MenuItemExt) {
        self.inner.remove_item(&downcast_menu_item(item).inner);
    }
    /// See `elwindui_core::ui::Menu::items`'s own doc comment.
    fn items(&self) -> &dyn elwindui_core::ui::ListExt<dyn elwindui_core::ui::MenuItemExt> {
        self
    }

    #[inherent]
    pub fn create_flyout(
        &self,
    ) -> Result<crate::bindings::Microsoft::UI::Xaml::Controls::MenuFlyout, windows::core::Error> {
        self.inner.create_flyout()
    }
}

pub(crate) fn downcast_menu_item(item: &dyn elwindui_core::ui::MenuItemExt) -> &MenuItem {
    item.as_any()
        .downcast_ref::<MenuItem>()
        .expect("MenuExt: item must be this backend's MenuItem")
}

impl elwindui_core::ui::ListExt<dyn elwindui_core::ui::MenuItemExt> for Menu {
    fn add(&self, item: Rc<dyn elwindui_core::ui::MenuItemExt>) {
        self.inner.add_item(&downcast_menu_item(&*item).inner);
        self.children.borrow_mut().push(item);
    }
    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::MenuItemExt>) {
        // See `MenuBar`'s own `ListExt::insert` — same "append, then reconcile position" caveat.
        self.inner.add_item(&downcast_menu_item(&*item).inner);
        let mut children = self.children.borrow_mut();
        let index = index.min(children.len());
        children.insert(index, item);
    }
    fn remove(&self, item: &Rc<dyn elwindui_core::ui::MenuItemExt>) -> bool {
        let mut children = self.children.borrow_mut();
        let Some(pos) = children.iter().position(|old| Rc::ptr_eq(old, item)) else {
            return false;
        };
        self.inner
            .remove_item(&downcast_menu_item(&*children[pos]).inner);
        children.remove(pos);
        true
    }
    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::MenuItemExt> {
        let mut children = self.children.borrow_mut();
        let item = children.remove(index);
        self.inner.remove_item(&downcast_menu_item(&*item).inner);
        item
    }
    fn clear(&self) {
        let mut children = self.children.borrow_mut();
        for item in children.drain(..) {
            self.inner.remove_item(&downcast_menu_item(&*item).inner);
        }
    }
    fn len(&self) -> usize {
        self.children.borrow().len()
    }
    fn is_empty(&self) -> bool {
        self.children.borrow().is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::MenuItemExt>> {
        self.children.borrow().clone()
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuItemExt)]
pub struct MenuItem {
    inner: InnerMenuItem,
}

#[elwindui_macros::class]
impl MenuItem {
    fn construct() -> Self {
        Self {
            inner: InnerMenuItem::new(),
        }
    }

    fn text(&self) -> String {
        self.inner.text()
    }
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_shortcut(&self, key_equivalent: &str) {
        self.inner.set_shortcut(key_equivalent);
    }
    fn set_on_select(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_select(callback);
    }
    fn select(&self) {
        self.inner.select();
    }
}
