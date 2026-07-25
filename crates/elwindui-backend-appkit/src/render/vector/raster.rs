//! Offscreen rasterization: turning a subtree, a `CALayer`, or a whole vector image into
//! pixels. Every path here is bounded by `MAX_OFFSCREEN_DIMENSION` so a pathological
//! mask/filter region cannot blow up memory.

use crate::render::image::release_boxed_pixels;
use elwindui_core::base::{AffineTransform, Point, Rect};
use elwindui_core::graphics::{
    VectorImage, VectorNode,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo};
use objc2_foundation::NSString;
use objc2_quartz_core::CALayer;
use std::collections::HashMap;

use super::*;

/// Rasterizes `image`'s `src_rect` region (in the image's own view-box units) into a fresh
/// `pixel_width × pixel_height` `CGImage`, for `VectorRasterizeMode::Auto`/`Fixed`
/// (`draw_vector_image`'s own doc comment). Reuses the exact same offscreen infrastructure
/// (`rasterize_calayer_to_pixels`/`pixels_to_cgimage`) mask/pattern/filter rendering already
/// depends on, and calls `render_group` unchanged — every feature that pipeline supports (masks,
/// patterns, filters, nested groups) therefore keeps working identically here, just rendered once
/// into a cached bitmap instead of left as a live `CALayer` tree.
pub(crate) fn rasterize_vector_image_to_cgimage(
    image: &VectorImage,
    src_rect: Rect,
    pixel_width: usize,
    pixel_height: usize,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
) -> Option<CFRetained<CGImage>> {
    if pixel_width == 0
        || pixel_height == 0
        || pixel_width > MAX_OFFSCREEN_DIMENSION
        || pixel_height > MAX_OFFSCREEN_DIMENSION
        || src_rect.width.abs() <= 1e-6
        || src_rect.height.abs() <= 1e-6
    {
        return None;
    }
    let scale_x = pixel_width as f32 / src_rect.width;
    let scale_y = pixel_height as f32 / src_rect.height;
    let root = CALayer::new();
    root.setBounds(CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(pixel_width as f64, pixel_height as f64),
    ));
    let src_to_pixel = AffineTransform::scale(scale_x, scale_y)
        .concat(&AffineTransform::translation(-src_rect.x, -src_rect.y));
    render_group(&root, image.root(), &src_to_pixel, 1.0, image_cache);
    let (pixels, width, height) = rasterize_calayer_to_pixels(&root, pixel_width, pixel_height)?;
    pixels_to_cgimage(pixels, width, height)
}

/// Decides whether `VectorRasterizeMode::Auto` needs to (re)rasterize, and at what size, given
/// the previous cache entry's size (if any, `cached`) and the size actually being requested now
/// (`requested`) — `draw_vector_image`'s own `Auto` arm. `None` means "reuse the existing cached
/// bitmap unchanged": shrinking (or an unchanged size) never triggers a rerasterization, since
/// `build_image_container_layer` downscales a larger cached bitmap to fit `dest` with no quality
/// loss (unlike upscaling, which would blur). `Some(size)` means a fresh rasterization is needed —
/// on growth, padded to 1.5x the *previous* cached size (rounded up) as long as the newly
/// requested size still fits under that margin, so a gradual, continuous enlargement (e.g. a live
/// window resize drag) doesn't force a rasterization on every single size change; a jump past the
/// 1.5x margin just rasterizes at the size actually requested, rather than over-allocating a
/// buffer that wouldn't even cover it.
pub(crate) fn auto_raster_target_size(cached: Option<(u32, u32)>, requested: (u32, u32)) -> Option<(u32, u32)> {
    let Some((cached_width, cached_height)) = cached else {
        return Some(requested);
    };
    if requested.0 <= cached_width && requested.1 <= cached_height {
        return None;
    }
    let margin_width = (cached_width as f32 * 1.5).ceil() as u32;
    let margin_height = (cached_height as f32 * 1.5).ceil() as u32;
    if requested.0 < margin_width && requested.1 < margin_height {
        Some((margin_width, margin_height))
    } else {
        Some(requested)
    }
}

#[cfg(test)]
mod auto_raster_target_size_tests {
    use super::auto_raster_target_size;

    #[test]
    fn no_existing_cache_rasterizes_at_the_requested_size() {
        assert_eq!(auto_raster_target_size(None, (100, 50)), Some((100, 50)));
    }

    #[test]
    fn shrinking_in_both_axes_reuses_the_existing_cache() {
        assert_eq!(auto_raster_target_size(Some((100, 100)), (50, 40)), None);
    }

    #[test]
    fn unchanged_size_reuses_the_existing_cache() {
        assert_eq!(auto_raster_target_size(Some((100, 100)), (100, 100)), None);
    }

    #[test]
    fn one_axis_shrinking_and_the_other_unchanged_is_not_growth() {
        assert_eq!(auto_raster_target_size(Some((100, 100)), (60, 100)), None);
    }

    #[test]
    fn growth_within_1_5x_margin_pads_to_1_5x_the_cached_size() {
        // 100 * 1.5 = 150, 80 * 1.5 = 120 — both requested dims (120, 100) sit strictly under
        // that margin, so the target is the margin itself, not the raw request.
        assert_eq!(auto_raster_target_size(Some((100, 80)), (120, 100)), Some((150, 120)));
    }

    #[test]
    fn growth_at_or_past_the_1_5x_margin_in_either_axis_uses_the_exact_requested_size() {
        // Width alone (200 >= 150) already exceeds its margin, even though height (90) doesn't.
        assert_eq!(auto_raster_target_size(Some((100, 80)), (200, 90)), Some((200, 90)));
    }

    #[test]
    fn margin_rounds_up_for_odd_cached_sizes() {
        // 101 * 1.5 = 151.5 -> ceil 152.
        assert_eq!(auto_raster_target_size(Some((101, 101)), (110, 110)), Some((152, 152)));
    }
}

/// Renders `children` (already in `local_rect`'s own coordinate space) into a fresh, appropriately
/// sized `CALayer` tree and rasterizes it to premultiplied top-down RGBA8 pixels. Returns `None`
/// for a degenerate or pathologically large region rather than allocating unboundedly.
pub(crate) fn rasterize_nodes_to_pixels(
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

pub(crate) fn rasterize_calayer_to_pixels(
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

pub(crate) fn pixels_to_cgimage(pixels: Vec<u8>, width: usize, height: usize) -> Option<CFRetained<CGImage>> {
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
pub(crate) fn place_offscreen_image(
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
