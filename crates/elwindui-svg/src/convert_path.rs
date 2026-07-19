//! `usvg::Path` → `VectorPathNode` (実装指示書§15.3).

use crate::convert::{leaf_relative_transform, point_from_usvg};
use crate::convert_paint::{convert_fill, convert_stroke};
use elwindui_core::graphics::{
    Path, PathBuilder, VectorPaintOrder, VectorPathNode, VectorShapeRendering,
};

pub(crate) fn convert_path_node(path: &usvg::Path, parent_abs: usvg::Transform) -> VectorPathNode {
    VectorPathNode {
        path: path_from_usvg(path.data()),
        transform: leaf_relative_transform(parent_abs, path.abs_transform()),
        fill: path.fill().map(convert_fill),
        stroke: path.stroke().map(convert_stroke),
        paint_order: match path.paint_order() {
            usvg::PaintOrder::FillAndStroke => VectorPaintOrder::FillStroke,
            usvg::PaintOrder::StrokeAndFill => VectorPaintOrder::StrokeFill,
        },
        rendering: match path.rendering_mode() {
            usvg::ShapeRendering::OptimizeSpeed => VectorShapeRendering::OptimizeSpeed,
            usvg::ShapeRendering::CrispEdges => VectorShapeRendering::CrispEdges,
            usvg::ShapeRendering::GeometricPrecision => VectorShapeRendering::GeometricPrecision,
        },
        visibility: path.is_visible(),
    }
}

/// `usvg`'s own geometry guarantee (only absolute `MoveTo`/`LineTo`/`QuadTo`/`CurveTo`/
/// `ClosePath`, arcs already converted to cubics — see `usvg`'s own crate-level doc comment) means
/// this is a direct, lossless 1:1 segment mapping onto `PathBuilder`, never `ArcTo`.
fn path_from_usvg(data: &tiny_skia_path::Path) -> Path {
    let mut builder = PathBuilder::new();
    for segment in data.segments() {
        match segment {
            tiny_skia_path::PathSegment::MoveTo(p) => {
                builder.move_to(point_from_usvg(p));
            }
            tiny_skia_path::PathSegment::LineTo(p) => {
                builder.line_to(point_from_usvg(p));
            }
            tiny_skia_path::PathSegment::QuadTo(c, p) => {
                builder.quad_to(point_from_usvg(c), point_from_usvg(p));
            }
            tiny_skia_path::PathSegment::CubicTo(c1, c2, p) => {
                builder.cubic_to(point_from_usvg(c1), point_from_usvg(c2), point_from_usvg(p));
            }
            tiny_skia_path::PathSegment::Close => {
                builder.close();
            }
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| PathBuilder::new().build().expect("an empty path always builds"))
}
