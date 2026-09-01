//! Dock target overlays and transient previews.

use crate::DockTarget;
use crate::core::graphics::Brush;
use crate::core::layout::Visibility;
use crate::core::ui::{Rectangle, ShapeExt, UIElementExt};
use std::rc::Rc;

pub(crate) struct DropPreview {
    target: Option<DockTarget>,
    rectangle: Rc<Rectangle>,
}

impl DropPreview {
    pub(crate) fn new() -> Self {
        let rectangle = Rectangle::new();
        rectangle.set_fill(Some(Brush::from("#3388ff")));
        rectangle.set_width(120.0);
        rectangle.set_height(80.0);
        rectangle.set_visibility(Visibility::Collapsed);
        Self {
            target: None,
            rectangle,
        }
    }

    pub(crate) fn set_target(&mut self, target: DockTarget) {
        self.target = Some(target);
        self.rectangle.set_visibility(Visibility::Visible);
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<DockTarget> {
        self.target
    }

    pub(crate) fn clear(&mut self) {
        self.target = None;
        self.rectangle.set_visibility(Visibility::Collapsed);
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.rectangle.clone()
    }
}
