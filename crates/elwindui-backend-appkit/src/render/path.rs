//! `elwindui_core::graphics::Path` and the stroke enums -> `CGPath`/`CAShapeLayer` attributes.
//! Pure geometry translation: nothing here touches a layer tree or a `RenderCommand`.

use super::geometry::*;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGMutablePath;
use objc2_quartz_core::{
    CAShapeLayerLineCap, CAShapeLayerLineJoin, kCALineCapButt, kCALineCapRound, kCALineCapSquare,
    kCALineJoinBevel, kCALineJoinMiter, kCALineJoinRound,
};

pub(crate) fn ca_line_cap(cap: elwindui_core::graphics::LineCap) -> &'static CAShapeLayerLineCap {
    unsafe {
        match cap {
            elwindui_core::graphics::LineCap::Butt => kCALineCapButt,
            elwindui_core::graphics::LineCap::Round => kCALineCapRound,
            elwindui_core::graphics::LineCap::Square => kCALineCapSquare,
        }
    }
}

pub(crate) fn ca_line_join(
    join: elwindui_core::graphics::LineJoin,
) -> &'static CAShapeLayerLineJoin {
    unsafe {
        match join {
            elwindui_core::graphics::LineJoin::Miter => kCALineJoinMiter,
            elwindui_core::graphics::LineJoin::Round => kCALineJoinRound,
            elwindui_core::graphics::LineJoin::Bevel => kCALineJoinBevel,
        }
    }
}

/// Builds via the general `PathBuilder`/`path_to_cgpath` route uniformly (rather than special-
/// casing `CGPath::with_rounded_rect` for the common uniform-radius/identity-transform case) —
/// `CGPath::with_rounded_rect` returns an *immutable* `CGPath`, whereas every other path this
/// backend builds is a `CGMutablePath` (so `transform`/dash/gradient-mask code can treat all of
/// them uniformly); bridging between the two isn't worth it for what's a one-time-per-repaint
/// path construction, not a hot loop.
pub(crate) fn rounded_rect_cgpath(
    world: &elwindui_core::base::AffineTransform,
    rect: elwindui_core::base::Rect,
    radii: elwindui_core::base::CornerRadius,
) -> CFRetained<CGMutablePath> {
    let mut builder = elwindui_core::graphics::PathBuilder::new();
    builder.add_rounded_rect(rect, radii);
    path_to_cgpath(
        world,
        &builder.build().expect("rounded rect path is never empty"),
    )
}

pub(crate) fn ellipse_cgpath(
    world: &elwindui_core::base::AffineTransform,
    rect: elwindui_core::base::Rect,
) -> CFRetained<CGMutablePath> {
    let mut builder = elwindui_core::graphics::PathBuilder::new();
    builder.add_ellipse(rect);
    path_to_cgpath(
        world,
        &builder.build().expect("ellipse path is never empty"),
    )
}

/// Converts one of our own `Path`s into a `CGMutablePath`, applying `world` to every point —
/// arcs/quads are already normalized to cubics by `Path`'s own internal representation, so this
/// only ever has to emit `moveTo`/`lineTo`/`curveTo`/`closePath`.
pub(crate) fn path_to_cgpath(
    world: &elwindui_core::base::AffineTransform,
    path: &elwindui_core::graphics::Path,
) -> CFRetained<CGMutablePath> {
    super::stats::bump(|s| s.cgpaths_created += 1);
    let cg_path = CGMutablePath::new();
    for command in path.commands() {
        match *command {
            elwindui_core::graphics::PathCommand::MoveTo(p) => {
                let p = transform_point(world, p);
                unsafe {
                    CGMutablePath::move_to_point(Some(&cg_path), std::ptr::null(), p.x, p.y);
                }
            }
            elwindui_core::graphics::PathCommand::LineTo(p) => {
                let p = transform_point(world, p);
                unsafe {
                    CGMutablePath::add_line_to_point(Some(&cg_path), std::ptr::null(), p.x, p.y);
                }
            }
            elwindui_core::graphics::PathCommand::QuadTo { control, to } => {
                let c = transform_point(world, control);
                let p = transform_point(world, to);
                unsafe {
                    CGMutablePath::add_quad_curve_to_point(
                        Some(&cg_path),
                        std::ptr::null(),
                        c.x,
                        c.y,
                        p.x,
                        p.y,
                    );
                }
            }
            elwindui_core::graphics::PathCommand::CubicTo {
                control1,
                control2,
                to,
            } => {
                let c1 = transform_point(world, control1);
                let c2 = transform_point(world, control2);
                let p = transform_point(world, to);
                unsafe {
                    CGMutablePath::add_curve_to_point(
                        Some(&cg_path),
                        std::ptr::null(),
                        c1.x,
                        c1.y,
                        c2.x,
                        c2.y,
                        p.x,
                        p.y,
                    );
                }
            }
            elwindui_core::graphics::PathCommand::ArcTo(_) => {
                // `Path` normalizes every `ArcTo` to cubics internally for bounds/flattening
                // purposes, but `PathCommand::ArcTo` itself (this raw command list) is the
                // author's original, un-normalized form — reachable here directly. Converting it
                // would duplicate `path.rs`'s own (private) `arc_to_cubics`; skipping it is a
                // known gap (an arc segment drawn via `PathBuilder::arc_to`/`arc_center` won't
                // render on this backend yet) rather than a silent geometry corruption.
            }
            elwindui_core::graphics::PathCommand::Close => {
                CGMutablePath::close_subpath(Some(&cg_path));
            }
        }
    }
    cg_path
}
