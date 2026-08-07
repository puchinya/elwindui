//! `builtin::Dropdown` — the `DropdownExt` implementation. Rebuilds its native `NSPopUpButton`
//! item list from scratch on every `items` change (see `inner/dropdown.rs`'s own doc comment for
//! why a full rebuild, not incremental diffing like `TabView`/`MenuBar`, is the right call here).

use super::NativeControl;
use super::dropdown_item::DropdownItem;
use crate::AnyView;
use crate::inner::InnerDropdown;
use elwindui_core::ui::UIElementExt;
use std::cell::Cell;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::DropdownExt, inherits = crate::NativeControl)]
pub struct Dropdown {
    inner: InnerDropdown,
    children: elwindui_core::ui::ChildList<dyn elwindui_core::ui::DropdownItemExt>,
    selected_index: Cell<usize>,
}

#[elwindui_macros::class]
impl Dropdown {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_selected_index(&self, selected_index: usize) {
        self.selected_index.set(selected_index);
        self.inner.set_selected_index(selected_index);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_change(callback);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    /// See `elwindui_core::ui::Menu::items`'s own doc comment for why this returns a borrow, not
    /// an owned `Rc`.
    fn items(&self) -> &dyn elwindui_core::ui::ListExt<dyn elwindui_core::ui::DropdownItemExt> {
        self
    }

    fn construct() -> Self {
        let inner = InnerDropdown::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            children: elwindui_core::ui::ChildList::new(),
            selected_index: Cell::new(0),
        }
    }

    fn on_constructed(&self) {
        self.set_tab_stop(true);
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_selected_index_change(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_change(callback);
    }

    /// `#[content(items)]`'s bulk setter — `codegen` calls this directly with the concrete,
    /// already-constructed `DropdownItem` children (mirrors `native_ui::MenuBar::set_children`'s
    /// own signature exactly).
    #[inherent]
    pub fn set_items(&self, items: Vec<Rc<DropdownItem>>) {
        self.children.replace_all(
            items
                .into_iter()
                .map(|item| item as Rc<dyn elwindui_core::ui::DropdownItemExt>)
                .collect(),
        );
        self.sync_items();
    }

    #[inherent]
    fn sync_items(&self) {
        let texts: Vec<String> = self
            .children
            .to_vec()
            .iter()
            .map(|item| downcast_dropdown_item(&**item).text())
            .collect();
        self.inner.rebuild_items(&texts);
        // `rebuild_items` (`removeAllItems` + re-add) resets `NSPopUpButton`'s own selection, so
        // the previously-set `selected_index` must be reapplied afterward or every `items` mutation
        // would silently drop the current selection back to none.
        self.inner.set_selected_index(self.selected_index.get());
    }
}

fn downcast_dropdown_item(item: &dyn elwindui_core::ui::DropdownItemExt) -> &DropdownItem {
    item.as_any()
        .downcast_ref::<DropdownItem>()
        .expect("DropdownExt: item must be this backend's DropdownItem")
}

impl elwindui_core::ui::ListExt<dyn elwindui_core::ui::DropdownItemExt> for Dropdown {
    fn add(&self, item: Rc<dyn elwindui_core::ui::DropdownItemExt>) {
        self.children.add(item);
        self.sync_items();
    }
    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::DropdownItemExt>) {
        self.children.insert(index, item);
        self.sync_items();
    }
    fn remove(&self, item: &Rc<dyn elwindui_core::ui::DropdownItemExt>) -> bool {
        let removed = self.children.remove(item);
        if removed {
            self.sync_items();
        }
        removed
    }
    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::DropdownItemExt> {
        let item = self.children.remove_at(index);
        self.sync_items();
        item
    }
    fn clear(&self) {
        self.children.clear();
        self.sync_items();
    }
    fn len(&self) -> usize {
        self.children.len()
    }
    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::DropdownItemExt>> {
        self.children.to_vec()
    }
}
