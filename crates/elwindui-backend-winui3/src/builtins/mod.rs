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
use crate::{Button as _, Menu as _, MenuBar as _, MenuBarItem as _, MenuItem as _, TextArea as _};
use elwindui_core::tree::UIElement;
use std::cell::RefCell;
use std::rc::Rc;

/// `component NotepadWindow inherits Window` ("host composition", docs/elwindui_spec.md 付録H.2.1a)
/// is what actually inherits this — hence the `Impl` rename + paired empty-marker `Window` trait
/// below, the same trait+Impl+base split every other inherited class in this module follows.
pub struct WindowImpl {
    inner: winui3::Window,
}

impl WindowImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::Window::new() })
    }

    pub fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }

    pub fn set_menu_bar(&self, menu_bar: Rc<MenuBar>) {
        self.inner.set_menu_bar(&menu_bar.inner);
    }

    pub fn set_content(&self, content: Rc<dyn elwindui_core::tree::UIElement>) {
        self.inner.set_content(content);
    }

    pub fn show(&self) {
        self.inner.show();
    }
}

/// `WindowImpl`'s own class trait — empty marker (docs/elwindui_spec.md 付録H.2.1a), the same shape
/// as `Menu`/`MenuBar`/`MenuBarItem`/`MenuItem`'s own traits in this crate's root.
pub trait Window {}
impl Window for WindowImpl {}

pub struct TextArea {
    inner: winui3::TextAreaImpl,
}

impl UIElement for TextArea {
    fn base(&self) -> &elwindui_core::tree::UIElementImpl {
        self.inner.base()
    }
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.inner.visual_children()
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
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_text_area() })
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
    fn visual_children(&self) -> Vec<Rc<dyn UIElement>> {
        self.inner.visual_children()
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
    pub fn new() -> Rc<Self> {
        let inner = winui3::create_button();
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
    /// See `elwindui_backend_appkit::builtins::MenuBar::children`'s doc comment — same
    /// reconciliation pattern.
    children: RefCell<Vec<Rc<MenuBarItem>>>,
}

impl MenuBar {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_bar(), children: RefCell::new(Vec::new()) })
    }

    /// See `elwindui_backend_appkit::builtins::MenuBar::set_children`'s doc comment — same
    /// reconciliation pattern.
    pub fn set_children(&self, children: Vec<Rc<MenuBarItem>>) {
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

pub struct MenuBarItem {
    inner: winui3::MenuBarItemImpl,
}

impl MenuBarItem {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_bar_item() })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    pub fn set_submenu(&self, submenu: Rc<Menu>) {
        self.inner.set_submenu(&submenu.inner);
    }
}

pub struct Menu {
    inner: winui3::MenuImpl,
    /// See `elwindui_backend_appkit::builtins::MenuBar::children`'s doc comment — same
    /// reconciliation pattern.
    children: RefCell<Vec<Rc<MenuItem>>>,
}

impl Menu {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu(), children: RefCell::new(Vec::new()) })
    }

    /// See `elwindui_backend_appkit::builtins::MenuBar::set_children`'s doc comment — same
    /// reconciliation pattern.
    pub fn set_children(&self, children: Vec<Rc<MenuItem>>) {
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

pub struct MenuItem {
    inner: winui3::MenuItemImpl,
}

impl MenuItem {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_item() })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
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
