//! Popup surface implementation for macOS AppKit.
//!
//! Hosts arbitrary `UIElement` subtrees inside a borderless, floating, transparent `NSWindow`
//! above normal content and native controls.

use crate::ffi::mtm;
use crate::host::TreeHostView;
use elwindui_core::ui::popup::{
    PopupDismissPolicy, PopupFocusPolicy, PopupHost, PopupRequest, PopupSurfaceHandle,
};
use objc2::rc::Retained;
use objc2::{MainThreadOnly, msg_send};
use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSEventType, NSFloatingWindowLevel,
    NSScreen, NSWindow, NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;

/// Internal AppKit representation of a standalone popup surface.
pub(crate) struct InnerPopupSurface {
    window: Retained<NSWindow>,
    content_host: Retained<TreeHostView>,
    is_open: RefCell<bool>,
    local_monitor: RefCell<Option<Retained<AnyObject>>>,
    global_monitor: RefCell<Option<Retained<AnyObject>>>,
}

impl InnerPopupSurface {
    /// Creates and immediately displays a new popup surface.
    pub(crate) fn show(
        request: PopupRequest,
        owner_window: Option<&NSWindow>,
    ) -> Rc<Self> {
        let m = mtm();
        let primary_screen_height = NSScreen::screens(m)
            .firstObject()
            .or_else(|| NSScreen::mainScreen(m))
            .or_else(|| owner_window.and_then(|w| w.screen()))
            .map(|s| s.frame().size.height)
            .or_else(|| owner_window.map(|w| w.frame().size.height))
            .unwrap_or(0.0);

        let appkit_y = primary_screen_height - (request.position.y as f64 + request.size.height as f64);
        let content_rect = NSRect::new(
            NSPoint::new(request.position.x as f64, appkit_y),
            NSSize::new(request.size.width as f64, request.size.height as f64),
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

        // Floating level ensures popup sits above controls
        window.setLevel(NSFloatingWindowLevel);
        window.setOpaque(false);
        window.setHasShadow(true);
        window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));

        let content_host = TreeHostView::new();
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        content_host.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(request.size.width as f64, request.size.height as f64),
        ));
        content_host.set_tree(Rc::clone(&request.content));
        window.setContentView(Some(&content_host));

        if let Some(owner) = owner_window {
            unsafe {
                owner.addChildWindow_ordered(&window, NSWindowOrderingMode::Above);
            }
        }

        let surface = Rc::new(Self {
            window,
            content_host,
            is_open: RefCell::new(true),
            local_monitor: RefCell::new(None),
            global_monitor: RefCell::new(None),
        });

        surface.window.makeKeyAndOrderFront(None);
        surface.window.display();

        if request.focus_policy == PopupFocusPolicy::Root {
            surface.window.makeFirstResponder(Some(&surface.content_host));
            surface.content_host.focus_element(&request.content);
        }

        if request.dismiss_policy == PopupDismissPolicy::LightDismiss {
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
        }

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
            if let Some(parent) = self.window.parentWindow() {
                parent.removeChildWindow(&self.window);
            }
            self.window.orderOut(None);

            let host_raw = Retained::into_raw(self.content_host.clone()) as usize;
            dispatch2::DispatchQueue::main().exec_async(move || {
                let ptr = host_raw as *mut TreeHostView;
                if let Some(host) = unsafe { Retained::from_raw(ptr) } {
                    host.clear_tree();
                }
            });
        }
    }
}

impl Drop for InnerPopupSurface {
    fn drop(&mut self) {
        self.close();
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
pub struct AppKitPopupHost {
    pub(crate) owner_window: Option<Retained<NSWindow>>,
}

impl AppKitPopupHost {
    /// Creates a popup host associated with an optional owner window.
    pub fn new(owner_window: Option<Retained<NSWindow>>) -> Self {
        Self { owner_window }
    }
}

impl PopupHost for AppKitPopupHost {
    fn show_popup(&self, request: PopupRequest) -> Rc<dyn PopupSurfaceHandle> {
        let surface = InnerPopupSurface::show(request, self.owner_window.as_deref());
        Rc::new(AppKitPopupHandle { surface })
    }
}
