//! Popup surface implementation for Windows WinUI 3.
//!
//! Hosts arbitrary `UIElement` subtrees inside a lightweight, light-dismissable XAML `Popup`.

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Popup;
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::host::TreeHostPanel;
use elwindui_core::base::{Point, Size};
use elwindui_core::ui::{PopupHost, PopupSurfaceHandle, UIElementExt};
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
    pub(crate) fn show(content: Rc<dyn UIElementExt>, position: Point, size: Size) -> Rc<Self> {
        let popup = Popup::new().expect("Popup::new");
        let content_host = TreeHostPanel::new();
        content_host.set_tree(content);

        let canvas = content_host.canvas();
        let fe: FrameworkElement = canvas.cast().expect("Canvas as FrameworkElement");
        fe.SetWidth(size.width as f64).ok();
        fe.SetHeight(size.height as f64).ok();

        let uie: crate::bindings::Microsoft::UI::Xaml::UIElement =
            canvas.cast().expect("Canvas as UIElement");
        popup.SetChild(&uie).ok();
        popup.SetHorizontalOffset(position.x as f64).ok();
        popup.SetVerticalOffset(position.y as f64).ok();
        popup.SetIsLightDismissEnabled(true).ok();
        popup.SetIsOpen(true).ok();

        Rc::new(Self {
            popup,
            content_host,
            is_open: RefCell::new(true),
        })
    }

    /// Closes and hides the popup surface.
    pub(crate) fn close(&self) {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
            self.popup.SetIsOpen(false).ok();
        }
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
pub struct WinUI3PopupHost;

impl PopupHost for WinUI3PopupHost {
    fn show_popup(
        &self,
        content: Rc<dyn UIElementExt>,
        position: Point,
        size: Size,
    ) -> Rc<dyn PopupSurfaceHandle> {
        let surface = InnerPopupSurface::show(content, position, size);
        Rc::new(WinUI3PopupHandle { surface })
    }
}
