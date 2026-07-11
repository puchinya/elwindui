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
//! (`Button`/`TextArea`/`MenuBar<MenuBarItemImpl>`/`MenuBarItem<MenuImpl>`/`Menu<MenuItemImpl>`/
//! `MenuItem`) — see that module's own doc comment for why these traits live there instead of being
//! declared separately per backend.
//!
//! `Type::new()` takes **no** arguments — every declared `#[param]` is applied afterward via its
//! own `set_<field>(..)` call (docs/elwindui_spec.md 付録H.2.1a's post-construction setter
//! convention, extended from the common `margin`/`data_context`/`grid_cell` attributes to every
//! builtin property), the same generic mechanism `resync`/two-way change-back already relies on —
//! see `elwindui-codegen`'s `build_component_setters`.

mod tab_view;
pub use tab_view::{TabViewImpl, TabViewItemImpl};

use crate as appkit;
// Re-exported so `component X inherits Window` call sites can keep importing `Window` from this
// same `builtins` module alongside `WindowImpl`, rather than needing a separate `elwindui_core`
// import just for the (now-shared) marker trait.
pub use elwindui_core::ui::Window;
use elwindui_core::ui::{Button as _, Menu as _, MenuBar as _, MenuBarItem as _, MenuItem as _, TextArea as _, UIElement};
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

impl elwindui_core::ui::Window for WindowImpl {}

pub struct TextAreaImpl {
    inner: appkit::TextAreaImpl,
}

impl UIElement for TextAreaImpl {
    fn base(&self) -> &elwindui_core::ui::UIElementImpl {
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

impl TextAreaImpl {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_text_area() })
    }

    /// `#[two_way] text` (`TextArea` in `builtins.elwind`) — the change-back half of the binding;
    /// `elwindui_core::ui::TextArea::set_text` is the model→widget half.
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        self.inner.base.handle.clone()
    }
}

impl elwindui_core::ui::TextArea for TextAreaImpl {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }
}

pub struct ButtonImpl {
    inner: appkit::ButtonImpl,
}

impl UIElement for ButtonImpl {
    fn base(&self) -> &elwindui_core::ui::UIElementImpl {
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

impl ButtonImpl {
    pub fn new() -> Rc<Self> {
        let inner = appkit::create_button();
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
                elwindui_core::ui::dispatch_routed(&node, "on_click", &(), &args);
            }));
        }
        this
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

impl elwindui_core::ui::Button for ButtonImpl {
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_click(callback);
    }
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
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

impl elwindui_core::ui::MenuBarItem<MenuImpl> for MenuBarItemImpl {
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_submenu(&self, submenu: &MenuImpl) {
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
