//! `VectorImage` — the SVG-non-specific, cheap-clone handle a loaded vector document is carried
//! as through `RenderCommand`/`RenderContext` (SVG読み込み・ベクター描画対応 実装指示書 §5). Mirrors
//! `Image`(`super::image`)'s own design: an immutable `Arc<...Data>` payload behind a plain value
//! type, never `Arc<VectorImage>` in public APIs (指示書§1.2).

use super::image::Image;
use super::vector_scene::VectorGroup;
use crate::base::{Rect, Size};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// What a `builtin::Image`-style element draws — a raster bitmap or a loaded vector document, kept
/// as an enum rather than two separate optional fields (指示書§14: enumが唯一の値集合機構) (SVG読
/// み込み・ベクター描画対応 実装指示書§24). Never `Arc<VectorImage>` — `VectorImage` is already
/// cheap to clone on its own (指示書§1.2).
#[derive(Debug, Clone)]
pub enum ImageSource {
    Raster(Image),
    Vector(VectorImage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VectorImageId(u64);

fn next_vector_image_id() -> VectorImageId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    VectorImageId(NEXT.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreserveAspectRatioAlign {
    None,
    XMinYMin,
    XMidYMin,
    XMaxYMin,
    XMinYMid,
    XMidYMid,
    XMaxYMid,
    XMinYMax,
    XMidYMax,
    XMaxYMax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreserveAspectRatioMeetOrSlice {
    Meet,
    Slice,
}

/// SVG's `preserveAspectRatio` attribute — governs how a nested viewport (or a `VectorPattern`'s
/// own `viewBox`) maps its content into its allotted rect. Top-level `VectorImage` placement into
/// an arbitrary `dest` rect is controlled separately by `VectorImageDrawOptions` (指示書§17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreserveAspectRatio {
    pub align: PreserveAspectRatioAlign,
    pub meet_or_slice: PreserveAspectRatioMeetOrSlice,
}

impl Default for PreserveAspectRatio {
    fn default() -> Self {
        Self {
            align: PreserveAspectRatioAlign::XMidYMid,
            meet_or_slice: PreserveAspectRatioMeetOrSlice::Meet,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorImageError;

impl fmt::Display for VectorImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid vector image: intrinsic size and viewBox must be finite and positive"
        )
    }
}
impl std::error::Error for VectorImageError {}

#[derive(Debug)]
struct VectorImageData {
    id: VectorImageId,
    intrinsic_size: Size,
    view_box: Rect,
    preserve_aspect_ratio: PreserveAspectRatio,
    root: VectorGroup,
}

/// A loaded, immutable vector document — the SVG analogue of [`super::image::Image`]. Cheap to
/// `Clone` (an `Arc` bump only; `path`/`filter`/`mask` graphs inside `root` are never deep-cloned)
/// and never re-parsed after construction (指示書§1.2/§25).
#[derive(Debug, Clone)]
pub struct VectorImage {
    inner: Arc<VectorImageData>,
}

impl VectorImage {
    pub fn id(&self) -> VectorImageId {
        self.inner.id
    }
    pub fn intrinsic_size(&self) -> Size {
        self.inner.intrinsic_size
    }
    pub fn view_box(&self) -> Rect {
        self.inner.view_box
    }
    pub fn preserve_aspect_ratio(&self) -> PreserveAspectRatio {
        self.inner.preserve_aspect_ratio
    }
    pub fn root(&self) -> &VectorGroup {
        &self.inner.root
    }
    /// Resource-identity comparison, not structural equality — see the module doc comment and
    /// `PartialEq`'s own impl below (指示書§5: "巨大sceneのdeep compareを暗黙に実行しない").
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Resource identity (by `VectorImageId`, assigned uniquely per construction), not a deep
/// structural comparison of the scene graph — matches `ptr_eq` since every `VectorImageData` gets
/// a fresh id.
impl PartialEq for VectorImage {
    fn eq(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
    }
}

/// Builds a [`VectorImage`]. `finish()` returns `VectorImage` directly, never `Arc<VectorImage>`
/// (指示書§5).
pub struct VectorImageBuilder {
    intrinsic_size: Size,
    view_box: Rect,
    preserve_aspect_ratio: PreserveAspectRatio,
    root: Option<VectorGroup>,
}

impl VectorImageBuilder {
    pub fn new(intrinsic_size: Size, view_box: Rect) -> Result<Self, VectorImageError> {
        let finite_positive = |w: f32, h: f32| w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0;
        if !finite_positive(intrinsic_size.width, intrinsic_size.height)
            || !finite_positive(view_box.width, view_box.height)
            || !view_box.x.is_finite()
            || !view_box.y.is_finite()
        {
            return Err(VectorImageError);
        }
        Ok(Self {
            intrinsic_size,
            view_box,
            preserve_aspect_ratio: PreserveAspectRatio::default(),
            root: None,
        })
    }

    #[must_use]
    pub fn preserve_aspect_ratio(mut self, value: PreserveAspectRatio) -> Self {
        self.preserve_aspect_ratio = value;
        self
    }

    #[must_use]
    pub fn root(mut self, root: VectorGroup) -> Self {
        self.root = Some(root);
        self
    }

    pub fn finish(self) -> Result<VectorImage, VectorImageError> {
        Ok(VectorImage {
            inner: Arc::new(VectorImageData {
                id: next_vector_image_id(),
                intrinsic_size: self.intrinsic_size,
                view_box: self.view_box,
                preserve_aspect_ratio: self.preserve_aspect_ratio,
                root: self.root.unwrap_or_default(),
            }),
        })
    }
}

/// How a [`VectorImage`] is placed into an arbitrary `dest` rect by
/// [`super::context::RenderContext::draw_vector_image`] — the SVG analogue of `ImageDrawOptions`
/// (指示書§16.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorImageDrawOptions {
    pub opacity: f32,
    pub fit: super::image::ImageFit,
    pub alignment_x: super::brush::AlignmentX,
    pub alignment_y: super::brush::AlignmentY,
    pub clip_to_dest: bool,
}

impl Default for VectorImageDrawOptions {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            fit: super::image::ImageFit::Contain,
            alignment_x: super::brush::AlignmentX::Center,
            alignment_y: super::brush::AlignmentY::Center,
            clip_to_dest: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_builder() -> VectorImageBuilder {
        VectorImageBuilder::new(
            Size {
                width: 24.0,
                height: 24.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 24.0,
                height: 24.0,
            },
        )
        .unwrap()
    }

    #[test]
    fn clone_preserves_id_and_is_ptr_eq() {
        let image = valid_builder().finish().unwrap();
        let cloned = image.clone();
        assert_eq!(image.id(), cloned.id());
        assert!(image.ptr_eq(&cloned));
        assert_eq!(image, cloned);
    }

    #[test]
    fn two_separately_built_images_have_distinct_ids() {
        let a = valid_builder().finish().unwrap();
        let b = valid_builder().finish().unwrap();
        assert_ne!(a.id(), b.id());
        assert!(!a.ptr_eq(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn builder_rejects_non_finite_or_non_positive_size() {
        let bad_size = Size {
            width: 0.0,
            height: 24.0,
        };
        let ok_view_box = Rect {
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        };
        assert!(VectorImageBuilder::new(bad_size, ok_view_box).is_err());

        let nan_view_box = Rect {
            x: 0.0,
            y: 0.0,
            width: f32::NAN,
            height: 24.0,
        };
        assert!(VectorImageBuilder::new(
            Size {
                width: 24.0,
                height: 24.0
            },
            nan_view_box
        )
        .is_err());
    }

    #[test]
    fn finish_without_root_produces_an_empty_default_group() {
        let image = valid_builder().finish().unwrap();
        assert!(image.root().children.is_empty());
    }

    #[test]
    fn draw_options_default_matches_documented_values() {
        let options = VectorImageDrawOptions::default();
        assert_eq!(options.opacity, 1.0);
        assert_eq!(options.fit, super::super::image::ImageFit::Contain);
        assert!(options.clip_to_dest);
    }
}
