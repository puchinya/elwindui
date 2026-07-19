use super::brush::Brush;
use super::color::Color;
use super::image::{Image, ImageDrawOptions};
use super::path::{FillRule, Path};
use super::stroke::StrokeStyle;
use super::vector_image::{VectorImage, VectorImageDrawOptions};
use crate::base::{AffineTransform, CornerRadius, Rect};
use std::any::Any;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}
impl Default for TextAlignment {
    fn default() -> Self {
        Self::Left
    }
}

/// Backend-independent font placeholder — font selection/shaping/measurement is out of scope for
/// this graphics API revision (painter design doc §1/§22); `Font` stays a zero-sized marker until
/// that work happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Font;

/// A clip region pushed via [`super::context::RenderContext::with_clip`]. `Path` is
/// `Arc`-backed internally, so cloning a `Path` variant is cheap — no indirection trickery needed
/// to keep this type light to carry around on the (rarely deep) clip/transform/opacity stack.
#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    Rect(Rect),
    RoundedRect { rect: Rect, radii: CornerRadius },
    Path { path: Path, rule: FillRule },
}

/// A backend-independent command owned by one Visual. Coordinates are local to its RenderGroup.
/// This intentionally mirrors [`super::context::RenderContext`]'s own primitive set one-to-one —
/// see painter design doc §11.
#[derive(Clone)]
pub enum RenderCommand {
    FillRect {
        rect: Rect,
        brush: Brush,
    },
    StrokeRect {
        rect: Rect,
        brush: Brush,
        stroke: StrokeStyle,
    },
    FillRoundedRect {
        rect: Rect,
        radii: CornerRadius,
        brush: Brush,
    },
    StrokeRoundedRect {
        rect: Rect,
        radii: CornerRadius,
        brush: Brush,
        stroke: StrokeStyle,
    },
    FillEllipse {
        rect: Rect,
        brush: Brush,
    },
    StrokeEllipse {
        rect: Rect,
        brush: Brush,
        stroke: StrokeStyle,
    },
    DrawLine {
        from: crate::base::Point,
        to: crate::base::Point,
        brush: Brush,
        stroke: StrokeStyle,
    },
    FillPath {
        path: Path,
        brush: Brush,
        rule: FillRule,
    },
    StrokePath {
        path: Path,
        brush: Brush,
        stroke: StrokeStyle,
    },
    DrawImage {
        image: Image,
        dest: Rect,
        source: Option<Rect>,
        options: ImageDrawOptions,
    },
    /// `image`'s internal `Arc` makes this variant's own `Clone` cheap — see
    /// `VectorImage`'s own doc comment (SVG読み込み・ベクター描画対応 実装指示書§1.3).
    DrawVectorImage {
        image: VectorImage,
        dest: Rect,
        source: Option<Rect>,
        options: VectorImageDrawOptions,
    },
    Text {
        content: String,
        rect: Rect,
        font: Font,
        color: Option<Color>,
        alignment: TextAlignment,
    },
    PushClip {
        clip: Clip,
    },
    PopClip,
    PushTransform {
        transform: AffineTransform,
    },
    PopTransform,
    PushOpacity {
        opacity: f32,
    },
    PopOpacity,
    /// The handle stays type-erased until a backend replays this command. `owner_id` resolves to
    /// the owning UIElement through RenderTree's weak index.
    NativeControl {
        owner_id: u64,
        handle: Rc<dyn Any>,
        rect: Rect,
    },
}
