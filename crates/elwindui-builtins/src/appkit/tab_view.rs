//! See docs/elwindui_builtins_spec.md 付録Y. `TabView` supports two mutually exclusive child-
//! declaration modes (`elwindui-codegen::validate::check_tab_view_mode` rejects using both or
//! neither):
//! - **Static**: `TabViewItem { .. }` written literally as nested children (WinUI3's XAML style).
//! - **Dynamic**: `items_source` (+ `header_template`/`item_template`), a data-bound collection —
//!   one `TabViewItem` is synthesized per element, keyed by that element's own `Rc<T>` pointer
//!   (`data_context`) so an unchanged item reuses its previously-synthesized `TabViewItem` (and
//!   thus its already-built `content` tree) across `set_items_source` resyncs, instead of a
//!   separate `key` closure.
//!
//! Both modes funnel into the same `entries: Vec<Rc<TabViewItem>>` that `rebuild()` operates over
//! uniformly — `TabViewItem` itself *is* the normalized per-tab representation, not just the
//! static-mode surface syntax.
//!
//! Each entry gets its own persistent `elwindui_backend_appkit::TreeHostView` (created once, the
//! first time that entry — identified by `Rc::as_ptr` pointer identity, the same key
//! `sync_dynamic_entries` reconciles by — appears in `entries`), shown/hidden on selection rather
//! than destroyed and rebuilt — a single shared content pane (this type's earlier design) had no
//! way to restore an already-shown-then-hidden tab's content after switching away from it. Tab
//! chips are reconciled the same way (by the same key), not fully rebuilt every resync, since a
//! chip and its host are always created/destroyed together (`appkit::TabView::insert_tab`/
//! `remove_tab`).

use elwindui_backend_appkit as appkit;
use elwindui_backend_appkit::TabView as _;
use objc2::rc::Retained;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The normalized per-tab representation — written literally in static mode, or synthesized once
/// per `items_source` element in dynamic mode (see module doc comment).
pub struct TabViewItem {
    data_context: Option<Rc<dyn Any>>,
    header: RefCell<String>,
    // Handed to this entry's persistent content host (`TreeHostView::set_tree`) the first time
    // it's actually inserted as a real tab — an `Rc`, so (unlike the `Box` this used to be) it
    // stays readable afterward too, not that anything currently needs to re-read it (the host, not
    // this field, is what keeps the tab's content alive and visible thereafter).
    content: RefCell<Option<std::rc::Rc<dyn elwindui_core::tree::UIElement>>>,
    closable: Cell<bool>,
    on_close: RefCell<Option<Box<dyn Fn()>>>,
}

impl TabViewItem {
    pub fn new<T: 'static>(
        data_context: Option<Rc<T>>,
        header: &str,
        content: std::rc::Rc<dyn elwindui_core::tree::UIElement>,
        closable: Option<bool>,
    ) -> Rc<Self> {
        Self::new_erased(data_context.map(|dc| dc as Rc<dyn Any>), header, content, closable)
    }

    /// Same as `new`, but for a caller (`TabView::sync_dynamic_entries`) that already holds an
    /// erased `Rc<dyn Any>` — `new`'s own `T: 'static` (implicitly `Sized`) can't be instantiated
    /// with the unsized `dyn Any` itself.
    fn new_erased(
        data_context: Option<Rc<dyn Any>>,
        header: &str,
        content: std::rc::Rc<dyn elwindui_core::tree::UIElement>,
        closable: Option<bool>,
    ) -> Rc<Self> {
        Rc::new(Self {
            data_context,
            header: RefCell::new(header.to_string()),
            content: RefCell::new(Some(content)),
            closable: Cell::new(closable.unwrap_or(true)),
            on_close: RefCell::new(None),
        })
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
struct DynamicSource {
    header_template: Box<dyn Fn(&Rc<dyn Any>) -> String>,
    item_template: Box<dyn Fn(&Rc<dyn Any>) -> std::rc::Rc<dyn elwindui_core::tree::UIElement>>,
    closable_default: bool,
}

pub struct TabView {
    inner: appkit::TabViewImpl,
    entries: RefCell<Vec<Rc<TabViewItem>>>,
    dynamic: RefCell<Option<DynamicSource>>,
    selected_index: Cell<usize>,
    /// Parallel to `displayed` below — each currently-displayed entry's chip + persistent content
    /// host, in the same order.
    chips: RefCell<Vec<(appkit::TabChipImpl, Retained<appkit::TreeHostView>)>>,
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

impl TabView {
    pub fn new<T: 'static>(
        children: Vec<Rc<TabViewItem>>,
        items_source: Option<Vec<Rc<T>>>,
        header_template: Option<Box<dyn Fn(&Rc<T>) -> String>>,
        item_template: Option<Box<dyn Fn(&Rc<T>) -> std::rc::Rc<dyn elwindui_core::tree::UIElement>>>,
        closable: Option<bool>,
        selected_index: usize,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            inner: appkit::create_tab_view(),
            entries: RefCell::new(Vec::new()),
            dynamic: RefCell::new(None),
            selected_index: Cell::new(selected_index),
            chips: RefCell::new(Vec::new()),
            displayed: RefCell::new(Vec::new()),
            visible: RefCell::new(None),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        });
        *this.weak_self.borrow_mut() = Rc::downgrade(&this);

        if !children.is_empty() {
            *this.entries.borrow_mut() = children;
        } else if let (Some(items), Some(header_template), Some(item_template)) = (items_source, header_template, item_template) {
            *this.dynamic.borrow_mut() = Some(DynamicSource {
                header_template: erase_render_string(header_template),
                item_template: erase_render(item_template),
                closable_default: closable.unwrap_or(true),
            });
            this.sync_dynamic_entries(erase_items(items));
        }
        // Deliberately doesn't `rebuild()` here — the enclosing generated component's first
        // `resync()` (called once, right after every widget is constructed and wired) calls
        // `set_items_source`/`set_selected_index` unconditionally, which triggers it — by which
        // point `on_select`/`on_close` are also already wired (wiring runs before that first
        // `resync()` call too).
        this
    }

    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_select.borrow_mut() = Some(callback);
    }

    pub fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(callback);
    }

    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_new_tab(callback);
    }

    /// WinUI3's `SelectedItem` concept — not threaded through `on_select` (see
    /// `TabView` in `src/builtins.elwind`'s doc comment on why), so exposed as a plain accessor for
    /// advanced/manual use from hand-written Rust glue code instead.
    pub fn selected_item(&self) -> Option<Rc<dyn Any>> {
        self.entries.borrow().get(self.selected_index.get()).and_then(|e| e.data_context.clone())
    }

    /// WinUI3's `SelectedContainer` concept — see `selected_item`'s doc comment.
    pub fn selected_container(&self) -> Option<Rc<TabViewItem>> {
        self.entries.borrow().get(self.selected_index.get()).cloned()
    }

    /// Dynamic mode only — resyncs `items_source`. Reuses each already-synthesized `TabViewItem`
    /// whose `data_context` is still `Rc::ptr_eq` to the same element (see module doc comment);
    /// only genuinely new elements call `header_template`/`item_template`.
    pub fn set_items_source<T: 'static>(&self, items: Vec<Rc<T>>) {
        self.sync_dynamic_entries(erase_items(items));
        self.rebuild();
    }

    pub fn set_selected_index(&self, selected_index: usize) {
        self.selected_index.set(selected_index);
        self.rebuild();
    }

    /// Not read by `elwindui_backend_appkit::TabChip` today (a close button always shows) —
    /// accepted for interface consistency with the generic resync convention. Only meaningful as
    /// the dynamic-mode default in principle; static mode's per-`TabViewItem` `closable` is what
    /// actually matters once `TabChip` grows real support.
    pub fn set_closable(&self, _closable: bool) {}

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }

    fn sync_dynamic_entries(&self, items: Vec<Rc<dyn Any>>) {
        let dynamic = self.dynamic.borrow();
        let Some(dynamic) = dynamic.as_ref() else { return };
        let mut entries = self.entries.borrow_mut();
        let new_entries: Vec<Rc<TabViewItem>> = items
            .iter()
            .map(|item| {
                entries
                    .iter()
                    .find(|e| e.data_context.as_ref().is_some_and(|dc| Rc::ptr_eq(dc, item)))
                    .cloned()
                    .unwrap_or_else(|| {
                        let header = (dynamic.header_template)(item);
                        let content = (dynamic.item_template)(item);
                        TabViewItem::new_erased(Some(Rc::clone(item)), &header, content, Some(dynamic.closable_default))
                    })
            })
            .collect();
        *entries = new_entries;
    }

    /// Keyed diff (pointer identity — see `displayed`'s doc comment): removes displayed tabs whose
    /// `TabViewItem` no longer appears in `entries` (chip + persistent host together), inserts a
    /// chip + a fresh host (seeded with that entry's one-time `content`) for each not-yet-
    /// displayed one, refreshes every displayed tab's title (labels can change independently of
    /// identity), and shows/hides content hosts so only the selected entry's is visible. Does not
    /// handle reordering existing tabs (no reorder op is exposed, and nothing in notepad's command
    /// set reorders tabs today — only appends/removes).
    fn rebuild(&self) {
        let this = self
            .weak_self
            .borrow()
            .upgrade()
            .expect("elwindui: TabView dropped while rebuilding");
        let entries = self.entries.borrow();
        let selected = self.selected_index.get();
        let new_keys: Vec<usize> = entries.iter().map(|e| Rc::as_ptr(e) as usize).collect();

        let mut chips = self.chips.borrow_mut();
        let mut displayed = self.displayed.borrow_mut();

        for i in (0..displayed.len()).rev() {
            if !new_keys.contains(&displayed[i]) {
                let (chip, host) = chips.remove(i);
                self.inner.remove_tab(&chip, &host);
                displayed.remove(i);
            }
        }

        for (target_index, (key, entry)) in new_keys.iter().zip(entries.iter()).enumerate() {
            if displayed.contains(key) {
                continue;
            }
            let label = entry.header.borrow().clone();
            let key = *key;
            let on_select: Box<dyn Fn()> = {
                let this = Rc::clone(&this);
                Box::new(move || {
                    let index = this.entries.borrow().iter().position(|e| Rc::as_ptr(e) as usize == key);
                    if let (Some(index), Some(cb)) = (index, this.on_select.borrow().as_ref()) {
                        cb(index);
                    }
                })
            };
            let on_close: Box<dyn Fn()> = {
                let this = Rc::clone(&this);
                Box::new(move || {
                    let entries = this.entries.borrow();
                    let Some(index) = entries.iter().position(|e| Rc::as_ptr(e) as usize == key) else { return };
                    let entry = Rc::clone(&entries[index]);
                    drop(entries);
                    // A static `TabViewItem`'s own `on_close` (if set) takes precedence — it's
                    // the per-item declaration in that mode; dynamic mode has none, so its
                    // `TabView`-level `on_close(index)` is used instead.
                    if let Some(cb) = entry.on_close.borrow().as_ref() {
                        cb();
                    } else if let Some(cb) = this.on_close.borrow().as_ref() {
                        cb(index);
                    }
                })
            };
            let insert_at = target_index.min(displayed.len());
            let (chip, host) = self.inner.insert_tab(insert_at, &label, on_select, on_close);
            if let Some(content) = entry.content.borrow().clone() {
                host.set_tree(content);
            }
            chips.insert(insert_at, (chip, host));
            displayed.insert(insert_at, key);
        }

        for (i, key) in displayed.iter().enumerate() {
            if let Some(entry) = entries.iter().find(|e| Rc::as_ptr(e) as usize == *key) {
                chips[i].0.set_title(&entry.header.borrow());
            }
        }

        let selected_key = entries.get(selected).map(|e| Rc::as_ptr(e) as usize);
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
