//! Mock-peer tests for the Core-owned container and menu controls (Issue #313 stage ④):
//! `Dropdown`, `ScrollView`, `TabView`/`TabViewItem` and `Menu`/`MenuItem`/`MenuBar`/
//! `MenuBarItem`.

use super::*;
use crate::ui::native::{
    DropdownPeer, MenuBarItemPeer, MenuBarPeer, MenuItemPeer, MenuPeer, NativePeer,
    NativePeerFactory, ScrollViewPeer, TabViewPeer, clear_native_peer_factory,
    register_native_peer_factory,
};

type Log = Rc<RefCell<Vec<String>>>;
type Slot<T> = RefCell<Option<Rc<dyn Fn(T)>>>;

/// Calls a stored callback without holding the slot borrow, like a backend dispatching a native
/// event.
fn fire<T>(slot: &Slot<T>, value: T) {
    let callback = slot.borrow().clone();
    if let Some(callback) = callback {
        callback(value);
    }
}

fn store<T: 'static>(slot: &Slot<T>, callback: Option<Box<dyn Fn(T)>>) {
    *slot.borrow_mut() = callback.map(Rc::from);
}

/// The shared `NativePeer` part of every mock.
macro_rules! mock_native_peer {
    ($ty:ty) => {
        impl NativePeer for $ty {
            fn handle(&self) -> &dyn Any {
                &()
            }
            fn measure(&self, _available: Size) -> Size {
                Size {
                    width: 80.0,
                    height: 24.0,
                }
            }
            fn apply_background(&self, _background: Option<&Brush>) -> bool {
                true
            }
            fn set_tooltip(&self, _tooltip: Option<&str>) {}
            fn supports_text_style(&self) -> bool {
                false
            }
            fn apply_text_style(&self, _style: &crate::graphics::CascadedTextStyle) -> bool {
                true
            }
            fn dispose(&self) {
                self.log.borrow_mut().push("dispose".to_string());
            }
        }
    };
}

struct MockDropdown {
    log: Log,
    on_change: Slot<usize>,
}
mock_native_peer!(MockDropdown);

impl DropdownPeer for MockDropdown {
    fn set_items(&self, texts: &[String]) {
        self.log
            .borrow_mut()
            .push(format!("items:{}", texts.join(",")));
    }
    fn set_selected_index(&self, index: usize) {
        self.log.borrow_mut().push(format!("selected:{index}"));
    }
    fn set_enabled(&self, enabled: bool) {
        self.log.borrow_mut().push(format!("enabled:{enabled}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(usize)>>) {
        self.log
            .borrow_mut()
            .push(format!("on_change:{}", callback.is_some()));
        store(&self.on_change, callback);
    }
}

struct MockScrollView {
    log: Log,
}
mock_native_peer!(MockScrollView);

impl ScrollViewPeer for MockScrollView {
    fn set_content(&self, content: Rc<dyn UIElementExt>, owner: Option<Rc<dyn UIElementExt>>) {
        let owned =
            owner.is_some_and(|owner| content.parent().is_none() && Rc::strong_count(&owner) > 0);
        self.log.borrow_mut().push(format!(
            "content:{:p}:{owned}",
            Rc::as_ptr(&content) as *const ()
        ));
    }
    fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        self.log.borrow_mut().push(format!("horizontal:{enabled}"));
    }
    fn set_vertical_scroll_enabled(&self, enabled: bool) {
        self.log.borrow_mut().push(format!("vertical:{enabled}"));
    }
    fn unmount_content(&self) {
        self.log.borrow_mut().push("unmount_content".to_string());
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MockTabView {
    log: Log,
    on_select: Slot<usize>,
    on_close: Slot<usize>,
    on_new_tab: Slot<()>,
    /// Runs inside the next `set_items`, like a content `on_unmount` hook re-entering Core.
    reenter: RefCell<Option<Box<dyn FnOnce()>>>,
}
mock_native_peer!(MockTabView);

impl TabViewPeer for MockTabView {
    fn set_items(&self, items: &[Rc<dyn TabViewItemExt>], owner: Option<Rc<dyn UIElementExt>>) {
        let headers: Vec<String> = items
            .iter()
            .map(|item| item.as_ui_element().header())
            .collect();
        self.log
            .borrow_mut()
            .push(format!("items:{}:{}", headers.join(","), owner.is_some()));
        let reenter = self.reenter.borrow_mut().take();
        if let Some(reenter) = reenter {
            reenter();
        }
    }
    fn set_selected_index(&self, index: usize) {
        self.log.borrow_mut().push(format!("selected:{index}"));
    }
    fn set_tab_header(&self, index: usize, header: &str) {
        self.log
            .borrow_mut()
            .push(format!("header:{index}:{header}"));
    }
    fn set_tab_closable(&self, index: usize, closable: bool) {
        self.log
            .borrow_mut()
            .push(format!("closable:{index}:{closable}"));
    }
    fn set_on_select(&self, callback: Option<Box<dyn Fn(usize)>>) {
        store(&self.on_select, callback);
    }
    fn set_on_close(&self, callback: Option<Box<dyn Fn(usize)>>) {
        store(&self.on_close, callback);
    }
    fn set_on_new_tab(&self, callback: Option<Box<dyn Fn()>>) {
        store(
            &self.on_new_tab,
            callback.map(|callback| Box::new(move |()| callback()) as Box<dyn Fn(())>),
        );
    }
    fn unmount_content(&self) {
        self.log.borrow_mut().push("unmount_content".to_string());
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MockMenuItem {
    log: Log,
    on_select: Slot<()>,
}

impl MenuItemPeer for MockMenuItem {
    fn set_text(&self, text: &str) {
        self.log.borrow_mut().push(format!("item_text:{text}"));
    }
    fn set_icon(&self, icon: Option<crate::graphics::IconSource>) {
        self.log
            .borrow_mut()
            .push(format!("item_icon:{}", icon.is_some()));
    }
    fn set_shortcut(&self, shortcut: &str) {
        self.log
            .borrow_mut()
            .push(format!("item_shortcut:{shortcut}"));
    }
    fn set_enabled(&self, enabled: bool) {
        self.log
            .borrow_mut()
            .push(format!("item_enabled:{enabled}"));
    }
    fn set_on_select(&self, callback: Option<Box<dyn Fn()>>) {
        store(
            &self.on_select,
            callback.map(|callback| Box::new(move |()| callback()) as Box<dyn Fn(())>),
        );
    }
    fn dispose(&self) {
        self.log.borrow_mut().push("item_dispose".to_string());
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MockMenu {
    log: Log,
}

impl MenuPeer for MockMenu {
    fn set_items(&self, items: &[Rc<dyn MenuItemPeer>]) {
        self.log
            .borrow_mut()
            .push(format!("menu_items:{}", items.len()));
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MockMenuBarItem {
    log: Log,
}

impl MenuBarItemPeer for MockMenuBarItem {
    fn set_text(&self, text: &str) {
        self.log.borrow_mut().push(format!("bar_item_text:{text}"));
    }
    fn set_submenu(&self, _submenu: Rc<dyn MenuPeer>) {
        self.log.borrow_mut().push("bar_item_submenu".to_string());
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MockMenuBar {
    log: Log,
}

impl MenuBarPeer for MockMenuBar {
    fn set_items(&self, items: &[Rc<dyn MenuBarItemPeer>]) {
        self.log
            .borrow_mut()
            .push(format!("bar_items:{}", items.len()));
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default)]
struct MockFactory {
    log: Log,
    created: RefCell<Vec<&'static str>>,
    dropdowns: RefCell<Vec<Rc<MockDropdown>>>,
    tab_views: RefCell<Vec<Rc<MockTabView>>>,
    menu_items: RefCell<Vec<Rc<MockMenuItem>>>,
}

impl MockFactory {
    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
    fn created(&self, control: &str) -> usize {
        self.created
            .borrow()
            .iter()
            .filter(|c| **c == control)
            .count()
    }
}

impl NativePeerFactory for MockFactory {
    fn backend_name(&self) -> &'static str {
        "mock"
    }
    fn create_dropdown(&self) -> Rc<dyn DropdownPeer> {
        self.created.borrow_mut().push("Dropdown");
        let peer = Rc::new(MockDropdown {
            log: self.log.clone(),
            on_change: RefCell::new(None),
        });
        self.dropdowns.borrow_mut().push(peer.clone());
        peer
    }
    fn create_scroll_view(&self) -> Rc<dyn ScrollViewPeer> {
        self.created.borrow_mut().push("ScrollView");
        Rc::new(MockScrollView {
            log: self.log.clone(),
        })
    }
    fn create_tab_view(&self) -> Rc<dyn TabViewPeer> {
        self.created.borrow_mut().push("TabView");
        let peer = Rc::new(MockTabView {
            log: self.log.clone(),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            on_new_tab: RefCell::new(None),
            reenter: RefCell::new(None),
        });
        self.tab_views.borrow_mut().push(peer.clone());
        peer
    }
    fn create_menu_item(&self) -> Rc<dyn MenuItemPeer> {
        self.created.borrow_mut().push("MenuItem");
        let peer = Rc::new(MockMenuItem {
            log: self.log.clone(),
            on_select: RefCell::new(None),
        });
        self.menu_items.borrow_mut().push(peer.clone());
        peer
    }
    fn create_menu(&self) -> Rc<dyn MenuPeer> {
        self.created.borrow_mut().push("Menu");
        Rc::new(MockMenu {
            log: self.log.clone(),
        })
    }
    fn create_menu_bar_item(&self) -> Rc<dyn MenuBarItemPeer> {
        self.created.borrow_mut().push("MenuBarItem");
        Rc::new(MockMenuBarItem {
            log: self.log.clone(),
        })
    }
    fn create_menu_bar(&self) -> Rc<dyn MenuBarPeer> {
        self.created.borrow_mut().push("MenuBar");
        Rc::new(MockMenuBar {
            log: self.log.clone(),
        })
    }
}

fn install_mock() -> Rc<MockFactory> {
    let factory = Rc::new(MockFactory::default());
    register_native_peer_factory(factory.clone());
    factory
}

fn realize(node: &dyn UIElementExt) {
    node.measure(Size {
        width: 200.0,
        height: 100.0,
    });
}

fn recorder<T: Clone + 'static>() -> (Rc<RefCell<Vec<T>>>, Box<dyn Fn(T)>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    (
        seen,
        Box::new(move |value: T| sink.borrow_mut().push(value)),
    )
}

fn dropdown_item(text: &str) -> Rc<dyn DropdownItemExt> {
    let item = DropdownItem::new();
    item.set_text(text);
    item
}

fn tab(header: &str) -> Rc<dyn TabViewItemExt> {
    let item = TabViewItem::new();
    item.set_header(header);
    item
}

#[test]
fn dropdown_pushes_items_before_selection_and_drops_echoes() {
    let factory = install_mock();
    let dropdown = Dropdown::new();
    dropdown.add(dropdown_item("Small"));
    dropdown.add(dropdown_item("Large"));
    let (seen, sink) = recorder();
    dropdown.set_on_selected_index_change(sink);
    assert_eq!(
        factory.created("Dropdown"),
        0,
        "no peer before hosted layout"
    );

    realize(dropdown.as_ref());
    realize(dropdown.as_ref());
    assert_eq!(factory.created("Dropdown"), 1);
    assert_eq!(
        factory.take_log(),
        [
            "items:Small,Large",
            "selected:0",
            "enabled:true",
            "on_change:true"
        ]
    );

    // A list change re-pushes the texts and then the stored selection.
    dropdown.add(dropdown_item("Huge"));
    assert_eq!(factory.take_log(), ["items:Small,Large,Huge", "selected:0"]);

    // A user selection is stored, notified once and exposed as the accessibility value.
    let peer = factory.dropdowns.borrow()[0].clone();
    fire(&peer.on_change, 1);
    fire(&peer.on_change, 1);
    assert_eq!(
        *seen.borrow(),
        [1],
        "an echo of the stored index is dropped"
    );
    let value = dropdown
        .intrinsic_accessibility_semantics()
        .and_then(|semantics| semantics.value);
    assert_eq!(value.as_deref(), Some("Large"));

    // Programmatic selection is pushed, not notified.
    dropdown.set_selected_index(2);
    assert_eq!(factory.take_log(), ["selected:2"]);
    assert_eq!(*seen.borrow(), [1]);

    // After the owner is gone a stale native event is a no-op.
    drop(dropdown);
    assert_eq!(factory.take_log(), ["dispose"]);
    fire(&peer.on_change, 0);
    assert_eq!(*seen.borrow(), [1]);
    clear_native_peer_factory();
}

#[test]
fn scroll_view_pushes_axes_then_content_and_retires_content_on_unmount() {
    let factory = install_mock();
    let unrealized = ScrollView::new();
    unrealized.set_content(VerticalLayout::new());
    let unrealized_element: Rc<dyn UIElementExt> = unrealized.clone();
    crate::ui::unmount_subtree(&unrealized_element);
    assert_eq!(
        factory.created("ScrollView"),
        0,
        "an unrealized ScrollView neither realizes nor retires anything on unmount"
    );

    let scroll_view = ScrollView::new();
    let first: Rc<dyn UIElementExt> = VerticalLayout::new();
    scroll_view.set_horizontal_scroll_enabled(true);
    scroll_view.set_content(first.clone());
    realize(scroll_view.as_ref());
    let first_entry = format!("content:{:p}:true", Rc::as_ptr(&first) as *const ());
    assert_eq!(
        factory.take_log(),
        ["horizontal:true", "vertical:true", first_entry.as_str()]
    );

    let second: Rc<dyn UIElementExt> = VerticalLayout::new();
    scroll_view.set_content(second.clone());
    let second_entry = format!("content:{:p}:true", Rc::as_ptr(&second) as *const ());
    assert_eq!(factory.take_log(), [second_entry]);
    assert!(Rc::ptr_eq(&scroll_view.content().unwrap(), &second));

    let element: Rc<dyn UIElementExt> = scroll_view.clone();
    crate::ui::unmount_subtree(&element);
    assert_eq!(factory.take_log(), ["unmount_content"]);
    clear_native_peer_factory();
}

#[test]
fn tab_view_owns_the_selection_policy_and_reports_each_change_once() {
    let factory = install_mock();
    let tab_view = TabView::new();
    let items: Vec<Rc<dyn TabViewItemExt>> = ["a", "b", "c", "d"].map(tab).to_vec();
    for item in &items {
        tab_view.add(item.clone());
    }
    let (seen, sink) = recorder();
    tab_view.set_on_selected_index_change(sink);
    realize(tab_view.as_ref());
    assert_eq!(factory.created("TabView"), 1);
    assert_eq!(factory.take_log(), ["items:a,b,c,d:true", "selected:0"]);
    let peer = factory.tab_views.borrow()[0].clone();

    // A user selection the peer already shows: stored and reported once; its echo is dropped.
    fire(&peer.on_select, 2);
    fire(&peer.on_select, 2);
    assert_eq!(*seen.borrow(), [2]);
    assert_eq!(tab_view.selected_index(), 2);

    // Removing an earlier tab keeps the selected tab by identity; its index shifts once.
    ListExt::remove_at(&*tab_view, 0);
    assert_eq!(factory.take_log(), ["items:b,c,d:true", "selected:1"]);
    assert_eq!(*seen.borrow(), [2, 1]);

    // Removing the selected last tab selects the new last tab.
    fire(&peer.on_select, 2);
    ListExt::remove_at(&*tab_view, 2);
    assert_eq!(tab_view.selected_index(), 1);
    assert_eq!(*seen.borrow(), [2, 1, 2, 1]);

    // Removing the selected middle tab selects the tab now at its index: same index, no report.
    tab_view.add(tab("e"));
    factory.take_log();
    ListExt::remove_at(&*tab_view, 1);
    assert_eq!(factory.take_log(), ["items:b,e:true", "selected:1"]);
    assert_eq!(
        *seen.borrow(),
        [2, 1, 2, 1],
        "unchanged index is not reported"
    );

    // A programmatic selection is pushed, not reported.
    tab_view.set_selected_index(0);
    assert_eq!(factory.take_log(), ["selected:0"]);
    assert_eq!(seen.borrow().len(), 4);

    // Header and closable changes of a displayed item reach its tab by index.
    let e = tab_view.to_vec()[1].clone();
    e.set_header("E");
    e.set_closable(false);
    assert_eq!(
        factory.take_log(),
        [
            "header:1:E",
            "closable:1:true",
            "header:1:E",
            "closable:1:false"
        ]
    );
    clear_native_peer_factory();
}

#[test]
fn tab_view_close_prefers_the_item_handler_and_new_tab_is_forwarded() {
    let factory = install_mock();
    let tab_view = TabView::new();
    let own = TabViewItem::new();
    let own_closed = Rc::new(Cell::new(0));
    let counter = own_closed.clone();
    own.set_on_close(Box::new(move || counter.set(counter.get() + 1)));
    tab_view.add(own);
    tab_view.add(tab("plain"));
    let (closed, sink) = recorder();
    tab_view.set_on_close(sink);
    let new_tabs = Rc::new(Cell::new(0));
    let counter = new_tabs.clone();
    tab_view.set_on_new_tab(Box::new(move || counter.set(counter.get() + 1)));
    realize(tab_view.as_ref());
    let peer = factory.tab_views.borrow()[0].clone();

    fire(&peer.on_close, 0);
    fire(&peer.on_close, 1);
    fire(&peer.on_new_tab, ());
    assert_eq!(own_closed.get(), 1, "the tab's own handler wins");
    assert_eq!(
        *closed.borrow(),
        [1],
        "the fallback gets the other tab's index"
    );
    assert_eq!(new_tabs.get(), 1);

    drop(tab_view);
    fire(&peer.on_close, 1);
    fire(&peer.on_select, 1);
    assert_eq!(*closed.borrow(), [1], "stale native events are no-ops");
    clear_native_peer_factory();
}

/// A list change made reentrantly from inside a peer push (a removed tab's content `on_unmount`
/// hook removing another tab) is pushed, and only the outermost push applies the final selection
/// and reports it once (§6 last-write-wins).
#[test]
fn tab_view_reentrant_list_change_applies_the_latest_selection_once() {
    let factory = install_mock();
    let tab_view = TabView::new();
    for header in ["a", "b", "c", "d"] {
        tab_view.add(tab(header));
    }
    let (seen, sink) = recorder();
    tab_view.set_on_selected_index_change(sink);
    realize(tab_view.as_ref());
    let peer = factory.tab_views.borrow()[0].clone();
    fire(&peer.on_select, 3);
    factory.take_log();

    let weak = Rc::downgrade(&tab_view);
    *peer.reenter.borrow_mut() = Some(Box::new(move || {
        if let Some(tab_view) = weak.upgrade() {
            ListExt::remove_at(&*tab_view, 0);
        }
    }));
    ListExt::remove_at(&*tab_view, 1);
    assert_eq!(
        factory.take_log(),
        ["items:a,c,d:true", "items:c,d:true", "selected:1"],
        "the inner push only updates the list; the outer one applies the final selection"
    );
    assert_eq!(tab_view.selected_index(), 1, "d stays selected by identity");
    assert_eq!(
        *seen.borrow(),
        [3, 1],
        "reported once, with the final index"
    );
    clear_native_peer_factory();
}

#[test]
fn menus_realize_item_peers_on_demand_and_route_selection_through_core() {
    let factory = install_mock();
    let menu = Menu::new();
    let item = MenuItem::new();
    item.set_text("Open");
    item.set_shortcut("o");
    let selected = Rc::new(Cell::new(0));
    let counter = selected.clone();
    item.set_on_select(Box::new(move || counter.set(counter.get() + 1)));
    menu.add(item.clone());
    assert!(
        factory.created.borrow().is_empty(),
        "a logical menu needs no backend until shown natively"
    );

    let menu_peer = __menu_peer_of(&*menu);
    let again = __menu_peer_of(&*menu);
    assert!(std::ptr::addr_eq(
        Rc::as_ptr(&menu_peer),
        Rc::as_ptr(&again)
    ));
    assert_eq!(factory.created("Menu"), 1, "realized once");
    assert_eq!(factory.created("MenuItem"), 1);
    assert_eq!(
        factory.take_log(),
        [
            "item_text:Open",
            "item_shortcut:o",
            "item_enabled:true",
            "menu_items:1"
        ]
    );

    // Later changes reach the realized peers.
    item.set_enabled(false);
    menu.add(MenuItem::new());
    assert_eq!(
        factory.take_log(),
        [
            "item_enabled:false",
            "item_text:",
            "item_enabled:true",
            "menu_items:2"
        ]
    );

    // A native activation runs Core's `select`, which runs `on_select`.
    let item_peer = factory.menu_items.borrow()[0].clone();
    fire(&item_peer.on_select, ());
    item.select();
    assert_eq!(selected.get(), 2);

    // Dropping the last owner disposes the item peer; a stale activation is a no-op.
    menu.clear();
    factory.take_log();
    drop(item);
    assert_eq!(factory.take_log(), ["item_dispose"]);
    fire(&item_peer.on_select, ());
    assert_eq!(selected.get(), 2);
    clear_native_peer_factory();
}

#[test]
fn menu_bar_realizes_entries_and_their_submenus() {
    let factory = install_mock();
    let bar = MenuBar::new();
    let file = MenuBarItem::new();
    file.set_text("File");
    let submenu = Menu::new();
    submenu.add(MenuItem::new());
    file.set_submenu(submenu);
    bar.add(file.clone());

    let _peer = __menu_bar_peer_of(&*bar);
    assert_eq!(
        factory.take_log(),
        [
            "bar_item_text:File",
            "item_text:",
            "item_enabled:true",
            "menu_items:1",
            "bar_item_submenu",
            "bar_items:1"
        ]
    );
    file.set_text("Datei");
    ListExt::remove(&*bar, &(file.clone() as Rc<dyn MenuBarItemExt>));
    assert_eq!(factory.take_log(), ["bar_item_text:Datei", "bar_items:0"]);
    clear_native_peer_factory();
}

/// Drives `reconcile_ordered_peers` against an append/remove-only "native" list and returns the
/// resulting native order plus how many removals it needed.
fn reconcile_into(native: &RefCell<Vec<Rc<str>>>, old: &[Rc<str>], new: &[Rc<str>]) -> usize {
    let removals = Cell::new(0);
    crate::ui::native::reconcile_ordered_peers(
        old,
        new,
        |removed| {
            removals.set(removals.get() + 1);
            native.borrow_mut().retain(|e| !Rc::ptr_eq(e, removed));
        },
        |added| native.borrow_mut().push(added.clone()),
    );
    removals.get()
}

#[test]
fn ordered_peer_reconcile_matches_the_pushed_order() {
    let (a, b, c, x): (Rc<str>, Rc<str>, Rc<str>, Rc<str>) =
        (Rc::from("A"), Rc::from("B"), Rc::from("C"), Rc::from("X"));
    let names = |native: &RefCell<Vec<Rc<str>>>| {
        native
            .borrow()
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    };
    let native = RefCell::new(Vec::new());
    let mut current: Vec<Rc<str>> = Vec::new();
    let mut push = |next: Vec<Rc<str>>| {
        let removals = reconcile_into(&native, &current, &next);
        current = next;
        removals
    };

    // Appending keeps the kept entries in place (no removals).
    assert_eq!(push(vec![a.clone(), b.clone()]), 0);
    assert_eq!(names(&native), ["A", "B"]);
    // An insertion before a kept entry (the review case `[A,B] -> insert(0, X)`).
    push(vec![x.clone(), a.clone(), b.clone()]);
    assert_eq!(names(&native), ["X", "A", "B"]);
    // A removal in the middle only removes that entry.
    assert_eq!(push(vec![x.clone(), b.clone()]), 1);
    assert_eq!(names(&native), ["X", "B"]);
    // A reorder of kept entries, plus an append.
    push(vec![b.clone(), x.clone(), c.clone()]);
    assert_eq!(names(&native), ["B", "X", "C"]);
    // Removing the head while appending: the rest stays a prefix.
    assert_eq!(push(vec![x.clone(), c.clone(), a.clone()]), 1);
    assert_eq!(names(&native), ["X", "C", "A"]);
    push(Vec::new());
    assert!(native.borrow().is_empty());
}

/// Runs `action`, which must panic with the duplicate-item diagnostic for `owner`.
fn expect_duplicate_panic(owner: &str, action: impl FnOnce()) {
    let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
        .expect_err("a duplicate item must be rejected");
    let message = error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        message.contains(&format!("already in the `{owner}`'s list")),
        "diagnostic names the list: {message}"
    );
}

#[test]
fn duplicate_items_are_rejected_before_any_change() {
    let factory = install_mock();

    // Menu: `add` and `insert` of an item already present (review case: add twice, remove once).
    let menu = Menu::new();
    let item = MenuItem::new();
    menu.add(item.clone());
    __menu_peer_of(&*menu);
    factory.take_log();
    expect_duplicate_panic("Menu", || menu.add(item.clone()));
    expect_duplicate_panic("Menu", || ListExt::insert(&*menu, 0, item.clone()));
    assert_eq!(menu.len(), 1, "the list is unchanged");
    assert!(factory.take_log().is_empty(), "nothing was pushed");
    assert!(ListExt::remove(
        &*menu,
        &(item.clone() as Rc<dyn MenuItemExt>)
    ));
    assert!(menu.is_empty());
    // After removal, the same item can be inserted again.
    ListExt::insert(&*menu, 0, item.clone());
    assert_eq!(menu.len(), 1);

    // MenuBar.
    let menu_bar = MenuBar::new();
    let bar_item = MenuBarItem::new();
    menu_bar.add(bar_item.clone());
    __menu_bar_peer_of(&*menu_bar);
    factory.take_log();
    expect_duplicate_panic("MenuBar", || {
        ListExt::insert(&*menu_bar, 1, bar_item.clone())
    });
    assert_eq!(menu_bar.len(), 1);
    assert!(factory.take_log().is_empty());

    // TabView: `add`, `insert` and a `set_children` list holding one item twice.
    let tab_view = TabView::new();
    realize(&*tab_view);
    let first = TabViewItem::new();
    let second = TabViewItem::new();
    tab_view.set_children(vec![first.clone(), second.clone()]);
    factory.take_log();
    expect_duplicate_panic("TabView", || tab_view.add(first.clone()));
    expect_duplicate_panic("TabView", || ListExt::insert(&*tab_view, 0, second.clone()));
    expect_duplicate_panic("TabView", || {
        tab_view.set_children(vec![first.clone(), first.clone()])
    });
    assert_eq!(tab_view.len(), 2, "the tabs are unchanged");
    assert!(factory.take_log().is_empty());
    clear_native_peer_factory();
}

#[test]
fn ordered_peer_reconcile_never_slices_out_of_range() {
    // A list that breaks the distinct-items contract must not panic in the helper: it takes the
    // full-rebuild path (review case `old = [a, a]`, `new = [a]`).
    let a: Rc<str> = Rc::from("A");
    let native = RefCell::new(vec![a.clone(), a.clone()]);
    reconcile_into(&native, &[a.clone(), a.clone()], &[a.clone()]);
    assert_eq!(native.borrow().len(), 1);
}
