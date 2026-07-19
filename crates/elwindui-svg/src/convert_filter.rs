//! `usvg::filter::Filter` → `VectorFilter` (実装指示書§9).

use crate::convert::{color_from_usvg, convert_group, rect_from_nonzero};
use elwindui_core::graphics::{
    VectorBlendFilter, VectorColorChannel, VectorColorInterpolation, VectorColorMatrixFilter,
    VectorColorMatrixKind, VectorComponentTransferFilter, VectorCompositeFilter,
    VectorCompositeOperator, VectorConvolveMatrixFilter, VectorDiffuseLightingFilter,
    VectorDisplacementMapFilter, VectorDropShadowFilter, VectorEdgeMode, VectorFilter,
    VectorFilterImage, VectorFilterInput, VectorFilterPrimitive, VectorFilterPrimitiveNode,
    VectorFilterResultId, VectorFloodFilter, VectorGaussianBlurFilter, VectorLightSource,
    VectorMergeFilter, VectorMorphologyFilter, VectorMorphologyOperator, VectorOffsetFilter,
    VectorSpecularLightingFilter, VectorTileFilter, VectorTransferFunction,
    VectorTurbulenceFilter, VectorTurbulenceKind,
};
use std::collections::HashMap;

pub(crate) fn convert_filter(filter: &usvg::filter::Filter) -> VectorFilter {
    // `usvg::filter::Input::Reference` carries the target primitive's `result` *name*; resolving
    // it to a `VectorFilterResultId` index happens here, once, at conversion time — the render
    // path never sees primitive names (実装指示書§9's own `VectorFilterResultId` doc comment).
    let mut named_results: HashMap<&str, u32> = HashMap::new();

    let primitives: Vec<VectorFilterPrimitiveNode> = filter
        .primitives()
        .iter()
        .enumerate()
        .map(|(index, primitive)| {
            let node = VectorFilterPrimitiveNode {
                rect: rect_from_nonzero(primitive.rect()),
                color_interpolation: convert_color_interpolation(primitive.color_interpolation()),
                kind: convert_kind(primitive.kind(), index as u32, &named_results),
            };
            if !primitive.result().is_empty() {
                named_results.insert(primitive.result(), index as u32);
            }
            node
        })
        .collect();

    VectorFilter {
        bounds: rect_from_nonzero(filter.rect()),
        primitives: primitives.into(),
    }
}

fn convert_color_interpolation(mode: usvg::filter::ColorInterpolation) -> VectorColorInterpolation {
    match mode {
        usvg::filter::ColorInterpolation::SRGB => VectorColorInterpolation::SRgb,
        usvg::filter::ColorInterpolation::LinearRGB => VectorColorInterpolation::LinearRgb,
    }
}

/// Resolves `in`/`in2` per SVG's own fallback rule: an unresolvable/absent named reference means
/// `SourceGraphic` for the first primitive in the chain, or the immediately preceding primitive's
/// result otherwise.
fn convert_input(
    input: &usvg::filter::Input,
    self_index: u32,
    named_results: &HashMap<&str, u32>,
) -> VectorFilterInput {
    match input {
        usvg::filter::Input::SourceGraphic => VectorFilterInput::SourceGraphic,
        usvg::filter::Input::SourceAlpha => VectorFilterInput::SourceAlpha,
        usvg::filter::Input::Reference(name) => match named_results.get(name.as_str()) {
            Some(&index) => VectorFilterInput::Result(VectorFilterResultId(index)),
            None if self_index == 0 => VectorFilterInput::SourceGraphic,
            None => VectorFilterInput::Result(VectorFilterResultId(self_index - 1)),
        },
    }
}

fn convert_light_source(light: usvg::filter::LightSource) -> VectorLightSource {
    match light {
        usvg::filter::LightSource::DistantLight(l) => VectorLightSource::Distant {
            azimuth: l.azimuth,
            elevation: l.elevation,
        },
        usvg::filter::LightSource::PointLight(l) => VectorLightSource::Point {
            x: l.x,
            y: l.y,
            z: l.z,
        },
        usvg::filter::LightSource::SpotLight(l) => VectorLightSource::Spot {
            x: l.x,
            y: l.y,
            z: l.z,
            points_at_x: l.points_at_x,
            points_at_y: l.points_at_y,
            points_at_z: l.points_at_z,
            specular_exponent: l.specular_exponent.get(),
            limiting_cone_angle: l.limiting_cone_angle,
        },
    }
}

fn convert_transfer_function(func: &usvg::filter::TransferFunction) -> VectorTransferFunction {
    match func {
        usvg::filter::TransferFunction::Identity => VectorTransferFunction::Identity,
        usvg::filter::TransferFunction::Table(v) => VectorTransferFunction::Table(v.as_slice().into()),
        usvg::filter::TransferFunction::Discrete(v) => {
            VectorTransferFunction::Discrete(v.as_slice().into())
        }
        usvg::filter::TransferFunction::Linear { slope, intercept } => {
            VectorTransferFunction::Linear {
                slope: *slope,
                intercept: *intercept,
            }
        }
        usvg::filter::TransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => VectorTransferFunction::Gamma {
            amplitude: *amplitude,
            exponent: *exponent,
            offset: *offset,
        },
    }
}

fn convert_edge_mode(mode: usvg::filter::EdgeMode) -> VectorEdgeMode {
    match mode {
        usvg::filter::EdgeMode::None => VectorEdgeMode::None,
        usvg::filter::EdgeMode::Duplicate => VectorEdgeMode::Duplicate,
        usvg::filter::EdgeMode::Wrap => VectorEdgeMode::Wrap,
    }
}

fn convert_color_channel(channel: usvg::filter::ColorChannel) -> VectorColorChannel {
    match channel {
        usvg::filter::ColorChannel::R => VectorColorChannel::R,
        usvg::filter::ColorChannel::G => VectorColorChannel::G,
        usvg::filter::ColorChannel::B => VectorColorChannel::B,
        usvg::filter::ColorChannel::A => VectorColorChannel::A,
    }
}

fn convert_kind(
    kind: &usvg::filter::Kind,
    self_index: u32,
    named_results: &HashMap<&str, u32>,
) -> VectorFilterPrimitive {
    let input = |i: &usvg::filter::Input| convert_input(i, self_index, named_results);
    match kind {
        usvg::filter::Kind::Blend(fe) => VectorFilterPrimitive::Blend(VectorBlendFilter {
            input1: input(fe.input1()),
            input2: input(fe.input2()),
            mode: super::convert_paint::blend_mode_from_usvg(fe.mode()),
        }),
        usvg::filter::Kind::ColorMatrix(fe) => {
            VectorFilterPrimitive::ColorMatrix(VectorColorMatrixFilter {
                input: input(fe.input()),
                kind: match fe.kind() {
                    usvg::filter::ColorMatrixKind::Matrix(m) => {
                        let arr: Result<[f32; 20], _> = m.as_slice().try_into();
                        match arr {
                            Ok(arr) => VectorColorMatrixKind::Matrix(std::sync::Arc::new(arr)),
                            Err(_) => VectorColorMatrixKind::Matrix(std::sync::Arc::new(
                                IDENTITY_COLOR_MATRIX,
                            )),
                        }
                    }
                    usvg::filter::ColorMatrixKind::Saturate(v) => {
                        VectorColorMatrixKind::Saturate(v.get())
                    }
                    usvg::filter::ColorMatrixKind::HueRotate(v) => {
                        VectorColorMatrixKind::HueRotate(*v)
                    }
                    usvg::filter::ColorMatrixKind::LuminanceToAlpha => {
                        VectorColorMatrixKind::LuminanceToAlpha
                    }
                },
            })
        }
        usvg::filter::Kind::ComponentTransfer(fe) => {
            VectorFilterPrimitive::ComponentTransfer(VectorComponentTransferFilter {
                input: input(fe.input()),
                red: convert_transfer_function(fe.func_r()),
                green: convert_transfer_function(fe.func_g()),
                blue: convert_transfer_function(fe.func_b()),
                alpha: convert_transfer_function(fe.func_a()),
            })
        }
        usvg::filter::Kind::Composite(fe) => VectorFilterPrimitive::Composite(VectorCompositeFilter {
            input1: input(fe.input1()),
            input2: input(fe.input2()),
            operator: match fe.operator() {
                usvg::filter::CompositeOperator::Over => VectorCompositeOperator::Over,
                usvg::filter::CompositeOperator::In => VectorCompositeOperator::In,
                usvg::filter::CompositeOperator::Out => VectorCompositeOperator::Out,
                usvg::filter::CompositeOperator::Atop => VectorCompositeOperator::Atop,
                usvg::filter::CompositeOperator::Xor => VectorCompositeOperator::Xor,
                usvg::filter::CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
                    VectorCompositeOperator::Arithmetic { k1, k2, k3, k4 }
                }
            },
        }),
        usvg::filter::Kind::ConvolveMatrix(fe) => {
            let matrix = fe.matrix();
            VectorFilterPrimitive::ConvolveMatrix(VectorConvolveMatrixFilter {
                input: input(fe.input()),
                order_x: matrix.columns(),
                order_y: matrix.rows(),
                kernel: matrix.data().into(),
                divisor: fe.divisor().get(),
                bias: fe.bias(),
                target_x: matrix.target_x() as i32,
                target_y: matrix.target_y() as i32,
                edge_mode: convert_edge_mode(fe.edge_mode()),
                preserve_alpha: fe.preserve_alpha(),
            })
        }
        usvg::filter::Kind::DiffuseLighting(fe) => {
            VectorFilterPrimitive::DiffuseLighting(VectorDiffuseLightingFilter {
                input: input(fe.input()),
                surface_scale: fe.surface_scale(),
                diffuse_constant: fe.diffuse_constant(),
                lighting_color: color_from_usvg(fe.lighting_color(), 1.0),
                light: convert_light_source(fe.light_source()),
            })
        }
        usvg::filter::Kind::DisplacementMap(fe) => {
            VectorFilterPrimitive::DisplacementMap(VectorDisplacementMapFilter {
                input1: input(fe.input1()),
                input2: input(fe.input2()),
                scale: fe.scale(),
                x_channel: convert_color_channel(fe.x_channel_selector()),
                y_channel: convert_color_channel(fe.y_channel_selector()),
            })
        }
        usvg::filter::Kind::DropShadow(fe) => VectorFilterPrimitive::DropShadow(VectorDropShadowFilter {
            input: input(fe.input()),
            dx: fe.dx(),
            dy: fe.dy(),
            std_dev_x: fe.std_dev_x().get(),
            std_dev_y: fe.std_dev_y().get(),
            color: color_from_usvg(fe.color(), 1.0),
            opacity: fe.opacity().get(),
        }),
        usvg::filter::Kind::Flood(fe) => VectorFilterPrimitive::Flood(VectorFloodFilter {
            color: color_from_usvg(fe.color(), 1.0),
            opacity: fe.opacity().get(),
        }),
        usvg::filter::Kind::GaussianBlur(fe) => {
            VectorFilterPrimitive::GaussianBlur(VectorGaussianBlurFilter {
                input: input(fe.input()),
                std_dev_x: fe.std_dev_x().get(),
                std_dev_y: fe.std_dev_y().get(),
            })
        }
        usvg::filter::Kind::Image(fe) => VectorFilterPrimitive::Image(VectorFilterImage {
            root: convert_group(fe.root()),
        }),
        usvg::filter::Kind::Merge(fe) => VectorFilterPrimitive::Merge(VectorMergeFilter {
            inputs: fe.inputs().iter().map(input).collect(),
        }),
        usvg::filter::Kind::Morphology(fe) => {
            VectorFilterPrimitive::Morphology(VectorMorphologyFilter {
                input: input(fe.input()),
                operator: match fe.operator() {
                    usvg::filter::MorphologyOperator::Erode => VectorMorphologyOperator::Erode,
                    usvg::filter::MorphologyOperator::Dilate => VectorMorphologyOperator::Dilate,
                },
                radius_x: fe.radius_x().get(),
                radius_y: fe.radius_y().get(),
            })
        }
        usvg::filter::Kind::Offset(fe) => VectorFilterPrimitive::Offset(VectorOffsetFilter {
            input: input(fe.input()),
            dx: fe.dx(),
            dy: fe.dy(),
        }),
        usvg::filter::Kind::SpecularLighting(fe) => {
            VectorFilterPrimitive::SpecularLighting(VectorSpecularLightingFilter {
                input: input(fe.input()),
                surface_scale: fe.surface_scale(),
                specular_constant: fe.specular_constant(),
                specular_exponent: fe.specular_exponent(),
                lighting_color: color_from_usvg(fe.lighting_color(), 1.0),
                light: convert_light_source(fe.light_source()),
            })
        }
        usvg::filter::Kind::Tile(fe) => VectorFilterPrimitive::Tile(VectorTileFilter {
            input: input(fe.input()),
        }),
        usvg::filter::Kind::Turbulence(fe) => VectorFilterPrimitive::Turbulence(VectorTurbulenceFilter {
            base_frequency_x: fe.base_frequency_x().get(),
            base_frequency_y: fe.base_frequency_y().get(),
            num_octaves: fe.num_octaves(),
            seed: fe.seed(),
            stitch_tiles: fe.stitch_tiles(),
            kind: match fe.kind() {
                usvg::filter::TurbulenceKind::FractalNoise => VectorTurbulenceKind::FractalNoise,
                usvg::filter::TurbulenceKind::Turbulence => VectorTurbulenceKind::Turbulence,
            },
        }),
    }
}

const IDENTITY_COLOR_MATRIX: [f32; 20] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    0.0,
];
