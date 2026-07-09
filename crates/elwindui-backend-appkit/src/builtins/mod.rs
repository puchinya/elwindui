//! AppKit-backed DSL-facing wrappers for every *native* builtin except `TabView` (see `tab_view.rs`
//! — it's large enough to warrant its own file). `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no wrapper here at all: `elwindui-codegen` builds their
//! `elwindui_core::tree::UIElement` values directly (see docs/elwindui_spec.md 付録H.2), so
//! there's no `Type::new(..)` call site for a wrapper to intercept. Each type below wraps the
//! matching `crate::` (this same crate's raw AppKit) widget and exposes exactly the methods
//! `elwindui-codegen`'s generic conventions call: `Type::new(..)` (construction, args in the paired
//! `elwindui-codegen`'s `builtins.elwind` declaration's `#[param]` order), `set_<attr>` (resync /
//! two-way change-back), `set_on_<event>` (an `on_*` callback), `set_on_<attr>_change` (a
//! `#[two_way]` attribute's change-back), and — for anything embeddable as a child —
//! `into_any_view`.
//!
//! Lives in its own `builtins` module (rather than at this crate's root, alongside the raw
//! `Window`/`ButtonImpl`/etc. types it wraps) purely to avoid name collisions: the DSL shape
//! declares a component named e.g. `Button`, which must resolve to a struct named `Button` here —
//! but this crate's root already has a `trait Button` (the raw widget's own class trait, per the
//! trait+Impl+base convention, docs/elwindui_spec.md 付録H.2.1a) that a same-named struct at the
//! same module level would collide with.

mod tab_view;
pub use tab_view::{TabView, TabViewItem};

use crate as appkit;
use crate::{Button as _, MenuItem as _, TextArea as _};
use elwindui_core::tree::UIElement;
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

impl UIElement for TextArea {
    fn base(&self) -> &elwindui_core::tree::UIElementImpl {
        self.inner.base()
    }
    fn children(&self) -> &[Rc<dyn UIElement>] {
        self.inner.children()
    }
    fn measure_override(&self, available: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> elwindui_core::layout::Size {
        self.inner.measure_override(available, child_sizes)
    }
    fn arrange_override(&self, final_size: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> Vec<elwindui_core::layout::Rect> {
        self.inner.arrange_override(final_size, child_sizes)
    }
    fn as_native_control(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_native_control()
    }
}

impl TextArea {
    pub fn new(text: &str) -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_text_area(text) })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    /// `#[two_way] text` (`TextArea` in `builtins.elwind`) — the change-back half of the binding;
    /// `set_text` above is the model→widget half.
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        self.inner.base.handle.clone()
    }
}

pub struct Button {
    inner: appkit::ButtonImpl,
}

impl UIElement for Button {
    fn base(&self) -> &elwindui_core::tree::UIElementImpl {
        self.inner.base()
    }
    fn children(&self) -> &[Rc<dyn UIElement>] {
        self.inner.children()
    }
    fn measure_override(&self, available: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> elwindui_core::layout::Size {
        self.inner.measure_override(available, child_sizes)
    }
    fn arrange_override(&self, final_size: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> Vec<elwindui_core::layout::Rect> {
        self.inner.arrange_override(final_size, child_sizes)
    }
    fn as_native_control(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_native_control()
    }
}

impl Button {
    pub fn new(text: &str, enabled: Option<bool>) -> Rc<Self> {
        let inner = appkit::create_button(text);
        if let Some(enabled) = enabled {
            inner.set_enabled(enabled);
        }
        let this = Rc::new(Self { inner });
        // Wires the real `NSButton` click directly to `dispatch_routed`, once, right here, rather
        // than re-detecting/re-wiring it on every `relayout` the way the old `wire_routed_click`
        // used to (which existed only because the tree node used to be a separate value, built
        // later, external to this widget). Unconditional — `dispatch_routed` already no-ops
        // gracefully when nothing is registered for `"on_click"` at this node or any ancestor
        // (`elwindui-codegen`'s `emit_wiring` registers the actual `#[routed] on_click` handler
        // here, via `register_routed_handler` below, right after this constructor returns).
        {
            let node: Rc<dyn UIElement> = this.clone();
            this.inner.set_on_click(Box::new(move || {
                let args = elwindui_core::input::RoutedEventArgs::default();
                elwindui_core::tree::dispatch_routed(&node, "on_click", &(), &args);
            }));
        }
        this
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    /// `#[routed] on_click` (`Button` in `builtins.elwind`) is registered directly onto this
    /// widget's own `base` — real since construction (see `new`), and already wired (also in `new`)
    /// to fire `dispatch_routed` starting at this same node.
    pub fn register_routed_handler<T: 'static>(&self, name: &'static str, handler: Box<dyn Fn(&T, &elwindui_core::input::RoutedEventArgs)>) {
        self.inner.base().register_routed_handler(name, handler);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        self.inner.base.handle.clone()
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
