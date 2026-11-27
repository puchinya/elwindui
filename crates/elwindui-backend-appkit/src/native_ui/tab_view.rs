//! The AppKit peer behind `elwindui::ui::TabView` (a Core-owned Component, Issue #313).
//!
//! AppKit has no tabbed-document control, so this backend hand-rolls a chip strip
//! (`crate::inner::InnerTabView`). Each displayed tab owns a chip and a persistent content host;
//! selecting a tab shows its host and hides the previous one without touching either one's
//! content. Core owns the item list, the selected index and the selection policy; this peer owns
//! the chips, the hosts and which host is visible. Reorder, content-host retention and removal are
//! verified on macOS by the `native_peer_runtime` example and the #316 host run.

use super::control::HandlePeer;
use crate::host::TreeHost;
use crate::inner::{InnerTabView, TabChipImpl};
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::native::{NativePeer, TabViewPeer};
use elwindui_core::ui::{TabViewItemExt, UIElementExt};
use objc2::rc::Retained;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

type Slot<T> = RefCell<Option<Rc<dyn Fn(T)>>>;

/// Calls the slot's callback without holding the slot borrowed during the call.
fn invoke<T>(slot: &Slot<T>, value: T) {
    let callback = slot.borrow().clone();
    if let Some(callback) = callback {
        callback(value);
    }
}

/// [`TabViewPeer`] over the chip strip and its content container.
pub(crate) struct AppKitTabViewPeer {
    base: HandlePeer,
    state: Rc<TabViewState>,
}

struct TabViewState {
    inner: InnerTabView,
    /// The latest item list Core pushed.
    items: RefCell<Vec<Rc<dyn TabViewItemExt>>>,
    generation: Cell<u64>,
    rebuilding: Cell<bool>,
    /// Pointer identities of the displayed tabs, in display order, parallel to `chips`.
    displayed: RefCell<Vec<usize>>,
    chips: RefCell<Vec<(TabChipImpl, Retained<TreeHost>)>>,
    /// The key whose host is currently visible.
    visible: Cell<Option<usize>>,
    selected_index: Cell<usize>,
    on_select: Slot<usize>,
    on_close: Slot<usize>,
    on_new_tab: Slot<()>,
}

fn item_key(item: &Rc<dyn TabViewItemExt>) -> usize {
    Rc::as_ptr(item) as *const () as usize
}

impl AppKitTabViewPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerTabView::new();
        let base = HandlePeer::new(inner.handle());
        let state = Rc::new(TabViewState {
            inner,
            items: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            rebuilding: Cell::new(false),
            displayed: RefCell::new(Vec::new()),
            chips: RefCell::new(Vec::new()),
            visible: Cell::new(None),
            selected_index: Cell::new(0),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            on_new_tab: RefCell::new(None),
        });
        let weak = Rc::downgrade(&state);
        state.inner.set_on_new_tab(Box::new(move || {
            let state: Option<Rc<TabViewState>> = weak.upgrade();
            if let Some(state) = state {
                invoke(&state.on_new_tab, ());
            }
        }));
        Self { base, state }
    }
}

impl TabViewState {
    fn index_of(&self, key: usize) -> Option<usize> {
        self.items
            .borrow()
            .iter()
            .position(|item| item_key(item) == key)
    }

    /// A chip click: show the tab, then report it to Core.
    fn user_select(&self, key: usize) {
        let Some(index) = self.index_of(key) else {
            return;
        };
        self.selected_index.set(index);
        self.sync_selection();
        invoke(&self.on_select, index);
    }

    fn user_close(&self, key: usize) {
        if let Some(index) = self.index_of(key) {
            invoke(&self.on_close, index);
        }
    }

    fn rebuild(self: &Rc<Self>) {
        if self.rebuilding.replace(true) {
            return;
        }
        loop {
            let generation = self.generation.get();
            self.rebuild_once();
            if self.generation.get() == generation {
                break;
            }
        }
        self.rebuilding.set(false);
        self.sync_selection();
    }

    fn rebuild_once(self: &Rc<Self>) {
        let items = self.items.borrow().clone();
        let new_keys: Vec<usize> = items.iter().map(item_key).collect();
        // Bind the length first: a borrow in the range expression would live for the whole loop.
        let displayed_len = self.displayed.borrow().len();
        for i in (0..displayed_len).rev() {
            let key = self.displayed.borrow()[i];
            if !new_keys.contains(&key) {
                let (chip, host) = self.chips.borrow_mut().remove(i);
                self.displayed.borrow_mut().remove(i);
                self.inner.remove_tab(&chip, &host);
                if self.visible.get() == Some(key) {
                    self.visible.set(None);
                }
            }
        }
        for (target_index, (key, entry)) in new_keys.iter().zip(items.iter()).enumerate() {
            if self.displayed.borrow().contains(key) {
                continue;
            }
            let key = *key;
            let item = entry.as_ui_element();
            let on_select: Box<dyn Fn()> = {
                let weak: Weak<TabViewState> = Rc::downgrade(self);
                Box::new(move || {
                    let state: Option<Rc<TabViewState>> = weak.upgrade();
                    if let Some(state) = state {
                        state.user_select(key);
                    }
                })
            };
            let on_close: Box<dyn Fn()> = {
                let weak: Weak<TabViewState> = Rc::downgrade(self);
                Box::new(move || {
                    let state: Option<Rc<TabViewState>> = weak.upgrade();
                    if let Some(state) = state {
                        state.user_close(key);
                    }
                })
            };
            let insert_at = target_index.min(self.displayed.borrow().len());
            let (chip, host) = self.inner.insert_tab(
                insert_at,
                &item.header(),
                item.closable(),
                on_select,
                on_close,
            );
            self.chips
                .borrow_mut()
                .insert(insert_at, (chip, host.clone()));
            self.displayed.borrow_mut().insert(insert_at, key);
            if let Some(content) = item.content() {
                host.set_tree(content);
            }
        }
        // Keyed reorder: move each kept chip (with its host) to its pushed position, so the strip
        // order, `selected_index` and the visible host agree with Core's list.
        for (target_index, key) in new_keys.iter().enumerate() {
            let position = self.displayed.borrow().iter().position(|k| k == key);
            if let Some(position) = position.filter(|position| *position != target_index) {
                let entry = self.chips.borrow_mut().remove(position);
                self.inner.move_tab(&entry.0, target_index);
                self.chips.borrow_mut().insert(target_index, entry);
                let moved = self.displayed.borrow_mut().remove(position);
                self.displayed.borrow_mut().insert(target_index, moved);
            }
        }
        for (key, entry) in new_keys.iter().zip(items.iter()) {
            let position = self.displayed.borrow().iter().position(|k| k == key);
            if let Some(position) = position {
                let item = entry.as_ui_element();
                let chips = self.chips.borrow();
                chips[position].0.set_title(&item.header());
                chips[position].0.set_closable(item.closable());
            }
        }
    }

    /// Highlights the selected chip and shows only its content host.
    fn sync_selection(&self) {
        if self.rebuilding.get() {
            return;
        }
        let selected_key = self
            .items
            .borrow()
            .get(self.selected_index.get())
            .map(item_key);
        let displayed = self.displayed.borrow().clone();
        let chips = self.chips.borrow();
        for (position, key) in displayed.iter().enumerate() {
            chips[position].0.set_selected(Some(*key) == selected_key);
        }
        if self.visible.get() == selected_key {
            return;
        }
        if let Some(old) = self.visible.get() {
            if let Some(position) = displayed.iter().position(|k| *k == old) {
                self.inner
                    .set_tab_content_visible(&chips[position].1, false);
            }
        }
        let mut visible = None;
        if let Some(new) = selected_key {
            if let Some(position) = displayed.iter().position(|k| *k == new) {
                self.inner.set_tab_content_visible(&chips[position].1, true);
                visible = Some(new);
            }
        }
        self.visible.set(visible);
    }

    fn displayed_position(&self, index: usize) -> Option<usize> {
        let key = self.items.borrow().get(index).map(item_key)?;
        self.displayed.borrow().iter().position(|k| *k == key)
    }
}

impl NativePeer for AppKitTabViewPeer {
    fn handle(&self) -> &dyn Any {
        self.base.handle()
    }
    fn measure(&self, available: Size) -> Size {
        self.base.measure(available)
    }
    fn apply_background(&self, background: Option<&Brush>) -> bool {
        self.base.apply_background(background)
    }
    fn set_tooltip(&self, tooltip: Option<&str>) {
        self.base.set_tooltip(tooltip);
    }
    fn supports_text_style(&self) -> bool {
        self.base.supports_text_style()
    }
    fn apply_text_style(&self, style: &CascadedTextStyle) -> bool {
        self.base.apply_text_style(style)
    }
    fn dispose(&self) {
        *self.state.on_select.borrow_mut() = None;
        *self.state.on_close.borrow_mut() = None;
        *self.state.on_new_tab.borrow_mut() = None;
    }
}

#[cfg(all(feature = "native-peer-regression", target_os = "macos"))]
impl AppKitTabViewPeer {
    /// `(title, content host identity, host visible)` per chip, in the native strip's arranged
    /// order. Only the `native_peer_runtime` regression reads it.
    pub(crate) fn regression_snapshot(&self) -> Vec<(String, usize, bool)> {
        let chips = self.state.chips.borrow();
        let refs: Vec<&TabChipImpl> = chips.iter().map(|(chip, _)| chip).collect();
        self.state
            .inner
            .native_chip_order(&refs)
            .into_iter()
            .map(|index| {
                let (chip, host) = &chips[index];
                (
                    chip.title_button.native_button().title().to_string(),
                    Retained::as_ptr(host) as usize,
                    !host.isHidden(),
                )
            })
            .collect()
    }
}

impl TabViewPeer for AppKitTabViewPeer {
    fn set_items(&self, items: &[Rc<dyn TabViewItemExt>], owner: Option<Rc<dyn UIElementExt>>) {
        // The AppKit content hosts have no logical-parent link yet (unchanged by #313).
        let _ = owner;
        *self.state.items.borrow_mut() = items.to_vec();
        self.state.generation.set(self.state.generation.get() + 1);
        self.state.rebuild();
    }
    fn set_selected_index(&self, index: usize) {
        self.state.selected_index.set(index);
        self.state.sync_selection();
    }
    fn set_tab_header(&self, index: usize, header: &str) {
        if let Some(position) = self.state.displayed_position(index) {
            self.state.chips.borrow()[position].0.set_title(header);
        }
    }
    fn set_tab_closable(&self, index: usize, closable: bool) {
        if let Some(position) = self.state.displayed_position(index) {
            self.state.chips.borrow()[position].0.set_closable(closable);
        }
    }
    fn set_on_select(&self, callback: Option<Box<dyn Fn(usize)>>) {
        *self.state.on_select.borrow_mut() = callback.map(Rc::from);
    }
    fn set_on_close(&self, callback: Option<Box<dyn Fn(usize)>>) {
        *self.state.on_close.borrow_mut() = callback.map(Rc::from);
    }
    fn set_on_new_tab(&self, callback: Option<Box<dyn Fn()>>) {
        *self.state.on_new_tab.borrow_mut() = callback.map(|callback| {
            let callback: Rc<dyn Fn(())> = Rc::new(move |()| callback());
            callback
        });
    }
    /// The AppKit content hosts have no hosted-content retirement yet (unchanged by #313).
    fn unmount_content(&self) {}
    fn as_any(&self) -> &dyn Any {
        self
    }
}
