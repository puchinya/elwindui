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
    /// Tabs removed while a list push of this TabView was still running (a content unmount hook
    /// re-entering `remove`). Their leases and listeners are released only once the outermost
    /// push has returned, i.e. after the backend retired every removed tab's content (Issue #323).
    /// Holding the `Rc`s also keeps them alive until then.
    pending_release: RefCell<Vec<Rc<dyn TabViewItemExt>>>,
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
            pending_release: RefCell::new(Vec::new()),
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

    /// Replaces every tab at once. Panics if `children` holds the same item twice (`ui_spec.md`).
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<TabViewItem>>) {
        let before = self.selected_key();
        let old = self.children.to_vec();
        let new: Vec<Rc<dyn TabViewItemExt>> = children
            .into_iter()
            .map(|item| item as Rc<dyn TabViewItemExt>)
            .collect();
        crate::ui::collections::assert_distinct_items(&new, "TabView");
        let added: Vec<Rc<dyn TabViewItemExt>> = new
            .iter()
            .filter(|new| !old.iter().any(|old| Rc::ptr_eq(old, new)))
            .cloned()
            .collect();
        // Every incoming tab is checked before anything changes: one conflict rejects the call.
        let owner = self.lease_owner();
        for item in &added {
            item.as_ui_element()
                .tab_view_lease()
                .check(&owner, "TabViewItem", "TabView");
        }
        let removed = old
            .iter()
            .any(|old| !new.iter().any(|new| Rc::ptr_eq(old, new)));
        for item in &added {
            item.as_ui_element().tab_view_lease().claim(&owner);
            self.attach_item(item);
        }
        self.children.replace_all(new);
        self.items_changed(before, removed);
        self.release(&old);
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

    /// This TabView, as the owner recorded in its tabs' single-parent leases (Issue #323).
    fn lease_owner(&self) -> Weak<dyn UIElementExt> {
        let owner = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("elwindui: a live TabView has an owner");
        Rc::downgrade(&owner)
    }

    /// Rejects `item` before any change when it is already in this list or belongs to another
    /// live `TabView`; otherwise claims it and installs its change listener.
    fn accept(&self, item: &Rc<dyn TabViewItemExt>) {
        self.children.assert_absent(item, "TabView");
        let owner = self.lease_owner();
        let lease = item.as_ui_element().tab_view_lease();
        lease.check(&owner, "TabViewItem", "TabView");
        lease.claim(&owner);
        self.attach_item(item);
    }

    /// Called after a removal's list push. Inside another push of this TabView (a reentrant
    /// removal from a content unmount hook) the backend may only have queued that list, so the
    /// removed tabs are parked; the outermost removal then releases every parked tab that this
    /// TabView still owns and that is absent from the final list, and drops its change listener.
    /// Kept tabs are untouched. No borrow is held while releasing.
    fn release(&self, removed: &[Rc<dyn TabViewItemExt>]) {
        self.pending_release
            .borrow_mut()
            .extend(removed.iter().cloned());
        self.release_parked();
    }

    /// Releases the parked tabs once no list push of this TabView is running.
    fn release_parked(&self) {
        if self.push_depth.get() != 0 {
            return;
        }
        let parked = std::mem::take(&mut *self.pending_release.borrow_mut());
        if parked.is_empty() {
            return;
        }
        crate::ui::collections::release_removed_items(
            &parked,
            &self.children.to_vec(),
            &self.lease_owner(),
            |item| item.as_ui_element().tab_view_lease(),
            |item| item.as_ui_element().set_change_listener(None),
        );
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
            // Tabs removed reentrantly during this push are retired by now.
            self.release_parked();
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
        self.accept(&item);
        let before = self.selected_key();
        self.children.add(item);
        self.items_changed(before, false);
    }
    fn insert(&self, index: usize, item: Rc<dyn TabViewItemExt>) {
        self.accept(&item);
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
        self.release(std::slice::from_ref(item));
        true
    }
    fn remove_at(&self, index: usize) -> Rc<dyn TabViewItemExt> {
        let before = self.selected_key();
        let item = self.children.remove_at(index);
        self.items_changed(before, true);
        self.release(std::slice::from_ref(&item));
        item
    }
    fn clear(&self) {
        let before = self.selected_key();
        // The snapshot keeps every removed tab alive until its content is retired.
        let removed_items = self.children.to_vec();
        let removed = !removed_items.is_empty();
        self.children.clear();
        self.items_changed(before, removed);
        self.release(&removed_items);
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
