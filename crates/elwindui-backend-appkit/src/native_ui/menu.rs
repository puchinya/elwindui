//! `elwindui::ui::MenuBar`/`MenuBarItem`/`Menu`/`MenuItem` and their `ListExt` collections.

use crate::inner::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem};
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuBarExt)]
pub struct MenuBar {
    pub(crate) inner: InnerMenuBar,
    /// The currently-installed children, in display order — the "before" side of `set_children`'s
    /// diff against its own new `children` argument (the "after" side), mirroring `TabView`'s own
    /// `entries`/reconciliation pattern. Also `items()`'s own backing storage (`ListExt` impl
    /// below) — trait-object-typed (`Rc<dyn MenuBarItemExt>`, not the concrete `Rc<MenuBarItem>`
    /// this crate itself always actually constructs) to match `items()`'s `elwindui_core`-shared
    /// signature, the same way `UIElementCollection` stores `Rc<dyn UIElementExt>` rather than a
    /// concrete leaf type.
    pub(crate) children: elwindui_core::ui::ChildList<dyn elwindui_core::ui::MenuBarItemExt>,
}

#[elwindui_macros::class]
impl MenuBar {
    fn construct() -> Self {
        Self {
            inner: InnerMenuBar::new(),
            children: elwindui_core::ui::ChildList::new(),
        }
    }

    /// Reconciles the native menu bar's installed items against `children` by `Rc` pointer
    /// identity (matching `TabView`'s own reconciliation convention) — an item present in both the
    /// old and new list is left alone; one only in the old list is removed; one only in the new
    /// list is added.
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<MenuBarItem>>) {
        let mut current = self.children.to_vec();
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
        self.children.replace_all(current);
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

fn downcast_menu_bar_item(item: &dyn elwindui_core::ui::MenuBarItemExt) -> &MenuBarItem {
    item.as_any()
        .downcast_ref::<MenuBarItem>()
        .expect("MenuBarExt: item must be this backend's MenuBarItem")
}

impl elwindui_core::ui::ListExt<dyn elwindui_core::ui::MenuBarItemExt> for MenuBar {
    fn add(&self, item: Rc<dyn elwindui_core::ui::MenuBarItemExt>) {
        self.inner.add_item(&downcast_menu_bar_item(&*item).inner);
        self.children.add(item);
    }
    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::MenuBarItemExt>) {
        // AppKit's `InnerMenuBar` has no positional insert — appended, then reconciled into
        // logical position via a fresh `set_children` pass (matching `set_children`'s own
        // reconciliation, not a real native reorder).
        self.inner.add_item(&downcast_menu_bar_item(&*item).inner);
        self.children.insert(index, item);
    }
    fn remove(&self, item: &Rc<dyn elwindui_core::ui::MenuBarItemExt>) -> bool {
        if !self.children.remove(item) {
            return false;
        }
        self.inner
            .remove_item(&downcast_menu_bar_item(&**item).inner);
        true
    }
    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::MenuBarItemExt> {
        let item = self.children.remove_at(index);
        self.inner
            .remove_item(&downcast_menu_bar_item(&*item).inner);
        item
    }
    fn clear(&self) {
        for item in self.children.to_vec() {
            self.inner
                .remove_item(&downcast_menu_bar_item(&*item).inner);
        }
        self.children.clear();
    }
    fn len(&self) -> usize {
        self.children.len()
    }
    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::MenuBarItemExt>> {
        self.children.to_vec()
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
    // imported `AsAny` — is required here. `AsAny`'s blanket impl (`impl<T: Any> AsAny for T`) also
    // technically applies to `Rc<dyn MenuExt>` *itself* (any `'static` `Sized` type gets it, and an
    // `Rc` is always `Sized` even when its pointee isn't) — this is Rust's well-known "as-any"
    // gotcha (see e.g. lucumr.pocoo.org/2022/1/7/as-any-hack). Method resolution tries the
    // receiver's *own* type before dereferencing, so if `AsAny` is directly `use`-imported in this
    // file, `submenu.as_any()` resolves to *that* blanket impl on `Rc<dyn MenuExt>` — returning a
    // `dyn Any` for the `Rc` smart pointer itself, whose `downcast_ref::<Menu>()` then always fails
    // (confirmed empirically: same address, wrong `TypeId`, every time). Relying on `AsAny` being
    // reachable only as `MenuExt`'s own supertrait (not separately imported — see this file's own
    // top-level `use` block) makes method resolution skip straight past `Rc<dyn MenuExt>` (`AsAny`
    // isn't otherwise in scope for that unrelated type) to `dyn MenuExt` itself, correctly reaching
    // `Menu`'s own vtable slot.
    fn set_submenu(&self, submenu: Rc<dyn elwindui_core::ui::MenuExt>) {
        // `submenu` itself is dropped at the end of this call — the underlying native menu stays
        // alive regardless, retained natively once the submenu is set.
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
    children: elwindui_core::ui::ChildList<dyn elwindui_core::ui::MenuItemExt>,
}

#[elwindui_macros::class]
impl Menu {
    fn construct() -> Self {
        Self {
            inner: InnerMenu::new(),
            children: elwindui_core::ui::ChildList::new(),
        }
    }

    /// See `MenuBar::set_children`'s doc comment — same reconciliation pattern.
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<MenuItem>>) {
        let mut current = self.children.to_vec();
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
        self.children.replace_all(current);
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
}

fn downcast_menu_item(item: &dyn elwindui_core::ui::MenuItemExt) -> &MenuItem {
    item.as_any()
        .downcast_ref::<MenuItem>()
        .expect("MenuExt: item must be this backend's MenuItem")
}

impl elwindui_core::ui::ListExt<dyn elwindui_core::ui::MenuItemExt> for Menu {
    fn add(&self, item: Rc<dyn elwindui_core::ui::MenuItemExt>) {
        self.inner.add_item(&downcast_menu_item(&*item).inner);
        self.children.add(item);
    }
    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::MenuItemExt>) {
        // See `MenuBar`'s own `ListExt::insert` — same "append, then reconcile position" caveat.
        self.inner.add_item(&downcast_menu_item(&*item).inner);
        self.children.insert(index, item);
    }
    fn remove(&self, item: &Rc<dyn elwindui_core::ui::MenuItemExt>) -> bool {
        if !self.children.remove(item) {
            return false;
        }
        self.inner.remove_item(&downcast_menu_item(&**item).inner);
        true
    }
    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::MenuItemExt> {
        let item = self.children.remove_at(index);
        self.inner.remove_item(&downcast_menu_item(&*item).inner);
        item
    }
    fn clear(&self) {
        for item in self.children.to_vec() {
            self.inner.remove_item(&downcast_menu_item(&*item).inner);
        }
        self.children.clear();
    }
    fn len(&self) -> usize {
        self.children.len()
    }
    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::MenuItemExt>> {
        self.children.to_vec()
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
}
