//! See docs/elwindui_builtins_spec.md 付録Y and `elwindui-builtins::appkit::tab_view`'s doc
//! comment for the overall convention (`elwindui-codegen` only ever calls `TabView::new`/
//! `set_tabs`/`set_selected`/`set_closable`/`set_on_select`/`set_on_close`/`set_on_new_tab`
//! generically). This isn't a line-for-line port of that file, though — WinUI3's
//! `Microsoft.UI.Xaml.Controls.TabView` is a real native tabbed-document control (unlike AppKit,
//! which has none, hence `elwindui-backend-appkit`'s hand-rolled `TabChip`/`TabStrip`), and each
//! `TabViewItem`'s `Content` here is a live `TreeHostPanel` holding that tab's whole widget tree —
//! recreating it on every resync (as AppKit's *chips*, which are cheap, safely do) would reset a
//! document's `TextArea` (lost cursor/focus) on every keystroke. So `rebuild` below does a real
//! keyed diff using `key` (accepted but deliberately *unused* for reconciliation on the AppKit
//! side, per that file's doc comment — here it's load-bearing) instead of a full teardown/rebuild.

use elwindui_backend_winui3 as winui3;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct TabView {
    inner: winui3::TabView,
    tabs: RefCell<Vec<Rc<dyn Any>>>,
    key: Box<dyn Fn(&Rc<dyn Any>) -> usize>,
    render_label: Box<dyn Fn(&Rc<dyn Any>) -> String>,
    render_content: Box<dyn Fn(&Rc<dyn Any>) -> Box<dyn elwindui_core::tree::UIElement>>,
    /// Keys of the tabs currently reflected as real `TabViewItem`s, in display order — the "before"
    /// side of `rebuild`'s diff against `tabs`' current keys (the "after" side).
    displayed_keys: RefCell<Vec<usize>>,
    closable: Cell<bool>,
}

impl TabView {
    pub fn new<T: 'static>(
        tabs: Vec<Rc<T>>,
        key: Box<dyn Fn(&Rc<T>) -> usize>,
        render_label: Box<dyn Fn(&Rc<T>) -> String>,
        render_content: Box<dyn Fn(&Rc<T>) -> Box<dyn elwindui_core::tree::UIElement>>,
        selected: usize,
        closable: bool,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            inner: winui3::TabView::new(),
            tabs: RefCell::new(erase_tabs(tabs)),
            key: erase_render(key),
            render_label: erase_render(render_label),
            render_content: erase_render(render_content),
            displayed_keys: RefCell::new(Vec::new()),
            closable: Cell::new(closable),
        });
        // Deliberately doesn't `rebuild()`/`set_selected_index` here — the enclosing generated
        // component's `resync()` (called once, right after every widget is constructed and wired)
        // calls `set_tabs`/`set_selected` unconditionally, which triggers the first real build —
        // by which point `on_select`/`on_close` are also already wired (wiring runs before that
        // first `resync()` call too), matching `elwindui-builtins::appkit::tab_view`'s convention.
        let _ = selected;
        this
    }

    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_select(callback);
    }

    pub fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_close(callback);
    }

    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_new_tab(callback);
    }

    pub fn set_tabs<T: 'static>(&self, tabs: Vec<Rc<T>>) {
        *self.tabs.borrow_mut() = erase_tabs(tabs);
        self.rebuild();
    }

    /// Unlike `elwindui-builtins::appkit::tab_view` (where selecting a tab means manually swapping
    /// the single visible content pane, done inside `rebuild`), `Controls::TabView` already shows/
    /// hides each `TabViewItem`'s own persistent `Content` based on `SelectedIndex` natively — so
    /// this is just a straight passthrough, no rebuild needed.
    pub fn set_selected(&self, selected: usize) {
        self.inner.set_selected_index(selected);
    }

    pub fn set_closable(&self, closable: bool) {
        self.closable.set(closable);
    }

    pub fn into_any_view(&self) -> winui3::AnyView {
        winui3::AnyView::from(self.inner.clone())
    }

    /// Keyed diff: removes displayed tabs whose key no longer appears in `tabs`, inserts a real
    /// `TabViewItem` (+ freshly-built content tree) for each key not yet displayed, and refreshes
    /// every displayed tab's title (labels can change independently, e.g. a document's file name).
    /// Does not handle reordering existing tabs (no reorder op is exposed, and nothing in
    /// notepad's command set reorders tabs today — only appends/removes) — an already-displayed
    /// tab is left in its current slot rather than physically moved to match `tabs`' exact order.
    fn rebuild(&self) {
        let tabs = self.tabs.borrow();
        let closable = self.closable.get();
        let new_keys: Vec<usize> = tabs.iter().map(|t| (self.key)(t)).collect();
        let mut displayed = self.displayed_keys.borrow_mut();

        for i in (0..displayed.len()).rev() {
            if !new_keys.contains(&displayed[i]) {
                self.inner.remove_tab_at(i);
                displayed.remove(i);
            }
        }

        for (target_index, (key, tab)) in new_keys.iter().zip(tabs.iter()).enumerate() {
            if !displayed.contains(key) {
                let label = (self.render_label)(tab);
                let content_host = self.inner.insert_tab(target_index.min(displayed.len()), &label, closable);
                content_host.set_tree((self.render_content)(tab));
                displayed.insert(target_index.min(displayed.len()), *key);
            }
        }
        drop(displayed);

        for (index, tab) in tabs.iter().enumerate() {
            self.inner.set_tab_title(index, &(self.render_label)(tab));
        }
    }
}

fn erase_tabs<T: 'static>(tabs: Vec<Rc<T>>) -> Vec<Rc<dyn Any>> {
    tabs.into_iter().map(|t| t as Rc<dyn Any>).collect()
}

/// Wraps a caller-supplied `Fn(&Rc<T>) -> R` so it can be stored as `Fn(&Rc<dyn Any>) -> R` —
/// downcasting back to the concrete `T` on every call. The `Rc<dyn Any>`s it's ever actually
/// called with all come from `erase_tabs::<T>` for this same `TabView`, so the downcast always
/// succeeds.
fn erase_render<T: 'static, R: 'static>(f: Box<dyn Fn(&Rc<T>) -> R>) -> Box<dyn Fn(&Rc<dyn Any>) -> R> {
    Box::new(move |doc: &Rc<dyn Any>| {
        let doc: Rc<T> = Rc::clone(doc).downcast::<T>().unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&doc)
    })
}
