//! `elwindui::ui::Dropdown` — a native, non-editable selection control (AppKit: `NSPopUpButton`;
//! WinUI3: `ComboBox`).

use super::*;
use crate::ui::native::{DropdownPeer, NativePeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::Dropdown`, a native non-editable selection control. Its content is a live,
/// ordered collection of [`DropdownItem`]s. `selected_index` is the single source of truth for
/// which item is selected (no per-item `selected` flag, see `docs/specs/ui_spec.md#dropdown`).
///
/// Declared once here as a Core-owned Component (Issue #313) over a lazily realized
/// [`DropdownPeer`]. Every item-list change pushes the item texts and the stored selection to the
/// peer. A user selection reaches Core through the peer's change callback: an echo of the stored
/// index is dropped, a new index is stored and notified once. A programmatic `set_selected_index`
/// does not notify.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[content(items)]
#[prop(items: Vec<std::rc::Rc<dyn crate::ui::DropdownItemExt>>)]
#[prop(two_way, selected_index: usize)]
#[prop(enabled: Option<bool>)]
pub struct Dropdown {
    items: ChildList<dyn DropdownItemExt>,
    selected_index: Cell<usize>,
    enabled: Cell<bool>,
    on_change: RefCell<Option<Rc<dyn Fn(usize)>>>,
    peer: OnceCell<Rc<dyn DropdownPeer>>,
}

#[elwindui_macros::component]
impl Dropdown {
    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("Dropdown", |factory| factory.create_dropdown());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a Dropdown's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: items before the selection, then the change callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        peer.set_items(&self.item_texts());
        peer.set_selected_index(self.selected_index.get());
        peer.set_enabled(self.enabled.get());
        peer.set_on_change(Some(weak_native_callback(
            self.as_ui_element(),
            |this: &Dropdown, index: usize| this.on_native_change(index),
        )));
    }

    fn items(&self) -> &dyn ListExt<dyn DropdownItemExt> {
        self
    }

    /// Programmatic: updates the display without notifying `on_change`.
    fn set_selected_index(&self, selected_index: usize) {
        self.selected_index.set(selected_index);
        if let Some(peer) = self.peer.get() {
            peer.set_selected_index(selected_index);
        }
        self.sync_value_semantics();
    }

    fn set_on_change(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        if let Some(peer) = self.peer.get() {
            peer.set_enabled(enabled);
        }
        sync_native_enabled(self.as_ui_element(), self.enabled.get());
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_selected_index_change(&self, callback: Box<dyn Fn(usize)>) {
        self.set_on_change(callback);
    }

    /// Replaces every item at once (the `#[content(items)]` bulk setter).
    #[inherent]
    pub fn set_items(&self, items: Vec<Rc<DropdownItem>>) {
        self.items.replace_all(
            items
                .into_iter()
                .map(|item| item as Rc<dyn DropdownItemExt>)
                .collect(),
        );
        self.items_changed();
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            items: ChildList::new(),
            selected_index: Cell::new(0),
            enabled: Cell::new(true),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        self.base
            .set_intrinsic_accessibility_semantics(native_semantics(
                AccessibilityRole::ComboBox,
                &[AccessibilityActionKind::Focus],
            ));
        self.set_tab_stop(true);
    }
}

impl Dropdown {
    fn item_texts(&self) -> Vec<String> {
        self.items
            .to_vec()
            .iter()
            .map(|item| item.as_ui_element().text())
            .collect()
    }

    /// Pushes the item texts, then the stored selection (a rebuilt native list loses it).
    fn items_changed(&self) {
        if let Some(peer) = self.peer.get().cloned() {
            peer.set_items(&self.item_texts());
            peer.set_selected_index(self.selected_index.get());
        }
        self.sync_value_semantics();
    }

    /// A user selection reported by the peer (§5 two-way rule).
    fn on_native_change(&self, index: usize) {
        if self.selected_index.get() == index {
            return;
        }
        self.selected_index.set(index);
        self.sync_value_semantics();
        notify_change(&self.on_change, index);
    }

    /// The accessibility value is the selected item's caption.
    fn sync_value_semantics(&self) {
        let value: Option<String> = self
            .items
            .to_vec()
            .get(self.selected_index.get())
            .map(|item| item.as_ui_element().text());
        let Some(mut semantics) = self.intrinsic_accessibility_semantics() else {
            return;
        };
        semantics.value = value;
        self.set_intrinsic_accessibility_semantics(semantics);
    }
}

impl ListExt<dyn DropdownItemExt> for Dropdown {
    fn add(&self, item: Rc<dyn DropdownItemExt>) {
        self.items.add(item);
        self.items_changed();
    }
    fn insert(&self, index: usize, item: Rc<dyn DropdownItemExt>) {
        self.items.insert(index, item);
        self.items_changed();
    }
    fn remove(&self, item: &Rc<dyn DropdownItemExt>) -> bool {
        let removed = self.items.remove(item);
        if removed {
            self.items_changed();
        }
        removed
    }
    fn remove_at(&self, index: usize) -> Rc<dyn DropdownItemExt> {
        let item = self.items.remove_at(index);
        self.items_changed();
        item
    }
    fn clear(&self) {
        self.items.clear();
        self.items_changed();
    }
    fn len(&self) -> usize {
        self.items.len()
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn DropdownItemExt>> {
        self.items.to_vec()
    }
}
