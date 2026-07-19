//! `RenderCommand::DrawVectorImage` rendering — `VectorGroup`/`VectorNode` traversal into
//! `CALayer` trees, reusing the same paint-command helpers (`path_to_cgpath`, `add_shape_layer`,
//! `clip_mask_layer`, `build_image_container_layer`, ...) `inner.rs`'s own ordinary
//! `replay_paint_command` uses (実装指示書§18: 巨大な処理を`inner.rs`一箇所へ詰め込まない).
//!
//! Full support: group transform/opacity/clip/mask/blend-mode/filters, path fill & stroke (solid
//! color and gradients under arbitrary rotation/scale), single-tile pattern fill, embedded raster
//! images, and nested groups (nested SVG and flattened text are both already ordinary
//! `VectorGroup`s by the time they reach this module). Mask and filter both rasterize their
//! subject into an offscreen buffer (実装指示書§9/§19: filter対象groupのlayer boundsだけtemporary
//! surface) — never the whole `VectorImage`, only the mask/filter region itself. Filter primitives
//! map onto Core Image (`objc2_core_image`); the handful with no reasonable Core Image equivalent
//! (`Tile`, `Turbulence`, `DiffuseLighting`/`SpecularLighting`, `DisplacementMap`, non-3x3/5x5
//! `ConvolveMatrix`, non-`Arithmetic`-covered `Composite::Xor`) pass their input through unchanged
//! and report once via [`report_unsupported`] rather than aborting the whole filter chain or
//! silently producing a wrong-looking result.

use crate::inner::{
    add_shape_layer, apply_stroke, build_image_container_layer, clip_mask_layer, color_to_cgcolor,
    fitted_image_rect, gradient_unit_point, path_to_cgpath, resolve_cgimage,
};
use elwindui_core::base::{AffineTransform, Point, Rect};
use elwindui_core::graphics::{
    Brush, Clip, Color, FillRule, GradientStop, Path, StrokeStyle, VectorBlendMode, VectorFill,
    VectorFilter, VectorFilterInput, VectorFilterPrimitive, VectorFilterPrimitiveNode,
    VectorGroup, VectorImage, VectorImageDrawOptions, VectorMask, VectorMaskType, VectorNode,
    VectorPaint, VectorPathNode, VectorPattern, VectorRasterNode, VectorStroke,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo};
use objc2_core_image::{CIColor, CIContext, CIFilter, CIImage, CIVector};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_quartz_core::{
    CAGradientLayer, CALayer, CAShapeLayer, kCAFillRuleEvenOdd, kCAFillRuleNonZero,
    kCAGradientLayerAxial, kCAGradientLayerRadial,
};
use std::collections::HashMap;

/// The largest offscreen buffer dimension (mask/pattern-tile/filter rasterization) allowed in
/// either axis — a defensive cap against a pathological `mask`/`filter` region blowing up memory,
/// independent of `elwindui-svg`'s own `SvgLimits` (which bounds the *source* document, not what a
/// particular backend chooses to rasterize it at).
const MAX_OFFSCREEN_DIMENSION: usize = 4096;

/// A vector feature with no reasonable mapping onto this backend's native APIs — reported once
/// (debug builds only, matching `elwindui-backend-winui3`'s own `unsupported_command!`
/// convention) rather than silently dropped; the surrounding content still renders.
fn report_unsupported(feature: &str) {
    #[cfg(debug_assertions)]
    eprintln!("[elwindui-backend-appkit] unsupported VectorImage feature: {feature}");
    #[cfg(not(debug_assertions))]
    let _ = feature;
}

/// Entry point called from `inner.rs`'s `replay_paint_command` for `RenderCommand::DrawVectorImage`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_vector_image(
    layer: &Retained<CALayer>,
    image: &VectorImage,
    dest: Rect,
    source: Option<Rect>,
    options: &VectorImageDrawOptions,
    world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let src_rect = source.unwrap_or_else(|| image.view_box());
    if src_rect.width <= 0.0 || src_rect.height <= 0.0 {
        return;
    }

    // Reuses the exact same `Fill`/`Contain`/`Cover`/`None` + alignment placement math ordinary
    // `DrawImage` uses (実装指示書§17) — `src_rect`'s size stands in for `DrawImage`'s own
    // `image_size` parameter.
    let placed = fitted_image_rect(
        dest,
        (src_rect.width, src_rect.height),
        options.fit,
        options.alignment_x,
        options.alignment_y,
    );
    let scale_x = if src_rect.width.abs() > 1e-6 {
        placed.width / src_rect.width
    } else {
        1.0
    };
    let scale_y = if src_rect.height.abs() > 1e-6 {
        placed.height / src_rect.height
    } else {
        1.0
    };
    let root_local = AffineTransform::translation(dest.x + placed.x, dest.y + placed.y)
        .concat(&AffineTransform::scale(scale_x, scale_y))
        .concat(&AffineTransform::translation(-src_rect.x, -src_rect.y));
    let root_world = world.concat(&root_local);
    let combined_opacity = opacity * options.opacity;

    let container = if options.clip_to_dest {
        let clip_container = CALayer::new();
        clip_container.setName(Some(&NSString::from_str("elwindui-paint")));
        clip_container.setFrame(layer.bounds());
        let mask = clip_mask_layer(world, &Clip::Rect(dest));
        unsafe { clip_container.setMask(Some(&mask)) };
        layer.addSublayer(&clip_container);
        clip_container
    } else {
        layer.clone()
    };

    render_group(&container, image.root(), &root_world, combined_opacity, image_cache);
}

fn render_node(
    layer: &Retained<CALayer>,
    node: &VectorNode,
    world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    match node {
        VectorNode::Group(child) => render_group(layer, child, world, opacity, image_cache),
        VectorNode::Path(path_node) => render_path_node(layer, path_node, world, opacity, image_cache),
        VectorNode::RasterImage(raster_node) => {
            render_raster_node(layer, raster_node, world, opacity, image_cache)
        }
    }
}

/// Renders `group`'s own children into `target`, honoring `group.filters` — the "content" half of
/// [`render_group`]'s two-stage pipeline (content, then clip/mask/opacity/blend applied to it).
fn render_group_content(
    target: &Retained<CALayer>,
    group: &VectorGroup,
    world: &AffineTransform,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    if group.filters.is_empty() {
        for child in group.children.iter() {
            render_node(target, child, world, 1.0, image_cache);
        }
    } else {
        render_filtered_content(target, &group.children, &group.filters, world, image_cache);
    }
}

/// `VectorGroup` traversal. Composition order matches SVG's own: render content (children,
/// through any `filters`) → clip-path → mask → opacity → blend-mode, then hand the fully
/// composited result to the caller as one sublayer of `layer`.
fn render_group(
    layer: &Retained<CALayer>,
    group: &VectorGroup,
    parent_world: &AffineTransform,
    parent_opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let world = parent_world.concat(&group.transform);

    let wrapper = CALayer::new();
    wrapper.setName(Some(&NSString::from_str("elwindui-paint")));
    wrapper.setFrame(layer.bounds());

    // clip-path gets its own inner layer so its mask slot doesn't collide with the SVG `mask`'s
    // own mask slot on `wrapper` below — `CALayer` only has one `.mask` property each.
    let content_target = if group.clip_path.is_some() {
        let content = CALayer::new();
        content.setName(Some(&NSString::from_str("elwindui-paint")));
        content.setFrame(layer.bounds());
        wrapper.addSublayer(&content);
        content
    } else {
        wrapper.clone()
    };

    render_group_content(&content_target, group, &world, image_cache);

    if let Some(clip_path) = &group.clip_path {
        let path = clip_path.to_path().transformed(world);
        let mask = clip_mask_layer(
            &AffineTransform::identity(),
            &Clip::Path {
                path,
                rule: FillRule::NonZero,
            },
        );
        unsafe { content_target.setMask(Some(&mask)) };
    }

    if let Some(mask) = &group.mask {
        if let Some(mask_layer) = build_mask_layer(mask, &world, image_cache) {
            unsafe { wrapper.setMask(Some(&mask_layer)) };
        }
    }

    wrapper.setOpacity(parent_opacity * group.opacity);
    apply_blend_mode(&wrapper, group.blend_mode);

    layer.addSublayer(&wrapper);
}

fn apply_blend_mode(layer: &CALayer, mode: VectorBlendMode) {
    let Some(name) = ci_blend_mode_filter_name(mode) else {
        return;
    };
    let filter = unsafe { CIFilter::filterWithName(&NSString::from_str(name)) };
    if let Some(filter) = filter {
        unsafe { layer.setCompositingFilter(Some(filter.as_ref() as &AnyObject)) };
    } else {
        report_unsupported("group blend mode (CIFilter unavailable)");
    }
}

fn ci_blend_mode_filter_name(mode: VectorBlendMode) -> Option<&'static str> {
    match mode {
        VectorBlendMode::Normal => None,
        VectorBlendMode::Multiply => Some("CIMultiplyBlendMode"),
        VectorBlendMode::Screen => Some("CIScreenBlendMode"),
        VectorBlendMode::Overlay => Some("CIOverlayBlendMode"),
        VectorBlendMode::Darken => Some("CIDarkenBlendMode"),
        VectorBlendMode::Lighten => Some("CILightenBlendMode"),
        VectorBlendMode::ColorDodge => Some("CIColorDodgeBlendMode"),
        VectorBlendMode::ColorBurn => Some("CIColorBurnBlendMode"),
        VectorBlendMode::HardLight => Some("CIHardLightBlendMode"),
        VectorBlendMode::SoftLight => Some("CISoftLightBlendMode"),
        VectorBlendMode::Difference => Some("CIDifferenceBlendMode"),
        VectorBlendMode::Exclusion => Some("CIExclusionBlendMode"),
        VectorBlendMode::Hue => Some("CIHueBlendMode"),
        VectorBlendMode::Saturation => Some("CISaturationBlendMode"),
        VectorBlendMode::Color => Some("CIColorBlendMode"),
        VectorBlendMode::Luminosity => Some("CILuminosityBlendMode"),
    }
}

// ---------------------------------------------------------------------------------------------
// Offscreen rasterization (shared by mask / pattern-tile / filter content)
// ---------------------------------------------------------------------------------------------

/// Renders `children` (already in `local_rect`'s own coordinate space) into a fresh, appropriately
/// sized `CALayer` tree and rasterizes it to premultiplied top-down RGBA8 pixels. Returns `None`
/// for a degenerate or pathologically large region rather than allocating unboundedly.
fn rasterize_nodes_to_pixels(
    children: &[VectorNode],
    local_rect: Rect,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) -> Option<(Vec<u8>, usize, usize)> {
    let width = local_rect.width.ceil().max(1.0) as usize;
    let height = local_rect.height.ceil().max(1.0) as usize;
    if width == 0 || height == 0 || width > MAX_OFFSCREEN_DIMENSION || height > MAX_OFFSCREEN_DIMENSION {
        return None;
    }

    let root = CALayer::new();
    root.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width as f64, height as f64)));
    let local_to_pixel = AffineTransform::translation(-local_rect.x, -local_rect.y);
    for child in children {
        render_node(&root, child, &local_to_pixel, 1.0, image_cache);
    }

    rasterize_calayer_to_pixels(&root, width, height)
}

fn rasterize_calayer_to_pixels(
    root: &Retained<CALayer>,
    width: usize,
    height: usize,
) -> Option<(Vec<u8>, usize, usize)> {
    let bytes_per_row = width * 4;
    let mut pixels = vec![0u8; bytes_per_row * height];
    let color_space = CGColorSpace::new_device_rgb()?;
    #[allow(deprecated)]
    let bitmap_info =
        CGImageAlphaInfo::PremultipliedLast.0 | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
    let ctx = unsafe {
        objc2_core_graphics::CGBitmapContextCreate(
            pixels.as_mut_ptr() as *mut _,
            width,
            height,
            8,
            bytes_per_row,
            Some(&color_space),
            bitmap_info,
        )
    }?;
    // Cancels `CALayer.renderInContext:`'s own Y-flip (see `inner.rs::golden_tests`'s doc comment
    // on the same behavior) so the resulting buffer is ordinary top-down RGBA8.
    objc2_core_graphics::CGContext::translate_ctm(Some(&ctx), 0.0, height as f64);
    objc2_core_graphics::CGContext::scale_ctm(Some(&ctx), 1.0, -1.0);
    root.renderInContext(&ctx);
    Some((pixels, width, height))
}

/// Releases the boxed pixel buffer `CGDataProvider::with_data` was given ownership of — local
/// twin of `inner.rs`'s own private `release_boxed_pixels` (not reused directly since that
/// function isn't `pub(crate)` and duplicating one tiny `extern "C-unwind"` callback is simpler
/// than widening `inner.rs`'s own visibility further for it).
unsafe extern "C-unwind" fn release_boxed_pixels(
    _info: *mut std::ffi::c_void,
    data: std::ptr::NonNull<std::ffi::c_void>,
    size: usize,
) {
    unsafe {
        drop(Vec::from_raw_parts(data.as_ptr() as *mut u8, size, size));
    }
}

fn pixels_to_cgimage(pixels: Vec<u8>, width: usize, height: usize) -> Option<CFRetained<CGImage>> {
    let bytes_per_row = width * 4;
    let mut owned = pixels.into_boxed_slice();
    let len = owned.len();
    let ptr = owned.as_mut_ptr();
    std::mem::forget(owned);
    let provider = unsafe {
        CGDataProvider::with_data(std::ptr::null_mut(), ptr as *const _, len, Some(release_boxed_pixels))
    }?;
    let color_space = CGColorSpace::new_device_rgb()?;
    #[allow(deprecated)]
    let alpha_info = CGImageAlphaInfo::PremultipliedLast;
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            bytes_per_row,
            Some(&color_space),
            objc2_core_graphics::CGBitmapInfo(alpha_info.0 as _),
            Some(&provider),
            std::ptr::null(),
            false,
            objc2_core_graphics::CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// Places an already-rendered `cgimage` (covering `local_rect` in its subject's own local
/// coordinate space) at `world`'s image of that rect — the same `position`/`bounds`/
/// `affineTransform` technique `build_image_container_layer` uses for ordinary `DrawImage`, shared
/// here by pattern tiles, filter results, and mask content so each rotates/scales correctly under
/// an arbitrary `world` instead of being restricted to pure translation.
fn place_offscreen_image(
    cgimage: &CFRetained<CGImage>,
    local_rect: Rect,
    world: &AffineTransform,
    opacity: f32,
) -> Retained<CALayer> {
    let layer = CALayer::new();
    layer.setName(Some(&NSString::from_str("elwindui-paint")));
    layer.setBounds(CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(local_rect.width as f64, local_rect.height as f64),
    ));
    let center = world.transform_point(Point {
        x: local_rect.x + local_rect.width / 2.0,
        y: local_rect.y + local_rect.height / 2.0,
    });
    layer.setPosition(CGPoint::new(center.x as f64, center.y as f64));
    layer.setAffineTransform(CGAffineTransform {
        a: world.m11 as f64,
        b: world.m12 as f64,
        c: world.m21 as f64,
        d: world.m22 as f64,
        tx: 0.0,
        ty: 0.0,
    });
    unsafe { layer.setContents(Some(cgimage.as_ref() as &AnyObject)) };
    layer.setOpacity(opacity);
    layer
}

// ---------------------------------------------------------------------------------------------
// Mask
// ---------------------------------------------------------------------------------------------

fn build_mask_layer(
    mask: &VectorMask,
    world: &AffineTransform,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) -> Option<Retained<CALayer>> {
    let local_rect = mask.bounds;
    let mask_world = world.concat(&mask.transform);
    let (mut pixels, width, height) =
        rasterize_nodes_to_pixels(std::slice::from_ref(&VectorNode::Group(mask.root.clone())), local_rect, image_cache)?;

    if mask.mask_type == VectorMaskType::Luminance {
        // Premultiplied R/G/B already carry a factor of the original alpha, so the standard sRGB
        // luma weights applied directly to them equal `alpha * luminance(straight rgb)` — exactly
        // the SVG luminance-mask formula — with no separate unpremultiply/premultiply round trip
        // needed.
        for px in pixels.chunks_exact_mut(4) {
            let luminance = 0.2125 * px[0] as f32 + 0.7154 * px[1] as f32 + 0.0721 * px[2] as f32;
            px[3] = luminance.round().clamp(0.0, 255.0) as u8;
        }
    }

    if let Some(nested) = &mask.nested {
        if let Some(nested_layer_pixels) = rasterize_nodes_to_pixels(
            std::slice::from_ref(&VectorNode::Group(nested.root.clone())),
            nested.bounds,
            image_cache,
        ) {
            let (mut nested_pixels, nested_w, nested_h) = nested_layer_pixels;
            if nested.mask_type == VectorMaskType::Luminance {
                for px in nested_pixels.chunks_exact_mut(4) {
                    let luminance = 0.2125 * px[0] as f32 + 0.7154 * px[1] as f32 + 0.0721 * px[2] as f32;
                    px[3] = luminance.round().clamp(0.0, 255.0) as u8;
                }
            }
            // Nested masks intersect (multiply alphas) — only well-defined when both cover the
            // same pixel grid, which holds when both masks share their referencing element's
            // bounds (the common case); a size mismatch degrades to the outer mask alone rather
            // than attempting a misaligned resample.
            if nested_w == width && nested_h == height {
                for (outer, inner) in pixels.chunks_exact_mut(4).zip(nested_pixels.chunks_exact(4)) {
                    outer[3] = ((outer[3] as u32 * inner[3] as u32) / 255) as u8;
                }
            } else {
                report_unsupported("nested mask with mismatched bounds (outer mask only applied)");
            }
        }
    }

    let cgimage = pixels_to_cgimage(pixels, width, height)?;
    Some(place_offscreen_image(&cgimage, local_rect, &mask_world, 1.0))
}

// ---------------------------------------------------------------------------------------------
// Raster leaf
// ---------------------------------------------------------------------------------------------

fn render_raster_node(
    layer: &Retained<CALayer>,
    node: &VectorRasterNode,
    parent_world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let world = parent_world.concat(&node.transform);
    let Some(resolved) = resolve_cgimage(&node.image, image_cache) else {
        return;
    };
    let options = elwindui_core::graphics::ImageDrawOptions {
        opacity: node.opacity,
        sampling: node.sampling,
        fit: elwindui_core::graphics::ImageFit::Fill,
        alignment_x: elwindui_core::graphics::AlignmentX::Center,
        alignment_y: elwindui_core::graphics::AlignmentY::Center,
        repeat: elwindui_core::graphics::TileMode::None,
    };
    if let Some(container) =
        build_image_container_layer(&resolved, node.rect, None, &options, &world, opacity)
    {
        layer.addSublayer(&container);
    }
}

// ---------------------------------------------------------------------------------------------
// Path fill/stroke (solid, gradient, pattern)
// ---------------------------------------------------------------------------------------------

fn render_path_node(
    layer: &Retained<CALayer>,
    node: &VectorPathNode,
    parent_world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    if !node.visibility || (node.fill.is_none() && node.stroke.is_none()) {
        return;
    }
    let world = parent_world.concat(&node.transform);
    let local_bounds = node.path.bounds();

    let fill_is_simple = matches!(
        node.fill.as_ref().map(|f| &f.paint),
        None | Some(VectorPaint::Brush(Brush::Solid(_)))
    );
    let stroke_is_simple = matches!(
        node.stroke.as_ref().map(|s| &s.paint),
        None | Some(VectorPaint::Brush(Brush::Solid(_)))
    );

    if fill_is_simple && stroke_is_simple {
        let cg_path = path_to_cgpath(&world, &node.path);
        add_shape_layer(
            layer,
            &cg_path,
            solid_fill_brush(node.fill.as_ref()).as_ref(),
            solid_stroke(node.stroke.as_ref()).as_ref().map(|(b, s)| (b, s)),
            opacity,
            local_bounds,
        );
        return;
    }

    if let Some(fill) = &node.fill {
        render_fill(layer, &node.path, &world, local_bounds, fill, opacity, image_cache);
    }
    if let Some(stroke) = &node.stroke {
        render_stroke(layer, &node.path, &world, local_bounds, stroke, opacity, image_cache);
    }
}

fn solid_fill_brush(fill: Option<&VectorFill>) -> Option<Brush> {
    match fill {
        Some(VectorFill {
            paint: VectorPaint::Brush(Brush::Solid(color)),
            opacity,
            ..
        }) => Some(Brush::Solid(with_opacity(*color, *opacity))),
        _ => None,
    }
}

fn solid_stroke(stroke: Option<&VectorStroke>) -> Option<(Brush, StrokeStyle)> {
    match stroke {
        Some(VectorStroke {
            paint: VectorPaint::Brush(Brush::Solid(color)),
            opacity,
            style,
        }) => Some((Brush::Solid(with_opacity(*color, *opacity)), style.clone())),
        _ => None,
    }
}

fn with_opacity(color: Color, opacity: f32) -> Color {
    Color::rgba(
        color.r,
        color.g,
        color.b,
        (color.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_fill(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    local_bounds: Rect,
    fill: &VectorFill,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    match &fill.paint {
        VectorPaint::Brush(Brush::Solid(color)) => {
            let cg_path = path_to_cgpath(world, path);
            add_shape_layer(
                layer,
                &cg_path,
                Some(&Brush::Solid(with_opacity(*color, fill.opacity))),
                None,
                opacity,
                local_bounds,
            );
        }
        VectorPaint::Brush(brush @ (Brush::LinearGradient(_) | Brush::RadialGradient(_))) => {
            add_gradient_shape_layer(
                layer, path, world, local_bounds, brush, fill.opacity, fill.rule, opacity,
            );
        }
        VectorPaint::Brush(Brush::Image(_)) => {
            report_unsupported("image-brush path fill");
        }
        VectorPaint::Pattern(pattern) => {
            add_pattern_shape_layer(
                layer, path, world, pattern, fill.rule, fill.opacity, opacity, image_cache,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_stroke(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    local_bounds: Rect,
    stroke: &VectorStroke,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let _ = image_cache;
    // Gradient/pattern strokes render via their first available color, matching `inner.rs`'s own
    // ordinary `apply_stroke` gradient handling — stroking a gradient/pattern along an arbitrary
    // path outline (rather than filling one) needs the outline turned into fill geometry first,
    // which neither this backend nor the non-SVG `StrokePath` command does today.
    let brush = match &stroke.paint {
        VectorPaint::Brush(b) => b.clone(),
        VectorPaint::Pattern(_) => {
            report_unsupported("pattern stroke (rendered as solid fallback)");
            Brush::Solid(Color::BLACK)
        }
    };
    let cg_path = path_to_cgpath(world, path);
    let shape_layer = CAShapeLayer::new();
    shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
    shape_layer.setPath(Some(&cg_path));
    shape_layer.setFillColor(None);
    apply_stroke(&shape_layer, &brush, &stroke.style, local_bounds);
    shape_layer.setOpacity(opacity * stroke.opacity);
    let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
    layer.addSublayer(&shape_layer);
}

/// Gradient-on-arbitrary-path fill: a masked `CAGradientLayer` like `inner.rs`'s own
/// `try_add_gradient_fill_layer`, but placed via `position`/`bounds`/`affineTransform` (the same
/// technique `build_image_container_layer` already uses for `DrawImage`) instead of `setFrame`,
/// so it isn't restricted to a pure-translation `world` — SVG content is rotated/scaled far more
/// often than not (viewBox scaling alone applies to nearly every real SVG), so that restriction
/// would make gradient fills fall back to a flat color for almost every real document.
#[allow(clippy::too_many_arguments)]
fn add_gradient_shape_layer(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    local_bounds: Rect,
    brush: &Brush,
    paint_opacity: f32,
    fill_rule: FillRule,
    opacity: f32,
) {
    let gradient_layer = CAGradientLayer::new();
    gradient_layer.setName(Some(&NSString::from_str("elwindui-paint")));
    let ca_layer: &CALayer = &gradient_layer;
    ca_layer.setBounds(CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(local_bounds.width as f64, local_bounds.height as f64),
    ));
    let center_absolute = world.transform_point(Point {
        x: local_bounds.x + local_bounds.width / 2.0,
        y: local_bounds.y + local_bounds.height / 2.0,
    });
    ca_layer.setPosition(CGPoint::new(center_absolute.x as f64, center_absolute.y as f64));
    ca_layer.setAffineTransform(CGAffineTransform {
        a: world.m11 as f64,
        b: world.m12 as f64,
        c: world.m21 as f64,
        d: world.m22 as f64,
        tx: 0.0,
        ty: 0.0,
    });
    ca_layer.setOpacity(opacity * paint_opacity);

    let stops: &[GradientStop] = match brush {
        Brush::LinearGradient(g) => {
            unsafe { gradient_layer.setType(kCAGradientLayerAxial) };
            gradient_layer.setStartPoint(gradient_unit_point(g.start, g.mapping, local_bounds));
            gradient_layer.setEndPoint(gradient_unit_point(g.end, g.mapping, local_bounds));
            &g.stops
        }
        Brush::RadialGradient(g) => {
            unsafe { gradient_layer.setType(kCAGradientLayerRadial) };
            let center = gradient_unit_point(g.center, g.mapping, local_bounds);
            gradient_layer.setStartPoint(center);
            let (rx, ry) = match g.mapping {
                elwindui_core::graphics::BrushMappingMode::RelativeToBounds => (g.radius_x, g.radius_y),
                elwindui_core::graphics::BrushMappingMode::Absolute => (
                    g.radius_x / local_bounds.width.max(1e-6),
                    g.radius_y / local_bounds.height.max(1e-6),
                ),
            };
            gradient_layer.setEndPoint(CGPoint::new(center.x + rx as f64, center.y + ry as f64));
            &g.stops
        }
        _ => return,
    };
    if stops.is_empty() {
        return;
    }

    let colors: Vec<CFRetained<objc2_core_graphics::CGColor>> =
        stops.iter().map(|s| color_to_cgcolor(s.color)).collect();
    let color_refs: Vec<&AnyObject> = colors
        .iter()
        .map(|c| c.as_ref() as &objc2_core_foundation::CFType)
        .map(|c| c.as_ref())
        .collect();
    unsafe { gradient_layer.setColors(Some(&objc2_foundation::NSArray::from_slice(&color_refs))) };
    let locations: Vec<Retained<NSNumber>> =
        stops.iter().map(|s| NSNumber::new_f64(s.offset as f64)).collect();
    let location_refs: Vec<&NSNumber> = locations.iter().map(|n| n.as_ref()).collect();
    gradient_layer.setLocations(Some(&objc2_foundation::NSArray::from_slice(&location_refs)));

    // Mask expressed in the gradient layer's own local (`bounds`-relative) space — same reasoning
    // as `try_add_gradient_fill_layer`'s own mask, built from the path's *local* geometry directly
    // (an arbitrary `VectorPathNode` has no simpler rect/ellipse primitive to fall back to).
    let mask_translate = AffineTransform::translation(-local_bounds.x, -local_bounds.y);
    let mask_path = path_to_cgpath(&mask_translate, path);
    let mask_layer = CAShapeLayer::new();
    mask_layer.setPath(Some(&mask_path));
    mask_layer.setFillRule(match fill_rule {
        FillRule::NonZero => unsafe { kCAFillRuleNonZero },
        FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
    });
    mask_layer.setFillColor(Some(&color_to_cgcolor(Color::BLACK)));
    let mask_layer: Retained<CALayer> = Retained::into_super(mask_layer);
    unsafe { ca_layer.setMask(Some(&mask_layer)) };

    let gradient_layer: Retained<CALayer> = Retained::into_super(gradient_layer);
    layer.addSublayer(&gradient_layer);
}

/// Single-tile pattern fill: renders `pattern.root` once into an offscreen image sized to
/// `pattern.tile_rect`, then places and masks it exactly like the gradient case above. This does
/// not repeat the tile across the fill region (a real `CGPatternCallbacks`-based infinite tiling
/// is real future work) — content renders once at its natural position rather than being dropped,
/// which is the closer approximation for the common case of a pattern tile that already covers (or
/// exceeds) its filled shape.
#[allow(clippy::too_many_arguments)]
fn add_pattern_shape_layer(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    pattern: &VectorPattern,
    fill_rule: FillRule,
    paint_opacity: f32,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let Some((pixels, w, h)) = rasterize_nodes_to_pixels(
        std::slice::from_ref(&VectorNode::Group(pattern.root.clone())),
        pattern.tile_rect,
        image_cache,
    ) else {
        report_unsupported("pattern fill (offscreen render failed)");
        return;
    };
    let Some(tile_cgimage) = pixels_to_cgimage(pixels, w, h) else {
        return;
    };

    let tile_world = world.concat(&pattern.transform);
    let image_layer = place_offscreen_image(&tile_cgimage, pattern.tile_rect, &tile_world, opacity * paint_opacity);

    let mask_path = path_to_cgpath(world, path);
    let mask_layer = CAShapeLayer::new();
    mask_layer.setPath(Some(&mask_path));
    mask_layer.setFillRule(match fill_rule {
        FillRule::NonZero => unsafe { kCAFillRuleNonZero },
        FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
    });
    mask_layer.setFillColor(Some(&color_to_cgcolor(Color::BLACK)));
    let mask_layer: Retained<CALayer> = Retained::into_super(mask_layer);
    unsafe { image_layer.setMask(Some(&mask_layer)) };

    layer.addSublayer(&image_layer);
}

// ---------------------------------------------------------------------------------------------
// Filter graph (Core Image)
// ---------------------------------------------------------------------------------------------

fn filters_bounds(filters: &[VectorFilter]) -> Option<Rect> {
    filters
        .iter()
        .map(|f| f.bounds)
        .reduce(|a, b| union_rect(a, b))
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

fn render_filtered_content(
    target: &Retained<CALayer>,
    children: &[VectorNode],
    filters: &[VectorFilter],
    world: &AffineTransform,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) {
    let Some(local_rect) = filters_bounds(filters) else {
        for child in children {
            render_node(target, child, world, 1.0, image_cache);
        }
        return;
    };
    let Some((pixels, width, height)) = rasterize_nodes_to_pixels(children, local_rect, image_cache) else {
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
            let output = apply_filter_primitive(primitive, &current, &source_ci, &results, local_rect);
            let output = output.unwrap_or_else(|| results.last().cloned().unwrap_or_else(|| current.clone()));
            results.push(output);
        }
        if let Some(last) = results.last() {
            current = last.clone();
        }
    }

    let ci_context = unsafe { CIContext::context() };
    let render_rect = CGRect::new(
        CGPoint::new(local_rect.x as f64, local_rect.y as f64),
        CGSize::new(local_rect.width as f64, local_rect.height as f64),
    );
    let Some(result_cgimage) = (unsafe { ci_context.createCGImage_fromRect(&current, render_rect) }) else {
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
fn retained_to_cf_cgimage(image: Retained<CGImage>) -> CFRetained<CGImage> {
    let ptr = std::ptr::NonNull::new(Retained::into_raw(image)).expect("Retained is never null");
    unsafe { CFRetained::from_raw(ptr) }
}

fn resolve_filter_input(
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

fn ci_dict(pairs: &[(&str, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| k.as_ref()).collect();
    let values: Vec<&AnyObject> = pairs.iter().map(|(_, v)| *v).collect();
    NSDictionary::from_slices(&key_refs, &values)
}

fn ci_vector4(x: f32, y: f32, z: f32, w: f32) -> Retained<CIVector> {
    unsafe { CIVector::vectorWithX_Y_Z_W(x as f64, y as f64, z as f64, w as f64) }
}

fn apply_filter_primitive(
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
                    let params = ci_dict(&[("inputBackgroundImage", input2.as_ref() as &AnyObject)]);
                    Some(unsafe {
                        input1.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params)
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
            Some(unsafe { input.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params) })
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
            report_unsupported("feTile filter primitive (input passed through)");
            resolve_filter_input(&fe.input, source_graphic, results)
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

fn composite(
    input1: &Retained<CIImage>,
    input2: &Retained<CIImage>,
    operator: elwindui_core::graphics::VectorCompositeOperator,
) -> Option<Retained<CIImage>> {
    use elwindui_core::graphics::VectorCompositeOperator;
    let name = match operator {
        VectorCompositeOperator::Over => "CISourceOverCompositing",
        VectorCompositeOperator::In => "CISourceInCompositing",
        VectorCompositeOperator::Out => "CISourceOutCompositing",
        VectorCompositeOperator::Atop => "CISourceAtopCompositing",
        VectorCompositeOperator::Xor | VectorCompositeOperator::Arithmetic { .. } => {
            report_unsupported("feComposite Xor/Arithmetic operator (treated as Over)");
            "CISourceOverCompositing"
        }
    };
    let params = ci_dict(&[("inputBackgroundImage", input2.as_ref() as &AnyObject)]);
    Some(unsafe { input1.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params) })
}

fn apply_color_matrix(
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
    let bias = unsafe { CIVector::vectorWithX_Y_Z_W(matrix[4] as f64, matrix[9] as f64, matrix[14] as f64, matrix[19] as f64) };
    let params = ci_dict(&[
        ("inputRVector", r.as_ref() as &AnyObject),
        ("inputGVector", g.as_ref() as &AnyObject),
        ("inputBVector", b.as_ref() as &AnyObject),
        ("inputAVector", a.as_ref() as &AnyObject),
        ("inputBiasVector", bias.as_ref() as &AnyObject),
    ]);
    Some(unsafe {
        input.imageByApplyingFilter_withInputParameters(&NSString::from_str("CIColorMatrix"), &params)
    })
}

const LUMINANCE_TO_ALPHA_MATRIX: [f32; 20] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2125, 0.7154, 0.0721,
    0.0, 0.0,
];

/// Standard SVG `feColorMatrix type="saturate"` matrix (SVG 1.1 §15.10).
fn saturate_matrix(s: f32) -> [f32; 20] {
    [
        0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0.0, 0.0,
        0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0.0, 0.0,
        0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ]
}

/// Standard SVG `feColorMatrix type="hueRotate"` matrix (SVG 1.1 §15.10).
fn hue_rotate_matrix(degrees: f32) -> [f32; 20] {
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
        0.0, 0.0, 0.0, 1.0, 0.0,
    ]
}

fn apply_convolve_matrix(
    input: &Retained<CIImage>,
    fe: &elwindui_core::graphics::VectorConvolveMatrixFilter,
) -> Option<Retained<CIImage>> {
    let name = match (fe.order_x, fe.order_y) {
        (3, 3) => "CIConvolution3X3",
        (5, 5) => "CIConvolution5X5",
        _ => {
            report_unsupported("feConvolveMatrix with an order other than 3x3/5x5 (input passed through)");
            return Some(input.clone());
        }
    };
    let count = fe.kernel.len();
    let mut values: Vec<f64> = fe.kernel.iter().map(|&v| v as f64 / fe.divisor.max(1e-6) as f64).collect();
    values.reverse(); // SVG kernels are specified in reading order; Core Image expects the flipped orientation.
    let values_ptr = std::ptr::NonNull::new(values.as_mut_ptr()).expect("non-empty kernel");
    let weights = unsafe { CIVector::vectorWithValues_count(values_ptr, count) };
    let bias_num = NSNumber::new_f64(fe.bias as f64);
    let params = ci_dict(&[
        ("inputWeights", weights.as_ref() as &AnyObject),
        ("inputBias", bias_num.as_ref() as &AnyObject),
    ]);
    Some(unsafe { input.imageByApplyingFilter_withInputParameters(&NSString::from_str(name), &params) })
}

fn apply_drop_shadow(
    input: &Retained<CIImage>,
    fe: &elwindui_core::graphics::VectorDropShadowFilter,
) -> Option<Retained<CIImage>> {
    // feDropShadow ≈ feGaussianBlur → feOffset → flood(color) composited under the original
    // (SVG 1.1 §15.15's own "equivalent to" definition), built directly from `CIImage` steps
    // rather than a single named CIFilter (Core Image has no exact `feDropShadow` counterpart).
    let alpha_matrix = ci_dict(&[
        ("inputRVector", ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputGVector", ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputBVector", ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputAVector", ci_vector4(0.0, 0.0, 0.0, 1.0).as_ref() as &AnyObject),
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
        input.imageByApplyingFilter_withInputParameters(&NSString::from_str("CIColorMatrix"), &alpha_matrix)
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
        ("inputRVector", ci_vector4(1.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputGVector", ci_vector4(0.0, 1.0, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputBVector", ci_vector4(0.0, 0.0, 1.0, 0.0).as_ref() as &AnyObject),
        ("inputAVector", ci_vector4(0.0, 0.0, 0.0, fe.opacity).as_ref() as &AnyObject),
        ("inputBiasVector", ci_vector4(0.0, 0.0, 0.0, 0.0).as_ref() as &AnyObject),
    ]);
    let shadow = unsafe {
        offset.imageByApplyingFilter_withInputParameters(&NSString::from_str("CIColorMatrix"), &opacity_matrix)
    };
    Some(unsafe { input.imageByCompositingOverImage(&shadow) })
}

fn apply_component_transfer(
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
        report_unsupported("feComponentTransfer with a Table/Discrete/Gamma function (input passed through)");
        return Some(input.clone());
    };
    let params = ci_dict(&[
        ("inputRVector", ci_vector4(rs, 0.0, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputGVector", ci_vector4(0.0, gs, 0.0, 0.0).as_ref() as &AnyObject),
        ("inputBVector", ci_vector4(0.0, 0.0, bs, 0.0).as_ref() as &AnyObject),
        ("inputAVector", ci_vector4(0.0, 0.0, 0.0, as_).as_ref() as &AnyObject),
        ("inputBiasVector", ci_vector4(ri, gi, bi, ai).as_ref() as &AnyObject),
    ]);
    Some(unsafe {
        input.imageByApplyingFilter_withInputParameters(&NSString::from_str("CIColorMatrix"), &params)
    })
}
