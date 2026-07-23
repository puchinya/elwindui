//! Implements every `elwindui_core::ui` builtin trait this backend provides, by composing the
//! matching `crate::inner` type (see that module's own doc comment) — each class here is a thin
//! "call into `self.inner`" layer; all genuinely AppKit-specific complexity lives in `inner.rs`.
//! See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no type here at all:
//! they're `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds directly, reflected
//! into real `NSView`s/`CAShapeLayer`s/`CATextLayer`s by `inner::TreeHostView` (used by both
//! `Window`'s content view and `TabView`'s per-tab content area).

use crate::AnyView;
use crate::inner::{
    InnerButton, InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem, InnerPasswordBox,
    InnerTabView, InnerTextArea, InnerTextBox, InnerWindow, TabChipImpl,
};
// Deliberately *not* `use elwindui_core::base::AsAny;` here — see the doc comment on
// `MenuBarItem::set_submenu` (the one place that pattern is explained in full) for why importing
// `AsAny` directly, rather than relying on it as `MenuBarItemExt`/`MenuExt`/etc.'s own supertrait,
// silently breaks every `.as_any().downcast_ref::<T>()` call in this file.
use elwindui_core::ui::UIElementExt;
use objc2::rc::Retained;
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
/// itself. `Window` is deliberately *not* a `UIElement` (no `inherits` here at all) — like WinUI3's
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
        // WinUI3's `TextBox`/AppKit's `NSTextField` are tab stops by default — see
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
        // AppKit's `NSTextField`/WinUI3's `TextBox` are tab stops by default — see
        // docs/elwindui_gui_framework_design.md §5.5.
        self.set_tab_stop(true);
        // Enter-key submit rides the ordinary inherited `on_key_down` (see
        // `elwindui_core::ui::TextBox`'s own doc comment on why this isn't a dedicated field) —
        // wired here, once, the same way `Button::on_constructed` wires `on_click`.
        // `InnerTextBox::set_on_submit` is the one narrowly-scoped AppKit addition that makes a
        // native `NSTextField`'s own Enter key actually reach this dispatch at all (AppKit doesn't
        // otherwise forward its own key handling into `on_key_down` — see
        // `docs/elwindui_gui_framework_design.md` §5.5/§8.1's "known limitation" note).
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

#[elwindui_macros::class(struct_only = elwindui_core::ui::PasswordBoxExt, inherits = crate::NativeControl)]
pub struct PasswordBox {
    inner: InnerPasswordBox,
}

#[elwindui_macros::class]
impl PasswordBox {
    /// `#[two_way] password` (`PasswordBox` in `builtins.elwind`) — the change-back half of the
    /// binding; `elwindui_core::ui::PasswordBox::set_password` is the model→widget half.
    #[inherent]
    pub fn set_on_password_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_password(&self, password: &str) {
        self.inner.set_password(password);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }
    fn set_placeholder(&self, text: &str) {
        self.inner.set_placeholder(text);
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.inner.set_max_length(max_length);
    }
    fn set_reveal_enabled(&self, enabled: bool) {
        self.inner.set_reveal_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerPasswordBox::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // AppKit's `NSSecureTextField`/WinUI3's `PasswordBox` are tab stops by default — see
        // docs/elwindui_gui_framework_design.md §5.5.
        self.set_tab_stop(true);
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
        // Wires the real `NSButton` click directly to `dispatch_routed`, once, right here, rather
        // than re-detecting/re-wiring it on every relayout. Unconditional — `dispatch_routed`
        // already no-ops gracefully when nothing is registered for `"on_click"` at this node or any
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

/// See docs/elwindui_builtins_spec.md 付録Y. `TabView` owns an ordered collection of literal
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
    children: RefCell<Vec<Rc<dyn elwindui_core::ui::TabViewItemExt>>>,
    selected_index: Cell<usize>,
    /// Parallel to `displayed` below — each currently-displayed entry's chip + persistent content
    /// host, in the same order.
    chips: RefCell<Vec<(TabChipImpl, Retained<crate::inner::TreeHostView>)>>,
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
            children: RefCell::new(Vec::new()),
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
        // docs/elwindui_gui_framework_design.md §5.5.
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
        *self.children.borrow_mut() = children
            .into_iter()
            .map(|item| item as Rc<dyn elwindui_core::ui::TabViewItemExt>)
            .collect();
        self.rebuild();
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
            .borrow()
            .iter()
            .find(|item| tab_view_item_key(item) == key)
            .cloned()
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
        let children = self.children.borrow();
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
                        .borrow()
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
                    let children = this.children.borrow();
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
    /// below) — trait-object-typed (`Rc<dyn MenuBarItemExt>`, not the concrete `Rc<MenuBarItem>`
    /// this crate itself always actually constructs) to match `items()`'s `elwindui_core`-shared
    /// signature, the same way `UIElementCollection` stores `Rc<dyn UIElementExt>` rather than a
    /// concrete leaf type.
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
        // AppKit's `InnerMenuBar` has no positional insert — appended, then reconciled into
        // logical position via a fresh `set_children` pass (matching `set_children`'s own
        // reconciliation, not a real native reorder).
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
    // imported `AsAny` — is required here. `AsAny`'s blanket impl (`impl<T: Any> AsAny for T`) also
    // technically applies to `Rc<dyn MenuExt>` *itself* (any `'static` `Sized` type gets it, and an
    // `Rc` is always `Sized` even when its pointee isn't) — this is Rust's well-known "as-any"
    // gotcha (see e.g. lucumr.pocoo.org/2022/1/7/as-any-hack). Method resolution tries the
    // receiver's *own* type before dereferencing, so if `AsAny` is directly `use`-imported in this
    // file, `submenu.as_any()` resolves to *that* blanket impl on `Rc<dyn MenuExt>` — returning a
    // `dyn Any` for the `Rc` smart pointer itself, whose `downcast_ref::<Menu>()` then always fails
    // (confirmed empirically: same address, wrong `TypeId`, every time). Relying on `AsAny` being
    // reachable only as `MenuExt`'s own supertrait (not separately imported — see this file's own
    // top-level `use` block) makes method resolution skip straight past `Rc<dyn MenuExt>` (`AsAny`
    // isn't otherwise in scope for that unrelated type) to `dyn MenuExt` itself, correctly reaching
    // `Menu`'s own vtable slot.
    fn set_submenu(&self, submenu: Rc<dyn elwindui_core::ui::MenuExt>) {
        // `submenu` itself is dropped at the end of this call — the underlying native menu stays
        // alive regardless, retained natively once the submenu is set.
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
