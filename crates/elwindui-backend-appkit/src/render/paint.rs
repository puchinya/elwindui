//! Turning an `elwindui_core::graphics::Brush`/`StrokeStyle` into real `CALayer`s: flat fills
//! and strokes set directly on a `CAShapeLayer`, gradients and image brushes as masked sublayers
//! (Core Animation has no gradient/image *fill* on a shape layer, only whole-layer ones).

use super::geometry::*;
use super::image::*;
use super::layer::{add_sublayer_scaled, set_mask_scaled};
use super::path::*;
use objc2::rc::Retained;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGColor, CGImage, CGMutablePath};
use objc2_foundation::{NSArray, NSNumber, NSRect, NSString};
use objc2_quartz_core::{
    CAGradientLayer, CALayer, CAShapeLayer, kCAGradientLayerAxial, kCAGradientLayerRadial,
};
use std::collections::HashMap;

pub(crate) fn add_shape_layer(
    layer: &Retained<CALayer>,
    path: &CFRetained<CGMutablePath>,
    fill: Option<&elwindui_core::graphics::Brush>,
    stroke: Option<(
        &elwindui_core::graphics::Brush,
        &elwindui_core::graphics::StrokeStyle,
    )>,
    opacity: f32,
    bounds: elwindui_core::base::Rect,
) {
    let shape_layer = CAShapeLayer::new();
    shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
    shape_layer.setPath(Some(path));
    // `CAShapeLayer.fillColor` defaults to opaque black, not nil — `apply_fill`'s own `None` arm
    // (`setFillColor(None)`) must always run for a stroke-only shape, or the shape silently paints
    // as if solid-black-filled underneath its stroke.
    apply_fill(&shape_layer, fill, bounds);
    if let Some((brush, style)) = stroke {
        apply_stroke(&shape_layer, brush, style, bounds);
    }
    shape_layer.setOpacity(opacity);
    let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
    add_sublayer_scaled(layer, &shape_layer);
}

/// Which built-in shape a gradient's clip mask should take — mirrors `replay_paint_command`'s own
/// `FillRect`/`FillRoundedRect`/`FillEllipse` distinction, since a gradient fill needs a *local*
/// (mask-space, not canvas-absolute) path rebuilt for the mask layer (see
/// `try_add_gradient_fill_layer`'s own doc comment).
pub(crate) enum GradientMaskShape {
    RoundedRect(elwindui_core::base::CornerRadius),
    Ellipse,
}

/// Realizes a `LinearGradient`/`RadialGradient` fill as a real `CAGradientLayer` (rather than
/// `apply_fill`'s flat first-stop-color fallback), masked to `shape`'s outline. Returns `false`
/// (does nothing) for anything else — a solid brush, an `Image` brush (handled separately by
/// `try_add_image_fill_layer`), or a gradient under a non-translation `world` (rotated/scaled
/// group) — so the caller falls back to `add_shape_layer`'s existing solid-color path in those
/// cases.
///
/// The mask needs its own path expressed in the *gradient layer's local* coordinate space (origin
/// at the gradient layer's own top-left, not the canvas-absolute space `path_to_cgpath`/
/// `rounded_rect_cgpath` normally build in) — `CALayer.mask` interprets its mask layer exactly
/// like an ordinary sublayer of the layer being masked. `bounds` (already the pre-`world` local
/// rect every `replay_paint_command` call site already has on hand) rebuilt through
/// `AffineTransform::translation(-bounds.x, -bounds.y)` produces exactly that.
///
/// `GradientStop`'s own `offset` aside, `LinearGradientBrush`/`RadialGradientBrush::spread`
/// (`GradientSpreadMethod::{Pad,Reflect,Repeat}`) is never read here: `CAGradientLayer` has no
/// native notion of a spread method beyond clamping at the first/last stop (`Pad`'s own behavior),
/// so every brush renders as `Pad` regardless of what `spread` is actually set to — `Reflect`/
/// `Repeat` would need tiling multiple `CAGradientLayer`s across the fill region, not attempted
/// here (painter design doc §9.4 accepts a documented-but-unimplemented gap in the same spirit).
pub(crate) fn try_add_gradient_fill_layer(
    layer: &Retained<CALayer>,
    brush: &elwindui_core::graphics::Brush,
    bounds: elwindui_core::base::Rect,
    mask_shape: GradientMaskShape,
    world: &elwindui_core::base::AffineTransform,
    opacity: f32,
) -> bool {
    use elwindui_core::graphics::Brush;
    if !is_pure_translation(world) {
        return false;
    }
    let absolute_origin = world.transform_point(elwindui_core::base::Point {
        x: bounds.x,
        y: bounds.y,
    });
    let gradient_layer = CAGradientLayer::new();
    gradient_layer.setName(Some(&NSString::from_str("elwindui-paint")));
    let ca_layer: &CALayer = &gradient_layer;
    ca_layer.setFrame(NSRect::new(
        objc2_foundation::NSPoint::new(absolute_origin.x as f64, absolute_origin.y as f64),
        objc2_foundation::NSSize::new(bounds.width as f64, bounds.height as f64),
    ));
    ca_layer.setOpacity(opacity);

    let stops: &[elwindui_core::graphics::GradientStop] = match brush {
        Brush::LinearGradient(g) => {
            unsafe { gradient_layer.setType(kCAGradientLayerAxial) };
            gradient_layer.setStartPoint(gradient_unit_point(g.start, g.mapping, bounds));
            gradient_layer.setEndPoint(gradient_unit_point(g.end, g.mapping, bounds));
            &g.stops
        }
        Brush::RadialGradient(g) => {
            unsafe { gradient_layer.setType(kCAGradientLayerRadial) };
            let center = gradient_unit_point(g.center, g.mapping, bounds);
            gradient_layer.setStartPoint(center);
            let (rx, ry) = match g.mapping {
                elwindui_core::graphics::BrushMappingMode::RelativeToBounds => {
                    (g.radius_x, g.radius_y)
                }
                elwindui_core::graphics::BrushMappingMode::Absolute => (
                    g.radius_x / bounds.width.max(1e-6),
                    g.radius_y / bounds.height.max(1e-6),
                ),
            };
            // `CAGradientLayer`'s radial `endPoint` encodes *both* radii at once, as the vector
            // from `startPoint` (the center) to this point — an endpoint level with the center on
            // one axis (e.g. `(center.x + rx, center.y)`) collapses that axis's radius to zero
            // instead of leaving it at `rx`, making the gradient degenerate/invisible.
            gradient_layer.setEndPoint(objc2_core_foundation::CGPoint::new(
                center.x + rx as f64,
                center.y + ry as f64,
            ));
            &g.stops
        }
        _ => return false,
    };
    if stops.is_empty() {
        return false;
    }

    let colors: Vec<CFRetained<CGColor>> =
        stops.iter().map(|s| color_to_cgcolor(s.color)).collect();
    let color_refs: Vec<&objc2::runtime::AnyObject> = colors
        .iter()
        .map(|c| c.as_ref() as &objc2_core_foundation::CFType)
        .map(|c| c.as_ref())
        .collect();
    let colors_array = NSArray::from_slice(&color_refs);
    unsafe { gradient_layer.setColors(Some(&colors_array)) };

    let locations: Vec<Retained<NSNumber>> = stops
        .iter()
        .map(|s| NSNumber::new_f64(s.offset as f64))
        .collect();
    let location_refs: Vec<&NSNumber> = locations.iter().map(|n| n.as_ref()).collect();
    gradient_layer.setLocations(Some(&NSArray::from_slice(&location_refs)));

    // Attach before masking: `add_sublayer_scaled` stamps `ca_layer`'s scale from `layer` at
    // attach time, and `set_mask_scaled` below needs that already-set scale on `ca_layer` to
    // propagate correctly onto `mask_layer`.
    add_sublayer_scaled(layer, ca_layer);

    // `local_bounds` is already `bounds` re-anchored at (0, 0) — the identity transform (not
    // another `translation(-bounds.x, -bounds.y)`) is what belongs alongside it; applying both
    // shifts the mask a second time; for a `bounds` far from the canvas origin (any cell but the
    // very first) that moves the mask entirely outside `gradient_layer`'s own local bounds,
    // leaving nothing visible at all (an *empty* intersection, not just a misaligned one).
    let mask_layer = CAShapeLayer::new();
    let identity = elwindui_core::base::AffineTransform::identity();
    let local_bounds = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        ..bounds
    };
    let mask_path = match mask_shape {
        GradientMaskShape::RoundedRect(radii) => {
            rounded_rect_cgpath(&identity, local_bounds, radii)
        }
        GradientMaskShape::Ellipse => ellipse_cgpath(&identity, local_bounds),
    };
    mask_layer.setPath(Some(&mask_path));
    mask_layer.setFillColor(Some(&color_to_cgcolor(
        elwindui_core::graphics::Color::black(),
    )));
    let mask_layer: Retained<CALayer> = Retained::into_super(mask_layer);
    set_mask_scaled(ca_layer, &mask_layer);

    true
}

pub(crate) fn gradient_unit_point(
    p: elwindui_core::base::Point,
    mapping: elwindui_core::graphics::BrushMappingMode,
    bounds: elwindui_core::base::Rect,
) -> objc2_core_foundation::CGPoint {
    match mapping {
        elwindui_core::graphics::BrushMappingMode::RelativeToBounds => {
            objc2_core_foundation::CGPoint::new(p.x as f64, p.y as f64)
        }
        elwindui_core::graphics::BrushMappingMode::Absolute => objc2_core_foundation::CGPoint::new(
            ((p.x - bounds.x) / bounds.width.max(1e-6)) as f64,
            ((p.y - bounds.y) / bounds.height.max(1e-6)) as f64,
        ),
    }
}

/// Realizes an `ImageBrush` fill as a real image `CALayer`, masked to `shape`'s outline — the
/// `Brush::Image` sibling of `try_add_gradient_fill_layer` above, same masked-sublayer strategy
/// (see that function's own doc comment for why the mask needs its own local-space path). Returns
/// `false` (does nothing) for anything but an `Image` brush under a pure-translation `world`, so
/// the caller falls back to `add_shape_layer`'s existing (no-op-for-`Image`) path in those cases.
pub(crate) fn try_add_image_fill_layer(
    layer: &Retained<CALayer>,
    brush: &elwindui_core::graphics::Brush,
    bounds: elwindui_core::base::Rect,
    mask_shape: GradientMaskShape,
    world: &elwindui_core::base::AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) -> bool {
    use elwindui_core::graphics::Brush;
    let Brush::Image(image_brush) = brush else {
        return false;
    };
    if !is_pure_translation(world) {
        return false;
    }
    let Some(resolved) = resolve_cgimage(&image_brush.image, image_cache) else {
        return false;
    };
    let Some(cg_image) = crop_cgimage(&resolved, image_brush.source_rect) else {
        return false;
    };
    let image_size = (
        CGImage::width(Some(&cg_image)) as f32,
        CGImage::height(Some(&cg_image)) as f32,
    );

    let absolute_origin = world.transform_point(elwindui_core::base::Point {
        x: bounds.x,
        y: bounds.y,
    });
    let container = CALayer::new();
    container.setName(Some(&NSString::from_str("elwindui-paint")));
    container.setMasksToBounds(true);
    container.setFrame(NSRect::new(
        objc2_foundation::NSPoint::new(absolute_origin.x as f64, absolute_origin.y as f64),
        objc2_foundation::NSSize::new(bounds.width as f64, bounds.height as f64),
    ));
    container.setOpacity(opacity * image_brush.opacity);

    let local_bounds = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        ..bounds
    };
    match image_brush.tile_mode {
        elwindui_core::graphics::TileMode::None => {
            let placed = fitted_image_rect(
                local_bounds,
                image_size,
                image_brush.stretch.into(),
                image_brush.alignment_x,
                image_brush.alignment_y,
            );
            let image_layer = CALayer::new();
            image_layer.setFrame(NSRect::new(
                objc2_foundation::NSPoint::new(placed.x as f64, placed.y as f64),
                objc2_foundation::NSSize::new(placed.width as f64, placed.height as f64),
            ));
            unsafe {
                image_layer.setContents(Some(cg_image.as_ref() as &objc2::runtime::AnyObject))
            };
            add_sublayer_scaled(&container, &image_layer);
        }
        tile_mode @ (elwindui_core::graphics::TileMode::Tile
        | elwindui_core::graphics::TileMode::FlipX
        | elwindui_core::graphics::TileMode::FlipY
        | elwindui_core::graphics::TileMode::FlipXY) => {
            // Rasterized at `scale` pixels per point — same reasoning as `render::vector::raster::
            // rasterize_nodes_to_pixels`: this bitmap is generated once and then displayed at a
            // fixed point size (`local_bounds`), so the scale has to be baked into the raster
            // itself, not just into the displaying `image_layer`'s `contentsScale` (which
            // `add_sublayer_scaled` sets correctly below regardless, but that alone would just
            // upscale an already-1x bitmap).
            let scale = layer.contentsScale() as f32;
            let pixel_width = (local_bounds.width * scale).ceil().max(1.0) as usize;
            let pixel_height = (local_bounds.height * scale).ceil().max(1.0) as usize;
            if pixel_width <= crate::render::vector::MAX_OFFSCREEN_DIMENSION
                && pixel_height <= crate::render::vector::MAX_OFFSCREEN_DIMENSION
            {
                // Keep the retained tree bounded: the per-cell layers exist only while Core
                // Animation rasterizes this brush, then one CGImage-backed layer replaces them.
                let tile_root = CALayer::new();
                tile_root.setBounds(objc2_core_foundation::CGRect::new(
                    objc2_core_foundation::CGPoint::new(0.0, 0.0),
                    objc2_core_foundation::CGSize::new(pixel_width as f64, pixel_height as f64),
                ));
                // `tile_root` renders in pixel space, so both the tile grid's extent and each
                // tile's own size (which `add_tiled_image_layers` derives from `tile_transform`'s
                // diagonal) need the same `scale` folded in — `AffineTransform::scale` composed
                // via `concat` scales `image_brush.transform`'s diagonal directly (the only part
                // `add_tiled_image_layers` reads for sizing).
                let pixel_local_bounds = elwindui_core::base::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: local_bounds.width * scale,
                    height: local_bounds.height * scale,
                };
                let pixel_transform = elwindui_core::base::AffineTransform::scale(scale, scale)
                    .concat(&image_brush.transform);
                add_tiled_image_layers(
                    &tile_root,
                    &cg_image,
                    image_size,
                    pixel_transform,
                    tile_mode,
                    pixel_local_bounds,
                );
                if let Some((pixels, pixel_width, pixel_height)) =
                    crate::render::rasterize_calayer_to_pixels(&tile_root, pixel_width, pixel_height)
                {
                    if let Some(tiled_image) =
                        crate::render::pixels_to_cgimage(pixels, pixel_width, pixel_height)
                    {
                        let image_layer = CALayer::new();
                        image_layer.setFrame(NSRect::new(
                            objc2_foundation::NSPoint::new(
                                local_bounds.x as f64,
                                local_bounds.y as f64,
                            ),
                            objc2_foundation::NSSize::new(
                                local_bounds.width as f64,
                                local_bounds.height as f64,
                            ),
                        ));
                        // `rasterize_calayer_to_pixels` returns a top-down bitmap, while a
                        // CGImage used as CALayer contents is interpreted in the opposite Y
                        // direction.  The temporary per-tile layers therefore look correct, but
                        // their single aggregated replacement is vertically inverted unless its
                        // contents layer is flipped back around its default center anchor.
                        image_layer.setAffineTransform(objc2_core_foundation::CGAffineTransform {
                            a: 1.0,
                            b: 0.0,
                            c: 0.0,
                            d: -1.0,
                            tx: 0.0,
                            ty: 0.0,
                        });
                        unsafe {
                            image_layer.setContents(Some(
                                tiled_image.as_ref() as &objc2::runtime::AnyObject
                            ))
                        };
                        add_sublayer_scaled(&container, &image_layer);
                    } else {
                        add_tiled_image_layers(
                            &container,
                            &cg_image,
                            image_size,
                            image_brush.transform,
                            tile_mode,
                            local_bounds,
                        );
                    }
                } else {
                    add_tiled_image_layers(
                        &container,
                        &cg_image,
                        image_size,
                        image_brush.transform,
                        tile_mode,
                        local_bounds,
                    );
                }
            } else {
                // Explicit bounded fallback for oversized target regions: preserve rendering with
                // the existing capped grid rather than allocating an unbounded bitmap.
                add_tiled_image_layers(
                    &container,
                    &cg_image,
                    image_size,
                    image_brush.transform,
                    tile_mode,
                    local_bounds,
                );
            }
        }
    }

    // Same re-anchored-at-(0,0) mask path `try_add_gradient_fill_layer` builds — see that
    // function's own doc comment for why `local_bounds` (not another `translation(-bounds.x,
    // -bounds.y)`) is what belongs alongside the identity transform here.
    let mask_layer = CAShapeLayer::new();
    let identity = elwindui_core::base::AffineTransform::identity();
    let mask_path = match mask_shape {
        GradientMaskShape::RoundedRect(radii) => {
            rounded_rect_cgpath(&identity, local_bounds, radii)
        }
        GradientMaskShape::Ellipse => ellipse_cgpath(&identity, local_bounds),
    };
    mask_layer.setPath(Some(&mask_path));
    mask_layer.setFillColor(Some(&color_to_cgcolor(
        elwindui_core::graphics::Color::black(),
    )));
    let mask_layer: Retained<CALayer> = Retained::into_super(mask_layer);
    // `container`'s own scale isn't authoritative yet at this point (it isn't attached to `layer`
    // until the next line) — `add_sublayer_scaled(layer, &container)` below recursively re-stamps
    // `container`, this mask, and every sublayer already added to `container` above, so this
    // `set_mask_scaled` call is a harmless (and cheap — a mask has no children of its own here)
    // no-op that keeps every attach in this function going through the same helper.
    set_mask_scaled(&container, &mask_layer);

    add_sublayer_scaled(layer, &container);
    true
}

/// Fills `local_bounds` (already `container`'s own `(0,0)`-anchored local space) with repeated
/// copies of `cg_image`, one tile per grid cell — the `TileMode::Tile`/`FlipX`/`FlipY`/`FlipXY`
/// sibling of `try_add_image_fill_layer`'s single-placement `TileMode::None` branch.
///
/// A tile's rendered size is `image_size` scaled by `tile_transform`'s *diagonal* only
/// (`m11`/`m22`) — off-diagonal rotation/skew components aren't supported for sizing a tile, a
/// deliberate simplification in the same spirit as this file's other documented-not-silent gaps
/// (e.g. `try_add_gradient_fill_layer`'s own doc comment on `GradientSpreadMethod::{Reflect,
/// Repeat}`). `ImageBrush` has no dedicated "one tile's size" field (unlike WPF's `TileBrush.
/// Viewport`) — SwiftUI's `ImagePaint(image:sourceRect:scale:)` is the closer prior art (a single
/// scale factor, no separate viewport), which is what this mirrors: reusing the existing
/// `transform` field's scale rather than adding a new one.
///
/// Each tile is positioned via `position`/`bounds`/`affineTransform` (default `anchorPoint`
/// `(0.5, 0.5)`), the same convention `build_image_container_layer`'s rotation fix and this
/// function's own `container` use — `affineTransform` here only ever carries a +/-1 diagonal
/// flip: `Tile` is the identity case (`flip_x`/`flip_y` both `false`), `FlipX`/`FlipY`/`FlipXY`
/// mirror alternating columns/rows/both, matching WPF `TileMode`'s semantics. Row/column counts
/// are capped at `MAX_TILES_PER_AXIS` so a near-zero `tile_transform` scale (e.g. a misconfigured
/// brush) produces a bounded, if visually wrong, sublayer count rather than an unbounded one.
pub(crate) fn add_tiled_image_layers(
    container: &Retained<CALayer>,
    cg_image: &CFRetained<CGImage>,
    image_size: (f32, f32),
    tile_transform: elwindui_core::base::AffineTransform,
    tile_mode: elwindui_core::graphics::TileMode,
    local_bounds: elwindui_core::base::Rect,
) {
    use elwindui_core::graphics::TileMode;
    const MAX_TILES_PER_AXIS: i32 = 64;
    let tile_w = (image_size.0 * tile_transform.m11.abs()).max(1.0);
    let tile_h = (image_size.1 * tile_transform.m22.abs()).max(1.0);
    let cols = ((local_bounds.width / tile_w).ceil() as i32).clamp(1, MAX_TILES_PER_AXIS);
    let rows = ((local_bounds.height / tile_h).ceil() as i32).clamp(1, MAX_TILES_PER_AXIS);
    for row in 0..rows {
        for col in 0..cols {
            let flip_x = matches!(tile_mode, TileMode::FlipX | TileMode::FlipXY) && col % 2 == 1;
            let flip_y = matches!(tile_mode, TileMode::FlipY | TileMode::FlipXY) && row % 2 == 1;
            let image_layer = CALayer::new();
            image_layer.setBounds(objc2_core_foundation::CGRect::new(
                objc2_core_foundation::CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(tile_w as f64, tile_h as f64),
            ));
            image_layer.setPosition(objc2_core_foundation::CGPoint::new(
                (local_bounds.x + col as f32 * tile_w + tile_w / 2.0) as f64,
                (local_bounds.y + row as f32 * tile_h + tile_h / 2.0) as f64,
            ));
            image_layer.setAffineTransform(objc2_core_foundation::CGAffineTransform {
                a: if flip_x { -1.0 } else { 1.0 },
                b: 0.0,
                c: 0.0,
                d: if flip_y { -1.0 } else { 1.0 },
                tx: 0.0,
                ty: 0.0,
            });
            unsafe {
                image_layer.setContents(Some(cg_image.as_ref() as &objc2::runtime::AnyObject))
            };
            add_sublayer_scaled(container, &image_layer);
        }
    }
}

/// Applies `brush` as `shape_layer`'s fill. A gradient brush is realized as a masked
/// `CAGradientLayer` sibling rather than `CAShapeLayer.fillColor` (which only accepts a solid
/// color) — `shape_layer` itself is left with no fill color (transparent interior) and the
/// gradient layer, masked by a copy of the same shape, is added alongside it in `shape_layer`'s
/// own superlayer once `shape_layer` itself has been added (see call sites).
pub(crate) fn apply_fill(
    shape_layer: &CAShapeLayer,
    brush: Option<&elwindui_core::graphics::Brush>,
    bounds: elwindui_core::base::Rect,
) {
    match brush {
        None => shape_layer.setFillColor(None),
        Some(elwindui_core::graphics::Brush::Solid(color)) => {
            shape_layer.setFillColor(Some(&color_to_cgcolor(*color)));
        }
        Some(
            brush @ (elwindui_core::graphics::Brush::LinearGradient(_)
            | elwindui_core::graphics::Brush::RadialGradient(_)),
        ) => {
            // No direct sibling-insertion point here (that needs the *superlayer*, only known
            // once `shape_layer` itself is added) — approximate with the gradient's first stop as
            // a flat fill instead. A `CAGradientLayer`+mask upgrade is real future work (painter
            // design doc §6), not a silent capability gap: this is the one brush combination this
            // backend doesn't fully realize yet, and it degrades to *a* reasonable solid color
            // rather than nothing.
            if let Some(color) = first_gradient_stop_color(brush) {
                shape_layer.setFillColor(Some(&color_to_cgcolor(color)));
            }
        }
        Some(elwindui_core::graphics::Brush::Image(_)) => {
            // `FillRect`/`FillRoundedRect`/`FillEllipse` never reach this arm for an `Image`
            // brush — their call sites try `try_add_image_fill_layer` first and only fall back
            // to `add_shape_layer` (hence here) when that returns `false` (a non-translation
            // `world`). `FillPath`/`StrokePath` have no such upstream attempt, so an `Image`
            // brush there still degrades to no fill at all, same as the gradient case above.
        }
    }
    let _ = bounds;
}

pub(crate) fn apply_stroke(
    shape_layer: &CAShapeLayer,
    brush: &elwindui_core::graphics::Brush,
    style: &elwindui_core::graphics::StrokeStyle,
    _bounds: elwindui_core::base::Rect,
) {
    let color = match brush {
        elwindui_core::graphics::Brush::Solid(color) => *color,
        other => {
            first_gradient_stop_color(other).unwrap_or(elwindui_core::graphics::Color::black())
        }
    };
    shape_layer.setStrokeColor(Some(&color_to_cgcolor(color)));
    shape_layer.setLineWidth(style.width as f64);
    shape_layer.setMiterLimit(style.miter_limit as f64);
    shape_layer.setLineCap(ca_line_cap(style.end_cap));
    shape_layer.setLineJoin(ca_line_join(style.line_join));
    if !style.dash_pattern.is_empty() {
        let numbers: Vec<Retained<NSNumber>> = style
            .dash_pattern
            .iter()
            .map(|&d| NSNumber::new_f64(d as f64))
            .collect();
        let refs: Vec<&NSNumber> = numbers.iter().map(|n| n.as_ref()).collect();
        let array = NSArray::from_slice(&refs);
        shape_layer.setLineDashPattern(Some(&array));
        shape_layer.setLineDashPhase(style.dash_offset as f64);
    } else {
        shape_layer.setLineDashPattern(None);
    }
}

pub(crate) fn first_gradient_stop_color(
    brush: &elwindui_core::graphics::Brush,
) -> Option<elwindui_core::graphics::Color> {
    match brush {
        elwindui_core::graphics::Brush::LinearGradient(g) => g.stops.first().map(|s| s.color),
        elwindui_core::graphics::Brush::RadialGradient(g) => g.stops.first().map(|s| s.color),
        _ => None,
    }
}
