//! AppKit-backed implementations of every *native* builtin except `TabView` (see `tab_view.rs` —
//! it's large enough to warrant its own file). `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no wrapper here at all: `elwindui-codegen` builds their
//! `elwindui_core::tree::UIElement` values directly (see docs/elwindui_spec.md 付録H.2), so
//! there's no `Type::new(..)` call site for a wrapper to intercept. Each type below wraps the
//! matching `elwindui_backend_appkit` widget and exposes exactly the methods `elwindui-codegen`'s
//! generic conventions call: `Type::new(..)` (construction, args in the paired
//! `src/builtins.elwind` declaration's `#[param]` order), `set_<attr>` (resync / two-way
//! change-back), `set_on_<event>` (an `on_*` callback), `set_on_<attr>_change` (a `#[two_way]`
//! attribute's change-back), and — for anything embeddable as a child — `into_any_view`.

mod tab_view;
pub use tab_view::{TabView, TabViewItem};

use elwindui_backend_appkit as appkit;
use elwindui_backend_appkit::{Button as _, MenuItem as _, TextArea as _};
use std::rc::Rc;

pub struct Window {
    inner: appkit::Window,
}

impl Window {
    pub fn new(
        title: &str,
        menu_bar: Option<Rc<MenuBar>>,
        content: Rc<dyn elwindui_core::tree::UIElement>,
    ) -> Rc<Self> {
        let inner = appkit::Window::new(title);
        inner.set_content(content);
        if let Some(menu_bar) = &menu_bar {
            inner.set_menu_bar(&menu_bar.inner);
        }
        Rc::new(Self { inner })
    }

    pub fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    pub fn show(&self) {
        self.inner.show();
    }
}

pub struct TextArea {
    inner: appkit::TextAreaImpl,
}

impl TextArea {
    pub fn new(text: &str) -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_text_area(text) })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    /// `#[two_way] text` (`TextArea` in `src/builtins.elwind`) — the change-back half of the binding;
    /// `set_text` above is the model→widget half.
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct Button {
    inner: appkit::ButtonImpl,
    /// `#[routed] on_click` (`Button` in `src/builtins.elwind`) is registered here at `Button`'s own
    /// construction/wiring time — long before the `NativeControl` tree node wrapping it exists
    /// (tree construction is bottom-up: children before parents). `elwindui-codegen`'s
    /// `into_node_if_needed` shares this same `Rc` into that node's `UIElementBase.routed_handlers`
    /// once it's built, so `dispatch_routed` finds these handlers when bubbling starts here. The
    /// real `NSButton` click itself isn't wired to call them directly — see
    /// `elwindui_backend_appkit::TreeHostView::relayout`, which wires the real click to
    /// `dispatch_routed` starting at this element's own tree node once that node exists.
    routed_handlers: elwindui_core::tree::RoutedHandlers,
}

impl Button {
    pub fn new(text: &str, enabled: Option<bool>) -> Rc<Self> {
        let inner = appkit::create_button(text);
        if let Some(enabled) = enabled {
            inner.set_enabled(enabled);
        }
        Rc::new(Self { inner, routed_handlers: Default::default() })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    /// See `routed_handlers`'s own doc comment.
    pub fn register_routed_handler<T: 'static>(&self, name: &'static str, handler: Box<dyn Fn(&T, &elwindui_core::input::RoutedEventArgs)>) {
        elwindui_core::tree::register_routed_handler(&self.routed_handlers, name, handler);
    }

    /// See `routed_handlers`'s own doc comment — shared into the `NativeControl` wrapping this
    /// `Button` by `elwindui-codegen`'s `into_node_if_needed`.
    pub fn routed_handlers(&self) -> elwindui_core::tree::RoutedHandlers {
        self.routed_handlers.clone()
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct MenuBar {
    inner: appkit::MenuBarImpl,
}

impl MenuBar {
    pub fn new(children: Vec<Rc<MenuBarItem>>) -> Rc<Self> {
        let items = children.iter().map(|c| c.inner.clone()).collect();
        Rc::new(Self { inner: appkit::create_menu_bar(items) })
    }
}

pub struct MenuBarItem {
    inner: appkit::MenuBarItemImpl,
}

impl MenuBarItem {
    pub fn new(text: &str, submenu: Rc<Menu>) -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_bar_item(text, submenu.inner.clone()) })
    }

    /// See `MenuItem::set_text`'s doc comment — same reason (no title setter in
    /// `elwindui_backend_appkit`, `text` is effectively static in practice, and the generic resync
    /// convention still expects the method to exist).
    pub fn set_text(&self, _text: &str) {}
}

pub struct Menu {
    inner: appkit::MenuImpl,
}

impl Menu {
    pub fn new(children: Vec<Rc<MenuItem>>) -> Rc<Self> {
        let items = children.iter().map(|c| c.inner.clone()).collect();
        Rc::new(Self { inner: appkit::create_menu(items) })
    }
}

pub struct MenuItem {
    inner: appkit::MenuItemImpl,
}

impl MenuItem {
    pub fn new(text: &str, shortcut: Option<&str>, enabled: Option<bool>) -> Rc<Self> {
        let inner = appkit::create_menu_item(text);
        if let Some(shortcut) = shortcut {
            inner.set_shortcut(shortcut);
        }
        if let Some(enabled) = enabled {
            inner.set_enabled(enabled);
        }
        Rc::new(Self { inner })
    }

    pub fn set_text(&self, _text: &str) {
        // `elwindui_backend_appkit::MenuItem` has no title setter today (menu items are static in
        // practice); accepted here only so the generic resync convention has something to call if
        // `text` is ever written as a dynamic (non-literal) attribute value.
    }

    pub fn set_shortcut(&self, shortcut: &str) {
        self.inner.set_shortcut(shortcut);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    pub fn set_on_select(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_select(callback);
    }
}
