//! `elwindui::ui::Window` — the top-level native window.

use super::*;
use crate::base::Rect;
use crate::ui::native::{WindowPeer, create_native_peer};
use std::cell::OnceCell;

/// `elwindui::ui::Window`, the top-level native window. Declared once here as a Core-owned
/// Component (Issue #313, `docs/design/runtime/native_peer_design.md` §7).
///
/// `Window` is deliberately not a `UIElement`: it is a separate top-level concept and cannot be
/// embedded as a child. `component X inherits Window` (host composition,
/// `docs/design/runtime/ui_tree_design.md`) inherits this class; the generated component
/// overrides `show`/`hide`/`close`/`mount_override`/`unmount_override` and chains to the methods
/// here.
///
/// # Peer lifecycle
///
/// Core keeps the title, menu bar, content, transparency, always-on-top and the one-time initial
/// bounds as logical state. The OS window is a [`WindowPeer`] that the backend creates lazily,
/// exactly once, on the first of:
///
/// - `show()`;
/// - `set_content()`, because hosting content installs the native host services (focus,
///   accessibility, invalidation) that the content's mount relies on;
/// - a bounds getter whose value was never set logically, or another native-only operation
///   (floating-title setup).
///
/// Realization creates the peer, pushes the stored logical state, and installs the close-request
/// and bounds-changed handlers last (§6 two-phase order). Unshown, unrealized Windows need no
/// backend, so constructing one never requires `elwindui::init()`. Dropping the Window calls
/// [`WindowPeer::dispose`] once.
///
/// The backend's retention registry, not this struct, keeps a shown Window alive: the peer
/// receives only this Window's most-derived `Weak` owner and upgrades it on first show.
#[elwindui_macros::component]
#[prop(title: String)]
#[prop(menu_bar: std::rc::Rc<dyn crate::ui::MenuBarExt>)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(transparent: bool)]
#[prop(always_on_top: bool)]
#[prop(onetime, left: Option<f32>)]
#[prop(onetime, top: Option<f32>)]
#[prop(onetime, width: Option<f32>)]
#[prop(onetime, height: Option<f32>)]
pub struct Window {
    /// The most-derived Window: the backend retention registry upgrades it on first show.
    self_weak: Weak<dyn WindowExt>,
    peer: OnceCell<Rc<dyn WindowPeer>>,
    title: RefCell<String>,
    menu_bar: RefCell<Option<Rc<dyn MenuBarExt>>>,
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    transparent: Cell<bool>,
    always_on_top: Cell<bool>,
    left: Cell<Option<f32>>,
    top: Cell<Option<f32>>,
    width: Cell<Option<f32>>,
    height: Cell<Option<f32>>,
    close_request_handler: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    bounds_changed_handler: RefCell<Option<Rc<dyn Fn(Rect)>>>,
}

#[elwindui_macros::component]
impl Window {
    fn set_title(&self, title: &str) {
        *self.as_ui_element().title.borrow_mut() = title.to_string();
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_title(title);
        }
    }

    fn set_menu_bar(&self, menu_bar: Rc<dyn MenuBarExt>) {
        *self.as_ui_element().menu_bar.borrow_mut() = Some(menu_bar.clone());
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_menu_bar(menu_bar);
        }
    }

    fn content_element(&self) -> Option<Rc<dyn UIElementExt>> {
        self.as_ui_element().content.borrow().clone()
    }

    /// Sets the window's root element. Realizes the native window (see the type docs).
    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        *self.as_ui_element().content.borrow_mut() = Some(content.clone());
        match self.as_ui_element().peer.get() {
            Some(peer) => peer.set_content(content),
            // Realization pushes the stored content.
            None => {
                self.as_ui_element().__window_peer();
            }
        }
    }

    /// Enables or disables an alpha-capable client surface.
    ///
    /// Transparent pixels reveal windows behind this one; native window decorations are unchanged.
    fn set_transparent(&self, transparent: bool) {
        self.as_ui_element().transparent.set(transparent);
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_transparent(transparent);
        }
    }

    /// Pins this window above normal application windows, or restores normal Z-order when false.
    fn set_always_on_top(&self, always_on_top: bool) {
        self.as_ui_element().always_on_top.set(always_on_top);
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_always_on_top(always_on_top);
        }
    }

    /// Shows the window, realizing it on the first call.
    #[overridable]
    fn show(&self) {
        self.as_ui_element().__window_peer().show();
    }

    /// Visibility only: the mounted subtree, Environment subscriptions, and state all remain alive.
    /// A subsequent `show()` makes the window visible again without remounting/rebuilding
    /// (docs/specs/dsl_spec.md's Window contract; CI-8 of #80). Does nothing before realization.
    #[overridable]
    fn hide(&self) {
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.hide();
        }
    }

    /// Ends this Window's mount lifetime: releases the native window and (for a host-composition
    /// generated component's own `#[overrides]`) its own Environment subscriptions. See
    /// docs/design/runtime/component_lifecycle_design.md §4g. A Window that was never realized
    /// has no native window to close.
    #[overridable]
    fn close(&self) {
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.close();
        }
    }

    fn left(&self) -> f32 {
        match (
            self.as_ui_element().peer.get(),
            self.as_ui_element().left.get(),
        ) {
            (Some(peer), _) => peer.left(),
            (None, Some(left)) => left,
            (None, None) => self.as_ui_element().__window_peer().left(),
        }
    }

    fn set_left(&self, left: f32) {
        self.as_ui_element().left.set(Some(left));
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_left(left);
        }
    }

    fn top(&self) -> f32 {
        match (
            self.as_ui_element().peer.get(),
            self.as_ui_element().top.get(),
        ) {
            (Some(peer), _) => peer.top(),
            (None, Some(top)) => top,
            (None, None) => self.as_ui_element().__window_peer().top(),
        }
    }

    fn set_top(&self, top: f32) {
        self.as_ui_element().top.set(Some(top));
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_top(top);
        }
    }

    fn width(&self) -> f32 {
        match (
            self.as_ui_element().peer.get(),
            self.as_ui_element().width.get(),
        ) {
            (Some(peer), _) => peer.width(),
            (None, Some(width)) => width,
            (None, None) => self.as_ui_element().__window_peer().width(),
        }
    }

    fn set_width(&self, width: f32) {
        self.as_ui_element().width.set(Some(width));
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_width(width);
        }
    }

    fn height(&self) -> f32 {
        match (
            self.as_ui_element().peer.get(),
            self.as_ui_element().height.get(),
        ) {
            (Some(peer), _) => peer.height(),
            (None, Some(height)) => height,
            (None, None) => self.as_ui_element().__window_peer().height(),
        }
    }

    fn set_height(&self, height: f32) {
        self.as_ui_element().height.set(Some(height));
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.set_height(height);
        }
    }

    /// Issue #162 §3.14-§3.17: a framework-internal implementation hook, not a second lifecycle
    /// authoring surface. It runs once, before the generated Window content is built and before
    /// the user's own `on_mount`, so the generated host-composition Window can install the native
    /// close-request handler (`WindowLifecycleHost`, below) through the ordinary
    /// `#[overridable]`/`#[overrides]` chain. Core's own implementation does nothing. A
    /// user-authored `#[overrides] fn mount_override` is rejected at compile time; user lifecycle
    /// code belongs in `on_mount`.
    #[overridable]
    fn mount_override(&self, _environment: crate::environment::EnvironmentContext) {}

    /// The `unmount()` counterpart to `mount_override`. It runs once, before the generated Window
    /// content subtree unmounts and before the user's own `on_unmount`, and closes the window's
    /// active popup (`docs/design/runtime/popup_context_menu_design.md`) while the owner Window's
    /// content is still intact. Same framework-reserved status as `mount_override`.
    #[overridable]
    fn unmount_override(&self) {
        if let Some(peer) = self.as_ui_element().peer.get() {
            peer.close_active_popup();
        }
    }

    fn construct() -> Self {
        Self {
            self_weak: __self_weak.clone(),
            peer: OnceCell::new(),
            title: RefCell::new(String::new()),
            menu_bar: RefCell::new(None),
            content: RefCell::new(None),
            transparent: Cell::new(false),
            always_on_top: Cell::new(false),
            left: Cell::new(None),
            top: Cell::new(None),
            width: Cell::new(None),
            height: Cell::new(None),
            close_request_handler: RefCell::new(None),
            bounds_changed_handler: RefCell::new(None),
        }
    }
}

impl Window {
    /// Returns the native window peer, realizing it first if needed (see the type docs).
    ///
    /// # Panics
    ///
    /// Panics when realization is needed and no native backend is registered on this thread, or
    /// the backend does not support `Window`.
    #[doc(hidden)]
    pub fn __window_peer(&self) -> &Rc<dyn WindowPeer> {
        if let Some(peer) = self.peer.get() {
            return peer;
        }
        // Phase 1: create only.
        let peer = create_native_peer("Window", |factory| {
            factory.create_window(self.self_weak.clone())
        });
        if self.peer.set(peer).is_err() {
            panic!("elwindui: a Window's native peer was realized more than once");
        }
        let peer = self.peer.get().expect("peer was just installed");
        // Phase 2: push the logical state. Values are cloned out first: the peer may re-enter.
        let title = self.title.borrow().clone();
        if !title.is_empty() {
            peer.set_title(&title);
        }
        let menu_bar = self.menu_bar.borrow().clone();
        if let Some(menu_bar) = menu_bar {
            peer.set_menu_bar(menu_bar);
        }
        if self.transparent.get() {
            peer.set_transparent(true);
        }
        if self.always_on_top.get() {
            peer.set_always_on_top(true);
        }
        if let Some(left) = self.left.get() {
            peer.set_left(left);
        }
        if let Some(top) = self.top.get() {
            peer.set_top(top);
        }
        if let Some(width) = self.width.get() {
            peer.set_width(width);
        }
        if let Some(height) = self.height.get() {
            peer.set_height(height);
        }
        let content = self.content.borrow().clone();
        if let Some(content) = content {
            peer.set_content(content);
        }
        // Phase 3: native-to-Core handlers last.
        let close_request = self.close_request_handler.borrow().clone();
        if close_request.is_some() {
            peer.set_close_request_handler(close_request);
        }
        let bounds_changed = self.bounds_changed_handler.borrow().clone();
        if bounds_changed.is_some() {
            peer.set_bounds_changed_handler(bounds_changed);
        }
        peer
    }

    /// Whether this Window's native peer has been realized.
    #[doc(hidden)]
    pub fn __has_window_peer(&self) -> bool {
        self.peer.get().is_some()
    }

    /// Applies a complete native rectangle in one operation where the backend supports it. A
    /// framework hook for retained floating hosts (Docking); ordinary Window code keeps the
    /// individual bound properties.
    #[doc(hidden)]
    pub fn __set_bounds(&self, bounds: Rect) {
        self.left.set(Some(bounds.x));
        self.top.set(Some(bounds.y));
        self.width.set(Some(bounds.width));
        self.height.set(Some(bounds.height));
        if let Some(peer) = self.peer.get() {
            peer.set_bounds(bounds);
        }
    }

    /// Framework-only floating host appearance (Docking). Realizes the native window.
    #[doc(hidden)]
    pub fn __configure_floating_title(&self) -> Result<(), String> {
        self.__window_peer().configure_floating_title()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some(peer) = self.peer.get() {
            peer.dispose();
        }
    }
}

/// Issue #162 §3.19-§3.20: lets the generated host-composition Window register a close-request
/// callback that a backend's native title-bar/system close affordance must route through, so a
/// user clicking the OS close button enters the same fixed `mount_override`/popup-teardown/
/// `unmount_override` lifecycle a programmatic `WindowExt::close()` call does, rather than
/// destroying the native window directly and bypassing it. `#[doc(hidden)]`: framework-internal,
/// never a DSL-visible property, never threaded through `EnvironmentContext`/`ViewBuildContext`.
///
/// The callback returns `true` when the generated most-derived Window accepted the request (it
/// ran, or is running, the common Window close lifecycle — the native layer must not also close
/// independently) or `false` when the generated owner is already gone (the native layer should
/// fall back to its own default close behavior). Ownership must stay acyclic: the handler's
/// closure captures only `Weak<GeneratedWindow>`, never `Rc<GeneratedWindow>`.
///
/// Core `Window` stores each handler as logical state and pushes it to its [`WindowPeer`] when
/// realized (installed last during realization).
#[doc(hidden)]
pub trait WindowLifecycleHost {
    /// Installs or clears the generated Window's native close-request bridge.
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>);

    /// Installs a private bridge for native move/resize updates. Backends that do not expose
    /// native bounds notifications may ignore it.
    fn set_bounds_changed_handler(&self, _handler: Option<Rc<dyn Fn(crate::base::Rect)>>) {}

    /// Activates the native window without changing its bounds.
    fn activate(&self) {}
}

impl WindowLifecycleHost for Window {
    fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        *self.close_request_handler.borrow_mut() = handler.clone();
        if let Some(peer) = self.peer.get() {
            peer.set_close_request_handler(handler);
        }
    }

    fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        *self.bounds_changed_handler.borrow_mut() = handler.clone();
        if let Some(peer) = self.peer.get() {
            peer.set_bounds_changed_handler(handler);
        }
    }

    fn activate(&self) {
        if let Some(peer) = self.peer.get() {
            peer.activate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::native::{
        NativePeerFactory, clear_native_peer_factory, register_native_peer_factory,
    };

    thread_local! {
        /// Stands in for a backend's application-layer retention registry (`app::WINDOWS`).
        static RETAINED: RefCell<Vec<Rc<dyn WindowExt>>> = const { RefCell::new(Vec::new()) };
    }

    fn retained_count() -> usize {
        RETAINED.with(|retained| retained.borrow().len())
    }

    /// Records every call a Core `Window` makes on its peer and models the backend's retention
    /// registry: the owner is upgraded and retained on first show, released on close.
    struct MockWindowPeer {
        log: Rc<RefCell<Vec<String>>>,
        owner: Weak<dyn WindowExt>,
        retained: Cell<bool>,
        close_request: RefCell<Option<Rc<dyn Fn() -> bool>>>,
        bounds_changed: RefCell<Option<Rc<dyn Fn(Rect)>>>,
    }

    impl MockWindowPeer {
        fn record(&self, entry: impl Into<String>) {
            self.log.borrow_mut().push(entry.into());
        }

        /// A native title-bar close: asks the Core handler, like `AppWindow.Closing`.
        fn native_close_request(&self) -> bool {
            let handler = self.close_request.borrow().clone();
            handler.is_some_and(|handler| handler())
        }
    }

    impl WindowPeer for MockWindowPeer {
        fn show(&self) {
            self.record("show");
            if !self.retained.replace(true) {
                let owner = self.owner.upgrade().expect("owner alive at first show");
                RETAINED.with(|retained| retained.borrow_mut().push(owner));
            }
        }
        fn hide(&self) {
            self.record("hide");
        }
        fn close(&self) {
            self.record("close");
            if self.retained.replace(false) {
                // Drop the owner only after the registry borrow ends, like `release_window`.
                let released = RETAINED.with(|retained| {
                    let mut retained = retained.borrow_mut();
                    let owner = self.owner.upgrade();
                    let index = retained.iter().position(|candidate| {
                        owner
                            .as_ref()
                            .is_some_and(|owner| Rc::ptr_eq(candidate, owner))
                    });
                    index.map(|index| retained.remove(index))
                });
                drop(released);
            }
        }
        fn activate(&self) {
            self.record("activate");
        }
        fn close_active_popup(&self) {
            self.record("close_active_popup");
        }
        fn set_title(&self, title: &str) {
            self.record(format!("title:{title}"));
        }
        fn set_menu_bar(&self, _menu_bar: Rc<dyn MenuBarExt>) {
            self.record("menu_bar");
        }
        fn set_content(&self, _content: Rc<dyn UIElementExt>) {
            self.record("content");
        }
        fn set_transparent(&self, transparent: bool) {
            self.record(format!("transparent:{transparent}"));
        }
        fn set_always_on_top(&self, always_on_top: bool) {
            self.record(format!("always_on_top:{always_on_top}"));
        }
        fn left(&self) -> f32 {
            11.0
        }
        fn set_left(&self, left: f32) {
            self.record(format!("left:{left}"));
        }
        fn top(&self) -> f32 {
            12.0
        }
        fn set_top(&self, top: f32) {
            self.record(format!("top:{top}"));
        }
        fn width(&self) -> f32 {
            13.0
        }
        fn set_width(&self, width: f32) {
            self.record(format!("width:{width}"));
        }
        fn height(&self) -> f32 {
            14.0
        }
        fn set_height(&self, height: f32) {
            self.record(format!("height:{height}"));
        }
        fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
            self.record(format!("close_request:{}", handler.is_some()));
            *self.close_request.borrow_mut() = handler;
        }
        fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
            self.record(format!("bounds_changed:{}", handler.is_some()));
            *self.bounds_changed.borrow_mut() = handler;
        }
        fn dispose(&self) {
            self.record("dispose");
            self.close_request.borrow_mut().take();
            self.bounds_changed.borrow_mut().take();
        }
    }

    #[derive(Default)]
    struct MockFactory {
        created: Cell<usize>,
        log: Rc<RefCell<Vec<String>>>,
        last: RefCell<Option<Rc<MockWindowPeer>>>,
    }

    impl NativePeerFactory for MockFactory {
        fn backend_name(&self) -> &'static str {
            "mock"
        }
        fn create_window(&self, owner: Weak<dyn WindowExt>) -> Rc<dyn WindowPeer> {
            self.created.set(self.created.get() + 1);
            let peer = Rc::new(MockWindowPeer {
                log: self.log.clone(),
                owner,
                retained: Cell::new(false),
                close_request: RefCell::new(None),
                bounds_changed: RefCell::new(None),
            });
            *self.last.borrow_mut() = Some(peer.clone());
            peer
        }
    }

    struct EmptyFactory;

    impl NativePeerFactory for EmptyFactory {
        fn backend_name(&self) -> &'static str {
            "empty"
        }
    }

    fn install_mock() -> Rc<MockFactory> {
        RETAINED.with(|retained| retained.borrow_mut().clear());
        let factory = Rc::new(MockFactory::default());
        register_native_peer_factory(factory.clone());
        factory
    }

    fn peer(factory: &MockFactory) -> Rc<MockWindowPeer> {
        factory.last.borrow().clone().expect("peer realized")
    }

    /// The handler a generated host-composition Window installs: a `Weak` owner, `true` when
    /// the owner accepted the request and ran its own close.
    fn generated_close_handler(window: &Rc<Window>) -> Rc<dyn Fn() -> bool> {
        let weak = Rc::downgrade(window);
        Rc::new(move || {
            let Some(window) = weak.upgrade() else {
                return false;
            };
            window.close();
            true
        })
    }

    #[test]
    fn construction_and_setters_do_not_need_a_backend() {
        clear_native_peer_factory();
        let window = Window::new();
        window.set_title("Title");
        window.set_transparent(true);
        window.set_left(10.0);
        window.set_width(300.0);
        window.hide();
        window.close();
        window.unmount_override();
        WindowLifecycleHost::activate(window.as_ref());
        assert_eq!((window.left(), window.width()), (10.0, 300.0));
        assert!(!window.__has_window_peer());
    }

    #[test]
    fn first_show_realizes_once_with_state_before_handlers() {
        let factory = install_mock();
        let window = Window::new();
        window.set_title("Title");
        window.set_transparent(true);
        window.set_always_on_top(true);
        window.set_left(1.0);
        window.set_top(2.0);
        window.set_width(3.0);
        window.set_height(4.0);
        WindowLifecycleHost::set_close_request_handler(
            window.as_ref(),
            Some(generated_close_handler(&window)),
        );
        WindowLifecycleHost::set_bounds_changed_handler(window.as_ref(), Some(Rc::new(|_| {})));
        assert_eq!(factory.created.get(), 0);

        window.show();
        assert_eq!(
            factory.log.borrow().as_slice(),
            [
                "title:Title",
                "transparent:true",
                "always_on_top:true",
                "left:1",
                "top:2",
                "width:3",
                "height:4",
                "close_request:true",
                "bounds_changed:true",
                "show",
            ]
        );

        // hide / re-show / activate / getters reuse the same peer and the native values.
        window.hide();
        window.show();
        WindowLifecycleHost::activate(window.as_ref());
        assert_eq!((window.left(), window.height()), (11.0, 14.0));
        assert_eq!(factory.created.get(), 1);
        assert_eq!(retained_count(), 1, "re-showing must not retain twice");

        // After realization, setters push straight to the peer.
        factory.log.borrow_mut().clear();
        window.set_title("Renamed");
        window.__set_bounds(Rect {
            x: 5.0,
            y: 6.0,
            width: 7.0,
            height: 8.0,
        });
        assert_eq!(
            factory.log.borrow().as_slice(),
            ["title:Renamed", "left:5", "top:6", "width:7", "height:8"]
        );
        window.close();
        clear_native_peer_factory();
    }

    #[test]
    fn set_content_realizes_so_content_mounts_under_the_native_host() {
        let factory = install_mock();
        let window = Window::new();
        window.set_title("T");
        window.set_content(crate::ui::TextBlock::new());
        assert_eq!(factory.created.get(), 1);
        assert_eq!(factory.log.borrow().as_slice(), ["title:T", "content"]);
        assert!(window.content_element().is_some());
        window.set_content(crate::ui::TextBlock::new());
        assert_eq!(factory.created.get(), 1);
        clear_native_peer_factory();
    }

    #[test]
    fn a_shown_window_is_retained_until_it_closes() {
        let factory = install_mock();
        let window = Window::new();
        let weak = Rc::downgrade(&window);
        window.show();
        drop(window);
        assert!(
            weak.upgrade().is_some(),
            "the registry keeps a shown Window alive after the caller drops it"
        );
        assert_eq!(retained_count(), 1);

        // A never-shown Window is not retained and its close releases nothing.
        let never_shown = Window::new();
        let never_shown_weak = Rc::downgrade(&never_shown);
        never_shown.close();
        drop(never_shown);
        assert!(never_shown_weak.upgrade().is_none());
        assert_eq!(retained_count(), 1);

        weak.upgrade().expect("still retained").close();
        assert!(
            weak.upgrade().is_none(),
            "close releases the registry entry"
        );
        assert_eq!(retained_count(), 0);
        let disposals = factory
            .log
            .borrow()
            .iter()
            .filter(|entry| *entry == "dispose")
            .count();
        assert_eq!(disposals, 1, "dropping the Window disposes its peer once");
        clear_native_peer_factory();
    }

    #[test]
    fn native_close_routes_through_the_weak_handler_and_releases_once() {
        let factory = install_mock();
        let window = Window::new();
        let weak = Rc::downgrade(&window);
        WindowLifecycleHost::set_close_request_handler(
            window.as_ref(),
            Some(generated_close_handler(&window)),
        );
        window.show();
        drop(window);
        let native = peer(&factory);

        // The handler accepts the request and its own close re-enters the peer's close; the
        // Window is released exactly once.
        assert!(native.native_close_request());
        assert!(weak.upgrade().is_none());
        assert_eq!(retained_count(), 0);
        let closes = factory
            .log
            .borrow()
            .iter()
            .filter(|entry| *entry == "close")
            .count();
        assert_eq!(closes, 1);
        // The handler holds only a `Weak`, and dispose cleared it: a stale request is refused.
        assert!(!native.native_close_request());
        clear_native_peer_factory();
    }

    #[test]
    fn a_close_handler_for_a_dropped_owner_reports_owner_gone() {
        let factory = install_mock();
        let owner = Window::new();
        let handler = generated_close_handler(&owner);
        let window = Window::new();
        WindowLifecycleHost::set_close_request_handler(window.as_ref(), Some(handler));
        window.show();
        drop(owner);
        // `false` tells the backend to fall back to its own default close behavior.
        assert!(!peer(&factory).native_close_request());
        window.close();
        clear_native_peer_factory();
    }

    #[test]
    fn unmount_closes_the_active_popup_before_content_teardown() {
        let factory = install_mock();
        let window = Window::new();
        window.show();
        factory.log.borrow_mut().clear();
        window.unmount_override();
        WindowLifecycleHost::set_close_request_handler(window.as_ref(), None);
        assert_eq!(
            factory.log.borrow().as_slice(),
            ["close_active_popup", "close_request:false"]
        );
        window.close();
        clear_native_peer_factory();
    }

    #[test]
    #[should_panic(expected = "the `empty` backend does not support the native control `Window`")]
    fn showing_on_a_backend_without_windows_panics() {
        register_native_peer_factory(Rc::new(EmptyFactory));
        Window::new().show();
    }

    #[test]
    #[should_panic(
        expected = "cannot create the native peer for `Window`: no native backend is registered"
    )]
    fn showing_without_a_backend_panics_with_a_diagnostic() {
        clear_native_peer_factory();
        Window::new().show();
    }
}
