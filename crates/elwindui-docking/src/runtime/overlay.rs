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
use crate::runtime::{accent_brush, themed_brush};
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
    GroupCross,
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
    /// Floating surfaces have no root-edge targets (the WinUI.Dock reference shows only the group
    /// compass there).
    root_targets_hidden: bool,
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
            let hidden_root = layout.root_targets_hidden
                && matches!(entry.role, OverlayElementRole::RootTarget(_));
            // Visibility is decided outside layout (`OverlayLayout::sync_visibility`): flipping it
            // here would invalidate measure mid-arrange and rerun the whole-tree pass.
            if let Some(rect) = overlay_element_rect(entry.role, final_size, layout.group_bounds)
                .filter(|_| !hidden_root)
            {
                entry.element.arrange(rect);
            } else {
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

impl OverlayLayout {
    /// Shows exactly the elements the next arrange will place: root targets unless hidden, and
    /// the group compass only while a group is hovered. Runs before layout, so the first overlay
    /// appearance costs one layout pass instead of a second one triggered from inside arrange.
    fn sync_visibility(&self) {
        for entry in &self.elements {
            let visible = match entry.role {
                OverlayElementRole::RootTarget(_) => !self.root_targets_hidden,
                OverlayElementRole::GroupTarget(_) | OverlayElementRole::GroupCross => {
                    self.group_bounds.is_some()
                }
            };
            let visibility = if visible {
                Visibility::Visible
            } else {
                Visibility::Collapsed
            };
            if entry.element.visibility() != visibility {
                entry.element.set_visibility(visibility);
            }
        }
    }
}

/// Surface-local rect of a drawn root-edge target. Drop resolution uses this same geometry, so a
/// root target resolves exactly where it is drawn.
pub(crate) fn root_target_rect(target: DockTarget, surface: Size) -> Option<Rect> {
    overlay_element_rect(OverlayElementRole::RootTarget(target), surface, None)
}

/// Surface-local rect of a drawn compass cell for a group arranged at `group_bounds`.
pub(crate) fn group_target_rect(target: DockTarget, group_bounds: Rect) -> Option<Rect> {
    overlay_element_rect(
        OverlayElementRole::GroupTarget(target),
        Size {
            width: 0.0,
            height: 0.0,
        },
        Some(group_bounds),
    )
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
        OverlayElementRole::GroupCross => {
            let group = group_bounds.filter(valid_rect)?;
            let origin_x = group.x + (group.width - COMPASS_SIZE) * 0.5;
            let origin_y = group.y + (group.height - COMPASS_SIZE) * 0.5;
            Some(Rect {
                x: origin_x,
                y: origin_y,
                width: COMPASS_SIZE,
                height: COMPASS_SIZE,
            })
        }
    }
}

impl InsertionMarkerLayer {
    fn set_rect(&self, rect: Option<Rect>) {
        let rect = rect.filter(valid_rect);
        // Pointer moves repeat the same state; skip it so a drag never re-invalidates layout.
        if self.marker_rect() == rect {
            return;
        }
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
        // A one-pixel outer frame plus four pixels of padding leaves a 26px glyph.
        glyph.set_width(26.0);
        glyph.set_height(26.0);
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

    let accent = accent_brush();
    let mut outline = PathBuilder::new();
    let mut detail = PathBuilder::new();
    let mut top_edge = PathBuilder::new();
    let mut fill = None;
    let mut detail_dash: Arc<[f32]> = Arc::from([]);
    match kind {
        TargetGlyphKind::Center => {
            outline.add_rounded_rect(
                Rect {
                    x: 0.5,
                    y: 0.5,
                    width: 25.0,
                    height: 25.0,
                },
                crate::core::base::CornerRadius::uniform(2.0),
            );
            top_edge.add_line(Point { x: 2.0, y: 1.5 }, Point { x: 24.0, y: 1.5 });
        }
        TargetGlyphKind::Split(target) => {
            outline.add_rounded_rect(
                Rect {
                    x: 0.5,
                    y: 0.5,
                    width: 25.0,
                    height: 25.0,
                },
                crate::core::base::CornerRadius::uniform(2.0),
            );
            top_edge.add_line(Point { x: 2.0, y: 1.5 }, Point { x: 24.0, y: 1.5 });
            match target {
                DockTarget::SplitLeft | DockTarget::SplitRight => {
                    detail.add_line(Point { x: 13.0, y: 3.0 }, Point { x: 13.0, y: 25.0 });
                }
                DockTarget::SplitTop | DockTarget::SplitBottom => {
                    detail.add_line(Point { x: 1.0, y: 13.0 }, Point { x: 25.0, y: 13.0 });
                }
                _ => {}
            }
            detail_dash = Arc::from([3.0, 1.0]);
        }
        TargetGlyphKind::Dock(target) => {
            let half = match target {
                DockTarget::DockLeft => Rect {
                    x: 0.5,
                    y: 0.5,
                    width: 11.5,
                    height: 25.0,
                },
                DockTarget::DockRight => Rect {
                    x: 14.0,
                    y: 0.5,
                    width: 11.5,
                    height: 25.0,
                },
                DockTarget::DockTop => Rect {
                    x: 0.5,
                    y: 0.5,
                    width: 25.0,
                    height: 11.5,
                },
                DockTarget::DockBottom => Rect {
                    x: 0.5,
                    y: 14.0,
                    width: 25.0,
                    height: 11.5,
                },
                _ => Rect {
                    x: 4.0,
                    y: 4.0,
                    width: 16.0,
                    height: 16.0,
                },
            };
            outline.add_rounded_rect(half, crate::core::base::CornerRadius::uniform(2.0));
            top_edge.add_line(
                Point {
                    x: half.x + 1.5,
                    y: half.y + 1.0,
                },
                Point {
                    x: half.x + half.width - 1.5,
                    y: half.y + 1.0,
                },
            );
            fill = Some(VectorFill {
                paint: VectorPaint::Brush(
                    themed_brush(BrushStyle::Secondary)
                        .unwrap_or_else(|| Brush::Solid(Color::rgb(112, 112, 112))),
                ),
                opacity: 1.0,
                rule: crate::core::graphics::FillRule::NonZero,
            });
            // The reference marks the docking side with a half document and a small square on the
            // opposite side (where the existing content goes), not an arrow.
            let marker = match target {
                DockTarget::DockLeft => Some(Rect {
                    x: 17.5,
                    y: 10.5,
                    width: 5.0,
                    height: 5.0,
                }),
                DockTarget::DockRight => Some(Rect {
                    x: 3.5,
                    y: 10.5,
                    width: 5.0,
                    height: 5.0,
                }),
                DockTarget::DockTop => Some(Rect {
                    x: 10.5,
                    y: 17.5,
                    width: 5.0,
                    height: 5.0,
                }),
                DockTarget::DockBottom => Some(Rect {
                    x: 10.5,
                    y: 3.5,
                    width: 5.0,
                    height: 5.0,
                }),
                _ => None,
            };
            if let Some(marker) = marker {
                detail.add_rect(marker);
            }
        }
    }

    let outline = outline.build().expect("static dock target path is valid");
    let detail = detail.build().expect("static dock target detail is valid");
    let top_edge = top_edge.build().expect("static document top edge is valid");
    let mut nodes = Vec::new();
    nodes.push(VectorNode::Path(VectorPathNode {
        path: outline,
        transform: AffineTransform::IDENTITY,
        fill,
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(accent.clone()),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.0,
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
            paint: VectorPaint::Brush(accent.clone()),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.0,
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
    nodes.push(VectorNode::Path(VectorPathNode {
        path: top_edge,
        transform: AffineTransform::IDENTITY,
        fill: None,
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(accent),
            opacity: 1.0,
            style: StrokeStyle {
                width: 3.0,
                ..StrokeStyle::default()
            },
        }),
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::GeometricPrecision,
        visibility: true,
    }));
    VectorImageBuilder::new(
        Size {
            width: 26.0,
            height: 26.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 26.0,
            height: 26.0,
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

/// Builds one rounded cross from a three-cell grid. Rounding the contour rather than
/// overlapping stroked rectangles avoids seams through the connected backing.
fn compass_backing() -> crate::core::graphics::VectorImage {
    use crate::core::graphics::VectorFill;
    let cell = (COMPASS_SIZE - 4.0) / 3.0;
    let low = cell + 0.5;
    let high = COMPASS_SIZE - low;
    let edge = COMPASS_SIZE - 0.5;
    let corners = [
        Point { x: low, y: 0.5 },
        Point { x: high, y: 0.5 },
        Point { x: high, y: low },
        Point { x: edge, y: low },
        Point { x: edge, y: high },
        Point { x: high, y: high },
        Point { x: high, y: edge },
        Point { x: low, y: edge },
        Point { x: low, y: high },
        Point { x: 0.5, y: high },
        Point { x: 0.5, y: low },
        Point { x: low, y: low },
    ];
    let toward = |corner: Point, neighbor: Point| {
        let dx = neighbor.x - corner.x;
        let dy = neighbor.y - corner.y;
        let distance = dx.hypot(dy);
        Point {
            x: corner.x + dx * 4.0 / distance,
            y: corner.y + dy * 4.0 / distance,
        }
    };
    let mut path = PathBuilder::new();
    for (index, corner) in corners.iter().copied().enumerate() {
        let before = toward(corner, corners[(index + corners.len() - 1) % corners.len()]);
        let after = toward(corner, corners[(index + 1) % corners.len()]);
        if index == 0 {
            path.move_to(before);
        } else {
            path.line_to(before);
        }
        path.quad_to(corner, after);
    }
    path.close();
    let node = VectorNode::Path(VectorPathNode {
        path: path.build().expect("rounded compass contour is valid"),
        transform: AffineTransform::IDENTITY,
        fill: Some(VectorFill {
            paint: VectorPaint::Brush(
                themed_brush(BrushStyle::Secondary)
                    .unwrap_or_else(|| Brush::Solid(Color::rgb(32, 32, 32))),
            ),
            opacity: 1.0,
            rule: crate::core::graphics::FillRule::NonZero,
        }),
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(
                themed_brush(BrushStyle::Separator)
                    .unwrap_or_else(|| Brush::Solid(Color::rgb(96, 96, 96))),
            ),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.0,
                ..StrokeStyle::default()
            },
        }),
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::GeometricPrecision,
        visibility: true,
    });
    VectorImageBuilder::new(
        Size {
            width: COMPASS_SIZE,
            height: COMPASS_SIZE,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: COMPASS_SIZE,
            height: COMPASS_SIZE,
        },
    )
    .expect("compass viewport is valid")
    .root(VectorGroup {
        children: Arc::from([node]),
        ..VectorGroup::default()
    })
    .finish()
    .expect("compass scene is valid")
}

/// The target chrome stays retained and non-hit-testable; its selected state is tracked for
/// diagnostics while the resolver-owned preview communicates the active destination.
pub(crate) struct DockTargetOverlay {
    layer: Rc<DockTargetOverlayLayer>,
    layout: Rc<RefCell<OverlayLayout>>,
    group_buttons: Vec<(DockTarget, Rc<DockTargetVisual>)>,
    root_buttons: Vec<(DockTarget, Rc<DockTargetVisual>)>,
    cross_background: Rc<IconSourceElement>,
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

        let cross_background = IconSourceElement::new();
        cross_background.set_width(COMPASS_SIZE);
        cross_background.set_height(COMPASS_SIZE);
        cross_background.set_hit_test_visible(false);
        cross_background.set_icon_source(Some(IconSource::Image(ImageSource::Vector(
            compass_backing(),
        ))));
        root.children().add(cross_background.clone());
        layout.borrow_mut().elements.push(OverlayElement {
            element: cross_background.clone(),
            role: OverlayElementRole::GroupCross,
        });

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
            cross_background,
            selected: Cell::new(None),
        }
    }

    /// Shows root targets and, when a group is hovered, its compass. `target` is the resolved
    /// target, if any; the compass and root targets stay visible while no cell is resolved.
    pub(crate) fn show(&self, target: Option<DockTarget>, group_bounds: Option<Rect>) {
        let group_bounds = group_bounds.filter(valid_rect);
        let changed = self.selected.get() != target
            || self.layout.borrow().group_bounds != group_bounds
            || self.layer.visibility() != Visibility::Visible;
        if !changed {
            return;
        }
        self.selected.set(target);
        self.layout.borrow_mut().group_bounds = group_bounds;
        self.layout.borrow().sync_visibility();
        // Visibility only flips when the overlay first appears; a moving drag changes arrangement
        // only, so it never pays for a whole-tree Measure pass.
        self.layer.set_visibility(Visibility::Visible);
        self.layer.invalidate_arrange();
    }

    /// Floating surfaces show only the group compass; their root-edge targets stay hidden and
    /// never resolve (`docking_spec.md`).
    pub(crate) fn set_root_targets_hidden(&self, hidden: bool) {
        let changed = self.layout.borrow().root_targets_hidden != hidden;
        if changed {
            self.layout.borrow_mut().root_targets_hidden = hidden;
            self.layout.borrow().sync_visibility();
            self.layer.invalidate_arrange();
        }
    }

    pub(crate) fn clear(&self) {
        self.selected.set(None);
        self.layout.borrow_mut().group_bounds = None;
        self.layer.set_visibility(Visibility::Collapsed);
    }

    pub(crate) fn refresh_theme(&self) {
        self.cross_background
            .set_icon_source(Some(IconSource::Image(ImageSource::Vector(
                compass_backing(),
            ))));
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

    #[cfg(test)]
    pub(crate) fn cross_background_for_test(&self) -> Rc<IconSourceElement> {
        self.cross_background.clone()
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
        if self.preview_rect() == rect {
            return;
        }
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
            rectangle.set_fill(Some(accent_brush()));
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
            rectangle.set_fill(Some(accent_brush()));
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
