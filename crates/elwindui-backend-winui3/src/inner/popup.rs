//! Popup surface implementation for Windows WinUI 3.
//!
//! Hosts arbitrary `UIElement` subtrees inside a lightweight, light-dismissable XAML `Popup`.

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Popup;
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::host::TreeHostPanel;
use elwindui_core::ui::popup::{
    PopupDismissPolicy, PopupFocusPolicy, PopupHost, PopupRequest, PopupSurfaceHandle,
};
use elwindui_core::ui::UIElementExt;
use std::cell::RefCell;
use std::rc::Rc;
use windows::core::Interface;

/// Internal WinUI 3 representation of a standalone popup surface.
pub(crate) struct InnerPopupSurface {
    popup: Popup,
    content_host: TreeHostPanel,
    is_open: RefCell<bool>,
}

impl InnerPopupSurface {
    /// Creates and immediately displays a new popup surface.
    pub(crate) fn show(
        request: PopupRequest,
        owner_canvas: Option<&crate::bindings::Microsoft::UI::Xaml::Controls::Canvas>,
    ) -> Rc<Self> {
        let popup = Popup::new().expect("Popup::new");
        let content_host = TreeHostPanel::new();
        content_host.set_tree(Rc::clone(&request.content));

        let canvas = content_host.canvas();
        let fe: FrameworkElement = canvas.cast().expect("Canvas as FrameworkElement");
        fe.SetWidth(request.size.width as f64).ok();
        fe.SetHeight(request.size.height as f64).ok();

        let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
            canvas.cast().expect("Canvas as UIElement");
        popup.SetChild(&uie).ok();

        let (offset_x, offset_y) = if let Some(owner) = owner_canvas {
            if let Some(local) = TreeHostPanel::screen_logical_to_xaml_local(owner, request.position) {
                (local.x as f64, local.y as f64)
            } else {
                (request.position.x as f64, request.position.y as f64)
            }
        } else {
            (request.position.x as f64, request.position.y as f64)
        };

        popup.SetHorizontalOffset(offset_x).ok();
        popup.SetVerticalOffset(offset_y).ok();
        popup.SetShouldConstrainToRootBounds(false).ok();
        let is_light_dismiss = request.dismiss_policy == PopupDismissPolicy::LightDismiss;
        popup.SetIsLightDismissEnabled(is_light_dismiss).ok();
        popup.SetIsOpen(true).ok();

        let surface = Rc::new(Self {
            popup,
            content_host,
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

        surface
    }

    /// Closes and hides the popup surface.
    pub(crate) fn close(&self) {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
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
#[derive(Default, Clone)]
pub struct WinUI3PopupHost {
    owner_canvas: Option<crate::bindings::Microsoft::UI::Xaml::Controls::Canvas>,
}

impl WinUI3PopupHost {
    /// Creates a new popup host associated with an owner canvas context.
    pub fn new(owner_canvas: crate::bindings::Microsoft::UI::Xaml::Controls::Canvas) -> Self {
        Self {
            owner_canvas: Some(owner_canvas),
        }
    }
}

impl PopupHost for WinUI3PopupHost {
    fn show_popup(&self, request: PopupRequest) -> Rc<dyn PopupSurfaceHandle> {
        let surface = InnerPopupSurface::show(request, self.owner_canvas.as_ref());
        Rc::new(WinUI3PopupHandle { surface })
    }
}
