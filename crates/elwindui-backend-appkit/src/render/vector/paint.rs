//! Vector leaves and their paint: path/raster nodes, plus the fill and stroke work
//! (solid, gradient, pattern) and the mask layers a `clip-path`/`mask` needs.

use crate::render::{
    add_shape_layer, apply_stroke, build_image_container_layer, color_to_cgcolor,
    gradient_unit_point, path_to_cgpath, resolve_cgimage,
};
use elwindui_core::base::{AffineTransform, Point, Rect};
use elwindui_core::graphics::{
    Brush, Color, FillRule, GradientStop, Path, StrokeStyle, VectorFill, VectorMask,
    VectorMaskType, VectorNode, VectorPaint, VectorPathNode, VectorPattern, VectorRasterNode,
    VectorStroke,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSNumber, NSString};
use objc2_quartz_core::{
    CAGradientLayer, CALayer, CAShapeLayer, kCAFillRuleEvenOdd, kCAFillRuleNonZero,
    kCAGradientLayerAxial, kCAGradientLayerRadial,
};
use std::collections::HashMap;

use super::raster::*;
use super::*;

pub(crate) fn build_mask_layer(
    mask: &VectorMask,
    world: &AffineTransform,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) -> Option<Retained<CALayer>> {
    let local_rect = mask.bounds;
    let mask_world = world.concat(&mask.transform);
    let (mut pixels, width, height) = rasterize_nodes_to_pixels(
        std::slice::from_ref(&VectorNode::Group(mask.root.clone())),
        local_rect,
        image_cache,
    )?;

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
                    let luminance =
                        0.2125 * px[0] as f32 + 0.7154 * px[1] as f32 + 0.0721 * px[2] as f32;
                    px[3] = luminance.round().clamp(0.0, 255.0) as u8;
                }
            }
            // Nested masks intersect (multiply alphas) — only well-defined when both cover the
            // same pixel grid, which holds when both masks share their referencing element's
            // bounds (the common case); a size mismatch degrades to the outer mask alone rather
            // than attempting a misaligned resample.
            if nested_w == width && nested_h == height {
                for (outer, inner) in pixels
                    .chunks_exact_mut(4)
                    .zip(nested_pixels.chunks_exact(4))
                {
                    outer[3] = ((outer[3] as u32 * inner[3] as u32) / 255) as u8;
                }
            } else {
                report_unsupported("nested mask with mismatched bounds (outer mask only applied)");
            }
        }
    }

    let cgimage = pixels_to_cgimage(pixels, width, height)?;
    Some(place_offscreen_image(
        &cgimage,
        local_rect,
        &mask_world,
        1.0,
    ))
}

pub(crate) fn render_raster_node(
    layer: &Retained<CALayer>,
    node: &VectorRasterNode,
    parent_world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
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

pub(crate) fn render_path_node(
    layer: &Retained<CALayer>,
    node: &VectorPathNode,
    parent_world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
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
            solid_stroke(node.stroke.as_ref())
                .as_ref()
                .map(|(b, s)| (b, s)),
            opacity,
            local_bounds,
        );
        return;
    }

    if let Some(fill) = &node.fill {
        render_fill(
            layer,
            &node.path,
            &world,
            local_bounds,
            fill,
            opacity,
            image_cache,
        );
    }
    if let Some(stroke) = &node.stroke {
        render_stroke(
            layer,
            &node.path,
            &world,
            local_bounds,
            stroke,
            opacity,
            image_cache,
        );
    }
}

pub(crate) fn solid_fill_brush(fill: Option<&VectorFill>) -> Option<Brush> {
    match fill {
        Some(VectorFill {
            paint: VectorPaint::Brush(Brush::Solid(color)),
            opacity,
            ..
        }) => Some(Brush::Solid(with_opacity(*color, *opacity))),
        _ => None,
    }
}

pub(crate) fn solid_stroke(stroke: Option<&VectorStroke>) -> Option<(Brush, StrokeStyle)> {
    match stroke {
        Some(VectorStroke {
            paint: VectorPaint::Brush(Brush::Solid(color)),
            opacity,
            style,
        }) => Some((Brush::Solid(with_opacity(*color, *opacity)), style.clone())),
        _ => None,
    }
}

pub(crate) fn with_opacity(color: Color, opacity: f32) -> Color {
    Color::rgba(
        color.r,
        color.g,
        color.b,
        (color.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_fill(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    local_bounds: Rect,
    fill: &VectorFill,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
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
                layer,
                path,
                world,
                local_bounds,
                brush,
                fill.opacity,
                fill.rule,
                opacity,
            );
        }
        VectorPaint::Brush(Brush::Image(_)) => {
            report_unsupported("image-brush path fill");
        }
        VectorPaint::Pattern(pattern) => {
            add_pattern_shape_layer(
                layer,
                path,
                world,
                local_bounds,
                pattern,
                fill.rule,
                fill.opacity,
                opacity,
                image_cache,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_stroke(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    local_bounds: Rect,
    stroke: &VectorStroke,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
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
pub(crate) fn add_gradient_shape_layer(
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
    ca_layer.setPosition(CGPoint::new(
        center_absolute.x as f64,
        center_absolute.y as f64,
    ));
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
                elwindui_core::graphics::BrushMappingMode::RelativeToBounds => {
                    (g.radius_x, g.radius_y)
                }
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
    let locations: Vec<Retained<NSNumber>> = stops
        .iter()
        .map(|s| NSNumber::new_f64(s.offset as f64))
        .collect();
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

/// Largest pattern tile grid extent allowed along either axis — same defensive cap
/// `add_tiled_image_layers` (`inner.rs`, the `ImageBrush` tile-fill counterpart this mirrors)
/// already applies, so a tiny `tile_rect` against a large fill region produces a bounded (if
/// visually truncated) tile count rather than an unbounded one.
pub(crate) const MAX_PATTERN_TILES_PER_AXIS: i32 = 64;

/// Repeating pattern fill: renders `pattern.root` once into an offscreen image sized to
/// `pattern.tile_rect`, then tiles that single rendered image as a grid of sibling `CALayer`s
/// covering `local_bounds` (the fill shape's own bounding box) — real infinite tiling, not a
/// single placement, following the same "render once, stamp many `CALayer`s at `position`/
/// `bounds`" strategy `add_tiled_image_layers` (`inner.rs`) already uses for `ImageBrush` tile
/// fills. Unlike that helper, the whole tile grid is wrapped in one parent layer that carries
/// `world`'s rotation/scale via `position`/`bounds`/`affineTransform` (`place_offscreen_image`'s
/// own technique), so — like this module's gradient fill — pattern tiling isn't restricted to a
/// pure-translation `world`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_pattern_shape_layer(
    layer: &Retained<CALayer>,
    path: &Path,
    world: &AffineTransform,
    local_bounds: Rect,
    pattern: &VectorPattern,
    fill_rule: FillRule,
    paint_opacity: f32,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) {
    let tile_rect = pattern.tile_rect;
    if tile_rect.width <= 0.0 || tile_rect.height <= 0.0 {
        report_unsupported("pattern fill (degenerate tile rect)");
        return;
    }

    let Some((pixels, w, h)) = rasterize_nodes_to_pixels(
        std::slice::from_ref(&VectorNode::Group(pattern.root.clone())),
        tile_rect,
        image_cache,
    ) else {
        report_unsupported("pattern fill (offscreen render failed)");
        return;
    };
    let Some(tile_cgimage) = pixels_to_cgimage(pixels, w, h) else {
        return;
    };

    // The tile grid cells (relative to `tile_rect`'s own declared origin, which need not align
    // with `local_bounds`'s own top-left) needed to cover the fill shape's bounding box, in both
    // directions.
    let start_col = ((local_bounds.x - tile_rect.x) / tile_rect.width).floor() as i32;
    let end_col = (((local_bounds.x + local_bounds.width - tile_rect.x) / tile_rect.width).ceil()
        as i32)
        .max(start_col + 1);
    let start_row = ((local_bounds.y - tile_rect.y) / tile_rect.height).floor() as i32;
    let end_row = (((local_bounds.y + local_bounds.height - tile_rect.y) / tile_rect.height).ceil()
        as i32)
        .max(start_row + 1);
    let start_col = start_col.max(end_col - MAX_PATTERN_TILES_PER_AXIS);
    let start_row = start_row.max(end_row - MAX_PATTERN_TILES_PER_AXIS);

    let grid_local = Rect {
        x: tile_rect.x + start_col as f32 * tile_rect.width,
        y: tile_rect.y + start_row as f32 * tile_rect.height,
        width: (end_col - start_col) as f32 * tile_rect.width,
        height: (end_row - start_row) as f32 * tile_rect.height,
    };

    let tile_world = world.concat(&pattern.transform);
    let wrapper = CALayer::new();
    wrapper.setName(Some(&NSString::from_str("elwindui-paint")));
    wrapper.setBounds(CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(grid_local.width as f64, grid_local.height as f64),
    ));
    let center = tile_world.transform_point(Point {
        x: grid_local.x + grid_local.width / 2.0,
        y: grid_local.y + grid_local.height / 2.0,
    });
    wrapper.setPosition(CGPoint::new(center.x as f64, center.y as f64));
    wrapper.setAffineTransform(CGAffineTransform {
        a: tile_world.m11 as f64,
        b: tile_world.m12 as f64,
        c: tile_world.m21 as f64,
        d: tile_world.m22 as f64,
        tx: 0.0,
        ty: 0.0,
    });
    wrapper.setOpacity(opacity * paint_opacity);

    // Every tile shares the one already-rendered `tile_cgimage` (a cheap `contents` pointer set
    // per sublayer, not a re-render) and only needs plain axis-aligned placement within `wrapper`,
    // since `wrapper` itself already carries `tile_world`'s rotation/scale.
    for row in start_row..end_row {
        for col in start_col..end_col {
            let tile_layer = CALayer::new();
            tile_layer.setBounds(CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(tile_rect.width as f64, tile_rect.height as f64),
            ));
            let local_x = tile_rect.x + col as f32 * tile_rect.width - grid_local.x;
            let local_y = tile_rect.y + row as f32 * tile_rect.height - grid_local.y;
            tile_layer.setPosition(CGPoint::new(
                (local_x + tile_rect.width / 2.0) as f64,
                (local_y + tile_rect.height / 2.0) as f64,
            ));
            unsafe { tile_layer.setContents(Some(tile_cgimage.as_ref() as &AnyObject)) };
            wrapper.addSublayer(&tile_layer);
        }
    }

    let mask_path = path_to_cgpath(world, path);
    let mask_layer = CAShapeLayer::new();
    mask_layer.setPath(Some(&mask_path));
    mask_layer.setFillRule(match fill_rule {
        FillRule::NonZero => unsafe { kCAFillRuleNonZero },
        FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
    });
    mask_layer.setFillColor(Some(&color_to_cgcolor(Color::BLACK)));
    let mask_layer: Retained<CALayer> = Retained::into_super(mask_layer);
    unsafe { wrapper.setMask(Some(&mask_layer)) };

    layer.addSublayer(&wrapper);
}
