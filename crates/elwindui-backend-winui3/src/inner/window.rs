//! The XAML `Window` and its content-host wiring.

use crate::host::TreeHostPanel;
use super::InnerMenuBar;
use crate::bindings;
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::{SizeChangedEventHandler, Window as XamlWindow};
use windows::Graphics::{PointInt32, SizeInt32};
use std::cell::Cell;
use std::rc::Rc;
use windows::core::HSTRING;

pub(crate) struct InnerWindow {
    xaml: XamlWindow,
    content_host: TreeHostPanel,
    retained: Cell<bool>,
}

impl InnerWindow {
    pub(crate) fn new() -> Self {
        let xaml = XamlWindow::new().expect("Window::new");
        let content_host = TreeHostPanel::new();
        let _ = xaml.SetContent(&content_host.as_element());
        Self { xaml, content_host, retained: Cell::new(false) }
    }

    /// Replaces the window's whole content tree — see `TreeHostPanel` for how an `Rc<dyn
    /// UIElement>` (layouts/shapes/text mixed freely with native controls, at any nesting depth)
    /// gets reflected into real XAML elements.
    pub(crate) fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.content_host.set_tree(content);
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
        if !self.retained.replace(true) {
            crate::app::retain_window(&self.xaml);
        }
        let _ = self.xaml.Activate();
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
