//! `elwindui::ui::TabViewItem` — one tab of a `TabView`.

use super::*;

/// `elwindui::ui::TabViewItem`, one tab of a [`TabView`]: a header, the tab's content subtree, an
/// optional close affordance and a per-tab `on_close` handler. Declared once here as a Core-owned
/// Component (Issue #313, `native_peer_design.md` §7c).
///
/// No `inherits`: like `Window`, a `TabViewItem` is never itself embedded as a `UIElement` node.
/// The owning `TabView`'s peer hosts the content in that tab's nested content host. A header or
/// closable change on an item that is already displayed is forwarded to the owning `TabView`.
#[elwindui_macros::component(sealed)]
#[prop(header: String)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(closable: Option<bool>)]
#[prop(on_close: fn())]
pub struct TabViewItem {
    header: RefCell<String>,
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    closable: Cell<bool>,
    on_close: RefCell<Option<Rc<dyn Fn()>>>,
    /// Installed by the owning `TabView`; captures only a `Weak` to it.
    on_changed: RefCell<Option<Rc<dyn Fn()>>>,
    /// The `TabView` this tab belongs to (Issue #323 single-parent lease).
    owner: crate::ui::collections::ParentLease<dyn UIElementExt>,
}

#[elwindui_macros::component]
impl TabViewItem {
    fn set_header(&self, header: &str) {
        let this = self.as_ui_element();
        if *this.header.borrow() == header {
            return;
        }
        *this.header.borrow_mut() = header.to_string();
        this.notify_changed();
    }

    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        *self.as_ui_element().content.borrow_mut() = Some(content);
    }

    fn set_closable(&self, closable: bool) {
        let this = self.as_ui_element();
        if this.closable.get() == closable {
            return;
        }
        this.closable.set(closable);
        this.notify_changed();
    }

    fn set_on_close(&self, callback: Box<dyn Fn()>) {
        *self.as_ui_element().on_close.borrow_mut() = Some(Rc::from(callback));
    }

    fn construct() -> Self {
        Self {
            header: RefCell::new(String::new()),
            content: RefCell::new(None),
            closable: Cell::new(true),
            on_close: RefCell::new(None),
            on_changed: RefCell::new(None),
            owner: crate::ui::collections::ParentLease::new(),
        }
    }
}

impl TabViewItem {
    /// The tab header.
    pub fn header(&self) -> String {
        self.header.borrow().clone()
    }

    /// Whether the tab shows a close affordance (default `true`).
    pub fn closable(&self) -> bool {
        self.closable.get()
    }

    /// The tab's content subtree, if any.
    pub fn content(&self) -> Option<Rc<dyn UIElementExt>> {
        self.content.borrow().clone()
    }

    /// The per-tab close handler, if any.
    pub(crate) fn close_handler(&self) -> Option<Rc<dyn Fn()>> {
        self.on_close.borrow().clone()
    }

    /// The single-parent lease the owning `TabView` checks, claims and releases.
    pub(crate) fn tab_view_lease(&self) -> &crate::ui::collections::ParentLease<dyn UIElementExt> {
        &self.owner
    }

    pub(crate) fn set_change_listener(&self, listener: Option<Rc<dyn Fn()>>) {
        *self.on_changed.borrow_mut() = listener;
    }

    fn notify_changed(&self) {
        let listener = self.on_changed.borrow().clone();
        if let Some(listener) = listener {
            listener();
        }
    }
}
