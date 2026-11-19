//! Backend peer contract for Core-owned native Components (Issue #313,
//! `docs/design/runtime/native_peer_design.md`).
//!
//! Each native builtin (`Button`, ...) is declared once in `elwindui-core` as an ordinary
//! Component. Its OS widget is a *peer*: an object implementing one of the traits below, created
//! lazily by the [`NativePeerFactory`] the active backend registered for the current UI thread.
//! Core owns the public properties, logical state, routed events, and lifecycle. The peer only
//! translates values to the native widget and reports native notifications back through callbacks
//! Core installs.
//!
//! These are backend-implementer traits, not DSL surface. Core never names a platform type: the
//! native handle crosses the boundary only as `&dyn Any` ([`NativePeer::handle`]), which the
//! backend's own render/host reconciliation downcasts to its handle type.

use crate::base::Size;
use crate::graphics::{Brush, CascadedTextStyle};
use crate::ui::ButtonRole;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// The native widget behind a `NativeControl`.
///
/// Every method is called on the UI thread that created the peer. Core never holds a `RefCell`
/// borrow across a peer call, so an implementation may synchronously re-enter Core (for example
/// through a native event raised by a setter).
pub trait NativePeer {
    /// The backend's type-erased native handle (for example the WinUI3/AppKit `AnyView`).
    ///
    /// Returned to Core's render pipeline through `UIElementExt::try_as_native_control`, whose
    /// consumer downcasts it to the backend handle type. It must stay the same object for the
    /// peer's whole lifetime.
    fn handle(&self) -> &dyn Any;

    /// The widget's natural size for `available`, used as the control's `measure_override`.
    fn measure(&self, available: Size) -> Size;

    /// Applies an explicit background, or restores the toolkit default for `None`.
    ///
    /// Returns `false` when the native call failed. Core then keeps the value pending and pushes
    /// the current logical background again at the start of every later measure pass until the
    /// peer returns `true`; a newer `set_background`/`clear_background` replaces the pending value.
    fn apply_background(&self, background: Option<&Brush>) -> bool;

    /// Sets the hover tooltip, or removes it for `None`.
    fn set_tooltip(&self, tooltip: Option<&str>);

    /// Whether this widget can display a text style at all. Core skips text-style resolution for
    /// widgets that return `false`.
    fn supports_text_style(&self) -> bool;

    /// Applies a resolved text style. Returns `false` when the native call failed; Core then
    /// retries on the next measure pass instead of caching the style as applied.
    fn apply_text_style(&self, style: &CascadedTextStyle) -> bool;

    /// Whether Core must re-push the resolved text style on every measure pass instead of only
    /// when it changed.
    ///
    /// For widgets that can silently lose their attributed text when a sibling subtree is
    /// reconciled (AppKit `NSButton`). The default is `false`.
    fn refresh_text_style_on_measure(&self) -> bool {
        false
    }

    /// Detaches every native target and callback registration this peer owns.
    ///
    /// Called exactly once by Core when the owning Component is dropped. Must be idempotent and,
    /// once it returns, no callback previously installed by Core may run again. The default does
    /// nothing, for peers that own no registrations.
    fn dispose(&self) {}
}

/// The native push button behind `elwindui::ui::Button`.
pub trait ButtonPeer: NativePeer {
    /// Sets the button caption.
    fn set_text(&self, text: &str);

    /// Enables or disables native interaction.
    fn set_enabled(&self, enabled: bool);

    /// Applies a [`ButtonRole`]'s native emphasis, fully resetting the previous role's effect.
    fn set_role(&self, role: ButtonRole);

    /// Marks the button as the window default (activated by Return).
    fn set_is_default(&self, is_default: bool);

    /// Installs, or with `None` removes, the native activation callback.
    ///
    /// Core installs exactly one callback when the peer is realized. It captures only a `Weak`
    /// reference to the Component, so it is a no-op after the Component is gone.
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>);
}

/// Creates the native peers for one backend.
///
/// A backend registers exactly one factory per UI thread from its `init()` through
/// [`register_native_peer_factory`]. Every `create_*` method has a default body that panics with
/// an "unsupported control" diagnostic, so a backend that does not provide a control (for example
/// GTK4 today) fails loudly instead of silently falling back.
pub trait NativePeerFactory {
    /// A short backend name used in diagnostics (for example `"winui3"`).
    fn backend_name(&self) -> &'static str;

    /// Creates the peer for a new `Button`.
    fn create_button(&self) -> Rc<dyn ButtonPeer> {
        unsupported_native_control(self.backend_name(), "Button")
    }
}

thread_local! {
    static NATIVE_PEER_FACTORY: RefCell<Option<Rc<dyn NativePeerFactory>>> =
        const { RefCell::new(None) };
}

/// Registers `factory` as this UI thread's native peer factory, replacing any previous one.
///
/// Called by each backend's `init()` on the thread that calls `elwindui::init()`. Peers that
/// already exist keep working; only later realizations use the new factory. Replacing a factory
/// is intended for tests (each test thread registers its own mock); production code registers
/// once.
pub fn register_native_peer_factory(factory: Rc<dyn NativePeerFactory>) {
    NATIVE_PEER_FACTORY.with(|cell| *cell.borrow_mut() = Some(factory));
}

/// Removes this thread's native peer factory. Mainly for test hygiene.
pub fn clear_native_peer_factory() {
    NATIVE_PEER_FACTORY.with(|cell| *cell.borrow_mut() = None);
}

/// Whether a native peer factory is registered on the current thread.
pub fn has_native_peer_factory() -> bool {
    NATIVE_PEER_FACTORY.with(|cell| cell.borrow().is_some())
}

/// Runs `create` with this thread's registered factory, for realizing a peer of `control`.
///
/// # Panics
///
/// Panics with a diagnostic naming `control` and the current thread when no factory is
/// registered on this thread. That happens when a native control is realized before
/// `elwindui::init()`, or on a thread other than the UI thread that called it. There is no
/// fallback backend.
pub fn create_native_peer<R>(
    control: &'static str,
    create: impl FnOnce(&dyn NativePeerFactory) -> R,
) -> R {
    // Clone the `Rc` out first so the factory may itself register callbacks or re-enter the
    // registry without hitting a `RefCell` double borrow.
    let factory = NATIVE_PEER_FACTORY.with(|cell| cell.borrow().clone());
    match factory {
        Some(factory) => create(factory.as_ref()),
        None => {
            let thread = std::thread::current();
            panic!(
                "elwindui: cannot create the native peer for `{control}`: no native backend is \
                 registered on thread {:?} ({:?}). Call `elwindui::init()` on the UI thread before \
                 showing native controls, and only use them from that thread.",
                thread.name().unwrap_or("<unnamed>"),
                thread.id(),
            )
        }
    }
}

/// Panics with the standard "unsupported control" diagnostic. Used by the default
/// [`NativePeerFactory`] methods.
pub fn unsupported_native_control(backend: &str, control: &str) -> ! {
    panic!(
        "elwindui: the `{backend}` backend does not support the native control `{control}`. \
         No fallback is used; use a backend that implements `{control}`."
    )
}
