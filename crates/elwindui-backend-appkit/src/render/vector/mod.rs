//! `RenderCommand::DrawVectorImage` — the SVG scene renderer.
//!
//! Entry point is [`draw_vector_image`]; this file owns the node-tree walk (group nesting,
//! blend modes, transparency layers) and delegates the leaves: `paint` draws paths/rasters and
//! their fills, `raster` handles every offscreen-buffer path (masks, pattern tiles, filter
//! inputs, whole-image rasterization), and `filter` implements the Core Image filter chain.
//!
//! Sits under `render` (rather than beside it, as the old top-level `vector_renderer.rs` did)
//! so it can share `render`'s path/paint/image helpers without either side importing the other
//! — the arrangement that removed the original `inner` <-> `vector_renderer` cycle.

use crate::render::{
    add_sublayer_scaled, build_image_container_layer, clip_mask_layer, fitted_image_rect,
    paint_layer_name, set_mask_scaled,
};
use elwindui_core::base::{AffineTransform, Rect};
use elwindui_core::graphics::{
    Clip, FillRule, ImageDrawOptions, VectorBlendMode, VectorGroup, VectorImage,
    VectorImageDrawOptions, VectorImageId, VectorNode, VectorRasterizeMode,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_core_image::{CIContext, CIFilter};
use objc2_foundation::NSString;
use objc2_quartz_core::CALayer;
use std::collections::HashMap;

mod filter;
mod paint;
mod raster;

use filter::*;
use paint::*;
use raster::*;

pub(crate) use raster::{
    pixels_to_cgimage, rasterize_calayer_to_pixels, rasterize_vector_image_to_cgimage,
};

/// The largest offscreen buffer dimension (mask/pattern-tile/filter rasterization) allowed in
/// either axis — a defensive cap against a pathological `mask`/`filter` region blowing up memory,
/// independent of `elwindui-svg`'s own `SvgLimits` (which bounds the *source* document, not what a
/// particular backend chooses to rasterize it at).
pub(crate) const MAX_OFFSCREEN_DIMENSION: usize = 4096;

thread_local! {
    /// One `CIContext` reused for every filter-chain render on this (AppKit's single UI) thread,
    /// rather than a fresh one per call — `CIContext` construction sets up a real GPU/Metal
    /// rendering pipeline and Apple's own documentation calls it expensive enough to create once
    /// and reuse for the app's lifetime, not per render. `thread_local!` (rather than proving
    /// `Retained<CIContext>` is `Send`/`Sync`, which it generally isn't for an arbitrary
    /// Objective-C object) is sufficient since every caller here already runs on the main thread.
    static SHARED_CI_CONTEXT: Retained<CIContext> = unsafe { CIContext::context() };
}

/// A vector feature with no reasonable mapping onto this backend's native APIs — reported once
/// (debug builds only, matching `elwindui-backend-winui3`'s own `unsupported_command!`
/// convention) rather than silently dropped; the surrounding content still renders.
pub(crate) fn report_unsupported(feature: &str) {
    #[cfg(debug_assertions)]
    eprintln!("[elwindui-backend-appkit] unsupported VectorImage feature: {feature}");
    #[cfg(not(debug_assertions))]
    let _ = feature;
}

/// Entry point called from `inner.rs`'s `replay_paint_command` for `RenderCommand::DrawVectorImage`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_vector_image(
    layer: &Retained<CALayer>,
    image: &VectorImage,
    dest: Rect,
    source: Option<Rect>,
    options: &VectorImageDrawOptions,
    world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<VectorImageId, (u32, u32, u8, CFRetained<CGImage>)>,
) {
    let src_rect = source.unwrap_or_else(|| image.view_box());
    if src_rect.width <= 0.0 || src_rect.height <= 0.0 {
        return;
    }
    let combined_opacity = opacity * options.opacity;

    let container = if options.clip_to_dest {
        let clip_container = CALayer::new();
        clip_container.setName(Some(&paint_layer_name()));
        clip_container.setFrame(layer.bounds());
        // Attach before masking — `add_sublayer_scaled` stamps `clip_container`'s scale from
        // `layer` at attach time, which `set_mask_scaled` below needs already set to propagate
        // correctly onto `mask`.
        add_sublayer_scaled(layer, &clip_container);
        let mask = clip_mask_layer(world, &Clip::Rect(dest));
        set_mask_scaled(&clip_container, &mask);
        clip_container
    } else {
        layer.clone()
    };

    match options.rasterize {
        VectorRasterizeMode::Vector => {
            // Reuses the exact same `Fill`/`Contain`/`Cover`/`None` + alignment placement math
            // ordinary `DrawImage` uses (実装指示書§17) — `src_rect`'s size stands in for
            // `DrawImage`'s own `image_size` parameter.
            let placed = fitted_image_rect(
                dest,
                (src_rect.width, src_rect.height),
                options.fit,
                options.alignment_x,
                options.alignment_y,
            );
            let scale_x = if src_rect.width.abs() > 1e-6 {
                placed.width / src_rect.width
            } else {
                1.0
            };
            let scale_y = if src_rect.height.abs() > 1e-6 {
                placed.height / src_rect.height
            } else {
                1.0
            };
            let root_local = AffineTransform::translation(dest.x + placed.x, dest.y + placed.y)
                .concat(&AffineTransform::scale(scale_x, scale_y))
                .concat(&AffineTransform::translation(-src_rect.x, -src_rect.y));
            let root_world = world.concat(&root_local);
            render_group(
                &container,
                image.root(),
                &root_world,
                combined_opacity,
                image_cache,
            );
        }
        VectorRasterizeMode::Auto | VectorRasterizeMode::Fixed { .. } => {
            let cg_image = match options.rasterize {
                VectorRasterizeMode::Auto => {
                    // `dest`'s actually-displayed size (in points) times this layer's own
                    // `contentsScale` — `layer` here is always a layer already attached through
                    // `render::add_sublayer_scaled`, which stamps it down from
                    // `TreeHostView::backing_scale_factor` at attach time (Core Animation does
                    // *not* propagate `contentsScale` from a superlayer on its own; see
                    // `render::layer`'s doc comment), so `layer.contentsScale()` is authoritative
                    // here without this function needing its own screen/window lookup.
                    let placed = fitted_image_rect(
                        dest,
                        (src_rect.width, src_rect.height),
                        options.fit,
                        options.alignment_x,
                        options.alignment_y,
                    );
                    let scale = layer.contentsScale() as f32;
                    let requested = (
                        (placed.width * scale).round().max(1.0) as u32,
                        (placed.height * scale).round().max(1.0) as u32,
                    );
                    let cached_size = vector_raster_cache
                        .get(&image.id())
                        .map(|(w, h, _, _)| (*w, *h));
                    let shrink =
                        cached_size.is_some_and(|cached| is_materially_smaller(cached, requested));
                    let target = if shrink {
                        let entry = vector_raster_cache
                            .get_mut(&image.id())
                            .expect("cached size was present");
                        entry.2 = entry.2.saturating_add(1);
                        (entry.2 >= 3).then_some(requested)
                    } else {
                        if let Some(entry) = vector_raster_cache.get_mut(&image.id()) {
                            entry.2 = 0;
                        }
                        auto_raster_target_size(cached_size, requested)
                    };
                    match target {
                        None => vector_raster_cache
                            .get(&image.id())
                            .map(|(_, _, _, cg_image)| cg_image.clone())
                            .expect(
                                "cached_size was Some when auto_raster_target_size returned None",
                            ),
                        Some((target_width, target_height)) => {
                            let Some(cg_image) = rasterize_vector_image_to_cgimage(
                                image,
                                src_rect,
                                target_width as usize,
                                target_height as usize,
                                image_cache,
                            ) else {
                                return;
                            };
                            vector_raster_cache.insert(
                                image.id(),
                                (target_width, target_height, 0, cg_image.clone()),
                            );
                            cg_image
                        }
                    }
                }
                VectorRasterizeMode::Fixed {
                    pixel_width,
                    pixel_height,
                } => {
                    let cached = vector_raster_cache
                        .get(&image.id())
                        .filter(|(w, h, _, _)| *w == pixel_width && *h == pixel_height)
                        .map(|(_, _, _, cg_image)| cg_image.clone());
                    match cached {
                        Some(cg_image) => cg_image,
                        None => {
                            let Some(cg_image) = rasterize_vector_image_to_cgimage(
                                image,
                                src_rect,
                                pixel_width as usize,
                                pixel_height as usize,
                                image_cache,
                            ) else {
                                return;
                            };
                            vector_raster_cache.insert(
                                image.id(),
                                (pixel_width, pixel_height, 0, cg_image.clone()),
                            );
                            cg_image
                        }
                    }
                }
                VectorRasterizeMode::Vector => unreachable!(),
            };
            let image_options = ImageDrawOptions {
                opacity: options.opacity,
                fit: options.fit,
                alignment_x: options.alignment_x,
                alignment_y: options.alignment_y,
                ..Default::default()
            };
            if let Some(image_layer) =
                build_image_container_layer(&cg_image, dest, None, &image_options, world, opacity)
            {
                add_sublayer_scaled(&container, &image_layer);
            }
        }
    }
}

pub(crate) fn render_node(
    layer: &Retained<CALayer>,
    node: &VectorNode,
    world: &AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) {
    match node {
        VectorNode::Group(child) => render_group(layer, child, world, opacity, image_cache),
        VectorNode::Path(path_node) => {
            render_path_node(layer, path_node, world, opacity, image_cache)
        }
        VectorNode::RasterImage(raster_node) => {
            render_raster_node(layer, raster_node, world, opacity, image_cache)
        }
    }
}

/// Renders `group`'s own children into `target`, honoring `group.filters` — the "content" half of
/// [`render_group`]'s two-stage pipeline (content, then clip/mask/opacity/blend applied to it).
pub(crate) fn render_group_content(
    target: &Retained<CALayer>,
    group: &VectorGroup,
    world: &AffineTransform,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) {
    if group.filters.is_empty() {
        for child in group.children.iter() {
            render_node(target, child, world, 1.0, image_cache);
        }
    } else {
        render_filtered_content(target, &group.children, &group.filters, world, image_cache);
    }
}

/// `VectorGroup` traversal. Composition order matches SVG's own: render content (children,
/// through any `filters`) → clip-path → mask → opacity → blend-mode, then hand the fully
/// composited result to the caller as one sublayer of `layer`.
pub(crate) fn render_group(
    layer: &Retained<CALayer>,
    group: &VectorGroup,
    parent_world: &AffineTransform,
    parent_opacity: f32,
    image_cache: &mut HashMap<elwindui_core::graphics::ImageId, CFRetained<CGImage>>,
) {
    let world = parent_world.concat(&group.transform);

    // Groups that carry no visual meaning of their own (a bare organizational `<g>`, extremely
    // common in Illustrator/Figma exports — see the doc comment on `is_transparent_passthrough`)
    // are rendered straight into `layer` with no `CALayer` of their own: skipping this wrapper is
    // invisible to the final image but matters a lot for a document with thousands of such groups
    // (`elwind_chan.svg`'s 5864 paths are largely each wrapped in their own trivial `<g>`), since
    // every skipped wrapper is one fewer `CALayer` this backend must synchronously construct.
    if is_transparent_passthrough(group) {
        for child in group.children.iter() {
            render_node(layer, child, &world, parent_opacity, image_cache);
        }
        return;
    }

    let wrapper = CALayer::new();
    wrapper.setName(Some(&paint_layer_name()));
    wrapper.setFrame(layer.bounds());

    // clip-path gets its own inner layer so its mask slot doesn't collide with the SVG `mask`'s
    // own mask slot on `wrapper` below — `CALayer` only has one `.mask` property each.
    let content_target = if group.clip_path.is_some() {
        let content = CALayer::new();
        content.setName(Some(&paint_layer_name()));
        content.setFrame(layer.bounds());
        add_sublayer_scaled(&wrapper, &content);
        content
    } else {
        wrapper.clone()
    };

    render_group_content(&content_target, group, &world, image_cache);

    if let Some(clip_path) = &group.clip_path {
        let path = clip_path.to_path().transformed(world);
        let mask = clip_mask_layer(
            &AffineTransform::identity(),
            &Clip::Path {
                path,
                rule: FillRule::NonZero,
            },
        );
        set_mask_scaled(&content_target, &mask);
    }

    if let Some(mask) = &group.mask {
        if let Some(mask_layer) =
            build_mask_layer(mask, &world, layer.contentsScale() as f32, image_cache)
        {
            set_mask_scaled(&wrapper, &mask_layer);
        }
    }

    wrapper.setOpacity(parent_opacity * group.opacity);
    apply_blend_mode(&wrapper, group.blend_mode);

    add_sublayer_scaled(layer, &wrapper);
}

/// True when `group` itself contributes nothing to the final image beyond grouping its
/// children — no transform, full opacity, normal blending, no clip-path/mask/filter, and not an
/// isolated stacking context (`isolate` would need its own offscreen compositing pass to be
/// correct, which flattening away would break) — so `render_group` can hand its children straight
/// to the parent `CALayer` instead of allocating a `wrapper` (and possibly a second `clip_path`
/// content layer) purely to relay them unchanged.
pub(crate) fn is_transparent_passthrough(group: &VectorGroup) -> bool {
    group.transform == AffineTransform::IDENTITY
        && group.opacity == 1.0
        && group.blend_mode == VectorBlendMode::Normal
        && !group.isolate
        && group.clip_path.is_none()
        && group.mask.is_none()
        && group.filters.is_empty()
}

pub(crate) fn apply_blend_mode(layer: &CALayer, mode: VectorBlendMode) {
    let Some(name) = ci_blend_mode_filter_name(mode) else {
        return;
    };
    let filter = unsafe { CIFilter::filterWithName(&NSString::from_str(name)) };
    if let Some(filter) = filter {
        unsafe { layer.setCompositingFilter(Some(filter.as_ref() as &AnyObject)) };
    } else {
        report_unsupported("group blend mode (CIFilter unavailable)");
    }
}

pub(crate) fn ci_blend_mode_filter_name(mode: VectorBlendMode) -> Option<&'static str> {
    match mode {
        VectorBlendMode::Normal => None,
        VectorBlendMode::Multiply => Some("CIMultiplyBlendMode"),
        VectorBlendMode::Screen => Some("CIScreenBlendMode"),
        VectorBlendMode::Overlay => Some("CIOverlayBlendMode"),
        VectorBlendMode::Darken => Some("CIDarkenBlendMode"),
        VectorBlendMode::Lighten => Some("CILightenBlendMode"),
        VectorBlendMode::ColorDodge => Some("CIColorDodgeBlendMode"),
        VectorBlendMode::ColorBurn => Some("CIColorBurnBlendMode"),
        VectorBlendMode::HardLight => Some("CIHardLightBlendMode"),
        VectorBlendMode::SoftLight => Some("CISoftLightBlendMode"),
        VectorBlendMode::Difference => Some("CIDifferenceBlendMode"),
        VectorBlendMode::Exclusion => Some("CIExclusionBlendMode"),
        VectorBlendMode::Hue => Some("CIHueBlendMode"),
        VectorBlendMode::Saturation => Some("CISaturationBlendMode"),
        VectorBlendMode::Color => Some("CIColorBlendMode"),
        VectorBlendMode::Luminosity => Some("CILuminosityBlendMode"),
    }
}
