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
//! Both modes funnel into the same `entries: Vec<Rc<TabViewItemImpl>>` that `rebuild()` operates over
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

use crate as appkit;
use crate::TabView as _;
// `NativeControl` (the struct) needed unqualified: the `as_native_control()` accessor
// `#[elwindui_macros::class(inherits = appkit::NativeTabView)]` generates below is built from
// `NativeTabView`'s own registered `inherits = NativeControl` (stored, and so replayed here, exactly
// as written in `appkit`'s `lib.rs` — bare, not `appkit::`-qualified).
use crate::NativeControl;
use objc2::rc::Retained;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The normalized per-tab representation — written literally in static mode, or synthesized once
/// per `items_source` element in dynamic mode (see module doc comment).
pub struct TabViewItemImpl {
    data_context: RefCell<Option<Rc<dyn Any>>>,
    header: RefCell<String>,
    // Handed to this entry's persistent content host (`TreeHostView::set_tree`) the first time
    // it's actually inserted as a real tab — an `Rc`, so (unlike the `Box` this used to be) it
    // stays readable afterward too, not that anything currently needs to re-read it (the host, not
    // this field, is what keeps the tab's content alive and visible thereafter).
    content: RefCell<Option<std::rc::Rc<dyn elwindui_core::ui::UIElementExt>>>,
    closable: Cell<bool>,
    on_close: RefCell<Option<Box<dyn Fn()>>>,
}

impl TabViewItemImpl {
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
        content: std::rc::Rc<dyn elwindui_core::ui::UIElementExt>,
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

    pub fn set_content(&self, content: std::rc::Rc<dyn elwindui_core::ui::UIElementExt>) {
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
    item_template: Option<Box<dyn Fn(&Rc<dyn Any>) -> std::rc::Rc<dyn elwindui_core::ui::UIElementExt>>>,
    closable_default: bool,
}

impl Default for DynamicSource {
    fn default() -> Self {
        DynamicSource { header_template: None, item_template: None, closable_default: true }
    }
}

#[elwindui_macros::class(inherits = appkit::NativeTabView)]
pub struct TabView {
    entries: RefCell<Vec<Rc<TabViewItemImpl>>>,
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

#[elwindui_macros::class]
impl TabView {
    #[inherent]
    pub fn new() -> Rc<Self> {
        let this = Rc::new(Self {
            base: appkit::create_tab_view(),
            entries: RefCell::new(Vec::new()),
            dynamic: RefCell::new(None),
            selected_index: Cell::new(0),
            chips: RefCell::new(Vec::new()),
            displayed: RefCell::new(Vec::new()),
            visible: RefCell::new(None),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        });
        *this.weak_self.borrow_mut() = Rc::downgrade(&this);
        this
    }

    /// Static mode: the literal `TabViewItem { .. }` children (mutually exclusive with
    /// `set_items_source`'s dynamic mode — see module doc comment). Populates `entries`
    /// immediately, unlike the dynamic-mode setters below (whose own values only take effect once
    /// `set_items_source` — always called again by the enclosing generated component's first
    /// `resync()`, see that method's doc comment — actually reconciles them).
    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<TabViewItemImpl>>) {
        if !children.is_empty() {
            *self.entries.borrow_mut() = children;
        }
    }

    /// Dynamic mode: establishes `self.dynamic`'s `header_template`/`item_template`.
    /// `elwindui-codegen`'s setter-based construction (docs/elwindui_spec.md 付録H.2.1a) combines
    /// `items_source`/`header_template`/`item_template` into this one call rather than three
    /// independent ones — all three share the same generic `T` (the `.elwind` viewmodel's item
    /// type), and Rust can only infer a generic call's type parameter from *that call*'s own
    /// arguments; `header_template`/`item_template`'s closure bodies (e.g. `|doc| doc.file_name()`)
    /// carry no concrete type on their own, so they need `items` (concretely `Vec<Rc<Document>>`,
    /// say) in the very same call to pin `T` down (see `build_component_setters`'s own doc comment
    /// for the codegen side of this).
    ///
    /// `items` itself is **not** used to populate `entries` here — only `header_template`/
    /// `item_template` are established. Real population happens on the enclosing generated
    /// component's first `resync()`, which unconditionally calls `set_items_source` right after
    /// every dynamic-mode setter (including `set_closable`) has already run — synthesizing entries
    /// here instead would risk baking in `DynamicSource::default()`'s `closable_default` (`true`)
    /// before this use site's real `closable:` attribute (if any) has been applied.
    #[inherent]
    pub fn set_dynamic_source<T: 'static>(
        &self,
        items: Vec<Rc<T>>,
        header_template: Box<dyn Fn(&Rc<T>) -> String>,
        item_template: Box<dyn Fn(&Rc<T>) -> std::rc::Rc<dyn elwindui_core::ui::UIElementExt>>,
    ) {
        let mut dynamic = self.dynamic.borrow_mut();
        let entry = dynamic.get_or_insert_with(DynamicSource::default);
        entry.header_template = Some(erase_render_string(header_template));
        entry.item_template = Some(erase_render(item_template));
        drop(dynamic);
        let _ = items;
    }

    /// The default `closable` for a synthesized `TabViewItem` in dynamic mode (static mode's own
    /// `TabViewItem::set_closable` is what matters there instead — see `set_children`).
    #[inherent]
    pub fn set_closable(&self, closable: bool) {
        self.dynamic.borrow_mut().get_or_insert_with(DynamicSource::default).closable_default = closable;
    }

    #[inherent]
    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_select.borrow_mut() = Some(callback);
    }

    #[inherent]
    pub fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(callback);
    }

    #[inherent]
    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.base.set_on_new_tab(callback);
    }

    /// WinUI3's `SelectedItem` concept — not threaded through `on_select` (see
    /// `TabView` in `src/builtins.elwind`'s doc comment on why), so exposed as a plain accessor for
    /// advanced/manual use from hand-written Rust glue code instead.
    #[inherent]
    pub fn selected_item(&self) -> Option<Rc<dyn Any>> {
        self.entries.borrow().get(self.selected_index.get()).and_then(|e| e.data_context.borrow().clone())
    }

    /// WinUI3's `SelectedContainer` concept — see `selected_item`'s doc comment.
    #[inherent]
    pub fn selected_container(&self) -> Option<Rc<TabViewItemImpl>> {
        self.entries.borrow().get(self.selected_index.get()).cloned()
    }

    /// Dynamic mode only — resyncs `items_source`. Reuses each already-synthesized `TabViewItem`
    /// whose `data_context` is still `Rc::ptr_eq` to the same element (see module doc comment);
    /// only genuinely new elements call `header_template`/`item_template`. Also the construction-
    /// time trigger in practice (see `set_header_template`'s doc comment): `elwindui-codegen`'s own
    /// setter-based construction calls this once (before `header_template`/`item_template` are
    /// necessarily set), then the enclosing generated component's first `resync()` calls it again
    /// unconditionally — by which point every dynamic-mode setter has run, so `sync_dynamic_entries`
    /// can actually synthesize entries and `rebuild()` has real content to show.
    #[inherent]
    pub fn set_items_source<T: 'static>(&self, items: Vec<Rc<T>>) {
        self.sync_dynamic_entries(erase_items(items));
        self.rebuild();
    }

    #[inherent]
    pub fn set_selected_index(&self, selected_index: usize) {
        self.selected_index.set(selected_index);
        self.rebuild();
    }

    #[inherent]
    pub fn into_any_view(&self) -> appkit::AnyView {
        self.base.base.handle.clone()
    }

    #[inherent]
    fn sync_dynamic_entries(&self, items: Vec<Rc<dyn Any>>) {
        let dynamic = self.dynamic.borrow();
        let Some(dynamic) = dynamic.as_ref() else { return };
        let (Some(header_template), Some(item_template)) = (&dynamic.header_template, &dynamic.item_template) else { return };
        let mut entries = self.entries.borrow_mut();
        let new_entries: Vec<Rc<TabViewItemImpl>> = items
            .iter()
            .map(|item| {
                match entries.iter().find(|e| e.data_context.borrow().as_ref().is_some_and(|dc| Rc::ptr_eq(dc, item))) {
                    // Re-run `header_template` even for a reused entry — the label (e.g. a
                    // document's file name) can change independently of the item's own identity,
                    // and `entry.header` is otherwise never refreshed after construction.
                    Some(existing) => {
                        existing.set_header(&header_template(item));
                        Rc::clone(existing)
                    }
                    None => {
                        let header = header_template(item);
                        let content = item_template(item);
                        TabViewItemImpl::new_erased(Some(Rc::clone(item)), &header, content, Some(dynamic.closable_default))
                    }
                }
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
    #[inherent]
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
                self.base.remove_tab(&chip, &host);
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
            let (chip, host) = self.base.insert_tab(insert_at, &label, on_select, on_close);
            if let Some(content) = entry.content.borrow().clone() {
                host.set_tree(content);
            }
            chips.insert(insert_at, (chip, host));
            displayed.insert(insert_at, key);
        }

        let selected_key = entries.get(selected).map(|e| Rc::as_ptr(e) as usize);

        for (i, key) in displayed.iter().enumerate() {
            if let Some(entry) = entries.iter().find(|e| Rc::as_ptr(e) as usize == *key) {
                chips[i].0.set_title(&entry.header.borrow());
            }
            chips[i].0.set_selected(Some(*key) == selected_key);
        }

        let mut visible = self.visible.borrow_mut();
        if *visible != selected_key {
            if let Some(old_key) = *visible {
                if let Some(pos) = displayed.iter().position(|k| *k == old_key) {
                    self.base.set_tab_content_visible(&chips[pos].1, false);
                }
            }
            if let Some(new_key) = selected_key {
                if let Some(pos) = displayed.iter().position(|k| *k == new_key) {
                    self.base.set_tab_content_visible(&chips[pos].1, true);
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
    f: Box<dyn Fn(&Rc<T>) -> std::rc::Rc<dyn elwindui_core::ui::UIElementExt>>,
) -> Box<dyn Fn(&Rc<dyn Any>) -> std::rc::Rc<dyn elwindui_core::ui::UIElementExt>> {
    Box::new(move |item: &Rc<dyn Any>| {
        let item: Rc<T> = Rc::clone(item).downcast::<T>().unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&item)
    })
}
