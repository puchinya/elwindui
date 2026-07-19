//! `usvg::Tree` → `VectorImage` conversion entry point, plus geometry helpers shared by
//! `convert_path`/`convert_paint`/`convert_filter` (実装指示書§15).

use crate::convert_filter::convert_filter;
use crate::convert_paint::{convert_clip_path, convert_mask};
use crate::convert_path::convert_path_node;
use crate::error::{SvgError, SvgLimitKind};
use crate::loader::SvgLimits;
use elwindui_core::base::{AffineTransform, Point, Rect, Size};
use elwindui_core::graphics::{
    Color, Image, ImageFormat, ImageSampling, VectorGroup, VectorImage, VectorImageBuilder,
    VectorNode, VectorRasterNode,
};
use std::sync::Arc;

pub(crate) fn transform_from_usvg(t: usvg::Transform) -> AffineTransform {
    AffineTransform {
        m11: t.sx,
        m12: t.ky,
        m21: t.kx,
        m22: t.sy,
        dx: t.tx,
        dy: t.ty,
    }
}

pub(crate) fn point_from_usvg(p: tiny_skia_path::Point) -> Point {
    Point { x: p.x, y: p.y }
}

pub(crate) fn rect_from_nonzero(r: usvg::NonZeroRect) -> Rect {
    Rect {
        x: r.x(),
        y: r.y(),
        width: r.width(),
        height: r.height(),
    }
}

pub(crate) fn color_from_usvg(c: usvg::Color, opacity: f32) -> Color {
    let a = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::rgba(c.red, c.green, c.blue, a)
}

/// The transform a leaf node (`Path`/`Image`/flattened-`Text`-group) needs *relative to its
/// immediate containing group* to reproduce `leaf_abs` once composed with that group's own
/// (already-accumulated) absolute transform — `usvg` only exposes each leaf's fully-resolved
/// `abs_transform()`, not a per-leaf relative one, so this factors it back out via
/// `AffineTransform::invert` (実装指示書§15.4 group nesting; confirmed against `usvg`'s own
/// converter source that `Path::abs_transform() == parent.abs_transform` and
/// `Image::abs_transform() == parent.abs_transform.pre_concat(placement)`, i.e. every leaf kind
/// reduces to "parent's absolute transform composed with a leaf-local placement component" — this
/// helper recovers exactly that local component generically, without needing to special-case any
/// leaf kind).
pub(crate) fn leaf_relative_transform(
    parent_abs: usvg::Transform,
    leaf_abs: usvg::Transform,
) -> AffineTransform {
    let parent = transform_from_usvg(parent_abs);
    let leaf = transform_from_usvg(leaf_abs);
    match parent.invert() {
        Some(inv) => inv.concat(&leaf),
        None => AffineTransform::IDENTITY,
    }
}

pub(crate) fn image_format_from_kind(kind: &usvg::ImageKind) -> Option<ImageFormat> {
    match kind {
        usvg::ImageKind::JPEG(_) => Some(ImageFormat::Jpeg),
        usvg::ImageKind::PNG(_) => Some(ImageFormat::Png),
        usvg::ImageKind::GIF(_) => Some(ImageFormat::Gif),
        usvg::ImageKind::WEBP(_) => Some(ImageFormat::WebP),
        usvg::ImageKind::SVG(_) => None,
    }
}

pub(crate) fn image_sampling_from_usvg(mode: usvg::ImageRendering) -> ImageSampling {
    match mode {
        usvg::ImageRendering::Pixelated | usvg::ImageRendering::CrispEdges => {
            ImageSampling::Nearest
        }
        _ => ImageSampling::Linear,
    }
}

/// Node/path-command/group-depth/filter-primitive tallies accumulated while converting, checked
/// against [`SvgLimits`] once conversion finishes (実装指示書§13) — a single post-conversion check
/// is simpler and just as effective as gating every intermediate allocation, since a rejected tree
/// is discarded wholesale either way.
#[derive(Default)]
struct LimitCounters {
    nodes: usize,
    path_commands: usize,
    max_depth: usize,
    filter_primitives: usize,
}

pub fn convert_tree(tree: &usvg::Tree, limits: &SvgLimits) -> Result<VectorImage, SvgError> {
    let root = convert_group(tree.root());

    let mut counters = LimitCounters::default();
    tally_group(&root, 1, &mut counters);
    if counters.nodes > limits.max_nodes {
        return Err(SvgError::ResourceLimitExceeded {
            kind: SvgLimitKind::Nodes,
            actual: counters.nodes,
            limit: limits.max_nodes,
        });
    }
    if counters.path_commands > limits.max_path_commands {
        return Err(SvgError::ResourceLimitExceeded {
            kind: SvgLimitKind::PathCommands,
            actual: counters.path_commands,
            limit: limits.max_path_commands,
        });
    }
    if counters.max_depth > limits.max_group_depth {
        return Err(SvgError::ResourceLimitExceeded {
            kind: SvgLimitKind::GroupDepth,
            actual: counters.max_depth,
            limit: limits.max_group_depth,
        });
    }
    if counters.filter_primitives > limits.max_filter_primitives {
        return Err(SvgError::ResourceLimitExceeded {
            kind: SvgLimitKind::FilterPrimitives,
            actual: counters.filter_primitives,
            limit: limits.max_filter_primitives,
        });
    }

    let size = tree.size();
    let intrinsic_size = Size {
        width: size.width(),
        height: size.height(),
    };
    let view_box = Rect {
        x: 0.0,
        y: 0.0,
        width: size.width(),
        height: size.height(),
    };

    VectorImageBuilder::new(intrinsic_size, view_box)
        .map_err(|e| SvgError::InvalidGeometry {
            message: e.to_string().into(),
        })?
        .root(root)
        .finish()
        .map_err(|e| SvgError::InvalidGeometry {
            message: e.to_string().into(),
        })
}

fn tally_group(group: &VectorGroup, depth: usize, counters: &mut LimitCounters) {
    counters.max_depth = counters.max_depth.max(depth);
    for filter in group.filters.iter() {
        counters.filter_primitives += filter.primitives.len();
    }
    for node in group.children.iter() {
        counters.nodes += 1;
        match node {
            VectorNode::Group(child) => tally_group(child, depth + 1, counters),
            VectorNode::Path(path_node) => {
                counters.path_commands += path_node.path.commands().len();
            }
            VectorNode::RasterImage(_) => {}
        }
    }
}

pub(crate) fn convert_group(group: &usvg::Group) -> VectorGroup {
    let children: Vec<VectorNode> = group
        .children()
        .iter()
        .map(|node| convert_node(node, group.abs_transform()))
        .collect();

    let filters: Vec<_> = group.filters().iter().map(|f| convert_filter(f)).collect();

    VectorGroup {
        transform: transform_from_usvg(group.transform()),
        opacity: group.opacity().get(),
        blend_mode: super::convert_paint::blend_mode_from_usvg(group.blend_mode()),
        isolate: group.isolate(),
        clip_path: group.clip_path().map(|c| Arc::new(convert_clip_path(c))),
        mask: group.mask().map(|m| Arc::new(convert_mask(m))),
        filters: filters.into(),
        // Object/local-space bounds (not `abs_layer_bounding_box`'s canvas-space ones) — `layer_bounds`
        // composes with `transform` the same way every other local-space field on this type does.
        // `usvg` guarantees a valid (if degenerate, 0x0x1x1) box even for empty groups, so this is
        // never `None` for a group actually produced by `usvg` — kept `Option` on the elwindui-core
        // side only because a hand-built `VectorGroup` may not want to precompute one.
        layer_bounds: Some(rect_from_nonzero(group.layer_bounding_box())),
        children: children.into(),
    }
}

fn convert_node(node: &usvg::Node, parent_abs: usvg::Transform) -> VectorNode {
    match node {
        usvg::Node::Group(group) => VectorNode::Group(convert_group(group)),
        usvg::Node::Path(path) => VectorNode::Path(convert_path_node(path, parent_abs)),
        usvg::Node::Image(image) => convert_image_node(image, parent_abs),
        // `usvg` already resolves text into a `Group` of filled/stroked paths (glyph outlines);
        // converting that flattened group is indistinguishable from any other nested group, so
        // text needs no dedicated `VectorNode` variant of its own (実装指示書§10).
        usvg::Node::Text(text) => VectorNode::Group(convert_group(text.flattened())),
    }
}

fn convert_image_node(image: &usvg::Image, parent_abs: usvg::Transform) -> VectorNode {
    let transform = leaf_relative_transform(parent_abs, image.abs_transform());
    let size = image.size();
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: size.width(),
        height: size.height(),
    };
    match image.kind() {
        usvg::ImageKind::SVG(nested_tree) => {
            // Nested SVG stays a real subtree, never a raster fallback (実装指示書§11.2).
            // `nested_tree.root()`'s own `.transform()` is always identity (usvg never sets a
            // non-default transform directly on a `Tree::root`, only on synthetic children of
            // it — see this module's own `convert_tree` doc comment), so overwriting it with the
            // leaf-relative placement transform computed above is safe.
            let mut group = convert_group(nested_tree.root());
            group.transform = transform;
            VectorNode::Group(group)
        }
        kind @ (usvg::ImageKind::JPEG(bytes)
        | usvg::ImageKind::PNG(bytes)
        | usvg::ImageKind::GIF(bytes)
        | usvg::ImageKind::WEBP(bytes)) => {
            let format = image_format_from_kind(kind).unwrap_or(ImageFormat::Unknown);
            let elwindui_image = Image::from_encoded_with_format(bytes.as_slice().to_vec(), format);
            VectorNode::RasterImage(VectorRasterNode {
                image: elwindui_image,
                rect,
                transform,
                sampling: image_sampling_from_usvg(image.rendering_mode()),
                opacity: 1.0,
            })
        }
    }
}
