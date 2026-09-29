//! Dock target overlays and transient previews.

use crate::DockTarget;
use crate::core::base::{AffineTransform, Point, Rect, Size};
use crate::core::graphics::{
    Brush, Color, IconSource, ImageSource, LineCap, LineJoin, PathBuilder, StrokeStyle,
    VectorGroup, VectorImageBuilder, VectorNode, VectorPaint, VectorPaintOrder, VectorPathNode,
    VectorShapeRendering, VectorStroke,
};
use crate::core::layout::{GridLength, HorizontalAlignment, VerticalAlignment, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    ControlExt, Grid, GridExt, IconSourceElement, IconSourceElementExt, LayoutExt, Rectangle,
    RectangleExt, ShapeExt, UIElementExt,
};
use crate::runtime::drag::ResolvedDockTarget;
use crate::runtime::metrics::{
    COMPASS_BUTTON_SIZE, COMPASS_SIZE, ROOT_TARGET_EDGE_INSET, TAB_INSERTION_MARKER_WIDTH,
};
use crate::runtime::themed_brush;
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// A retained, non-participating layout layer whose child is arranged in the surface's local
/// coordinate space. Grid rows/columns cannot express a pointer-selected arbitrary rectangle, so
/// the rectangle is positioned by this layer's arrange override instead.
#[elwindui::component(inherits Control)]
pub(crate) struct DropPreviewLayer {
    #[state(default = None)]
    preview_rect: Option<Rect>,
    template: template_view!(|_this: Self| { Rectangle {} }),
}

/// One retained insertion marker. Its geometry is updated in place while a tab drag moves; the
/// marker never participates in hit testing or in the tab presenter's layout.
#[elwindui::component(inherits Control)]
pub(crate) struct InsertionMarkerLayer {
    #[state(default = None)]
    marker_rect: Option<Rect>,
    template: template_view!(|_this: Self| { Rectangle {} }),
}

#[derive(Clone, Copy)]
enum OverlayElementRole {
    GroupCrossHorizontal,
    GroupCrossVertical,
    GroupTarget(DockTarget),
    RootTarget(DockTarget),
}

struct OverlayElement {
    element: Rc<dyn UIElementExt>,
    role: OverlayElementRole,
}

#[derive(Default)]
pub struct OverlayLayout {
    elements: Vec<OverlayElement>,
    group_bounds: Option<Rect>,
}

/// Surface-sized retained layer. Its children are arranged from the coordinator's resolved
/// target facts so the group compass can follow an off-center or nested target group.
#[elwindui::component(inherits Control)]
pub(crate) struct DockTargetOverlayLayer {
    #[state(default = None)]
    layout_state: Option<Rc<RefCell<OverlayLayout>>>,
    template: template_view!(|_this: Self| { Grid {} }),
}

#[elwindui::component]
impl InsertionMarkerLayer {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
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
        let Some(rect) = self.marker_rect().filter(valid_rect) else {
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

#[elwindui::component]
impl DockTargetOverlayLayer {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
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
        let surface_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: final_size.width.max(0.0),
            height: final_size.height.max(0.0),
        };
        root.arrange(surface_rect);
        let Some(layout) = self.layout_state() else {
            return final_size;
        };
        let layout = layout.borrow();
        for entry in &layout.elements {
            if let Some(rect) = overlay_element_rect(entry.role, final_size, layout.group_bounds) {
                entry.element.set_visibility(Visibility::Visible);
                entry.element.arrange(rect);
            } else {
                entry.element.set_visibility(Visibility::Collapsed);
                entry.element.arrange(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                });
            }
        }
        final_size
    }
}

fn overlay_element_rect(
    role: OverlayElementRole,
    surface: Size,
    group_bounds: Option<Rect>,
) -> Option<Rect> {
    match role {
        OverlayElementRole::RootTarget(target) => {
            let inset = ROOT_TARGET_EDGE_INSET;
            let size = 36.0;
            let rect = match target {
                DockTarget::DockLeft => Rect {
                    x: inset,
                    y: (surface.height - size) * 0.5,
                    width: size,
                    height: size,
                },
                DockTarget::DockTop => Rect {
                    x: (surface.width - size) * 0.5,
                    y: inset,
                    width: size,
                    height: size,
                },
                DockTarget::DockRight => Rect {
                    x: surface.width - inset - size,
                    y: (surface.height - size) * 0.5,
                    width: size,
                    height: size,
                },
                DockTarget::DockBottom => Rect {
                    x: (surface.width - size) * 0.5,
                    y: surface.height - inset - size,
                    width: size,
                    height: size,
                },
                _ => return None,
            };
            valid_rect(&rect).then_some(rect)
        }
        OverlayElementRole::GroupTarget(target) => {
            let group = group_bounds.filter(valid_rect)?;
            let origin = Point {
                x: group.x + (group.width - COMPASS_SIZE) * 0.5,
                y: group.y + (group.height - COMPASS_SIZE) * 0.5,
            };
            let offset = match target {
                DockTarget::SplitTop => Point { x: 44.0, y: 4.0 },
                DockTarget::SplitLeft => Point { x: 4.0, y: 44.0 },
                DockTarget::Center => Point { x: 44.0, y: 44.0 },
                DockTarget::SplitRight => Point { x: 84.0, y: 44.0 },
                DockTarget::SplitBottom => Point { x: 44.0, y: 84.0 },
                _ => return None,
            };
            Some(Rect {
                x: origin.x + offset.x,
                y: origin.y + offset.y,
                width: COMPASS_BUTTON_SIZE,
                height: COMPASS_BUTTON_SIZE,
            })
        }
        OverlayElementRole::GroupCrossHorizontal | OverlayElementRole::GroupCrossVertical => {
            let group = group_bounds.filter(valid_rect)?;
            let origin_x = group.x + (group.width - COMPASS_SIZE) * 0.5;
            let origin_y = group.y + (group.height - COMPASS_SIZE) * 0.5;
            Some(
                if matches!(role, OverlayElementRole::GroupCrossHorizontal) {
                    Rect {
                        x: origin_x,
                        y: origin_y + 44.0,
                        width: COMPASS_SIZE,
                        height: COMPASS_BUTTON_SIZE,
                    }
                } else {
                    Rect {
                        x: origin_x + 44.0,
                        y: origin_y,
                        width: COMPASS_BUTTON_SIZE,
                        height: COMPASS_SIZE,
                    }
                },
            )
        }
    }
}

impl InsertionMarkerLayer {
    fn set_rect(&self, rect: Option<Rect>) {
        let rect = rect.filter(valid_rect);
        self.set_marker_rect(rect);
        if let Some(root) = self.__template_root() {
            root.set_visibility(if self.marker_rect().is_some() {
                Visibility::Visible
            } else {
                Visibility::Collapsed
            });
        }
        self.invalidate_arrange();
    }
}

pub(crate) struct InsertionMarker {
    layer: Rc<InsertionMarkerLayer>,
}

impl InsertionMarker {
    pub(crate) fn new() -> Self {
        let layer = InsertionMarkerLayer::new();
        layer.set_hit_test_visible(false);
        layer.apply_template();
        if let Some(root) = layer.__template_root() {
            root.set_visibility(Visibility::Collapsed);
            let marker = root
                .as_any()
                .downcast_ref::<Rectangle>()
                .expect("insertion marker template root is a Rectangle");
            marker.set_width(TAB_INSERTION_MARKER_WIDTH);
            marker.set_fill(themed_brush(BrushStyle::Tint));
        }
        Self { layer }
    }

    pub(crate) fn show(&self, rect: Option<Rect>) {
        self.layer.set_rect(rect);
    }

    pub(crate) fn clear(&self) {
        self.layer.set_rect(None);
    }

    #[cfg(test)]
    pub(crate) fn marker_rect_for_test(&self) -> Option<Rect> {
        self.layer.marker_rect()
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.layer.clone()
    }

    pub(crate) fn refresh_theme(&self) {
        if let Some(root) = self.layer.__template_root() {
            root.as_any()
                .downcast_ref::<Rectangle>()
                .expect("insertion marker template root is a Rectangle")
                .set_fill(themed_brush(BrushStyle::Tint));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetGlyphKind {
    Center,
    Split(DockTarget),
    Dock(DockTarget),
}

struct DockTargetVisual {
    element: Rc<Grid>,
    frame: Rc<Rectangle>,
    glyph: Rc<IconSourceElement>,
    kind: TargetGlyphKind,
}

impl DockTargetVisual {
    fn new(target: DockTarget) -> Self {
        let element = Grid::new();
        element.set_width(COMPASS_BUTTON_SIZE);
        element.set_height(COMPASS_BUTTON_SIZE);
        element.set_hit_test_visible(false);
        let frame = Rectangle::new();
        frame.set_width(COMPASS_BUTTON_SIZE);
        frame.set_height(COMPASS_BUTTON_SIZE);
        frame.set_corner_radius(4.0);
        frame.set_stroke_width(1.0);
        frame.set_hit_test_visible(false);
        let glyph = IconSourceElement::new();
        glyph.set_width(24.0);
        glyph.set_height(24.0);
        glyph.set_horizontal_alignment(HorizontalAlignment::Center);
        glyph.set_vertical_alignment(VerticalAlignment::Center);
        glyph.set_hit_test_visible(false);
        element.children().add(frame.clone());
        element.children().add(glyph.clone());
        let kind = match target {
            DockTarget::Center => TargetGlyphKind::Center,
            DockTarget::SplitTop
            | DockTarget::SplitLeft
            | DockTarget::SplitRight
            | DockTarget::SplitBottom => TargetGlyphKind::Split(target),
            _ => TargetGlyphKind::Dock(target),
        };
        let visual = Self {
            element,
            frame,
            glyph,
            kind,
        };
        visual.refresh_theme();
        visual
    }

    fn refresh_theme(&self) {
        self.frame.set_fill(themed_brush(BrushStyle::Secondary));
        self.frame.set_stroke(themed_brush(BrushStyle::Separator));
        self.glyph
            .set_icon_source(Some(IconSource::Image(ImageSource::Vector(target_glyph(
                self.kind,
            )))));
    }
}

fn target_glyph(kind: TargetGlyphKind) -> crate::core::graphics::VectorImage {
    use crate::core::graphics::VectorFill;
    use crate::core::theme::BrushStyle;

    let accent =
        themed_brush(BrushStyle::Primary).unwrap_or_else(|| Brush::Solid(Color::rgb(72, 144, 232)));
    let mut outline = PathBuilder::new();
    let mut detail = PathBuilder::new();
    let mut fill = None;
    let mut detail_dash: Arc<[f32]> = Arc::from([]);
    match kind {
        TargetGlyphKind::Center => {
            outline.add_rect(Rect {
                x: 4.0,
                y: 3.0,
                width: 16.0,
                height: 18.0,
            });
            detail.add_line(Point { x: 4.0, y: 7.0 }, Point { x: 20.0, y: 7.0 });
        }
        TargetGlyphKind::Split(target) => {
            outline.add_rect(Rect {
                x: 4.0,
                y: 3.0,
                width: 16.0,
                height: 18.0,
            });
            detail.add_line(Point { x: 4.0, y: 7.0 }, Point { x: 20.0, y: 7.0 });
            match target {
                DockTarget::SplitLeft | DockTarget::SplitRight => {
                    detail.add_line(Point { x: 12.0, y: 4.0 }, Point { x: 12.0, y: 20.0 });
                }
                DockTarget::SplitTop | DockTarget::SplitBottom => {
                    detail.add_line(Point { x: 5.0, y: 12.0 }, Point { x: 19.0, y: 12.0 });
                }
                _ => {}
            }
            detail_dash = Arc::from([2.5, 2.0]);
        }
        TargetGlyphKind::Dock(target) => {
            let half = match target {
                DockTarget::DockLeft => Rect {
                    x: 4.0,
                    y: 6.0,
                    width: 7.0,
                    height: 12.0,
                },
                DockTarget::DockRight => Rect {
                    x: 13.0,
                    y: 6.0,
                    width: 7.0,
                    height: 12.0,
                },
                DockTarget::DockTop => Rect {
                    x: 6.0,
                    y: 4.0,
                    width: 12.0,
                    height: 7.0,
                },
                DockTarget::DockBottom => Rect {
                    x: 6.0,
                    y: 13.0,
                    width: 12.0,
                    height: 7.0,
                },
                _ => Rect {
                    x: 4.0,
                    y: 4.0,
                    width: 16.0,
                    height: 16.0,
                },
            };
            outline.add_rect(half);
            fill = Some(VectorFill {
                paint: VectorPaint::Brush(
                    themed_brush(BrushStyle::Tertiary)
                        .unwrap_or_else(|| Brush::Solid(Color::rgb(112, 112, 112))),
                ),
                opacity: 1.0,
                rule: crate::core::graphics::FillRule::NonZero,
            });
            match target {
                DockTarget::DockLeft => {
                    detail.add_line(Point { x: 12.0, y: 12.0 }, Point { x: 20.0, y: 12.0 });
                    detail.add_line(Point { x: 16.0, y: 8.0 }, Point { x: 20.0, y: 12.0 });
                    detail.add_line(Point { x: 16.0, y: 16.0 }, Point { x: 20.0, y: 12.0 });
                }
                DockTarget::DockRight => {
                    detail.add_line(Point { x: 12.0, y: 12.0 }, Point { x: 4.0, y: 12.0 });
                    detail.add_line(Point { x: 8.0, y: 8.0 }, Point { x: 4.0, y: 12.0 });
                    detail.add_line(Point { x: 8.0, y: 16.0 }, Point { x: 4.0, y: 12.0 });
                }
                DockTarget::DockTop => {
                    detail.add_line(Point { x: 12.0, y: 12.0 }, Point { x: 12.0, y: 20.0 });
                    detail.add_line(Point { x: 8.0, y: 16.0 }, Point { x: 12.0, y: 20.0 });
                    detail.add_line(Point { x: 16.0, y: 16.0 }, Point { x: 12.0, y: 20.0 });
                }
                DockTarget::DockBottom => {
                    detail.add_line(Point { x: 12.0, y: 12.0 }, Point { x: 12.0, y: 4.0 });
                    detail.add_line(Point { x: 8.0, y: 8.0 }, Point { x: 12.0, y: 4.0 });
                    detail.add_line(Point { x: 16.0, y: 8.0 }, Point { x: 12.0, y: 4.0 });
                }
                _ => {}
            }
        }
    }

    let outline = outline.build().expect("static dock target path is valid");
    let detail = detail.build().expect("static dock target detail is valid");
    let mut nodes = Vec::new();
    nodes.push(VectorNode::Path(VectorPathNode {
        path: outline,
        transform: AffineTransform::IDENTITY,
        fill,
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(accent.clone()),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.5,
                start_cap: LineCap::Round,
                end_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..StrokeStyle::default()
            },
        }),
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::GeometricPrecision,
        visibility: true,
    }));
    nodes.push(VectorNode::Path(VectorPathNode {
        path: detail,
        transform: AffineTransform::IDENTITY,
        fill: None,
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(accent),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.5,
                start_cap: LineCap::Round,
                end_cap: LineCap::Round,
                dash_cap: LineCap::Round,
                line_join: LineJoin::Round,
                dash_pattern: detail_dash,
                ..StrokeStyle::default()
            },
        }),
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::GeometricPrecision,
        visibility: true,
    }));
    VectorImageBuilder::new(
        Size {
            width: 24.0,
            height: 24.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        },
    )
    .expect("target glyph viewport is valid")
    .root(VectorGroup {
        children: Arc::from(nodes),
        ..VectorGroup::default()
    })
    .finish()
    .expect("target glyph scene is valid")
}

/// The target chrome stays retained and non-hit-testable; its selected state is tracked for
/// diagnostics while the resolver-owned preview communicates the active destination.
pub(crate) struct DockTargetOverlay {
    layer: Rc<DockTargetOverlayLayer>,
    layout: Rc<RefCell<OverlayLayout>>,
    group_buttons: Vec<(DockTarget, Rc<DockTargetVisual>)>,
    root_buttons: Vec<(DockTarget, Rc<DockTargetVisual>)>,
    cross_backgrounds: [Rc<Rectangle>; 2],
    selected: Cell<Option<DockTarget>>,
}

impl DockTargetOverlay {
    pub(crate) fn new() -> Self {
        let layer = DockTargetOverlayLayer::new();
        layer.set_hit_test_visible(false);
        layer.apply_template();
        let root_node = layer
            .__template_root()
            .expect("dock target overlay layer template should be applied");
        let root = root_node
            .as_any()
            .downcast_ref::<Grid>()
            .expect("dock target overlay template root is a Grid");
        root.set_rows(vec![GridLength::Star(1.0)]);
        root.set_columns(vec![GridLength::Star(1.0)]);
        let layout = Rc::new(RefCell::new(OverlayLayout::default()));
        layer.set_layout_state(Some(layout.clone()));

        let horizontal = Rectangle::new();
        horizontal.set_width(COMPASS_SIZE);
        horizontal.set_height(COMPASS_BUTTON_SIZE);
        horizontal.set_corner_radius(4.0);
        horizontal.set_stroke_width(1.0);
        horizontal.set_hit_test_visible(false);
        let vertical = Rectangle::new();
        vertical.set_width(COMPASS_BUTTON_SIZE);
        vertical.set_height(COMPASS_SIZE);
        vertical.set_corner_radius(4.0);
        vertical.set_stroke_width(1.0);
        vertical.set_hit_test_visible(false);
        root.children().add(horizontal.clone());
        root.children().add(vertical.clone());
        layout.borrow_mut().elements.extend([
            OverlayElement {
                element: horizontal.clone(),
                role: OverlayElementRole::GroupCrossHorizontal,
            },
            OverlayElement {
                element: vertical.clone(),
                role: OverlayElementRole::GroupCrossVertical,
            },
        ]);

        let group_buttons = [
            DockTarget::SplitTop,
            DockTarget::SplitLeft,
            DockTarget::Center,
            DockTarget::SplitRight,
            DockTarget::SplitBottom,
        ]
        .into_iter()
        .map(|target| {
            let visual = Rc::new(DockTargetVisual::new(target));
            let element: Rc<dyn UIElementExt> = visual.element.clone();
            root.children().add(visual.element.clone());
            layout.borrow_mut().elements.push(OverlayElement {
                element,
                role: OverlayElementRole::GroupTarget(target),
            });
            (target, visual)
        })
        .collect::<Vec<_>>();
        let root_buttons = [
            DockTarget::DockTop,
            DockTarget::DockLeft,
            DockTarget::DockRight,
            DockTarget::DockBottom,
        ]
        .into_iter()
        .map(|target| {
            let visual = Rc::new(DockTargetVisual::new(target));
            let element: Rc<dyn UIElementExt> = visual.element.clone();
            root.children().add(visual.element.clone());
            layout.borrow_mut().elements.push(OverlayElement {
                element,
                role: OverlayElementRole::RootTarget(target),
            });
            (target, visual)
        })
        .collect::<Vec<_>>();
        layer.set_visibility(Visibility::Collapsed);
        Self {
            layer,
            layout,
            group_buttons,
            root_buttons,
            cross_backgrounds: [horizontal, vertical],
            selected: Cell::new(None),
        }
    }

    pub(crate) fn show(&self, target: DockTarget, group_bounds: Option<Rect>) {
        self.selected.set(Some(target));
        self.layout.borrow_mut().group_bounds = group_bounds.filter(valid_rect);
        self.layer.set_visibility(Visibility::Visible);
        self.layer.invalidate_arrange();
    }

    pub(crate) fn clear(&self) {
        self.selected.set(None);
        self.layout.borrow_mut().group_bounds = None;
        self.layer.set_visibility(Visibility::Collapsed);
    }

    pub(crate) fn refresh_theme(&self) {
        for background in &self.cross_backgrounds {
            background.set_fill(themed_brush(BrushStyle::Secondary));
            background.set_stroke(themed_brush(BrushStyle::Separator));
        }
        for (_, visual) in self.group_buttons.iter().chain(self.root_buttons.iter()) {
            visual.refresh_theme();
        }
    }

    pub(crate) fn visual(&self) -> Rc<dyn UIElementExt> {
        self.layer.clone()
    }

    #[cfg(test)]
    pub(crate) fn button_counts(&self) -> (usize, usize) {
        (self.group_buttons.len(), self.root_buttons.len())
    }

    #[cfg(test)]
    pub(crate) fn selected_target(&self) -> Option<DockTarget> {
        self.selected.get()
    }

    #[cfg(test)]
    pub(crate) fn button_rects(&self) -> Vec<(DockTarget, Option<Rect>)> {
        self.group_buttons
            .iter()
            .chain(self.root_buttons.iter())
            .map(|(target, visual)| {
                (
                    *target,
                    visual
                        .element
                        .arranged_offset()
                        .zip(
                            visual
                                .element
                                .arranged_width()
                                .zip(visual.element.arranged_height()),
                        )
                        .map(|(offset, (width, height))| Rect {
                            x: offset.x,
                            y: offset.y,
                            width,
                            height,
                        }),
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn target_glyph_kinds(&self) -> Vec<(DockTarget, TargetGlyphKind)> {
        self.group_buttons
            .iter()
            .chain(self.root_buttons.iter())
            .map(|(target, visual)| (*target, visual.kind))
            .collect()
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
        layer.set_hit_test_visible(false);
        layer.apply_template();
        if let Some(root) = layer.__template_root() {
            root.set_visibility(Visibility::Collapsed);
            let rectangle = root
                .as_any()
                .downcast_ref::<Rectangle>()
                .expect("drop preview template root is a Rectangle");
            rectangle.set_fill(themed_brush(BrushStyle::Primary));
            rectangle.set_stroke(themed_brush(BrushStyle::Separator));
            rectangle.set_stroke_width(4.0);
            rectangle.set_corner_radius(4.0);
            rectangle.set_opacity(0.4);
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

    pub(crate) fn refresh_theme(&self) {
        if let Some(root) = self.layer.__template_root() {
            let rectangle = root
                .as_any()
                .downcast_ref::<Rectangle>()
                .expect("drop preview template root is a Rectangle");
            rectangle.set_fill(themed_brush(BrushStyle::Primary));
            rectangle.set_stroke(themed_brush(BrushStyle::Separator));
        }
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
