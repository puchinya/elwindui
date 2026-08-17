//! Popup surface implementation for macOS AppKit.
//!
//! Hosts arbitrary `UIElement` subtrees inside a borderless, floating, transparent `NSWindow`
//! above normal content and native controls.

use crate::ffi::mtm;
use crate::host::TreeHostView;
use elwindui_core::ui::popup::{
    PopupDismissPolicy, PopupFocusPolicy, PopupHost, PopupRequest, PopupSurfaceHandle,
};
use elwindui_core::ui::{UIElementExt, unmount_subtree};
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
    // `RefCell<Option<..>>`, not a bare `Rc`: `close()` must be able to release this surface's own
    // strong reference to the popup content root once teardown completes, not merely unmount it —
    // otherwise a closed-but-not-yet-dropped `InnerPopupSurface` (reachable via `active_popup` until
    // the host replaces or drops it) would keep the entire already-unmounted popup subtree alive.
    // `close()`'s `.borrow_mut().take()` is this field's only consumer; every other read either
    // doesn't exist or would be redundant with the local `content_host` tree it was built from.
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
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
            content: RefCell::new(Some(Rc::clone(&request.content))),
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
    ///
    /// Teardown-before-detach: generic Component/UIElement lifecycle teardown
    /// (`unmount_subtree` — `on_unmount` hooks, subscription cancellation) runs synchronously
    /// here, before *any* native detach — event monitor removal is not itself a detach of the
    /// popup's window relationship/visibility/host tree, so it may stay ahead of `unmount_subtree`,
    /// but `removeChildWindow`/`orderOut` (window relationship + visibility) and
    /// `TreeHostView::clear_tree()` (host tree/native resource release) must both run only after
    /// `unmount_subtree` has completed, so `on_unmount` observes an intact window/tree/Environment.
    /// `clear_tree()` itself stays deferred to the next main-queue turn (PR #156): `close()` may be
    /// invoked reentrantly from inside a popup-internal event handler already on the call stack, and
    /// `clear_tree()` takes `TreeHostView`'s own `tree`/`render_tree` `RefCell`s mutably, which a
    /// live event-dispatch frame may still be borrowing. `unmount_subtree` does not touch those
    /// `TreeHostView`-owned cells (it only walks/mutates the `UIElementExt` tree's own
    /// `visual_collection`/lifecycle state), so running it synchronously ahead of the window/host
    /// detach is safe even when `close()` is reentrant — verified by `elwindui-core`'s
    /// `unmount_subtree_reentrant_from_within_own_event_dispatch_does_not_panic`.
    ///
    /// `self.content.borrow_mut().take()` both makes this exactly-once (a second `close()` call
    /// finds `None` and skips straight to the `is_open` guard above, which already short-circuits)
    /// and releases this surface's own strong reference to the popup content root once teardown
    /// completes — see `content`'s own field doc comment for why that release matters. The taken
    /// local `content` only needs to stay alive long enough for `unmount_subtree` to run; nothing
    /// after that point in `close()` touches the content root itself (the deferred `clear_tree()`
    /// below acts on `content_host`'s own independently-owned tree reference, not on this one).
    pub(crate) fn close(&self) {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
            if let Some(mon) = self.local_monitor.borrow_mut().take() {
                unsafe { NSEvent::removeMonitor(&mon) };
            }
            if let Some(mon) = self.global_monitor.borrow_mut().take() {
                unsafe { NSEvent::removeMonitor(&mon) };
            }

            if let Some(content) = self.content.borrow_mut().take() {
                unmount_subtree(&content);
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
    fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
        // `InnerPopupSurface::show` has no fallible step on this backend (unlike WinUI3's
        // coordinate-conversion/`Popup::new()` path) — always `Some`.
        let surface = InnerPopupSurface::show(request, self.owner_window.as_deref());
        Some(Rc::new(AppKitPopupHandle { surface }))
    }
}
