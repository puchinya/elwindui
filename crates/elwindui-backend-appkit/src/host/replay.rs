//! The `RenderGroup`/`RenderCommand` -> `CALayer` replay pass, plus its per-group cache key.
//!
//! Lives under `host` rather than `render` because it is `TreeHostView`'s own rendering pass —
//! but note that none of the functions below actually take a `&TreeHostView` any more. Everything
//! they read or write across passes lives in [`ReplayState`], a plain, ObjC-free struct that
//! `cargo test` can construct directly; the one thing a pass genuinely needs a live view for
//! (creating/attaching a `RenderCommand::NativeControl`'s `NSView` island) is factored out behind
//! the small [`NativeIslandHost`] trait instead. This is what lets a unit test drive `replay_group`
//! against a bare `CALayer` — see `TreeHostView`'s own `NativeIslandHost` impl in `host::mod` for
//! the real one used in production, and this crate's own `testsupport::golden` for why a real
//! `TreeHostView` can't be constructed from a `cargo test` worker thread at all
//! (`MainThreadMarker::new()` returns `None` there).

use crate::ffi::AnyView;
use crate::render::{
    GradientMaskShape, add_shape_layer, add_sublayer_scaled, apply_fill, apply_stroke,
    build_image_container_layer, clip_bounds, clip_mask_layer, ellipse_cgpath, geometry_bounds,
    path_to_cgpath, resolve_cgimage, rounded_rect_cgpath, set_mask_scaled, transform_point,
    try_add_gradient_fill_layer, try_add_image_fill_layer,
};
use elwindui_core::graphics::{Brush, ImageId, RenderCommand, RenderGroup, VectorImageId};
use elwindui_core::ui::TextAlignment;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_core_foundation::{CFRetained, CGFloat};
use objc2_core_graphics::{CGImage, CGMutablePath};
use objc2_foundation::{NSRect, NSString};
use objc2_quartz_core::{
    CALayer, CAShapeLayer, CATextLayer, kCAAlignmentCenter, kCAAlignmentLeft, kCAAlignmentRight,
    kCAFillRuleEvenOdd, kCAFillRuleNonZero,
};
use std::collections::{HashMap, HashSet};

/// What `ReplayState::group_layers[id]`'s sublayers were last rebuilt from — see
/// `GroupCacheEntry`'s own doc comment for why `RenderGroup::generation` alone isn't a sufficient
/// cache key.
///
/// `scale` (the host's `backing_scale_factor()` at the time of the last rebuild) is part of this
/// key so a group whose geometry is byte-for-byte unchanged still rebuilds when the window moves
/// to a display with a different backing scale — see `TreeHostView::backing_scale_factor` and
/// `render::add_sublayer_scaled`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct GroupCacheKey {
    origin: elwindui_core::base::Point,
    clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    generation: u64,
    scale: CGFloat,
}

/// Everything a `GroupCacheKey` hit/miss needs to restore without replaying `group.commands` —
/// what native controls, raster images, and vector images that group's cached sublayers reference.
/// Bundling these three liveness lists with the key itself (rather than four parallel
/// `HashMap<u64, _>`s, as this used to be) removes a latent bug: a cache hit used to restore
/// image/vector liveness only when a `group_native_controls` entry happened to exist for that
/// group id, relying on the rebuild path always inserting one (even an empty `Vec`) to keep the
/// two in sync. One struct, one lookup, no coupling to maintain.
pub(crate) struct GroupCacheEntry {
    key: GroupCacheKey,
    native_controls: Vec<usize>,
    image_ids: Vec<ImageId>,
    vector_image_ids: Vec<VectorImageId>,
}

/// Everything a replay pass reads or writes that is not the live `CALayer`/`NSView` tree itself —
/// held by `TreeHostIvars` as a single `RefCell<ReplayState>` so a pass takes exactly one borrow
/// across its own recursion instead of the many small ones this used to take. Fully ObjC-free
/// apart from the `Retained<CALayer>`/`CFRetained<CGImage>` values themselves, so it can be
/// constructed directly in a `cargo test` — see this module's own doc comment.
#[derive(Default)]
pub(crate) struct ReplayState {
    /// Per-`RenderGroup` id, the persistent container `CALayer` holding that group's own painted
    /// sublayers — a flat sibling of the root paint layer (`frame` always exactly matches the
    /// root's own `bounds()`, a zero-offset "namespace" rather than a real nested coordinate
    /// space) so every existing absolute-canvas-coordinate drawing helper keeps working completely
    /// unchanged. Reused across passes — see `group_cache`'s own doc comment for when its
    /// contents get rebuilt vs. left alone.
    pub(crate) group_layers: HashMap<u64, Retained<CALayer>>,
    /// What `group_layers[id]`'s sublayers were last rebuilt from, plus the resource liveness
    /// that rebuild discovered — see `GroupCacheEntry`'s own doc comment. A `RenderGroup`'s own
    /// `generation` alone can't tell a pass whether a rebuild is needed: this backend bakes the
    /// *full accumulated* origin/clip/transform/opacity directly into each leaf's `CGPath`/frame
    /// (not a live nested `CALayer` transform, by deliberate design — see `replay_group`'s own doc
    /// comment), so a group whose own `commands` are byte-for-byte unchanged still needs
    /// rebuilding if an ancestor's offset moved (the group's own relative `offset` stays the same,
    /// so its `generation` never bumps, even though the *absolute* geometry baked into its cached
    /// sublayers is now stale). Comparing the full `GroupCacheKey` tuple each pass catches both
    /// that and a window moved to a display with a different `backing_scale_factor`.
    pub(crate) group_cache: HashMap<u64, GroupCacheEntry>,
    /// Decoded-image cache (`RenderCommand::DrawImage`'s `elwindui_core::graphics::Image` -> real
    /// `CGImage`), keyed by the image's stable `ImageId`. Pruned after each pass to the resources
    /// referenced by the currently retained render tree.
    pub(crate) image_cache: HashMap<ImageId, CFRetained<CGImage>>,
    /// `RenderCommand::DrawVectorImage`'s `VectorRasterizeMode::Auto`/`Fixed` cache — the
    /// rasterized-bitmap counterpart to `image_cache` above, keyed by `VectorImageId` rather than
    /// pointer identity since the *same* `VectorImage` may legitimately need re-rasterizing at a
    /// different pixel size. At most one entry per id.
    pub(crate) vector_raster_cache: HashMap<VectorImageId, (u32, u32, u8, CFRetained<CGImage>)>,
}

/// The one thing a replay pass genuinely needs a live `NSView`-backed host for: the container
/// island a `RenderCommand::NativeControl` renders its native leaf into. Everything else a pass
/// touches lives in [`ReplayState`] and needs no live view at all. `TreeHostView`'s own impl
/// (`host::mod`) is the one used in production; a test double whose methods `unreachable!()` is
/// enough for any tree that contains no `NativeControl` commands.
pub(crate) trait NativeIslandHost {
    /// Returns the persistent per-identity container view for `identity`, creating (and
    /// registering `owner_id` for) a new one if this is the first time `identity` has been seen
    /// by this host. The `bool` is whether the container was freshly created by this call.
    fn island(&self, identity: usize, owner_id: u64) -> (Retained<NSView>, bool);
    /// Attaches a freshly created `container` (and its own inner `nsview`) into the real view
    /// hierarchy. Called exactly once per identity, right after `island` returns `is_new == true`.
    fn attach_island(&self, container: &NSView, nsview: &NSView);
}

/// Returns the raster and vector resources a group's command list can resolve. The result is
/// stored alongside the cached layer so a cache hit can restore liveness without replaying the
/// command list.
fn resource_ids(commands: &[RenderCommand]) -> (Vec<ImageId>, Vec<VectorImageId>) {
    fn add_image(images: &mut Vec<ImageId>, id: ImageId) {
        if !images.contains(&id) {
            images.push(id);
        }
    }

    let mut images = Vec::new();
    let mut vectors = Vec::new();
    let add_brush = |brush: &Brush, images: &mut Vec<ImageId>| {
        if let Brush::Image(image_brush) = brush {
            add_image(images, image_brush.image.id());
        }
    };
    for command in commands {
        match command {
            RenderCommand::FillRect { brush, .. }
            | RenderCommand::StrokeRect { brush, .. }
            | RenderCommand::FillRoundedRect { brush, .. }
            | RenderCommand::StrokeRoundedRect { brush, .. }
            | RenderCommand::FillEllipse { brush, .. }
            | RenderCommand::StrokeEllipse { brush, .. }
            | RenderCommand::DrawLine { brush, .. }
            | RenderCommand::FillPath { brush, .. }
            | RenderCommand::StrokePath { brush, .. } => add_brush(brush, &mut images),
            RenderCommand::DrawImage { image, .. } => {
                add_image(&mut images, image.id());
            }
            RenderCommand::DrawVectorImage { image, .. } => {
                let id = image.id();
                if !vectors.contains(&id) {
                    vectors.push(id);
                }
            }
            RenderCommand::Text { foreground, .. } => {
                if let Some(brush) = foreground {
                    add_brush(brush, &mut images);
                }
            }
            RenderCommand::PushClip { .. }
            | RenderCommand::PopClip
            | RenderCommand::PushTransform { .. }
            | RenderCommand::PopTransform
            | RenderCommand::PushOpacity { .. }
            | RenderCommand::PopOpacity
            | RenderCommand::NativeControl { .. } => {}
        }
    }
    (images, vectors)
}

/// One retained-render replay pass over a `RenderGroup` tree, appending real `CALayer`s to
/// `root_layer` (ordinary painted content) and real `NSView` islands via `native` (native
/// controls), in traversal order so both interleave in the correct Z order (painter design doc
/// §14.2's "single custom drawing surface" intent, adapted to AppKit's native layer-composition
/// model rather than a `NSView.draw(_:)`/`CGContext` replay — `CAShapeLayer`/`CAGradientLayer`
/// already cover fill/stroke/dash/cap/join/miter/gradient natively, so a full `CGContext`-based
/// rewrite would only add complexity without adding capability here). `transform`/`opacity` are
/// plain accumulators (composed/multiplied down the recursion, applied when building each leaf's
/// own geometry/`opacity` — not modeled as extra nested `CALayer`s, which would need fighting
/// `CALayer`'s anchor-point-relative transform semantics for no benefit) — `clip` is the one
/// state that genuinely needs geometry-level handling, done here as a simple bounding-box
/// intersection test (skip a leaf whose rect doesn't overlap `clip` at all) rather than true
/// per-pixel masking, mirroring `Shape::hit_test_content`'s own "whole bounding rect, not
/// per-pixel" simplification elsewhere in this codebase.
///
/// Each `RenderGroup` gets one persistent, cached container `CALayer` (`ReplayState::
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
    native: &dyn NativeIslandHost,
    root_layer: &Retained<CALayer>,
    group: &RenderGroup,
    origin: elwindui_core::base::Point,
    inherited_clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    scale: CGFloat,
    live_native_controls: &mut HashSet<usize>,
    live_group_ids: &mut HashSet<u64>,
    live_image_ids: &mut HashSet<ImageId>,
    live_vector_image_ids: &mut HashSet<VectorImageId>,
    state: &mut ReplayState,
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
    crate::render::stats::bump(|s| s.groups_visited += 1);

    let is_new = !state.group_layers.contains_key(&group.id);
    let container = state
        .group_layers
        .entry(group.id)
        .or_insert_with(|| {
            crate::render::stats::bump(|s| s.layers_created += 1);
            let c = CALayer::new();
            c.setName(Some(&NSString::from_str("elwindui-paint")));
            c
        })
        .clone();
    container.setFrame(root_layer.bounds());
    // Set directly from `scale` (not via `add_sublayer_scaled`, which would also recursively
    // re-stamp every one of this container's *existing* sublayers on every single pass, including
    // cache hits) — `GroupCacheKey::scale` below already forces a full rebuild, which re-attaches
    // every descendant through `add_sublayer_scaled` and picks up this value, whenever the scale
    // genuinely changes. This keeps a cache-hit pass exactly as cheap as it was before this fix.
    container.setContentsScale(scale);
    crate::render::stats::bump(|s| s.add_sublayer_calls += 1);
    root_layer.addSublayer(&container);

    let key = GroupCacheKey {
        origin,
        clip: effective_clip,
        transform,
        opacity,
        generation: group.generation,
        scale,
    };
    let stale = is_new || state.group_cache.get(&group.id).map(|entry| entry.key) != Some(key);
    if stale {
        crate::render::stats::bump(|s| s.groups_rebuilt += 1);
        if let Some(existing) = unsafe { container.sublayers() } {
            // `removeFromSuperlayer` while iterating `existing` (a live view onto `container`'s
            // own sublayer array, not a snapshot) trips Foundation's mutation-during-enumeration
            // guard — collect into a plain `Vec` first, then iterate that instead.
            let old: Vec<_> = existing.iter().collect();
            crate::render::stats::bump(|s| s.layers_removed += old.len() as u32);
            for sub in old {
                sub.removeFromSuperlayer();
            }
        }
        let native_controls_before: HashSet<usize> = live_native_controls.clone();
        replay_commands(
            native,
            &container,
            &group.commands,
            0,
            origin,
            effective_clip,
            transform,
            opacity,
            live_native_controls,
            &mut state.image_cache,
            &mut state.vector_raster_cache,
        );
        let discovered_native_controls: Vec<usize> = live_native_controls
            .difference(&native_controls_before)
            .copied()
            .collect();
        let (image_ids, vector_image_ids) = resource_ids(&group.commands);
        live_image_ids.extend(image_ids.iter().copied());
        live_vector_image_ids.extend(vector_image_ids.iter().copied());
        state.group_cache.insert(
            group.id,
            GroupCacheEntry {
                key,
                native_controls: discovered_native_controls,
                image_ids,
                vector_image_ids,
            },
        );
    } else {
        crate::render::stats::bump(|s| s.groups_cache_hit += 1);
        if let Some(entry) = state.group_cache.get(&group.id) {
            live_native_controls.extend(&entry.native_controls);
            live_image_ids.extend(&entry.image_ids);
            live_vector_image_ids.extend(&entry.vector_image_ids);
        }
    }

    for child in &group.children {
        replay_group(
            native,
            root_layer,
            child,
            origin,
            effective_clip,
            transform,
            opacity,
            scale,
            live_native_controls,
            live_group_ids,
            live_image_ids,
            live_vector_image_ids,
            state,
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
    native: &dyn NativeIslandHost,
    layer: &Retained<CALayer>,
    commands: &[RenderCommand],
    start: usize,
    origin: elwindui_core::base::Point,
    clip: Option<elwindui_core::base::Rect>,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    live_native_controls: &mut HashSet<usize>,
    image_cache: &mut HashMap<ImageId, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<VectorImageId, (u32, u32, u8, CFRetained<CGImage>)>,
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
                crate::render::stats::bump(|s| s.layers_created += 1);
                let container = CALayer::new();
                container.setName(Some(&NSString::from_str("elwindui-paint")));
                container.setFrame(layer.bounds());
                // Attach before masking, not after: `add_sublayer_scaled` stamps `container`'s
                // scale from `layer` at attach time, and `set_mask_scaled` needs that already-set
                // scale on `container` to propagate correctly onto `mask_layer`.
                add_sublayer_scaled(layer, &container);
                let mask_layer = clip_mask_layer(&world, pushed);
                set_mask_scaled(&container, &mask_layer);
                idx = replay_commands(
                    native,
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
                    native,
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
                    native,
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
                let visible_rect = clip.and_then(|clip| rect.intersect(clip)).unwrap_or(rect);
                if visible_rect.width <= 0.0 || visible_rect.height <= 0.0 {
                    idx += 1;
                    continue;
                }
                // This is deliberately a native island only around an actual native command;
                // ordinary painted content continues to replay to `layer` above.
                let (container, is_new) = native.island(identity, *owner_id);
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
                    crate::render::stats::bump(|s| s.subview_added += 1);
                    native.attach_island(&container, &nsview);
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
                let world = elwindui_core::base::AffineTransform::translation(origin.x, origin.y)
                    .concat(&transform);
                if geometry_bounds(command, &world)
                    .is_none_or(|bounds| clip.is_none_or(|clip| bounds.intersect(clip).is_some()))
                {
                    replay_paint_command(
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
    layer: &Retained<CALayer>,
    command: &RenderCommand,
    origin: elwindui_core::base::Point,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<ImageId, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<VectorImageId, (u32, u32, u8, CFRetained<CGImage>)>,
) {
    let world =
        elwindui_core::base::AffineTransform::translation(origin.x, origin.y).concat(&transform);
    let rounded_rect_path = |rect: &elwindui_core::base::Rect,
                             radii: elwindui_core::base::CornerRadius| {
        rounded_rect_cgpath(&world, *rect, radii)
    };
    match command {
        RenderCommand::FillRect { rect, brush } => {
            if !try_add_gradient_fill_layer(
                layer,
                brush,
                *rect,
                GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()),
                &world,
                opacity,
            ) && !try_add_image_fill_layer(
                layer,
                brush,
                *rect,
                GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()),
                &world,
                opacity,
                image_cache,
            ) {
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
            if !try_add_gradient_fill_layer(
                layer,
                brush,
                *rect,
                GradientMaskShape::RoundedRect(*radii),
                &world,
                opacity,
            ) && !try_add_image_fill_layer(
                layer,
                brush,
                *rect,
                GradientMaskShape::RoundedRect(*radii),
                &world,
                opacity,
                image_cache,
            ) {
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
            if !try_add_gradient_fill_layer(
                layer,
                brush,
                *rect,
                GradientMaskShape::Ellipse,
                &world,
                opacity,
            ) && !try_add_image_fill_layer(
                layer,
                brush,
                *rect,
                GradientMaskShape::Ellipse,
                &world,
                opacity,
                image_cache,
            ) {
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
            crate::render::stats::bump(|s| s.cgpaths_created += 1);
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
            crate::render::stats::bump(|s| s.layers_created += 1);
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
            add_sublayer_scaled(layer, &shape_layer);
        }
        RenderCommand::StrokePath {
            path,
            brush,
            stroke,
        } => {
            let cg_path = path_to_cgpath(&world, path);
            crate::render::stats::bump(|s| s.layers_created += 1);
            let shape_layer = CAShapeLayer::new();
            shape_layer.setName(Some(&NSString::from_str("elwindui-paint")));
            shape_layer.setPath(Some(&cg_path));
            // `CAShapeLayer.fillColor` defaults to opaque black — must be explicitly nilled for a
            // stroke-only shape, same reasoning as `add_shape_layer`'s own doc comment.
            shape_layer.setFillColor(None);
            apply_stroke(&shape_layer, brush, stroke, path.bounds());
            shape_layer.setOpacity(opacity);
            let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
            add_sublayer_scaled(layer, &shape_layer);
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
            add_sublayer_scaled(layer, &container);
        }
        RenderCommand::DrawVectorImage {
            image,
            dest,
            source,
            options,
        } => {
            crate::render::draw_vector_image(
                layer,
                image,
                *dest,
                *source,
                options,
                &world,
                opacity,
                image_cache,
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
            crate::render::stats::bump(|s| {
                s.layers_created += 1;
                s.text_layers_created += 1;
            });
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
            // The attributed string remains the source of font, foreground, and kerning. Its
            // paragraph alignment is required for AppKit measurement, but CATextLayer's actual
            // paint layout follows `alignmentMode`; set the equivalent native value explicitly
            // so a `RenderCommand::Text` with `Center` or `Right` is not painted as left-aligned.
            text_layer.setAlignmentMode(match alignment {
                TextAlignment::Left => unsafe { kCAAlignmentLeft },
                TextAlignment::Center => unsafe { kCAAlignmentCenter },
                TextAlignment::Right => unsafe { kCAAlignmentRight },
            });
            unsafe {
                text_layer.setString(Some(&crate::render::attributed_string(
                    content,
                    style,
                    foreground.as_ref(),
                    *alignment,
                )));
            }
            text_layer.setOpacity(opacity);
            let text_layer: Retained<CALayer> = Retained::into_super(text_layer);
            // `add_sublayer_scaled` stamps this layer's `contentsScale` from `layer`'s — this
            // sublayer would otherwise default to `1.0` and render blurry on a Retina display
            // regardless of the group layer's own scale (see `render::layer`'s doc comment).
            add_sublayer_scaled(layer, &text_layer);
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

#[cfg(test)]
mod tests {
    use super::*;
    use elwindui_core::base::{AffineTransform, CornerRadius, Point, Rect};
    use elwindui_core::graphics::{Brush, Color};

    /// A `NativeIslandHost` for trees that contain no `RenderCommand::NativeControl` — every real
    /// method call is a test bug (a tree that needs a real native island), not a code path this
    /// double is meant to support. Proves `replay_group`/`replay_commands` need no live `NSView`
    /// host at all for ordinary painted content — see this module's own doc comment on why that
    /// matters (`TreeHostView` itself can't be constructed off the real main thread that `cargo
    /// test`'s worker threads are not).
    struct NoNativeIslands;

    impl NativeIslandHost for NoNativeIslands {
        fn island(&self, _identity: usize, _owner_id: u64) -> (Retained<NSView>, bool) {
            unreachable!("test tree must not contain a RenderCommand::NativeControl")
        }
        fn attach_island(&self, _container: &NSView, _nsview: &NSView) {
            unreachable!("test tree must not contain a RenderCommand::NativeControl")
        }
    }

    fn solid_fill_rect_group(id: u64) -> RenderGroup {
        let mut group = RenderGroup::new(id, Point { x: 0.0, y: 0.0 }, None);
        group.commands = vec![RenderCommand::FillRect {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            brush: Brush::Solid(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        }];
        group
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_once(
        root_layer: &Retained<CALayer>,
        group: &RenderGroup,
        state: &mut ReplayState,
    ) {
        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut live_image_ids = HashSet::new();
        let mut live_vector_image_ids = HashSet::new();
        replay_group(
            &NoNativeIslands,
            root_layer,
            group,
            Point { x: 0.0, y: 0.0 },
            None,
            AffineTransform::identity(),
            1.0,
            1.0,
            &mut live_native_controls,
            &mut live_group_ids,
            &mut live_image_ids,
            &mut live_vector_image_ids,
            state,
        );
    }

    #[test]
    fn second_replay_of_an_unchanged_group_hits_the_cache_and_creates_nothing_new() {
        let root_layer = CALayer::new();
        let group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();

        replay_once(&root_layer, &group, &mut state);
        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        // `add_sublayer_calls` is deliberately not asserted here — `replay_group` still
        // unconditionally re-`addSublayer`s the group container every pass (that's Step 2's own
        // fix, tracked by the full §22 no-op assertion added once it lands). What Step 0 already
        // guarantees is that a cache hit builds nothing new.
        assert_eq!(stats.groups_rebuilt, 0, "unchanged group must not rebuild");
        assert_eq!(stats.groups_cache_hit, 1);
        assert_eq!(stats.layers_created, 0, "cache hit must create no new CALayer");
        assert_eq!(stats.layers_removed, 0);
        assert_eq!(stats.cgpaths_created, 0, "cache hit must not rebuild any CGPath");
    }

    #[test]
    fn replaying_at_a_different_origin_forces_a_rebuild() {
        // `GroupCacheKey::origin` is what makes a scrolled ancestor's offset change invalidate an
        // unchanged descendant group — see `ReplayState::group_cache`'s own doc comment. This
        // guards that the `ReplayState` refactor preserved that behavior byte-for-byte.
        let root_layer = CALayer::new();
        let group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();
        replay_once(&root_layer, &group, &mut state);

        crate::render::stats::reset();
        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut live_image_ids = HashSet::new();
        let mut live_vector_image_ids = HashSet::new();
        replay_group(
            &NoNativeIslands,
            &root_layer,
            &group,
            Point { x: 5.0, y: 0.0 },
            None,
            AffineTransform::identity(),
            1.0,
            1.0,
            &mut live_native_controls,
            &mut live_group_ids,
            &mut live_image_ids,
            &mut live_vector_image_ids,
            &mut state,
        );

        let stats = crate::render::stats::snapshot();
        assert_eq!(stats.groups_rebuilt, 1, "a moved ancestor must force a rebuild");
        assert!(stats.cgpaths_created > 0);
    }

    #[test]
    fn a_changed_command_forces_a_rebuild_via_generation() {
        let root_layer = CALayer::new();
        let mut group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();
        replay_once(&root_layer, &group, &mut state);

        // Simulate core's own `reconcile_render_group` recording new commands: bump `generation`
        // alongside the content change, exactly as `engine.rs`'s `record_group_commands` does.
        group.commands = vec![RenderCommand::FillRoundedRect {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            radii: CornerRadius::uniform(4.0),
            brush: Brush::Solid(Color {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            }),
        }];
        group.generation += 1;

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        assert_eq!(stats.groups_rebuilt, 1);
        assert_eq!(stats.groups_cache_hit, 0);
    }
}
