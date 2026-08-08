//! `CALayer` backing-store resolution: keeping every manually created layer in this backend's
//! render trees at the host window's real `backingScaleFactor` instead of `CALayer::new()`'s
//! default `1.0`.
//!
//! `CALayer::new()` always starts at `contentsScale == 1.0`, and Core Animation does **not**
//! inherit `contentsScale` from a superlayer — only the backing layer AppKit itself creates for a
//! layer-backed `NSView` (`view.layer()` after `setWantsLayer(true)`) is kept in sync with the
//! window's `backingScaleFactor`. Every layer this backend builds by hand (`CATextLayer` glyphs,
//! `CAShapeLayer` fills/strokes, `CAGradientLayer`s, clip/mask layers, image containers, SVG scene
//! layers) is therefore rasterized at 1 pixel per point by default and then upscaled by the
//! compositor on a Retina display — the root cause of blurry `TextBlock` text and soft vector
//! edges alike.
//!
//! The fix is to stamp the correct scale down the tree at the moment a layer is attached to its
//! parent (`add_sublayer_scaled`/`set_mask_scaled`), rather than at each of the ~25 individual
//! creation sites: every attachment site already has the parent (and therefore its resolved
//! scale) in hand, and several helpers build a multi-layer subtree before it has a parent at all
//! (e.g. `build_image_container_layer`, `place_offscreen_image`), so a creation-time stamp would
//! miss those inner layers. See `host::TreeHostView::backing_scale_factor` for where the
//! authoritative scale value comes from.

use objc2_core_foundation::CGFloat;
use objc2_quartz_core::{CALayer, CATransaction};

/// Suppresses Core Animation's implicit (default ~0.25s) property animations for the duration of
/// one render synchronization pass. Every `setFrame`/`addSublayer`/`setPath`/`setString`/etc. this
/// backend issues outside this guard runs inside AppKit's own ambient transaction and therefore
/// animates implicitly — harmless for a genuinely new value, but a visible "smear" on every
/// no-op-content, layout-only relayout (a window resize, a theme repaint) where nothing the user
/// asked to animate actually changed. Always `Drop`-based, never a bare `begin()`/`commit()` pair,
/// because the caller (`TreeHostView::relayout_inner`) has several early `return`s.
pub(crate) struct ImplicitAnimationGuard;

impl ImplicitAnimationGuard {
    pub(crate) fn begin() -> Self {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        Self
    }
}

impl Drop for ImplicitAnimationGuard {
    fn drop(&mut self) {
        CATransaction::commit();
    }
}

/// Sets `layer`'s `contentsScale` to `scale`, and recursively does the same for its mask and
/// every sublayer (and their own masks and sublayers, transitively).
///
/// Masks are visited explicitly because `CALayer.mask` is not part of `sublayers` — a
/// `CAShapeLayer` mask left at `1.0` produces visibly stair-stepped clip edges even when the
/// layer it masks is correctly at `2.0`.
pub(crate) fn set_contents_scale_recursive(layer: &CALayer, scale: CGFloat) {
    layer.setContentsScale(scale);
    if let Some(mask) = layer.mask() {
        set_contents_scale_recursive(&mask, scale);
    }
    if let Some(sublayers) = unsafe { layer.sublayers() } {
        for sublayer in sublayers.iter() {
            set_contents_scale_recursive(&sublayer, scale);
        }
    }
}

/// `parent.addSublayer(child)` plus `set_contents_scale_recursive(child, parent.contentsScale())`.
///
/// This is the single choke point every layer attachment in this backend should go through
/// instead of a raw `addSublayer` call. The stamp is recursive rather than a single
/// `setContentsScale` on `child` alone because `child` may already own a subtree built before it
/// had a parent (e.g. `build_image_container_layer`, `place_offscreen_image`, the vector pattern
/// wrapper in `render::vector::paint`).
///
/// A parent still at the default `1.0` (an offscreen rasterization root, or the golden-test
/// harness, which never attaches to a real window) stamps `1.0` onto `child` — a no-op there by
/// construction, which is what keeps every existing golden byte-identical.
pub(crate) fn add_sublayer_scaled(parent: &CALayer, child: &CALayer) {
    super::stats::bump(|s| s.add_sublayer_calls += 1);
    parent.addSublayer(child);
    set_contents_scale_recursive(child, parent.contentsScale());
}

/// `layer.setMask(Some(mask))` plus the same recursive stamp — the mask counterpart of
/// `add_sublayer_scaled`, needed because `CALayer.mask` is not reachable through `sublayers`.
pub(crate) fn set_mask_scaled(layer: &CALayer, mask: &CALayer) {
    unsafe { layer.setMask(Some(mask)) };
    set_contents_scale_recursive(mask, layer.contentsScale());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sublayer_scaled_propagates_to_sublayer_mask_and_grandchild() {
        let parent = CALayer::new();
        parent.setContentsScale(2.0);

        let child = CALayer::new();
        let grandchild = CALayer::new();
        child.addSublayer(&grandchild);
        let mask = CALayer::new();
        unsafe { child.setMask(Some(&mask)) };

        add_sublayer_scaled(&parent, &child);

        assert_eq!(child.contentsScale(), 2.0);
        assert_eq!(grandchild.contentsScale(), 2.0);
        assert_eq!(mask.contentsScale(), 2.0);
    }

    #[test]
    fn add_sublayer_scaled_is_a_no_op_at_the_default_scale() {
        // Guards golden-test invariance: offscreen rasterization roots and the golden-test
        // harness never attach to a real window, so they stay at the default 1.0, and this must
        // not silently change that.
        let parent = CALayer::new();
        let child = CALayer::new();

        add_sublayer_scaled(&parent, &child);

        assert_eq!(parent.contentsScale(), 1.0);
        assert_eq!(child.contentsScale(), 1.0);
    }

    #[test]
    fn set_mask_scaled_propagates_to_the_masks_own_sublayers() {
        let layer = CALayer::new();
        layer.setContentsScale(3.0);

        let mask = CALayer::new();
        let mask_child = CALayer::new();
        mask.addSublayer(&mask_child);

        set_mask_scaled(&layer, &mask);

        assert_eq!(mask.contentsScale(), 3.0);
        assert_eq!(mask_child.contentsScale(), 3.0);
    }
}
