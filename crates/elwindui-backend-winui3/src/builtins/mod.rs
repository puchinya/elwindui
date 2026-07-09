//! WinUI3-backed DSL-facing wrappers for every *native* builtin except `TabView` (see
//! `tab_view.rs`), mirroring `elwindui_backend_appkit::builtins`'s structure exactly (see that
//! module's doc comment for the overall convention: `Type::new(..)`/`set_<attr>`/
//! `set_on_<event>`/`set_on_<attr>_change`/`into_any_view`, and for why this lives in its own
//! `builtins` module rather than at this crate's root). `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no wrapper here either, for the same reason as the
//! AppKit side: `elwindui-codegen` builds their `elwindui_core::tree::UIElement` values directly.
//!
//! UNVERIFIED — see `elwindui-backend-winui3`'s crate-level doc comment. This file only calls
//! that crate's own API (which is under this project's control and self-consistent), so it should
//! need little to no correction even if the backend crate's underlying WinRT calls do.

mod tab_view;
pub use tab_view::{TabView, TabViewItem};

use crate as winui3;
use crate::{Button as _, MenuItem as _, TextArea as _};
use elwindui_core::tree::UIElement;
use std::rc::Rc;

pub struct Window {
    inner: winui3::Window,
}

impl Window {
    pub fn new(title: &str, menu_bar: Option<Rc<MenuBar>>, content: Rc<dyn elwindui_core::tree::UIElement>) -> Rc<Self> {
        let inner = winui3::Window::new(title);
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
    inner: winui3::TextAreaImpl,
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
        Rc::new(Self { inner: winui3::create_text_area(text) })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    /// `#[two_way] text` (`TextArea` in `builtins.elwind`) — the change-back half of the binding;
    /// `set_text` above is the model→widget half.
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    pub fn into_any_view(&self) -> winui3::AnyView {
        self.inner.base.handle.clone()
    }
}

pub struct Button {
    inner: winui3::ButtonImpl,
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
        let inner = winui3::create_button(text);
        if let Some(enabled) = enabled {
            inner.set_enabled(enabled);
        }
        let this = Rc::new(Self { inner });
        // Wires the real XAML click directly to `dispatch_routed`, once, right here — mirrors
        // `elwindui_backend_appkit::builtins::Button::new`'s own doc comment for the rationale.
        // Unconditional — `dispatch_routed` already no-ops gracefully when nothing is registered
        // for `"on_click"` at this node or any ancestor (`elwindui-codegen`'s `emit_wiring`
        // registers the actual `#[routed] on_click` handler here, via `register_routed_handler`
        // below, right after this constructor returns).
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

    pub fn into_any_view(&self) -> winui3::AnyView {
        self.inner.base.handle.clone()
    }
}

pub struct MenuBar {
    inner: winui3::MenuBarImpl,
}

impl MenuBar {
    pub fn new(children: Vec<Rc<MenuBarItem>>) -> Rc<Self> {
        let items = children.iter().map(|c| c.inner.clone()).collect();
        Rc::new(Self { inner: winui3::create_menu_bar(items) })
    }
}

pub struct MenuBarItem {
    inner: winui3::MenuBarItemImpl,
}

impl MenuBarItem {
    pub fn new(text: &str, submenu: Rc<Menu>) -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_bar_item(text, submenu.inner.clone()) })
    }

    pub fn set_text(&self, _text: &str) {}
}

pub struct Menu {
    inner: winui3::MenuImpl,
}

impl Menu {
    pub fn new(children: Vec<Rc<MenuItem>>) -> Rc<Self> {
        let items = children.iter().map(|c| c.inner.clone()).collect();
        Rc::new(Self { inner: winui3::create_menu(items) })
    }
}

pub struct MenuItem {
    inner: winui3::MenuItemImpl,
}

impl MenuItem {
    pub fn new(text: &str, shortcut: Option<&str>, enabled: Option<bool>) -> Rc<Self> {
        let inner = winui3::create_menu_item(text);
        if let Some(shortcut) = shortcut {
            inner.set_shortcut(shortcut);
        }
        if let Some(enabled) = enabled {
            inner.set_enabled(enabled);
        }
        Rc::new(Self { inner })
    }

    pub fn set_text(&self, _text: &str) {}

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
