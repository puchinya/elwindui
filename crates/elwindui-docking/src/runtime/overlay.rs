//! Dock target overlays and transient previews.

#[cfg(test)]
use crate::DockTarget;
use crate::core::base::{Rect, Size};
use crate::core::graphics::Brush;
use crate::core::layout::Visibility;
use crate::core::ui::{ControlExt, Rectangle, ShapeExt, UIElementExt};
use crate::runtime::drag::ResolvedDockTarget;
use std::rc::Rc;

/// A retained, non-participating layout layer whose child is arranged in the surface's local
/// coordinate space. Grid rows/columns cannot express a pointer-selected arbitrary rectangle, so
/// the rectangle is positioned by this layer's arrange override instead.
#[elwindui::component(inherits Control)]
pub(crate) struct DropPreviewLayer {
    #[state(default = None)]
    preview_rect: Option<Rect>,
    template: template_view!(|_this: Self| { Rectangle {} }),
}

#[elwindui::component]
impl DropPreviewLayer {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        // The layer is an overlay. Its child never contributes to the surface's desired size.
        Size {
            width: 0.0,
            height: 0.0,
        }
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let Some(root) = self.__template_root() else {
            return final_size;
        };
        let Some(rect) = self.preview_rect().filter(valid_rect) else {
            root.set_visibility(Visibility::Collapsed);
            root.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            });
            return final_size;
        };
        root.set_visibility(Visibility::Visible);
        root.arrange(rect);
        final_size
    }
}

impl DropPreviewLayer {
    fn set_rect(&self, rect: Option<Rect>) {
        let rect = rect.filter(valid_rect);
        self.set_preview_rect(rect);
        if let Some(root) = self.__template_root() {
            root.set_visibility(if self.preview_rect().is_some() {
                Visibility::Visible
            } else {
                Visibility::Collapsed
            });
        }
        self.invalidate_arrange();
    }
}

pub(crate) struct DropPreview {
    target: Option<ResolvedDockTarget>,
    layer: Rc<DropPreviewLayer>,
}

impl DropPreview {
    pub(crate) fn new() -> Self {
        let layer = DropPreviewLayer::new();
        layer.apply_template();
        if let Some(root) = layer.__template_root() {
            root.set_visibility(Visibility::Collapsed);
            root.as_any()
                .downcast_ref::<Rectangle>()
                .expect("drop preview template root is a Rectangle")
                .set_fill(Some(Brush::from("#3388ff")));
        }
        Self {
            target: None,
            layer,
        }
    }

    pub(crate) fn show(&mut self, target: &ResolvedDockTarget) {
        self.target = Some(target.clone());
        self.layer.set_rect(Some(target.preview_rect));
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<DockTarget> {
        self.target.as_ref().map(|target| target.target)
    }

    #[cfg(test)]
    pub(crate) fn preview_rect(&self) -> Option<Rect> {
        self.target.as_ref().map(|target| target.preview_rect)
    }

    pub(crate) fn clear(&mut self) {
        self.target = None;
        self.layer.set_rect(None);
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.layer.clone()
    }

    #[cfg(test)]
    pub(crate) fn layer(&self) -> Rc<DropPreviewLayer> {
        self.layer.clone()
    }
}

fn valid_rect(rect: &Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}
