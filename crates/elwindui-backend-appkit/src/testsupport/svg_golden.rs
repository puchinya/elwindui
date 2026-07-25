//! Golden tests for the vector renderer (`render::vector`), compared against `resvg`'s own
//! rasterization of the same SVG source.
//!
//! `RenderCommand::DrawVectorImage` golden tests (SVG読み込み・ベクター描画対応 実装指示書§22.8) —
//! same offscreen `CALayer.renderInContext` + sample-point-with-tolerance technique as
//! `golden_tests` above, cross-checked against `resvg`'s own rasterization of the same fixture SVG
//! (a dev-dependency only — see `vector_renderer.rs`'s own module doc comment on why production
//! rendering never touches `usvg`/`resvg`). Sample points are chosen on the canvas's own vertical
//! center line wherever possible, same reasoning `golden_tests`'s own doc comment gives for why
//! that's Y-flip-invariant and safe to compare directly against `CALayer.renderInContext`'s
//! flipped output without correcting for it.

use objc2_quartz_core::CALayer;
use std::collections::HashMap;
use crate::testsupport::bitmap::Bitmap;
use elwindui_core::graphics::VectorImageDrawOptions;


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
    
    use objc2_core_foundation::CFRetained;
    use objc2_core_graphics::CGImage;
    use std::collections::HashMap;
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
