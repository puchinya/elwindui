//! Primitive fast-paths: replacing `CGPath` + `CAShapeLayer` construction with plain `CALayer`
//! properties (`backgroundColor`/`borderColor`/`borderWidth`/`cornerRadius`) when a command's
//! geometry and brush are simple enough to allow it. See the AppKit render optimization
//! implementation guide §7-9/§18 (this is that guide's Phase 2).
//!
//! Every fast path here is a pure *classification* — deciding whether a command qualifies, and
//! if so applying the matching `CALayer` properties directly. The existing `CAShapeLayer`/
//! `CAGradientLayer` machinery in `paint`/`path` is never modified and stays the fallback for
//! everything a fast path declines (rotated/sheared groups, non-uniform radii, dashed or
//! non-default-joined strokes, gradient/image brushes) — so every golden test that exercises that
//! machinery directly keeps passing unchanged.

use elwindui_core::base::{AffineTransform, CornerRadius, Rect};
use elwindui_core::graphics::{Brush, Color, LineCap, LineJoin, RenderCommand, StrokeStyle};
use objc2::rc::Retained;
use objc2_quartz_core::CALayer;

use super::geometry::{color_to_cgcolor, is_pure_translation};

/// Extracts `(rect, radii, brush)` from a fill command this module can fast-path, unifying
/// `FillRect`'s implicit zero radius with `FillRoundedRect`'s explicit one.
fn fill_shape(command: &RenderCommand) -> Option<(Rect, CornerRadius, &Brush)> {
    match command {
        RenderCommand::FillRect { rect, brush } => Some((*rect, CornerRadius::default(), brush)),
        RenderCommand::FillRoundedRect { rect, radii, brush } => Some((*rect, *radii, brush)),
        _ => None,
    }
}

/// Extracts `(rect, radii, brush, stroke)` from a stroke command this module can fast-path.
fn stroke_shape(command: &RenderCommand) -> Option<(Rect, CornerRadius, &Brush, &StrokeStyle)> {
    match command {
        RenderCommand::StrokeRect { rect, brush, stroke } => {
            Some((*rect, CornerRadius::default(), brush, stroke))
        }
        RenderCommand::StrokeRoundedRect {
            rect,
            radii,
            brush,
            stroke,
        } => Some((*rect, *radii, brush, stroke)),
        _ => None,
    }
}

/// A single scalar `CornerRadius` a `CALayer.cornerRadius` fast path can express — `None` for any
/// non-uniform radius (per-corner geometry needs the general `CGPath` path).
fn uniform_radius(radii: CornerRadius) -> Option<f32> {
    if radii.top_left == radii.top_right
        && radii.top_left == radii.bottom_right
        && radii.top_left == radii.bottom_left
    {
        Some(radii.top_left)
    } else {
        None
    }
}

/// Whether `stroke` is simple enough for `CALayer.borderWidth`/`borderColor` — a solid, undashed,
/// default-capped/joined outline is the only shape `CALayer`'s own border drawing can express;
/// everything else (dash pattern, round/square caps, bevel/round joins, asymmetric miter limits)
/// needs the general `CGPath` + `CAShapeLayer` stroke path instead.
fn is_simple_border(stroke: &StrokeStyle) -> bool {
    stroke.dash_pattern.is_empty()
        && stroke.start_cap == LineCap::Butt
        && stroke.end_cap == LineCap::Butt
        && stroke.line_join == LineJoin::Miter
}

fn solid_color(brush: &Brush) -> Option<Color> {
    match brush {
        Brush::Solid(color) => Some(*color),
        _ => None,
    }
}

/// Attempts to render `command` (and, for the fill+stroke fusion case, the following command
/// `next`) as plain `CALayer` properties on a freshly created layer, appending it to `layer` if
/// successful. Returns how many commands were consumed: `0` means no fast path applied and the
/// caller must fall back to the general path for `command` alone; `1`/`2` mean `command` (and,
/// for `2`, `next` too) were fully handled.
///
/// `world` must already be known pure-translation (`FillRect`/`FillRoundedRect`/`StrokeRect`/
/// `StrokeRoundedRect` under a rotated or sheared group fall back to the general path, same as
/// `try_add_gradient_fill_layer` already does for gradients — `CALayer.cornerRadius`/`borderWidth`
/// have no rotation-aware equivalent to a transformed `CGPath`).
pub(crate) fn try_fast_path(
    layer: &Retained<CALayer>,
    command: &RenderCommand,
    next: Option<&RenderCommand>,
    world: &AffineTransform,
    opacity: f32,
) -> usize {
    if !is_pure_translation(world) {
        return 0;
    }
    let Some((rect, radii, fill_brush)) = fill_shape(command) else {
        // Not a fill — still try the standalone-stroke fast path below.
        return try_stroke_only(layer, command, world, opacity);
    };
    let Some(fill_color) = solid_color(fill_brush) else {
        return 0;
    };
    let Some(radius) = uniform_radius(radii) else {
        return 0;
    };

    // Fusion: a StrokeRect/StrokeRoundedRect immediately following, on the same rect and radii,
    // with a solid simple-border brush, folds into the same CALayer — 2 layers become 1 (guide
    // §8/§18's flagship example). Consumed only when every condition holds; otherwise this
    // command is still handled alone via the plain fill-only branch below.
    if let Some(next) = next {
        if let Some((stroke_rect, stroke_radii, stroke_brush, stroke_style)) = stroke_shape(next) {
            if stroke_rect == rect
                && stroke_radii == radii
                && is_simple_border(stroke_style)
                && let Some(stroke_color) = solid_color(stroke_brush)
            {
                let origin = world.transform_point(elwindui_core::base::Point {
                    x: rect.x,
                    y: rect.y,
                });
                let ca_layer = CALayer::new();
                super::stats::bump(|s| s.layers_created += 1);
                place(&ca_layer, origin, rect, opacity);
                ca_layer.setBackgroundColor(Some(&color_to_cgcolor(fill_color)));
                ca_layer.setCornerRadius(radius as f64);
                ca_layer.setBorderColor(Some(&color_to_cgcolor(stroke_color)));
                ca_layer.setBorderWidth(stroke_style.width as f64);
                super::add_sublayer_scaled(layer, &ca_layer);
                return 2;
            }
        }
    }

    let origin = world.transform_point(elwindui_core::base::Point { x: rect.x, y: rect.y });
    let ca_layer = CALayer::new();
    super::stats::bump(|s| s.layers_created += 1);
    place(&ca_layer, origin, rect, opacity);
    ca_layer.setBackgroundColor(Some(&color_to_cgcolor(fill_color)));
    if radius != 0.0 {
        ca_layer.setCornerRadius(radius as f64);
    }
    super::add_sublayer_scaled(layer, &ca_layer);
    1
}

/// The standalone-stroke half of `try_fast_path` — a `StrokeRect`/`StrokeRoundedRect` not
/// preceded by (or not fusable with) a matching fill.
fn try_stroke_only(
    layer: &Retained<CALayer>,
    command: &RenderCommand,
    world: &AffineTransform,
    opacity: f32,
) -> usize {
    let Some((rect, radii, brush, stroke)) = stroke_shape(command) else {
        return 0;
    };
    let Some(color) = solid_color(brush) else {
        return 0;
    };
    let Some(radius) = uniform_radius(radii) else {
        return 0;
    };
    if !is_simple_border(stroke) {
        return 0;
    }
    let origin = world.transform_point(elwindui_core::base::Point { x: rect.x, y: rect.y });
    let ca_layer = CALayer::new();
    super::stats::bump(|s| s.layers_created += 1);
    place(&ca_layer, origin, rect, opacity);
    ca_layer.setBorderColor(Some(&color_to_cgcolor(color)));
    ca_layer.setBorderWidth(stroke.width as f64);
    if radius != 0.0 {
        ca_layer.setCornerRadius(radius as f64);
    }
    super::add_sublayer_scaled(layer, &ca_layer);
    1
}

/// Positions `ca_layer` at `origin` (already `world`-transformed) with `rect`'s own untransformed
/// size — valid because `try_fast_path`/`try_stroke_only` only ever run under a confirmed
/// pure-translation `world` (no rotation/scale to fold into the frame).
fn place(ca_layer: &CALayer, origin: elwindui_core::base::Point, rect: Rect, opacity: f32) {
    ca_layer.setFrame(objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(origin.x as f64, origin.y as f64),
        objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
    ));
    ca_layer.setOpacity(opacity);
}
