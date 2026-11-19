//! `elwindui::ui::Button` — the native push button.

use super::*;
use crate::ui::native::{ButtonPeer, NativePeer, create_native_peer};
use std::cell::OnceCell;

/// How much emphasis a [`Button`] carries, and what it implies about the action behind it.
///
/// Mapped to each toolkit's *own* emphasis affordance rather than to elwindui-drawn styling, so a
/// `Primary` button looks like the platform's accent button and follows the user's system accent
/// colour instead of a colour this framework picked (AppKit `NSButton.bezelColor` /
/// `hasDestructiveAction`, WinUI3 `AccentButtonStyle`). There is deliberately no per-role override
/// beyond this: a role-specific background would fight the system accent colour users expect to
/// control themselves. An explicit `background:` still wins, because `NativeControl`'s background
/// is applied independently of — and after — the role's bezel.
///
/// Orthogonal to [`Button`]'s `is_default`: a role says what kind of action this is, `is_default`
/// says whether Return activates it. A destructive button can be the default one, and often should
/// not be.
///
/// # Example
///
/// ```ignore
/// Button {
///     text: "Delete"
///     role: elwindui::core::ui::ButtonRole::Destructive
///     on_click: vm.delete
/// }
/// ```
///
/// The fully qualified path is required: the DSL resolves an enum-typed property's value as an
/// ordinary Rust expression, so a bare `Destructive` does not resolve. Shorthand would need
/// enum-aware value resolution in `elwindui-codegen`, which does not exist today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonRole {
    /// An ordinary, unemphasized action — the platform's plain push button. The default.
    #[default]
    Normal,
    /// The action this view is primarily for, drawn with the platform's accent fill.
    Primary,
    /// An action that destroys or discards something, drawn with the platform's destructive
    /// treatment (red text on AppKit). Marks intent — it does not add a confirmation step.
    Destructive,
}

/// `elwindui::ui::Button`, the native push button.
///
/// Declared once here as a Core-owned Component (Issue #313). The `#[prop]` lines are this
/// element's whole DSL-visible surface; `#[class]` turns them into `__elwindui_props_Button!` and
/// `__elwindui_ctor_Button!`. The OS widget is a [`ButtonPeer`] that the active backend creates
/// lazily, the first time the button takes part in hosted layout (see [`NativeControl`]). Until
/// then every setter only updates Core's logical state, which is pushed to the peer when it is
/// realized.
///
/// `tooltip` is not declared here: it comes from `NativeControl`, so every native leaf has it.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(enabled: Option<bool>)]
#[prop(role: Option<crate::ui::ButtonRole>)]
#[prop(is_default: Option<bool>)]
#[prop(routed, on_click: fn())]
pub struct Button {
    text: RefCell<String>,
    enabled: Cell<bool>,
    role: Cell<ButtonRole>,
    is_default: Cell<bool>,
    /// A callback installed with `set_on_click`. When present it replaces the routed `on_click`
    /// dispatch for native activation. Shared with the native click callback.
    click_override: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    peer: OnceCell<Rc<dyn ButtonPeer>>,
}

#[elwindui_macros::component]
impl Button {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Activate => {
                let Some(node) = self.as_ui_element().visual_collection.owner_rc() else {
                    return false;
                };
                let args = crate::input::RoutedEventArgs::default();
                crate::ui::dispatch_routed(&node, "on_click", &(), &args);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("Button", |factory| factory.create_button());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a Button's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: runs after `NativeControl` pushed `background`/`tooltip`. Pushes the
    /// button's own state, then installs the click callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self
            .peer
            .get()
            .expect("Button::__initialize_native_peer runs after __create_native_peer")
            .clone();
        // Clone the logical values out before calling the peer: it may re-enter this button.
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        peer.set_enabled(self.enabled.get());
        peer.set_role(self.role.get());
        peer.set_is_default(self.is_default.get());
        // The callback holds only a `Weak` to this button, so the peer never keeps the button
        // alive. It upgrades first and keeps the strong reference until it returns: a click
        // after the button is gone is a no-op (override or not), and a handler that removes the
        // button from its tree cannot drop it mid-dispatch.
        let node = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .map(|node| Rc::downgrade(&node));
        let click_override = self.click_override.clone();
        peer.set_on_click(Some(Box::new(move || {
            let Some(node) = node.as_ref().and_then(Weak::upgrade) else {
                return;
            };
            let replacement = click_override.borrow().clone();
            if let Some(callback) = replacement {
                callback();
            } else {
                let args = crate::input::RoutedEventArgs::default();
                crate::ui::dispatch_routed(&node, "on_click", &(), &args);
            }
            drop(node);
        })));
    }

    /// Registers a `#[routed] on_click` handler on this button's own node. Native activation
    /// dispatches `on_click` starting at this node (see `__create_native_peer`).
    #[inherent]
    pub fn register_routed_handler<T: 'static>(
        &self,
        name: &'static str,
        handler: Box<dyn Fn(&T, &crate::input::RoutedEventArgs)>,
    ) {
        self.base
            .as_ui_element()
            .register_routed_handler(name, handler);
    }

    /// Enables or disables the button, including its accessibility `disabled` state.
    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        if let Some(peer) = self.peer.get() {
            peer.set_enabled(enabled);
        }
        sync_intrinsic_enabled(self.base.as_ui_element(), enabled);
    }

    /// Replaces native activation's routed `on_click` dispatch with `callback`.
    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        *self.click_override.borrow_mut() = Some(Rc::from(callback));
    }

    /// Sets the caption, which is also the accessibility label.
    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_text(text);
        }
        let mut semantics = self
            .base
            .intrinsic_accessibility_semantics()
            .unwrap_or_else(button_accessibility_semantics);
        semantics.label = Some(text.to_string());
        self.base.set_intrinsic_accessibility_semantics(semantics);
        // Some toolkits drop the attributed title when it changes (AppKit `NSButton.setTitle`),
        // and the caption changes the fitting size.
        self.base.reapply_text_style();
        self.base.invalidate_measure();
    }

    /// Applies a [`ButtonRole`]'s native emphasis treatment, replacing whatever the previous role
    /// applied, so setting `Normal` returns the button to the plain platform appearance.
    fn set_role(&self, role: ButtonRole) {
        self.role.set(role);
        if let Some(peer) = self.peer.get() {
            peer.set_role(role);
        }
    }

    /// Makes this the window's default button — the one Return activates. Setting it on more than
    /// one button in the same window is a caller mistake neither toolkit resolves for you.
    fn set_is_default(&self, is_default: bool) {
        self.is_default.set(is_default);
        if let Some(peer) = self.peer.get() {
            peer.set_is_default(is_default);
        }
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            text: RefCell::new(String::new()),
            enabled: Cell::new(true),
            role: Cell::new(ButtonRole::Normal),
            is_default: Cell::new(false),
            click_override: Rc::new(RefCell::new(None)),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        self.base
            .set_intrinsic_accessibility_semantics(button_accessibility_semantics());
        // A native button is a tab stop by default — see
        // docs/design/runtime/input_focus_design.md.
        self.set_tab_stop(true);
    }
}

fn button_accessibility_semantics() -> AccessibilitySemantics {
    let mut semantics = AccessibilitySemantics::new(AccessibilityRole::Button);
    semantics.actions = vec![
        AccessibilityActionKind::Activate,
        AccessibilityActionKind::Focus,
    ];
    semantics
}

fn sync_intrinsic_enabled(node: &dyn UIElementExt, enabled: bool) {
    let Some(mut semantics) = node.intrinsic_accessibility_semantics() else {
        return;
    };
    semantics.state.disabled = !enabled;
    node.set_intrinsic_accessibility_semantics(semantics);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::native::{
        NativePeerFactory, clear_native_peer_factory, register_native_peer_factory,
    };

    /// Records every call a Core `Button` makes on its peer.
    struct MockButtonPeer {
        log: Rc<RefCell<Vec<String>>>,
        background_fails: Rc<Cell<bool>>,
        click: RefCell<Option<Box<dyn Fn()>>>,
        handle: &'static str,
    }

    impl MockButtonPeer {
        fn record(&self, entry: impl Into<String>) {
            self.log.borrow_mut().push(entry.into());
        }
    }

    impl NativePeer for MockButtonPeer {
        fn handle(&self) -> &dyn Any {
            &self.handle
        }
        fn measure(&self, _available: Size) -> Size {
            self.record("measure");
            Size {
                width: 80.0,
                height: 24.0,
            }
        }
        fn apply_background(&self, background: Option<&Brush>) -> bool {
            let accepted = !self.background_fails.get();
            self.record(format!("background:{}:{accepted}", background.is_some()));
            accepted
        }
        fn set_tooltip(&self, tooltip: Option<&str>) {
            self.record(format!("tooltip:{tooltip:?}"));
        }
        fn supports_text_style(&self) -> bool {
            false
        }
        fn apply_text_style(&self, _style: &crate::graphics::CascadedTextStyle) -> bool {
            true
        }
        fn dispose(&self) {
            self.record("dispose");
            self.click.borrow_mut().take();
        }
    }

    impl ButtonPeer for MockButtonPeer {
        fn set_text(&self, text: &str) {
            self.record(format!("text:{text}"));
        }
        fn set_enabled(&self, enabled: bool) {
            self.record(format!("enabled:{enabled}"));
        }
        fn set_role(&self, role: ButtonRole) {
            self.record(format!("role:{role:?}"));
        }
        fn set_is_default(&self, is_default: bool) {
            self.record(format!("is_default:{is_default}"));
        }
        fn set_on_click(&self, callback: Option<Box<dyn Fn()>>) {
            self.record(format!("on_click:{}", callback.is_some()));
            *self.click.borrow_mut() = callback;
        }
    }

    #[derive(Default)]
    struct MockFactory {
        created: Cell<usize>,
        log: Rc<RefCell<Vec<String>>>,
        background_fails: Rc<Cell<bool>>,
        last: RefCell<Option<Rc<MockButtonPeer>>>,
    }

    impl NativePeerFactory for MockFactory {
        fn backend_name(&self) -> &'static str {
            "mock"
        }
        fn create_button(&self) -> Rc<dyn ButtonPeer> {
            self.created.set(self.created.get() + 1);
            let peer = Rc::new(MockButtonPeer {
                log: self.log.clone(),
                background_fails: self.background_fails.clone(),
                click: RefCell::new(None),
                handle: "mock-button",
            });
            *self.last.borrow_mut() = Some(peer.clone());
            peer
        }
    }

    /// A factory that implements no control, like a backend without native controls.
    struct EmptyFactory;

    impl NativePeerFactory for EmptyFactory {
        fn backend_name(&self) -> &'static str {
            "empty"
        }
    }

    fn install_mock() -> Rc<MockFactory> {
        let factory = Rc::new(MockFactory::default());
        register_native_peer_factory(factory.clone());
        factory
    }

    fn available() -> Size {
        Size {
            width: 200.0,
            height: 100.0,
        }
    }

    fn click(factory: &MockFactory) {
        let peer = factory.last.borrow().clone().expect("peer realized");
        let callback = peer.click.borrow_mut().take();
        if let Some(callback) = callback {
            callback();
            *peer.click.borrow_mut() = Some(callback);
        }
    }

    #[test]
    fn construction_and_setters_do_not_need_a_backend() {
        clear_native_peer_factory();
        let button = Button::new();
        button.set_text("Save");
        button.set_enabled(false);
        button.set_tooltip("hint");
        assert!(!button.__has_native_peer());
        let semantics = button.intrinsic_accessibility_semantics().unwrap();
        assert_eq!(semantics.label.as_deref(), Some("Save"));
        assert!(semantics.state.disabled);
    }

    #[test]
    fn peer_is_realized_once_with_the_logical_state_pushed_first() {
        let factory = install_mock();
        let button = Button::new();
        button.set_text("Save");
        button.set_role(ButtonRole::Primary);
        button.set_is_default(true);
        button.set_tooltip("hint");
        button.set_background(Some(Brush::Solid(crate::graphics::Color::rgba(
            1, 2, 3, 255,
        ))));
        assert_eq!(factory.created.get(), 0);

        // Base state first, then the button's own state, then the callback (design §6).
        button.measure(available());
        assert_eq!(button.measured_size().unwrap().width, 80.0);
        assert_eq!(
            factory.log.borrow().as_slice(),
            [
                "background:true:true",
                "tooltip:Some(\"hint\")",
                "text:Save",
                "enabled:true",
                "role:Primary",
                "is_default:true",
                "on_click:true",
                "measure",
            ]
        );

        // Relayout, render collection, hide/show and unmount/remount reuse the same peer.
        button.measure(available());
        let handle = button.try_as_native_control().unwrap();
        assert_eq!(handle.downcast_ref::<&str>(), Some(&"mock-button"));
        button.set_visibility(Visibility::Collapsed);
        button.set_visibility(Visibility::Visible);
        let node: Rc<dyn UIElementExt> = button.clone();
        crate::ui::unmount_subtree(&node);
        button.measure(available());
        assert_eq!(factory.created.get(), 1);

        // After realization, setters push straight to the peer.
        factory.log.borrow_mut().clear();
        button.set_text("Saved");
        button.set_enabled(false);
        assert_eq!(
            factory.log.borrow().as_slice(),
            ["text:Saved", "enabled:false"]
        );
        clear_native_peer_factory();
    }

    #[test]
    fn native_click_dispatches_routed_on_click_once() {
        let factory = install_mock();
        let button = Button::new();
        let clicks = Rc::new(Cell::new(0));
        button.register_routed_handler::<()>(
            "on_click",
            Box::new({
                let clicks = clicks.clone();
                move |_, _| clicks.set(clicks.get() + 1)
            }),
        );
        button.measure(available());
        click(&factory);
        assert_eq!(clicks.get(), 1);

        // `set_on_click` replaces the routed dispatch for native activation.
        let replaced = Rc::new(Cell::new(0));
        button.set_on_click(Box::new({
            let replaced = replaced.clone();
            move || replaced.set(replaced.get() + 1)
        }));
        click(&factory);
        assert_eq!((clicks.get(), replaced.get()), (1, 1));
        clear_native_peer_factory();
    }

    #[test]
    fn a_rejected_background_is_retried_on_later_measure_passes() {
        let factory = install_mock();
        let button = Button::new();
        factory.background_fails.set(true);
        button.set_background(Some(Brush::Solid(crate::graphics::Color::rgba(
            1, 2, 3, 255,
        ))));
        button.measure(available());
        button.invalidate_measure();
        button.measure(available());
        factory.background_fails.set(false);
        button.invalidate_measure();
        button.measure(available());
        // Accepted now: later passes stop pushing it.
        button.invalidate_measure();
        button.measure(available());
        let backgrounds: Vec<String> = factory
            .log
            .borrow()
            .iter()
            .filter(|entry| entry.starts_with("background:"))
            .cloned()
            .collect();
        assert_eq!(
            backgrounds,
            [
                "background:true:false",
                "background:true:false",
                "background:true:true"
            ]
        );
        clear_native_peer_factory();
    }

    #[test]
    fn a_click_after_drop_runs_neither_the_override_nor_the_routed_handler() {
        let factory = install_mock();
        let button = Button::new();
        let clicks = Rc::new(Cell::new(0));
        button.register_routed_handler::<()>(
            "on_click",
            Box::new({
                let clicks = clicks.clone();
                move |_, _| clicks.set(clicks.get() + 1)
            }),
        );
        let replaced = Rc::new(Cell::new(0));
        button.set_on_click(Box::new({
            let replaced = replaced.clone();
            move || replaced.set(replaced.get() + 1)
        }));
        button.measure(available());
        let peer = factory.last.borrow().clone().unwrap();
        let callback = peer.click.borrow_mut().take().expect("click callback");
        let weak = Rc::downgrade(&button);
        drop(button);
        assert!(weak.upgrade().is_none());
        callback();
        assert_eq!((clicks.get(), replaced.get()), (0, 0));
        clear_native_peer_factory();
    }

    #[test]
    fn a_click_handler_that_releases_the_last_owner_finishes_safely() {
        let factory = install_mock();
        let holder: Rc<RefCell<Option<Rc<Button>>>> = Rc::new(RefCell::new(None));
        let button = Button::new();
        button.set_on_click(Box::new({
            let holder = holder.clone();
            move || {
                holder.borrow_mut().take();
            }
        }));
        button.measure(available());
        let weak = Rc::downgrade(&button);
        *holder.borrow_mut() = Some(button);
        let peer = factory.last.borrow().clone().unwrap();
        let callback = peer.click.borrow_mut().take().expect("click callback");
        // The dispatch keeps the button alive until it returns, then the drop disposes once.
        callback();
        assert!(weak.upgrade().is_none());
        let disposals = factory
            .log
            .borrow()
            .iter()
            .filter(|entry| *entry == "dispose")
            .count();
        assert_eq!(disposals, 1);
        clear_native_peer_factory();
    }

    #[test]
    fn dropping_the_button_disposes_the_peer_and_leaves_no_strong_cycle() {
        let factory = install_mock();
        let button = Button::new();
        let clicks = Rc::new(Cell::new(0));
        button.register_routed_handler::<()>(
            "on_click",
            Box::new({
                let clicks = clicks.clone();
                move |_, _| clicks.set(clicks.get() + 1)
            }),
        );
        button.measure(available());
        let peer = factory.last.borrow().clone().unwrap();
        let callback = peer
            .click
            .borrow_mut()
            .take()
            .expect("click callback installed");
        let weak = Rc::downgrade(&button);
        drop(button);

        assert!(
            weak.upgrade().is_none(),
            "the peer must not keep its Button alive"
        );
        let disposals = factory
            .log
            .borrow()
            .iter()
            .filter(|entry| *entry == "dispose")
            .count();
        assert_eq!(disposals, 1);
        // A native click that races with teardown is a no-op.
        callback();
        assert_eq!(clicks.get(), 0);
        clear_native_peer_factory();
    }

    #[test]
    #[should_panic(
        expected = "cannot create the native peer for `Button`: no native backend is registered"
    )]
    fn realizing_without_a_factory_panics_with_a_diagnostic() {
        clear_native_peer_factory();
        Button::new().measure(available());
    }

    #[test]
    #[should_panic(expected = "the `empty` backend does not support the native control `Button`")]
    fn unsupported_control_panics_without_fallback() {
        register_native_peer_factory(Rc::new(EmptyFactory));
        Button::new().measure(available());
    }

    #[test]
    fn realizing_on_another_thread_panics() {
        let _factory = install_mock();
        let result = std::thread::spawn(|| {
            Button::new().measure(available());
        })
        .join();
        let message = result
            .expect_err("realization on a thread without a factory must panic")
            .downcast::<String>()
            .expect("panic message");
        assert!(message.contains("no native backend is registered on thread"));
        clear_native_peer_factory();
    }
}
