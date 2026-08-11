//! Pixel-level golden tests for the Core Animation drawing path (`render`): fills, strokes,
//! gradients, image fit/crop, transforms, opacity.
//!
//! Offscreen golden-scene rendering tests (painter design doc §20.2) — renders a handful of
//! representative scenes into an in-memory `CGBitmapContext` via `CALayer.renderInContext`
//! (no window/screen involved, so no Screen Recording permission is needed and these run
//! headlessly in `cargo test`) and asserts specific sample pixels rather than diffing against a
//! checked-in reference PNG — a narrower, self-contained foundation for this class of test rather
//! than the full 24-scene cross-backend suite the design doc describes (WinUI3/GTK4 can't run on
//! this machine at all — see `docs/status/implementation_status.md` — so a true cross-backend
//! image diff isn't achievable here regardless).

use crate::render::fitted_image_rect;
use crate::render::{
    GradientMaskShape, add_shape_layer, apply_fill, build_image_container_layer, ellipse_cgpath,
    path_to_cgpath, resolve_cgimage, rounded_rect_cgpath, try_add_gradient_fill_layer,
};
use crate::testsupport::bitmap::Bitmap;
use objc2::rc::Retained;
use objc2_core_graphics::CGMutablePath;
use objc2_quartz_core::CAShapeLayer;
use objc2_quartz_core::{CALayer, kCAFillRuleEvenOdd, kCAFillRuleNonZero};
use std::collections::HashMap;

/// `CALayer.renderInContext:` against a `CGBitmapContext` renders **Y-flipped** relative to
/// the logical/path coordinates fed to `add_shape_layer`/`rounded_rect_cgpath`/etc — a shape
/// built at logical `y` ends up at roughly `bitmap.pixel(x, bitmap.height - y)`, not
/// `bitmap.pixel(x, y)`. The 4 original tests below never surfaced this (they only ever sample
/// flip-symmetric geometry: bounding-box corners of a uniform shape, or points exactly on the
/// canvas's own vertical center) — any *new* test with real top/bottom asymmetry (e.g. one
/// rounded corner vs one sharp corner, a curve that bows toward one edge) must account for it.
fn render_layer(root: &Retained<CALayer>, bitmap: &Bitmap) {
    root.renderInContext(&bitmap.ctx);
}

fn approx(actual: (u8, u8, u8, u8), expected: (u8, u8, u8, u8), tolerance: u8) {
    let close = |a: u8, b: u8| a.abs_diff(b) <= tolerance;
    assert!(
        close(actual.0, expected.0)
            && close(actual.1, expected.1)
            && close(actual.2, expected.2)
            && close(actual.3, expected.3),
        "expected {expected:?}, got {actual:?} (tolerance {tolerance})"
    );
}

#[test]
fn solid_filled_rect_paints_the_expected_color_and_nothing_outside_it() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let rect = elwindui_core::base::Rect {
        x: 16.0,
        y: 16.0,
        width: 32.0,
        height: 32.0,
    };
    let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
    add_shape_layer(
        &root,
        &path,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(255, 0, 0),
        )),
        None,
        1.0,
        rect,
    );
    render_layer(&root, &bitmap);
    approx(bitmap.pixel(32, 32), (255, 0, 0, 255), 50);
    approx(bitmap.pixel(2, 2), (0, 0, 0, 0), 10);
}

#[test]
fn filled_ellipse_is_transparent_at_its_corners() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let rect = elwindui_core::base::Rect {
        x: 8.0,
        y: 8.0,
        width: 48.0,
        height: 48.0,
    };
    let path = ellipse_cgpath(&world, rect);
    add_shape_layer(
        &root,
        &path,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 128, 255),
        )),
        None,
        1.0,
        rect,
    );
    render_layer(&root, &bitmap);
    // Ellipse center: opaque blue.
    approx(bitmap.pixel(32, 32), (0, 128, 255, 255), 50);
    // Bounding-box corner: outside the ellipse's curve, must stay transparent.
    approx(bitmap.pixel(9, 9), (0, 0, 0, 0), 10);
}

#[test]
fn stroked_rect_paints_only_near_its_border() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let rect = elwindui_core::base::Rect {
        x: 16.0,
        y: 16.0,
        width: 32.0,
        height: 32.0,
    };
    let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 4.0,
        ..Default::default()
    };
    add_shape_layer(
        &root,
        &path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        rect,
    );
    render_layer(&root, &bitmap);
    // Interior of the rect (well inside the 4px-wide border): unpainted.
    approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 10);
    // Right on the border: opaque black.
    approx(bitmap.pixel(16, 32), (0, 0, 0, 255), 40);
}

#[test]
fn opacity_accumulator_scales_down_alpha() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let rect = elwindui_core::base::Rect {
        x: 16.0,
        y: 16.0,
        width: 32.0,
        height: 32.0,
    };
    let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
    add_shape_layer(
        &root,
        &path,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 255, 0),
        )),
        None,
        0.5,
        rect,
    );
    render_layer(&root, &bitmap);
    let (_, _, _, a) = bitmap.pixel(32, 32);
    assert!(
        a < 200,
        "half-opacity fill should not be fully opaque, got alpha {a}"
    );
    assert!(
        a > 50,
        "half-opacity fill should still be visibly painted, got alpha {a}"
    );
}

#[test]
fn fitted_image_rect_fill_always_matches_dest_regardless_of_image_size() {
    let dest = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 50.0,
    };
    let placed = fitted_image_rect(
        dest,
        (20.0, 80.0),
        elwindui_core::graphics::ImageFit::Fill,
        elwindui_core::graphics::AlignmentX::Center,
        elwindui_core::graphics::AlignmentY::Center,
    );
    assert_eq!(
        placed,
        elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0
        }
    );
}

#[test]
fn fitted_image_rect_contain_letterboxes_without_overflowing_dest() {
    let dest = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    // A 200x100 (2:1) image `Contain`ed into a 100x100 square must shrink to fit the narrower
    // axis (height), leaving horizontal letterboxing rather than overflowing either axis.
    let placed = fitted_image_rect(
        dest,
        (200.0, 100.0),
        elwindui_core::graphics::ImageFit::Contain,
        elwindui_core::graphics::AlignmentX::Center,
        elwindui_core::graphics::AlignmentY::Center,
    );
    assert_eq!(placed.width, 100.0);
    assert_eq!(placed.height, 50.0);
    assert_eq!(placed.x, 0.0);
    assert_eq!(placed.y, 25.0);
}

#[test]
fn fitted_image_rect_cover_fills_dest_and_overflows_the_wider_axis() {
    let dest = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    // The same 2:1 image `Cover`ing a 100x100 square must grow to fill the *shorter* axis
    // (height), overflowing width — the opposite of `Contain`'s letterboxing.
    let placed = fitted_image_rect(
        dest,
        (200.0, 100.0),
        elwindui_core::graphics::ImageFit::Cover,
        elwindui_core::graphics::AlignmentX::Center,
        elwindui_core::graphics::AlignmentY::Center,
    );
    assert_eq!(placed.width, 200.0);
    assert_eq!(placed.height, 100.0);
    assert_eq!(placed.x, -50.0);
    assert_eq!(placed.y, 0.0);
}

#[test]
fn fitted_image_rect_none_draws_at_intrinsic_size_and_honors_alignment() {
    let dest = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    let placed = fitted_image_rect(
        dest,
        (30.0, 20.0),
        elwindui_core::graphics::ImageFit::None,
        elwindui_core::graphics::AlignmentX::Right,
        elwindui_core::graphics::AlignmentY::Bottom,
    );
    assert_eq!(placed.width, 30.0);
    assert_eq!(placed.height, 20.0);
    assert_eq!(placed.x, 70.0);
    assert_eq!(placed.y, 80.0);
}

// The remaining tests below extend coverage toward painter design doc §20.2's ~19-scene
// checklist (only the 4 tests above existed before this pass). Not covered by this lightweight
// harness (a bare `CALayer` fed straight to the drawing helpers, no `TreeHostView`/real window):
// native-control/painted-content Z-order interleaving — that needs a real `NSView` subview
// hierarchy, out of reach here without much heavier test infrastructure. Also not covered:
// clockwise/counterclockwise arc sweep — `path_to_cgpath`'s own doc comment already documents
// `PathCommand::ArcTo` as unrendered on this backend (a known gap, not something this test pass
// introduced), so a "does the sweep direction change the rendered shape" test would just fail
// against that pre-existing gap rather than exercising real behavior.

#[test]
fn rounded_rect_applies_each_corner_radius_independently() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let rect = elwindui_core::base::Rect {
        x: 8.0,
        y: 8.0,
        width: 48.0,
        height: 48.0,
    };
    // `top_left` (the (rect.x, rect.y) corner — see `PathBuilder::add_rounded_rect`) stays
    // sharp; the other three corners are rounded.
    let radii = elwindui_core::base::CornerRadius {
        top_left: 0.0,
        top_right: 20.0,
        bottom_right: 20.0,
        bottom_left: 20.0,
    };
    let path = rounded_rect_cgpath(&world, rect, radii);
    add_shape_layer(
        &root,
        &path,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 200, 0),
        )),
        None,
        1.0,
        rect,
    );
    render_layer(&root, &bitmap);
    // The sharp (radius 0) corner is painted right up to (rect.x, rect.y) — `render_layer`'s
    // own Y-flip note applies (logical y=9 lands near pixel row 64-9=55).
    approx(bitmap.pixel(9, 55), (0, 200, 0, 255), 50);
    // The rounded (radius 20) opposite corner stays unpainted this close to (x+w, y+h).
    approx(bitmap.pixel(55, 9), (0, 0, 0, 0), 10);
}

#[test]
fn line_cap_butt_does_not_extend_past_the_segment_endpoint() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let path = CGMutablePath::new();
    unsafe {
        CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 16.0, 32.0);
        CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 48.0, 32.0);
    }
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 10.0,
        start_cap: elwindui_core::graphics::LineCap::Butt,
        end_cap: elwindui_core::graphics::LineCap::Butt,
        ..Default::default()
    };
    let bounds = elwindui_core::base::Rect {
        x: 16.0,
        y: 27.0,
        width: 32.0,
        height: 10.0,
    };
    add_shape_layer(
        &root,
        &path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        bounds,
    );
    render_layer(&root, &bitmap);
    // Well inside the segment: painted.
    approx(bitmap.pixel(32, 32), (0, 0, 0, 255), 50);
    // 3px beyond the endpoint at x=16 — a butt cap stops exactly at the endpoint, so this
    // stays unpainted.
    approx(bitmap.pixel(13, 32), (0, 0, 0, 0), 10);
}

#[test]
fn line_cap_round_extends_past_the_segment_endpoint() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let path = CGMutablePath::new();
    unsafe {
        CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 16.0, 32.0);
        CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 48.0, 32.0);
    }
    // Half the 10.0 stroke width is 5.0, so a round cap extends ~5px past x=16 — well past
    // the same x=13 sample point a butt cap (the test above) leaves unpainted.
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 10.0,
        start_cap: elwindui_core::graphics::LineCap::Round,
        end_cap: elwindui_core::graphics::LineCap::Round,
        ..Default::default()
    };
    let bounds = elwindui_core::base::Rect {
        x: 16.0,
        y: 27.0,
        width: 32.0,
        height: 10.0,
    };
    add_shape_layer(
        &root,
        &path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        bounds,
    );
    render_layer(&root, &bitmap);
    approx(bitmap.pixel(13, 32), (0, 0, 0, 255), 80);
}

/// Builds a narrow, acute-angled "V" (two segments meeting at `(32, 10)`, opening downward)
/// stroked with `join`/`miter_limit` — shared by the miter/bevel/miter-limit tests below, since
/// they only differ in that one `StrokeStyle`.
fn stroke_acute_v(join: elwindui_core::graphics::LineJoin, miter_limit: f32) -> (u8, u8, u8, u8) {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let path = CGMutablePath::new();
    unsafe {
        CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 10.0, 50.0);
        CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 32.0, 10.0);
        CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 54.0, 50.0);
    }
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 8.0,
        line_join: join,
        miter_limit,
        ..Default::default()
    };
    let bounds = elwindui_core::base::Rect {
        x: 10.0,
        y: 10.0,
        width: 44.0,
        height: 40.0,
    };
    add_shape_layer(
        &root,
        &path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        bounds,
    );
    render_layer(&root, &bitmap);
    // Between the bevel's flat cut (~y=6.5) and the full miter tip (~y=1.7) along the
    // vertex's outward bisector — a miter join reaches this point, a bevel join does not.
    // `render_layer`'s own Y-flip note applies (logical y=4 lands near pixel row 64-4=60).
    bitmap.pixel(32, 60)
}

#[test]
fn line_join_miter_extends_the_outer_corner_of_an_acute_angle() {
    // Default `miter_limit` (10.0) comfortably exceeds this vertex's own ~2.07 ratio, so the
    // join renders as a true miter.
    approx(
        stroke_acute_v(elwindui_core::graphics::LineJoin::Miter, 10.0),
        (0, 0, 0, 255),
        80,
    );
}

#[test]
fn line_join_bevel_does_not_extend_the_outer_corner_of_an_acute_angle() {
    approx(
        stroke_acute_v(elwindui_core::graphics::LineJoin::Bevel, 10.0),
        (0, 0, 0, 0),
        10,
    );
}

#[test]
fn miter_limit_below_the_vertex_ratio_forces_a_bevel_style_corner() {
    // This vertex needs a miter-length/half-width ratio of ~2.07; 1.5 falls short, so even a
    // `LineJoin::Miter` request must fall back to a bevel-style flat corner.
    approx(
        stroke_acute_v(elwindui_core::graphics::LineJoin::Miter, 1.5),
        (0, 0, 0, 0),
        10,
    );
}

#[test]
fn dash_pattern_alternates_on_and_off_segments_along_the_line() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let path = CGMutablePath::new();
    unsafe {
        CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 4.0, 32.0);
        CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 60.0, 32.0);
    }
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 6.0,
        dash_pattern: std::sync::Arc::from([8.0, 8.0]),
        ..Default::default()
    };
    let bounds = elwindui_core::base::Rect {
        x: 4.0,
        y: 29.0,
        width: 56.0,
        height: 6.0,
    };
    add_shape_layer(
        &root,
        &path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        bounds,
    );
    render_layer(&root, &bitmap);
    // [4, 12) is the first "on" segment.
    approx(bitmap.pixel(8, 32), (0, 0, 0, 255), 50);
    // [12, 20) is the first "off" gap.
    approx(bitmap.pixel(16, 32), (0, 0, 0, 0), 10);
}

#[test]
fn dash_offset_shifts_the_on_off_phase_along_the_line() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let path = CGMutablePath::new();
    unsafe {
        CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 4.0, 32.0);
        CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 60.0, 32.0);
    }
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 6.0,
        dash_pattern: std::sync::Arc::from([8.0, 8.0]),
        dash_offset: 8.0,
        ..Default::default()
    };
    let bounds = elwindui_core::base::Rect {
        x: 4.0,
        y: 29.0,
        width: 56.0,
        height: 6.0,
    };
    add_shape_layer(
        &root,
        &path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        bounds,
    );
    render_layer(&root, &bitmap);
    // With no offset, x=8 sits in the first "on" segment (the test above). Shifting the phase
    // by a full dash period (8.0) flips it to "off".
    approx(bitmap.pixel(8, 32), (0, 0, 0, 0), 10);
}

/// The path shared by the `NonZero`/`EvenOdd` tests below: two 30x30 squares, sharing the same
/// winding order, overlapping in their bottom-right/top-left quadrant.
fn two_overlapping_same_winding_squares() -> elwindui_core::graphics::Path {
    let mut builder = elwindui_core::graphics::PathBuilder::new();
    builder.add_rect(elwindui_core::base::Rect {
        x: 10.0,
        y: 10.0,
        width: 30.0,
        height: 30.0,
    });
    builder.add_rect(elwindui_core::base::Rect {
        x: 25.0,
        y: 25.0,
        width: 30.0,
        height: 30.0,
    });
    builder.build().expect("two rects is never an empty path")
}

#[test]
fn nonzero_fill_rule_fills_the_overlap_of_two_same_winding_subpaths() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let path = two_overlapping_same_winding_squares();
    let cg_path = path_to_cgpath(&world, &path);
    let shape_layer = CAShapeLayer::new();
    shape_layer.setPath(Some(&cg_path));
    shape_layer.setFillRule(unsafe { kCAFillRuleNonZero });
    apply_fill(
        &shape_layer,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 150, 0),
        )),
        path.bounds(),
    );
    shape_layer.setOpacity(1.0);
    let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
    root.addSublayer(&shape_layer);
    render_layer(&root, &bitmap);
    approx(bitmap.pixel(32, 32), (0, 150, 0, 255), 50); // overlap: two windings, still filled
    approx(bitmap.pixel(15, 49), (0, 150, 0, 255), 50); // first square only (Y-flipped)
}

#[test]
fn evenodd_fill_rule_punches_a_hole_where_two_same_winding_subpaths_overlap() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let path = two_overlapping_same_winding_squares();
    let cg_path = path_to_cgpath(&world, &path);
    let shape_layer = CAShapeLayer::new();
    shape_layer.setPath(Some(&cg_path));
    shape_layer.setFillRule(unsafe { kCAFillRuleEvenOdd });
    apply_fill(
        &shape_layer,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 150, 0),
        )),
        path.bounds(),
    );
    shape_layer.setOpacity(1.0);
    let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
    root.addSublayer(&shape_layer);
    render_layer(&root, &bitmap);
    approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 10); // overlap: even crossing count -> a hole
    approx(bitmap.pixel(15, 49), (0, 150, 0, 255), 50); // first square only: still filled (Y-flipped)
}

#[test]
fn quadratic_bezier_bows_away_from_the_straight_chord_between_its_endpoints() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let mut builder = elwindui_core::graphics::PathBuilder::new();
    builder.move_to(elwindui_core::base::Point { x: 10.0, y: 50.0 });
    builder.quad_to(
        elwindui_core::base::Point { x: 32.0, y: 10.0 },
        elwindui_core::base::Point { x: 54.0, y: 50.0 },
    );
    let path = builder
        .build()
        .expect("a moved-to, curved path is never empty");
    let cg_path = path_to_cgpath(&world, &path);
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 6.0,
        ..Default::default()
    };
    add_shape_layer(
        &root,
        &cg_path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        path.bounds(),
    );
    render_layer(&root, &bitmap);
    // The curve's own midpoint (t=0.5) sits at (32, 30) — nowhere near the straight chord's
    // midpoint (32, 50), proving the quadratic control point actually bent the curve.
    // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
    approx(bitmap.pixel(32, 34), (0, 0, 0, 255), 50);
    approx(bitmap.pixel(32, 14), (0, 0, 0, 0), 10);
}

#[test]
fn cubic_bezier_bows_away_from_the_straight_chord_between_its_endpoints() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let mut builder = elwindui_core::graphics::PathBuilder::new();
    builder.move_to(elwindui_core::base::Point { x: 10.0, y: 50.0 });
    builder.cubic_to(
        elwindui_core::base::Point { x: 20.0, y: 10.0 },
        elwindui_core::base::Point { x: 44.0, y: 10.0 },
        elwindui_core::base::Point { x: 54.0, y: 50.0 },
    );
    let path = builder
        .build()
        .expect("a moved-to, curved path is never empty");
    let cg_path = path_to_cgpath(&world, &path);
    let stroke = elwindui_core::graphics::StrokeStyle {
        width: 6.0,
        ..Default::default()
    };
    add_shape_layer(
        &root,
        &cg_path,
        None,
        Some((
            &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
            &stroke,
        )),
        1.0,
        path.bounds(),
    );
    render_layer(&root, &bitmap);
    // The curve's own midpoint (t=0.5) sits at (32, 20) — nowhere near the straight chord's
    // midpoint (32, 50), proving both control points actually bent the curve.
    // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
    approx(bitmap.pixel(32, 44), (0, 0, 0, 255), 50);
    approx(bitmap.pixel(32, 14), (0, 0, 0, 0), 10);
}

#[test]
fn linear_gradient_interpolates_between_its_two_stop_colors() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let rect = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 64.0,
        height: 64.0,
    };
    let brush = elwindui_core::graphics::Brush::LinearGradient(
        elwindui_core::graphics::LinearGradientBrush::new(
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            elwindui_core::base::Point { x: 1.0, y: 0.0 },
            vec![
                elwindui_core::graphics::GradientStop::new(
                    0.0,
                    elwindui_core::graphics::Color::rgb(255, 0, 0),
                )
                .unwrap(),
                elwindui_core::graphics::GradientStop::new(
                    1.0,
                    elwindui_core::graphics::Color::rgb(0, 0, 255),
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    );
    let world = elwindui_core::base::AffineTransform::identity();
    let realized = try_add_gradient_fill_layer(
        &root,
        &brush,
        rect,
        GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()),
        &world,
        1.0,
    );
    assert!(
        realized,
        "a pure-translation world must realize a gradient brush as a real CAGradientLayer"
    );
    render_layer(&root, &bitmap);
    approx(bitmap.pixel(4, 32), (255, 0, 0, 255), 80); // near the left edge: close to stop 0
    approx(bitmap.pixel(60, 32), (0, 0, 255, 255), 80); // near the right edge: close to stop 1
}

#[test]
fn radial_gradient_interpolates_from_center_to_edge() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let rect = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 64.0,
        height: 64.0,
    };
    let brush = elwindui_core::graphics::Brush::RadialGradient(
        elwindui_core::graphics::RadialGradientBrush::new(
            elwindui_core::base::Point { x: 0.5, y: 0.5 },
            0.5,
            0.5,
            vec![
                elwindui_core::graphics::GradientStop::new(
                    0.0,
                    elwindui_core::graphics::Color::rgb(255, 0, 0),
                )
                .unwrap(),
                elwindui_core::graphics::GradientStop::new(
                    1.0,
                    elwindui_core::graphics::Color::rgb(0, 0, 255),
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    );
    let world = elwindui_core::base::AffineTransform::identity();
    let realized =
        try_add_gradient_fill_layer(&root, &brush, rect, GradientMaskShape::Ellipse, &world, 1.0);
    assert!(realized);
    render_layer(&root, &bitmap);
    approx(bitmap.pixel(32, 32), (255, 0, 0, 255), 60); // center: close to stop 0
    approx(bitmap.pixel(32, 4), (0, 0, 255, 255), 90); // near the edge: close to stop 1
}

#[test]
fn draw_image_contain_letterboxes_and_leaves_the_gap_unpainted() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    // A 20x10 solid-blue image `Contain`ed into a 20x20 square must shrink to fit the width
    // (already exact) while the height (half of the square) leaves 5px letterbox gaps above
    // and below, centered by default alignment.
    let pixels = vec![0u8, 0, 255, 255].repeat(20 * 10);
    let image = elwindui_core::graphics::Image::from_rgba8(
        20,
        10,
        20 * 4,
        pixels,
        elwindui_core::graphics::AlphaMode::Opaque,
    )
    .expect("valid RGBA8 buffer");
    let mut image_cache = HashMap::new();
    let resolved = resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
    let dest = elwindui_core::base::Rect {
        x: 2.0,
        y: 2.0,
        width: 20.0,
        height: 20.0,
    };
    let options = elwindui_core::graphics::ImageDrawOptions {
        fit: elwindui_core::graphics::ImageFit::Contain,
        ..Default::default()
    };
    let world = elwindui_core::base::AffineTransform::identity();
    let container = build_image_container_layer(&resolved, dest, None, &options, &world, 1.0)
        .expect("no source crop means there's always something to draw");
    assert!(
        unsafe { container.sublayers() }.is_none_or(|layers| layers.is_empty()),
        "a fully contained image must be represented by one direct image layer"
    );
    root.addSublayer(&container);
    render_layer(&root, &bitmap);
    // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
    approx(bitmap.pixel(12, 52), (0, 0, 255, 255), 50); // inside the letterboxed image
    approx(bitmap.pixel(12, 60), (0, 0, 0, 0), 10); // top letterbox gap: left unpainted
}

#[test]
fn transformed_image_uses_a_container_layer() {
    let image = elwindui_core::graphics::Image::from_rgba8(
        1,
        1,
        4,
        vec![0, 0, 255, 255],
        elwindui_core::graphics::AlphaMode::Opaque,
    )
    .expect("valid RGBA8 buffer");
    let mut image_cache = HashMap::new();
    let resolved = resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
    let dest = elwindui_core::base::Rect {
        x: -10.0,
        y: -10.0,
        width: 20.0,
        height: 20.0,
    };
    let world = elwindui_core::base::AffineTransform::translation(32.0, 32.0)
        .concat(&elwindui_core::base::AffineTransform::rotation(0.25));
    let layer = build_image_container_layer(
        &resolved,
        dest,
        None,
        &elwindui_core::graphics::ImageDrawOptions::default(),
        &world,
        1.0,
    )
    .expect("the image is drawable");
    assert_eq!(
        unsafe { layer.sublayers() }.map_or(0, |layers| layers.len()),
        1,
        "a transformed image keeps its local-coordinate container"
    );
}

#[test]
fn resolving_cloned_images_keeps_one_decoded_cache_entry() {
    let image = elwindui_core::graphics::Image::from_rgba8(
        1,
        1,
        4,
        vec![0, 0, 0, 255],
        elwindui_core::graphics::AlphaMode::Opaque,
    )
    .expect("valid RGBA8 buffer");
    let mut image_cache = HashMap::new();
    let first = resolve_cgimage(&image, &mut image_cache).expect("first decode succeeds");
    let second =
        resolve_cgimage(&image.clone(), &mut image_cache).expect("clone cache hit succeeds");
    assert_eq!(image_cache.len(), 1);
    assert_eq!(
        objc2_core_foundation::CFRetained::as_ptr(&first),
        objc2_core_foundation::CFRetained::as_ptr(&second),
        "a clone must reuse the same decoded CGImage"
    );
}

#[test]
fn draw_image_source_crop_only_shows_the_cropped_region() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    // A 2x1 image: left pixel red, right pixel blue.
    let pixels = vec![255u8, 0, 0, 255, 0, 0, 255, 255];
    let image = elwindui_core::graphics::Image::from_rgba8(
        2,
        1,
        2 * 4,
        pixels,
        elwindui_core::graphics::AlphaMode::Opaque,
    )
    .expect("valid RGBA8 buffer");
    let mut image_cache = HashMap::new();
    let resolved = resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
    let dest = elwindui_core::base::Rect {
        x: 2.0,
        y: 2.0,
        width: 20.0,
        height: 20.0,
    };
    // Crop to just the right (blue) pixel.
    let source = elwindui_core::base::Rect {
        x: 1.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let options = elwindui_core::graphics::ImageDrawOptions {
        fit: elwindui_core::graphics::ImageFit::Fill,
        ..Default::default()
    };
    let world = elwindui_core::base::AffineTransform::identity();
    let container =
        build_image_container_layer(&resolved, dest, Some(source), &options, &world, 1.0)
            .expect("the crop rect is fully inside the image, not an empty intersection");
    root.addSublayer(&container);
    render_layer(&root, &bitmap);
    // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
    approx(bitmap.pixel(12, 52), (0, 0, 255, 255), 50);
}

// The two tests below exercise nested `PushTransform`/`PushOpacity` *composition* — but not
// through `replay_commands`'s own Push/Pop recursion itself: that needs a real `&TreeHostView`
// (its `NativeControl` arm touches `host.ivars()`), and constructing one (`TreeHostView::new`)
// asserts the calling thread is the app's main thread, which `cargo test`'s worker-thread pool
// never is. Instead, each test computes the exact composed `AffineTransform`/`opacity`
// `replay_commands`' `PushTransform`/`PushOpacity` arms would produce (`transform.concat
// (pushed)`, `opacity * pushed` — see those arms' own source) and feeds it straight to
// `rounded_rect_cgpath`/`add_shape_layer`, the same one-level-below approach every other test
// in this module already uses.

#[test]
fn nested_push_transform_composes_both_transforms_in_order() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let outer = elwindui_core::base::AffineTransform::translation(20.0, 0.0);
    let inner = elwindui_core::base::AffineTransform::translation(0.0, 20.0);
    let world = outer.concat(&inner);
    let rect = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
    add_shape_layer(
        &root,
        &path,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 200, 0),
        )),
        None,
        1.0,
        rect,
    );
    render_layer(&root, &bitmap);
    // Both translations compose: the 10x10 rect, originally at (0,0), ends up at (20,20).
    // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
    approx(bitmap.pixel(25, 39), (0, 200, 0, 255), 50);
    approx(bitmap.pixel(5, 59), (0, 0, 0, 0), 10);
}

#[test]
fn nested_push_opacity_multiplies_both_levels() {
    let bitmap = Bitmap::new(64, 64);
    let root = CALayer::new();
    root.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(64.0, 64.0),
    ));
    let world = elwindui_core::base::AffineTransform::identity();
    let opacity = 0.5f32 * 0.5f32;
    let rect = elwindui_core::base::Rect {
        x: 16.0,
        y: 16.0,
        width: 32.0,
        height: 32.0,
    };
    let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
    add_shape_layer(
        &root,
        &path,
        Some(&elwindui_core::graphics::Brush::Solid(
            elwindui_core::graphics::Color::rgb(0, 255, 0),
        )),
        None,
        opacity,
        rect,
    );
    render_layer(&root, &bitmap);
    // The rect is centered on the canvas, so this sample point is Y-flip-invariant.
    let (_, _, _, a) = bitmap.pixel(32, 32);
    // 0.5 * 0.5 = 0.25 net opacity, far below what a single 0.5 level would give (~127) —
    // proving the two `PushOpacity` levels multiplied instead of only the inner (or outer)
    // value winning.
    assert!(
        a < 100,
        "nested 0.5*0.5 opacity should be far below ~127, got {a}"
    );
    assert!(
        a > 20,
        "nested opacity should still be visibly painted, got {a}"
    );
}

// A pixel-level `CATextLayer`/`renderInContext:` ink-coverage golden (default/bold+large/kerned)
// was attempted here and removed: off the real AppKit main thread — which `cargo test`'s worker
// threads are not, confirmed empirically (`MainThreadMarker::new()` returns `None` there) — actual
// glyph rasterization through `CATextLayer` intermittently deadlocked (`render/text.rs`'s own
// `NSAttributedString`-based *measurement* tests, which never rasterize a glyph, never showed
// this). Production code always runs on the true main thread under a live run loop
// (`elwindui-backend-appkit::app::run`), so this is a test-harness-only constraint, not a
// production bug — but it makes a headless pixel golden for real text rendering unreliable in this
// environment. `render/text.rs`'s `ns_font`/`measure_text` unit tests (weight/italic/stretch/
// fallback/growth/wrap/kerning) are the real-machine verification for this feature instead; see
// `docs/design/runtime/text_design.md` for the full record of what was tried and why.
