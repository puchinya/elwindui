//! Native-side AppKit plumbing — every type here is `Inner`-prefixed and, except for `AnyView`
//! itself (re-exported at the crate root; see `lib.rs`'s own doc comment), private to this crate.
//! `native_ui.rs` composes these as plain fields and calls into them; this module owns every bit
//! of genuinely AppKit-specific complexity (NSTextView delegates, tab strip bookkeeping, ...) so
//! `native_ui.rs` stays a thin, uniform "implement the core-side trait by delegating" layer.


mod button;
mod menu;
mod scroll_view;
mod tab_view;
mod text;
mod window;

pub(crate) use button::InnerButton;
pub(crate) use menu::{InnerMenu, InnerMenuBar, InnerMenuBarItem, InnerMenuItem};
pub(crate) use scroll_view::InnerScrollView;
pub(crate) use tab_view::{InnerTabView, TabChipImpl};
pub(crate) use text::{InnerPasswordBox, InnerTextArea, InnerTextBox};
pub(crate) use window::InnerWindow;



















































/// Offscreen golden-scene rendering tests (painter design doc §20.2) — renders a handful of
/// representative scenes into an in-memory `CGBitmapContext` via `CALayer.renderInContext`
/// (no window/screen involved, so no Screen Recording permission is needed and these run
/// headlessly in `cargo test`) and asserts specific sample pixels rather than diffing against a
/// checked-in reference PNG — a narrower, self-contained foundation for this class of test rather
/// than the full 24-scene cross-backend suite the design doc describes (WinUI3/GTK4 can't run on
/// this machine at all — see `docs/elwindui_implementation_status.md` — so a true cross-backend
/// image diff isn't achievable here regardless).
#[cfg(test)]
mod golden_tests {
    use objc2::rc::Retained;
    use crate::render::{GradientMaskShape, add_shape_layer, apply_fill, ellipse_cgpath,
        path_to_cgpath, rounded_rect_cgpath, try_add_gradient_fill_layer,
        try_add_image_fill_layer, build_image_container_layer, resolve_cgimage};
    use objc2_core_foundation::CFRetained;
    use objc2_quartz_core::{CALayer, kCAFillRuleEvenOdd, kCAFillRuleNonZero};
    use objc2_core_graphics::{CGImage, CGMutablePath};
    use objc2_quartz_core::CAShapeLayer;
    use std::collections::HashMap;
    use crate::render::fitted_image_rect;
    use objc2_core_graphics::CGColorSpace;
    use super::*;

    struct Bitmap {
        ctx: CFRetained<objc2_core_graphics::CGContext>,
        pixels: Box<[u8]>,
        width: usize,
        height: usize,
        bytes_per_row: usize,
    }

    impl Bitmap {
        fn new(width: usize, height: usize) -> Self {
            let bytes_per_row = width * 4;
            let mut pixels = vec![0u8; bytes_per_row * height].into_boxed_slice();
            let color_space = CGColorSpace::new_device_rgb().expect("device RGB color space");
            let bitmap_info = objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast.0
                | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
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
            }
            .expect("CGBitmapContextCreate");
            Self {
                ctx,
                pixels,
                width,
                height,
                bytes_per_row,
            }
        }

        fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
            assert!(x < self.width && y < self.height);
            let offset = y * self.bytes_per_row + x * 4;
            (
                self.pixels[offset],
                self.pixels[offset + 1],
                self.pixels[offset + 2],
                self.pixels[offset + 3],
            )
        }
    }

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
        assert_eq!(placed, elwindui_core::base::Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 });
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
    fn stroke_acute_v(
        join: elwindui_core::graphics::LineJoin,
        miter_limit: f32,
    ) -> (u8, u8, u8, u8) {
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
        let path = builder.build().expect("a moved-to, curved path is never empty");
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
        let path = builder.build().expect("a moved-to, curved path is never empty");
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
        let realized = try_add_gradient_fill_layer(
            &root,
            &brush,
            rect,
            GradientMaskShape::Ellipse,
            &world,
            1.0,
        );
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
        let resolved =
            resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
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
        root.addSublayer(&container);
        render_layer(&root, &bitmap);
        // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
        approx(bitmap.pixel(12, 52), (0, 0, 255, 255), 50); // inside the letterboxed image
        approx(bitmap.pixel(12, 60), (0, 0, 0, 0), 10); // top letterbox gap: left unpainted
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
        let resolved =
            resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
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
        assert!(a < 100, "nested 0.5*0.5 opacity should be far below ~127, got {a}");
        assert!(a > 20, "nested opacity should still be visibly painted, got {a}");
    }
}

/// `RenderCommand::DrawVectorImage` golden tests (SVG読み込み・ベクター描画対応 実装指示書§22.8) —
/// same offscreen `CALayer.renderInContext` + sample-point-with-tolerance technique as
/// `golden_tests` above, cross-checked against `resvg`'s own rasterization of the same fixture SVG
/// (a dev-dependency only — see `vector_renderer.rs`'s own module doc comment on why production
/// rendering never touches `usvg`/`resvg`). Sample points are chosen on the canvas's own vertical
/// center line wherever possible, same reasoning `golden_tests`'s own doc comment gives for why
/// that's Y-flip-invariant and safe to compare directly against `CALayer.renderInContext`'s
/// flipped output without correcting for it.
#[cfg(test)]
mod svg_golden_tests {
    use objc2::rc::Retained;
    use objc2_core_foundation::CFRetained;
    use objc2_core_graphics::CGImage;
    use objc2_quartz_core::CALayer;
    use std::collections::HashMap;
    use objc2_core_graphics::CGColorSpace;
    use super::*;
    use elwindui_core::graphics::VectorImageDrawOptions;

    struct Bitmap {
        ctx: CFRetained<objc2_core_graphics::CGContext>,
        pixels: Box<[u8]>,
        width: usize,
        height: usize,
        bytes_per_row: usize,
    }

    impl Bitmap {
        fn new(width: usize, height: usize) -> Self {
            let bytes_per_row = width * 4;
            let mut pixels = vec![0u8; bytes_per_row * height].into_boxed_slice();
            let color_space = CGColorSpace::new_device_rgb().expect("device RGB color space");
            #[allow(deprecated)]
            let bitmap_info = objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast.0
                | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
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
            }
            .expect("CGBitmapContextCreate");
            Self {
                ctx,
                pixels,
                width,
                height,
                bytes_per_row,
            }
        }

        fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
            assert!(x < self.width && y < self.height);
            let offset = y * self.bytes_per_row + x * 4;
            (
                self.pixels[offset],
                self.pixels[offset + 1],
                self.pixels[offset + 2],
                self.pixels[offset + 3],
            )
        }
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

    fn render_via_elwindui(svg: &str, size: usize) -> Bitmap {
        let image = elwindui_svg::load_svg_str(svg).expect("valid fixture SVG");
        let bitmap = Bitmap::new(size, size);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(size as f64, size as f64),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: size as f32,
            height: size as f32,
        };
        let mut cache = HashMap::new();
        let mut vector_raster_cache = HashMap::new();
        crate::render::draw_vector_image(
            &root,
            &image,
            dest,
            None,
            &VectorImageDrawOptions::default(),
            &world,
            1.0,
            &mut cache,
            &mut vector_raster_cache,
        );
        root.renderInContext(&bitmap.ctx);
        bitmap
    }

    fn render_via_resvg(svg: &str, size: u32) -> resvg::tiny_skia::Pixmap {
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_str(svg, &opt).expect("valid fixture SVG");
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("non-zero pixmap size");
        let tree_size = tree.size();
        let scale = (size as f32 / tree_size.width()).min(size as f32 / tree_size.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        pixmap
    }

    fn resvg_pixel(pixmap: &resvg::tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let c = pixmap.pixel(x, y).unwrap_or(resvg::tiny_skia::PremultipliedColorU8::TRANSPARENT);
        (c.red(), c.green(), c.blue(), c.alpha())
    }

    const SOLID_RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect x="16" y="16" width="32" height="32" fill="#ff0000"/></svg>"##;

    #[test]
    fn solid_rect_matches_resvg_at_center_and_is_transparent_outside() {
        let bitmap = render_via_elwindui(SOLID_RECT_SVG, 64);
        let reference = render_via_resvg(SOLID_RECT_SVG, 64);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 40);
        approx(bitmap.pixel(2, 2), (0, 0, 0, 0), 10);
    }

    const LINEAR_GRADIENT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stop-color="#0000ff"/>
            <stop offset="1" stop-color="#ffff00"/>
        </linearGradient></defs>
        <rect x="0" y="0" width="64" height="64" fill="url(#g)"/>
    </svg>"##;

    #[test]
    fn linear_gradient_matches_resvg_at_left_and_right_samples() {
        let bitmap = render_via_elwindui(LINEAR_GRADIENT_SVG, 64);
        let reference = render_via_resvg(LINEAR_GRADIENT_SVG, 64);
        // Both sample points sit on the vertical center row (y=32), which a horizontal-only
        // gradient never varies along — Y-flip-invariant, same reasoning as `golden_tests`'s own
        // sample point choices.
        for x in [4u32, 60u32] {
            approx(
                bitmap.pixel(x as usize, 32),
                resvg_pixel(&reference, x, 32),
                50,
            );
        }
    }

    const GROUP_OPACITY_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <g opacity="0.5"><rect x="16" y="16" width="32" height="32" fill="#00ff00"/></g>
    </svg>"##;

    #[test]
    fn group_opacity_matches_resvg_alpha_at_center() {
        let bitmap = render_via_elwindui(GROUP_OPACITY_SVG, 64);
        let reference = render_via_resvg(GROUP_OPACITY_SVG, 64);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 50);
    }

    const CLIP_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <defs><clipPath id="c"><circle cx="32" cy="32" r="16"/></clipPath></defs>
        <rect x="0" y="0" width="64" height="64" fill="#ff00ff" clip-path="url(#c)"/>
    </svg>"##;

    #[test]
    fn clip_path_matches_resvg_inside_the_circle_and_is_transparent_outside() {
        let bitmap = render_via_elwindui(CLIP_SVG, 64);
        let reference = render_via_resvg(CLIP_SVG, 64);
        // Wider tolerance than the other fixtures here: `CAShapeLayer`-mask compositing carries
        // more inherent AA/blending softness than a plain shape fill even at the mask's own
        // center, well away from its edge (empirically observed ~64/255 green-channel deviation at
        // this fixture's dead center) — still tight enough to catch a genuinely broken clip (e.g.
        // one that fails open/fully-transparent).
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 90);
        assert!(
            bitmap.pixel(2, 2).3 < 30,
            "outside the clipPath circle should be near-transparent, got alpha {}",
            bitmap.pixel(2, 2).3
        );
    }

    const PATTERN_TILE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <defs>
            <pattern id="p" x="0" y="0" width="8" height="8" patternUnits="userSpaceOnUse">
                <rect width="8" height="8" fill="#0000ff"/>
            </pattern>
        </defs>
        <rect x="0" y="0" width="64" height="64" fill="url(#p)"/>
    </svg>"##;

    #[test]
    fn pattern_fill_repeats_across_the_whole_shape_not_just_the_first_tile() {
        let bitmap = render_via_elwindui(PATTERN_TILE_SVG, 64);
        let reference = render_via_resvg(PATTERN_TILE_SVG, 64);
        // A single-tile-only implementation would leave everything outside the pattern's own
        // declared `[0,8)x[0,8)` tile fully transparent — sampling far from the origin (here, deep
        // into the 8th tile column/row) is exactly what distinguishes "repeats infinitely" from
        // "drawn once at its own position".
        for (x, y) in [(60usize, 60usize), (36, 4), (4, 36)] {
            let (_, _, b, a) = bitmap.pixel(x, y);
            assert!(
                a > 200 && b > 150,
                "expected an opaque blue tile at ({x},{y}), got rgba={:?}",
                bitmap.pixel(x, y)
            );
        }
        approx(bitmap.pixel(60, 60), resvg_pixel(&reference, 60, 60), 60);
    }

    const FE_COMPOSITE_XOR_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <filter id="f" x="0" y="0" width="64" height="64" filterUnits="userSpaceOnUse">
            <feFlood flood-color="#ff0000" result="a"/>
            <feFlood flood-color="#0000ff" result="b"/>
            <feComposite in="a" in2="b" operator="xor"/>
        </filter>
        <rect x="0" y="0" width="64" height="64" fill="#000000" filter="url(#f)"/>
    </svg>"##;

    #[test]
    fn fe_composite_xor_cancels_out_two_fully_overlapping_opaque_floods() {
        let bitmap = render_via_elwindui(FE_COMPOSITE_XOR_SVG, 64);
        let reference = render_via_resvg(FE_COMPOSITE_XOR_SVG, 64);
        // Two same-extent, fully opaque flood fills XOR'd together cancel out completely (each is
        // entirely "covered" by the other, so both `SourceOut` halves are empty) — a deterministic
        // outcome distinct from the old "treated as Over" fallback, which would show the top
        // (red) flood solidly instead.
        approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 40);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 40);
    }

    const FE_COMPOSITE_ARITHMETIC_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <filter id="f" x="0" y="0" width="64" height="64" filterUnits="userSpaceOnUse">
            <feFlood flood-color="#ff0000" result="a"/>
            <feFlood flood-color="#0000ff" result="b"/>
            <feComposite in="a" in2="b" operator="arithmetic" k1="0.5" k2="0.5" k3="0.5" k4="0"/>
        </filter>
        <rect x="0" y="0" width="64" height="64" fill="#000000" filter="url(#f)"/>
    </svg>"##;

    #[test]
    fn fe_composite_arithmetic_matches_resvg() {
        let bitmap = render_via_elwindui(FE_COMPOSITE_ARITHMETIC_SVG, 64);
        let reference = render_via_resvg(FE_COMPOSITE_ARITHMETIC_SVG, 64);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 40);
    }

    const FE_TILE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <filter id="f" x="0" y="0" width="64" height="64" filterUnits="userSpaceOnUse">
            <feFlood flood-color="#00ff00" result="flood"/>
            <feTile in="flood"/>
        </filter>
        <rect x="0" y="0" width="64" height="64" fill="#000000" filter="url(#f)"/>
    </svg>"##;

    #[test]
    fn fe_tile_filter_primitive_runs_without_error_and_preserves_flood_color() {
        // A full-region `feFlood` already covers the entire filter region (this pipeline doesn't
        // apply each primitive's own `x`/`y`/`width`/`height` subregion before feeding it to the
        // next primitive — a pre-existing simplification orthogonal to this test), so tiling it
        // is visually a no-op; this fixture's job is to confirm `CIAffineTile` accepts the
        // `NSValue`-boxed identity `inputTransform` without erroring and the color survives,
        // rather than demonstrating visible repetition (see `pattern_fill_repeats_...` above for
        // an infinite-repetition test where the tile source's extent isn't pipeline-constrained).
        let bitmap = render_via_elwindui(FE_TILE_SVG, 64);
        approx(bitmap.pixel(32, 32), (0, 255, 0, 255), 40);
    }

    /// `VectorRasterizeMode::Auto`/`Fixed`/`Vector` — the rasterize-and-cache draw modes
    /// (`vector_renderer.rs::draw_vector_image`'s own doc comment), tested against
    /// `vector_raster_cache` directly rather than pixel output (already covered by every test
    /// above, all of which now exercise `Auto`, the new default) — these instead confirm *when* a
    /// cached bitmap is reused vs. rebuilt.
    mod rasterize_mode {
        use super::*;
        use elwindui_core::graphics::VectorRasterizeMode;

        fn draw_into(
            image: &elwindui_core::graphics::VectorImage,
            dest: elwindui_core::base::Rect,
            rasterize: VectorRasterizeMode,
            image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
            vector_raster_cache: &mut HashMap<
                elwindui_core::graphics::VectorImageId,
                (u32, u32, CFRetained<CGImage>),
            >,
        ) {
            let root = CALayer::new();
            root.setBounds(objc2_core_foundation::CGRect::new(
                objc2_core_foundation::CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(64.0, 64.0),
            ));
            crate::render::draw_vector_image(
                &root,
                image,
                dest,
                None,
                &VectorImageDrawOptions {
                    rasterize,
                    ..Default::default()
                },
                &elwindui_core::base::AffineTransform::identity(),
                1.0,
                image_cache,
                vector_raster_cache,
            );
        }

        fn small_rect_image() -> elwindui_core::graphics::VectorImage {
            elwindui_svg::load_svg_str(SOLID_RECT_SVG).expect("valid fixture SVG")
        }

        fn dest(size: f32) -> elwindui_core::base::Rect {
            elwindui_core::base::Rect { x: 0.0, y: 0.0, width: size, height: size }
        }

        #[test]
        fn auto_mode_reuses_the_cached_bitmap_when_the_drawn_size_is_unchanged() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w1, h1, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w1, h1), (w2, h2));
            assert_eq!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "same size should reuse the exact same cached CGImage, not rasterize again"
            );
        }

        #[test]
        fn auto_mode_rerasterizes_at_the_exact_size_when_growth_jumps_past_the_1_5x_margin() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (_, _, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            // 128 >= 64 * 1.5 (96), so this isn't a "gradual" enlargement the margin should
            // absorb — the fresh rasterization lands exactly on the requested size.
            draw_into(&image, dest(128.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w2, h2), (128, 128));
            assert_ne!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "a growth past the 1.5x margin must trigger a fresh rasterization"
            );
        }

        #[test]
        fn auto_mode_never_rerasterizes_when_the_drawn_size_shrinks() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(128.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (_, _, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            // The larger bitmap is kept as-is — `build_image_container_layer` just downscales it
            // to fit the smaller `dest`, so there is nothing to gain from rerasterizing smaller.
            assert_eq!((w2, h2), (128, 128));
            assert_eq!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "shrinking the drawn size must never trigger a rerasterization"
            );
        }

        #[test]
        fn auto_mode_pads_a_gradual_enlargement_to_1_5x_and_then_reuses_that_padding() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            // 80 < 64 * 1.5 (96) — growth within the margin pads to 96, not the raw 80 requested.
            draw_into(&image, dest(80.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("padded rasterization cached");
            assert_eq!((w2, h2), (96, 96));
            // A further, still-modest enlargement that fits inside the 96x96 padding must reuse
            // it without rerasterizing — this is the whole point of padding on growth.
            draw_into(&image, dest(90.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w3, h3, cg3) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w3, h3), (96, 96));
            assert_eq!(
                CFRetained::as_ptr(&cg2),
                CFRetained::as_ptr(&cg3),
                "growth that still fits inside the padded bitmap must not rerasterize"
            );
        }

        #[test]
        fn fixed_mode_keeps_the_same_bitmap_across_a_dest_resize() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            let fixed = VectorRasterizeMode::Fixed { pixel_width: 32, pixel_height: 32 };
            draw_into(&image, dest(64.0), fixed, &mut image_cache, &mut cache);
            let (w1, h1, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            assert_eq!((w1, h1), (32, 32));
            // A `dest` resize that would have changed `Auto`'s target pixel size must not affect
            // `Fixed` at all — that's the whole point of specifying a fixed rasterization size.
            draw_into(&image, dest(128.0), fixed, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w2, h2), (32, 32));
            assert_eq!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "Fixed mode must not rerasterize when only the display size changes"
            );
        }

        #[test]
        fn vector_mode_never_populates_the_raster_cache() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Vector, &mut image_cache, &mut cache);
            assert!(
                cache.is_empty(),
                "Vector mode should render the live CALayer tree, never touching the raster cache"
            );
        }
    }
}
