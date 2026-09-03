//! `NSWindow` — including the `makeFirstResponder:` override that routes native focus changes
//! back into core's `FocusTracker` — and the window's content-host wiring.

use super::InnerMenuBar;
use crate::ffi::mtm;
use crate::host::TreeHostView;
use elwindui_core::base::Rect;
use elwindui_core::input::FocusState;
use elwindui_core::ui::UIElementExt;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor,
    NSFloatingWindowLevel, NSNormalWindowLevel, NSResponder, NSScreen, NSView, NSWindow,
    NSWindowDelegate, NSWindowDidMoveNotification, NSWindowDidResizeNotification,
    NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSRect, NSString};
use std::cell::RefCell;
use std::rc::Rc;

/// Walks up from `responder`'s own `NSView` ancestor chain looking for the nearest `TreeHostView`
/// (the window's own top-level content host, or a nested one — `InnerTabView`'s per-tab host,
/// `InnerScrollView`'s content host once that exists, ...) that has `responder`'s *immediate child*
/// registered as one of its own native leaf islands (`TreeHostView::native_containers`). Returns
/// that host together with the owning element's `render_group_id`, ready for
/// `elwindui_core::focus::native_focus_gained`/`native_focus_lost`. Returns `None` for anything not
/// reachable this way — most commonly a `TabView` chip/close button (an `InnerButton` created
/// directly by `create_tab_chip`, never wrapped in a `RenderCommand::NativeControl`) or the
/// `TreeHostView`/`NSWindow` itself becoming first responder (e.g. on window activation with
/// nothing else focused yet) — both are correctly not elwindui-visible focus targets.
fn resolve_focus_owner(
    responder: Option<Retained<NSResponder>>,
) -> Option<(Retained<TreeHostView>, u64)> {
    let mut previous: Option<Retained<NSView>> = None;
    let mut current: Option<Retained<NSView>> = responder.and_then(|r| r.downcast::<NSView>().ok());
    while let Some(view) = current {
        match view.downcast::<TreeHostView>() {
            Ok(host) => {
                let owner_id = previous
                    .as_deref()
                    .and_then(|c| host.resolve_native_owner_id(c))?;
                return Some((host, owner_id));
            }
            Err(view) => {
                current = unsafe { view.superview() };
                previous = Some(view);
            }
        }
    }
    None
}

/// Issue #162 §3.21: the native close-request handler `ElwinduiWindow::windowShouldClose:`
/// consults. Lives on `ElwinduiWindow`'s own ivars (not `InnerWindow`) since that override method
/// only ever has `self: &ElwinduiWindow`/`self.ivars()` in scope, not a real `InnerWindow`.
#[derive(Default)]
pub(crate) struct ElwinduiWindowIvars {
    close_request_handler: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    bounds_changed_handler: RefCell<Option<Rc<dyn Fn(Rect)>>>,
}

define_class!(
    /// A plain `NSWindow` subclass whose only job is bridging AppKit's own first-responder changes
    /// into `elwindui_core::focus::FocusTracker` — see `docs/design/runtime/input_focus_design.md`.
    /// Subclassing the window (rather than every individual native leaf class) is the standard,
    /// minimal-surface-area AppKit technique for observing "did some view anywhere in this window
    /// become/stop being first responder" without per-widget-class overrides, and mirrors this same
    /// file's own `TreeHostView` subclassing convention. Also owns the Issue #162 §3.21 native
    /// close-request veto (`windowShouldClose:`).
    #[unsafe(super(NSWindow))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ElwinduiWindowIvars]
    pub(crate) struct ElwinduiWindow;

    unsafe impl NSObjectProtocol for ElwinduiWindow {}

    // AppKit asks the window delegate about a title-bar close. Keep the delegate on the window
    // subclass itself so the close bridge stored in its ivars is used for both the main window and
    // docking-created floating windows.
    unsafe impl NSWindowDelegate for ElwinduiWindow {}

    impl ElwinduiWindow {
        /// Detects a real, click/API-driven focus change (`ok == true`) and bridges it into
        /// `elwindui_core::focus`. Whether `responder` lands on a *native leaf* window's own
        /// `resolve_focus_owner` decides whether anything happens at all — see that function's own
        /// doc comment for what's intentionally excluded. `FocusState::Pointer` is used
        /// unconditionally for the gained side (Phase 1 simplification — distinguishing a real
        /// mouse click from AppKit's own Tab-driven key-view-loop focus change would need
        /// inspecting `NSApp.currentEvent`, and no such key-view loop is wired between elwindui
        /// elements yet regardless — see `docs/design/runtime/input_focus_design.md`'s "known
        /// limitation" notes on Tab/Shift+Tab out of a focused native control).
        ///
        /// Resolves the target through `host.ivars().render_tree.borrow()` in its own `let`
        /// statement, ending that borrow *before* calling `native_focus_gained` — this used to be
        /// one `if let Some(render_tree) = ...borrow().as_ref() { native_focus_gained(render_tree,
        /// ..) }`, which held the borrow for the whole call. `native_focus_gained` dispatches
        /// `on_got_focus`, which can run arbitrary user code; in `examples/controls-demo`'s TextBox
        /// tab, that handler sets an `#[observable]` field bound to another `TextBlock`, whose
        /// property-change notification synchronously calls `AppKitRelayoutHost::request_relayout`
        /// — which itself needs `render_tree.borrow_mut()` to mark the tree dirty (only the actual
        /// AppKit layout pass is deferred via `setNeedsLayout`, not this). With the borrow still
        /// held from the outer `if let`, that `borrow_mut()` panicked with `BorrowMutError` on every
        /// click or Enter-driven focus change that touched a bound sibling element — crashing the
        /// whole app, since the panic then unwound across this method's own ObjC callback boundary.
        #[unsafe(method(makeFirstResponder:))]
        fn make_first_responder(&self, responder: Option<&NSResponder>) -> Bool {
            let old = self.firstResponder();
            let ok: Bool = unsafe { msg_send![super(self), makeFirstResponder: responder] };
            if !ok.as_bool() {
                return ok;
            }
            let new = self.firstResponder();
            if let Some((host, owner_id)) = resolve_focus_owner(new) {
                let target = host
                    .ivars()
                    .render_tree
                    .borrow()
                    .as_ref()
                    .and_then(|rt| elwindui_core::focus::resolve_native_focus_target(rt, owner_id));
                if let Some(target) = target {
                    elwindui_core::focus::native_focus_gained(
                        &target,
                        &host.ivars().keyboard.focus,
                        FocusState::Pointer,
                    );
                }
            } else if let Some((host, owner_id)) = resolve_focus_owner(old) {
                elwindui_core::focus::native_focus_lost(&host.ivars().keyboard.focus, owner_id);
            }
            ok
        }

        /// Issue #162 §3.19-§3.21: AppKit's own pre-close veto hook — participates in both a
        /// user's title-bar click and a programmatic `-performClose:`, but is *not* consulted by
        /// `NSWindow::close` itself (Apple's documented contract), so the framework's own
        /// generated `Window::close()` calling `InnerWindow::close()` -> `self.ns.close()` at the
        /// end of the common lifecycle never re-enters this method — no reentrancy guard needed
        /// here (unlike WinUI3's `AppWindow.Closing`, which Step 12's WinUI3 half does need one
        /// for). No handler installed (`None` — before `mount_override` ever ran, or after
        /// `unmount_override` cleared it) means "allow the native default": `true`.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, sender: &NSWindow) -> bool {
            // A close handler may synchronously remove the floating host that owns `sender`. Keep
            // the native window alive until AppKit has consumed this delegate return value.
            let _sender_keep_alive = sender.retain();
            let handler = self.ivars().close_request_handler.borrow().clone();
            match handler {
                None => true,
                Some(handler) => {
                    // The generated Window::close() this may call reaches back into AppKit
                    // (InnerWindow::close -> self.ns.close()) — never re-entering this method
                    // (see this fn's own doc comment) but still reentering *other* framework
                    // code, so the handler is called with no borrow of our own ivars held.
                    should_allow_native_close(handler())
                }
            }
        }

        #[unsafe(method(elwinduiWindowBoundsChanged:))]
        fn bounds_changed(&self, _notification: &NSNotification) {
            let Some(handler) = self.ivars().bounds_changed_handler.borrow().clone() else {
                return;
            };
            let frame = self.frame();
            let screen_height = self
                .screen()
                .or_else(|| NSScreen::mainScreen(mtm()))
                .map(|screen| screen.frame().size.height)
                .unwrap_or(0.0);
            let bounds = Rect {
                x: frame.origin.x as f32,
                y: (screen_height - (frame.origin.y + frame.size.height)) as f32,
                width: frame.size.width as f32,
                height: frame.size.height as f32,
            };
            if bounds.x.is_finite()
                && bounds.y.is_finite()
                && bounds.width.is_finite()
                && bounds.height.is_finite()
                && bounds.width > 0.0
                && bounds.height > 0.0
            {
                handler(bounds);
            }
        }
    }
);

/// PR #165 review remediation, A6/T22-T23: pure decision logic for `windowShouldClose:`,
/// extracted so it is unit-testable without any real `NSWindow` construction (which requires the
/// main thread — unavailable in this crate's own `#[test]` harness, see this module's own
/// `type_checked_new_show_hide_close_usage`-style convention elsewhere). Mirrors WinUI3's own
/// `decide_native_close`/`should_veto_native_close` (`elwindui-backend-winui3::inner::window`) —
/// AppKit needs no equivalent `framework_initiated`/reentrancy branch, since `NSWindow::close`
/// never consults `windowShouldClose:` at all (Apple's documented contract, this same `impl`
/// block's own `window_should_close` doc comment).
///
/// `true` (T22): the installed close-request handler accepted the request and is now handling it
/// through the framework's own lifecycle — veto this native close attempt. `false` (T23): the
/// generated owner is already gone — allow AppKit's native default close to proceed.
pub(crate) fn should_allow_native_close(handler_result: bool) -> bool {
    !handler_result
}

#[cfg(test)]
mod native_close_decision_tests {
    use super::*;

    /// T22: the handler accepting the close (`true`) vetoes the native attempt (`false` — do not
    /// allow the native default).
    #[test]
    fn handler_accepting_the_close_vetoes_the_native_attempt() {
        assert!(!should_allow_native_close(true));
    }

    /// T23: the handler declining the close (generated owner already gone, `false`) allows the
    /// native default close to proceed (`true`) — this is the exact case A1 originally got wrong
    /// on the WinUI3 side (cancelling unconditionally regardless of the handler's return value);
    /// AppKit's own implementation never had that bug, but is covered here for parity and
    /// regression protection.
    #[test]
    fn handler_declining_the_close_allows_native_default() {
        assert!(should_allow_native_close(false));
    }
}

/// Raw `NSWindow` + content host — composed by `native_ui::Window`.
#[derive(Clone)]
pub(crate) struct InnerWindow {
    ns: Retained<NSWindow>,
    content_host: Retained<TreeHostView>,
}

impl InnerWindow {
    pub(crate) fn new() -> Self {
        let mtm = mtm();
        let content_rect = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(480.0, 360.0),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // `ElwinduiWindow` (not a stock `NSWindow`) so `makeFirstResponder:` can bridge native
        // focus changes into `elwindui_core::focus` — see that type's own doc comment.
        let ns: Retained<NSWindow> = unsafe {
            let alloc = ElwinduiWindow::alloc(mtm).set_ivars(ElwinduiWindowIvars::default());
            let window: Retained<ElwinduiWindow> = msg_send![
                super(alloc),
                initWithContentRect: content_rect,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ];
            window.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*window)));
            Retained::into_super(window)
        };
        let notifications = objc2_foundation::NSNotificationCenter::defaultCenter();
        unsafe {
            notifications.addObserver_selector_name_object(
                &*ns as &objc2::runtime::AnyObject,
                objc2::sel!(elwinduiWindowBoundsChanged:),
                Some(NSWindowDidMoveNotification),
                Some(&*ns as &objc2::runtime::AnyObject),
            );
            notifications.addObserver_selector_name_object(
                &*ns as &objc2::runtime::AnyObject,
                objc2::sel!(elwinduiWindowBoundsChanged:),
                Some(NSWindowDidResizeNotification),
                Some(&*ns as &objc2::runtime::AnyObject),
            );
        }
        let content_host = TreeHostView::new();
        // `Window` property setters can resize the NSWindow after this content view has been
        // installed (the notepad starts at 640×480 although InnerWindow's construction rect is
        // 480×360). Keep the host synchronized with the client area just like per-tab hosts do.
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        content_host.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        ns.setContentView(Some(&content_host));
        Self { ns, content_host }
    }

    /// Issue #162 §3.18: closes this window's own active custom popup/context-menu surface, if
    /// any — the owner-Window-close half of the popup-before-owner-content teardown ordering
    /// (`Window::unmount_override`, `native_ui::window.rs`).
    pub(crate) fn close_active_popup(&self) {
        self.content_host.close_active_popup();
    }

    /// Issue #162 §3.19-§3.23: stores (or clears) the common Window close callback that
    /// `ElwinduiWindow::windowShouldClose:` (this module's own `define_class!` block) consults.
    /// `self.ns` was originally constructed as a real `ElwinduiWindow` (see `new`, above) before
    /// being upcast to `NSWindow` for storage here, so this downcast always succeeds.
    pub(crate) fn set_close_request_handler(&self, handler: Option<Rc<dyn Fn() -> bool>>) {
        let window = self
            .ns
            .downcast_ref::<ElwinduiWindow>()
            .expect("InnerWindow::ns is always a real ElwinduiWindow");
        *window.ivars().close_request_handler.borrow_mut() = handler;
    }

    pub(crate) fn set_bounds_changed_handler(&self, handler: Option<Rc<dyn Fn(Rect)>>) {
        let window = self
            .ns
            .downcast_ref::<ElwinduiWindow>()
            .expect("InnerWindow::ns is always a real ElwinduiWindow");
        *window.ivars().bounds_changed_handler.borrow_mut() = handler;
    }

    pub(crate) fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.content_host.set_tree(content);
    }

    pub(crate) fn set_transparent(&self, transparent: bool) {
        self.ns.setOpaque(!transparent);
        let background = if transparent {
            NSColor::clearColor()
        } else {
            NSColor::windowBackgroundColor()
        };
        self.ns.setBackgroundColor(Some(&background));
    }

    pub(crate) fn set_always_on_top(&self, always_on_top: bool) {
        self.ns.setLevel(if always_on_top {
            NSFloatingWindowLevel
        } else {
            NSNormalWindowLevel
        });
    }

    fn sync_content_host_frame(&self) {
        let client = self.ns.contentRectForFrameRect(self.ns.frame());
        self.content_host.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            client.size,
        ));
        self.content_host.setNeedsLayout(true);
    }

    pub(crate) fn set_title(&self, title: &str) {
        self.ns.setTitle(&NSString::from_str(title));
    }

    /// Sets `NSApplication.mainMenu` (macOS has one global top menu bar, not a per-window one).
    pub(crate) fn set_menu_bar(&self, menu_bar: &InnerMenuBar) {
        NSApplication::sharedApplication(mtm()).setMainMenu(Some(&menu_bar.ns));
    }

    pub(crate) fn show(&self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        self.ns.makeKeyAndOrderFront(None);
        app.activate();
    }

    pub(crate) fn activate(&self) {
        self.ns.makeKeyAndOrderFront(None);
    }

    /// Visibility only (CI-8 of #80) — `orderOut:` is `makeKeyAndOrderFront:`'s natural AppKit
    /// counterpart; does not close/release the `NSWindow`.
    pub(crate) fn hide(&self) {
        self.ns.orderOut(None);
    }

    /// Releases the native window (CI-8 of #80). `NSWindow::close` (distinct from this same crate's
    /// unrelated `Path::close()` vector-drawing method) triggers the standard AppKit teardown,
    /// including releasing the window from any owner that retains it only via being on-screen.
    pub(crate) fn close(&self) {
        self.ns.close();
    }

    fn screen_height(&self) -> f64 {
        self.ns
            .screen()
            .or_else(|| NSScreen::mainScreen(mtm()))
            .map(|screen| screen.frame().size.height)
            .unwrap_or(0.0)
    }

    pub(crate) fn left(&self) -> f32 {
        self.ns.frame().origin.x as f32
    }

    pub(crate) fn set_left(&self, left: f32) {
        let mut frame = self.ns.frame();
        frame.origin.x = left as f64;
        self.ns.setFrame_display(frame, true);
    }

    pub(crate) fn top(&self) -> f32 {
        let frame = self.ns.frame();
        (self.screen_height() - (frame.origin.y + frame.size.height)) as f32
    }

    pub(crate) fn set_top(&self, top: f32) {
        let screen_height = self.screen_height();
        let mut frame = self.ns.frame();
        frame.origin.y = screen_height - top as f64 - frame.size.height;
        self.ns.setFrame_display(frame, true);
    }

    pub(crate) fn width(&self) -> f32 {
        self.ns.frame().size.width as f32
    }

    pub(crate) fn set_width(&self, width: f32) {
        let mut frame = self.ns.frame();
        frame.size.width = width as f64;
        self.ns.setFrame_display(frame, true);
        self.sync_content_host_frame();
    }

    pub(crate) fn height(&self) -> f32 {
        self.ns.frame().size.height as f32
    }

    pub(crate) fn set_height(&self, height: f32) {
        let mut frame = self.ns.frame();
        let old_height = frame.size.height;
        frame.size.height = height as f64;
        frame.origin.y -= height as f64 - old_height;
        self.ns.setFrame_display(frame, true);
        self.sync_content_host_frame();
    }
}
