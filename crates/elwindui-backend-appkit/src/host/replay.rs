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
use elwindui_core::graphics::{
    Brush, CommandKind, ImageId, RenderCommand, RenderGroup, VectorImageId,
};
use elwindui_core::ui::TextAlignment;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_core_foundation::{CFRetained, CGFloat};
use objc2_core_graphics::{CGImage, CGMutablePath};
use objc2_foundation::NSRect;
use objc2_quartz_core::{
    CALayer, CAShapeLayer, CATextLayer, kCAAlignmentCenter, kCAAlignmentLeft, kCAAlignmentRight,
    kCAFillRuleEvenOdd, kCAFillRuleNonZero,
};
use std::collections::{HashMap, HashSet};

/// A group's own on-screen extent (`(0, 0, group.size.width, group.size.height)`, in this
/// group's *local* space) classified against the clip it inherited from its ancestors — the
/// piece that lets [`GroupCacheKey`] drop absolute `origin`/absolute-`clip` entirely (see that
/// struct's own doc comment) without losing correctness or, just as importantly, without losing
/// the whole point of dropping `origin`: a scrolled `ScrollView`'s inherited clip is *fixed* in
/// absolute space, so expressed as a plain local rect it would shift by the scroll delta on
/// every single tick, changing this key just as often as raw `origin` used to. `Inside`/`Outside`
/// are what stay stable across a scroll — only a group whose own extent straddles the viewport
/// edge is ever `Partial`, and only that group's `GroupCacheKey` actually changes tick to tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ClipRelation {
    /// No clip inherited from any ancestor.
    Unclipped,
    /// This group's own extent is fully contained in the inherited clip — safe to replay with no
    /// per-leaf bounding-box culling at all (equivalent to `Unclipped` for that purpose).
    Inside,
    /// This group's own extent does not intersect the inherited clip at all. `replay_group`
    /// hides the container and skips replaying `commands` entirely — the free subtree culling
    /// this classification exists to provide — while still restoring cached resource liveness
    /// exactly as an ordinary cache hit would, so a native control or decoded image scrolled
    /// offscreen isn't torn down just because its owning group went unrendered this pass.
    Outside,
    /// Straddles the inherited clip's own edge — `Rect` is the intersection, already expressed
    /// in this group's local space, and is what `replay_commands` bbox-culls individual leaves
    /// against.
    Partial(elwindui_core::base::Rect),
}

impl ClipRelation {
    /// `local_clip` is the inherited clip already converted to this group's own local space
    /// (`inherited_clip - origin`); `None` means no ancestor clip at all.
    fn classify(
        local_clip: Option<elwindui_core::base::Rect>,
        size: elwindui_core::base::Size,
    ) -> Self {
        let Some(local_clip) = local_clip else {
            return ClipRelation::Unclipped;
        };
        let extent = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: size.width,
            height: size.height,
        };
        match extent.intersect(local_clip) {
            None => ClipRelation::Outside,
            Some(intersection) if intersection == extent => ClipRelation::Inside,
            Some(intersection) => ClipRelation::Partial(intersection),
        }
    }
}

/// What `ReplayState::group_layers[id]`'s sublayers were last rebuilt from — see
/// `GroupCacheEntry`'s own doc comment for why `RenderGroup::generation` alone isn't a sufficient
/// cache key.
///
/// Deliberately holds no absolute `origin` — see `ClipRelation`'s own doc comment for why `clip`
/// is expressed that way instead of as a raw absolute rect, which is the other half of the same
/// problem. `transform`/`opacity` stay as accumulators exactly as before (in practice always
/// identity/`1.0` at this level today — nothing currently makes a `RenderGroup` boundary inherit
/// a non-identity `transform` from an ancestor's own commands — but kept for correctness should
/// that change).
///
/// `scale` (the host's `backing_scale_factor()` at the time of the last rebuild) is part of this
/// key so a group whose geometry is byte-for-byte unchanged still rebuilds when the window moves
/// to a display with a different backing scale — see `TreeHostView::backing_scale_factor` and
/// `render::add_sublayer_scaled`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct GroupCacheKey {
    clip: ClipRelation,
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
    /// `(identity, owner_id, local_rect)` for every native control this group's commands placed —
    /// `local_rect` is that control's `visible_local` rect (relative to this group's own absolute
    /// origin, already clip-intersected) as of the pass that last rebuilt this entry. Kept (rather
    /// than just `identity`) so a cache-hit pass can still re-derive each control's *current*
    /// absolute frame from this group's freshly recomputed `origin` — see `replay_group`'s
    /// unconditional native-control resync step, right after the `stale`/cache-hit branch.
    native_controls: Vec<(usize, u64, elwindui_core::base::Rect)>,
    image_ids: Vec<ImageId>,
    vector_image_ids: Vec<VectorImageId>,
    /// Per-command `CommandKind`, in the same order as `RenderGroup::commands`, captured only
    /// when this group's rebuild took the flat-leaf fast path (`replay_flat_commands` — see
    /// `is_flat_leaf_only`) — left empty otherwise. An empty vec here (or one whose length doesn't
    /// match a later `RenderGroup::commands`) is exactly what makes `try_update_group_in_place`
    /// decline: there is nothing to compare kinds against.
    command_kinds: Vec<CommandKind>,
    /// Parallel to `command_kinds` — each position's own fast-path `CALayer`, or `None` for a
    /// position the general paint path had to handle (not fast-path-eligible) or that was
    /// consumed as the second half of a fill+stroke fusion (see `try_fast_path`'s own doc
    /// comment). Any `None` here also makes `try_update_group_in_place` decline for this group:
    /// there is no single `CALayer` at that position to update in place.
    fast_path_layers: Vec<Option<Retained<CALayer>>>,
}

fn append_cached_native_order(
    entry: &GroupCacheEntry,
    visible: bool,
    new_native_order: &mut Vec<usize>,
) {
    if visible {
        new_native_order.extend(entry.native_controls.iter().map(|(identity, ..)| *identity));
    }
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
    /// The `RenderGroup` traversal order of the previous pass, in final Z order (every group
    /// container is a flat sibling of `root_layer`, so this one flat list covers the whole tree,
    /// not just root-level groups) — compared against the current pass's own traversal order so
    /// `relayout_inner` can tell whether `root_layer`'s sublayer array needs any reordering *at
    /// all* before touching it. See `relayout_inner`'s own doc comment on why re-`addSublayer`ing
    /// in this order, when it *is* needed, is enough to fix the order (no `insertSublayer:atIndex:`
    /// bookkeeping) without disturbing native-control layers interleaved in the same array.
    pub(crate) group_order: Vec<u64>,
    /// The `RenderCommand::NativeControl` traversal order of the previous pass — the `NSView`
    /// counterpart of `group_order`, compared the same way to decide whether the native z-order
    /// restore loop (`relayout_inner`) needs to run at all.
    pub(crate) native_order: Vec<usize>,
    /// Decoded-image cache (`RenderCommand::DrawImage`'s `elwindui_core::graphics::BitmapImage` -> real
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

/// Whether every command in `commands` is an ordinary paint leaf — no `Push*`/`Pop*` (a nested
/// scope) and no `NativeControl` (a real `NSView` island, not a `CALayer`). Gates both
/// `replay_flat_commands` (able to skip `replay_commands`'s general recursion entirely, since
/// there is no scope to recurse into) and, transitively through `GroupCacheEntry::command_kinds`/
/// `fast_path_layers` only ever being populated from that flat path, the in-place update
/// `try_update_group_in_place` performs later.
fn is_flat_leaf_only(commands: &[RenderCommand]) -> bool {
    commands.iter().all(|c| {
        !matches!(
            c.kind(),
            CommandKind::PushClip
                | CommandKind::PopClip
                | CommandKind::PushTransform
                | CommandKind::PopTransform
                | CommandKind::PushOpacity
                | CommandKind::PopOpacity
                | CommandKind::NativeControl
        )
    })
}

/// Replays a flat, non-nested command list (`is_flat_leaf_only(commands)` already known true —
/// no `Push*`/`Pop*`/`NativeControl`, so `origin` is always zero and `transform` is the group's
/// own `world`) directly, without going through `replay_commands`'s general recursion, so each
/// position's own fast-path `CALayer` (if any) can be captured for a future in-place update — see
/// `try_update_fast_path`. Only called when this group's inherited clip classified as
/// `Unclipped`/`Inside` (see `replay_group`'s own call site), so there is no per-leaf
/// bounding-box culling to do here — every command in `commands` is drawn.
fn replay_flat_commands(
    layer: &Retained<CALayer>,
    commands: &[RenderCommand],
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    image_cache: &mut HashMap<ImageId, CFRetained<CGImage>>,
    vector_raster_cache: &mut HashMap<VectorImageId, (u32, u32, u8, CFRetained<CGImage>)>,
) -> Vec<Option<Retained<CALayer>>> {
    let origin = elwindui_core::base::Point { x: 0.0, y: 0.0 };
    let mut fast_path_layers = vec![None; commands.len()];
    let mut idx = 0;
    while idx < commands.len() {
        let command = &commands[idx];
        // Batching (Step 8) is tried first, same as in `replay_commands`'s own general loop —
        // this is the *common* case a flat, `Unclipped`/`Inside` group actually hits, so batching
        // must be reachable here too, not just from the general path (see `try_batch_fills`'s own
        // doc comment). A batched run's positions get no per-position `CALayer` here (`None`
        // stays in `fast_path_layers` for each of them, its already-initialized default), so a
        // group containing one simply never becomes eligible for Step 7b's in-place update later
        // — the same fate as any other group whose leaf list isn't uniformly single-command
        // fast-path-eligible.
        let batched =
            crate::render::try_batch_fills(layer, commands, idx, &transform, None, opacity);
        if batched > 0 {
            idx += batched;
            continue;
        }
        let next = commands.get(idx + 1);
        let (consumed, created) =
            crate::render::try_fast_path(layer, command, next, &transform, opacity);
        if consumed == 0 {
            replay_paint_command(
                layer,
                command,
                origin,
                transform,
                opacity,
                image_cache,
                vector_raster_cache,
            );
            idx += 1;
        } else {
            fast_path_layers[idx] = created;
            idx += consumed;
        }
    }
    fast_path_layers
}

/// Attempts the in-place fast-path update described in `replay_group`'s own doc comment on its
/// `in_place` local. Returns `true` only when `entry` came from a *previous* rebuild of this exact
/// group id that took the flat-leaf fast path (`entry.command_kinds`/`fast_path_layers` both
/// non-empty and every layer `Some`), the command *kinds* are unchanged in both length and order,
/// and every single position's `try_update_fast_path` call succeeds. On a `false` return, some
/// positions may have already been mutated regardless — harmless, since the caller's own full
/// rebuild discards them anyway when this returns `false`.
fn try_update_group_in_place(
    entry: Option<&GroupCacheEntry>,
    commands: &[RenderCommand],
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
) -> bool {
    let Some(entry) = entry else {
        return false;
    };
    if entry.fast_path_layers.len() != commands.len() || entry.command_kinds.len() != commands.len()
    {
        return false;
    }
    let kinds_match = entry
        .command_kinds
        .iter()
        .zip(commands)
        .all(|(kind, command)| *kind == command.kind());
    if !kinds_match {
        return false;
    }
    entry
        .fast_path_layers
        .iter()
        .zip(commands)
        .all(|(layer, command)| match layer {
            Some(layer) => crate::render::try_update_fast_path(layer, command, &transform, opacity),
            None => false,
        })
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
/// group's own container. Since Step 6 of the AppKit render optimization work, a container's
/// `frame` no longer equals `root_layer.bounds()` — instead `anchorPoint = (0, 0)`,
/// `bounds.size == root_layer.bounds().size`, and `position` is this group's own *absolute*
/// origin (the accumulated `offset` chain from every ancestor `RenderGroup`, exactly what used to
/// get baked into every leaf's own `CGPath`/frame instead). This is why `origin` passed to
/// `replay_commands` below is always `Point::ZERO`: inside this container, `(0, 0)` *is* this
/// group's own absolute origin, so every existing leaf drawing helper (building geometry in
/// whatever coordinate space `origin`+`transform` describe) keeps working completely unchanged —
/// only *where that space starts* moved, from `root_layer` up to this container. The payoff: a
/// scrolled ancestor now costs this group one `setPosition`, not a `CGPath` rebuild —
/// `GroupCacheKey` has no `origin` field at all any more, see that struct's own doc comment.
/// `anchorPoint = (0, 0)` matches the convention every other multi-layer subtree in this backend
/// already uses (`build_image_container_layer`, `place_offscreen_image`,
/// `add_gradient_shape_layer`, every fill/stroke mask).
///
/// A container is `addSublayer`ed to `root_layer` only the first time it's ever created — see
/// `new_group_order`'s own doc comment (and `relayout_inner`'s) for how Z-order among a mix of
/// rebuilt and cache-hit groups stays correct without re-`addSublayer`ing every one of them,
/// every pass, forever. The actually expensive part (`CGPath`/`CAShapeLayer`/`CAGradientLayer`
/// construction) only happens when `GroupCacheKey` shows this group's replay inputs actually
/// changed since last time (painter design doc §15's renderer cache, acceptance criterion 14:
/// "画像・pathリソースを毎フレーム再生成しない").
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
    // `new_group_order`/`new_native_order`: this pass's traversal order, accumulated alongside the
    // liveness sets above and compared by `relayout_inner` (after the whole tree has been walked)
    // against `ReplayState::group_order`/`native_order` — see those fields' own doc comments.
    new_group_order: &mut Vec<u64>,
    new_native_order: &mut Vec<usize>,
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
    // Absolute space — this is what descendant groups need as their own `inherited_clip`, so it
    // is computed and threaded down exactly as before Step 6.
    let effective_clip = match (inherited_clip, group_clip) {
        (Some(a), Some(b)) => a.intersect(b),
        (Some(clip), None) | (None, Some(clip)) => Some(clip),
        (None, None) => None,
    };
    // This group's own local space (relative to `origin`) — see `ClipRelation`'s own doc comment
    // for why this, not the absolute `effective_clip` above, belongs in the cache key and in the
    // bounding-box culling test this group's own leaves are checked against.
    let local_clip = effective_clip.map(|clip| elwindui_core::base::Rect {
        x: clip.x - origin.x,
        y: clip.y - origin.y,
        width: clip.width,
        height: clip.height,
    });
    let clip_relation = ClipRelation::classify(local_clip, group.size);

    live_group_ids.insert(group.id);
    new_group_order.push(group.id);
    crate::render::stats::bump(|s| s.groups_visited += 1);

    let (container, is_new) = match state.group_layers.entry(group.id) {
        std::collections::hash_map::Entry::Occupied(entry) => (entry.get().clone(), false),
        std::collections::hash_map::Entry::Vacant(entry) => {
            crate::render::stats::bump(|s| s.layers_created += 1);
            let c = CALayer::new();
            c.setName(Some(&crate::render::paint_layer_name()));
            // Set once at creation, never again — see this function's own doc comment.
            c.setAnchorPoint(objc2_core_foundation::CGPoint::new(0.0, 0.0));
            entry.insert(c.clone());
            (c, true)
        }
    };
    crate::render::set_bounds_if_changed(
        &container,
        objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            root_layer.bounds().size,
        ),
    );
    crate::render::set_position_if_changed(
        &container,
        objc2_core_foundation::CGPoint::new(origin.x as f64, origin.y as f64),
    );
    // Set directly from `scale` (not via `add_sublayer_scaled`, which would also recursively
    // re-stamp every one of this container's *existing* sublayers on every single pass, including
    // cache hits) — `GroupCacheKey::scale` below already forces a full rebuild, which re-attaches
    // every descendant through `add_sublayer_scaled` and picks up this value, whenever the scale
    // genuinely changes. This keeps a cache-hit pass exactly as cheap as it was before this fix.
    crate::render::set_contents_scale_if_changed(&container, scale);
    if container.superlayer().is_none() {
        // First time this container has ever been attached. Z-order among the rest is fixed up
        // in bulk by `relayout_inner` once the whole tree has been walked, not here — see
        // `ReplayState::group_order`'s own doc comment.
        crate::render::stats::bump(|s| s.add_sublayer_calls += 1);
        root_layer.addSublayer(&container);
    }

    let visible = clip_relation != ClipRelation::Outside;
    crate::render::set_hidden_if_changed(&container, !visible);

    let key = GroupCacheKey {
        clip: clip_relation,
        transform,
        opacity,
        generation: group.generation,
        scale,
    };
    let stale =
        visible && (is_new || state.group_cache.get(&group.id).map(|entry| entry.key) != Some(key));
    if stale {
        // `leaf_clip` is `None` for `Unclipped`/`Inside` (no per-leaf culling needed — see
        // `ClipRelation`'s own doc comment) and the intersection rect, already local, for
        // `Partial`. `Outside` can never reach here: `stale` is `false` whenever `!visible`.
        let leaf_clip = match clip_relation {
            ClipRelation::Partial(rect) => Some(rect),
            ClipRelation::Unclipped | ClipRelation::Inside => None,
            ClipRelation::Outside => unreachable!("stale is false whenever !visible"),
        };

        // In-place fast path (AppKit render optimization work, narrowed Step 7b): a rebuild whose
        // *content* didn't actually change shape, only leaf properties (color, geometry, opacity),
        // never needs `removeFromSuperlayer`/`CALayer::new` at all — see
        // `try_update_group_in_place`'s own doc comment for the exact eligibility rule.
        let in_place = !is_new
            && leaf_clip.is_none()
            && try_update_group_in_place(
                state.group_cache.get(&group.id),
                &group.commands,
                transform,
                opacity,
            );
        if in_place {
            crate::render::stats::bump(|s| s.groups_updated_in_place += 1);
            if let Some(entry) = state.group_cache.get_mut(&group.id) {
                entry.key = key;
            }
            if let Some(entry) = state.group_cache.get(&group.id) {
                live_native_controls
                    .extend(entry.native_controls.iter().map(|(identity, ..)| *identity));
                append_cached_native_order(entry, true, new_native_order);
                live_image_ids.extend(&entry.image_ids);
                live_vector_image_ids.extend(&entry.vector_image_ids);
            }
        } else {
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
            // A flat, non-nested leaf list under a clip that needs no per-leaf culling can be
            // replayed directly (skipping `replay_commands`'s general recursion) so each
            // position's own fast-path `CALayer` can be captured for a future in-place update —
            // see `replay_flat_commands`'s own doc comment. Anything else (nested `Push*` scopes,
            // `NativeControl`, or a `Partial` clip needing bbox culling) still goes through the
            // general path, and simply never becomes eligible for in-place updates later.
            let mut native_controls = Vec::new();
            let (command_kinds, fast_path_layers) =
                if leaf_clip.is_none() && is_flat_leaf_only(&group.commands) {
                    let layers = replay_flat_commands(
                        &container,
                        &group.commands,
                        transform,
                        opacity,
                        &mut state.image_cache,
                        &mut state.vector_raster_cache,
                    );
                    let kinds = group.commands.iter().map(|c| c.kind()).collect();
                    (kinds, layers)
                } else {
                    replay_commands(
                        native,
                        &container,
                        &group.commands,
                        0,
                        elwindui_core::base::Point { x: 0.0, y: 0.0 },
                        leaf_clip,
                        transform,
                        opacity,
                        origin,
                        live_native_controls,
                        new_native_order,
                        &mut native_controls,
                        &mut state.image_cache,
                        &mut state.vector_raster_cache,
                    );
                    (Vec::new(), Vec::new())
                };
            let (image_ids, vector_image_ids) = resource_ids(&group.commands);
            live_image_ids.extend(image_ids.iter().copied());
            live_vector_image_ids.extend(vector_image_ids.iter().copied());
            state.group_cache.insert(
                group.id,
                GroupCacheEntry {
                    key,
                    native_controls,
                    image_ids,
                    vector_image_ids,
                    command_kinds,
                    fast_path_layers,
                },
            );
        }
    } else {
        crate::render::stats::bump(|s| s.groups_cache_hit += 1);
        if let Some(entry) = state.group_cache.get(&group.id) {
            live_native_controls
                .extend(entry.native_controls.iter().map(|(identity, ..)| *identity));
            append_cached_native_order(entry, visible, new_native_order);
            live_image_ids.extend(&entry.image_ids);
            live_vector_image_ids.extend(&entry.vector_image_ids);
        }
    }

    // Native islands are real `NSView` subviews, not `CALayer` sublayers of `container` — so
    // unlike `container`'s own `position` (`set_position_if_changed` above, unconditional on
    // every pass), their frame does *not* ride along for free when only an ancestor's offset
    // changed and this group's own `GroupCacheKey` (no `origin` field, see that struct's own doc
    // comment) stayed a cache hit. Re-derive each control's absolute frame from this pass's fresh
    // `origin` and this group's cached `local_rect` unconditionally, regardless of whether the
    // branch above rebuilt or hit cache — otherwise a control whose owning group never itself goes
    // stale (e.g. a `Button` sitting in a row whose *ancestor* spacing changed, but whose own
    // local offset within that row didn't) freezes at its last-applied screen position forever.
    if let Some(entry) = state.group_cache.get(&group.id) {
        for &(identity, owner_id, local_rect) in &entry.native_controls {
            let (container, _is_new) = native.island(identity, owner_id);
            container.setFrame(NSRect::new(
                objc2_foundation::NSPoint::new(
                    (origin.x + local_rect.x) as f64,
                    (origin.y + local_rect.y) as f64,
                ),
                objc2_foundation::NSSize::new(local_rect.width as f64, local_rect.height as f64),
            ));
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
            new_group_order,
            new_native_order,
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
    // The owning group's own absolute origin (its `container`'s `position` — see `replay_group`'s
    // own doc comment) — `origin` above is local to this replay (always `Point::ZERO` when this
    // is the group's own top-level call, per `replay_group`). Used *only* by the `NativeControl`
    // arm: a native island is a real `NSView` subview of the host, not a sublayer of any group
    // container, so its frame must be in real absolute screen coordinates — the one place these
    // two coordinate spaces meet.
    group_origin: elwindui_core::base::Point,
    live_native_controls: &mut HashSet<usize>,
    new_native_order: &mut Vec<usize>,
    // `(identity, owner_id, visible_local)` for every `NativeControl` command this call (or its
    // own `Push*` recursion) placed — collected here rather than re-derived from
    // `live_native_controls`'s before/after diff so the owning `GroupCacheEntry` can keep each
    // control's local rect around for `replay_group`'s unconditional per-pass resync step.
    native_control_rects: &mut Vec<(usize, u64, elwindui_core::base::Rect)>,
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
                container.setName(Some(&crate::render::paint_layer_name()));
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
                    group_origin,
                    live_native_controls,
                    new_native_order,
                    native_control_rects,
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
                    group_origin,
                    live_native_controls,
                    new_native_order,
                    native_control_rects,
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
                    group_origin,
                    live_native_controls,
                    new_native_order,
                    native_control_rects,
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
                new_native_order.push(identity);
                // `rect`/`clip` are in this call's own local space (see `group_origin`'s own doc
                // comment) — the culled/visible portion is computed there, exactly as before, and
                // only the final `NSView` frame is translated into real absolute screen
                // coordinates via `group_origin` (a native island is a real subview, not a
                // sublayer of any group container, so it needs actual absolute placement).
                let local_rect = elwindui_core::base::Rect {
                    x: origin.x + rect.x,
                    y: origin.y + rect.y,
                    width: rect.width,
                    height: rect.height,
                };
                let visible_local = clip
                    .and_then(|clip| local_rect.intersect(clip))
                    .unwrap_or(local_rect);
                if visible_local.width <= 0.0 || visible_local.height <= 0.0 {
                    idx += 1;
                    continue;
                }
                native_control_rects.push((identity, *owner_id, visible_local));
                let visible_rect = elwindui_core::base::Rect {
                    x: group_origin.x + visible_local.x,
                    y: group_origin.y + visible_local.y,
                    width: visible_local.width,
                    height: visible_local.height,
                };
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
                    x: local_rect.x - visible_local.x,
                    y: local_rect.y - visible_local.y,
                    width: local_rect.width,
                    height: local_rect.height,
                });
                idx += 1;
            }
            command => {
                let world = elwindui_core::base::AffineTransform::translation(origin.x, origin.y)
                    .concat(&transform);
                if geometry_bounds(command, &world)
                    .is_none_or(|bounds| clip.is_none_or(|clip| bounds.intersect(clip).is_some()))
                {
                    // A run of >=2 consecutive same-solid-color fills batches into one
                    // `CAShapeLayer` before anything else is tried — see `try_batch_fills`'s own
                    // doc comment on why this is safe (a run can never cross a `Push*`/`Pop*`/
                    // `NativeControl` boundary) and why it's deliberately only reachable from
                    // here, never from `replay_flat_commands`. A `0` result (no run of 2+ found)
                    // falls through to the existing single-command handling completely unchanged.
                    let batched =
                        crate::render::try_batch_fills(layer, commands, idx, &world, clip, opacity);
                    if batched > 0 {
                        idx += batched;
                    } else {
                        // `try_fast_path` may also consume `commands[idx + 1]` (the FillRect+
                        // StrokeRect fusion case) — the shared rect makes this command's own cull
                        // decision above a reasonable stand-in for the pair's, the same bounding-box
                        // approximation `Shape::hit_test_content` already documents elsewhere in this
                        // codebase.
                        let next = commands.get(idx + 1);
                        let (consumed, _) =
                            crate::render::try_fast_path(layer, command, next, &world, opacity);
                        if consumed == 0 {
                            replay_paint_command(
                                layer,
                                command,
                                origin,
                                transform,
                                opacity,
                                image_cache,
                                vector_raster_cache,
                            );
                            idx += 1;
                        } else {
                            idx += consumed;
                        }
                    }
                } else {
                    idx += 1;
                }
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
            shape_layer.setName(Some(&crate::render::paint_layer_name()));
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
            shape_layer.setName(Some(&crate::render::paint_layer_name()));
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
            text_layer.setName(Some(&crate::render::paint_layer_name()));
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
    use elwindui_core::base::{AffineTransform, CornerRadius, Point, Rect, Size};
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

    fn parent_with_children(child_ids: &[u64]) -> RenderGroup {
        let mut parent = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        parent.children = child_ids
            .iter()
            .map(|&id| solid_fill_rect_group(id))
            .collect();
        parent
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_once(root_layer: &Retained<CALayer>, group: &RenderGroup, state: &mut ReplayState) {
        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut live_image_ids = HashSet::new();
        let mut live_vector_image_ids = HashSet::new();
        let mut new_group_order = Vec::new();
        let mut new_native_order = Vec::new();
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
            &mut new_group_order,
            &mut new_native_order,
            state,
        );
        // Mirrors what `relayout_inner` does after the top-level `replay_group` call: apply the
        // Z-order repair only when this pass's traversal order actually differs from last time.
        if state.group_order != new_group_order {
            for id in &new_group_order {
                if let Some(container) = state.group_layers.get(id) {
                    crate::render::stats::bump(|s| s.add_sublayer_calls += 1);
                    root_layer.addSublayer(container);
                }
            }
            state.group_order = new_group_order;
        }
    }

    #[test]
    fn a_visible_cached_group_restores_native_traversal_order() {
        let entry = GroupCacheEntry {
            key: GroupCacheKey {
                clip: ClipRelation::Unclipped,
                transform: AffineTransform::identity(),
                opacity: 1.0,
                generation: 0,
                scale: 1.0,
            },
            native_controls: vec![
                (
                    17,
                    1,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    },
                ),
                (
                    23,
                    2,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    },
                ),
            ],
            image_ids: Vec::new(),
            vector_image_ids: Vec::new(),
            command_kinds: Vec::new(),
            fast_path_layers: Vec::new(),
        };
        let mut native_order = vec![3];

        append_cached_native_order(&entry, true, &mut native_order);
        assert_eq!(native_order, vec![3, 17, 23]);

        append_cached_native_order(&entry, false, &mut native_order);
        assert_eq!(native_order, vec![3, 17, 23]);
    }

    /// The §22 no-op assertion: replaying an unchanged tree a second time must mutate the Core
    /// Animation tree not at all — no new/removed layers, no re-`addSublayer`, no `CGPath`
    /// rebuild. This is the regression harness every later optimization step is checked against.
    #[test]
    fn second_replay_of_an_unchanged_group_mutates_nothing() {
        let root_layer = CALayer::new();
        let group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();

        replay_once(&root_layer, &group, &mut state);
        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        assert_eq!(stats.groups_rebuilt, 0, "unchanged group must not rebuild");
        assert_eq!(stats.groups_cache_hit, 1);
        assert_eq!(
            stats.layers_created, 0,
            "cache hit must create no new CALayer"
        );
        assert_eq!(stats.layers_removed, 0);
        assert_eq!(
            stats.cgpaths_created, 0,
            "cache hit must not rebuild any CGPath"
        );
        assert_eq!(
            stats.add_sublayer_calls, 0,
            "an already-attached, unreordered container must not be re-addSublayer'd"
        );
    }

    #[test]
    fn a_reordered_child_list_repairs_z_order_without_rebuilding_content() {
        let root_layer = CALayer::new();
        let mut state = ReplayState::default();

        replay_once(&root_layer, &parent_with_children(&[2, 3]), &mut state);
        assert_eq!(state.group_order, vec![1, 2, 3]);

        crate::render::stats::reset();
        replay_once(&root_layer, &parent_with_children(&[3, 2]), &mut state);

        let stats = crate::render::stats::snapshot();
        assert_eq!(
            stats.groups_rebuilt, 0,
            "reordering alone must not rebuild any group's own content"
        );
        assert!(
            stats.add_sublayer_calls > 0,
            "a changed traversal order must trigger the Z-order repair pass"
        );
        assert_eq!(state.group_order, vec![1, 3, 2]);
    }

    #[test]
    fn replaying_at_a_different_unclipped_origin_moves_the_container_without_rebuilding() {
        // The whole point of Step 6 (AppKit render optimization work #46): a scrolled ancestor's
        // offset change must cost exactly one `setPosition` on the group's own persistent
        // container, never a rebuild — see `GroupCacheKey`'s and `ClipRelation`'s own doc
        // comments for why dropping `origin` from the cache key (in favor of a stable
        // `Unclipped`/`Inside`/`Outside` classification) is what makes that true.
        let root_layer = CALayer::new();
        let group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();
        replay_once(&root_layer, &group, &mut state);
        let container = state.group_layers.get(&1).unwrap().clone();
        assert_eq!(
            container.position(),
            objc2_core_foundation::CGPoint::new(0.0, 0.0)
        );

        crate::render::stats::reset();
        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut live_image_ids = HashSet::new();
        let mut live_vector_image_ids = HashSet::new();
        let mut new_group_order = Vec::new();
        let mut new_native_order = Vec::new();
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
            &mut new_group_order,
            &mut new_native_order,
            &mut state,
        );

        let stats = crate::render::stats::snapshot();
        assert_eq!(
            stats.groups_rebuilt, 0,
            "an unclipped group's origin must not force a rebuild"
        );
        assert_eq!(stats.groups_cache_hit, 1);
        assert_eq!(stats.layers_created, 0);
        assert_eq!(stats.cgpaths_created, 0);
        assert_eq!(
            container.position(),
            objc2_core_foundation::CGPoint::new(5.0, 0.0),
            "the container itself must carry the new absolute origin"
        );
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

    #[test]
    fn solid_fill_rect_takes_the_fast_path_and_builds_no_cgpath() {
        let root_layer = CALayer::new();
        let group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        assert_eq!(
            stats.cgpaths_created, 0,
            "a solid FillRect must not build a CGPath — see render::fastpath"
        );
        // 2, not 1: the group's own persistent container CALayer (replay_group) plus the one
        // fast-path CALayer for the FillRect itself.
        assert_eq!(stats.layers_created, 2);
    }

    #[test]
    fn fill_and_stroke_rect_on_the_same_rect_fuse_into_one_layer() {
        fn fill_and_stroke(same_rect: bool) -> RenderGroup {
            let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
            let fill_rect = Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            };
            let stroke_rect = if same_rect {
                fill_rect
            } else {
                Rect {
                    x: 20.0,
                    ..fill_rect
                }
            };
            group.commands = vec![
                RenderCommand::FillRect {
                    rect: fill_rect,
                    brush: Brush::Solid(Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    }),
                },
                RenderCommand::StrokeRect {
                    rect: stroke_rect,
                    brush: Brush::Solid(Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 255,
                    }),
                    stroke: elwindui_core::graphics::StrokeStyle::default(),
                },
            ];
            group
        }

        let fused_layers = {
            let root_layer = CALayer::new();
            let mut state = ReplayState::default();
            crate::render::stats::reset();
            replay_once(&root_layer, &fill_and_stroke(true), &mut state);
            crate::render::stats::snapshot().layers_created
        };
        let unfused_layers = {
            let root_layer = CALayer::new();
            let mut state = ReplayState::default();
            crate::render::stats::reset();
            replay_once(&root_layer, &fill_and_stroke(false), &mut state);
            crate::render::stats::snapshot().layers_created
        };

        assert_eq!(
            unfused_layers - fused_layers,
            1,
            "a FillRect immediately followed by a StrokeRect on the *same* rect must produce \
             exactly one fewer CALayer than the same pair on different rects"
        );
    }

    #[test]
    fn a_rotated_group_falls_back_to_the_general_cgpath_path() {
        let root_layer = CALayer::new();
        let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        group.commands = vec![
            RenderCommand::PushTransform {
                transform: AffineTransform::rotation(0.3),
            },
            RenderCommand::FillRect {
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
            },
            RenderCommand::PopTransform,
        ];
        let mut state = ReplayState::default();

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        assert!(
            stats.cgpaths_created > 0,
            "a rotated group must fall back to the general CGPath path, \
             since CALayer.cornerRadius/backgroundColor have no rotation-aware equivalent"
        );
    }

    #[test]
    fn clip_relation_classify_matches_its_own_contract() {
        let size = Size {
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(ClipRelation::classify(None, size), ClipRelation::Unclipped);
        assert_eq!(
            ClipRelation::classify(
                Some(Rect {
                    x: -5.0,
                    y: -5.0,
                    width: 100.0,
                    height: 100.0
                }),
                size
            ),
            ClipRelation::Inside,
            "a clip that fully contains the group's own extent must classify as Inside"
        );
        assert_eq!(
            ClipRelation::classify(
                Some(Rect {
                    x: 50.0,
                    y: 50.0,
                    width: 10.0,
                    height: 10.0
                }),
                size
            ),
            ClipRelation::Outside,
            "a clip disjoint from the group's own extent must classify as Outside"
        );
        assert_eq!(
            ClipRelation::classify(
                Some(Rect {
                    x: 5.0,
                    y: 5.0,
                    width: 100.0,
                    height: 100.0
                }),
                size
            ),
            ClipRelation::Partial(Rect {
                x: 5.0,
                y: 5.0,
                width: 5.0,
                height: 5.0
            }),
            "a clip straddling the group's own edge must classify as Partial with the intersection"
        );
    }

    #[test]
    fn a_group_outside_the_inherited_clip_hides_its_container_and_skips_rebuilding() {
        let root_layer = CALayer::new();
        let mut group = solid_fill_rect_group(1);
        group.size = Size {
            width: 10.0,
            height: 10.0,
        };
        let mut state = ReplayState::default();

        let mut live_native_controls = HashSet::new();
        let mut live_group_ids = HashSet::new();
        let mut live_image_ids = HashSet::new();
        let mut live_vector_image_ids = HashSet::new();
        let mut new_group_order = Vec::new();
        let mut new_native_order = Vec::new();
        // A clip entirely disjoint from the group's own (0,0,10,10) extent.
        let disjoint_clip = Some(Rect {
            x: 1000.0,
            y: 1000.0,
            width: 10.0,
            height: 10.0,
        });
        replay_group(
            &NoNativeIslands,
            &root_layer,
            &group,
            Point { x: 0.0, y: 0.0 },
            disjoint_clip,
            AffineTransform::identity(),
            1.0,
            1.0,
            &mut live_native_controls,
            &mut live_group_ids,
            &mut live_image_ids,
            &mut live_vector_image_ids,
            &mut new_group_order,
            &mut new_native_order,
            &mut state,
        );

        let stats = crate::render::stats::snapshot();
        assert_eq!(
            stats.groups_rebuilt, 0,
            "an Outside group must never be rebuilt"
        );
        assert_eq!(stats.cgpaths_created, 0);
        let container = state.group_layers.get(&1).unwrap();
        assert!(
            container.isHidden(),
            "an Outside group's container must be hidden"
        );
    }

    #[test]
    fn a_partial_clip_group_rebuilds_when_the_local_intersection_changes() {
        let root_layer = CALayer::new();
        let mut group = solid_fill_rect_group(1);
        group.size = Size {
            width: 10.0,
            height: 10.0,
        };
        let mut state = ReplayState::default();

        let replay_with_clip = |root_layer: &Retained<CALayer>,
                                group: &RenderGroup,
                                state: &mut ReplayState,
                                inherited_clip: Option<Rect>| {
            let mut live_native_controls = HashSet::new();
            let mut live_group_ids = HashSet::new();
            let mut live_image_ids = HashSet::new();
            let mut live_vector_image_ids = HashSet::new();
            let mut new_group_order = Vec::new();
            let mut new_native_order = Vec::new();
            replay_group(
                &NoNativeIslands,
                root_layer,
                group,
                Point { x: 0.0, y: 0.0 },
                inherited_clip,
                AffineTransform::identity(),
                1.0,
                1.0,
                &mut live_native_controls,
                &mut live_group_ids,
                &mut live_image_ids,
                &mut live_vector_image_ids,
                &mut new_group_order,
                &mut new_native_order,
                state,
            );
        };

        // Straddles the group's right edge (Partial).
        replay_with_clip(
            &root_layer,
            &group,
            &mut state,
            Some(Rect {
                x: -100.0,
                y: -100.0,
                width: 105.0,
                height: 200.0,
            }),
        );

        crate::render::stats::reset();
        // Same origin, but the viewport edge moved — the local intersection rect differs even
        // though nothing about the group's own content changed.
        replay_with_clip(
            &root_layer,
            &group,
            &mut state,
            Some(Rect {
                x: -100.0,
                y: -100.0,
                width: 103.0,
                height: 200.0,
            }),
        );

        let stats = crate::render::stats::snapshot();
        assert_eq!(
            stats.groups_rebuilt, 1,
            "a changed Partial intersection must still force a rebuild — only Inside/Outside/\
             Unclipped are stable across small viewport shifts"
        );
    }

    /// The Step 7b happy path: a flat, single-leaf, fast-path-eligible group whose leaf property
    /// (not shape) changes must update its existing `CALayer` in place — no
    /// `removeFromSuperlayer`/`CALayer::new` at all.
    #[test]
    fn changing_a_flat_fast_path_groups_color_updates_in_place() {
        let root_layer = CALayer::new();
        let mut group = solid_fill_rect_group(1);
        let mut state = ReplayState::default();
        replay_once(&root_layer, &group, &mut state);

        group.commands = vec![RenderCommand::FillRect {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
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
        assert_eq!(
            stats.groups_updated_in_place, 1,
            "an unchanged flat leaf-kind list must update in place"
        );
        assert_eq!(stats.groups_rebuilt, 0);
        assert_eq!(
            stats.layers_created, 0,
            "in-place update must create no new CALayer"
        );
        assert_eq!(
            stats.layers_removed, 0,
            "in-place update must not remove any CALayer"
        );
    }

    /// A fused fill+stroke pair never gets a tracked per-position `CALayer` (see
    /// `try_fast_path`'s own doc comment on why) — so a subsequent content change on that same
    /// group must always fall back to a full rebuild, never attempt (and potentially corrupt) an
    /// in-place update.
    #[test]
    fn a_fused_fill_and_stroke_group_never_updates_in_place() {
        let root_layer = CALayer::new();
        let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let stroke = elwindui_core::graphics::StrokeStyle::default();
        group.commands = vec![
            RenderCommand::FillRect {
                rect,
                brush: Brush::Solid(Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            },
            RenderCommand::StrokeRect {
                rect,
                brush: Brush::Solid(Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                stroke: stroke.clone(),
            },
        ];
        let mut state = ReplayState::default();
        replay_once(&root_layer, &group, &mut state);

        group.commands = vec![
            RenderCommand::FillRect {
                rect,
                brush: Brush::Solid(Color {
                    r: 0,
                    g: 255,
                    b: 0,
                    a: 255,
                }),
            },
            RenderCommand::StrokeRect {
                rect,
                brush: Brush::Solid(Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                stroke,
            },
        ];
        group.generation += 1;

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        assert_eq!(
            stats.groups_updated_in_place, 0,
            "a fused pair has no single tracked CALayer per position and must never update in place"
        );
        assert_eq!(stats.groups_rebuilt, 1);
    }

    /// A leaf `try_fast_path` declines (here: `FillRoundedRect` with non-uniform per-corner radii)
    /// leaves a `None` in that position's `fast_path_layers` forever, so a group containing one
    /// must always take the full-rebuild path, never in-place — same disqualification rule as the
    /// fusion case above, via a different mechanism (a single non-eligible leaf rather than a
    /// two-command fusion).
    #[test]
    fn a_group_with_a_non_fast_path_eligible_leaf_never_updates_in_place() {
        let root_layer = CALayer::new();
        let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let non_uniform_radii = CornerRadius {
            top_left: 1.0,
            top_right: 2.0,
            bottom_right: 3.0,
            bottom_left: 4.0,
        };
        group.commands = vec![RenderCommand::FillRoundedRect {
            rect,
            radii: non_uniform_radii,
            brush: Brush::Solid(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }),
        }];
        let mut state = ReplayState::default();
        replay_once(&root_layer, &group, &mut state);

        group.commands = vec![RenderCommand::FillRoundedRect {
            rect,
            radii: non_uniform_radii,
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
        assert_eq!(
            stats.groups_updated_in_place, 0,
            "a non-fast-path-eligible leaf must never be updated in place"
        );
        assert_eq!(stats.groups_rebuilt, 1);
    }

    /// Step 8's batching happy path: several adjacent solid fills sharing one color must collapse
    /// into a single `CAShapeLayer` instead of one layer per fill.
    #[test]
    fn a_run_of_same_color_fills_batches_into_one_shape_layer() {
        let root_layer = CALayer::new();
        let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        let color = Color {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        };
        group.commands = (0..5)
            .map(|i| RenderCommand::FillRect {
                rect: Rect {
                    x: i as f32 * 12.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                brush: Brush::Solid(color),
            })
            .collect();
        let mut state = ReplayState::default();

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        // 2, not 6: the group's own persistent container CALayer plus ONE batched CAShapeLayer
        // for all 5 same-color fills, instead of one fast-path CALayer per fill.
        assert_eq!(
            stats.layers_created, 2,
            "5 adjacent same-color fills must batch into a single CAShapeLayer"
        );
    }

    /// Adjacent fills with *different* colors must never batch — each stays its own fast-path
    /// layer, since a shared `CAShapeLayer` can only carry one `fillColor`.
    #[test]
    fn differently_colored_adjacent_fills_do_not_batch() {
        let root_layer = CALayer::new();
        let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        group.commands = (0..3)
            .map(|i| RenderCommand::FillRect {
                rect: Rect {
                    x: i as f32 * 12.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                brush: Brush::Solid(Color {
                    r: (i as u8) * 50,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            })
            .collect();
        let mut state = ReplayState::default();

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        // 4: container + 3 individual fast-path layers, one per distinct color.
        assert_eq!(stats.layers_created, 4);
    }

    /// The guide's own explicit Z-order concern: a same-color fill run must never batch *across*
    /// an intervening command of a different kind — here, real text sandwiched between two
    /// identically-colored fills. Batching the two fills together would either have to skip the
    /// text command (silently dropping it from paint order) or splice the merged shape layer in
    /// front of/behind the text incorrectly; verifying the exact `RenderCommand` kind sequence
    /// present in `command_kinds` after a rebuild is a much stronger check than counting layers,
    /// since a layer-count assertion alone could not distinguish "correctly kept separate" from
    /// "wrongly merged in a way that still happens to produce 3 layers".
    #[test]
    fn a_batchable_run_never_crosses_an_intervening_text_command() {
        let root_layer = CALayer::new();
        let mut group = RenderGroup::new(1, Point { x: 0.0, y: 0.0 }, None);
        let red = Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        group.commands = vec![
            RenderCommand::FillRect {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                brush: Brush::Solid(red),
            },
            RenderCommand::Text {
                content: "hi".to_string(),
                rect: Rect {
                    x: 20.0,
                    y: 0.0,
                    width: 50.0,
                    height: 20.0,
                },
                style: elwindui_core::graphics::ComputedTextStyle::default(),
                foreground: None,
                alignment: elwindui_core::ui::TextAlignment::Left,
            },
            RenderCommand::FillRect {
                rect: Rect {
                    x: 80.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                brush: Brush::Solid(red),
            },
        ];
        let mut state = ReplayState::default();

        crate::render::stats::reset();
        replay_once(&root_layer, &group, &mut state);

        let stats = crate::render::stats::snapshot();
        // container + fill + text + fill: the text command must still be its own CATextLayer,
        // never skipped over or merged into a batch with either fill.
        assert_eq!(stats.layers_created, 4);
        assert_eq!(stats.text_layers_created, 1);
    }
}
