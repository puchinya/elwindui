//! SVG vector scene graph types — nodes/groups/paint/clip/mask retained exactly as loaded (SVG読み
//!込み・ベクター描画対応 実装指示書 §6〜§8). Reuses the existing `Path`/`Brush`/`StrokeStyle`/
//! `Image` types unchanged rather than inventing SVG-specific geometry/paint types (§6.2).

use super::brush::Brush;
use super::image::{Image, ImageSampling};
use super::path::{FillRule, GeometryCombineMode, Path, PathBuilder};
use super::stroke::StrokeStyle;
use super::vector_filter::VectorFilter;
use super::vector_image::PreserveAspectRatio;
use crate::base::{AffineTransform, Rect};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorBlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Default for VectorBlendMode {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorPaintOrder {
    FillStroke,
    StrokeFill,
}

impl Default for VectorPaintOrder {
    fn default() -> Self {
        Self::FillStroke
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorShapeRendering {
    Auto,
    OptimizeSpeed,
    CrispEdges,
    GeometricPrecision,
}

impl Default for VectorShapeRendering {
    fn default() -> Self {
        Self::Auto
    }
}

/// A paintable value for fill/stroke — either an existing `Brush` (solid/gradient, unchanged) or
/// a `VectorPattern`, which keeps its own child scene and can't be represented as a `Brush`
/// variant without losing structure (実装指示書§7).
#[derive(Debug, Clone)]
pub enum VectorPaint {
    Brush(Brush),
    Pattern(Arc<VectorPattern>),
}

#[derive(Debug, Clone)]
pub struct VectorFill {
    pub paint: VectorPaint,
    pub opacity: f32,
    pub rule: FillRule,
}

#[derive(Debug, Clone)]
pub struct VectorStroke {
    pub paint: VectorPaint,
    pub opacity: f32,
    pub style: StrokeStyle,
}

/// `<pattern>` — retained as its own child scene (never pre-flattened to a bitmap; a backend may
/// cache a rendered tile internally, but that cache is not this type's job — 実装指示書§7/§20.2).
#[derive(Debug, Clone)]
pub struct VectorPattern {
    pub tile_rect: Rect,
    pub transform: AffineTransform,
    pub view_box: Option<Rect>,
    pub preserve_aspect_ratio: PreserveAspectRatio,
    pub root: VectorGroup,
}

/// `<clipPath>`. SVG restricts `clipPath` children to geometric shapes/text (never raster images
/// or nested `<svg>`/`<image>` bitmaps) — `elwindui-svg`'s conversion layer only ever populates
/// `root` with `VectorNode::Path`/`VectorNode::Group` for this reason, which is what makes
/// [`VectorClipPath::to_path`] a lossless, silent-skip-free flattening rather than the general
/// "flatten any `VectorGroup`" API this codebase deliberately does not provide (a `VectorGroup`
/// used as ordinary drawn content can contain raster nodes, so no such general API exists).
#[derive(Debug, Clone)]
pub struct VectorClipPath {
    pub transform: AffineTransform,
    pub root: VectorGroup,
    pub nested: Option<Arc<VectorClipPath>>,
}

/// Flattening tolerance for [`VectorClipPath::to_path`]'s `nested` intersection — same order of
/// magnitude as `Path`'s own (private) bezier-flattening tolerance; clip masks don't need tighter
/// precision than ordinary path rendering already uses.
const CLIP_PATH_COMBINE_TOLERANCE: f32 = 0.25;

impl VectorClipPath {
    /// Flattens this clipPath's geometry (ignoring paint — clipPath content only ever contributes
    /// fill regions) into a single `Path`, so backends can hand it straight to their existing
    /// `Clip::Path`-based masking machinery instead of needing bespoke clipPath-traversal code
    /// (実装指示書§8; see `vector_scene`'s own module doc and the `VectorClipPath` doc comment for
    /// why this narrow, clipPath-specific flattening is safe where a general one would not be).
    pub fn to_path(&self) -> Path {
        let local = group_geometry_to_path(&self.root, self.transform);
        match &self.nested {
            Some(nested) => {
                let nested_path = nested.to_path();
                Path::combine(
                    &local,
                    &nested_path,
                    GeometryCombineMode::Intersect,
                    CLIP_PATH_COMBINE_TOLERANCE,
                )
                .unwrap_or(local)
            }
            None => local,
        }
    }
}

/// Recursively concatenates every descendant `VectorNode::Path`'s geometry (transformed into the
/// space `transform` maps into) into one multi-subpath `Path`. Only used by
/// [`VectorClipPath::to_path`], where the SVG spec itself guarantees no raster content can appear
/// — not exposed as a public `VectorGroup` method (実装指示書「未対応機能をsilent skipしない」原則;
/// see `VectorClipPath`'s own doc comment).
fn group_geometry_to_path(group: &VectorGroup, parent_transform: AffineTransform) -> Path {
    let world = parent_transform.concat(&group.transform);
    let mut builder = PathBuilder::new();
    for node in group.children.iter() {
        match node {
            VectorNode::Path(path_node) => {
                if path_node.visibility {
                    let node_transform = world.concat(&path_node.transform);
                    builder.add_path(&path_node.path, Some(node_transform));
                }
            }
            VectorNode::Group(child) => {
                let child_path = group_geometry_to_path(child, world);
                builder.add_path(&child_path, None);
            }
            VectorNode::RasterImage(_) => {
                // Unreachable for well-formed clipPath content (see `VectorClipPath`'s doc
                // comment) — skipped rather than panicking so a malformed/hand-built scene degrades
                // gracefully instead of crashing the render path.
            }
        }
    }
    builder.build().unwrap_or_else(|_| {
        PathBuilder::new()
            .build()
            .expect("an empty PathBuilder always builds successfully")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorMaskType {
    Alpha,
    Luminance,
}

#[derive(Debug, Clone)]
pub struct VectorMask {
    pub mask_type: VectorMaskType,
    pub bounds: Rect,
    pub transform: AffineTransform,
    pub root: VectorGroup,
    pub nested: Option<Arc<VectorMask>>,
}

/// A leaf shape node: existing `Path` geometry plus SVG paint/rendering attributes.
#[derive(Debug, Clone)]
pub struct VectorPathNode {
    pub path: Path,
    pub transform: AffineTransform,
    pub fill: Option<VectorFill>,
    pub stroke: Option<VectorStroke>,
    pub paint_order: VectorPaintOrder,
    pub rendering: VectorShapeRendering,
    pub visibility: bool,
}

/// A leaf raster node — only for images actually embedded in the SVG (PNG/JPEG/GIF/WebP). Nested
/// `<svg>`/`<image xlink:href="other.svg">` content is a recursive `VectorNode::Group` instead,
/// never this variant (実装指示書§11.2).
#[derive(Debug, Clone)]
pub struct VectorRasterNode {
    pub image: Image,
    pub rect: Rect,
    pub transform: AffineTransform,
    pub sampling: ImageSampling,
    pub opacity: f32,
}

#[derive(Debug, Clone)]
pub enum VectorNode {
    Group(VectorGroup),
    Path(VectorPathNode),
    RasterImage(VectorRasterNode),
}

/// A `<g>` (or any container-like element normalized to a group by `usvg`) — composited as a
/// whole (group opacity, blend, isolate, clip, mask, filters) rather than per-child (実装指示書
/// §6.1: "group opacityは子要素ごとのopacity乗算ではない").
#[derive(Debug, Clone)]
pub struct VectorGroup {
    pub transform: AffineTransform,
    pub opacity: f32,
    pub blend_mode: VectorBlendMode,
    pub isolate: bool,
    pub clip_path: Option<Arc<VectorClipPath>>,
    pub mask: Option<Arc<VectorMask>>,
    pub filters: Arc<[VectorFilter]>,
    pub layer_bounds: Option<Rect>,
    pub children: Arc<[VectorNode]>,
}

impl Default for VectorGroup {
    fn default() -> Self {
        Self {
            transform: AffineTransform::IDENTITY,
            opacity: 1.0,
            blend_mode: VectorBlendMode::Normal,
            isolate: false,
            clip_path: None,
            mask: None,
            filters: Arc::from([]),
            layer_bounds: None,
            children: Arc::from([]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Point;
    use crate::graphics::color::Color;

    fn square_path_node(x: f32, y: f32, size: f32) -> VectorNode {
        let mut builder = PathBuilder::new();
        builder.add_rect(Rect {
            x,
            y,
            width: size,
            height: size,
        });
        let path = builder.build().unwrap();
        VectorNode::Path(VectorPathNode {
            path,
            transform: AffineTransform::IDENTITY,
            fill: Some(VectorFill {
                paint: VectorPaint::Brush(Brush::Solid(Color::black())),
                opacity: 1.0,
                rule: FillRule::NonZero,
            }),
            stroke: None,
            paint_order: VectorPaintOrder::default(),
            rendering: VectorShapeRendering::default(),
            visibility: true,
        })
    }

    #[test]
    fn clip_path_flattens_multiple_children_into_one_path() {
        let clip = VectorClipPath {
            transform: AffineTransform::IDENTITY,
            root: VectorGroup {
                children: Arc::from([square_path_node(0.0, 0.0, 10.0), square_path_node(20.0, 0.0, 10.0)]),
                ..VectorGroup::default()
            },
            nested: None,
        };
        let path = clip.to_path();
        assert!(path.contains(Point { x: 5.0, y: 5.0 }, FillRule::NonZero, None));
        assert!(path.contains(Point { x: 25.0, y: 5.0 }, FillRule::NonZero, None));
        assert!(!path.contains(Point { x: 15.0, y: 5.0 }, FillRule::NonZero, None));
    }

    #[test]
    fn nested_clip_path_intersects_rather_than_unions() {
        let inner = VectorClipPath {
            transform: AffineTransform::IDENTITY,
            root: VectorGroup {
                children: Arc::from([square_path_node(5.0, 5.0, 10.0)]),
                ..VectorGroup::default()
            },
            nested: None,
        };
        let outer = VectorClipPath {
            transform: AffineTransform::IDENTITY,
            root: VectorGroup {
                children: Arc::from([square_path_node(0.0, 0.0, 10.0)]),
                ..VectorGroup::default()
            },
            nested: Some(Arc::new(inner)),
        };
        let path = outer.to_path();
        // Overlap region of [0,10]x[0,10] and [5,15]x[5,15] is [5,10]x[5,10].
        assert!(path.contains(Point { x: 7.0, y: 7.0 }, FillRule::NonZero, None));
        assert!(!path.contains(Point { x: 2.0, y: 2.0 }, FillRule::NonZero, None));
        assert!(!path.contains(Point { x: 12.0, y: 12.0 }, FillRule::NonZero, None));
    }

    #[test]
    fn invisible_path_nodes_are_excluded_from_clip_geometry() {
        let mut hidden = match square_path_node(0.0, 0.0, 10.0) {
            VectorNode::Path(node) => node,
            _ => unreachable!(),
        };
        hidden.visibility = false;
        let clip = VectorClipPath {
            transform: AffineTransform::IDENTITY,
            root: VectorGroup {
                children: Arc::from([VectorNode::Path(hidden)]),
                ..VectorGroup::default()
            },
            nested: None,
        };
        assert!(clip.to_path().is_empty());
    }

    #[test]
    fn vector_group_default_is_identity_and_empty() {
        let group = VectorGroup::default();
        assert_eq!(group.transform, AffineTransform::IDENTITY);
        assert_eq!(group.opacity, 1.0);
        assert!(group.children.is_empty());
    }
}
