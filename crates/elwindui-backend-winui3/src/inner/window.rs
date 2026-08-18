//! The XAML `Window` and its content-host wiring.

use super::InnerMenuBar;
use crate::bindings;
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::{SizeChangedEventHandler, Window as XamlWindow};
use crate::host::TreeHostPanel;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use windows::Graphics::{PointInt32, SizeInt32};
use windows::core::HSTRING;
use windows::core::Interface;

pub(crate) struct InnerWindow {
    xaml: XamlWindow,
    content_host: TreeHostPanel,
    retained: Cell<bool>,
    always_on_top: Cell<bool>,
    /// Issue #162 §3.19-§3.23: the common Window close callback a native close affordance
    /// (`AppWindow.Closing`, below) routes through — `Weak<GeneratedWindow>`-capturing, never
    /// `Rc` (acyclic ownership). Wrapped in its own `Rc` so `try_register_closing_handler` can
    /// clone a handle into the event closure without needing a stable reference back to this
    /// `InnerWindow` itself (this struct is embedded by value inside the generated component's
    /// own `base` field, never `Rc`-wrapped on its own).
    close_request_handler: Rc<RefCell<Option<Rc<dyn Fn() -> bool>>>>,
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
}

impl InnerWindow {
    pub(crate) fn new() -> Self {
        let xaml = XamlWindow::new().expect("Window::new");
        let content_host = TreeHostPanel::new();
        let _ = xaml.SetContent(&content_host.as_element());
        let inner = Self {
            xaml,
            content_host,
            retained: Cell::new(false),
            always_on_top: Cell::new(false),
            close_request_handler: Rc::new(RefCell::new(None)),
            closing_registered: Cell::new(false),
            framework_initiated_close: Rc::new(Cell::new(false)),
        };
        // Best-effort: `AppWindow` is commonly already available by construction time, but isn't
        // guaranteed to be (see `closing_registered`'s own doc comment) — `show()` retries.
        inner.try_register_closing_handler();
        inner
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
        let handler = windows::Foundation::TypedEventHandler::new(move |_, args: &Option<_>| {
            let args: &Option<
                crate::bindings::Microsoft::UI::Windowing::AppWindowClosingEventArgs,
            > = args;
            let Some(args) = args else {
                return Ok(());
            };
            if framework_initiated_close.get() {
                // The framework's own final native close (`InnerWindow::close`, below) — let it
                // proceed unmodified; the close-request handler must not run again for our own
                // close (it would try to re-invoke `WindowExt::close()` on an already-closing
                // Window).
                return Ok(());
            }
            let handler = close_request_handler.borrow().clone();
            let Some(handler) = handler else {
                return Ok(());
            };
            // Prevent this native close attempt from proceeding independently — the framework's
            // own lifecycle decides whether/when the real native close happens (via this same
            // `InnerWindow::close()`, `framework_initiated_close`-guarded so it isn't re-entered
            // through this same handler once it runs).
            let _ = args.SetCancel(true);
            handler();
            Ok(())
        });
        if app_window.Closing(&handler).is_ok() {
            self.closing_registered.set(true);
        }
    }

    /// Replaces the window's whole content tree — see `TreeHostPanel` for how an `Rc<dyn
    /// UIElement>` (layouts/shapes/text mixed freely with native controls, at any nesting depth)
    /// gets reflected into real XAML elements.
    pub(crate) fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElementExt>) {
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
    pub(crate) fn set_menu_bar(&self, menu_bar: &InnerMenuBar) {
        const MENU_BAR_HEIGHT: f64 = 32.0;
        let outer = Canvas::new().expect("Canvas::new");
        if let Ok(children) = outer.Children() {
            let _ = children.Append(&menu_bar.xaml);
            let _ = children.Append(&self.content_host.as_element());
            let _ = Canvas::SetTop(&self.content_host.as_element(), MENU_BAR_HEIGHT);
        }
        // A plain `Canvas`'s children do not stretch to fill it the way `Window.Content` stretches
        // to fill the window — unlike the no-menu-bar case (`new`, above), where `content_host` is
        // set directly as `Window.Content` and so inherits that automatic fill, here it is merely
        // one more `Canvas` child and never receives an explicit `Width`/`Height` otherwise.
        // Without this, `content_host.ActualWidth`/`ActualHeight` stay `0` forever (nothing ever
        // sets or resizes them), so `TreeHostPanel::relayout_static` never lays anything out and
        // its whole subtree — `TabView`, every native control and drawn primitive in it — never
        // gets a visible size, even though property updates (e.g. `TextArea::set_text`) keep
        // reaching the native controls underneath correctly. Mirrors the same `SizeChanged`-driven
        // bootstrap `TreeHostPanel::new` already uses for its own `Canvas` in the no-menu-bar case.
        let content_host = self.content_host.as_element();
        let resize = {
            let outer = outer.clone();
            let content_host = content_host.clone();
            move || {
                let width = outer.ActualWidth().unwrap_or(0.0);
                let height = (outer.ActualHeight().unwrap_or(0.0) - MENU_BAR_HEIGHT).max(0.0);
                let _ = content_host.SetWidth(width);
                let _ = content_host.SetHeight(height);
            }
        };
        resize();
        let _ = outer.SizeChanged(&SizeChangedEventHandler::new(move |_, _| {
            resize();
            Ok(())
        }));
        let _ = self.xaml.SetContent(&outer);
    }

    /// Shows the window and retains its native wrapper until WinUI reports that it closed.
    pub(crate) fn show(&self) {
        self.apply_always_on_top();
        // Issue #162 §3.22: `AppWindow` is guaranteed to exist by this point (this same method's
        // own `apply_always_on_top`/`app_window()` calls above already rely on that) — retry here
        // in case `new()`'s own best-effort attempt ran too early.
        self.try_register_closing_handler();
        if !self.retained.replace(true) {
            crate::app::retain_window(&self.xaml);
        }
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
