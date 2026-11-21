//! `elwindui::ui::TabView` — multi-document tab container.

use super::*;
use crate::ui::native::{NativePeer, TabViewPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::TabView`, a multi-document tab container whose content is a live, ordered
/// collection of [`TabViewItem`]s. Dynamic child ranges update that collection directly.
///
/// Declared once here as a Core-owned Component (Issue #313, `native_peer_design.md` §7c). Core
/// owns the item list, `selected_index` and the selection policy after a removal
/// (`docs/specs/ui_spec.md` TabView): a surviving selected tab stays selected by identity; if the
/// selected tab itself was removed, the tab now at its index is selected, or the new last tab
/// when it was last. A changed selection is reported once, after the list change completes.
///
/// The lazily realized [`TabViewPeer`] owns each tab's nested content host and shows only the
/// selected one. Every list change pushes the full ordered list, then the stored selection. A
/// user selection is shown by the peer, then reported here: an echo of the stored index is
/// dropped, a new index is stored and notified once. A close request runs the tab's own
/// `on_close`, or else this TabView's `on_close(index)`.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[content(children)]
#[prop(children: Vec<std::rc::Rc<dyn crate::ui::TabViewItemExt>>)]
#[prop(two_way, selected_index: usize)]
#[prop(on_select: fn(usize))]
#[prop(on_new_tab: fn())]
pub struct TabView {
    children: ChildList<dyn TabViewItemExt>,
    selected_index: Cell<usize>,
    /// The last index the user callback / binding has seen, so a selection change is reported
    /// exactly once however it came about.
    reported_index: Cell<usize>,
    /// Nesting depth of list pushes: a list change made reentrantly from a peer push (a content
    /// unmount hook) only updates the list; the outermost push applies the final selection.
    push_depth: Cell<u32>,
    on_select: RefCell<Option<Rc<dyn Fn(usize)>>>,
    on_close: RefCell<Option<Rc<dyn Fn(usize)>>>,
    on_new_tab: RefCell<Option<Rc<dyn Fn()>>>,
    peer: OnceCell<Rc<dyn TabViewPeer>>,
}

#[elwindui_macros::component]
impl TabView {
    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("TabView", |factory| factory.create_tab_view());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a TabView's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: the items, the selection, then the callbacks last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        self.push_items();
        peer.set_on_select(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &TabView, index: usize| this.on_native_select(index),
        )));
        peer.set_on_close(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &TabView, index: usize| this.on_native_close(index),
        )));
        let on_new_tab = weak_native_callback(self.as_ui_element(), |this: &TabView, (): ()| {
            this.on_native_new_tab()
        });
        peer.set_on_new_tab(Some(Box::new(move || on_new_tab(()))));
    }

    fn children(&self) -> &dyn ListExt<dyn TabViewItemExt> {
        self
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            children: ChildList::new(),
            selected_index: Cell::new(0),
            reported_index: Cell::new(0),
            push_depth: Cell::new(0),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            on_new_tab: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    /// Tab content lives in nested hosts outside the Visual tree, so it is retired (popups first)
    /// when this TabView unmounts. WinUI 3's `TabView` is a tab stop by default
    /// (docs/design/runtime/input_focus_design.md).
    fn on_constructed(&self) {
        if let Some(owner) = self.as_ui_element().visual_collection.owner_rc() {
            let owner = Rc::downgrade(&owner);
            self.add_unmount_hook(Box::new(move || {
                let owner: Option<Rc<dyn UIElementExt>> = owner.upgrade();
                let Some(owner) = owner else { return };
                let Some(this) = owner.as_any().downcast_ref::<TabView>() else {
                    return;
                };
                let peer = this.peer.get().cloned();
                if let Some(peer) = peer {
                    peer.unmount_content();
                }
            }));
        }
        self.set_tab_stop(true);
    }

    /// Selects `selected_index` programmatically (no notification).
    #[inherent]
    pub fn set_selected_index(&self, selected_index: usize) {
        self.selected_index.set(selected_index);
        self.reported_index.set(selected_index);
        if let Some(peer) = self.peer.get().cloned() {
            peer.set_selected_index(selected_index);
        }
    }

    /// The selected index.
    #[inherent]
    pub fn selected_index(&self) -> usize {
        self.selected_index.get()
    }

    /// The user selection callback. It shares its slot with the `selected_index` binding
    /// write-back (`set_on_selected_index_change`).
    #[inherent]
    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_select.borrow_mut() = Some(Rc::from(callback));
    }

    /// Registers the write-back callback used by a two-way `selected_index` binding.
    #[inherent]
    pub fn set_on_selected_index_change(&self, callback: Box<dyn Fn(usize)>) {
        self.set_on_select(callback);
    }

    /// The fallback close handler, used for a tab without its own `on_close`.
    #[inherent]
    pub fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(Rc::from(callback));
    }

    /// The new-tab request handler (the native "+" affordance).
    #[inherent]
    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        *self.on_new_tab.borrow_mut() = Some(Rc::from(callback));
    }

    /// Replaces every tab at once.
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<TabViewItem>>) {
        let before = self.selected_key();
        let old = self.children.to_vec();
        let new: Vec<Rc<dyn TabViewItemExt>> = children
            .into_iter()
            .map(|item| item as Rc<dyn TabViewItemExt>)
            .collect();
        let removed = old
            .iter()
            .any(|old| !new.iter().any(|new| Rc::ptr_eq(old, new)));
        for item in &new {
            self.attach_item(item);
        }
        self.children.replace_all(new);
        self.items_changed(before, removed);
    }
}

fn item_key(item: &Rc<dyn TabViewItemExt>) -> *const () {
    Rc::as_ptr(item) as *const ()
}

impl TabView {
    /// The realized peer, realizing it first if needed. For backend-internal hosts and tests.
    #[doc(hidden)]
    pub fn __tab_view_peer(&self) -> Rc<dyn TabViewPeer> {
        self.__native_peer();
        self.peer.get().expect("realized above").clone()
    }

    fn selected_key(&self) -> Option<*const ()> {
        self.children
            .to_vec()
            .get(self.selected_index.get())
            .map(item_key)
    }

    /// Forwards header/closable changes of a displayed item to the peer.
    fn attach_item(&self, item: &Rc<dyn TabViewItemExt>) {
        let Some(owner) = self.as_ui_element().visual_collection.owner_rc() else {
            return;
        };
        let owner = Rc::downgrade(&owner);
        let key = item_key(item);
        item.as_ui_element()
            .set_change_listener(Some(Rc::new(move || {
                let owner: Option<Rc<dyn UIElementExt>> = owner.upgrade();
                let Some(owner) = owner else { return };
                if let Some(this) = owner.as_any().downcast_ref::<TabView>() {
                    this.item_changed(key);
                }
            })));
    }

    fn item_changed(&self, key: *const ()) {
        let children = self.children.to_vec();
        let Some(index) = children.iter().position(|item| item_key(item) == key) else {
            return;
        };
        let Some(peer) = self.peer.get().cloned() else {
            return;
        };
        let item = children[index].as_ui_element();
        peer.set_tab_header(index, &item.header());
        peer.set_tab_closable(index, item.closable());
    }

    /// Applies the selection policy after a list change, pushes the list and the selection, and
    /// reports a changed selection once.
    fn items_changed(&self, selected_before: Option<*const ()>, removed: bool) {
        if removed {
            if let Some(key) = selected_before {
                let children = self.children.to_vec();
                let survivor = children.iter().position(|item| item_key(item) == key);
                let replacement = survivor.or_else(|| {
                    (!children.is_empty())
                        .then(|| self.selected_index.get().min(children.len() - 1))
                });
                if let Some(index) = replacement {
                    self.selected_index.set(index);
                }
            }
        }
        self.push_items();
        if self.push_depth.get() == 0 {
            self.report_selection();
        }
    }

    /// Pushes the list, then (outermost push only) the latest stored selection.
    fn push_items(&self) {
        let Some(peer) = self.peer.get().cloned() else {
            return;
        };
        let items = self.children.to_vec();
        self.push_depth.set(self.push_depth.get() + 1);
        peer.set_items(&items, self.as_ui_element().visual_collection.owner_rc());
        self.push_depth.set(self.push_depth.get() - 1);
        if self.push_depth.get() == 0 {
            peer.set_selected_index(self.selected_index.get());
        }
    }

    fn report_selection(&self) {
        let index = self.selected_index.get();
        if self.reported_index.get() == index {
            return;
        }
        self.reported_index.set(index);
        notify_change(&self.on_select, index);
    }

    /// A user selection the peer already shows (§5 two-way rule).
    fn on_native_select(&self, index: usize) {
        if self.selected_index.get() == index {
            return;
        }
        self.selected_index.set(index);
        self.report_selection();
    }

    fn on_native_close(&self, index: usize) {
        let item = self.children.to_vec().get(index).cloned();
        let own = item.and_then(|item| item.as_ui_element().close_handler());
        match own {
            Some(handler) => handler(),
            None => notify_change(&self.on_close, index),
        }
    }

    fn on_native_new_tab(&self) {
        let callback = self.on_new_tab.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }
}

impl ListExt<dyn TabViewItemExt> for TabView {
    fn add(&self, item: Rc<dyn TabViewItemExt>) {
        self.attach_item(&item);
        let before = self.selected_key();
        self.children.add(item);
        self.items_changed(before, false);
    }
    fn insert(&self, index: usize, item: Rc<dyn TabViewItemExt>) {
        self.attach_item(&item);
        let before = self.selected_key();
        self.children.insert(index, item);
        self.items_changed(before, false);
    }
    fn remove(&self, item: &Rc<dyn TabViewItemExt>) -> bool {
        let before = self.selected_key();
        if !self.children.remove(item) {
            return false;
        }
        self.items_changed(before, true);
        true
    }
    fn remove_at(&self, index: usize) -> Rc<dyn TabViewItemExt> {
        let before = self.selected_key();
        let item = self.children.remove_at(index);
        self.items_changed(before, true);
        item
    }
    fn clear(&self) {
        let before = self.selected_key();
        let removed = !self.children.is_empty();
        self.children.clear();
        self.items_changed(before, removed);
    }
    fn len(&self) -> usize {
        self.children.len()
    }
    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn TabViewItemExt>> {
        self.children.to_vec()
    }
}
