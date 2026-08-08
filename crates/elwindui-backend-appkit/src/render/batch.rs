//! Consecutive same-color fill batching: replacing N `CAShapeLayer`s (or N `fastpath` `CALayer`s)
//! for a run of adjacent `FillRect`/`FillRoundedRect` commands sharing one solid brush with a
//! single `CAShapeLayer` holding one combined `CGMutablePath` (each member's own rounded-rect
//! geometry appended as its own subpath, via `CGMutablePath::add_path`). See the AppKit render
//! optimization implementation guide §8/§18's Phase 5 "batching" item — narrowed to solid-fill
//! runs only, for the same reason `fastpath` restricts itself to `Brush::Solid`: a gradient/image
//! `Brush` would need the deep, per-frame-hazardous `Image` comparison this codebase's own
//! convention forbids (see `command_fingerprint.rs`'s own doc comment on the same rule).
//!
//! Deliberately only ever consulted from `replay_commands`'s general recursion, never from
//! `replay_flat_commands` (Step 7b's leaf-diff-cache-eligible path): a batched run has no single
//! per-position `CALayer` to hand back for `GroupCacheEntry::fast_path_layers`, so a group
//! containing a batchable run simply never becomes eligible for Step 7b's own in-place update —
//! the same fate as any other group whose leaf list isn't uniformly fast-path-eligible, not a
//! special case carved out for batching specifically.
//!
//! A "consecutive" run, by construction, can never cross a `Push*`/`Pop*`/`NativeControl`
//! boundary (those are different `RenderCommand` variants, so the forward scan below stops the
//! instant it sees one) — batching therefore never reorders anything relative to any other
//! command, sidestepping the Z-order hazard the optimization guide itself warns this kind of
//! change can otherwise create invisibly to a static screenshot.

use elwindui_core::base::{AffineTransform, Rect};
use elwindui_core::graphics::{Brush, RenderCommand};
use objc2::rc::Retained;
use objc2_core_graphics::CGMutablePath;
use objc2_quartz_core::CALayer;

use super::fastpath::{fill_shape, solid_color};
use super::geometry::geometry_bounds;
use super::path::rounded_rect_cgpath;

/// Scans forward from `commands[start]` for a maximal run of consecutive `FillRect`/
/// `FillRoundedRect` commands sharing the exact same solid `Color`. Returns `0` (having drawn
/// nothing and touched no layer) when fewer than 2 such commands are found, or `commands[start]`
/// itself isn't a solid fill at all — the caller's own per-command handling for `commands[start]`
/// alone is unchanged in that case, exactly as if this function had never been tried.
///
/// Otherwise draws every *visible* run member — bbox-culled against `clip` exactly as
/// `replay_commands`'s own per-command check already does; a culled member simply contributes no
/// subpath rather than splitting or reordering the run — as one `CAShapeLayer`, and returns the
/// run's full length (including any culled members, which the caller must still not visit again).
pub(crate) fn try_batch_fills(
    layer: &Retained<CALayer>,
    commands: &[RenderCommand],
    start: usize,
    world: &AffineTransform,
    clip: Option<Rect>,
    opacity: f32,
) -> usize {
    let Some(color) = fill_shape(&commands[start]).and_then(|(_, _, brush)| solid_color(brush))
    else {
        return 0;
    };

    let mut end = start + 1;
    while end < commands.len() {
        let same_color = fill_shape(&commands[end])
            .and_then(|(_, _, brush)| solid_color(brush))
            .is_some_and(|c| c == color);
        if !same_color {
            break;
        }
        end += 1;
    }
    if end - start < 2 {
        return 0;
    }

    let is_visible = |command: &RenderCommand| {
        geometry_bounds(command, world)
            .is_none_or(|bounds| clip.is_none_or(|c| bounds.intersect(c).is_some()))
    };

    let combined = CGMutablePath::new();
    let mut union_bounds: Option<Rect> = None;
    for command in &commands[start..end] {
        if !is_visible(command) {
            continue;
        }
        let Some((rect, radii, _)) = fill_shape(command) else {
            unreachable!("every member in [start, end) was already confirmed fill_shape-extractable above")
        };
        let subpath = rounded_rect_cgpath(world, rect, radii);
        unsafe {
            CGMutablePath::add_path(Some(&combined), std::ptr::null(), Some(&subpath));
        }
        union_bounds = Some(union_bounds.map_or(rect, |u| u.union(rect)));
    }
    if let Some(bounds) = union_bounds {
        super::add_shape_layer(layer, &combined, Some(&Brush::Solid(color)), None, opacity, bounds);
    }
    end - start
}
