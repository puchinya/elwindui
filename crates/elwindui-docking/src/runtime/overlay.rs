//! Dock target overlays and transient previews.

use crate::DockTarget;
use crate::core::base::{Rect, Size};
use crate::core::layout::{GridLength, HorizontalAlignment, VerticalAlignment, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    ControlExt, Grid, GridExt, LayoutExt, Rectangle, RectangleExt, ShapeExt, UIElementExt,
};
use crate::runtime::drag::ResolvedDockTarget;
use crate::runtime::metrics::{COMPASS_BUTTON_SIZE, COMPASS_GAP, COMPASS_SIZE};
use crate::runtime::themed_brush;
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

/// A discoverable compass shown on every surface while a dock drag is over that surface. The
/// buttons are deliberately non-hit-testable: the coordinator remains the sole authority for
/// geometry and the compass cannot steal the originating pointer capture.
pub(crate) struct DockTargetOverlay {
    visual: Rc<Grid>,
    buttons: Vec<(DockTarget, Rc<Rectangle>)>,
}

impl DockTargetOverlay {
    pub(crate) fn new() -> Self {
        let visual = Grid::new();
        visual.set_width(COMPASS_SIZE);
        visual.set_height(COMPASS_SIZE);
        visual.set_horizontal_alignment(HorizontalAlignment::Center);
        visual.set_vertical_alignment(VerticalAlignment::Center);
        visual.set_hit_test_visible(false);
        visual.set_rows(vec![
            GridLength::Fixed(COMPASS_BUTTON_SIZE),
            GridLength::Fixed(COMPASS_GAP),
            GridLength::Fixed(COMPASS_BUTTON_SIZE),
            GridLength::Fixed(COMPASS_GAP),
            GridLength::Fixed(COMPASS_BUTTON_SIZE),
        ]);
        visual.set_columns(vec![
            GridLength::Fixed(COMPASS_BUTTON_SIZE),
            GridLength::Fixed(COMPASS_GAP),
            GridLength::Fixed(COMPASS_BUTTON_SIZE),
            GridLength::Fixed(COMPASS_GAP),
            GridLength::Fixed(COMPASS_BUTTON_SIZE),
        ]);
        let placements = [
            (DockTarget::SplitTop, 0, 2),
            (DockTarget::SplitLeft, 2, 0),
            (DockTarget::Center, 2, 2),
            (DockTarget::SplitRight, 2, 4),
            (DockTarget::SplitBottom, 4, 2),
        ];
        let buttons = placements
            .into_iter()
            .map(|(target, row, column)| {
                let button = Rectangle::new();
                button.set_width(COMPASS_BUTTON_SIZE);
                button.set_height(COMPASS_BUTTON_SIZE);
                button.set_corner_radius(4.0);
                button.set_fill(themed_brush(BrushStyle::Tint));
                button.set_stroke(themed_brush(BrushStyle::Separator));
                button.set_stroke_width(1.0);
                button.set_attached("Grid", "row", row);
                button.set_attached("Grid", "column", column);
                visual.children().add(button.clone());
                (target, button)
            })
            .collect();
        visual.set_visibility(Visibility::Collapsed);
        Self { visual, buttons }
    }

    pub(crate) fn show(&self, target: DockTarget) {
        for (candidate, button) in &self.buttons {
            let selected = *candidate == target
                || matches!(
                    (candidate, target),
                    (DockTarget::SplitLeft, DockTarget::DockLeft)
                        | (DockTarget::SplitTop, DockTarget::DockTop)
                        | (DockTarget::SplitRight, DockTarget::DockRight)
                        | (DockTarget::SplitBottom, DockTarget::DockBottom)
                );
            button.set_fill(themed_brush(if selected {
                BrushStyle::Selection
            } else {
                BrushStyle::Tint
            }));
        }
        self.visual.set_visibility(Visibility::Visible);
    }

    pub(crate) fn clear(&self) {
        self.visual.set_visibility(Visibility::Collapsed);
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.visual.clone()
    }
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
                .set_fill(themed_brush(BrushStyle::Selection));
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
