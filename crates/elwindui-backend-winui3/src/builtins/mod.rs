//! WinUI3-backed DSL-facing wrappers for every *native* builtin except `TabView` (see
//! `tab_view.rs`), mirroring `elwindui_backend_appkit::builtins`'s structure exactly (see that
//! module's doc comment for the overall convention: `Type::new(..)`/`set_<attr>`/
//! `set_on_<event>`/`set_on_<attr>_change`/`into_any_view`, why this lives in its own `builtins`
//! module rather than at this crate's root, and why every struct here compiles to `XImpl`
//! regardless of its bare source-level class name).
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no wrapper here
//! either, for the same reason as the AppKit side: `elwindui-codegen` builds their
//! `elwindui_core::ui::UIElement` values directly.
//!
//! UNVERIFIED — see `elwindui-backend-winui3`'s crate-level doc comment. This file only calls
//! that crate's own API (which is under this project's control and self-consistent), so it should
//! need little to no correction even if the backend crate's underlying WinRT calls do.

mod tab_view;
pub use tab_view::{TabViewImpl, TabViewItemImpl};

use crate as winui3;
use crate::AnyView;
// Bare names needed for the transitive ancestor-chain walk `#[elwindui_macros::class]` performs
// below (`winui3::TextAreaImpl`/`winui3::ButtonImpl` → `winui3::TextArea`/`winui3::Button`'s own
// registered `inherits = NativeControl` → `winui3::NativeControl`'s own `NativeControlImpl`):
// `NativeControlImpl` (the struct — the walk's `as_native_control()` accessor return type) and
// `NativeControl` (the trait — the walk's auto-generated `impl NativeControl for TextAreaImpl {}`/
// `for ButtonImpl {}`). See `elwindui-backend-appkit::builtins`'s matching import for the full
// rationale.
use crate::{NativeControl, NativeControlImpl};
// Re-exported so `component X inherits Window` call sites can keep importing `Window` from this
// same `builtins` module alongside `WindowImpl`, rather than needing a separate `elwindui_core`
// import just for the (now-shared) marker trait.
pub use elwindui_core::ui::Window;
use elwindui_core::ui::{Button as _, TextArea as _, UIElement};
use std::cell::RefCell;
use std::rc::Rc;

/// `component NotepadWindow inherits Window` ("host composition", docs/elwindui_spec.md 付録H.2.1a)
/// is what actually inherits this — hence `struct_only`'s target being `elwindui_core::ui::Window`
/// itself (see `elwindui_backend_appkit::builtins`'s matching struct for why this bare-name source
/// spelling still compiles to `WindowImpl` and never collides with the `Window` trait re-exported
/// just above).
#[elwindui_macros::class(struct_only = elwindui_core::ui::Window)]
pub struct Window {
    inner: winui3::Window,
}

#[elwindui_macros::class]
impl Window {
    // The bare (not `Rc`-wrapped) value `#[class]`'s auto-generated `new` wraps — this is also what
    // lets a `component X inherits Window` (host composition) embed a real `WindowImpl` directly as
    // its own `base` field, matching `#[class(inherits = elwindui::ui::Window)]`'s default `base`
    // field shape.
    fn construct() -> Self {
        Self { inner: winui3::Window::new() }
    }

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

#[elwindui_macros::class(struct_only = elwindui_core::ui::TextArea, inherits = winui3::TextAreaImpl)]
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
    pub fn into_any_view(&self) -> winui3::AnyView {
        self.base.base.handle.clone()
    }

    fn set_text(&self, text: &str) {
        self.base.set_text(text);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.base.set_on_change(callback);
    }

    fn new() -> Rc<Self> {
        Rc::new(Self { base: winui3::create_text_area() })
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::Button, inherits = winui3::ButtonImpl)]
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
    pub fn into_any_view(&self) -> winui3::AnyView {
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
        let base = winui3::create_button();
        let this = Rc::new(Self { base });
        // Wires the real XAML click directly to `dispatch_routed`, once, right here — mirrors
        // `elwindui_backend_appkit::builtins::ButtonImpl::new`'s own doc comment for the rationale.
        // Unconditional — `dispatch_routed` already no-ops gracefully when nothing is registered
        // for `"on_click"` at this node or any ancestor (`elwindui-codegen`'s `emit_wiring`
        // registers the actual `#[routed] on_click` handler here, via `register_routed_handler`
        // below, right after this constructor returns).
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

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuBar)]
pub struct MenuBar {
    inner: winui3::MenuBarImpl,
    /// See `elwindui_backend_appkit::builtins::MenuBar::children`'s doc comment — same
    /// reconciliation pattern.
    children: RefCell<Vec<Rc<MenuBarItemImpl>>>,
}

#[elwindui_macros::class]
impl MenuBar {
    fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_bar(), children: RefCell::new(Vec::new()) })
    }

    /// See `elwindui_backend_appkit::builtins::MenuBar::set_children`'s doc comment — same
    /// reconciliation pattern.
    #[inherent]
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

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuBarItem)]
pub struct MenuBarItem {
    inner: winui3::MenuBarItemImpl,
}

#[elwindui_macros::class]
impl MenuBarItem {
    fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_bar_item() })
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_submenu(&self, submenu: Rc<dyn elwindui_core::ui::Menu>) {
        // `submenu` itself is dropped at the end of this call — the underlying native menu stays
        // alive regardless (retained by whatever it gets installed into).
        let submenu = submenu
            .as_any()
            .downcast_ref::<MenuImpl>()
            .expect("MenuBarItem::set_submenu: submenu must be this backend's MenuImpl");
        self.inner.set_submenu(&submenu.inner);
    }
}

#[elwindui_macros::class(struct_only = elwindui_core::ui::Menu)]
pub struct Menu {
    inner: winui3::MenuImpl,
    /// See `elwindui_backend_appkit::builtins::MenuBar::children`'s doc comment — same
    /// reconciliation pattern.
    children: RefCell<Vec<Rc<MenuItemImpl>>>,
}

#[elwindui_macros::class]
impl Menu {
    fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu(), children: RefCell::new(Vec::new()) })
    }

    /// See `elwindui_backend_appkit::builtins::MenuBar::set_children`'s doc comment — same
    /// reconciliation pattern.
    #[inherent]
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

#[elwindui_macros::class(struct_only = elwindui_core::ui::MenuItem)]
pub struct MenuItem {
    inner: winui3::MenuItemImpl,
}

#[elwindui_macros::class]
impl MenuItem {
    fn new() -> Rc<Self> {
        Rc::new(Self { inner: winui3::create_menu_item() })
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
