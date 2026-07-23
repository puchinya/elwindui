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
    InnerTextArea, InnerTextBox, InnerWindow,
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
    fn construct(handle: AnyView) -> Self {
        Self {
            base: elwindui_core::ui::UIElement::construct(),
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

    fn construct() -> Self {
        let inner = InnerTextArea::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `TextBox` is a tab stop by default — see
        // docs/elwindui_gui_framework_design.md §5.5.
        self.set_tab_stop(true);
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextBoxExt, inherits = crate::NativeControl)]
pub struct TextBox {
    inner: InnerTextBox,
}

#[elwindui_macros::class]
impl TextBox {
    /// `#[two_way] text` (`TextBox` in `builtins.elwind`) — the change-back half of the binding;
    /// `elwindui_core::ui::TextBox::set_text` is the model→widget half. Mirrors
    /// `TextArea::set_on_text_change` above.
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
    fn set_placeholder(&self, text: &str) {
        self.inner.set_placeholder(text);
    }
    fn set_read_only(&self, read_only: bool) {
        self.inner.set_read_only(read_only);
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.inner.set_max_length(max_length);
    }
    fn set_text_alignment(&self, alignment: elwindui_core::ui::TextAlignment) {
        self.inner.set_text_alignment(alignment);
    }

    fn construct() -> Self {
        let inner = InnerTextBox::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `TextBox` is a tab stop by default — see
        // docs/elwindui_gui_framework_design.md §5.5.
        self.set_tab_stop(true);
        // Enter-key submit rides the ordinary inherited `on_key_down` — see
        // `elwindui_core::ui::TextBox`'s own doc comment on why this isn't a dedicated field, and
        // `InnerTextBox::set_on_submit`'s own doc comment on why WinUI3 (unlike AppKit) needs no
        // special-casing to make a focused `TextBox`'s own Enter key reach this at all.
        let node: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("TextBox::on_constructed: object must already be Rc-constructed");
        self.inner.set_on_submit(Box::new(move || {
            let args = elwindui_core::input::RoutedEventArgs::default();
            let key_args = elwindui_core::input::KeyEventArgs {
                key: elwindui_core::input::Key::Enter,
                modifiers: elwindui_core::input::KeyModifiers::default(),
                is_repeat: false,
            };
            elwindui_core::ui::dispatch_routed(&node, "on_key_down", &key_args, &args);
        }));
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

    fn construct() -> Self {
        let inner = InnerButton::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `Button` is a tab stop by default — see
        // docs/elwindui_gui_framework_design.md §5.5.
        self.set_tab_stop(true);
        // Wires the real XAML click directly to `dispatch_routed`, once, right here, rather than
        // re-detecting/re-wiring it on every relayout. Unconditional — `dispatch_routed` already
        // no-ops gracefully when nothing is registered for `"on_click"` at this node or any
        // ancestor (`elwindui-codegen`'s `emit_wiring` registers the actual `#[routed] on_click`
        // handler here, via `register_routed_handler` above, right after this constructor returns).
        // `owner_rc()` is guaranteed `Some` here — `on_constructed` only ever runs once the
        // enclosing `Rc` is fully built.
        let node: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("Button::on_constructed: object must already be Rc-constructed");
        self.inner.set_on_click(Box::new(move || {
            let args = elwindui_core::input::RoutedEventArgs::default();
            elwindui_core::ui::dispatch_routed(&node, "on_click", &(), &args);
        }));
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y and `elwindui_backend_appkit::native_ui::TabView`'s
/// doc comment for the shared `children` convention (static and transparent dynamic ranges) and
/// why `TabViewItem` — not a bespoke per-mode representation — is the thing every child range
/// normalizes into. `Microsoft.UI.Xaml.Controls.
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
    children: RefCell<Vec<Rc<dyn elwindui_core::ui::TabViewItemExt>>>,
    /// Pointer identities (`Rc::as_ptr`, as `usize`) of the `TabViewItem`s currently reflected as
    /// real `TabViewItem`s, in display order — the "before" side of `rebuild`'s diff against the
    /// current `children` pointers (the "after" side).
    displayed: RefCell<Vec<usize>>,
    /// Not read by this type itself (`set_on_select` passes callbacks straight through to
    /// `crate::inner::InnerTabView`, which has no getter of its own) — tracked here purely so
    /// `selected_item`/`selected_container` can read it back.
    selected_index: Cell<usize>,
    on_close: RefCell<Option<Box<dyn Fn(usize)>>>,
    weak_self: RefCell<Weak<TabView>>,
}

/// The backend-native representation of one declarative `TabViewItem`, whether it was written
/// literally or generated by a transparent dynamic child range.
/// `struct_only = elwindui_core::ui::TabViewItemExt` (a deliberately empty shared trait — see its
/// own doc comment in `elwindui-core`) — mirrors `elwindui_backend_appkit::native_ui::TabViewItem`'s
/// own shape; every method below stays `#[inherent]`, unchanged.
#[elwindui_macros::class(struct_only = elwindui_core::ui::TabViewItemExt)]
pub struct TabViewItem {
    header: RefCell<String>,
    on_header_changed: RefCell<Option<Box<dyn Fn()>>>,
    // Taken (moved into a real `TreeHostPanel`) the first time this `TabViewItem` is inserted as a
    // displayed tab; `None` afterward — see `TabView`'s own doc comment for why that's never a
    // problem here (unlike AppKit's single shared content pane).
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
            children: RefCell::new(Vec::new()),
            displayed: RefCell::new(Vec::new()),
            selected_index: Cell::new(0),
            on_close: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        }
    }

    fn on_constructed(&self) {
        // `owner_rc()` is guaranteed `Some` here (see `Button::on_constructed`'s own doc comment);
        // downcasting the type-erased owner back to this concrete `TabView` is what lets `rebuild`/
        // `attach_header_listener`/`set_on_close` upgrade `weak_self` into a real `Rc<TabView>`
        // later.
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
        // docs/elwindui_gui_framework_design.md §5.5.
        self.set_tab_stop(true);
    }

    #[inherent]
    pub fn set_children(&self, children: Vec<Rc<TabViewItem>>) {
        for item in &children {
            self.attach_header_listener(
                &(Rc::clone(item) as Rc<dyn elwindui_core::ui::TabViewItemExt>),
            );
        }
        *self.children.borrow_mut() = children
            .into_iter()
            .map(|item| item as Rc<dyn elwindui_core::ui::TabViewItemExt>)
            .collect();
        self.rebuild();
    }

    #[inherent]
    pub fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        self.inner.set_on_select(callback);
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
            let entry = this.children.borrow().get(index).cloned();
            let handled = entry.is_some_and(|e| {
                if let Some(cb) = downcast_tab_view_item(&*e).on_close.borrow().as_ref() {
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

    fn children(&self) -> &dyn elwindui_core::ui::ListExt<dyn elwindui_core::ui::TabViewItemExt> {
        self
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
            .borrow()
            .iter()
            .find(|item| tab_view_item_key(item) == key)
            .cloned()
        else {
            return;
        };
        self.inner
            .set_tab_title(index, &downcast_tab_view_item(&*item).header.borrow());
    }

    /// Keyed diff (pointer identity — see `displayed`'s doc comment): removes displayed tabs whose
    /// `TabViewItem` no longer appears in `children`, inserts a real `TabViewItem` (+ that entry's
    /// one-time `content`) for each not-yet-displayed one, and refreshes every displayed tab's
    /// title (labels can change independently of tab identity, e.g. a document's file name). Does
    /// not handle reordering existing tabs (no reorder op is exposed, and nothing in notepad's
    /// command set reorders tabs today — only appends/removes) — an already-displayed tab is left
    /// in its current slot rather than physically moved to match `children`' exact order.
    #[inherent]
    fn rebuild(&self) {
        let children = self.children.borrow();
        let new_keys: Vec<usize> = children.iter().map(tab_view_item_key).collect();
        let mut displayed = self.displayed.borrow_mut();

        for i in (0..displayed.len()).rev() {
            if !new_keys.contains(&displayed[i]) {
                self.inner.remove_tab_at(i);
                displayed.remove(i);
            }
        }

        for (target_index, (key, entry)) in new_keys.iter().zip(children.iter()).enumerate() {
            if !displayed.contains(key) {
                let entry = downcast_tab_view_item(&**entry);
                let label = entry.header.borrow().clone();
                let closable = entry.closable.get();
                let content_host =
                    self.inner
                        .insert_tab(target_index.min(displayed.len()), &label, closable);
                if let Some(content) = entry.content.borrow_mut().take() {
                    content_host.set_tree(content);
                }
                let insertion_index = target_index.min(displayed.len());
                displayed.insert(insertion_index, *key);
            }
        }
        drop(displayed);

        for (index, entry) in children.iter().enumerate() {
            self.inner
                .set_tab_title(index, &downcast_tab_view_item(&**entry).header.borrow());
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
        self.children.borrow_mut().push(item);
        self.rebuild();
    }

    fn insert(&self, index: usize, item: Rc<dyn elwindui_core::ui::TabViewItemExt>) {
        self.attach_header_listener(&item);
        let mut children = self.children.borrow_mut();
        let index = index.min(children.len());
        children.insert(index, item);
        drop(children);
        self.rebuild();
    }

    fn remove(&self, item: &Rc<dyn elwindui_core::ui::TabViewItemExt>) -> bool {
        let mut children = self.children.borrow_mut();
        let Some(index) = children.iter().position(|child| Rc::ptr_eq(child, item)) else {
            return false;
        };
        children.remove(index);
        drop(children);
        self.rebuild();
        true
    }

    fn remove_at(&self, index: usize) -> Rc<dyn elwindui_core::ui::TabViewItemExt> {
        let item = self.children.borrow_mut().remove(index);
        self.rebuild();
        item
    }

    fn clear(&self) {
        self.children.borrow_mut().clear();
        self.rebuild();
    }

    fn len(&self) -> usize {
        self.children.borrow().len()
    }
    fn is_empty(&self) -> bool {
        self.children.borrow().is_empty()
    }
    fn to_vec(&self) -> Vec<Rc<dyn elwindui_core::ui::TabViewItemExt>> {
        self.children.borrow().clone()
    }
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
    fn construct() -> Self {
        Self {
            inner: InnerMenuBar::new(),
            children: RefCell::new(Vec::new()),
        }
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
    fn construct() -> Self {
        Self {
            inner: InnerMenuBarItem::new(),
        }
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
    fn construct() -> Self {
        Self {
            inner: InnerMenu::new(),
            children: RefCell::new(Vec::new()),
        }
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
    fn construct() -> Self {
        Self {
            inner: InnerMenuItem::new(),
        }
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
