//! SVG filter graph types (`<filter>`/`fe*` primitives) — backend-independent data only, no
//! execution. `elwindui-svg` converts `usvg`'s filter module into these types once at load time
//! (SVG読み込み・ベクター描画対応 実装指示書 §9); each backend renderer interprets the graph at
//! paint time (native effect API where available, a shared CPU executor otherwise, or an explicit
//! `VectorRenderDiagnostic::UnsupportedFilterPrimitive` — never a silent skip).

use super::color::Color;
use super::vector_scene::{VectorBlendMode, VectorGroup};
use crate::base::Rect;
use std::sync::Arc;

/// Identifies one filter primitive's output within its own `VectorFilter.primitives` list — the
/// primitive at that index. `elwindui-svg` resolves `usvg`'s named `result="..."` references to
/// indices at conversion time, so this type stays a plain index rather than carrying string names
/// into the render path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VectorFilterResultId(pub u32);

/// What a filter primitive reads as input — either one of the SVG-defined implicit sources or a
/// prior primitive's output (`Result`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VectorFilterInput {
    SourceGraphic,
    SourceAlpha,
    BackgroundImage,
    BackgroundAlpha,
    FillPaint,
    StrokePaint,
    Result(VectorFilterResultId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorColorInterpolation {
    SRgb,
    LinearRgb,
}

/// SVG's own default for `color-interpolation-filters` (distinct from the sRGB default used
/// everywhere else in the graphics API).
impl Default for VectorColorInterpolation {
    fn default() -> Self {
        Self::LinearRgb
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorEdgeMode {
    Duplicate,
    Wrap,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorColorChannel {
    R,
    G,
    B,
    A,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorBlendFilter {
    pub input1: VectorFilterInput,
    pub input2: VectorFilterInput,
    pub mode: VectorBlendMode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VectorColorMatrixKind {
    Matrix(Arc<[f32; 20]>),
    Saturate(f32),
    HueRotate(f32),
    LuminanceToAlpha,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorColorMatrixFilter {
    pub input: VectorFilterInput,
    pub kind: VectorColorMatrixKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VectorTransferFunction {
    Identity,
    Table(Arc<[f32]>),
    Discrete(Arc<[f32]>),
    Linear { slope: f32, intercept: f32 },
    Gamma { amplitude: f32, exponent: f32, offset: f32 },
}

impl Default for VectorTransferFunction {
    fn default() -> Self {
        Self::Identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorComponentTransferFilter {
    pub input: VectorFilterInput,
    pub red: VectorTransferFunction,
    pub green: VectorTransferFunction,
    pub blue: VectorTransferFunction,
    pub alpha: VectorTransferFunction,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VectorCompositeOperator {
    Over,
    In,
    Out,
    Atop,
    Xor,
    Arithmetic { k1: f32, k2: f32, k3: f32, k4: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorCompositeFilter {
    pub input1: VectorFilterInput,
    pub input2: VectorFilterInput,
    pub operator: VectorCompositeOperator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorConvolveMatrixFilter {
    pub input: VectorFilterInput,
    pub order_x: u32,
    pub order_y: u32,
    pub kernel: Arc<[f32]>,
    pub divisor: f32,
    pub bias: f32,
    pub target_x: i32,
    pub target_y: i32,
    pub edge_mode: VectorEdgeMode,
    pub preserve_alpha: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VectorLightSource {
    Distant {
        azimuth: f32,
        elevation: f32,
    },
    Point {
        x: f32,
        y: f32,
        z: f32,
    },
    Spot {
        x: f32,
        y: f32,
        z: f32,
        points_at_x: f32,
        points_at_y: f32,
        points_at_z: f32,
        specular_exponent: f32,
        limiting_cone_angle: Option<f32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorDiffuseLightingFilter {
    pub input: VectorFilterInput,
    pub surface_scale: f32,
    pub diffuse_constant: f32,
    pub lighting_color: Color,
    pub light: VectorLightSource,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorDisplacementMapFilter {
    pub input1: VectorFilterInput,
    pub input2: VectorFilterInput,
    pub scale: f32,
    pub x_channel: VectorColorChannel,
    pub y_channel: VectorColorChannel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorDropShadowFilter {
    pub input: VectorFilterInput,
    pub dx: f32,
    pub dy: f32,
    pub std_dev_x: f32,
    pub std_dev_y: f32,
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorFloodFilter {
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorGaussianBlurFilter {
    pub input: VectorFilterInput,
    pub std_dev_x: f32,
    pub std_dev_y: f32,
}

/// `feImage` — `usvg` already resolves its `href` (external raster, nested SVG fragment, or an
/// in-document element reference) into a renderable `Group`, so this stays a `VectorGroup`
/// exactly like `usvg::filter::Image`, never flattened to a bitmap (実装指示書§1.1/§11: SVG全体を
/// ラスター化しない、nested SVGをbitmap化しない、の対象に`feImage`も含む).
#[derive(Debug, Clone)]
pub struct VectorFilterImage {
    pub root: VectorGroup,
}

#[derive(Debug, Clone)]
pub struct VectorMergeFilter {
    pub inputs: Arc<[VectorFilterInput]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorMorphologyOperator {
    Erode,
    Dilate,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorMorphologyFilter {
    pub input: VectorFilterInput,
    pub operator: VectorMorphologyOperator,
    pub radius_x: f32,
    pub radius_y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorOffsetFilter {
    pub input: VectorFilterInput,
    pub dx: f32,
    pub dy: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorSpecularLightingFilter {
    pub input: VectorFilterInput,
    pub surface_scale: f32,
    pub specular_constant: f32,
    pub specular_exponent: f32,
    pub lighting_color: Color,
    pub light: VectorLightSource,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorTileFilter {
    pub input: VectorFilterInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorTurbulenceKind {
    FractalNoise,
    Turbulence,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorTurbulenceFilter {
    pub base_frequency_x: f32,
    pub base_frequency_y: f32,
    pub num_octaves: u32,
    pub seed: i32,
    pub stitch_tiles: bool,
    pub kind: VectorTurbulenceKind,
}

/// One `<filter>` primitive (`fe*` element). 実装指示書§9のprimitive一覧を1:1でカバーする。
#[derive(Debug, Clone)]
pub enum VectorFilterPrimitive {
    Blend(VectorBlendFilter),
    ColorMatrix(VectorColorMatrixFilter),
    ComponentTransfer(VectorComponentTransferFilter),
    Composite(VectorCompositeFilter),
    ConvolveMatrix(VectorConvolveMatrixFilter),
    DiffuseLighting(VectorDiffuseLightingFilter),
    DisplacementMap(VectorDisplacementMapFilter),
    DropShadow(VectorDropShadowFilter),
    Flood(VectorFloodFilter),
    GaussianBlur(VectorGaussianBlurFilter),
    Image(VectorFilterImage),
    Merge(VectorMergeFilter),
    Morphology(VectorMorphologyFilter),
    Offset(VectorOffsetFilter),
    SpecularLighting(VectorSpecularLightingFilter),
    Tile(VectorTileFilter),
    Turbulence(VectorTurbulenceFilter),
}

/// One primitive slot within a `VectorFilter.primitives` chain — mirrors `usvg::filter::
/// Primitive`'s own shape, where subregion (`rect`) and `color-interpolation-filters` are set
/// per primitive, not once for the whole `<filter>`.
#[derive(Debug, Clone)]
pub struct VectorFilterPrimitiveNode {
    pub rect: Rect,
    pub color_interpolation: VectorColorInterpolation,
    pub kind: VectorFilterPrimitive,
}

/// One `<filter>` element: an ordered primitive chain plus the region it's applied within.
/// `primitives[i]`'s output is addressable by later primitives as
/// `VectorFilterInput::Result(VectorFilterResultId(i as u32))`.
#[derive(Debug, Clone)]
pub struct VectorFilter {
    pub bounds: Rect,
    pub primitives: Arc<[VectorFilterPrimitiveNode]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primitive_node(kind: VectorFilterPrimitive) -> VectorFilterPrimitiveNode {
        VectorFilterPrimitiveNode {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            color_interpolation: VectorColorInterpolation::default(),
            kind,
        }
    }

    #[test]
    fn filter_result_id_addresses_a_prior_primitive_by_index() {
        let filter = VectorFilter {
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            primitives: Arc::from([
                primitive_node(VectorFilterPrimitive::Flood(VectorFloodFilter {
                    color: Color::black(),
                    opacity: 1.0,
                })),
                primitive_node(VectorFilterPrimitive::GaussianBlur(VectorGaussianBlurFilter {
                    input: VectorFilterInput::Result(VectorFilterResultId(0)),
                    std_dev_x: 2.0,
                    std_dev_y: 2.0,
                })),
            ]),
        };
        assert_eq!(filter.primitives.len(), 2);
        assert!(matches!(
            &filter.primitives[1].kind,
            VectorFilterPrimitive::GaussianBlur(VectorGaussianBlurFilter {
                input: VectorFilterInput::Result(VectorFilterResultId(0)),
                ..
            })
        ));
    }

    #[test]
    fn color_interpolation_defaults_to_linear_rgb_per_svg_spec() {
        assert_eq!(
            VectorColorInterpolation::default(),
            VectorColorInterpolation::LinearRgb
        );
    }
}
