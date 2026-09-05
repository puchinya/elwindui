//! The XAML `Window` and its content-host wiring.

use super::InnerMenuBar;
use crate::bindings;
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::{Window as XamlWindow, WindowSizeChangedEventArgs};
use crate::ffi::{
    UiCallbackRegistryOwner, invoke_ui_bool_event_callback, invoke_ui_size_event_callback,
    register_ui_size_event_callback,
};
use crate::host::TreeHostPanel;
use elwindui_core::base::Rect;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use windows::Graphics::{PointInt32, SizeInt32};
use windows::core::HSTRING;
use windows::core::Interface;

/// Issue #225: the single place that decides content-host (and, when a menu bar is present,
/// menu-wrapper) sizing from a Window-logical `width`/`height` — shared by
/// `InnerWindow::sync_content_host_to_window_bounds` (bootstrap: before the first `set_tree`, and
/// right after `Activate()`) and the `Window.SizeChanged` handler registered once in `new()`
/// (ongoing resize), so there is exactly one sizing authority rather than two independently
/// computed ones. A non-positive width or negative height is treated as "not yet a real viewport"
/// (e.g. a transient pre-activation `Bounds` of `0x0`) and left unapplied rather than promoted
/// into a permanent `0x0` viewport for `TreeHostPanel::relayout_static`.
fn apply_window_viewport(
    content_host: &TreeHostPanel,
    menu_wrapper: &RefCell<Option<Canvas>>,
    top_inset: &Cell<f64>,
    width: f64,
    height: f64,
) {
    if !(width > 0.0) || !(height >= 0.0) {
        return;
    }
    if let Some(wrapper) = menu_wrapper.borrow().as_ref() {
        let _ = wrapper.SetWidth(width);
        let _ = wrapper.SetHeight(height);
    }
    let content_height = (height - top_inset.get()).max(0.0);
    content_host.set_viewport_size(width, content_height);
}

/// PR #165 review remediation, A1/T22-T24: pure decision logic for a native `AppWindow.Closing`
/// event, extracted so it is unit-testable without any real WinRT/native window machinery — this
/// crate is `#![cfg(target_os = "windows")]`-gated in its entirety, so a test exercising the real
/// `TypedEventHandler` closure below cannot run outside a Windows environment, but this pure
/// function can be verified anywhere. `try_register_closing_handler`'s closure is a thin wrapper
/// around this and `should_veto_native_close`, below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeCloseDecision {
    /// Let the native close proceed unmodified: either this is the framework's own close
    /// (`framework_initiated_close` — already being handled through `InnerWindow::close()`, T24),
    /// or no close-request handler is installed at all (e.g. `Created`/`Unmounted` state — T21 —
    /// nothing to route through).
    AllowNativeDefault,
    /// A close-request handler is installed and this is a genuine external close attempt —
    /// invoke it; its return value (T22/T23) decides the final outcome via
    /// `should_veto_native_close`.
    InvokeHandler,
}

pub(crate) fn decide_native_close(
    framework_initiated: bool,
    handler_installed: bool,
) -> NativeCloseDecision {
    if framework_initiated || !handler_installed {
        NativeCloseDecision::AllowNativeDefault
    } else {
        NativeCloseDecision::InvokeHandler
    }
}

/// Whether the original native close attempt should be vetoed
/// (`AppWindowClosingEventArgs::SetCancel(true)`), given the close-request handler's own return
/// value. `true` (T22): the framework accepted the close request and is now handling it through
/// its own lifecycle — veto the native attempt. `false` (T23): the generated owner is already
/// gone — allow the native default close to proceed (cancelling here would strand the native
/// window open with no framework owner left to ever close it).
pub(crate) fn should_veto_native_close(handler_result: bool) -> bool {
    handler_result
}

pub(crate) struct InnerWindow {
    xaml: XamlWindow,
    /// Issue #225: `Rc`-wrapped so the `Window.SizeChanged` handler registered in `new()` can hold
    /// only a `Weak<TreeHostPanel>` — never a strong back-reference to this `InnerWindow`/`Window`,
    /// which the native `Window` itself owns the handler's lifetime alongside.
    content_host: Rc<TreeHostPanel>,
    /// Issue #225: the current menu-bar wrapping `Canvas` (`set_menu_bar`), if any — `None` for
    /// the plain no-menu-bar case. Shared with the `Window.SizeChanged` handler via `Rc` so a
    /// menu bar set after construction is still picked up by the one already-registered handler.
    menu_wrapper: Rc<RefCell<Option<Canvas>>>,
    /// Issue #225: vertical space `set_menu_bar` reserves above the content host (`0.0` when no
    /// menu bar is set). Shared with the `Window.SizeChanged` handler for the same reason as
    /// `menu_wrapper`.
    top_inset: Rc<Cell<f64>>,
    retained: Cell<bool>,
    always_on_top: Cell<bool>,
    /// Issue #162 §3.19-§3.23: the common Window close callback a native close affordance
    /// (`AppWindow.Closing`, below) routes through — `Weak<GeneratedWindow>`-capturing, never
    /// `Rc` (acyclic ownership). Wrapped in its own `Rc` so `try_register_closing_handler` can
    /// clone a handle into the event closure without needing a stable reference back to this
    /// `InnerWindow` itself (this struct is embedded by value inside the generated component's
    /// own `base` field, never `Rc`-wrapped on its own).
    close_request_handler: Rc<RefCell<Option<Rc<dyn Fn() -> bool>>>>,
    callback_owner: UiCallbackRegistryOwner,
    /// Issue #162 §3.22: `AppWindow` may not exist yet when `new()` runs (before the window has
    /// ever been shown) — `true` once `AppWindow.Closing` has actually been registered, so `show`
    /// only retries when construction's own best-effort attempt didn't already succeed.
    closing_registered: Cell<bool>,
    /// Issue #162 §3.22 (final requirement): set for the duration of the framework's own
    /// `self.xaml.Close()` call (`close`, below — the tail of the common generated Window close
    /// lifecycle) — `AppWindow.Closing` fires for *any* close, including this one, and the
    /// registered handler (which shares this same `Rc<Cell<bool>>`, for the same "not `Rc`-
    /// wrapped, needs an independently clonable handle" reason `close_request_handler` is
    /// already `Rc`-wrapped) must not treat that as a second, independent user close request.
    framework_initiated_close: Rc<Cell<bool>>,
    bounds_changed_handler: Rc<RefCell<Option<Rc<dyn Fn(Rect)>>>>,
    bounds_changed_registered: Cell<bool>,
}

impl InnerWindow {
    pub(crate) fn new() -> Self {
        let xaml = XamlWindow::new().expect("Window::new");
        let content_host = Rc::new(TreeHostPanel::new());
        let _ = xaml.SetContent(&content_host.as_element());
        let menu_wrapper: Rc<RefCell<Option<Canvas>>> = Rc::new(RefCell::new(None));
        let top_inset = Rc::new(Cell::new(0.0f64));

        // Issue #225: the single top-level Window sizing authority — registered once, for the
        // Window's own life, regardless of whether/when a menu bar is later attached. The
        // generated `Window.SizeChanged` delegate (`TypedEventHandler<IInspectable,
        // WindowSizeChangedEventArgs>`) requires `Send`, which an `Rc`-holding closure is not —
        // same reason every other native handler in this crate goes through `crate::ffi`'s
        // numeric-key indirection (`register_ui_size_event_callback`/
        // `invoke_ui_size_event_callback`) instead of capturing `Rc` state directly. The `Rc`-
        // capturing logic below (upgrading `Weak<TreeHostPanel>`, reading `menu_wrapper`/
        // `top_inset`) lives in the plain Rust closure handed to `register_ui_size_event_callback`
        // — never a strong reference back to this `InnerWindow`/`Window`, so it safely no-ops once
        // the content host has actually been dropped, rather than keeping it (or, transitively,
        // this Window) alive.
        {
            let content_host_weak = Rc::downgrade(&content_host);
            let menu_wrapper = Rc::clone(&menu_wrapper);
            let top_inset = Rc::clone(&top_inset);
            let callback_id = register_ui_size_event_callback(Rc::new(move |width, height| {
                let Some(content_host) = Weak::upgrade(&content_host_weak) else {
                    return;
                };
                apply_window_viewport(&content_host, &menu_wrapper, &top_inset, width, height);
            }));
            let handler = windows::Foundation::TypedEventHandler::new(
                move |_sender, args: windows::core::Ref<'_, WindowSizeChangedEventArgs>| {
                    if let Some(args) = args.as_ref() {
                        if let Ok(size) = args.Size() {
                            invoke_ui_size_event_callback(
                                callback_id,
                                size.Width as f64,
                                size.Height as f64,
                            );
                        }
                    }
                    Ok(())
                },
            );
            let _ = xaml.SizeChanged(&handler);
        }

        let inner = Self {
            xaml,
            content_host,
            menu_wrapper,
            top_inset,
            retained: Cell::new(false),
            always_on_top: Cell::new(false),
            close_request_handler: Rc::new(RefCell::new(None)),
            callback_owner: UiCallbackRegistryOwner::default(),
            closing_registered: Cell::new(false),
            framework_initiated_close: Rc::new(Cell::new(false)),
            bounds_changed_handler: Rc::new(RefCell::new(None)),
            bounds_changed_registered: Cell::new(false),
        };
        // Best-effort: `AppWindow` is commonly already available by construction time, but isn't
        // guaranteed to be (see `closing_registered`'s own doc comment) — `show()` retries.
        inner.try_register_closing_handler();
        inner.try_register_bounds_changed_handler();
        inner
    }

    /// Issue #225: applies the Window's current `Bounds` to the content host (and menu wrapper,
    /// if any) right now, if those bounds already represent a real, initialized viewport. Called
    /// from `set_content` (bounds may already be valid if `Window` was shown before its content
    /// was replaced) and from `show()` immediately after `Activate()`, so the very first Core
    /// layout pass does not have to wait for `Window.SizeChanged` to fire at all — it is still the
    /// mechanism for every *subsequent* resize.
    fn sync_content_host_to_window_bounds(&self) {
        let Ok(bounds) = self.xaml.Bounds() else {
            return;
        };
        apply_window_viewport(
            &self.content_host,
            &self.menu_wrapper,
            &self.top_inset,
            bounds.Width as f64,
            bounds.Height as f64,
        );
    }

    /// Issue #162 §3.22: registers `AppWindow.Closing` at most once. A no-op once already
    /// registered, or while `AppWindow` still isn't available (retried from `show`, below).
    fn try_register_closing_handler(&self) {
        if self.closing_registered.get() {
            return;
        }
        let Some(app_window) = self.app_window() else {
            return;
        };
        let close_request_handler = Rc::clone(&self.close_request_handler);
        let framework_initiated_close = Rc::clone(&self.framework_initiated_close);
        let callback_id = self.callback_owner.register_bool_event(Rc::new(move || {
            let handler = close_request_handler.borrow().clone();
            match decide_native_close(framework_initiated_close.get(), handler.is_some()) {
                NativeCloseDecision::AllowNativeDefault => false,
                NativeCloseDecision::InvokeHandler => {
                    let handler = handler
                        .expect("InvokeHandler is only returned when handler_installed was true");
                    should_veto_native_close(handler())
                }
            }
        }));
        let handler = windows::Foundation::TypedEventHandler::new(
            move |_sender,
                  args: windows::core::Ref<
                '_,
                crate::bindings::Microsoft::UI::Windowing::AppWindowClosingEventArgs,
            >| {
                let Some(args) = args.as_ref() else {
                    return Ok(());
                };
                if invoke_ui_bool_event_callback(callback_id) {
                    let _ = args.SetCancel(true);
                }
                Ok(())
            },
        );
        if app_window.Closing(&handler).is_ok() {
            self.closing_registered.set(true);
        }
    }

    /// Registers the AppWindow geometry notification once the native AppWindow exists. The
    /// callback reads the effective native position and size, so programmatic moves and user
    /// drags share the same model-owned bounds publication path.
    fn try_register_bounds_changed_handler(&self) {
        if self.bounds_changed_registered.get() {
            return;
        }
        let Some(app_window) = self.app_window() else {
            return;
        };
        let bounds_changed_handler = Rc::clone(&self.bounds_changed_handler);
        let handler =
            windows::Foundation::TypedEventHandler::new(move |sender, args: &Option<_>| {
                let args: &Option<
                    crate::bindings::Microsoft::UI::Windowing::AppWindowChangedEventArgs,
                > = args;
                let Some(args) = args else {
                    return Ok(());
                };
                let position_changed = args.DidPositionChange().unwrap_or(false);
                let size_changed = args.DidSizeChange().unwrap_or(false);
                if !position_changed && !size_changed {
                    return Ok(());
                }
                let Some(observed_window) = sender.as_ref() else {
                    return Ok(());
                };
                let Some(position) = observed_window.Position().ok() else {
                    return Ok(());
                };
                let Some(size) = observed_window.Size().ok() else {
                    return Ok(());
                };
                let bounds = Rect {
                    x: position.X as f32,
                    y: position.Y as f32,
                    width: size.Width as f32,
                    height: size.Height as f32,
                };
                if !bounds.x.is_finite()
                    || !bounds.y.is_finite()
                    || !bounds.width.is_finite()
                    || !bounds.height.is_finite()
                    || bounds.width <= 0.0
                    || bounds.height <= 0.0
                {
                    return Ok(());
                }
                if let Some(callback) = bounds_changed_handler.borrow().clone() {
                    callback(bounds);
                }
                Ok(())
            });
        if app_window.Changed(&handler).is_ok() {
            self.bounds_changed_registered.set(true);
        }
    }

    /// Replaces the window's whole content tree — see `TreeHostPanel` for how an `Rc<dyn
    /// UIElement>` (layouts/shapes/text mixed freely with native controls, at any nesting depth)
    /// gets reflected into real XAML elements.
    pub(crate) fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElementExt>) {
        // Issue #225: apply a currently-valid Window viewport (if any) before the tree bootstrap
        // that `set_tree` performs synchronously below — a `Window` shown before its content is
        // replaced already has real `Bounds`, and the new tree should not have to wait for the
        // next `SizeChanged` to get one.
        self.sync_content_host_to_window_bounds();
        self.content_host.set_tree(content);
    }

    pub(crate) fn set_transparent(&self, transparent: bool) {
        self.content_host.set_transparent_background(transparent);
    }

    pub(crate) fn set_always_on_top(&self, always_on_top: bool) {
        self.always_on_top.set(always_on_top);
        self.apply_always_on_top();
    }

    fn apply_always_on_top(&self) {
        use crate::bindings::Microsoft::UI::Windowing::OverlappedPresenter;

        if let Some(presenter) = self
            .app_window()
            .and_then(|window| window.Presenter().ok())
            .and_then(|presenter| presenter.cast::<OverlappedPresenter>().ok())
        {
            let _ = presenter.SetIsAlwaysOnTop(self.always_on_top.get());
        }
    }

    pub(crate) fn set_title(&self, title: &str) {
        let _ = self.xaml.SetTitle(&HSTRING::from(title));
    }

    /// `Microsoft.UI.Xaml.Controls.MenuBar` is placed as a real element *above* the content host,
    /// unlike AppKit's single global `NSApplication.mainMenu` — this repacks `Window`'s content
    /// into a two-row layout (`MenuBar`, then the existing content host) the first time a menu bar
    /// is set. `VerticalLayout`/`HorizontalLayout` aren't available here (no backend struct — see
    /// the crate's module doc comment), so this uses a plain `Canvas`-less stack: a small dedicated
    /// host `Grid` with two rows would be the idiomatic XAML way to do this; simplified here to
    /// stacking two elements inside a fresh outer `Canvas` sized/positioned manually, mirroring
    /// `TreeHostPanel`'s own "don't trust native auto-layout, position everything explicitly"
    /// approach.
    ///
    /// Issue #225: a plain `Canvas`'s children do not stretch to fill it the way `Window.Content`
    /// stretches to fill the window, so this outer wrapper `Canvas` needs the same explicit
    /// sizing `content_host` does in the no-menu-bar case. Rather than this method owning its own
    /// independent `SizeChanged` handler (the previous approach — a second, competing sizing
    /// authority alongside the one `new()` already registers on `Window` itself, and one this
    /// investigation never actually confirmed fires reliably either, for the same "plain `Canvas`
    /// set as `Window.Content`" reason the no-menu-bar path's own `Canvas.SizeChanged` does not),
    /// this stores `outer` in `menu_wrapper` and the height inset in `top_inset` — the single
    /// `Window.SizeChanged` handler `new()` already registered picks both up on the very next
    /// resize, and `sync_content_host_to_window_bounds` (called below) applies the current
    /// `Window.Bounds` immediately.
    pub(crate) fn set_menu_bar(&self, menu_bar: &InnerMenuBar) {
        const MENU_BAR_HEIGHT: f64 = 32.0;
        let outer = Canvas::new().expect("Canvas::new");
        if let Ok(children) = outer.Children() {
            let _ = children.Append(&menu_bar.xaml);
            let _ = children.Append(&self.content_host.as_element());
            let _ = Canvas::SetTop(&self.content_host.as_element(), MENU_BAR_HEIGHT);
        }
        self.top_inset.set(MENU_BAR_HEIGHT);
        *self.menu_wrapper.borrow_mut() = Some(outer.clone());
        let _ = self.xaml.SetContent(&outer);
        self.sync_content_host_to_window_bounds();
    }

    /// Shows the window and retains its native wrapper until WinUI reports that it closed.
    pub(crate) fn show(&self) {
        self.apply_always_on_top();
        // Issue #162 §3.22: `AppWindow` is guaranteed to exist by this point (this same method's
        // own `apply_always_on_top`/`app_window()` calls above already rely on that) — retry here
        // in case `new()`'s own best-effort attempt ran too early.
        self.try_register_closing_handler();
        self.try_register_bounds_changed_handler();
        if !self.retained.replace(true) {
            crate::app::retain_window(&self.xaml);
        }
        let _ = self.xaml.Activate();
        // Issue #225: `Window.Bounds` is commonly already valid immediately after `Activate()` —
        // apply it now rather than waiting for the first `Window.SizeChanged` to fire, so the
        // first Core layout pass after a fresh `show()` does not have to guess at a `0x0`
        // viewport. `Window.SizeChanged` (registered once, in `new()`) remains the mechanism for
        // every subsequent resize, and covers the case where `Bounds` is not valid yet even here.
        self.sync_content_host_to_window_bounds();
    }

    pub(crate) fn activate(&self) {
        let _ = self.xaml.Activate();
    }

    /// Visibility only (CI-8 of #80): `AppWindow.Hide()` (Windows App SDK 1.3+, already available
    /// here via `app_window()` for position/size) is the natural counterpart to `show()`'s
    /// `Activate()` — does not close the window or release `crate::app`'s retain-list entry.
    pub(crate) fn hide(&self) {
        if let Some(app_window) = self.app_window() {
            let _ = app_window.Hide();
        }
    }

    /// Releases the native window (CI-8 of #80). `Window.Close()` fires the native `Closed` event
    /// this same `InnerWindow::show()`/`crate::app::retain_window` already registers a handler for
    /// (`crate::app::release_window`), so closing programmatically here reaches the exact same
    /// retain-list cleanup / possible-app-exit path a user clicking the close box already does —
    /// deliberately reusing that reactive path rather than duplicating its bookkeeping.
    pub(crate) fn close(&self) {
        // Issue #162 §3.22: guards the registered `AppWindow.Closing` handler against treating
        // this framework-initiated close as a second, independent user request (see
        // `framework_initiated_close`'s own doc comment). Cleared unconditionally afterward —
        // `self.xaml.Close()` is itself idempotent/safe to call again later were `close()` ever
        // re-entered, and leaving the flag set would incorrectly suppress a *future*, genuinely
        // new close request.
        self.framework_initiated_close.set(true);
        let _ = self.xaml.Close();
        self.framework_initiated_close.set(false);
    }

    /// Issue #162 §3.18: closes this window's own active custom popup/context-menu surface, if
    /// any — the owner-Window-close half of the popup-before-owner-content teardown ordering
    /// (`Window::unmount_override`, `native_ui::window.rs`).
    pub(crate) fn close_active_popup(&self) {
        self.content_host.close_active_popup();
    }

    /// Issue #162 §3.19-§3.23: stores (or clears) the common Window close callback
    /// `try_register_closing_handler`'s own registered `AppWindow.Closing` handler consults.
    pub(crate) fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        *self.close_request_handler.borrow_mut() = handler;
    }

    pub(crate) fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        *self.bounds_changed_handler.borrow_mut() = handler;
        self.try_register_bounds_changed_handler();
    }

    #[cfg(test)]
    pub(crate) fn is_visible_for_test(&self) -> bool {
        self.app_window()
            .and_then(|window| window.IsVisible().ok())
            .unwrap_or(false)
    }

    /// `Window.AppWindow` (Windows App SDK 1.3+) already handles the `WinRT.Interop.WindowNative`/
    /// `Win32Interop.GetWindowIdFromWindow` dance internally, so no manual interop is needed here.
    fn app_window(&self) -> Option<bindings::Microsoft::UI::Windowing::AppWindow> {
        self.xaml.AppWindow().ok()
    }

    /// WinUI3's `AppWindow.Position.X`/`.Y` and `AppWindow.Size.Width`/`.Height` are already
    /// top-left-origin, Y increasing downward — unlike `elwindui-backend-appkit`'s `Window`, no
    /// coordinate conversion is needed here. `None` (no `AppWindow` yet, e.g. before the window has
    /// ever been shown) reads back as `0.0`.
    pub(crate) fn left(&self) -> f32 {
        self.app_window()
            .and_then(|w| w.Position().ok())
            .map(|p| p.X as f32)
            .unwrap_or(0.0)
    }

    pub(crate) fn set_left(&self, left: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(position) = app_window.Position() {
                let _ = app_window.Move(PointInt32 {
                    X: left as i32,
                    Y: position.Y,
                });
            }
        }
    }

    pub(crate) fn top(&self) -> f32 {
        self.app_window()
            .and_then(|w| w.Position().ok())
            .map(|p| p.Y as f32)
            .unwrap_or(0.0)
    }

    pub(crate) fn set_top(&self, top: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(position) = app_window.Position() {
                let _ = app_window.Move(PointInt32 {
                    X: position.X,
                    Y: top as i32,
                });
            }
        }
    }

    pub(crate) fn width(&self) -> f32 {
        self.app_window()
            .and_then(|w| w.Size().ok())
            .map(|s| s.Width as f32)
            .unwrap_or(0.0)
    }

    pub(crate) fn set_width(&self, width: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(size) = app_window.Size() {
                let _ = app_window.Resize(SizeInt32 {
                    Width: width as i32,
                    Height: size.Height,
                });
            }
        }
    }

    pub(crate) fn height(&self) -> f32 {
        self.app_window()
            .and_then(|w| w.Size().ok())
            .map(|s| s.Height as f32)
            .unwrap_or(0.0)
    }

    pub(crate) fn set_height(&self, height: f32) {
        if let Some(app_window) = self.app_window() {
            if let Ok(size) = app_window.Size() {
                let _ = app_window.Resize(SizeInt32 {
                    Width: size.Width,
                    Height: height as i32,
                });
            }
        }
    }
}

#[cfg(test)]
mod native_close_decision_tests {
    use super::*;

    /// T24: a framework-initiated close (the framework's own `InnerWindow::close()` in progress)
    /// always allows the native default, regardless of whether a handler is installed — the
    /// reentrancy guard takes priority over handler dispatch.
    #[test]
    fn framework_initiated_close_allows_native_default_even_with_handler_installed() {
        assert_eq!(
            decide_native_close(true, true),
            NativeCloseDecision::AllowNativeDefault
        );
        assert_eq!(
            decide_native_close(true, false),
            NativeCloseDecision::AllowNativeDefault
        );
    }

    /// T21: no close-request handler installed (e.g. the generated owner never reached
    /// `mount_override`, or already cleared it in `unmount_override`) allows the native default.
    #[test]
    fn no_handler_installed_allows_native_default() {
        assert_eq!(
            decide_native_close(false, false),
            NativeCloseDecision::AllowNativeDefault
        );
    }

    /// A genuine external close attempt with a handler installed must invoke it.
    #[test]
    fn external_close_with_handler_installed_invokes_handler() {
        assert_eq!(
            decide_native_close(false, true),
            NativeCloseDecision::InvokeHandler
        );
    }

    /// T22: the handler returning `true` (framework accepted and is handling the close) vetoes
    /// the original native close attempt.
    #[test]
    fn handler_accepting_the_close_vetoes_the_native_attempt() {
        assert!(should_veto_native_close(true));
    }

    /// T23: the handler returning `false` (generated owner already gone) allows the native
    /// default close to proceed — this is the exact case A1 originally got wrong (cancelling
    /// unconditionally regardless of the handler's return value).
    #[test]
    fn handler_declining_the_close_allows_native_default() {
        assert!(!should_veto_native_close(false));
    }
}
