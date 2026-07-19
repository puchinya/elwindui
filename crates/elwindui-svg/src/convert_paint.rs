//! `usvg` paint/stroke/clip/mask → `elwindui_core::graphics` types (実装指示書§7/§8).

use crate::convert::{color_from_usvg, convert_group, rect_from_nonzero, transform_from_usvg};
use elwindui_core::base::Point;
use elwindui_core::graphics::{
    Brush, FillRule, GradientSpreadMethod, GradientStop, LinearGradientBrush, LineCap, LineJoin,
    RadialGradientBrush, StrokeStyle, VectorBlendMode, VectorClipPath, VectorFill, VectorMask,
    VectorMaskType, VectorPaint, VectorPattern, VectorStroke,
};
use std::sync::Arc;

pub(crate) fn blend_mode_from_usvg(mode: usvg::BlendMode) -> VectorBlendMode {
    match mode {
        usvg::BlendMode::Normal => VectorBlendMode::Normal,
        usvg::BlendMode::Multiply => VectorBlendMode::Multiply,
        usvg::BlendMode::Screen => VectorBlendMode::Screen,
        usvg::BlendMode::Overlay => VectorBlendMode::Overlay,
        usvg::BlendMode::Darken => VectorBlendMode::Darken,
        usvg::BlendMode::Lighten => VectorBlendMode::Lighten,
        usvg::BlendMode::ColorDodge => VectorBlendMode::ColorDodge,
        usvg::BlendMode::ColorBurn => VectorBlendMode::ColorBurn,
        usvg::BlendMode::HardLight => VectorBlendMode::HardLight,
        usvg::BlendMode::SoftLight => VectorBlendMode::SoftLight,
        usvg::BlendMode::Difference => VectorBlendMode::Difference,
        usvg::BlendMode::Exclusion => VectorBlendMode::Exclusion,
        usvg::BlendMode::Hue => VectorBlendMode::Hue,
        usvg::BlendMode::Saturation => VectorBlendMode::Saturation,
        usvg::BlendMode::Color => VectorBlendMode::Color,
        usvg::BlendMode::Luminosity => VectorBlendMode::Luminosity,
    }
}

fn convert_line_cap(cap: usvg::LineCap) -> LineCap {
    match cap {
        usvg::LineCap::Butt => LineCap::Butt,
        usvg::LineCap::Round => LineCap::Round,
        usvg::LineCap::Square => LineCap::Square,
    }
}

/// SVG has a single `stroke-linejoin` value, not WinUI3-style per-corner-kind variants, so it
/// maps onto `StrokeStyle.line_join` alone. `MiterClip` (an SVG2 addition with no `LineJoin`
/// counterpart here) approximates to `Miter` — visually close for all but extreme miter angles,
/// and not worth a dedicated variant on `LineJoin` (a type shared by every non-SVG caller of the
/// graphics API too) for this one SVG-specific edge case.
fn convert_line_join(join: usvg::LineJoin) -> LineJoin {
    match join {
        usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
        usvg::LineJoin::Round => LineJoin::Round,
        usvg::LineJoin::Bevel => LineJoin::Bevel,
    }
}

fn convert_stroke_style(stroke: &usvg::Stroke) -> StrokeStyle {
    let cap = convert_line_cap(stroke.linecap());
    StrokeStyle {
        width: stroke.width().get(),
        start_cap: cap,
        end_cap: cap,
        dash_cap: cap,
        line_join: convert_line_join(stroke.linejoin()),
        miter_limit: stroke.miterlimit().get(),
        dash_pattern: stroke.dasharray().map(Arc::from).unwrap_or_else(|| Arc::from([])),
        dash_offset: stroke.dashoffset(),
    }
}

fn convert_gradient_stops(stops: &[usvg::Stop]) -> Vec<GradientStop> {
    stops
        .iter()
        .filter_map(|s| {
            GradientStop::new(s.offset().get(), color_from_usvg(s.color(), s.opacity().get())).ok()
        })
        .collect()
}

fn convert_spread(method: usvg::SpreadMethod) -> GradientSpreadMethod {
    match method {
        usvg::SpreadMethod::Pad => GradientSpreadMethod::Pad,
        usvg::SpreadMethod::Reflect => GradientSpreadMethod::Reflect,
        usvg::SpreadMethod::Repeat => GradientSpreadMethod::Repeat,
    }
}

/// `usvg` has already resolved `gradientUnits="objectBoundingBox"` into absolute user-space
/// coordinates by the time a `LinearGradient`/`RadialGradient` reaches this converter (its own
/// crate doc comment: "`objectBoundingBox` will be replaced with `userSpaceOnUse`"), so every
/// converted gradient is `BrushMappingMode::Absolute` — there is no bounding-box-relative case
/// left to represent here.
fn convert_paint(paint: &usvg::Paint) -> VectorPaint {
    match paint {
        usvg::Paint::Color(c) => VectorPaint::Brush(Brush::Solid(color_from_usvg(*c, 1.0))),
        usvg::Paint::LinearGradient(lg) => {
            let stops = convert_gradient_stops(lg.stops());
            match LinearGradientBrush::new(
                Point { x: lg.x1(), y: lg.y1() },
                Point { x: lg.x2(), y: lg.y2() },
                stops,
            ) {
                Ok(mut brush) => {
                    brush.spread = convert_spread(lg.spread_method());
                    brush.transform = transform_from_usvg(lg.transform());
                    VectorPaint::Brush(Brush::LinearGradient(brush))
                }
                Err(_) => VectorPaint::Brush(Brush::Solid(elwindui_core::graphics::Color::TRANSPARENT)),
            }
        }
        usvg::Paint::RadialGradient(rg) => {
            let stops = convert_gradient_stops(rg.stops());
            match RadialGradientBrush::new(
                Point { x: rg.cx(), y: rg.cy() },
                rg.r().get(),
                rg.r().get(),
                stops,
            ) {
                Ok(mut brush) => {
                    brush.gradient_origin = Point { x: rg.fx(), y: rg.fy() };
                    brush.spread = convert_spread(rg.spread_method());
                    brush.transform = transform_from_usvg(rg.transform());
                    VectorPaint::Brush(Brush::RadialGradient(brush))
                }
                Err(_) => VectorPaint::Brush(Brush::Solid(elwindui_core::graphics::Color::TRANSPARENT)),
            }
        }
        usvg::Paint::Pattern(pattern) => VectorPaint::Pattern(Arc::new(convert_pattern(pattern))),
    }
}

fn convert_pattern(pattern: &usvg::Pattern) -> VectorPattern {
    VectorPattern {
        tile_rect: rect_from_nonzero(pattern.rect()),
        transform: transform_from_usvg(pattern.transform()),
        // `usvg` resolves `patternContentUnits`/nested `viewBox` scaling directly into `root`'s
        // own child transforms, so there is no separate viewBox to carry here (same
        // simplification as the top-level `Tree` — see `convert::convert_tree`'s doc comment).
        view_box: None,
        preserve_aspect_ratio: elwindui_core::graphics::PreserveAspectRatio::default(),
        root: convert_group(pattern.root()),
    }
}

pub(crate) fn convert_fill(fill: &usvg::Fill) -> VectorFill {
    VectorFill {
        paint: convert_paint(fill.paint()),
        opacity: fill.opacity().get(),
        rule: match fill.rule() {
            usvg::FillRule::NonZero => FillRule::NonZero,
            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
        },
    }
}

pub(crate) fn convert_stroke(stroke: &usvg::Stroke) -> VectorStroke {
    VectorStroke {
        paint: convert_paint(stroke.paint()),
        opacity: stroke.opacity().get(),
        style: convert_stroke_style(stroke),
    }
}

pub(crate) fn convert_clip_path(clip: &usvg::ClipPath) -> VectorClipPath {
    VectorClipPath {
        transform: transform_from_usvg(clip.transform()),
        root: convert_group(clip.root()),
        nested: clip.clip_path().map(|c| Arc::new(convert_clip_path(c))),
    }
}

pub(crate) fn convert_mask(mask: &usvg::Mask) -> VectorMask {
    VectorMask {
        mask_type: match mask.kind() {
            usvg::MaskType::Luminance => VectorMaskType::Luminance,
            usvg::MaskType::Alpha => VectorMaskType::Alpha,
        },
        bounds: rect_from_nonzero(mask.rect()),
        // `usvg::Mask` has no transform of its own — its coordinate content is already resolved
        // relative to `root`'s own transform chain (same simplification as `Pattern`/`ClipPath`'s
        // viewBox handling).
        transform: elwindui_core::base::AffineTransform::IDENTITY,
        root: convert_group(mask.root()),
        nested: mask.mask().map(|m| Arc::new(convert_mask(m))),
    }
}
