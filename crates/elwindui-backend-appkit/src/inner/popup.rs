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
use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSEventType, NSFloatingWindowLevel,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;

/// Internal AppKit representation of a standalone popup surface.
pub(crate) struct InnerPopupSurface {
    window: Retained<NSWindow>,
    #[allow(dead_code)]
    content_host: Retained<TreeHostView>,
    is_open: RefCell<bool>,
    local_monitor: RefCell<Option<Retained<AnyObject>>>,
    global_monitor: RefCell<Option<Retained<AnyObject>>>,
}

impl InnerPopupSurface {
    /// Creates and immediately displays a new popup surface.
    pub(crate) fn show(
        content: Rc<dyn UIElementExt>,
        position: Point,
        size: Size,
    ) -> Rc<Self> {
        let m = mtm();
        let content_rect = NSRect::new(
            NSPoint::new(position.x as f64, position.y as f64),
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
            local_monitor: RefCell::new(None),
            global_monitor: RefCell::new(None),
        });

        surface.window.makeKeyAndOrderFront(None);
        surface.window.orderFrontRegardless();
        surface.window.display();

        // Install local and global event monitors for light-dismiss (dismiss on outside click or Esc)
        let popup_window_num = surface.window.windowNumber();
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::RightMouseDown
            | NSEventMask::OtherMouseDown
            | NSEventMask::KeyDown;

        let weak_surface = Rc::downgrade(&surface);
        let local_block = RcBlock::new(move |event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { event_ptr.as_ref() };
            if let Some(s) = weak_surface.upgrade() {
                if event.r#type() == NSEventType::KeyDown {
                    if event.keyCode() == 53 {
                        // Esc key dismisses popup and consumes the key event
                        s.close();
                        return std::ptr::null_mut();
                    }
                } else if event.windowNumber() != popup_window_num {
                    // Click outside the popup window within the app
                    s.close();
                }
            }
            event_ptr.as_ptr()
        });

        let local_mon = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local_block)
        };
        *surface.local_monitor.borrow_mut() = local_mon;

        let weak_surface_global = Rc::downgrade(&surface);
        let global_mask = NSEventMask::LeftMouseDown
            | NSEventMask::RightMouseDown
            | NSEventMask::OtherMouseDown;
        let global_block = RcBlock::new(move |_event_ptr: NonNull<NSEvent>| {
            if let Some(s) = weak_surface_global.upgrade() {
                // Click outside the application (desktop / other apps / menubar)
                s.close();
            }
        });

        let global_mon =
            NSEvent::addGlobalMonitorForEventsMatchingMask_handler(global_mask, &global_block);
        *surface.global_monitor.borrow_mut() = global_mon;

        surface
    }

    /// Closes and removes the popup surface from the screen.
    pub(crate) fn close(&self) {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
            if let Some(mon) = self.local_monitor.borrow_mut().take() {
                unsafe { NSEvent::removeMonitor(&mon) };
            }
            if let Some(mon) = self.global_monitor.borrow_mut().take() {
                unsafe { NSEvent::removeMonitor(&mon) };
            }
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
