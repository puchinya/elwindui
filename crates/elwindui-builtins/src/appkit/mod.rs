//! AppKit-backed implementations of every builtin except `TabView` (see `tab_view.rs` — it's
//! large enough to warrant its own file). Each type wraps the matching `elwindui_backend_appkit`
//! widget and exposes exactly the methods `elwindui-codegen`'s generic conventions call:
//! `Type::new(..)` (construction, args in the paired `src/shapes/*.elwind` declaration's
//! `#[param]` order), `set_<attr>` (resync / two-way change-back), `set_on_<event>` (an `on_*`
//! callback), `set_on_<attr>_change` (a `#[two_way]` attribute's change-back), and — for anything
//! embeddable as a child — `into_any_view`.

mod tab_view;
pub use tab_view::TabView;

use elwindui_backend_appkit as appkit;
use std::rc::Rc;

pub struct Window {
    inner: appkit::Window,
}

impl Window {
    pub fn new(title: &str, menu_bar: Option<Rc<MenuBar>>, content: appkit::AnyView) -> Rc<Self> {
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

    pub fn show_and_run(&self) {
        self.inner.show_and_run();
    }
}

pub struct Column {
    inner: appkit::Column,
}

impl Column {
    pub fn new(children: Vec<appkit::AnyView>) -> Rc<Self> {
        Rc::new(Self { inner: appkit::Column::new(children) })
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct Row {
    inner: appkit::Row,
}

impl Row {
    pub fn new(children: Vec<appkit::AnyView>) -> Rc<Self> {
        Rc::new(Self { inner: appkit::Row::new(children) })
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct TextArea {
    inner: appkit::TextArea,
}

impl TextArea {
    pub fn new(text: &str) -> Rc<Self> {
        Rc::new(Self { inner: appkit::TextArea::new(text) })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    /// `#[two_way] text` (`src/shapes/text_area.elwind`) — the change-back half of the binding;
    /// `set_text` above is the model→widget half.
    pub fn set_on_text_change(&self, callback: Box<dyn Fn(String)>) {
        self.inner.set_on_change(callback);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct Button {
    inner: appkit::Button,
}

impl Button {
    pub fn new(text: &str, enabled: Option<bool>) -> Rc<Self> {
        let inner = appkit::Button::new(text);
        if let Some(enabled) = enabled {
            inner.set_enabled(enabled);
        }
        Rc::new(Self { inner })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    pub fn set_on_click(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_click(callback);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct Text {
    inner: appkit::Text,
}

impl Text {
    pub fn new(text: &str) -> Rc<Self> {
        Rc::new(Self { inner: appkit::Text::new(text) })
    }

    pub fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }

    pub fn into_any_view(&self) -> appkit::AnyView {
        appkit::AnyView::from(self.inner.clone())
    }
}

pub struct MenuBar {
    inner: appkit::MenuBar,
}

impl MenuBar {
    pub fn new(children: Vec<Rc<MenuBarItem>>) -> Rc<Self> {
        let items = children.iter().map(|c| c.inner.clone()).collect();
        Rc::new(Self { inner: appkit::MenuBar::new(items) })
    }
}

pub struct MenuBarItem {
    inner: appkit::MenuBarItem,
}

impl MenuBarItem {
    pub fn new(text: &str, submenu: Rc<Menu>) -> Rc<Self> {
        Rc::new(Self { inner: appkit::MenuBarItem::new(text, submenu.inner.clone()) })
    }

    /// See `MenuItem::set_text`'s doc comment — same reason (no title setter in
    /// `elwindui_backend_appkit`, `text` is effectively static in practice, and the generic resync
    /// convention still expects the method to exist).
    pub fn set_text(&self, _text: &str) {}
}

pub struct Menu {
    inner: appkit::Menu,
}

impl Menu {
    pub fn new(children: Vec<Rc<MenuItem>>) -> Rc<Self> {
        let items = children.iter().map(|c| c.inner.clone()).collect();
        Rc::new(Self { inner: appkit::Menu::new(items) })
    }
}

pub struct MenuItem {
    inner: appkit::MenuItem,
}

impl MenuItem {
    pub fn new(text: &str, shortcut: Option<&str>, enabled: Option<bool>) -> Rc<Self> {
        let inner = appkit::MenuItem::new(text);
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
