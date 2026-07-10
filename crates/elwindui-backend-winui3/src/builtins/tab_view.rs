//! See docs/elwindui_builtins_spec.md 付録Y and `elwindui_backend_appkit::builtins::tab_view`'s
//! doc comment for the overall two-mode convention (static `TabViewItem` children vs. `items_source` +
//! `header_template`/`item_template`) and why `TabViewItem` — not a bespoke per-mode
//! representation — is the thing both modes normalize into. This isn't a line-for-line port of
//! that file, though: WinUI3's `Microsoft.UI.Xaml.Controls.TabView` is a real native tabbed-
//! document control (unlike AppKit, which has none, hence that crate's hand-rolled
//! `TabChip`/`TabStrip`), and each `TabViewItem`'s `Content` here is a live `TreeHostPanel` holding
//! that tab's whole widget tree — recreating it on every resync (as AppKit's *chips*, which are
//! cheap, safely do) would reset a document's `TextArea` (lost cursor/focus) on every keystroke.
//! Unlike AppKit, this backend has **no** "content already shown once" limitation for static mode:
//! a `TabViewItem`'s `content` is moved into its own persistent `TreeHostPanel` exactly once, when
//! that `TabViewItem` is first inserted as a real native tab — it is never subsequently discarded
//! by selecting a different tab (`Controls::TabView` shows/hides each item's own `Content`
//! natively), so there's nothing to restore.

use crate as winui3;
use crate::TabView as _;
use elwindui_core::tree::UIElement;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The normalized per-tab representation — written literally in static mode, or synthesized once
/// per `items_source` element in dynamic mode (see module doc comment).
pub struct TabViewItem {
    data_context: RefCell<Option<Rc<dyn Any>>>,
    header: RefCell<String>,
    // Taken (moved into a real `TreeHostPanel`) the first time this `TabViewItem` is inserted as a
    // displayed tab; `None` afterward — see the module doc comment for why that's never a problem
    // here (unlike AppKit's single shared content pane).
    content: RefCell<Option<std::rc::Rc<dyn elwindui_core::tree::UIElement>>>,
    closable: Cell<bool>,
    on_close: RefCell<Option<Box<dyn Fn()>>>,
}

impl TabViewItem {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            data_context: RefCell::new(None),
            header: RefCell::new(String::new()),
            content: RefCell::new(None),
            closable: Cell::new(true),
            on_close: RefCell::new(None),
        })
    }

    /// Same shape as `sync_dynamic_entries`'s own erased construction need — kept as a free
    /// function (not a method) since it builds a whole `Self` from an already-erased
    /// `Rc<dyn Any>`, unlike `set_data_context<T>` (a real setter, generic over `T`).
    fn new_erased(
        data_context: Option<Rc<dyn Any>>,
        header: &str,
        content: std::rc::Rc<dyn elwindui_core::tree::UIElement>,
        closable: Option<bool>,
    ) -> Rc<Self> {
        Rc::new(Self {
            data_context: RefCell::new(data_context),
            header: RefCell::new(header.to_string()),
            content: RefCell::new(Some(content)),
            closable: Cell::new(closable.unwrap_or(true)),
            on_close: RefCell::new(None),
        })
    }

    pub fn set_data_context<T: 'static>(&self, data_context: Rc<T>) {
        *self.data_context.borrow_mut() = Some(data_context as Rc<dyn Any>);
    }

    pub fn set_header(&self, header: &str) {
        *self.header.borrow_mut() = header.to_string();
    }

    pub fn set_content(&self, content: std::rc::Rc<dyn elwindui_core::tree::UIElement>) {
        *self.content.borrow_mut() = Some(content);
    }

    pub fn set_closable(&self, closable: bool) {
        self.closable.set(closable);
    }

    pub fn set_on_close(&self, callback: Box<dyn Fn()>) {
        *self.on_close.borrow_mut() = Some(callback);
    }
}

/// Only set in dynamic mode — `None` for a `TabView` built from static `TabViewItem` children.
/// `header_template`/`item_template` are themselves `Option` (unlike the old all-at-once
/// constructor's shape) since `elwindui-codegen`'s setter-based construction (docs/elwindui_spec.md
/// 付録H.2.1a) now supplies `items_source`/`header_template`/`item_template` via three separate
/// `set_*` calls rather than one combined argument list — `sync_dynamic_entries` only actually
/// synthesizes entries once both are present (see that method).
struct DynamicSource {
    header_template: Option<Box<dyn Fn(&Rc<dyn Any>) -> String>>,
    item_template: Option<Box<dyn Fn(&Rc<dyn Any>) -> std::rc::Rc<dyn elwindui_core::tree::UIElement>>>,
    closable_default: bool,
}

impl Default for DynamicSource {
    fn default() -> Self {
        DynamicSource { header_template: None, item_template: None, closable_default: true }
    }
}

pub struct TabView {
    inner: winui3::TabViewImpl,
    entries: RefCell<Vec<Rc<TabViewItem>>>,
    dynamic: RefCell<Option<DynamicSource>>,
    /// Pointer identities (`Rc::as_ptr`, as `usize`) of the `TabViewItem`s currently reflected as
    /// real `TabViewItem`s, in display order — the "before" side of `rebuild`'s diff against
    /// `entries`' current pointers (the "after" side). Renamed from the old `displayed_keys` now
    /// that pointer identity *is* the key (no separate `key` closure).
    displayed: RefCell<Vec<usize>>,
    /// Not read by this type itself (`set_on_select` passes callbacks straight through to
    /// `elwindui_backend_winui3::TabView`, which has no getter of its own) — tracked here purely
    /// so `selected_item`/`selected_container` can read it back.
    selected_index: Cell<usize>,
    on_close: RefCell<Option<Box<dyn Fn(usize)>>>,
    weak_self: RefCell<Weak<TabView>>,
}

impl UIElement for TabView {
    fn base(&self) -> &elwindui_core::tree::UIElementImpl {
        self.inner.base()
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.inner.visual_children()
    }
    fn measure_override(&self, available: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> elwindui_core::layout::Size {
        self.inner.measure_override(available, child_sizes)
    }
    fn arrange_override(&self, final_size: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> Vec<elwindui_core::layout::Rect> {
        self.inner.arrange_override(final_size, child_sizes)
    }
    fn as_native_control(&self) -> Option<&dyn Any> {
        self.inner.as_native_control()
    }
}

impl TabView {
    pub fn new() -> Rc<Self> {
        let this = Rc::new(Self {
            inner: winui3::create_tab_view(),
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
    /// `set_items_source`'s dynamic mode — see module doc comment).
    pub fn set_children(&self, children: Vec<Rc<TabViewItem>>) {
        if !children.is_empty() {
            *self.entries.borrow_mut() = children;
        }
    }

    /// Dynamic mode: establishes `self.dynamic`'s `header_template`/`item_template` — see
    /// `elwindui_backend_appkit::builtins::tab_view::TabView::set_dynamic_source`'s doc comment for
    /// why `items_source`/`header_template`/`item_template` are combined into this one call rather
    /// than three independent ones (shared generic `T`, and `items` itself isn't used to populate
    /// `entries` here — that happens on the enclosing component's first `resync()` instead).
    pub fn set_dynamic_source<T: 'static>(
        &self,
        items: Vec<Rc<T>>,
        header_template: Box<dyn Fn(&Rc<T>) -> String>,
        item_template: Box<dyn Fn(&Rc<T>) -> std::rc::Rc<dyn elwindui_core::tree::UIElement>>,
    ) {
        let mut dynamic = self.dynamic.borrow_mut();
        let entry = dynamic.get_or_insert_with(DynamicSource::default);
        entry.header_template = Some(erase_render_string(header_template));
        entry.item_template = Some(erase_render(item_template));
        drop(dynamic);
        let _ = items;
    }

    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_select(callback);
    }

    /// WinUI3's `SelectedItem` concept — not threaded through `on_select` (see
    /// `TabView` in `src/builtins.elwind`'s doc comment on why), so exposed as a plain accessor for
    /// advanced/manual use from hand-written Rust glue code instead.
    pub fn selected_item(&self) -> Option<Rc<dyn Any>> {
        self.entries.borrow().get(self.selected_index.get()).and_then(|e| e.data_context.borrow().clone())
    }

    /// WinUI3's `SelectedContainer` concept — see `selected_item`'s doc comment.
    pub fn selected_container(&self) -> Option<Rc<TabViewItem>> {
        self.entries.borrow().get(self.selected_index.get()).cloned()
    }

    /// A static `TabViewItem`'s own `on_close` (if set) takes precedence — it's the per-item
    /// declaration in that mode; dynamic mode has none, so its `TabView`-level `on_close(index)`
    /// is used instead (same precedence as `elwindui-builtins::appkit::tab_view`).
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

    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_new_tab(callback);
    }

    /// Dynamic mode only — resyncs `items_source`. Reuses each already-synthesized `TabViewItem`
    /// whose `data_context` is still `Rc::ptr_eq` to the same element (see module doc comment);
    /// only genuinely new elements call `header_template`/`item_template`.
    pub fn set_items_source<T: 'static>(&self, items: Vec<Rc<T>>) {
        self.sync_dynamic_entries(erase_items(items));
        self.rebuild();
    }

    /// Unlike `elwindui-builtins::appkit::tab_view` (where selecting a tab means manually swapping
    /// the single visible content pane, done inside `rebuild`), `Controls::TabView` already shows/
    /// hides each `TabViewItem`'s own persistent `Content` based on `SelectedIndex` natively — so
    /// this is just a straight passthrough, no rebuild needed.
    pub fn set_selected_index(&self, selected_index: usize) {
        self.selected_index.set(selected_index);
        self.inner.set_selected_index(selected_index);
    }

    /// Dynamic mode, part 3 of 3 — the default `closable` for a synthesized `TabViewItem` (static
    /// mode's own `TabViewItem::set_closable` is what matters there instead). See
    /// `set_header_template`'s doc comment.
    pub fn set_closable(&self, closable: bool) {
        self.dynamic.borrow_mut().get_or_insert_with(DynamicSource::default).closable_default = closable;
    }

    pub fn into_any_view(&self) -> winui3::AnyView {
        self.inner.base.handle.clone()
    }

    fn sync_dynamic_entries(&self, items: Vec<Rc<dyn Any>>) {
        let dynamic = self.dynamic.borrow();
        let Some(dynamic) = dynamic.as_ref() else { return };
        let (Some(header_template), Some(item_template)) = (&dynamic.header_template, &dynamic.item_template) else { return };
        let mut entries = self.entries.borrow_mut();
        let new_entries: Vec<Rc<TabViewItem>> = items
            .iter()
            .map(|item| {
                entries
                    .iter()
                    .find(|e| e.data_context.borrow().as_ref().is_some_and(|dc| Rc::ptr_eq(dc, item)))
                    .cloned()
                    .unwrap_or_else(|| {
                        let header = header_template(item);
                        let content = item_template(item);
                        TabViewItem::new_erased(Some(Rc::clone(item)), &header, content, Some(dynamic.closable_default))
                    })
            })
            .collect();
        *entries = new_entries;
    }

    /// Keyed diff (pointer identity — see `displayed`'s doc comment): removes displayed tabs whose
    /// `TabViewItem` no longer appears in `entries`, inserts a real `TabViewItem` (+ that entry's
    /// one-time `content`) for each not-yet-displayed one, and refreshes every displayed tab's
    /// title (labels can change independently of tab identity, e.g. a document's file name).
    /// Does not handle reordering existing tabs (no reorder op is exposed, and nothing in
    /// notepad's command set reorders tabs today — only appends/removes) — an already-displayed
    /// tab is left in its current slot rather than physically moved to match `entries`' exact
    /// order.
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
                let content_host = self.inner.insert_tab(target_index.min(displayed.len()), &label, closable);
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
fn erase_render_string<T: 'static>(f: Box<dyn Fn(&Rc<T>) -> String>) -> Box<dyn Fn(&Rc<dyn Any>) -> String> {
    Box::new(move |item: &Rc<dyn Any>| {
        let item: Rc<T> = Rc::clone(item).downcast::<T>().unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&item)
    })
}

/// Same as `erase_render_string`, for `item_template`'s `Rc<dyn UIElement>`-returning shape.
fn erase_render<T: 'static>(
    f: Box<dyn Fn(&Rc<T>) -> std::rc::Rc<dyn elwindui_core::tree::UIElement>>,
) -> Box<dyn Fn(&Rc<dyn Any>) -> std::rc::Rc<dyn elwindui_core::tree::UIElement>> {
    Box::new(move |item: &Rc<dyn Any>| {
        let item: Rc<T> = Rc::clone(item).downcast::<T>().unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&item)
    })
}
