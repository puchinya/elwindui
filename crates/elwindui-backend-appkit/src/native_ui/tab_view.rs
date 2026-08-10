//! `elwindui::ui::TabView`/`TabViewItem`, their identity helpers, and the `ListExt` collection.

use super::NativeControl;
use crate::AnyView;
use crate::inner::{InnerTabView, TabChipImpl};
use elwindui_core::ui::UIElementExt;
use objc2::rc::Retained;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// See docs/specs/ui_spec.md#tabs. `TabView` owns an ordered collection of literal
/// `TabViewItem` children. Generated dynamic child slots reconcile that collection by `Rc`
/// identity; this backend reconciles the corresponding native chips and content hosts.
/// `struct_only = elwindui_core::ui::TabViewExt` (the shared trait exposes `children()`) —
/// see its own doc comment): every method below stays `#[inherent]`, exactly as when this was an
/// ordinary `inherits = NativeControl` class with its own backend-local auto-generated trait — this
/// only swaps which trait path `TabViewExt` resolves to. `insert_tab`/`remove_tab`/
/// `set_tab_content_visible` are plain `InnerTabView` methods, not a separate cross-backend trait,
/// since a real tab content host type differs per backend (AppKit's `Retained<TreeHostView>`/
/// `TabChipImpl` have no common shape with WinUI3's own equivalents worth sharing without
/// associated types this crate doesn't need yet).
#[elwindui_macros::class(struct_only = elwindui_core::ui::TabViewExt, inherits = crate::NativeControl)]
pub struct TabView {
    inner: InnerTabView,
    children: elwindui_core::ui::ChildList<dyn elwindui_core::ui::TabViewItemExt>,
    selected_index: Cell<usize>,
    /// Parallel to `displayed` below — each currently-displayed entry's chip + persistent content
    /// host, in the same order.
    chips: RefCell<Vec<(TabChipImpl, Retained<crate::host::TreeHostView>)>>,
    /// Pointer identities (`Rc::as_ptr`, as `usize`) of the entries currently reflected as real
    /// chips/hosts, in display order — the "before" side of `rebuild`'s diff against `entries`'
    /// current pointers (the "after" side). Mirrors `winui3::tab_view`'s `displayed`.
    displayed: RefCell<Vec<usize>>,
    /// Pointer identity of the entry whose host is currently visible (shown, all others hidden) —
    /// `None` before the first `rebuild`.
    visible: RefCell<Option<usize>>,
    on_select: RefCell<Option<Box<dyn Fn(usize)>>>,
    on_close: RefCell<Option<Box<dyn Fn(usize)>>>,
    weak_self: RefCell<Weak<TabView>>,
}

/// The backend-native representation of one declarative `TabViewItem`.
/// `struct_only = elwindui_core::ui::TabViewItemExt` (a deliberately empty shared trait — see its
/// own doc comment in `elwindui-core`): every method below stays `#[inherent]`, unchanged from
/// before this struct participated in the class hierarchy at all — this only makes
/// `elwindui_core::ui::TabViewItemExt` a real, resolvable path so `elwindui-codegen`'s
/// `builtin_trait_use` can treat `TabViewItem` uniformly with every other native/virtual builtin.
/// No `inherits`: like `Window`, never itself embedded as a real `Rc<dyn UIElement>` node.
#[elwindui_macros::class(struct_only = elwindui_core::ui::TabViewItemExt)]
pub struct TabViewItem {
    header: RefCell<String>,
    on_header_changed: RefCell<Option<Box<dyn Fn()>>>,
    // Handed to this entry's persistent content host (`TreeHostView::set_tree`) the first time
    // it's actually inserted as a real tab.
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    closable: Cell<bool>,
    on_close: RefCell<Option<Box<dyn Fn()>>>,
}

#[elwindui_macros::class]
impl TabViewItem {
    fn construct() -> Self {
        Self {
            header: RefCell::new(String::new()),
            on_header_changed: RefCell::new(None),
            content: RefCell::new(None),
            closable: Cell::new(true),
            on_close: RefCell::new(None),
        }
    }

    #[inherent]
    pub fn set_header(&self, header: &str) {
        if *self.header.borrow() == header {
            return;
        }
        *self.header.borrow_mut() = header.to_string();
        if let Some(callback) = self.on_header_changed.borrow().as_ref() {
            callback();
        }
    }

    #[inherent]
    pub fn set_content(&self, content: Rc<dyn UIElementExt>) {
        *self.content.borrow_mut() = Some(content);
    }

    #[inherent]
    pub fn set_closable(&self, closable: bool) {
        self.closable.set(closable);
    }

    #[inherent]
    pub fn set_on_close(&self, callback: Box<dyn Fn()>) {
        *self.on_close.borrow_mut() = Some(callback);
    }
}

#[elwindui_macros::class]
impl TabView {
    fn construct() -> Self {
        let inner = InnerTabView::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            children: elwindui_core::ui::ChildList::new(),
            selected_index: Cell::new(0),
            chips: RefCell::new(Vec::new()),
            displayed: RefCell::new(Vec::new()),
            visible: RefCell::new(None),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        }
    }

    fn on_constructed(&self) {
        // `owner_rc()` is guaranteed `Some` here (see `Button::on_constructed`'s own doc comment);
        // downcasting the type-erased owner back to this concrete `TabView` is what lets `rebuild`/
        // `attach_header_listener` upgrade `weak_self` into a real `Rc<TabView>` later.
        let node = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("TabView::on_constructed: object must already be Rc-constructed");
        let any_rc: Rc<dyn Any> = node;
        let this = any_rc
            .downcast::<TabView>()
            .expect("TabView::on_constructed: owner must be this TabView");
        *self.weak_self.borrow_mut() = Rc::downgrade(&this);
        // WinUI3's `TabView` is a tab stop by default — see
        // docs/design/gui_framework_design.md §5.5.
        self.set_tab_stop(true);
    }

    /// Replaces the declaratively constructed children in one operation.
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<TabViewItem>>) {
        for item in &children {
            self.attach_header_listener(
                &(Rc::clone(item) as Rc<dyn elwindui_core::ui::TabViewItemExt>),
            );
        }
        self.children.replace_all(
            children
                .into_iter()
                .map(|item| item as Rc<dyn elwindui_core::ui::TabViewItemExt>)
                .collect(),
        );
        self.rebuild();
    }

    #[inherent]
    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_select.borrow_mut() = Some(callback);
    }

    /// Registers the write-back callback used by a two-way `selected_index` binding.
    #[inherent]
    pub fn set_on_selected_index_change(&self, callback: Box<dyn Fn(usize)>) {
        self.set_on_select(callback);
    }

    #[inherent]
    pub fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(callback);
    }

    #[inherent]
    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_new_tab(callback);
    }

    fn children(&self) -> &dyn elwindui_core::ui::ListExt<dyn elwindui_core::ui::TabViewItemExt> {
        self
    }

    #[inherent]
    pub fn set_selected_index(&self, selected_index: usize) {
        if self.selected_index.get() == selected_index {
            return;
        }
        self.selected_index.set(selected_index);
        self.rebuild();
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    #[inherent]
    fn attach_header_listener(&self, item: &Rc<dyn elwindui_core::ui::TabViewItemExt>) {
        let key = tab_view_item_key(item);
        let weak = self.weak_self.borrow().clone();
        *downcast_tab_view_item(&**item)
            .on_header_changed
            .borrow_mut() = Some(Box::new(move || {
            if let Some(tab_view) = weak.upgrade() {
                tab_view.refresh_dynamic_header(key);
            }
        }));
    }

    #[inherent]
    fn refresh_dynamic_header(&self, key: usize) {
        let Some(index) = self
            .displayed
            .borrow()
            .iter()
            .position(|displayed| *displayed == key)
        else {
            return;
        };
        let Some(item) = self
            .children
            .to_vec()
            .into_iter()
            .find(|item| tab_view_item_key(item) == key)
        else {
            return;
        };
        self.chips.borrow()[index]
            .0
            .set_title(&downcast_tab_view_item(&*item).header.borrow());
    }

    /// Keyed diff (pointer identity — see `displayed`'s doc comment): removes displayed tabs whose
    /// `TabViewItem` no longer appears in `children` (chip + persistent host together), inserts a
    /// chip + a fresh host for each not-yet-displayed one, refreshes every displayed tab's title,
    /// and shows/hides content hosts so only the selected entry's is visible.
    #[inherent]
    fn rebuild(&self) {
        let this = self
            .weak_self
            .borrow()
            .upgrade()
            .expect("elwindui: TabView dropped while rebuilding");
        let children = self.children.to_vec();
        let selected = self.selected_index.get();
        let new_keys: Vec<usize> = children.iter().map(tab_view_item_key).collect();

        let mut chips = self.chips.borrow_mut();
        let mut displayed = self.displayed.borrow_mut();

        for i in (0..displayed.len()).rev() {
            if !new_keys.contains(&displayed[i]) {
                let (chip, host) = chips.remove(i);
                self.inner.remove_tab(&chip, &host);
                displayed.remove(i);
            }
        }

        for (target_index, (key, entry)) in new_keys.iter().zip(children.iter()).enumerate() {
            if displayed.contains(key) {
                continue;
            }
            let label = downcast_tab_view_item(&**entry).header.borrow().clone();
            let key = *key;
            let on_select: Box<dyn Fn()> = {
                let this = Rc::clone(&this);
                Box::new(move || {
                    let index = this
                        .children
                        .to_vec()
                        .iter()
                        .position(|e| tab_view_item_key(e) == key);
                    if let (Some(index), Some(cb)) = (index, this.on_select.borrow().as_ref()) {
                        cb(index);
                    }
                })
            };
            let on_close: Box<dyn Fn()> = {
                let this = Rc::clone(&this);
                Box::new(move || {
                    let children = this.children.to_vec();
                    let Some(index) = children.iter().position(|e| tab_view_item_key(e) == key)
                    else {
                        return;
                    };
                    let entry = Rc::clone(&children[index]);
                    drop(children);
                    // A static `TabViewItem`'s own `on_close` (if set) takes precedence.
                    if let Some(cb) = downcast_tab_view_item(&*entry).on_close.borrow().as_ref() {
                        cb();
                    } else if let Some(cb) = this.on_close.borrow().as_ref() {
                        cb(index);
                    }
                })
            };
            let insert_at = target_index.min(displayed.len());
            let (chip, host) = self
                .inner
                .insert_tab(insert_at, &label, on_select, on_close);
            if let Some(content) = downcast_tab_view_item(&**entry).content.borrow().clone() {
                host.set_tree(content);
            }
            chips.insert(insert_at, (chip, host));
            displayed.insert(insert_at, key);
        }

        let selected_key = children.get(selected).map(tab_view_item_key);

        for (i, key) in displayed.iter().enumerate() {
            if let Some(entry) = children.iter().find(|e| tab_view_item_key(e) == *key) {
                chips[i]
                    .0
                    .set_title(&downcast_tab_view_item(&**entry).header.borrow());
            }
            chips[i].0.set_selected(Some(*key) == selected_key);
        }

        let mut visible = self.visible.borrow_mut();
        if *visible != selected_key {
            if let Some(old_key) = *visible {
                if let Some(pos) = displayed.iter().position(|k| *k == old_key) {
                    self.inner.set_tab_content_visible(&chips[pos].1, false);
                }
            }
            if let Some(new_key) = selected_key {
                if let Some(pos) = displayed.iter().position(|k| *k == new_key) {
                    self.inner.set_tab_content_visible(&chips[pos].1, true);
                }
            }
            *visible = selected_key;
        }
    }
}

fn downcast_tab_view_item(item: &dyn elwindui_core::ui::TabViewItemExt) -> &TabViewItem {
    item.as_any()
        .downcast_ref::<TabViewItem>()
        .expect("TabViewExt: child must be this backend's TabViewItem")
}

fn tab_view_item_key(item: &Rc<dyn elwindui_core::ui::TabViewItemExt>) -> usize {
    Rc::as_ptr(item) as *const () as usize
}

impl elwindui_core::ui::ListExt<dyn elwindui_core::ui::TabViewItemExt> for TabView {
    fn add(&self, item: Rc<dyn elwindui_core::ui::TabViewItemExt>) {
        self.attach_header_listener(&item);
        self.children.add(item);
        self.rebuild();
    }

    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::TabViewItemExt>) {
        self.attach_header_listener(&item);
        self.children.insert(index, item);
        self.rebuild();
    }

    fn remove(&self, item: &Rc<dyn elwindui_core::ui::TabViewItemExt>) -> bool {
        if !self.children.remove(item) {
            return false;
        }
        self.rebuild();
        true
    }

    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::TabViewItemExt> {
        let item = self.children.remove_at(index);
        self.rebuild();
        item
    }

    fn clear(&self) {
        self.children.clear();
        self.rebuild();
    }

    fn len(&self) -> usize {
        self.children.len()
    }
    fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::TabViewItemExt>> {
        self.children.to_vec()
    }
}
