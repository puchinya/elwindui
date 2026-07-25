//! Decoding, cropping, and placing raster images: `elwindui_core::graphics::Image` ->
//! `CGImage`, plus the `masksToBounds` container layer a `DrawImage` command needs so a
//! `Cover`/`None` fit can overflow its destination rect without bleeding outside it.


use objc2::rc::Retained;
use objc2::{
    AnyThread,
};
use objc2_app_kit::NSImage;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGColorSpace, CGDataProvider, CGImage};
use objc2_foundation::{NSRect, NSString};
use objc2_quartz_core::{
    CALayer, kCAFilterLinear, kCAFilterNearest,
};
use std::collections::HashMap;

/// Crops `cg_image` to `source` (image-pixel coordinates, top-left origin — `CGImage::
/// with_image_in_rect`'s own convention for a raster image), clamped to the image's own bounds
/// first (painter design doc §13.2: "source が画像外にはみ出した場合は交差領域にクリップする").
/// `None` means "draw the image unchanged"; a `source` that clamps to an empty intersection means
/// "draw nothing", surfaced the same way (`None`) since both are indistinguishable to the caller
/// once resolved — `RenderCommand::DrawImage`'s handler treats either as "skip this command".
pub(crate) fn crop_cgimage(
    cg_image: &CFRetained<CGImage>,
    source: Option<elwindui_core::base::Rect>,
) -> Option<CFRetained<CGImage>> {
    let Some(source) = source else {
        return Some(cg_image.clone());
    };
    let image_bounds = elwindui_core::base::Rect {
        x: 0.0,
        y: 0.0,
        width: CGImage::width(Some(cg_image)) as f32,
        height: CGImage::height(Some(cg_image)) as f32,
    };
    let clamped = source.intersect(image_bounds)?;
    CGImage::with_image_in_rect(
        Some(cg_image),
        objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(clamped.x as f64, clamped.y as f64),
            objc2_core_foundation::CGSize::new(clamped.width as f64, clamped.height as f64),
        ),
    )
}

/// The `Rect` — in `dest`-relative local coordinates, i.e. `(0, 0)` is `dest`'s own top-left, the
/// coordinate space `RenderCommand::DrawImage`'s `masksToBounds` container layer uses for its
/// image sublayer — that `image_size` (the already-cropped image's own pixel dimensions) should
/// actually be drawn at once `fit`/`alignment_x`/`alignment_y` are applied. `Fill` always returns
/// `dest` reduced to `(0, 0)`-origin (unchanged from this command's pre-`fit` behavior);
/// `Contain`/`Cover` scale `image_size` to fit inside/cover `dest` while preserving its aspect
/// ratio; `None` draws at intrinsic size. Any leftover space (`Contain`/`None`) or overflow
/// (`Cover`/`None`) is distributed per `alignment_x`/`alignment_y` — overflow is why the caller
/// needs its own `masksToBounds` container rather than just handing this rect straight to `dest`'s
/// own layer.
pub(crate) fn fitted_image_rect(
    dest: elwindui_core::base::Rect,
    image_size: (f32, f32),
    fit: elwindui_core::graphics::ImageFit,
    alignment_x: elwindui_core::graphics::AlignmentX,
    alignment_y: elwindui_core::graphics::AlignmentY,
) -> elwindui_core::base::Rect {
    elwindui_core::graphics::fitted_image_rect(
        elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: dest.width,
            height: dest.height,
        },
        image_size,
        fit,
        alignment_x,
        alignment_y,
    )
}

/// Builds the `masksToBounds` container + inner image `CALayer` for one `RenderCommand::DrawImage`
/// — factored out of `replay_paint_command`'s own arm so `crop_cgimage`/`fitted_image_rect`'s
/// actual `CALayer` construction (not just their own pure-value-level unit tests) is directly
/// exercisable from `golden_tests` without needing a real `TreeHostView`/`NSView`. Returns `None`
/// when there's nothing to draw (`source` clamps to an empty crop against `resolved_cg_image`'s
/// own bounds).
pub(crate) fn build_image_container_layer(
    resolved_cg_image: &CFRetained<CGImage>,
    dest: elwindui_core::base::Rect,
    source: Option<elwindui_core::base::Rect>,
    options: &elwindui_core::graphics::ImageDrawOptions,
    world: &elwindui_core::base::AffineTransform,
    opacity: f32,
) -> Option<Retained<CALayer>> {
    let cg_image = crop_cgimage(resolved_cg_image, source)?;
    let image_size = (
        CGImage::width(Some(&cg_image)) as f32,
        CGImage::height(Some(&cg_image)) as f32,
    );
    let placed = fitted_image_rect(
        dest,
        image_size,
        options.fit,
        options.alignment_x,
        options.alignment_y,
    );

    // A `dest`-sized, `masksToBounds` container keeps `Cover`/`None` overflow (the placed image
    // can be larger than `dest`) from bleeding into neighboring content — `placed` is already
    // expressed in this container's own local (dest-relative) coordinate space, the same
    // re-anchoring `try_add_gradient_fill_layer`'s mask path uses for the same reason.
    //
    // `position`/`bounds`/`affineTransform` (not `setFrame`) is what actually lets this container
    // rotate/scale under a non-translation `world` — `setFrame` only ever places an *axis-aligned*
    // rect, so an earlier version of this function that transformed just `dest`'s origin point and
    // handed `setFrame` the untransformed `dest.width`/`dest.height` silently dropped any rotation
    // or scale in `world` (unlike every path-based paint command, which transforms each of its
    // path's points individually and so rotates/scales correctly). With `anchorPoint` left at
    // `CALayer`'s own default `(0.5, 0.5)`, `position` set to `world`'s image of `dest`'s *center*
    // and `bounds` set to `dest`'s own untransformed size, `affineTransform` only needs to carry
    // `world`'s linear part (`m11`/`m12`/`m21`/`m22` — translation is already folded into
    // `position` via the center point, and matrix composition keeps a transform's linear part
    // independent of any translation elsewhere in the chain, so reading it straight off `world` is
    // exact regardless of how `world` itself was built up). For a pure-translation `world` (the
    // common case) this reduces to exactly the old `setFrame` placement: identity linear part plus
    // a `position` that is `dest`'s translated center.
    let container = CALayer::new();
    container.setName(Some(&NSString::from_str("elwindui-paint")));
    container.setMasksToBounds(true);
    container.setBounds(objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(dest.width as f64, dest.height as f64),
    ));
    let center_absolute = world.transform_point(elwindui_core::base::Point {
        x: dest.x + dest.width / 2.0,
        y: dest.y + dest.height / 2.0,
    });
    container.setPosition(objc2_core_foundation::CGPoint::new(
        center_absolute.x as f64,
        center_absolute.y as f64,
    ));
    container.setAffineTransform(objc2_core_foundation::CGAffineTransform {
        a: world.m11 as f64,
        b: world.m12 as f64,
        c: world.m21 as f64,
        d: world.m22 as f64,
        tx: 0.0,
        ty: 0.0,
    });

    let image_layer = CALayer::new();
    image_layer.setFrame(NSRect::new(
        objc2_foundation::NSPoint::new(placed.x as f64, placed.y as f64),
        objc2_foundation::NSSize::new(placed.width as f64, placed.height as f64),
    ));
    unsafe { image_layer.setContents(Some(cg_image.as_ref() as &objc2::runtime::AnyObject)) };
    let filter = match options.sampling {
        elwindui_core::graphics::ImageSampling::Nearest => unsafe { kCAFilterNearest },
        elwindui_core::graphics::ImageSampling::Linear | elwindui_core::graphics::ImageSampling::Cubic => unsafe {
            kCAFilterLinear
        },
    };
    image_layer.setMagnificationFilter(filter);
    image_layer.setMinificationFilter(filter);
    container.addSublayer(&image_layer);
    container.setOpacity(opacity);
    Some(container)
}

/// Resolves an `Image` to a `CGImage`, decoding at most once per distinct `Image` (`image_cache`,
/// keyed by the `Image`'s own `Arc` pointer identity — cheap and stable since `Image` is
/// `Arc`-backed and the same logical image reuses the same `Arc` across relayouts unless the
/// application constructs a fresh one).
pub(crate) fn resolve_cgimage(
    image: &elwindui_core::graphics::Image,
    cache: &mut HashMap<usize, CFRetained<CGImage>>,
) -> Option<CFRetained<CGImage>> {
    let key = image as *const _ as usize;
    if let Some(cached) = cache.get(&key) {
        return Some(cached.clone());
    }
    let decoded = decode_cgimage(image)?;
    cache.insert(key, decoded.clone());
    Some(decoded)
}

/// Releases the boxed pixel buffer `with_data` was given ownership of — `CGDataProvider::with_data`
/// takes raw `(info, data, size)` with no built-in ownership story of its own, so this callback is
/// what actually frees it once Core Graphics is done (as opposed to going through `NSData`/`CFData`
/// bridging, which would need a toll-free-bridging guarantee this crate version doesn't expose a
/// convenient safe path for).
pub(crate) unsafe extern "C-unwind" fn release_boxed_pixels(
    _info: *mut std::ffi::c_void,
    data: std::ptr::NonNull<std::ffi::c_void>,
    size: usize,
) {
    unsafe {
        drop(Vec::from_raw_parts(data.as_ptr() as *mut u8, size, size));
    }
}

pub(crate) fn decode_cgimage(image: &elwindui_core::graphics::Image) -> Option<CFRetained<CGImage>> {
    match image.data() {
        elwindui_core::graphics::ImageData::Rgba8 {
            width,
            height,
            stride,
            pixels,
            alpha,
        } => {
            let mut owned = pixels.to_vec().into_boxed_slice();
            let len = owned.len();
            let ptr = owned.as_mut_ptr();
            std::mem::forget(owned);
            let provider = unsafe {
                CGDataProvider::with_data(
                    std::ptr::null_mut(),
                    ptr as *const _,
                    len,
                    Some(release_boxed_pixels),
                )
            }?;
            let color_space = CGColorSpace::new_device_rgb()?;
            let alpha_info = match alpha {
                elwindui_core::graphics::AlphaMode::Opaque => {
                    objc2_core_graphics::CGImageAlphaInfo::NoneSkipLast
                }
                _ => objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast,
            };
            unsafe {
                CGImage::new(
                    *width as usize,
                    *height as usize,
                    8,
                    32,
                    *stride as usize,
                    Some(&color_space),
                    objc2_core_graphics::CGBitmapInfo(alpha_info.0 as _),
                    Some(&provider),
                    std::ptr::null(),
                    false,
                    objc2_core_graphics::CGColorRenderingIntent::RenderingIntentDefault,
                )
            }
        }
        elwindui_core::graphics::ImageData::Encoded { bytes, .. } => {
            let data = objc2_foundation::NSData::with_bytes(bytes);
            let ns_image = NSImage::initWithData(NSImage::alloc(), &data)?;
            let mut rect = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), ns_image.size());
            let cg_image = unsafe {
                ns_image.CGImageForProposedRect_context_hints(&mut rect as *mut NSRect, None, None)
            }?;
            // `NSImage.CGImageForProposedRect:context:hints:` returns an Objective-C-managed
            // `Retained<CGImage>` even though every other `CGImage` this backend produces is a
            // Core-Foundation-managed `CFRetained<CGImage>` — `CGImageRef` is toll-free bridged
            // with `id`, so the two retain/release mechanisms are the same underlying operation,
            // and handing the raw pointer straight from one wrapper to the other is sound.
            let ptr = std::ptr::NonNull::new(Retained::into_raw(cg_image))
                .expect("Retained is never null");
            Some(unsafe { CFRetained::from_raw(ptr) })
        }
        elwindui_core::graphics::ImageData::Backend(handle) => {
            handle.0.downcast_ref::<CFRetained<CGImage>>().cloned()
        }
    }
}
