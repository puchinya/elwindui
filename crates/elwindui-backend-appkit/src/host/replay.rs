//! The `RenderGroup`/`RenderCommand` -> `CALayer` replay pass, plus its per-group cache key.
//!
//! Lives under `host` rather than `render` because it reads and writes four `TreeHostView`
//! ivars (`group_layers`, `group_layer_cache_keys`, `group_native_controls`,
//! `native_containers`) — it is this view's rendering pass, not stateless translation.


use crate::ffi::{AnyView, mtm};
use crate::render::{
    GradientMaskShape,
    add_shape_layer,
    apply_fill,
    apply_stroke,
    build_image_container_layer,
    clip_bounds,
    clip_mask_layer,
    ellipse_cgpath,
    geometry_bounds,
    path_to_cgpath,
    resolve_cgimage,
    rounded_rect_cgpath,
    transform_point,
    try_add_gradient_fill_layer,
    try_add_image_fill_layer,
};
use elwindui_core::graphics::{RenderCommand, RenderGroup};
use objc2::rc::Retained;
use objc2::DefinedClass;
use objc2_app_kit::NSView;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGImage, CGMutablePath};
use objc2_foundation::{NSRect, NSString};
use objc2_quartz_core::{
    CALayer, CAShapeLayer, CATextLayer,
    kCAFillRuleEvenOdd, kCAFillRuleNonZero,
};
use std::collections::{HashMap, HashSet};

use super::*;

/// What `TreeHostIvars::group_layers[id]`'s sublayers were last rebuilt from — see that field's
/// own doc comment for why `RenderGroup::generation` alone isn't a sufficient cache key.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct GroupCacheKey {
    origin: elwindui_core::base::Point,
    clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    generation: u64,
}

/// One retained-render replay pass over a `RenderGroup` tree, appending real `CALayer`s to
/// `root_layer` (ordinary painted content) and real `NSView` islands to `host` (native controls),
/// in traversal order so both interleave in the correct Z order (painter design doc §14.2's
/// "single custom drawing surface" intent, adapted to AppKit's native layer-composition model
/// rather than a `NSView.draw(_:)`/`CGContext` replay — `CAShapeLayer`/`CAGradientLayer` already
/// cover fill/stroke/dash/cap/join/miter/gradient natively, so a full `CGContext`-based rewrite
/// would only add complexity without adding capability here). `transform`/`opacity` are plain
/// accumulators (composed/multiplied down the recursion, applied when building each leaf's own
/// geometry/`opacity` — not modeled as extra nested `CALayer`s, which would need fighting
/// `CALayer`'s anchor-point-relative transform semantics for no benefit) — `clip` is the one
/// state that genuinely needs geometry-level handling, done here as a simple bounding-box
/// intersection test (skip a leaf whose rect doesn't overlap `clip` at all) rather than true
/// per-pixel masking, mirroring `Shape::hit_test_content`'s own "whole bounding rect, not
/// per-pixel" simplification elsewhere in this codebase.
///
/// Each `RenderGroup` gets one persistent, cached container `CALayer` (`TreeHostIvars::
/// group_layers`) rather than a fresh throwaway one every pass — a *flat* sibling of every other
/// group's own container (`frame` always exactly `root_layer.bounds()`, deliberately not nested
/// to match the `RenderGroup` tree shape, so the absolute-canvas-coordinate geometry every leaf
/// drawing helper already bakes in stays valid unchanged; nesting would need re-deriving all of
/// that in per-container-local coordinates for no benefit). Re-adding an already-attached
/// container to `root_layer` every pass (regardless of whether its *content* is rebuilt) moves it
/// to the top of the sublayer list, which is enough on its own to keep Z-order correct across a
/// mix of rebuilt and cache-hit groups each frame — the actually expensive part
/// (`CGPath`/`CAShapeLayer`/`CAGradientLayer` construction) only happens when `GroupCacheKey`
/// shows this group's replay inputs actually changed since last time (painter design doc §15's
/// renderer cache, acceptance criterion 14: "画像・pathリソースを毎フレーム再生成しない").
#[allow(clippy::too_many_arguments)]
pub(crate) fn replay_group(
    host: &TreeHostView,
    root_layer: &Retained<CALayer>,
    group: &RenderGroup,
    origin: elwindui_core::base::Point,
    inherited_clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    live_native_controls: &mut HashSet<usize>,
    live_group_ids: &mut HashSet<u64>,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<elwindui_core::graphics::VectorImageId, (u32, u32, CFRetained<CGImage>)>,
) {
    let origin = elwindui_core::base::Point {
        x: origin.x + group.offset.x,
        y: origin.y + group.offset.y,
    };
    let group_clip = group.clip.map(|clip| elwindui_core::base::Rect {
        x: origin.x + clip.x,
        y: origin.y + clip.y,
        width: clip.width,
        height: clip.height,
    });
    let effective_clip = match (inherited_clip, group_clip) {
        (Some(a), Some(b)) => a.intersect(b),
        (Some(clip), None) | (None, Some(clip)) => Some(clip),
        (None, None) => None,
    };
    live_group_ids.insert(group.id);

    let is_new = !host.ivars().group_layers.borrow().contains_key(&group.id);
    let container = host
        .ivars()
        .group_layers
        .borrow_mut()
        .entry(group.id)
        .or_insert_with(|| {
            let c = CALayer::new();
            c.setName(Some(&NSString::from_str("elwindui-paint")));
            c
        })
        .clone();
    container.setFrame(root_layer.bounds());
    root_layer.addSublayer(&container);

    let key = GroupCacheKey {
        origin,
        clip: effective_clip,
        transform,
        opacity,
        generation: group.generation,
    };
    let stale =
        is_new || host.ivars().group_layer_cache_keys.borrow().get(&group.id) != Some(&key);
    if stale {
        if let Some(existing) = unsafe { container.sublayers() } {
            // `removeFromSuperlayer` while iterating `existing` (a live view onto `container`'s
            // own sublayer array, not a snapshot) trips Foundation's mutation-during-enumeration
            // guard — collect into a plain `Vec` first, then iterate that instead.
            let old: Vec<_> = existing.iter().collect();
            for sub in old {
                sub.removeFromSuperlayer();
            }
        }
        let native_controls_before: HashSet<usize> = live_native_controls.clone();
        replay_commands(
            host,
            &container,
            &group.commands,
            0,
            origin,
            effective_clip,
            transform,
            opacity,
            live_native_controls,
            image_cache,
            vector_raster_cache,
        );
        let discovered_native_controls: Vec<usize> = live_native_controls
            .difference(&native_controls_before)
            .copied()
            .collect();
        host.ivars()
            .group_native_controls
            .borrow_mut()
            .insert(group.id, discovered_native_controls);
        host.ivars()
            .group_layer_cache_keys
            .borrow_mut()
            .insert(group.id, key);
    } else if let Some(ids) = host.ivars().group_native_controls.borrow().get(&group.id) {
        live_native_controls.extend(ids);
    }

    for child in &group.children {
        replay_group(
            host,
            root_layer,
            child,
            origin,
            effective_clip,
            transform,
            opacity,
            live_native_controls,
            live_group_ids,
            image_cache,
            vector_raster_cache,
        );
    }
}

/// Replays one `RenderGroup`'s own (flat) command list, starting at `commands[start]`. A `Push*`
/// command recurses with the updated accumulator (`transform`/`opacity`) or (for `PushClip`, the
/// one state needing real geometry) an intersected `clip`; the matching `Pop*` — always the first
/// `Pop*` this recursive call sees, since `RenderContext`'s own `push_*`/`pop_*` pair 1:1 in LIFO
/// order regardless of *kind* (see `elwindui_core::graphics::context`'s `stack_depth` counter) —
/// ends that call and returns control to the caller's own loop. Returns the index just past the
/// consumed slice.
#[allow(clippy::too_many_arguments)]
pub(crate) fn replay_commands(
    host: &TreeHostView,
    layer: &Retained<CALayer>,
    commands: &[RenderCommand],
    start: usize,
    origin: elwindui_core::base::Point,
    clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    live_native_controls: &mut HashSet<usize>,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<elwindui_core::graphics::VectorImageId, (u32, u32, CFRetained<CGImage>)>,
) -> usize {
    let mut idx = start;
    while idx < commands.len() {
        match &commands[idx] {
            RenderCommand::PopClip | RenderCommand::PopTransform | RenderCommand::PopOpacity => {
                return idx + 1;
            }
            RenderCommand::PushClip { clip: pushed } => {
                let pushed_rect = clip_bounds(pushed, origin);
                let new_clip = match (clip, pushed_rect) {
                    (Some(a), Some(b)) => a.intersect(b),
                    (Some(c), None) | (None, Some(c)) => Some(c),
                    (None, None) => None,
                };
                // Real per-pixel clipping (rounded corners, path shapes), not just `new_clip`'s
                // bounding-box culling test above: a masked container layer, sized to exactly
                // overlay `layer` (`frame = layer.bounds()`, so its local coordinate space stays
                // the same shared canvas-absolute space every other sublayer here already uses —
                // no re-anchoring needed, unlike `try_add_gradient_fill_layer`'s own mask). Nested
                // `PushClip`s recurse into their own container-of-a-container, so ancestor masks
                // compose via ordinary `CALayer.mask` nesting.
                let world = elwindui_core::base::AffineTransform::translation(origin.x, origin.y)
                    .concat(&transform);
                let container = CALayer::new();
                container.setName(Some(&NSString::from_str("elwindui-paint")));
                container.setFrame(layer.bounds());
                let mask_layer = clip_mask_layer(&world, pushed);
                unsafe { container.setMask(Some(&mask_layer)) };
                layer.addSublayer(&container);
                idx = replay_commands(
                    host,
                    &container,
                    commands,
                    idx + 1,
                    origin,
                    new_clip,
                    transform,
                    opacity,
                    live_native_controls,
                    image_cache,
                    vector_raster_cache,
                );
            }
            RenderCommand::PushTransform { transform: pushed } => {
                idx = replay_commands(
                    host,
                    layer,
                    commands,
                    idx + 1,
                    origin,
                    clip,
                    transform.concat(pushed),
                    opacity,
                    live_native_controls,
                    image_cache,
                    vector_raster_cache,
                );
            }
            RenderCommand::PushOpacity { opacity: pushed } => {
                idx = replay_commands(
                    host,
                    layer,
                    commands,
                    idx + 1,
                    origin,
                    clip,
                    transform,
                    opacity * *pushed,
                    live_native_controls,
                    image_cache,
                    vector_raster_cache,
                );
            }
            RenderCommand::NativeControl {
                owner_id,
                handle,
                rect,
            } => {
                let Some(mut view) = handle.downcast_ref::<AnyView>().cloned() else {
                    idx += 1;
                    continue;
                };
                let identity = view.identity();
                live_native_controls.insert(identity);
                let rect = elwindui_core::base::Rect {
                    x: origin.x + rect.x,
                    y: origin.y + rect.y,
                    width: rect.width,
                    height: rect.height,
                };
                let visible_rect = clip
                    .and_then(|clip| rect.intersect(clip))
                    .unwrap_or(rect);
                if visible_rect.width <= 0.0 || visible_rect.height <= 0.0 {
                    idx += 1;
                    continue;
                }
                // This is deliberately a native island only around an actual native command;
                // ordinary painted content continues to replay to `layer` above.
                let (container, is_new) = {
                    let mut containers = host.ivars().native_containers.borrow_mut();
                    if let Some(container) = containers.get(&identity) {
                        (container.clone(), false)
                    } else {
                        let container = NSView::new(mtm());
                        containers.insert(identity, container.clone());
                        host.ivars()
                            .native_owner_ids
                            .borrow_mut()
                            .insert(identity, *owner_id);
                        (container, true)
                    }
                };
                container.setFrame(NSRect::new(
                    objc2_foundation::NSPoint::new(visible_rect.x as f64, visible_rect.y as f64),
                    objc2_foundation::NSSize::new(
                        visible_rect.width as f64,
                        visible_rect.height as f64,
                    ),
                ));
                container.setClipsToBounds(true);
                let nsview = view.as_nsview();
                if is_new {
                    host.addSubview(&container);
                    container.addSubview(&nsview);
                }
                nsview.setTranslatesAutoresizingMaskIntoConstraints(true);
                view.arrange(elwindui_core::base::Rect {
                    x: rect.x - visible_rect.x,
                    y: rect.y - visible_rect.y,
                    width: rect.width,
                    height: rect.height,
                });
                idx += 1;
            }
            command => {
                if geometry_bounds(command, origin).is_none_or(|bounds| {
                    clip.is_none_or(|clip| bounds.intersect(clip).is_some())
                }) {
                    replay_paint_command(
                        host,
                        layer,
                        command,
                        origin,
                        transform,
                        opacity,
                        image_cache,
                        vector_raster_cache,
                    );
                }
                idx += 1;
            }
        }
    }
    idx
}

/// Builds and appends the one `CALayer` (`CAShapeLayer`/`CAGradientLayer`+mask/`CATextLayer`/
/// image-`CALayer`) a single ordinary paint `RenderCommand` needs, applying `transform` to its
/// geometry directly (each corner point individually — see `replay_group`'s own doc comment for
/// why this is simpler/more robust here than a nested `CALayer.affineTransform`) and `opacity` to
/// the resulting layer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn replay_paint_command(
    _host: &TreeHostView,
    layer: &Retained<CALayer>,
    command: &RenderCommand,
    origin: elwindui_core::base::Point,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<elwindui_core::graphics::VectorImageId, (u32, u32, CFRetained<CGImage>)>,
) {
    let world =
        elwindui_core::base::AffineTransform::translation(origin.x, origin.y).concat(&transform);
    let rounded_rect_path = |rect: &elwindui_core::base::Rect,
                             radii: elwindui_core::base::CornerRadius| {
        rounded_rect_cgpath(&world, *rect, radii)
    };
    match command {
        RenderCommand::FillRect { rect, brush } => {
            if !try_add_gradient_fill_layer(layer, brush, *rect, GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()), &world, opacity)
                && !try_add_image_fill_layer(layer, brush, *rect, GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()), &world, opacity, image_cache)
            {
                let path = rounded_rect_path(rect, elwindui_core::base::CornerRadius::default());
                add_shape_layer(layer, &path, Some(brush), None, opacity, *rect);
            }
        }
        RenderCommand::StrokeRect {
            rect,
            brush,
            stroke,
        } => {
            let path = rounded_rect_path(rect, elwindui_core::base::CornerRadius::default());
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, *rect);
        }
        RenderCommand::FillRoundedRect { rect, radii, brush } => {
            if !try_add_gradient_fill_layer(layer, brush, *rect, GradientMaskShape::RoundedRect(*radii), &world, opacity)
                && !try_add_image_fill_layer(layer, brush, *rect, GradientMaskShape::RoundedRect(*radii), &world, opacity, image_cache)
            {
                let path = rounded_rect_path(rect, *radii);
                add_shape_layer(layer, &path, Some(brush), None, opacity, *rect);
            }
        }
        RenderCommand::StrokeRoundedRect {
            rect,
            radii,
            brush,
            stroke,
        } => {
            let path = rounded_rect_path(rect, *radii);
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, *rect);
        }
        RenderCommand::FillEllipse { rect, brush } => {
            if !try_add_gradient_fill_layer(layer, brush, *rect, GradientMaskShape::Ellipse, &world, opacity)
                && !try_add_image_fill_layer(layer, brush, *rect, GradientMaskShape::Ellipse, &world, opacity, image_cache)
            {
                let path = ellipse_cgpath(&world, *rect);
                add_shape_layer(layer, &path, Some(brush), None, opacity, *rect);
            }
        }
        RenderCommand::StrokeEllipse {
            rect,
            brush,
            stroke,
        } => {
            let path = ellipse_cgpath(&world, *rect);
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, *rect);
        }
        RenderCommand::DrawLine {
            from,
            to,
            brush,
            stroke,
        } => {
            let path = CGMutablePath::new();
            unsafe {
                CGMutablePath::move_to_point(
                    Some(&path),
                    std::ptr::null(),
                    transform_point(&world, *from).x,
                    transform_point(&world, *from).y,
                );
            }
            unsafe {
                CGMutablePath::add_line_to_point(
                    Some(&path),
                    std::ptr::null(),
                    transform_point(&world, *to).x,
                    transform_point(&world, *to).y,
                );
            }
            let bounds = elwindui_core::base::Rect {
                x: from.x.min(to.x),
                y: from.y.min(to.y),
                width: (to.x - from.x).abs(),
                height: (to.y - from.y).abs(),
            };
            add_shape_layer(layer, &path, None, Some((brush, stroke)), opacity, bounds);
        }
        RenderCommand::FillPath { path, brush, rule } => {
            let cg_path = path_to_cgpath(&world, path);
            let shape_layer = CAShapeLayer::new();
            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            shape_layer.setPath(Some(&cg_path));
            shape_layer.setFillRule(match rule {
                elwindui_core::graphics::FillRule::NonZero => unsafe { kCAFillRuleNonZero },
                elwindui_core::graphics::FillRule::EvenOdd => unsafe { kCAFillRuleEvenOdd },
            });
            apply_fill(&shape_layer, Some(brush), path.bounds());
            shape_layer.setOpacity(opacity);
            let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
            layer.addSublayer(&shape_layer);
        }
        RenderCommand::StrokePath {
            path,
            brush,
            stroke,
        } => {
            let cg_path = path_to_cgpath(&world, path);
            let shape_layer = CAShapeLayer::new();
            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            shape_layer.setPath(Some(&cg_path));
            // `CAShapeLayer.fillColor` defaults to opaque black — must be explicitly nilled for a
            // stroke-only shape, same reasoning as `add_shape_layer`'s own doc comment.
            shape_layer.setFillColor(None);
            apply_stroke(&shape_layer, brush, stroke, path.bounds());
            shape_layer.setOpacity(opacity);
            let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
            layer.addSublayer(&shape_layer);
        }
        RenderCommand::DrawImage {
            image,
            dest,
            source,
            options,
        } => {
            // `options.repeat` (`TileMode::Tile`/`FlipX`/`FlipY`/`FlipXY`) has no direct
            // `CALayer.contents` equivalent — tiling would need multiple image sublayers stamped
            // across `dest` — and isn't attempted here; every `TileMode` draws as `None` (single
            // placement per `fitted_image_rect`) instead of silently ignoring the field outright.
            let Some(resolved) = resolve_cgimage(image, image_cache) else {
                return;
            };
            let Some(container) =
                build_image_container_layer(&resolved, *dest, *source, options, &world, opacity)
            else {
                return;
            };
            layer.addSublayer(&container);
        }
        RenderCommand::DrawVectorImage {
            image,
            dest,
            source,
            options,
        } => {
            crate::render::draw_vector_image(
                layer, image, *dest, *source, options, &world, opacity, image_cache,
                vector_raster_cache,
            );
        }
        RenderCommand::Text {
            content,
            rect,
            style,
            foreground,
            alignment,
        } => {
            let text_layer = CATextLayer::new();
            text_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            text_layer.setFrame(NSRect::new(
                transform_point(
                    &world,
                    elwindui_core::base::Point {
                        x: rect.x,
                        y: rect.y,
                    },
                ),
                objc2_foundation::NSSize::new(rect.width as f64, rect.height as f64),
            ));
            // Once an `NSAttributedString` is set, `CATextLayer` ignores its own `font`/
            // `fontSize`/`foregroundColor`/`alignmentMode` entirely — those setters are
            // deliberately *not* called here (they'd be a silently-dead second source of truth).
            // Font/foreground/kerning/alignment all come from the same `text_attributes` a
            // `TextBlock`'s own measurement (`AppKitTextBackend::measure_text`) used, so the
            // painted glyphs always match what was measured.
            unsafe {
                text_layer.setString(Some(&crate::render::attributed_string(
                    content,
                    style,
                    foreground.as_ref(),
                    *alignment,
                )));
            }
            // Matches `render::vector`'s own `contentsScale` inheritance (see that module's doc
            // comment) — this sublayer would otherwise default to `1.0` and render blurry on a
            // Retina display regardless of the group layer's own scale.
            text_layer.setContentsScale(layer.contentsScale());
            text_layer.setOpacity(opacity);
            let text_layer: Retained<CALayer> = Retained::into_super(text_layer);
            layer.addSublayer(&text_layer);
        }
        RenderCommand::NativeControl { .. }
        | RenderCommand::PushClip { .. }
        | RenderCommand::PopClip
        | RenderCommand::PushTransform { .. }
        | RenderCommand::PopTransform
        | RenderCommand::PushOpacity { .. }
        | RenderCommand::PopOpacity => {}
    }
}
