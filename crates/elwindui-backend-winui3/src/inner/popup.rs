//! Popup surface implementation for Windows WinUI 3.
//!
//! Hosts arbitrary `UIElement` subtrees inside a lightweight, light-dismissable XAML `Popup`.

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Popup;
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::host::TreeHostPanel;
use elwindui_core::ui::popup::{
    PopupDismissPolicy, PopupFocusPolicy, PopupHost, PopupRequest, PopupSurfaceHandle,
};
use elwindui_core::ui::{UIElementExt, unmount_subtree};
use std::cell::RefCell;
use std::rc::Rc;
use windows::core::Interface;

/// Internal WinUI 3 representation of a standalone popup surface.
pub(crate) struct InnerPopupSurface {
    popup: Popup,
    content_host: TreeHostPanel,
    // `RefCell<Option<..>>`, not a bare `Rc`: `close()` must release this surface's own strong
    // reference to the popup content root once teardown completes, not merely unmount it — see
    // AppKit's `InnerPopupSurface::content` (same shape) for the full rationale.
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    is_open: RefCell<bool>,
}

impl InnerPopupSurface {
    /// Creates and immediately displays a new popup surface if coordinate conversion succeeds.
    pub(crate) fn show(
        request: PopupRequest,
        owner_canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
    ) -> Option<Rc<Self>> {
        let local = TreeHostPanel::screen_logical_to_xaml_local(owner_canvas, request.position)?;
        let popup = Popup::new().ok()?;
        let content_host = TreeHostPanel::new();
        content_host.set_tree(Rc::clone(&request.content));

        let canvas = content_host.canvas();
        let fe: FrameworkElement = canvas.cast().ok()?;
        fe.SetWidth(request.size.width as f64).ok();
        fe.SetHeight(request.size.height as f64).ok();

        let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
            canvas.cast().ok()?;
        popup.SetChild(&uie).ok();

        popup.SetHorizontalOffset(local.x as f64).ok();
        popup.SetVerticalOffset(local.y as f64).ok();
        popup.SetShouldConstrainToRootBounds(false).ok();
        let is_light_dismiss = request.dismiss_policy == PopupDismissPolicy::LightDismiss;
        popup.SetIsLightDismissEnabled(is_light_dismiss).ok();
        popup.SetIsOpen(true).ok();

        let surface = Rc::new(Self {
            popup,
            content_host,
            content: RefCell::new(Some(Rc::clone(&request.content))),
            is_open: RefCell::new(true),
        });

        let weak_surface = Rc::downgrade(&surface);
        let _ = surface.popup.Closed(&windows::Foundation::EventHandler::new(move |_, _| {
            if let Some(s) = weak_surface.upgrade() {
                s.close();
            }
            Ok(())
        }));

        if request.focus_policy == PopupFocusPolicy::Root {
            let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
                surface.content_host.canvas().cast().expect("Canvas as UIElement");
            let _ = uie.Focus(crate::bindings::Microsoft::UI::Xaml::FocusState::Programmatic);
            surface.content_host.focus_element(&request.content);
        }

        Some(surface)
    }

    /// Closes and hides the popup surface.
    ///
    /// Teardown-before-detach: `unmount_subtree` (generic Component/UIElement lifecycle teardown —
    /// `on_unmount` hooks, subscription cancellation) runs before *any* native detach —
    /// `SetIsOpen(false)` (visibility) and `TreeHostPanel::clear_tree()` (host tree/native resource
    /// release) both run only after `unmount_subtree` has completed, so `on_unmount` observes an
    /// intact popup/tree/Environment, matching AppKit's ordering. Unlike AppKit, WinUI3's
    /// `clear_tree()` has no existing deferred-dispatch workaround, so everything here runs
    /// synchronously. `is_open` is marked closed *before* `unmount_subtree` runs, so a reentrant
    /// `close()` call from inside an `on_unmount` hook (or from `SetIsOpen(false)` synchronously
    /// re-raising `Popup.Closed`, below) is a no-op via the guard at the top of this method.
    /// `self.content.borrow_mut().take()` releases this surface's own strong reference to the
    /// popup content root once `unmount_subtree` has run on it — see `content`'s own field doc
    /// comment. `None` here can only mean an already-closed surface, which the `is_open` guard
    /// above already routes around.
    pub(crate) fn close(&self) {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
            if let Some(content) = self.content.borrow_mut().take() {
                unmount_subtree(&content);
            }
            self.popup.SetIsOpen(false).ok();
            self.content_host.clear_tree();
        }
    }
}

impl Drop for InnerPopupSurface {
    fn drop(&mut self) {
        self.close();
    }
}

/// Handle implementing [`PopupSurfaceHandle`] for programmatic dismissal.
///
/// Always wraps a live surface — unlike before this revision, `WinUI3PopupHost::show_popup` no
/// longer constructs a handle at all when `InnerPopupSurface::show` fails (it returns `None`
/// instead, per `PopupHost::show_popup`'s fallible contract), so there is no "dummy closed handle"
/// case to represent here anymore.
#[derive(Clone)]
pub struct WinUI3PopupHandle {
    surface: Rc<InnerPopupSurface>,
}

impl PopupSurfaceHandle for WinUI3PopupHandle {
    fn close(&self) {
        self.surface.close();
    }
}

/// Default [`PopupHost`] implementation for WinUI 3.
#[derive(Clone)]
pub struct WinUI3PopupHost {
    owner_canvas: crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
}

impl WinUI3PopupHost {
    /// Creates a new popup host associated with an owner canvas context.
    pub fn new(owner_canvas: crate::bindings::Microsoft::UI::Xaml::Controls::Canvas) -> Self {
        Self { owner_canvas }
    }
}

impl PopupHost for WinUI3PopupHost {
    fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
        // `InnerPopupSurface::show` can fail (coordinate conversion, `Popup::new()`, XAML setup) —
        // propagate that as `None` rather than a handle wrapping a nonexistent surface. The caller
        // (`ContextMenuService::open_custom_popup`/`open_custom_menu`) is responsible for unmounting
        // the already-built popup content in that case.
        let surface = InnerPopupSurface::show(request, &self.owner_canvas)?;
        Some(Rc::new(WinUI3PopupHandle { surface }))
    }
}
