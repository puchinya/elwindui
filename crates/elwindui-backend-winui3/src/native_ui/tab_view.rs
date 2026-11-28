//! The WinUI 3 peer behind `elwindui::ui::TabView` (a Core-owned Component, Issue #313).
//!
//! `Microsoft.UI.Xaml.Controls.TabView` is a real native tabbed-document control. Each displayed
//! tab's `Content` is a persistent `crate::host::TreeHost` holding that tab's whole widget tree —
//! recreating it on every resync would reset a document's `TextArea` (lost cursor/focus) on every
//! keystroke — so tabs are reconciled by item identity and only the selected tab's host takes
//! part in layout/render. Core owns the item list, the selected index and the selection policy;
//! this peer owns the hosts, their order and which one is active.

use super::control::HandlePeer;
use crate::host::TreeHost;
use crate::inner::{CallbackSlot, InnerTabView, invoke_slot, new_callback_slot as new_slot};
use elwindui_core::base::Size;
use elwindui_core::graphics::{Brush, CascadedTextStyle};
use elwindui_core::ui::native::{NativePeer, TabViewPeer};
use elwindui_core::ui::{TabViewItemExt, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// [`TabViewPeer`] over a XAML `TabView`. Created by [`super::WinUi3PeerFactory`].
pub(crate) struct WinUi3TabViewPeer {
    base: HandlePeer,
    pub(crate) state: Rc<TabViewState>,
}

/// The peer's state, shared (weakly) with the native event handlers.
pub(crate) struct TabViewState {
    pub(crate) inner: InnerTabView,
    /// The latest item list Core pushed.
    items: RefCell<Vec<Rc<dyn TabViewItemExt>>>,
    /// Bumped by every `set_items`, so a list pushed reentrantly during a rebuild is noticed.
    generation: Cell<u64>,
    /// The hosted content's logical parent (the TabView). Weak: the TabView owns this peer.
    owner: RefCell<Option<Weak<dyn UIElementExt>>>,
    /// Pointer identities of the items currently reflected as real XAML tabs, in display order —
    /// the "before" side of `rebuild`'s diff against `items` (the "after" side).
    displayed: RefCell<Vec<usize>>,
    /// Parallel to `displayed`: each real XAML tab's persistent content host, in display order.
    pub(crate) content_hosts: RefCell<Vec<TreeHost>>,
    /// Pointer identity of the entry whose host currently participates in layout/render. `None`
    /// means either no selected child exists or the selected index is out of range.
    pub(crate) active: Cell<Option<usize>>,
    /// Guards XAML collection edits that can synchronously raise selection/size events. Transient
    /// selection callbacks are suppressed, and host synchronization waits until `rebuild` has
    /// released its borrows.
    rebuilding: Cell<bool>,
    selected_index: Cell<usize>,
    on_select: CallbackSlot<usize>,
    on_close: CallbackSlot<usize>,
    on_new_tab: CallbackSlot<()>,
}

fn item_key(item: &Rc<dyn TabViewItemExt>) -> usize {
    Rc::as_ptr(item) as *const () as usize
}

impl WinUi3TabViewPeer {
    pub(crate) fn new() -> Self {
        let inner = InnerTabView::new();
        let base = HandlePeer::new(inner.handle());
        let state = Rc::new(TabViewState {
            inner,
            items: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            owner: RefCell::new(None),
            displayed: RefCell::new(Vec::new()),
            content_hosts: RefCell::new(Vec::new()),
            active: Cell::new(None),
            rebuilding: Cell::new(false),
            selected_index: Cell::new(0),
            on_select: new_slot(),
            on_close: new_slot(),
            on_new_tab: new_slot(),
        });
        let weak = Rc::downgrade(&state);
        state
            .inner
            .set_on_content_size_changed(Some(Rc::new(move |(width, height)| {
                let state: Option<Rc<TabViewState>> = weak.upgrade();
                if let Some(state) = state {
                    state.resize_active_content_host(width, height);
                }
            })));
        // A native (user) selection activates the selected tab's content host here, then reports
        // the index to Core. XAML raises transient selection changes synchronously while
        // `rebuild` edits `TabItems`; those are suppressed, and Core pushes the final selection
        // after the list change.
        let weak = Rc::downgrade(&state);
        state.inner.set_on_select(Some(Rc::new(move |index| {
            let state: Option<Rc<TabViewState>> = weak.upgrade();
            let Some(state) = state else { return };
            if state.rebuilding.get() {
                return;
            }
            state.selected_index.set(index);
            state.sync_active_content_host();
            invoke_slot(&state.on_select, index);
        })));
        let weak = Rc::downgrade(&state);
        state.inner.set_on_close(Some(Rc::new(move |index| {
            let state: Option<Rc<TabViewState>> = weak.upgrade();
            if let Some(state) = state {
                invoke_slot(&state.on_close, index);
            }
        })));
        let weak = Rc::downgrade(&state);
        state.inner.set_on_new_tab(Some(Rc::new(move |()| {
            let state: Option<Rc<TabViewState>> = weak.upgrade();
            if let Some(state) = state {
                invoke_slot(&state.on_new_tab, ());
            }
        })));
        Self { base, state }
    }

    #[cfg(test)]
    pub(crate) fn emit_select_for_test(&self, index: usize) {
        self.state.inner.emit_select_for_test(index);
    }

    #[cfg(test)]
    pub(crate) fn native_selected_index_for_test(&self) -> i32 {
        self.state.inner.native_selected_index_for_test()
    }
}

impl TabViewState {
    /// The displayed position of the item at Core `index`.
    fn displayed_position(&self, index: usize) -> Option<usize> {
        let key = self.items.borrow().get(index).map(item_key)?;
        self.displayed.borrow().iter().position(|k| *k == key)
    }

    /// Resizes only the host currently participating in layout/render.
    fn resize_active_content_host(&self, width: f64, height: f64) {
        if self.rebuilding.get() {
            return;
        }
        let Some(active_key) = self.active.get() else {
            return;
        };
        let host = {
            let displayed = self.displayed.borrow();
            let Some(index) = displayed.iter().position(|key| *key == active_key) else {
                return;
            };
            self.content_hosts.borrow().get(index).cloned()
        };
        if let Some(host) = host {
            self.inner.resize_content_host(&host, width, height);
        }
    }

    /// Deactivates the previously selected host, then sizes and activates the current one. Host
    /// clones are resolved before each state transition and no collection borrow is held across a
    /// relayout, avoiding `RefCell` re-entry if native focus events synchronously run user code.
    fn sync_active_content_host(&self) {
        let selected_key = self
            .items
            .borrow()
            .get(self.selected_index.get())
            .map(item_key);
        if self.active.get() == selected_key {
            return;
        }
        let old_host = self.active.get().and_then(|active_key| {
            let displayed = self.displayed.borrow();
            let index = displayed.iter().position(|key| *key == active_key)?;
            self.content_hosts.borrow().get(index).cloned()
        });
        if let Some(host) = old_host {
            host.set_active(false);
        }
        let new_host = selected_key.and_then(|selected_key| {
            let displayed = self.displayed.borrow();
            let index = displayed.iter().position(|key| *key == selected_key)?;
            self.content_hosts.borrow().get(index).cloned()
        });
        let active_key = if let (Some(selected_key), Some(host)) = (selected_key, new_host) {
            let (width, height) = self.inner.content_size();
            self.inner.resize_content_host(&host, width, height);
            host.set_active(true);
            Some(selected_key)
        } else {
            None
        };
        self.active.set(active_key);
    }

    /// Keyed diff (pointer identity): retires and removes displayed tabs whose item is gone,
    /// inserts a real XAML tab (+ a content host for the item's content) for each new item,
    /// moves kept tabs into the pushed order (keeping their hosts), and refreshes every tab's
    /// header and closable state. Repeats until no list was pushed reentrantly meanwhile.
    fn rebuild(&self) {
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
        self.sync_active_content_host();
    }

    fn rebuild_once(&self) {
        // Phase 1 — retire the content of every removed tab while its XAML TabViewItem and native
        // children are still attached: its popup closes, the content unmounts exactly once, and
        // only then are its native children detached (`TreeHost::unmount_hosted_tree`). No borrow
        // is held, because `on_unmount` may re-enter Core and push a new list; that list is
        // picked up by the next loop pass.
        let mut retired: Vec<usize> = Vec::new();
        loop {
            let pending: Vec<(usize, TreeHost)> = {
                let keys: Vec<usize> = self.items.borrow().iter().map(item_key).collect();
                let displayed = self.displayed.borrow();
                let hosts = self.content_hosts.borrow();
                displayed
                    .iter()
                    .zip(hosts.iter())
                    .filter(|(key, _)| !keys.contains(key) && !retired.contains(key))
                    .map(|(key, host)| (*key, host.clone()))
                    .collect()
            };
            if pending.is_empty() {
                break;
            }
            for (key, host) in pending {
                retired.push(key);
                host.unmount_hosted_tree();
            }
        }

        // Phase 2 — structural diff on a snapshot: remove the (already retired) XAML tabs, insert
        // new ones.
        let items = self.items.borrow().clone();
        let new_keys: Vec<usize> = items.iter().map(item_key).collect();
        // Bind the length first: a borrow in the range expression would live for the whole loop.
        let displayed_len = self.displayed.borrow().len();
        for i in (0..displayed_len).rev() {
            let key = self.displayed.borrow()[i];
            if !new_keys.contains(&key) {
                let host = self.content_hosts.borrow_mut().remove(i);
                host.set_active(false);
                self.inner.remove_tab_at(i);
                self.displayed.borrow_mut().remove(i);
                if self.active.get() == Some(key) {
                    self.active.set(None);
                }
            }
        }
        let owner = self.owner.borrow().as_ref().and_then(Weak::upgrade);
        for (target_index, (key, entry)) in new_keys.iter().zip(items.iter()).enumerate() {
            if self.displayed.borrow().contains(key) {
                continue;
            }
            let item = entry.as_ui_element();
            let insertion_index = target_index.min(self.displayed.borrow().len());
            let content_host =
                self.inner
                    .insert_tab(insertion_index, &item.header(), item.closable());
            self.content_hosts
                .borrow_mut()
                .insert(insertion_index, content_host.clone());
            self.displayed.borrow_mut().insert(insertion_index, *key);
            if let Some(content) = item.content() {
                content_host.set_hosted_tree(content, owner.clone());
            }
        }
        // Phase 3 — keyed reorder: move each kept tab (its `TabViewItem` and content host) to
        // its pushed position, so the native order, `selected_index` and the active host all
        // agree with Core's list. Positions before `target_index` are already final, so a tab is
        // only ever moved forward from a later position.
        for (target_index, key) in new_keys.iter().enumerate() {
            let position = self.displayed.borrow().iter().position(|k| k == key);
            if let Some(position) = position.filter(|position| *position != target_index) {
                self.inner.move_tab(position, target_index);
                let host = self.content_hosts.borrow_mut().remove(position);
                self.content_hosts.borrow_mut().insert(target_index, host);
                let moved = self.displayed.borrow_mut().remove(position);
                self.displayed.borrow_mut().insert(target_index, moved);
            }
        }
        for (key, entry) in new_keys.iter().zip(items.iter()) {
            let position = self.displayed.borrow().iter().position(|k| k == key);
            if let Some(position) = position {
                let item = entry.as_ui_element();
                self.inner.set_tab_title(position, &item.header());
                self.inner.set_tab_closable(position, item.closable());
            }
        }
    }
}

impl NativePeer for WinUi3TabViewPeer {
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

impl TabViewPeer for WinUi3TabViewPeer {
    fn set_items(&self, items: &[Rc<dyn TabViewItemExt>], owner: Option<Rc<dyn UIElementExt>>) {
        *self.state.items.borrow_mut() = items.to_vec();
        *self.state.owner.borrow_mut() = owner.as_ref().map(Rc::downgrade);
        self.state.generation.set(self.state.generation.get() + 1);
        self.state.rebuild();
    }
    /// Selects the matching XAML item and transfers layout/render participation to its
    /// persistent host. The native `SelectionChanged` this raises reaches Core as an echo.
    fn set_selected_index(&self, index: usize) {
        self.state.selected_index.set(index);
        self.state.inner.set_selected_index(index);
        if !self.state.rebuilding.get() {
            self.state.sync_active_content_host();
        }
    }
    fn set_tab_header(&self, index: usize, header: &str) {
        if let Some(position) = self.state.displayed_position(index) {
            self.state.inner.set_tab_title(position, header);
        }
    }
    fn set_tab_closable(&self, index: usize, closable: bool) {
        if let Some(position) = self.state.displayed_position(index) {
            self.state.inner.set_tab_closable(position, closable);
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
    /// Tab content lives in nested hosts outside the Visual tree, so it is retired (popups
    /// first) when the TabView unmounts — see `TreeHost::unmount_hosted_tree`.
    fn unmount_content(&self) {
        let hosts: Vec<TreeHost> = self.state.content_hosts.borrow().clone();
        for host in hosts {
            host.unmount_hosted_tree();
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Live XAML assertion helpers (Issue #157): removing a tab retires exactly that tab's content in
/// the order popup close → content unmount → native-children detach → XAML TabViewItem removal;
/// the TabView's own unmount never touches it again; and the selection stays consistent (native
/// `SelectedIndex`, `selected_index`, active host, user/binding callback) when the selected tab or
/// an earlier tab is removed. Run by `inner::button::hosted_xaml_regression_tests`.
#[cfg(test)]
pub(crate) mod hosted_lifecycle_tests {
    use super::*;
    use crate::host::hosted_context_lifecycle_tests::{
        Log, capture_events, count, open_recording_popup, probe_content,
    };
    use elwindui_core::ui::{ListExt, TabView, TabViewItem, TabViewItemExt};

    fn peer_of(tab_view: &TabView) -> Rc<dyn TabViewPeer> {
        tab_view.__tab_view_peer()
    }

    fn winui(peer: &Rc<dyn TabViewPeer>) -> &WinUi3TabViewPeer {
        peer.as_any()
            .downcast_ref::<WinUi3TabViewPeer>()
            .expect("WinUI 3 TabView peer")
    }

    fn hosts_of(tab_view: &TabView) -> Vec<TreeHost> {
        winui(&peer_of(tab_view))
            .state
            .content_hosts
            .borrow()
            .clone()
    }

    fn emit_select(tab_view: &TabView, index: usize) {
        winui(&peer_of(tab_view)).emit_select_for_test(index);
    }

    fn native_selected(tab_view: &TabView) -> i32 {
        winui(&peer_of(tab_view)).native_selected_index_for_test()
    }

    fn tab_view_with(log: &Log, names: &[&str]) -> (Rc<TabView>, Vec<Rc<dyn UIElementExt>>) {
        let tab_view = TabView::new();
        // Realize first, so every later list change reaches the native tabs immediately.
        peer_of(&tab_view);
        let contents: Vec<Rc<dyn UIElementExt>> =
            names.iter().map(|name| probe_content(log, name)).collect();
        for (name, content) in names.iter().zip(&contents) {
            let item = TabViewItem::new();
            item.set_header(name);
            item.set_content(Rc::clone(content));
            tab_view.add(item as Rc<dyn TabViewItemExt>);
        }
        (tab_view, contents)
    }

    fn take(log: &Log) -> Vec<String> {
        std::mem::take(&mut *log.borrow_mut())
    }

    /// The one retirement sequence every removed tab must produce, in this order.
    fn retirement(name: &str, with_popup: bool) -> Vec<String> {
        let mut expected = Vec::new();
        if with_popup {
            expected.push(format!("{name} popup close"));
        }
        expected.extend([
            "content retire".to_owned(),
            format!("{name} unmount"),
            "native children detach".to_owned(),
        ]);
        expected
    }

    pub(crate) fn tab_removal_retires_content_popup_first_exactly_once() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let _events = capture_events(&log);
        let (tab_view, contents) = tab_view_with(&log, &["c0", "c1", "c2", "c3"]);
        let hosts: Vec<TreeHost> = hosts_of(&tab_view);
        assert_eq!(hosts.len(), 4);
        assert!(contents.iter().all(|content| content.parent().is_some()));
        take(&log);

        // A native selection on an unbound TabView activates that tab's host (and only it).
        let selected = Rc::new(Cell::new(None::<usize>));
        let selected_for_callback = Rc::clone(&selected);
        tab_view.set_on_select(Box::new(move |index| {
            selected_for_callback.set(Some(index))
        }));
        emit_select(&tab_view, 2);
        assert!(hosts[2].is_active_for_test(), "selected tab host is active");
        assert!(
            !hosts[0].is_active_for_test(),
            "previous tab host is suppressed"
        );
        assert_eq!(selected.get(), Some(2), "user callback still runs");

        // (a)/(c) inactive tab with an open popup, before the selected tab (c3 at index 3): the
        // retirement happens while the XAML TabViewItem is still attached, and the selected tab
        // keeps its selection by identity although its index shifts to 2.
        emit_select(&tab_view, 3);
        selected.set(None);
        open_recording_popup(&hosts[1], &log, "c1");
        tab_view.remove_at(1);
        let mut expected = retirement("c1", true);
        expected.push("tab item remove".to_owned());
        assert_eq!(
            take(&log),
            expected,
            "popup → unmount → native detach → TabViewItem remove"
        );
        assert!(hosts[1].hosted_tree_for_test().is_none());
        assert!(
            contents[1].parent().is_none(),
            "removed content keeps no logical parent"
        );
        assert!(
            hosts[3].is_active_for_test(),
            "selected tab stays active after an earlier removal"
        );
        assert_eq!(tab_view.selected_index(), 2);
        assert_eq!(native_selected(&tab_view), 2);
        assert_eq!(selected.get(), Some(2), "binding is told the shifted index");

        // (b) clear removes several tabs; each is retired before any TabViewItem is removed.
        tab_view.clear();
        let events = take(&log);
        for name in ["c0", "c2", "c3"] {
            assert_eq!(count(&log, &format!("{name} unmount")), 0);
            let unmount = events.iter().position(|e| *e == format!("{name} unmount"));
            assert!(unmount.is_some(), "{name} unmounted by clear: {events:?}");
        }
        let first_remove = events.iter().position(|e| e == "tab item remove").unwrap();
        let last_unmount = events
            .iter()
            .rposition(|e| e.ends_with(" unmount"))
            .unwrap();
        assert!(
            last_unmount < first_remove,
            "all retirements precede TabViewItem removal: {events:?}"
        );

        // A tab added afterwards is unmounted with the TabView; earlier ones never again.
        let late = probe_content(&log, "c4");
        let item = TabViewItem::new();
        item.set_content(Rc::clone(&late));
        tab_view.add(item as Rc<dyn TabViewItemExt>);
        take(&log);
        let as_element: Rc<dyn UIElementExt> = tab_view.clone();
        elwindui_core::ui::unmount_subtree(&as_element);
        elwindui_core::ui::unmount_subtree(&as_element);
        let events = take(&log);
        assert_eq!(
            events.iter().filter(|e| *e == "c4 unmount").count(),
            1,
            "{events:?}"
        );
        assert!(events.iter().all(|e| !e.starts_with("c0")
            && !e.starts_with("c1")
            && !e.starts_with("c2")
            && !e.starts_with("c3")));
    }

    /// Issue #304: the native TabView hides its built-in `Ctrl+F4` accelerator tooltip, which
    /// otherwise appears over the whole tab content and lingers after a context menu closes.
    pub(crate) fn native_tab_view_hides_its_accelerator_tooltip() {
        use crate::bindings::Microsoft::UI::Xaml::Input::KeyboardAcceleratorPlacementMode;
        let tab_view = TabView::new();
        assert_eq!(
            winui(&peer_of(&tab_view))
                .state
                .inner
                .keyboard_accelerator_placement_mode_for_test(),
            KeyboardAcceleratorPlacementMode::Hidden
        );
    }

    /// Issue #332: the host containing a native TabView splices exactly the selected tab's
    /// nested host into its accessibility children, at the TabView's position; a non-selected
    /// or removed tab's host is never spliced, and a ScrollView inside a page is spliced into
    /// that page's host in turn (nested-in-nested).
    pub(crate) fn outer_accessibility_splices_only_the_selected_tab_host() {
        use crate::host::AccessibilityEntryForTest::{Nested, Semantic};
        use elwindui_core::ui::{
            LayoutExt, ScrollView, ScrollViewExt, TextBlock, TextBlockExt, VerticalLayout,
        };
        let text = |value: &str| -> Rc<dyn UIElementExt> {
            let block = TextBlock::new();
            block.set_text(value);
            block
        };
        let tab_view = TabView::new();
        peer_of(&tab_view);
        let scroll_view = ScrollView::new();
        let scrolled = text("scrolled");
        scroll_view.set_content(Rc::clone(&scrolled));
        let pages: Vec<Rc<dyn UIElementExt>> = vec![text("page a"), scroll_view.clone()];
        let items: Vec<Rc<TabViewItem>> = ["a", "b"]
            .iter()
            .zip(&pages)
            .map(|(name, page)| {
                let item = TabViewItem::new();
                item.set_header(name);
                item.set_content(Rc::clone(page));
                item
            })
            .collect();
        tab_view.set_children(items.clone());

        let (before, after) = (text("before"), text("after"));
        let root = VerticalLayout::new();
        LayoutExt::children(&*root).add(Rc::clone(&before));
        LayoutExt::children(&*root).add(tab_view.clone() as Rc<dyn UIElementExt>);
        LayoutExt::children(&*root).add(Rc::clone(&after));
        let outer = TreeHost::new();
        outer.set_tree(root as Rc<dyn UIElementExt>);
        let rebuild_all = || {
            outer.rebuild_accessibility_for_test();
            for host in hosts_of(&tab_view) {
                host.rebuild_accessibility_for_test();
            }
        };
        let expected = |host: &TreeHost| {
            vec![
                Semantic(before.accessibility_id().raw()),
                Nested(host.accessibility_identity_for_test()),
                Semantic(after.accessibility_id().raw()),
            ]
        };

        emit_select(&tab_view, 0);
        rebuild_all();
        let hosts = hosts_of(&tab_view);
        assert_eq!(
            outer.accessibility_entries_for_test(0),
            expected(&hosts[0]),
            "only the selected page, at the TabView's position"
        );
        assert_eq!(
            hosts[0].accessibility_entries_for_test(0),
            [Semantic(pages[0].accessibility_id().raw())]
        );

        emit_select(&tab_view, 1);
        rebuild_all();
        assert_eq!(
            outer.accessibility_entries_for_test(0),
            expected(&hosts[1]),
            "switching tabs swaps the spliced host"
        );
        let scroll_host =
            super::super::scroll_view::hosted_lifecycle_tests::content_host_of(&scroll_view);
        scroll_host.rebuild_accessibility_for_test();
        assert_eq!(
            hosts[1].accessibility_entries_for_test(0),
            [Nested(scroll_host.accessibility_identity_for_test())],
            "a ScrollView inside the page splices its own nested host"
        );
        assert_eq!(
            scroll_host.accessibility_entries_for_test(0),
            [Semantic(scrolled.accessibility_id().raw())]
        );

        tab_view.remove_at(1);
        rebuild_all();
        let remaining = hosts_of(&tab_view);
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            outer.accessibility_entries_for_test(0),
            expected(&remaining[0]),
            "a removed tab's host is gone and the replacement selection is spliced"
        );
        assert!(
            !outer
                .accessibility_entries_for_test(0)
                .contains(&Nested(hosts[1].accessibility_identity_for_test()))
        );
    }

    /// Review fix (PR #320): `set_children` with the existing items in a new order (and an
    /// insertion before a kept tab) moves the native tabs into Core's order, keeps every tab's
    /// content host and content (nothing is retired or re-hosted), and leaves the native
    /// selection and the active host on the tab Core's `selected_index` names.
    pub(crate) fn reordering_tabs_keeps_hosts_and_follows_core_order() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let tab_view = TabView::new();
        peer_of(&tab_view);
        let make = |name: &str| {
            let item = TabViewItem::new();
            item.set_header(name);
            item.set_content(probe_content(&log, name));
            item
        };
        let (a, b, c) = (make("a"), make("b"), make("c"));
        tab_view.set_children(vec![a.clone(), b.clone(), c.clone()]);
        let titles = || winui(&peer_of(&tab_view)).state.inner.tab_titles_for_test();
        assert_eq!(titles(), ["a", "b", "c"]);
        let host_elements = || {
            hosts_of(&tab_view)
                .iter()
                .map(TreeHost::as_element)
                .collect::<Vec<_>>()
        };
        let before = host_elements();
        emit_select(&tab_view, 2);
        take(&log);

        let assert_consistent = |label: &str| {
            let index = tab_view.selected_index();
            assert_eq!(native_selected(&tab_view), index as i32, "{label}: native");
            let hosts = hosts_of(&tab_view);
            assert!(hosts[index].is_active_for_test(), "{label}: active host");
            let active = hosts.iter().filter(|h| h.is_active_for_test()).count();
            assert_eq!(active, 1, "{label}: exactly one active host");
        };

        tab_view.set_children(vec![c.clone(), a.clone(), b.clone()]);
        assert_eq!(titles(), ["c", "a", "b"], "native order follows Core");
        assert_eq!(
            host_elements(),
            [before[2].clone(), before[0].clone(), before[1].clone()],
            "each tab keeps its own content host"
        );
        assert!(take(&log).is_empty(), "no content was retired or re-hosted");
        assert_consistent("reorder");

        let x = make("x");
        tab_view.set_children(vec![x.clone(), b.clone(), c.clone(), a.clone()]);
        assert_eq!(titles(), ["x", "b", "c", "a"], "insert before kept tabs");
        let hosts = host_elements();
        assert_eq!(hosts[1], before[1]);
        assert_eq!(hosts[2], before[2]);
        assert_eq!(hosts[3], before[0]);
        assert_consistent("insert + reorder");
    }

    /// Removing the selected tab itself selects the tab now at its index, or the new last tab;
    /// native `SelectedIndex`, `selected_index`, the active host and the user/binding callback agree;
    /// removing the only tab leaves an empty TabView with no active host.
    pub(crate) fn removing_the_selected_tab_selects_a_consistent_replacement() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let (tab_view, _contents) = tab_view_with(&log, &["s0", "s1", "s2"]);
        let hosts: Vec<TreeHost> = hosts_of(&tab_view);
        // A two-way binding: the callback writes the index back like generated code does.
        let reported = Rc::new(RefCell::new(Vec::<usize>::new()));
        let weak = Rc::downgrade(&tab_view);
        let reported_for_callback = Rc::clone(&reported);
        tab_view.set_on_select(Box::new(move |index| {
            reported_for_callback.borrow_mut().push(index);
            if let Some(tab_view) = weak.upgrade() {
                tab_view.set_selected_index(index);
            }
        }));
        let assert_selected = |index: usize, host: &TreeHost, label: &str| {
            assert_eq!(tab_view.selected_index(), index, "{label}: selected_index");
            assert_eq!(native_selected(&tab_view), index as i32, "{label}: native");
            assert!(
                host.is_active_for_test(),
                "{label}: replacement host active"
            );
        };

        // Selected last tab removed → new last tab.
        emit_select(&tab_view, 2);
        reported.borrow_mut().clear();
        tab_view.remove_at(2);
        assert_selected(1, &hosts[1], "selected last");
        assert_eq!(*reported.borrow(), [1], "binding told the replacement once");
        assert!(!hosts[0].is_active_for_test());

        // Selected middle tab removed → the tab now at its index (re-add one to have a middle).
        let item = TabViewItem::new();
        item.set_content(probe_content(&log, "s3"));
        tab_view.add(item as Rc<dyn TabViewItemExt>);
        let hosts: Vec<TreeHost> = hosts_of(&tab_view);
        emit_select(&tab_view, 1);
        reported.borrow_mut().clear();
        tab_view.remove_at(1);
        assert_selected(1, &hosts[2], "selected middle");
        assert!(
            reported.borrow().is_empty(),
            "same index: nothing to report"
        );

        // Only tab: remove until one is left, then remove it.
        tab_view.remove_at(0);
        assert_selected(0, &hosts_of(&tab_view)[0], "after removing first");
        let last_host = hosts_of(&tab_view)[0].clone();
        tab_view.remove_at(0);
        assert_eq!(tab_view.len(), 0);
        assert!(
            winui(&peer_of(&tab_view)).state.active.get().is_none(),
            "no active host in an empty TabView"
        );
        assert!(!last_host.is_active_for_test());
    }

    /// `TabView::remove_at` of a tab whose content A requests B on the tab's host from its
    /// `on_unmount`, and B requests C: the removed tab's host never re-attaches anything, ends
    /// empty, and A, B and C unmount exactly once.
    pub(crate) fn tab_removal_never_reattaches_reentrant_requests() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let (tab_view, contents) = tab_view_with(&log, &["A", "keep"]);
        let host = hosts_of(&tab_view)[0].clone();
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");
        let request = |content: &Rc<dyn UIElementExt>, next: &Rc<dyn UIElementExt>| {
            let host = host.clone();
            let next = Rc::clone(next);
            content.add_unmount_hook(Box::new(move || {
                host.set_hosted_tree(Rc::clone(&next), None);
            }));
        };
        request(&contents[0], &b);
        request(&b, &c);
        open_recording_popup(&host, &log, "A");

        tab_view.remove_at(0);

        assert!(
            host.hosted_tree_for_test().is_none(),
            "removed tab host stays empty"
        );
        assert_eq!(count(&log, "A popup close"), 1);
        for name in ["A", "B", "C"] {
            assert_eq!(
                count(&log, &format!("{name} unmount")),
                1,
                "{name} exactly once"
            );
        }
        assert_eq!(count(&log, "keep unmount"), 0, "the other tab is untouched");
        assert!(contents[0].parent().is_none() && b.parent().is_none() && c.parent().is_none());
    }

    /// One `Rc` set as the content of two tabs of the same TabView (one owner, two hosts): only
    /// the first tab's host publishes it; the second host stays empty. Selection works; removing
    /// either tab first leaves no host publishing an unmounted tree, and B unmounts exactly once.
    pub(crate) fn shared_content_is_published_by_one_tab_host_only() {
        for remove_owning_first in [true, false] {
            let label = if remove_owning_first {
                "remove owner tab first"
            } else {
                "remove other tab first"
            };
            let log: Log = Rc::new(RefCell::new(Vec::new()));
            let tab_view = TabView::new();
            peer_of(&tab_view);
            let b = probe_content(&log, "B");
            for name in ["T1", "T2"] {
                let item = TabViewItem::new();
                item.set_header(name);
                item.set_content(Rc::clone(&b));
                tab_view.add(item as Rc<dyn TabViewItemExt>);
            }
            let hosts: Vec<TreeHost> = hosts_of(&tab_view);
            let publishes_b = |host: &TreeHost| {
                host.hosted_tree_for_test()
                    .is_some_and(|tree| Rc::ptr_eq(&tree, &b))
            };
            assert!(publishes_b(&hosts[0]), "{label}: first tab host owns B");
            assert!(
                hosts[1].hosted_tree_for_test().is_none(),
                "{label}: second host stays empty"
            );
            let owner: Rc<dyn UIElementExt> = tab_view.clone();
            assert!(
                b.parent().is_some_and(|parent| std::ptr::addr_eq(
                    Rc::as_ptr(&parent),
                    Rc::as_ptr(&owner)
                )),
                "{label}: B linked to the TabView"
            );
            assert!(
                b.as_ui_element().invalidate_host.borrow().is_some(),
                "{label}: B wired"
            );
            emit_select(&tab_view, 1);
            assert!(
                hosts[1].is_active_for_test(),
                "{label}: selecting the second tab works"
            );
            emit_select(&tab_view, 0);
            assert!(
                hosts[0].is_active_for_test(),
                "{label}: back to the owning tab"
            );

            if remove_owning_first {
                tab_view.remove_at(0);
                assert_eq!(
                    count(&log, "B unmount"),
                    1,
                    "{label}: B unmounts with its tab"
                );
                assert!(hosts[0].hosted_tree_for_test().is_none());
                assert!(
                    hosts[1].hosted_tree_for_test().is_none(),
                    "{label}: no dead tree left"
                );
                tab_view.remove_at(0);
            } else {
                tab_view.remove_at(1);
                assert_eq!(
                    count(&log, "B unmount"),
                    0,
                    "{label}: other tab never touches B"
                );
                assert!(publishes_b(&hosts[0]), "{label}: owning tab keeps B");
                tab_view.remove_at(0);
            }
            assert_eq!(
                count(&log, "B unmount"),
                1,
                "{label}: B unmounts exactly once"
            );
            assert!(
                hosts
                    .iter()
                    .all(|host| host.hosted_tree_for_test().is_none())
            );
        }
    }
}
