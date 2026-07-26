//! The SVG filter chain (`VectorFilterPrimitive`) on top of Core Image. Each primitive maps
//! to a `CIFilter` (or a hand-built colour-matrix/convolution equivalent where Core Image has
//! no direct counterpart), composed in declaration order against the shared `CIContext`.

use elwindui_core::base::{AffineTransform, Rect};
use elwindui_core::graphics::{
    VectorFilter, VectorFilterInput, VectorFilterPrimitive, VectorFilterPrimitiveNode, VectorNode,
};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_core_image::{CIColor, CIImage, CIVector};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSValue};
use objc2_quartz_core::CALayer;
use std::collections::HashMap;

use super::raster::*;
use super::*;

pub(crate) fn filters_bounds(filters: &[VectorFilter]) -> Option<Rect> {
    filters.iter().map(|f| f.bounds).reduce(|a, b| a.union(b))
}

pub(crate) fn render_filtered_content(
    target: &Retained<CALayer>,
    children: &[VectorNode],
    filters: &[VectorFilter],
    world: &AffineTransform,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) {
    let Some(local_rect) = filters_bounds(filters) else {
        for child in children {
            render_node(target, child, world, 1.0, image_cache);
        }
        return;
    };
    let Some((pixels, width, height)) =
        rasterize_nodes_to_pixels(children, local_rect, image_cache)
    else {
        return;
    };
    let Some(source_cgimage) = pixels_to_cgimage(pixels, width, height) else {
        return;
    };
    let source_ci = unsafe { CIImage::imageWithCGImage(&source_cgimage) };
    // Shift the CIImage's extent to `local_rect`'s own origin so filter primitive subregions
    // (already in that same local coordinate space) line up with it.
    let source_ci = unsafe {
        source_ci.imageByApplyingTransform(CGAffineTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: local_rect.x as f64,
            ty: local_rect.y as f64,
        })
    };

    let mut current = source_ci.clone();
    for filter in filters {
        let mut results: Vec<Retained<CIImage>> = Vec::with_capacity(filter.primitives.len());
        for primitive in filter.primitives.iter() {
            let output =
                apply_filter_primitive(primitive, &current, &source_ci, &results, local_rect);
            let output = output
                .unwrap_or_else(|| results.last().cloned().unwrap_or_else(|| current.clone()));
            results.push(output);
        }
        if let Some(last) = results.last() {
            current = last.clone();
        }
    }

    let render_rect = CGRect::new(
        CGPoint::new(local_rect.x as f64, local_rect.y as f64),
        CGSize::new(local_rect.width as f64, local_rect.height as f64),
    );
    let Some(result_cgimage) =
        SHARED_CI_CONTEXT.with(|ctx| unsafe { ctx.createCGImage_fromRect(&current, render_rect) })
    else {
        return;
    };
    let result_cgimage = retained_to_cf_cgimage(result_cgimage);
    let result_layer = place_offscreen_image(&result_cgimage, local_rect, world, 1.0);
    target.addSublayer(&result_layer);
}

/// Bridges an `objc2`-managed `Retained<CGImage>` (what `CIContext::createCGImage:fromRect:`
/// returns) into the `objc2_core_foundation`-managed `CFRetained<CGImage>` every other `CGImage`
/// in this module is carried as — sound because `CGImageRef` is toll-free bridged with `id`, so
/// the two retain/release mechanisms are the same underlying operation (same reasoning
/// `inner.rs::decode_cgimage`'s own `NSImage.CGImageForProposedRect:...` bridge documents).
pub(crate) fn retained_to_cf_cgimage(image: Retained<CGImage>) -> CFRetained<CGImage> {
    let ptr = std::ptr::NonNull::new(Retained::into_raw(image)).expect("Retained is never null");
    unsafe { CFRetained::from_raw(ptr) }
}

pub(crate) fn resolve_filter_input(
    input: &VectorFilterInput,
    source_graphic: &Retained<CIImage>,
    results: &[Retained<CIImage>],
) -> Option<Retained<CIImage>> {
    match input {
        VectorFilterInput::SourceGraphic => Some(source_graphic.clone()),
        VectorFilterInput::SourceAlpha => Some(unsafe {
            source_graphic.imageByApplyingFilter(&NSString::from_str("CIMaskToAlpha"))
        }),
        VectorFilterInput::Result(id) => results.get(id.0 as usize).cloned(),
        // Neither backdrop compositing nor separate fill/stroke paint images are tracked through
        // this render path — see `VectorFilterInput`'s own doc comment; these fall back to
        // `SourceGraphic` rather than producing an empty/transparent input.
        VectorFilterInput::BackgroundImage
        | VectorFilterInput::BackgroundAlpha
        | VectorFilterInput::FillPaint
        | VectorFilterInput::StrokePaint => Some(source_graphic.clone()),
    }
}

pub(crate) fn ci_dict(pairs: &[(&str, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| k.as_ref()).collect();
    let values: Vec<&AnyObject> = pairs.iter().map(|(_, v)| *v).collect();
    NSDictionary::from_slices(&key_refs, &values)
}

pub(crate) fn ci_vector4(x: f32, y: f32, z: f32, w: f32) -> Retained<CIVector> {
    unsafe { CIVector::vectorWithX_Y_Z_W(x as f64, y as f64, z as f64, w as f64) }
}

pub(crate) fn apply_filter_primitive(
    node: &VectorFilterPrimitiveNode,
    default_input_image: &Retained<CIImage>,
    source_graphic: &Retained<CIImage>,
    results: &[Retained<CIImage>],
    local_rect: Rect,
) -> Option<Retained<CIImage>> {
    let _ = default_input_image;
    match &node.kind {
        VectorFilterPrimitive::GaussianBlur(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            let sigma = ((fe.std_dev_x + fe.std_dev_y) / 2.0).max(0.0) as f64;
            Some(unsafe { input.imageByApplyingGaussianBlurWithSigma(sigma) })
        }
        VectorFilterPrimitive::Offset(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            Some(unsafe {
                input.imageByApplyingTransform(CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    tx: fe.dx as f64,
                    ty: fe.dy as f64,
                })
            })
        }
        VectorFilterPrimitive::Merge(fe) => {
            let mut acc: Option<Retained<CIImage>> = None;
            for input in fe.inputs.iter() {
                let image = resolve_filter_input(input, source_graphic, results)?;
                acc = Some(match acc {
                    Some(dest) => unsafe { image.imageByCompositingOverImage(&dest) },
                    None => image,
                });
            }
            acc
        }
        VectorFilterPrimitive::Composite(fe) => {
            let input1 = resolve_filter_input(&fe.input1, source_graphic, results)?;
            let input2 = resolve_filter_input(&fe.input2, source_graphic, results)?;
            composite(&input1, &input2, fe.operator)
        }
        VectorFilterPrimitive::Blend(fe) => {
            let input1 = resolve_filter_input(&fe.input1, source_graphic, results)?;
            let input2 = resolve_filter_input(&fe.input2, source_graphic, results)?;
            match ci_blend_mode_filter_name(fe.mode) {
                Some(name) => {
                    let params =
                        ci_dict(&[("inputBackgroundImage", input2.as_ref() as &AnyObject)]);
                    Some(unsafe {
                        input1.imageByApplyingFilter_withInputParameters(
                            &NSString::from_str(name),
                            &params,
                        )
                    })
                }
                None => Some(unsafe { input1.imageByCompositingOverImage(&input2) }),
            }
        }
        VectorFilterPrimitive::Flood(fe) => {
            let color = unsafe {
                CIColor::colorWithRed_green_blue_alpha(
                    fe.color.r as f64 / 255.0,
                    fe.color.g as f64 / 255.0,
                    fe.color.b as f64 / 255.0,
                    fe.opacity as f64,
                )
            };
            let image = unsafe { CIImage::initWithColor(CIImage::alloc(), &color) };
            let rect = CGRect::new(
                CGPoint::new(local_rect.x as f64, local_rect.y as f64),
                CGSize::new(local_rect.width as f64, local_rect.height as f64),
            );
            Some(unsafe { image.imageByCroppingToRect(rect) })
        }
        VectorFilterPrimitive::ColorMatrix(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            apply_color_matrix(&input, fe)
        }
        VectorFilterPrimitive::Morphology(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            let name = match fe.operator {
                elwindui_core::graphics::VectorMorphologyOperator::Dilate => "CIMorphologyMaximum",
                elwindui_core::graphics::VectorMorphologyOperator::Erode => "CIMorphologyMinimum",
            };
            let radius = ((fe.radius_x + fe.radius_y) / 2.0).max(0.0);
            let radius_num = NSNumber::new_f64(radius as f64);
            let params = ci_dict(&[("inputRadius", radius_num.as_ref() as &AnyObject)]);
            Some(unsafe {
                input.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params)
            })
        }
        VectorFilterPrimitive::ConvolveMatrix(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            apply_convolve_matrix(&input, fe)
        }
        VectorFilterPrimitive::DropShadow(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            apply_drop_shadow(&input, fe)
        }
        VectorFilterPrimitive::Tile(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            // `CIAffineTile` ("applies an affine transformation to an image and then tiles the
            // transformed image") matches feTile's own semantics exactly under the identity
            // transform: repeat the input's own extent infinitely.
            let identity = NSValue::new(CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: 0.0,
                ty: 0.0,
            });
            let params = ci_dict(&[("inputTransform", identity.as_ref() as &AnyObject)]);
            Some(unsafe {
                input.imageByApplyingFilter_withInputParameters(
                    &NSString::from_str("CIAffineTile"),
                    &params,
                )
            })
        }
        VectorFilterPrimitive::Turbulence(_) => {
            report_unsupported("feTurbulence filter primitive (input passed through)");
            None
        }
        VectorFilterPrimitive::DiffuseLighting(fe) => {
            report_unsupported("feDiffuseLighting filter primitive (input passed through)");
            resolve_filter_input(&fe.input, source_graphic, results)
        }
        VectorFilterPrimitive::SpecularLighting(fe) => {
            report_unsupported("feSpecularLighting filter primitive (input passed through)");
            resolve_filter_input(&fe.input, source_graphic, results)
        }
        VectorFilterPrimitive::DisplacementMap(fe) => {
            report_unsupported("feDisplacementMap filter primitive (input passed through)");
            resolve_filter_input(&fe.input1, source_graphic, results)
        }
        VectorFilterPrimitive::ComponentTransfer(fe) => {
            let input = resolve_filter_input(&fe.input, source_graphic, results)?;
            apply_component_transfer(&input, fe)
        }
        VectorFilterPrimitive::Image(fe) => {
            let Some((pixels, w, h)) = rasterize_nodes_to_pixels(
                std::slice::from_ref(&VectorNode::Group(fe.root.clone())),
                local_rect,
                &mut HashMap::new(),
            ) else {
                return None;
            };
            let cgimage = pixels_to_cgimage(pixels, w, h)?;
            let ci = unsafe { CIImage::imageWithCGImage(&cgimage) };
            Some(unsafe {
                ci.imageByApplyingTransform(CGAffineTransform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    tx: local_rect.x as f64,
                    ty: local_rect.y as f64,
                })
            })
        }
    }
}

pub(crate) fn composite(
    input1: &Retained<CIImage>,
    input2: &Retained<CIImage>,
    operator: elwindui_core::graphics::VectorCompositeOperator,
) -> Option<Retained<CIImage>> {
    use elwindui_core::graphics::VectorCompositeOperator;
    match operator {
        VectorCompositeOperator::Over
        | VectorCompositeOperator::In
        | VectorCompositeOperator::Out
        | VectorCompositeOperator::Atop => {
            let name = match operator {
                VectorCompositeOperator::Over => "CISourceOverCompositing",
                VectorCompositeOperator::In => "CISourceInCompositing",
                VectorCompositeOperator::Out => "CISourceOutCompositing",
                VectorCompositeOperator::Atop => "CISourceAtopCompositing",
                _ => unreachable!("matched above"),
            };
            Some(apply_named(input1, name, input2))
        }
        // The union of "i1 minus i2" and "i2 minus i1" — SVG's own definition of Xor compositing
        // — built from the same `CISourceOutCompositing` the arm above already uses for `Out`.
        VectorCompositeOperator::Xor => {
            let out1 = apply_named(input1, "CISourceOutCompositing", input2);
            let out2 = apply_named(input2, "CISourceOutCompositing", input1);
            Some(unsafe { out1.imageByCompositingOverImage(&out2) })
        }
        // `result = k1*i1*i2 + k2*i1 + k3*i2 + k4`, SVG's own formula, computed directly from
        // existing named Core Image filters: a true per-pixel multiply (`CIMultiplyCompositing` —
        // distinct from `CIMultiplyBlendMode`, which this module's own group blend-mode handling
        // uses and which layers its own alpha-compositing formula on top rather than a bare
        // multiply), uniform per-channel scaling (`CIColorMatrix`, the same technique
        // `apply_drop_shadow`'s own tint/opacity matrices already use), and true per-pixel
        // addition (`CIAdditionCompositing`).
        VectorCompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
            let product = apply_named(input1, "CIMultiplyCompositing", input2);
            let term1 = scale_all_channels(&product, k1);
            let term2 = scale_all_channels(input1, k2);
            let term3 = scale_all_channels(input2, k3);
            let constant = flat_color_image(k4, k4, k4, k4, unsafe { input1.extent() });
            let sum = apply_named(&term1, "CIAdditionCompositing", &term2);
            let sum = apply_named(&sum, "CIAdditionCompositing", &term3);
            Some(apply_named(&sum, "CIAdditionCompositing", &constant))
        }
    }
}

/// Applies a two-input named Core Image compositing filter (`CISourceOverCompositing`,
/// `CIMultiplyCompositing`, `CIAdditionCompositing`, ...) — `image` is `inputImage`, `other` is
/// `inputBackgroundImage`, matching every two-input filter this module already uses this same
/// parameter-name convention for.
pub(crate) fn apply_named(
    image: &Retained<CIImage>,
    name: &str,
    other: &Retained<CIImage>,
) -> Retained<CIImage> {
    let params = ci_dict(&[("inputBackgroundImage", other.as_ref() as &AnyObject)]);
    unsafe { image.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params) }
}

/// Multiplies every channel (R, G, B, and A alike) of `image` by the same scalar `k` via
/// `CIColorMatrix` — the building block `composite`'s `Arithmetic` arm uses for its `k1`/`k2`/`k3`
/// terms, and the same diagonal-matrix technique `apply_drop_shadow` already uses for its own
/// tint/opacity adjustments.
pub(crate) fn scale_all_channels(image: &Retained<CIImage>, k: f32) -> Retained<CIImage> {
    let zero = ci_vector4(0.0, 0.0, 0.0, 0.0);
    let params = ci_dict(&[
        (
            "inputRVector",
            ci_vector4(k, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputGVector",
            ci_vector4(0.0, k, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputBVector",
            ci_vector4(0.0, 0.0, k, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputAVector",
            ci_vector4(0.0, 0.0, 0.0, k).as_ref() as &AnyObject,
        ),
        ("inputBiasVector", zero.as_ref() as &AnyObject),
    ]);
    unsafe {
        image.imageByApplyingFilter_withInputParameters(
            &NSString::from_str("CIColorMatrix"),
            &params,
        )
    }
}

/// A solid, uniform-color `CIImage` covering exactly `extent` — the `k4` constant term in
/// `composite`'s `Arithmetic` arm, and the same `CIColor`+`CIImage::initWithColor`+
/// `imageByCroppingToRect` technique the `Flood` filter primitive arm already uses (a bare
/// `CIImage` color fill has infinite extent until cropped).
pub(crate) fn flat_color_image(
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    extent: CGRect,
) -> Retained<CIImage> {
    let color =
        unsafe { CIColor::colorWithRed_green_blue_alpha(r as f64, g as f64, b as f64, a as f64) };
    let image = unsafe { CIImage::initWithColor(CIImage::alloc(), &color) };
    unsafe { image.imageByCroppingToRect(extent) }
}

pub(crate) fn apply_color_matrix(
    input: &Retained<CIImage>,
    fe: &elwindui_core::graphics::VectorColorMatrixFilter,
) -> Option<Retained<CIImage>> {
    use elwindui_core::graphics::VectorColorMatrixKind;
    let matrix: [f32; 20] = match &fe.kind {
        VectorColorMatrixKind::Matrix(m) => **m,
        VectorColorMatrixKind::Saturate(s) => saturate_matrix(*s),
        VectorColorMatrixKind::HueRotate(deg) => hue_rotate_matrix(*deg),
        VectorColorMatrixKind::LuminanceToAlpha => LUMINANCE_TO_ALPHA_MATRIX,
    };
    let r = ci_vector4(matrix[0], matrix[1], matrix[2], matrix[3]);
    let g = ci_vector4(matrix[5], matrix[6], matrix[7], matrix[8]);
    let b = ci_vector4(matrix[10], matrix[11], matrix[12], matrix[13]);
    let a = ci_vector4(matrix[15], matrix[16], matrix[17], matrix[18]);
    let bias = unsafe {
        CIVector::vectorWithX_Y_Z_W(
            matrix[4] as f64,
            matrix[9] as f64,
            matrix[14] as f64,
            matrix[19] as f64,
        )
    };
    let params = ci_dict(&[
        ("inputRVector", r.as_ref() as &AnyObject),
        ("inputGVector", g.as_ref() as &AnyObject),
        ("inputBVector", b.as_ref() as &AnyObject),
        ("inputAVector", a.as_ref() as &AnyObject),
        ("inputBiasVector", bias.as_ref() as &AnyObject),
    ]);
    Some(unsafe {
        input.imageByApplyingFilter_withInputParameters(
            &NSString::from_str("CIColorMatrix"),
            &params,
        )
    })
}

pub(crate) const LUMINANCE_TO_ALPHA_MATRIX: [f32; 20] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2125, 0.7154,
    0.0721, 0.0, 0.0,
];

/// Standard SVG `feColorMatrix type="saturate"` matrix (SVG 1.1 §15.10).
pub(crate) fn saturate_matrix(s: f32) -> [f32; 20] {
    [
        0.213 + 0.787 * s,
        0.715 - 0.715 * s,
        0.072 - 0.072 * s,
        0.0,
        0.0,
        0.213 - 0.213 * s,
        0.715 + 0.285 * s,
        0.072 - 0.072 * s,
        0.0,
        0.0,
        0.213 - 0.213 * s,
        0.715 - 0.715 * s,
        0.072 + 0.928 * s,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

/// Standard SVG `feColorMatrix type="hueRotate"` matrix (SVG 1.1 §15.10).
pub(crate) fn hue_rotate_matrix(degrees: f32) -> [f32; 20] {
    let (s, c) = degrees.to_radians().sin_cos();
    [
        0.213 + c * 0.787 - s * 0.213,
        0.715 - c * 0.715 - s * 0.715,
        0.072 - c * 0.072 + s * 0.928,
        0.0,
        0.0,
        0.213 - c * 0.213 + s * 0.143,
        0.715 + c * 0.285 + s * 0.140,
        0.072 - c * 0.072 - s * 0.283,
        0.0,
        0.0,
        0.213 - c * 0.213 - s * 0.787,
        0.715 - c * 0.715 + s * 0.715,
        0.072 + c * 0.928 + s * 0.072,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

pub(crate) fn apply_convolve_matrix(
    input: &Retained<CIImage>,
    fe: &elwindui_core::graphics::VectorConvolveMatrixFilter,
) -> Option<Retained<CIImage>> {
    let name = match (fe.order_x, fe.order_y) {
        (3, 3) => "CIConvolution3X3",
        (5, 5) => "CIConvolution5X5",
        _ => {
            report_unsupported(
                "feConvolveMatrix with an order other than 3x3/5x5 (input passed through)",
            );
            return Some(input.clone());
        }
    };
    let count = fe.kernel.len();
    let mut values: Vec<f64> = fe
        .kernel
        .iter()
        .map(|&v| v as f64 / fe.divisor.max(1e-6) as f64)
        .collect();
    values.reverse(); // SVG kernels are specified in reading order; Core Image expects the flipped orientation.
    let values_ptr = std::ptr::NonNull::new(values.as_mut_ptr()).expect("non-empty kernel");
    let weights = unsafe { CIVector::vectorWithValues_count(values_ptr, count) };
    let bias_num = NSNumber::new_f64(fe.bias as f64);
    let params = ci_dict(&[
        ("inputWeights", weights.as_ref() as &AnyObject),
        ("inputBias", bias_num.as_ref() as &AnyObject),
    ]);
    Some(unsafe {
        input.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params)
    })
}

pub(crate) fn apply_drop_shadow(
    input: &Retained<CIImage>,
    fe: &elwindui_core::graphics::VectorDropShadowFilter,
) -> Option<Retained<CIImage>> {
    // feDropShadow ≈ feGaussianBlur → feOffset → flood(color) composited under the original
    // (SVG 1.1 §15.15's own "equivalent to" definition), built directly from `CIImage` steps
    // rather than a single named CIFilter (Core Image has no exact `feDropShadow` counterpart).
    let alpha_matrix = ci_dict(&[
        (
            "inputRVector",
            ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputGVector",
            ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputBVector",
            ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputAVector",
            ci_vector4(0.0, 0.0, 0.0, 1.0).as_ref() as &AnyObject,
        ),
        (
            "inputBiasVector",
            unsafe {
                CIVector::vectorWithX_Y_Z_W(
                    fe.color.r as f64 / 255.0,
                    fe.color.g as f64 / 255.0,
                    fe.color.b as f64 / 255.0,
                    0.0,
                )
            }
            .as_ref() as &AnyObject,
        ),
    ]);
    let tinted = unsafe {
        input.imageByApplyingFilter_withInputParameters(
            &NSString::from_str("CIColorMatrix"),
            &alpha_matrix,
        )
    };
    let sigma = ((fe.std_dev_x + fe.std_dev_y) / 2.0).max(0.0) as f64;
    let blurred = unsafe { tinted.imageByApplyingGaussianBlurWithSigma(sigma) };
    let offset = unsafe {
        blurred.imageByApplyingTransform(CGAffineTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: fe.dx as f64,
            ty: fe.dy as f64,
        })
    };
    let opacity_matrix = ci_dict(&[
        (
            "inputRVector",
            ci_vector4(1.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputGVector",
            ci_vector4(0.0, 1.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputBVector",
            ci_vector4(0.0, 0.0, 1.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputAVector",
            ci_vector4(0.0, 0.0, 0.0, fe.opacity).as_ref() as &AnyObject,
        ),
        (
            "inputBiasVector",
            ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
    ]);
    let shadow = unsafe {
        offset.imageByApplyingFilter_withInputParameters(
            &NSString::from_str("CIColorMatrix"),
            &opacity_matrix,
        )
    };
    Some(unsafe { input.imageByCompositingOverImage(&shadow) })
}

pub(crate) fn apply_component_transfer(
    input: &Retained<CIImage>,
    fe: &elwindui_core::graphics::VectorComponentTransferFilter,
) -> Option<Retained<CIImage>> {
    use elwindui_core::graphics::VectorTransferFunction;
    // Only the common "every channel uses a Linear (or Identity) function" case maps cleanly onto
    // `CIColorMatrix`; `Table`/`Discrete`/`Gamma` piecewise curves have no direct Core Image
    // equivalent short of a custom color kernel, so they pass their input through unchanged.
    let linear = |f: &VectorTransferFunction| match f {
        VectorTransferFunction::Identity => Some((1.0, 0.0)),
        VectorTransferFunction::Linear { slope, intercept } => Some((*slope, *intercept)),
        _ => None,
    };
    let (Some((rs, ri)), Some((gs, gi)), Some((bs, bi)), Some((as_, ai))) = (
        linear(&fe.red),
        linear(&fe.green),
        linear(&fe.blue),
        linear(&fe.alpha),
    ) else {
        report_unsupported(
            "feComponentTransfer with a Table/Discrete/Gamma function (input passed through)",
        );
        return Some(input.clone());
    };
    let params = ci_dict(&[
        (
            "inputRVector",
            ci_vector4(rs, 0.0, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputGVector",
            ci_vector4(0.0, gs, 0.0, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputBVector",
            ci_vector4(0.0, 0.0, bs, 0.0).as_ref() as &AnyObject,
        ),
        (
            "inputAVector",
            ci_vector4(0.0, 0.0, 0.0, as_).as_ref() as &AnyObject,
        ),
        (
            "inputBiasVector",
            ci_vector4(ri, gi, bi, ai).as_ref() as &AnyObject,
        ),
    ]);
    Some(unsafe {
        input.imageByApplyingFilter_withInputParameters(
            &NSString::from_str("CIColorMatrix"),
            &params,
        )
    })
}
