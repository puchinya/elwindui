//! See docs/elwindui_builtins_spec.md 付録Y. This used to be a specialized codegen path
//! (`elwindui-codegen`'s old `emit_tabview_resync`) baked directly into the compiler; it's now an
//! ordinary hand-written widget like any other in this crate — `elwindui-codegen` only ever calls
//! `TabView::new`/`set_tabs`/`set_selected`/`set_closable`/`set_on_select`/`set_on_close`/
//! `set_on_new_tab` generically, the same way it calls `Button::set_enabled` or anything else.
//!
//! `render_label`/`render_content` are plain closures now (compiled from the DSL's own
//! `|doc| ...` syntax generically, in `elwindui-codegen`'s `emit_closure_value`) rather than being
//! inlined into a bespoke resync body — this type just calls them per tab.
//!
//! `TabView` itself is *not* generic: `elwindui-codegen`'s generated struct field for it is a bare
//! `std::rc::Rc<TabView>` (it has no way to know or spell a per-app item type as a struct field's
//! generic argument — every other builtin's shape has a concrete, non-generic Rust type). `new`/
//! `set_tabs` are generic instead, type-erasing each tab's `Rc<T>` into `Rc<dyn Any>` internally
//! (and the render/key closures along with it) — ordinary Rust type inference resolves `T` from
//! the caller's argument at each call site, no turbofish needed either at the call sites codegen
//! generates or here.
//!
//! `key` is accepted (matching `src/shapes/tab_view.elwind`'s declared shape) but not used for
//! reconciliation: chips are still fully rebuilt every `rebuild()` call (they hold no state worth
//! preserving — just a title and a close button), and the content pane's own `(selected, tab
//! count)` staleness check (not a keyed cache) is what avoids destroying the visible document's
//! native text view on every keystroke — there's no tab-reordering command in notepad's command
//! set for a full keyed cache to actually matter for.

use elwindui_backend_appkit as appkit;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub struct TabView {
    inner: appkit::TabView,
    tabs: RefCell<Vec<Rc<dyn Any>>>,
    render_label: Box<dyn Fn(&Rc<dyn Any>) -> String>,
    render_content: Box<dyn Fn(&Rc<dyn Any>) -> appkit::AnyView>,
    selected: Cell<usize>,
    chips: RefCell<Vec<appkit::TabChip>>,
    // `(selected index, tab count)`, not just the index — closing a tab shifts every later index
    // down, so tracking count too forces a content-pane rebuild instead of mistaking a different
    // document for "already showing" at a reused index.
    active: RefCell<Option<(usize, usize)>>,
    on_select: RefCell<Option<Box<dyn Fn(usize)>>>,
    on_close: RefCell<Option<Box<dyn Fn(usize)>>>,
    weak_self: RefCell<Weak<TabView>>,
}

impl TabView {
    pub fn new<T: 'static>(
        tabs: Vec<Rc<T>>,
        _key: Box<dyn Fn(&Rc<T>) -> usize>,
        render_label: Box<dyn Fn(&Rc<T>) -> String>,
        render_content: Box<dyn Fn(&Rc<T>) -> appkit::AnyView>,
        selected: usize,
        _closable: bool,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            inner: appkit::TabView::new(),
            tabs: RefCell::new(erase_tabs(tabs)),
            render_label: erase_render(render_label),
            render_content: erase_render(render_content),
            selected: Cell::new(selected),
            chips: RefCell::new(Vec::new()),
            active: RefCell::new(None),
            on_select: RefCell::new(None),
            on_close: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        });
        *this.weak_self.borrow_mut() = Rc::downgrade(&this);
        // Deliberately doesn't `rebuild()` here: the enclosing generated component's `resync()`
        // (called once, right after every widget is constructed and wired — see
        // `elwindui-codegen`'s `generate_view`) calls `set_tabs`/`set_selected` unconditionally,
        // which triggers the first `rebuild()` — by which point `on_select`/`on_close` are also
        // already wired (wiring runs before that first `resync()` call too).
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

    pub fn set_tabs<T: 'static>(&self, tabs: Vec<Rc<T>>) {
        *self.tabs.borrow_mut() = erase_tabs(tabs);
        self.rebuild();
    }

    pub fn set_selected(&self, selected: usize) {
        self.selected.set(selected);
        self.rebuild();
    }

    /// Not read by `elwindui_backend_appkit::TabChip` today (a close button always shows) —
    /// accepted for interface consistency with the generic resync convention.
    pub fn set_closable(&self, _closable: bool) {}

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }

    /// 1. Fully rebuilds the tab chip strip from the current `tabs` list every time (chips hold no
    ///    state worth preserving across a rebuild).
    /// 2. Only swaps the content pane when `(selected, tabs.len())` actually changed — doing this
    ///    unconditionally would destroy and recreate the native text view on every keystroke
    ///    (since typing itself triggers a resync via the enclosing component), losing cursor
    ///    position/focus each time.
    fn rebuild(&self) {
        let this = self
            .weak_self
            .borrow()
            .upgrade()
            .expect("elwindui: TabView dropped while rebuilding");
        let tabs = self.tabs.borrow();
        let selected = self.selected.get();

        let mut chips = self.chips.borrow_mut();
        for chip in chips.drain(..) {
            self.inner.remove_tab(&chip);
        }
        let mut new_chips = Vec::new();
        for (index, doc) in tabs.iter().enumerate() {
            let label = (self.render_label)(doc);
            let on_select: Box<dyn Fn()> = {
                let this = Rc::clone(&this);
                Box::new(move || {
                    if let Some(cb) = this.on_select.borrow().as_ref() {
                        cb(index);
                    }
                })
            };
            let on_close: Box<dyn Fn()> = {
                let this = Rc::clone(&this);
                Box::new(move || {
                    if let Some(cb) = this.on_close.borrow().as_ref() {
                        cb(index);
                    }
                })
            };
            new_chips.push(self.inner.insert_tab(index, &label, on_select, on_close));
        }
        *chips = new_chips;
        drop(chips);

        let already_showing = *self.active.borrow() == Some((selected, tabs.len()));
        if !already_showing {
            if let Some(doc) = tabs.get(selected) {
                let content = (self.render_content)(doc);
                self.inner.set_content(content);
            }
            *self.active.borrow_mut() = Some((selected, tabs.len()));
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
        let doc: Rc<T> = Rc::clone(doc)
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("elwindui: TabView item type mismatch"));
        f(&doc)
    })
}
