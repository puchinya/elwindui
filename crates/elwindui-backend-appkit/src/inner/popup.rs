//! Popup surface implementation for macOS AppKit.
//!
//! Hosts arbitrary `UIElement` subtrees inside a borderless, floating, transparent `NSWindow`
//! above normal content and native controls.

use crate::ffi::mtm;
use crate::host::TreeHostView;
use elwindui_core::base::{Point, Size};
use elwindui_core::ui::{PopupHost, PopupSurfaceHandle, UIElementExt};
use objc2::rc::Retained;
use objc2::{MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use std::rc::Rc;

/// Internal AppKit representation of a standalone popup surface.
pub(crate) struct InnerPopupSurface {
    window: Retained<NSWindow>,
    #[allow(dead_code)]
    content_host: Retained<TreeHostView>,
    is_open: RefCell<bool>,
}

impl InnerPopupSurface {
    /// Creates and immediately displays a new popup surface.
    pub(crate) fn show(
        content: Rc<dyn UIElementExt>,
        position: Point,
        size: Size,
    ) -> Rc<Self> {
        let m = mtm();
        let bottom_y = (position.y - size.height) as f64;
        let content_rect = NSRect::new(
            NSPoint::new(position.x as f64, bottom_y),
            NSSize::new(size.width as f64, size.height as f64),
        );
        let style = NSWindowStyleMask::Borderless;
        let window: Retained<NSWindow> = unsafe {
            let alloc = NSWindow::alloc(m);
            msg_send![
                alloc,
                initWithContentRect: content_rect,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };

        // Ensure popup displays above floating windows and native controls
        window.setLevel(NSFloatingWindowLevel + 2);
        window.setOpaque(false);
        window.setHasShadow(true);
        window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));

        let content_host = TreeHostView::new();
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        content_host.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(size.width as f64, size.height as f64),
        ));
        content_host.set_tree(content);
        window.setContentView(Some(&content_host));

        let surface = Rc::new(Self {
            window,
            content_host,
            is_open: RefCell::new(true),
        });

        surface.window.makeKeyAndOrderFront(None);
        surface.window.orderFrontRegardless();
        surface.window.display();
        surface
    }

    /// Closes and removes the popup surface from the screen.
    pub(crate) fn close(&self) {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
            self.window.orderOut(None);
        }
    }
}

/// Handle implementing [`PopupSurfaceHandle`] for programmatic dismissal.
#[derive(Clone)]
pub struct AppKitPopupHandle {
    surface: Rc<InnerPopupSurface>,
}

impl PopupSurfaceHandle for AppKitPopupHandle {
    fn close(&self) {
        self.surface.close();
    }
}

/// Default [`PopupHost`] implementation for AppKit.
#[derive(Default, Clone)]
pub struct AppKitPopupHost;

impl PopupHost for AppKitPopupHost {
    fn show_popup(
        &self,
        content: Rc<dyn UIElementExt>,
        position: Point,
        size: Size,
    ) -> Rc<dyn PopupSurfaceHandle> {
        let surface = InnerPopupSurface::show(content, position, size);
        Rc::new(AppKitPopupHandle { surface })
    }
}
