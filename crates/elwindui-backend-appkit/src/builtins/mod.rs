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
//! Lives in its own `builtins` module (rather than at this crate's root, alongside the raw
//! `Window`/`ButtonImpl`/etc. types it wraps) purely to avoid name collisions: the DSL shape
//! declares a component named e.g. `Button`, which must resolve to a struct named `Button` here —
//! but this crate's root already has a `trait Button` (the raw widget's own class trait, per the
//! trait+Impl+base convention, docs/elwindui_spec.md 付録H.2.1a) that a same-named struct at the
//! same module level would collide with.
//!
//! `Type::new()` takes **no** arguments — every declared `#[param]` is applied afterward via its
//! own `set_<field>(..)` call (docs/elwindui_spec.md 付録H.2.1a's post-construction setter
//! convention, extended from the common `margin`/`data_context`/`grid_cell` attributes to every
//! builtin property), the same generic mechanism `resync`/two-way change-back already relies on —
//! see `elwindui-codegen`'s `build_component_setters`.

mod tab_view;
pub use tab_view::{TabView, TabViewItem};

use crate as appkit;
use crate::{Button as _, Menu as _, MenuBar as _, MenuBarItem as _, MenuItem as _, TextArea as _};
use elwindui_core::ui::UIElement;
use std::cell::RefCell;
use std::rc::Rc;

/// `component NotepadWindow inherits Window` ("host composition", docs/elwindui_spec.md 付録H.2.1a)
/// is what actually inherits this — hence the `Impl` rename + paired empty-marker `Window` trait
/// below, the same trait+Impl+base split every other inherited class in this module follows.
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

    pub fn set_menu_bar(&self, menu_bar: Rc<MenuBar>) {
        self.inner.set_menu_bar(&menu_bar.inner);
    }

    pub fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElement>) {
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
    inner: appkit::TextAreaImpl,
}

impl UIElement for TextArea {
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

impl TextArea {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_text_area() })
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

impl Button {
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
    /// The currently-installed children, in display order — the "before" side of `set_children`'s
    /// diff against its own new `children` argument (the "after" side), mirroring `TabView`'s own
    /// `entries`/reconciliation pattern.
    children: RefCell<Vec<Rc<MenuBarItem>>>,
}

impl MenuBar {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_bar(), children: RefCell::new(Vec::new()) })
    }

    /// Reconciles the native `NSMenu`'s installed items against `children` by `Rc` pointer
    /// identity (matching `TabView`'s own reconciliation convention) — an item present in both the
    /// old and new list is left alone; one only in the old list is removed
    /// (`elwindui_backend_appkit::MenuBar::remove_item`); one only in the new list is added
    /// (`add_item`). Safe to call more than once (e.g. a future dynamic menu bar), though today's
    /// only caller (`elwindui-codegen`'s generated construction) calls it exactly once.
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
    inner: appkit::MenuBarItemImpl,
}

impl MenuBarItem {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_bar_item() })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    pub fn set_submenu(&self, submenu: Rc<Menu>) {
        self.inner.set_submenu(&submenu.inner);
    }
}

pub struct Menu {
    inner: appkit::MenuImpl,
    /// See `MenuBar::children`'s doc comment — same reconciliation pattern.
    children: RefCell<Vec<Rc<MenuItem>>>,
}

impl Menu {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu(), children: RefCell::new(Vec::new()) })
    }

    /// See `MenuBar::set_children`'s doc comment — same reconciliation pattern.
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
    inner: appkit::MenuItemImpl,
}

impl MenuItem {
    pub fn new() -> Rc<Self> {
        Rc::new(Self { inner: appkit::create_menu_item() })
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
