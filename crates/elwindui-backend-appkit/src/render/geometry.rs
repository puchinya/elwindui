//! Geometry, clipping, and colour helpers shared by the rest of `render` — the pieces that
//! translate `elwindui_core::base`/`graphics` values into Core Graphics/Core Animation equivalents
//! without themselves building any layer tree.


use super::path::*;
use elwindui_core::graphics::RenderCommand;
use objc2::rc::Retained;
use objc2_core_graphics::CGColor;
use objc2_quartz_core::{CALayer, CAShapeLayer, kCAFillRuleEvenOdd, kCAFillRuleNonZero};

/// Parses a `"#RRGGBB"`/`"#RRGGBBAA"` hex color (the only form `Rectangle`/`Ellipse`'s `fill`/
/// `stroke` params accept — see docs/elwindui_builtins_spec.md 付録N/G) into a `CGColor`. An
/// unparseable string falls back to opaque black rather than panicking, since this runs during
/// layout, not construction.
pub(crate) fn parse_color(hex: &str) -> objc2_core_foundation::CFRetained<CGColor> {
    let hex = hex.trim_start_matches('#');
    let (r, g, b, a) = match (hex.len(), u32::from_str_radix(hex, 16)) {
        (6, Ok(v)) => (
            ((v >> 16) & 0xFF) as f64,
            ((v >> 8) & 0xFF) as f64,
            (v & 0xFF) as f64,
            255.0,
        ),
        (8, Ok(v)) => (
            ((v >> 24) & 0xFF) as f64,
            ((v >> 16) & 0xFF) as f64,
            ((v >> 8) & 0xFF) as f64,
            (v & 0xFF) as f64,
        ),
        _ => (0.0, 0.0, 0.0, 255.0),
    };
    CGColor::new_generic_rgb(r / 255.0, g / 255.0, b / 255.0, a / 255.0)
}

/// The (already origin-adjusted, pre-transform) bounding rect a paint command occupies — used
/// only for the clip bounding-box overlap test in `replay_commands`, so a command with no
/// meaningful rect (nothing today) can return `None` to always pass.
pub(crate) fn geometry_bounds(
    command: &RenderCommand,
    origin: elwindui_core::base::Point,
) -> Option<elwindui_core::base::Rect> {
    let offset = |r: &elwindui_core::base::Rect| elwindui_core::base::Rect {
        x: origin.x + r.x,
        y: origin.y + r.y,
        width: r.width,
        height: r.height,
    };
    match command {
        RenderCommand::FillRect { rect, .. }
        | RenderCommand::StrokeRect { rect, .. }
        | RenderCommand::FillRoundedRect { rect, .. }
        | RenderCommand::StrokeRoundedRect { rect, .. }
        | RenderCommand::FillEllipse { rect, .. }
        | RenderCommand::StrokeEllipse { rect, .. }
        | RenderCommand::Text { rect, .. } => Some(offset(rect)),
        RenderCommand::DrawImage { dest, .. } | RenderCommand::DrawVectorImage { dest, .. } => {
            Some(offset(dest))
        }
        RenderCommand::DrawLine { .. }
        | RenderCommand::FillPath { .. }
        | RenderCommand::StrokePath { .. } => None,
        RenderCommand::NativeControl { .. }
        | RenderCommand::PushClip { .. }
        | RenderCommand::PopClip
        | RenderCommand::PushTransform { .. }
        | RenderCommand::PopTransform
        | RenderCommand::PushOpacity { .. }
        | RenderCommand::PopOpacity => None,
    }
}

/// Absolute (origin-adjusted) bounds of a `Clip` value, for `replay_commands`'s own clip-stack
/// intersection — `Clip::Path`'s bounds are used (a bounding-box approximation, consistent with
/// this whole replay pass never doing true per-pixel clipping).
pub(crate) fn clip_bounds(
    clip: &elwindui_core::graphics::Clip,
    origin: elwindui_core::base::Point,
) -> Option<elwindui_core::base::Rect> {
    let offset = |r: elwindui_core::base::Rect| elwindui_core::base::Rect {
        x: origin.x + r.x,
        y: origin.y + r.y,
        width: r.width,
        height: r.height,
    };
    match clip {
        elwindui_core::graphics::Clip::Rect(r) => Some(offset(*r)),
        elwindui_core::graphics::Clip::RoundedRect { rect, .. } => Some(offset(*rect)),
        elwindui_core::graphics::Clip::Path { path, .. } => Some(offset(path.bounds())),
    }
}

/// Builds the `CAShapeLayer` mask that gives `PushClip`/`PopClip` (`replay_commands`) real
/// per-pixel clipping — `world` is already `translation(origin) * transform` at the `PushClip`
/// site, keeping the mask path in the same canvas-absolute coordinate space the masked container
/// layer occupies (its `frame` is set to exactly overlay its parent, so no re-anchoring is needed).
pub(crate) fn clip_mask_layer(
    world: &elwindui_core::base::AffineTransform,
    clip: &elwindui_core::graphics::Clip,
) -> Retained<CALayer> {
    let mask_layer = CAShapeLayer::new();
    let (path, rule) = match clip {
        elwindui_core::graphics::Clip::Rect(rect) => (
            rounded_rect_cgpath(world, *rect, elwindui_core::base::CornerRadius::default()),
            elwindui_core::graphics::FillRule::NonZero,
        ),
        elwindui_core::graphics::Clip::RoundedRect { rect, radii } => {
            (rounded_rect_cgpath(world, *rect, *radii), elwindui_core::graphics::FillRule::NonZero)
        }
        elwindui_core::graphics::Clip::Path { path, rule } => (path_to_cgpath(world, path), *rule),
    };
    mask_layer.setPath(Some(&path));
    mask_layer.setFillRule(match rule {
        elwindui_core::graphics::FillRule::NonZero => unsafe { kCAFillRuleNonZero },
        elwindui_core::graphics::FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
    });
    mask_layer.setFillColor(Some(&color_to_cgcolor(elwindui_core::graphics::Color::black())));
    Retained::into_super(mask_layer)
}

pub(crate) fn transform_point(
    t: &elwindui_core::base::AffineTransform,
    p: elwindui_core::base::Point,
) -> objc2_foundation::NSPoint {
    let p = t.transform_point(p);
    objc2_foundation::NSPoint::new(p.x as f64, p.y as f64)
}

pub(crate) fn is_pure_translation(t: &elwindui_core::base::AffineTransform) -> bool {
    (t.m11 - 1.0).abs() < 1e-4
        && t.m12.abs() < 1e-4
        && t.m21.abs() < 1e-4
        && (t.m22 - 1.0).abs() < 1e-4
}

pub(crate) fn color_to_cgcolor(
    color: elwindui_core::graphics::Color,
) -> objc2_core_foundation::CFRetained<CGColor> {
    CGColor::new_generic_rgb(
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
        color.a as f64 / 255.0,
    )
}
