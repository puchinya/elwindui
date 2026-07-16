//! Implements every `elwindui_core::ui` builtin trait this backend provides, by composing the
//! matching `crate::inner` type (see that module's own doc comment) — each class here is a thin
//! "call into `self.inner`" layer; all genuinely WinUI3-specific complexity lives in `inner.rs`.
//! See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3. Mirrors
//! `elwindui_backend_appkit::native_ui`'s structure exactly.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no type here at all:
//! they're `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds directly, reflected
//! into real XAML elements by `inner::TreeHostPanel` (used by both `Window`'s content view and
//! `TabView`'s per-tab content area).

use crate::AnyView;
use crate::inner::{
    InnerButton, InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem, InnerTabView,
    InnerTextArea, InnerWindow,
};
// Deliberately *not* `use elwindui_core::base::AsAny;` here — see
// `elwindui_backend_appkit::native_ui::MenuBarItem::set_submenu`'s doc comment (the one place that
// pattern is explained in full) for why importing `AsAny` directly, rather than relying on it as
// `MenuBarItemExt`/`MenuExt`/etc.'s own supertrait, silently breaks every
// `.as_any().downcast_ref::<T>()` call in this file.
use elwindui_core::ui::UIElementExt;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The backend-owned counterpart to `elwindui_core::ui::NativeControl` (a pure marker trait with no
/// backing struct of its own — measuring/placing a native handle is entirely backend-specific, so
/// `elwindui-core` doesn't define this generically). Holding `handle: AnyView` here once, instead
/// of on each of `TextArea`/`Button`/`TabView` individually, is what lets `inherits = NativeControl`
/// resolve `base`'s field type to this same struct.
#[elwindui_macros::class(struct_only = elwindui_core::ui::NativeControlExt, inherits = elwindui_core::ui::UIElement)]
pub struct NativeControl {
    handle: AnyView,
}

#[elwindui_macros::class]
impl NativeControl {
    #[overrides]
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.handle.measure(available)
    }
    #[overrides]
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        Some(&self.handle)
    }
    pub fn new(handle: AnyView) -> Self {
        Self {
            base: elwindui_core::ui::UIElement::default(),
            handle,
        }
    }
}

/// `component X inherits Window` ("host composition", docs/elwindui_spec.md 付録H.2.1a) is what
/// actually inherits this — hence `struct_only`'s target being `elwindui_core::ui::WindowExt`
/// itself. `Window` is deliberately *not* a `UIElement` (no `inherits` here at all) — like AppKit's
/// `Window`, it's a separate top-level concept, not embeddable as a child.
#[elwindui_macros::class(struct_only = elwindui_core::ui::WindowExt)]
pub struct Window {
    inner: InnerWindow,
}

#[elwindui_macros::class]
impl Window {
    // The bare (not `Rc`-wrapped) value `#[class]`'s auto-generated `new` wraps — this is also what
    // lets a `component X inherits Window` (host composition) embed a real `Window` directly as its
    // own `base` field.
    fn construct() -> Self {
        Self {
            inner: InnerWindow::new(),
        }
    }

    fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    fn set_menu_bar(&self, menu_bar: Rc<dyn elwindui_core::ui::MenuBarExt>) {
        let menu_bar = menu_bar
            .as_any()
            .downcast_ref::<MenuBar>()
            .expect("WindowExt::set_menu_bar: menu_bar must be this backend's MenuBar");
        self.inner.set_menu_bar(&menu_bar.inner);
    }

    fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.inner.set_content(content);
    }

    fn show(&self) {
        self.inner.show();
    }

    fn left(&self) -> f32 {
        self.inner.left()
    }

    fn set_left(&self, left: f32) {
        self.inner.set_left(left);
    }

    fn top(&self) -> f32 {
        self.inner.top()
    }

    fn set_top(&self, top: f32) {
        self.inner.set_top(top);
    }

    fn width(&self) -> f32 {
        self.inner.width()
    }

    fn set_width(&self, width: f32) {
        self.inner.set_width(width);
    }

    fn height(&self) -> f32 {
        self.inner.height()
    }

    fn set_height(&self, height: f32) {
        self.inner.set_height(height);
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextAreaExt, inherits = crate::NativeControl)]
pub struct TextArea {
    inner: InnerTextArea,
}

#[elwindui_macros::class]
impl TextArea {
    /// `#[two_way] text` (`TextArea` in `builtins.elwind`) — the change-back half of the binding;
    /// `elwindui_core::ui::TextArea::set_text` is the model→widget half.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    fn new() -> Rc<Self> {
        let inner = InnerTextArea::new();
        let handle = inner.handle();
        Rc::new(Self {
            base: NativeControl::new(handle),
            inner,
        })
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::ButtonExt, inherits = crate::NativeControl)]
pub struct Button {
    inner: InnerButton,
}

#[elwindui_macros::class]
impl Button {
    /// `#[routed] on_click` (`Button` in `builtins.elwind`) is registered directly onto this
    /// widget's own `base` — real since construction (see `new`), and already wired (also in `new`)
    /// to fire `dispatch_routed` starting at this same node.
    #[inherent]
    pub fn register_routed_handler<T: 'static>(
        &self,
        name: &'static str,
        handler: Box<dyn Fn(&T, &elwindui_core::input::RoutedEventArgs)>,
    ) {
        self.base
            .as_ui_element()
            .register_routed_handler(name, handler);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_click(callback);
    }
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    fn new() -> Rc<Self> {
        let inner = InnerButton::new();
        let handle = inner.handle();
        let this = Rc::new(Self {
            base: NativeControl::new(handle),
            inner,
        });
        // Wires the real XAML click directly to `dispatch_routed`, once, right here, rather than
        // re-detecting/re-wiring it on every relayout. Unconditional — `dispatch_routed` already
        // no-ops gracefully when nothing is registered for `"on_click"` at this node or any
        // ancestor (`elwindui-codegen`'s `emit_wiring` registers the actual `#[routed] on_click`
        // handler here, via `register_routed_handler` above, right after this constructor returns).
        {
            let node: Rc<dyn UIElementExt> = this.clone();
            this.inner.set_on_click(Box::new(move || {
                let args = elwindui_core::input::RoutedEventArgs::default();
                elwindui_core::ui::dispatch_routed(&node, "on_click", &(), &args);
            }));
        }
        this
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y and `elwindui_backend_appkit::native_ui::TabView`'s
/// doc comment for the overall two-mode convention (static `TabViewItem` children vs.
/// `items_source` + `header_template`/`item_template`) and why `TabViewItem` — not a bespoke
/// per-mode representation — is the thing both modes normalize into. `Microsoft.UI.Xaml.Controls.
/// TabView` is a real native tabbed-document control (unlike AppKit, which has none, hence that
/// backend's hand-rolled `TabChip`/`TabStrip`), and each `TabViewItem`'s `Content` here is a live
/// `crate::inner::TreeHostPanel` holding that tab's whole widget tree — recreating it on every
/// resync (as AppKit's *chips*, which are cheap, safely do) would reset a document's `TextArea`
/// (lost cursor/focus) on every keystroke. Unlike AppKit, this backend has **no** "content already
/// shown once" limitation for static mode: a `TabViewItem`'s `content` is moved into its own
/// persistent `TreeHostPanel` exactly once, when that `TabViewItem` is first inserted as a real
/// native tab — it is never subsequently discarded by selecting a different tab
/// (`Controls::TabView` shows/hides each item's own `Content` natively), so there's nothing to
/// restore. `struct_only = elwindui_core::ui::TabViewExt` (a deliberately empty shared trait — see
/// its own doc comment in `elwindui-core`) — mirrors `elwindui_backend_appkit::native_ui::TabView`'s
/// own shape; every method below stays `#[inherent]`, unchanged.
#[elwindui_macros::class(struct_only = elwindui_core::ui::TabViewExt, inherits = crate::NativeControl)]
pub struct TabView {
    inner: InnerTabView,
    entries: RefCell<Vec<Rc<TabViewItemImpl>>>,
    dynamic: RefCell<Option<DynamicSource>>,
    /// Pointer identities (`Rc::as_ptr`, as `usize`) of the `TabViewItem`s currently reflected as
    /// real `TabViewItem`s, in display order — the "before" side of `rebuild`'s diff against
    /// `entries`' current pointers (the "after" side).
    displayed: RefCell<Vec<usize>>,
    /// Not read by this type itself (`set_on_select` passes callbacks straight through to
    /// `crate::inner::InnerTabView`, which has no getter of its own) — tracked here purely so
    /// `selected_item`/`selected_container` can read it back.
    selected_index: Cell<usize>,
    on_close: RefCell<Option<Box<dyn Fn(usize)>>>,
    weak_self: RefCell<Weak<TabView>>,
}

/// The normalized per-tab representation — written literally in static mode, or synthesized once
/// per `items_source` element in dynamic mode (see `TabView`'s own doc comment).
/// `struct_only = elwindui_core::ui::TabViewItemExt` (a deliberately empty shared trait — see its
/// own doc comment in `elwindui-core`) — mirrors `elwindui_backend_appkit::native_ui::TabViewItemImpl`'s
/// own shape; every method below stays `#[inherent]`, unchanged.
#[elwindui_macros::class(struct_only = elwindui_core::ui::TabViewItemExt)]
pub struct TabViewItemImpl {
    item: RefCell<Option<Rc<dyn Any>>>,
    header: RefCell<String>,
    // Taken (moved into a real `TreeHostPanel`) the first time this `TabViewItem` is inserted as a
    // displayed tab; `None` afterward — see `TabView`'s own doc comment for why that's never a
    // problem here (unlike AppKit's single shared content pane).
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    closable: Cell<bool>,
    on_close: RefCell<Option<Box<dyn Fn()>>>,
}

#[elwindui_macros::class]
impl TabViewItemImpl {
    #[inherent]
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            item: RefCell::new(None),
            header: RefCell::new(String::new()),
            content: RefCell::new(None),
            closable: Cell::new(true),
            on_close: RefCell::new(None),
        })
    }

    /// Same shape as `sync_dynamic_entries`'s own erased construction need — kept as a free
    /// function (not a method) since it builds a whole `Self` from an already-erased
    /// `Rc<dyn Any>` item used to preserve dynamic-entry identity.
    #[inherent]
    fn new_erased(
        item: Option<Rc<dyn Any>>,
        header: &str,
        content: Rc<dyn UIElementExt>,
        closable: Option<bool>,
    ) -> Rc<Self> {
        Rc::new(Self {
            item: RefCell::new(item),
            header: RefCell::new(header.to_string()),
            content: RefCell::new(Some(content)),
            closable: Cell::new(closable.unwrap_or(true)),
            on_close: RefCell::new(None),
        })
    }

    #[inherent]
    pub fn set_header(&self, header: &str) {
        *self.header.borrow_mut() = header.to_string();
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

/// Only set in dynamic mode — `None` for a `TabView` built from static `TabViewItem` children.
struct DynamicSource {
    header_template: Option<Box<dyn Fn(&Rc<dyn Any>) -> String>>,
    item_template: Option<Box<dyn Fn(&Rc<dyn Any>) -> Rc<dyn UIElementExt>>>,
    closable_default: bool,
}

impl Default for DynamicSource {
    fn default() -> Self {
        DynamicSource {
            header_template: None,
            item_template: None,
            closable_default: true,
        }
    }
}

#[elwindui_macros::class]
impl TabView {
    #[inherent]
    pub fn new() -> Rc<Self> {
        let inner = InnerTabView::new();
        let handle = inner.handle();
        let this = Rc::new(Self {
            base: NativeControl::new(handle),
            inner,
            entries: RefCell::new(Vec::new()),
            dynamic: RefCell::new(None),
            displayed: RefCell::new(Vec::new()),
            selected_index: Cell::new(0),
            on_close: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        });
        *this.weak_self.borrow_mut() = Rc::downgrade(&this);
        this
    }

    /// Static mode: the literal `TabViewItem { .. }` children (mutually exclusive with
    /// `set_items_source`'s dynamic mode — see `TabView`'s own doc comment).
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<TabViewItemImpl>>) {
        if !children.is_empty() {
            *self.entries.borrow_mut() = children;
        }
    }

    /// Dynamic mode: establishes `self.dynamic`'s `header_template`/`item_template`.
    #[inherent]
    pub fn set_dynamic_source<T: 'static>(
        &self,
        items: Vec<Rc<T>>,
        header_template: Box<dyn Fn(&Rc<T>) -> String>,
        item_template: Box<dyn Fn(&Rc<T>) -> Rc<dyn UIElementExt>>,
    ) {
        let mut dynamic = self.dynamic.borrow_mut();
        let entry = dynamic.get_or_insert_with(DynamicSource::default);
        entry.header_template = Some(erase_render_string(header_template));
        entry.item_template = Some(erase_render(item_template));
        drop(dynamic);
        let _ = items;
    }

    #[inherent]
    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_select(callback);
    }

    /// WinUI3's `SelectedItem` concept — exposed as a plain accessor for advanced/manual use from
    /// hand-written Rust glue code.
    #[inherent]
    pub fn selected_item(&self) -> Option<Rc<dyn Any>> {
        self.entries
            .borrow()
            .get(self.selected_index.get())
            .and_then(|e| e.item.borrow().clone())
    }

    /// WinUI3's `SelectedContainer` concept — see `selected_item`'s doc comment.
    #[inherent]
    pub fn selected_container(&self) -> Option<Rc<TabViewItemImpl>> {
        self.entries
            .borrow()
            .get(self.selected_index.get())
            .cloned()
    }

    /// A static `TabViewItem`'s own `on_close` (if set) takes precedence — it's the per-item
    /// declaration in that mode; dynamic mode has none, so its `TabView`-level `on_close(index)`
    /// is used instead (same precedence as `elwindui_backend_appkit::native_ui::TabView`).
    #[inherent]
    pub fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(callback);
        let this = self.weak_self.borrow().clone();
        self.inner.set_on_close(Box::new(move |index| {
            let Some(this) = this.upgrade() else { return };
            let entry = this.entries.borrow().get(index).cloned();
            let handled = entry.is_some_and(|e| {
                if let Some(cb) = e.on_close.borrow().as_ref() {
                    cb();
                    true
                } else {
                    false
                }
            });
            if !handled {
                if let Some(cb) = this.on_close.borrow().as_ref() {
                    cb(index);
                }
            }
        }));
    }

    #[inherent]
    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_new_tab(callback);
    }

    /// Dynamic mode only — resyncs `items_source`. Reuses each already-synthesized `TabViewItem`
    /// whose stored item is still `Rc::ptr_eq` to the same element; only genuinely new elements
    /// call `header_template`/`item_template`.
    #[inherent]
    pub fn set_items_source<T: 'static>(&self, items: Vec<Rc<T>>) {
        if self.sync_dynamic_entries(erase_items(items)) {
            self.rebuild();
        }
    }

    /// Unlike `elwindui_backend_appkit::native_ui::TabView` (where selecting a tab means manually
    /// swapping the single visible content pane, done inside `rebuild`), `Controls::TabView`
    /// already shows/hides each `TabViewItem`'s own persistent `Content` based on `SelectedIndex`
    /// natively — so this is just a straight passthrough, no rebuild needed.
    #[inherent]
    pub fn set_selected_index(&self, selected_index: usize) {
        if self.selected_index.get() == selected_index {
            return;
        }
        self.selected_index.set(selected_index);
        self.inner.set_selected_index(selected_index);
    }

    /// The default `closable` for a synthesized `TabViewItem` in dynamic mode.
    #[inherent]
    pub fn set_closable(&self, closable: bool) {
        self.dynamic
            .borrow_mut()
            .get_or_insert_with(DynamicSource::default)
            .closable_default = closable;
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    #[inherent]
    fn sync_dynamic_entries(&self, items: Vec<Rc<dyn Any>>) -> bool {
        let dynamic = self.dynamic.borrow();
        let Some(dynamic) = dynamic.as_ref() else {
            return false;
        };
        let (Some(header_template), Some(item_template)) =
            (&dynamic.header_template, &dynamic.item_template)
        else {
            return false;
        };
        let mut entries = self.entries.borrow_mut();
        let mut changed = entries.len() != items.len();
        let new_entries: Vec<Rc<TabViewItemImpl>> = items
            .iter()
            .map(|item| {
                match entries.iter().find(|e| {
                    e.item
                        .borrow()
                        .as_ref()
                        .is_some_and(|entry_item| Rc::ptr_eq(entry_item, item))
                }) {
                    // Re-run `header_template` even for a reused entry — the label (e.g. a
                    // document's file name) can change independently of the item's own identity,
                    // and `entry.header` is otherwise never refreshed after construction.
                    Some(existing) => {
                        let header = header_template(item);
                        changed |= *existing.header.borrow() != header;
                        existing.set_header(&header);
                        Rc::clone(existing)
                    }
                    None => {
                        changed = true;
                        let header = header_template(item);
                        let content = item_template(item);
                        TabViewItemImpl::new_erased(
                            Some(Rc::clone(item)),
                            &header,
                            content,
                            Some(dynamic.closable_default),
                        )
                    }
                }
            })
            .collect();
        changed |= entries
            .iter()
            .zip(new_entries.iter())
            .any(|(old, new)| !Rc::ptr_eq(old, new));
        *entries = new_entries;
        changed
    }

    /// Keyed diff (pointer identity — see `displayed`'s doc comment): removes displayed tabs whose
    /// `TabViewItem` no longer appears in `entries`, inserts a real `TabViewItem` (+ that entry's
    /// one-time `content`) for each not-yet-displayed one, and refreshes every displayed tab's
    /// title (labels can change independently of tab identity, e.g. a document's file name). Does
    /// not handle reordering existing tabs (no reorder op is exposed, and nothing in notepad's
    /// command set reorders tabs today — only appends/removes) — an already-displayed tab is left
    /// in its current slot rather than physically moved to match `entries`' exact order.
    #[inherent]
    fn rebuild(&self) {
        let entries = self.entries.borrow();
        let new_keys: Vec<usize> = entries.iter().map(|e| Rc::as_ptr(e) as usize).collect();
        let mut displayed = self.displayed.borrow_mut();

        for i in (0..displayed.len()).rev() {
            if !new_keys.contains(&displayed[i]) {
                self.inner.remove_tab_at(i);
                displayed.remove(i);
            }
        }

        for (target_index, (key, entry)) in new_keys.iter().zip(entries.iter()).enumerate() {
            if !displayed.contains(key) {
                let label = entry.header.borrow().clone();
                let closable = entry.closable.get();
                let content_host =
                    self.inner
                        .insert_tab(target_index.min(displayed.len()), &label, closable);
                if let Some(content) = entry.content.borrow_mut().take() {
                    content_host.set_tree(content);
                }
                displayed.insert(target_index.min(displayed.len()), *key);
            }
        }
        drop(displayed);

        for (index, entry) in entries.iter().enumerate() {
            self.inner.set_tab_title(index, &entry.header.borrow());
        }
    }
}

fn erase_items<T: 'static>(items: Vec<Rc<T>>) -> Vec<Rc<dyn Any>> {
    items.into_iter().map(|t| t as Rc<dyn Any>).collect()
}

/// Wraps a caller-supplied `Fn(&Rc<T>) -> String` so it can be stored as
/// `Fn(&Rc<dyn Any>) -> String` — downcasting back to the concrete `T` on every call. The
/// `Rc<dyn Any>`s it's ever actually called with all come from `erase_items::<T>` for this same
/// `TabView`, so the downcast always succeeds.
fn erase_render_string<T: 'static>(
    f: Box<dyn Fn(&Rc<T>) -> String>,
) -> Box<dyn Fn(&Rc<dyn Any>) -> String> {
    Box::new(move |item: &Rc<dyn Any>| {
        let item: Rc<T> = Rc::clone(item)
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&item)
    })
}

/// Same as `erase_render_string`, for `item_template`'s `Rc<dyn UIElement>`-returning shape.
fn erase_render<T: 'static>(
    f: Box<dyn Fn(&Rc<T>) -> Rc<dyn UIElementExt>>,
) -> Box<dyn Fn(&Rc<dyn Any>) -> Rc<dyn UIElementExt>> {
    Box::new(move |item: &Rc<dyn Any>| {
        let item: Rc<T> = Rc::clone(item)
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&item)
    })
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuBarExt)]
pub struct MenuBar {
    inner: InnerMenuBar,
    /// The currently-installed children, in display order — the "before" side of `set_children`'s
    /// diff against its own new `children` argument (the "after" side), mirroring `TabView`'s own
    /// `entries`/reconciliation pattern. Also `items()`'s own backing storage (`ListExt` impl
    /// below) — trait-object-typed, mirroring `elwindui_backend_appkit::native_ui::MenuBar`'s own
    /// shape (see its `children` field's own doc comment).
    children: RefCell<Vec<Rc<dyn elwindui_core::ui::MenuBarItemExt>>>,
}

#[elwindui_macros::class]
impl MenuBar {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            inner: InnerMenuBar::new(),
            children: RefCell::new(Vec::new()),
        })
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

fn downcast_menu_bar_item(item: &dyn elwindui_core::ui::MenuBarItemExt) -> &MenuBarItem {
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
    fn new() -> Rc<Self> {
        Rc::new(Self {
            inner: InnerMenuBarItem::new(),
        })
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
    fn new() -> Rc<Self> {
        Rc::new(Self {
            inner: InnerMenu::new(),
            children: RefCell::new(Vec::new()),
        })
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
}

fn downcast_menu_item(item: &dyn elwindui_core::ui::MenuItemExt) -> &MenuItem {
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
    fn new() -> Rc<Self> {
        Rc::new(Self {
            inner: InnerMenuItem::new(),
        })
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
