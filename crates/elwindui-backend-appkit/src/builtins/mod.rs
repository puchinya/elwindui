//! AppKit-backed DSL-facing wrappers for every *native* builtin except `TabView` (see `tab_view.rs`
//! — it's large enough to warrant its own file). `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no wrapper here at all: `elwindui-codegen` builds their
//! `elwindui_core::ui::UIElement` values directly (see docs/elwindui_spec.md 付録H.2), so
//! there's no `Type::new(..)` call site for a wrapper to intercept. Each type below wraps the
//! matching `crate::` (this same crate's raw AppKit) widget and exposes exactly the methods
//! `elwindui-codegen`'s generic conventions call: `Type::new(..)` (construction, args in the paired
//! `elwindui-codegen`'s `builtins.elwind` declaration's `#[param]` order), `set_<attr>` (resync /
//! two-way change-back), `set_on_<event>` (an `on_*` callback), `set_on_<attr>_change` (a
//! `#[two_way]` attribute's change-back), and — for anything embeddable as a child —
//! `into_any_view`.
//!
//! Every DSL-facing struct here is named `XImpl` (`WindowImpl`/`ButtonImpl`/`TextAreaImpl`/
//! `MenuBarImpl`/`MenuBarItemImpl`/`MenuImpl`/`MenuItemImpl`) rather than bare `X`, for two reasons:
//! avoiding a name collision with this crate's own root (which already declares a same-named raw
//! type, e.g. `crate::ButtonImpl`, or — for `Window` specifically — a same-named `pub struct
//! Window`), and so `elwindui-codegen`'s `concrete_type_ident` can treat every hand-written native
//! uniformly (docs/elwindui_spec.md 付録H.2.1a) the same way it already does for composed DSL
//! components. Each implements the matching property-setter trait from `elwindui_core::ui`
//! (`Button`/`TextArea`/`Window`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`) — see that module's own
//! doc comment for why these traits live there instead of being declared separately per backend.
//!
//! `Type::new()` takes **no** arguments — every declared `#[param]` is applied afterward via its
//! own `set_<field>(..)` call (docs/elwindui_spec.md 付録H.2.1a's post-construction setter
//! convention, extended from the common `margin`/`data_context`/attached-property attributes to
//! every builtin property), the same generic mechanism `resync`/two-way change-back already relies on —
//! see `elwindui-codegen`'s `build_component_setters`.

mod tab_view;
pub use tab_view::{TabViewImpl, TabViewItemImpl};

use crate as appkit;
use crate::AnyView;
// Re-exported so `component X inherits Window` call sites can keep importing `Window` from this
// same `builtins` module alongside `WindowImpl`, rather than needing a separate `elwindui_core`
// import just for the (now-shared) marker trait.
pub use elwindui_core::ui::Window;
use elwindui_core::ui::{Button as _, TextArea as _, UIElement};
use std::cell::RefCell;
use std::rc::Rc;

/// `component NotepadWindow inherits Window` ("host composition", docs/elwindui_spec.md 付録H.2.1a)
/// is what actually inherits this — hence the `Impl` rename + paired empty-marker
/// `elwindui_core::ui::Window` trait.
pub struct WindowImpl {
    inner: appkit::Window,
}

impl WindowImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::Window::new() })
    }

    pub fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    pub fn set_menu_bar(&self, menu_bar: Rc<MenuBarImpl>) {
        self.inner.set_menu_bar(&menu_bar.inner);
    }

    pub fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElement>) {
        self.inner.set_content(content);
    }

    pub fn show(&self) {
        self.inner.show();
    }

    pub fn left(&self) -> f32 {
        self.inner.left()
    }

    pub fn set_left(&self, left: f32) {
        self.inner.set_left(left);
    }

    pub fn top(&self) -> f32 {
        self.inner.top()
    }

    pub fn set_top(&self, top: f32) {
        self.inner.set_top(top);
    }

    pub fn width(&self) -> f32 {
        self.inner.width()
    }

    pub fn set_width(&self, width: f32) {
        self.inner.set_width(width);
    }

    pub fn height(&self) -> f32 {
        self.inner.height()
    }

    pub fn set_height(&self, height: f32) {
        self.inner.set_height(height);
    }
}

impl elwindui_core::ui::Window for WindowImpl {
    fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }
    fn set_menu_bar(&self, menu_bar: Rc<dyn elwindui_core::ui::MenuBar>) {
        let menu_bar = menu_bar
            .as_any()
            .downcast_ref::<MenuBarImpl>()
            .expect("Window::set_menu_bar: menu_bar must be this backend's MenuBarImpl");
        self.inner.set_menu_bar(&menu_bar.inner);
    }
    fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElement>) {
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

#[elwindui_macros::class(implements = elwindui_core::ui::TextArea, inherits = appkit::TextAreaImpl)]
pub struct TextAreaImpl {}

#[elwindui_macros::class]
impl TextAreaImpl {
    /// `#[two_way] text` (`TextArea` in `builtins.elwind`) — the change-back half of the binding;
    /// `elwindui_core::ui::TextArea::set_text` is the model→widget half.
    #[inherent]
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.base.set_on_change(callback);
    }

    #[inherent]
    pub fn into_any_view(&self) -> appkit::AnyView {
        self.base.base.handle.clone()
    }

    fn set_text(&self, text: &str) {
        self.base.set_text(text);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.base.set_on_change(callback);
    }

    fn new() -> Rc<Self> {
        Rc::new(Self { base: appkit::create_text_area() })
    }
}

#[elwindui_macros::class(implements = elwindui_core::ui::Button, inherits = appkit::ButtonImpl)]
pub struct ButtonImpl {}

#[elwindui_macros::class]
impl ButtonImpl {
    /// `#[routed] on_click` (`Button` in `builtins.elwind`) is registered directly onto this
    /// widget's own `base` — real since construction (see `new`), and already wired (also in `new`)
    /// to fire `dispatch_routed` starting at this same node.
    #[inherent]
    pub fn register_routed_handler<T: 'static>(&self, name: &'static str, handler: Box<dyn Fn(&T, &elwindui_core::input::RoutedEventArgs)>) {
        self.base.as_ui_element().register_routed_handler(name, handler);
    }

    #[inherent]
    pub fn into_any_view(&self) -> appkit::AnyView {
        self.base.base.handle.clone()
    }

    fn set_enabled(&self, enabled: bool) {
        self.base.set_enabled(enabled);
    }
    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        self.base.set_on_click(callback);
    }
    fn set_text(&self, text: &str) {
        self.base.set_text(text);
    }

    fn new() -> Rc<Self> {
        let base = appkit::create_button();
        let this = Rc::new(Self { base });
        // Wires the real `NSButton` click directly to `dispatch_routed`, once, right here, rather
        // than re-detecting/re-wiring it on every `relayout` the way the old `wire_routed_click`
        // used to (which existed only because the tree node used to be a separate value, built
        // later, external to this widget). Unconditional — `dispatch_routed` already no-ops
        // gracefully when nothing is registered for `"on_click"` at this node or any ancestor
        // (`elwindui-codegen`'s `emit_wiring` registers the actual `#[routed] on_click` handler
        // here, via `register_routed_handler` below, right after this constructor returns).
        {
            let node: Rc<dyn UIElement> = this.clone();
            this.base.set_on_click(Box::new(move || {
                let args = elwindui_core::input::RoutedEventArgs::default();
                elwindui_core::ui::dispatch_routed(&node, "on_click", &(), &args);
            }));
        }
        this
    }
}

pub struct MenuBarImpl {
    inner: appkit::MenuBarImpl,
    /// The currently-installed children, in display order — the "before" side of `set_children`'s
    /// diff against its own new `children` argument (the "after" side), mirroring `TabView`'s own
    /// `entries`/reconciliation pattern.
    children: RefCell<Vec<Rc<MenuBarItemImpl>>>,
}

impl MenuBarImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_bar(), children: RefCell::new(Vec::new()) })
    }

    /// Reconciles the native `NSMenu`'s installed items against `children` by `Rc` pointer
    /// identity (matching `TabView`'s own reconciliation convention) — an item present in both the
    /// old and new list is left alone; one only in the old list is removed
    /// (`elwindui_core::ui::MenuBar::remove_item`); one only in the new list is added
    /// (`add_item`). Safe to call more than once (e.g. a future dynamic menu bar), though today's
    /// only caller (`elwindui-codegen`'s generated construction) calls it exactly once.
    pub fn set_children(&self, children: Vec<Rc<MenuBarItemImpl>>) {
        let mut current = self.children.borrow_mut();
        current.retain(|old| {
            let keep = children.iter().any(|new| Rc::ptr_eq(old, new));
            if !keep {
                self.inner.remove_item(&old.inner);
            }
            keep
        });
        for item in &children {
            if !current.iter().any(|old| Rc::ptr_eq(old, item)) {
                self.inner.add_item(&item.inner);
            }
        }
        *current = children;
    }
}

impl elwindui_core::ui::MenuBar for MenuBarImpl {
    fn add_item(&self, item: &dyn elwindui_core::ui::MenuBarItem) {
        let item = item
            .as_any()
            .downcast_ref::<MenuBarItemImpl>()
            .expect("MenuBar::add_item: item must be this backend's MenuBarItemImpl");
        self.inner.add_item(&item.inner);
    }
    fn remove_item(&self, item: &dyn elwindui_core::ui::MenuBarItem) {
        let item = item
            .as_any()
            .downcast_ref::<MenuBarItemImpl>()
            .expect("MenuBar::remove_item: item must be this backend's MenuBarItemImpl");
        self.inner.remove_item(&item.inner);
    }
}

pub struct MenuBarItemImpl {
    inner: appkit::MenuBarItemImpl,
}

impl MenuBarItemImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_bar_item() })
    }

    pub fn set_submenu(&self, submenu: Rc<MenuImpl>) {
        // `submenu` itself is dropped at the end of this call — the underlying `NSMenu` stays
        // alive regardless, retained by AppKit itself once `NSMenuItem.setSubmenu` runs.
        self.inner.set_submenu(&submenu.inner);
    }
}

impl elwindui_core::ui::MenuBarItem for MenuBarItemImpl {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_submenu(&self, submenu: &dyn elwindui_core::ui::Menu) {
        let submenu = submenu
            .as_any()
            .downcast_ref::<MenuImpl>()
            .expect("MenuBarItem::set_submenu: submenu must be this backend's MenuImpl");
        self.inner.set_submenu(&submenu.inner);
    }
}

pub struct MenuImpl {
    inner: appkit::MenuImpl,
    /// See `MenuBarImpl::children`'s doc comment — same reconciliation pattern.
    children: RefCell<Vec<Rc<MenuItemImpl>>>,
}

impl MenuImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu(), children: RefCell::new(Vec::new()) })
    }

    /// See `MenuBarImpl::set_children`'s doc comment — same reconciliation pattern.
    pub fn set_children(&self, children: Vec<Rc<MenuItemImpl>>) {
        let mut current = self.children.borrow_mut();
        current.retain(|old| {
            let keep = children.iter().any(|new| Rc::ptr_eq(old, new));
            if !keep {
                self.inner.remove_item(&old.inner);
            }
            keep
        });
        for item in &children {
            if !current.iter().any(|old| Rc::ptr_eq(old, item)) {
                self.inner.add_item(&item.inner);
            }
        }
        *current = children;
    }
}

impl elwindui_core::ui::Menu for MenuImpl {
    fn add_item(&self, item: &dyn elwindui_core::ui::MenuItem) {
        let item = item
            .as_any()
            .downcast_ref::<MenuItemImpl>()
            .expect("Menu::add_item: item must be this backend's MenuItemImpl");
        self.inner.add_item(&item.inner);
    }
    fn remove_item(&self, item: &dyn elwindui_core::ui::MenuItem) {
        let item = item
            .as_any()
            .downcast_ref::<MenuItemImpl>()
            .expect("Menu::remove_item: item must be this backend's MenuItemImpl");
        self.inner.remove_item(&item.inner);
    }
}

pub struct MenuItemImpl {
    inner: appkit::MenuItemImpl,
}

impl MenuItemImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_item() })
    }
}

impl elwindui_core::ui::MenuItem for MenuItemImpl {
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
