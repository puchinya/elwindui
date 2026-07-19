//! Backend-independent drawing API — value types (color/brush/stroke/path/image), the
//! `RenderCommand`/`RenderContext` recording layer `UIElement::render` implementations write
//! against, and the retained `RenderGroup`/`RenderTree` visual tree each backend replays.
//! See `docs/elwindui_gui_framework_design.md` §5.7 for how `builtin::Canvas`'s (currently
//! unimplemented) `Painter` abstraction will eventually sit on top of `RenderContext`.

mod brush;
mod color;
mod command;
mod context;
mod image;
mod path;
mod path_combine;
mod render_tree;
mod stroke;
mod vector_filter;
mod vector_image;
mod vector_scene;

pub use brush::{
    AlignmentX, AlignmentY, Brush, BrushMappingMode, GradientError, GradientSpreadMethod,
    GradientStop, ImageBrush, LinearGradientBrush, RadialGradientBrush, Stretch, TileMode,
};
pub use color::{Color, ParseColorError};
pub use command::{Clip, Font, RenderCommand, TextAlignment};
pub use context::{Fill, RenderContext, SaveGuard, Stroke};
pub use image::{
    AlphaMode, BackendImageHandle, Image, ImageData, ImageDrawOptions, ImageError, ImageFit,
    ImageFormat, ImageSampling,
};
pub use path::{
    ArcSegment, FillRule, GeometryCombineMode, GeometryError, Path, PathBuilder, PathCommand,
    PathError, SweepDirection,
};
pub use render_tree::{RenderGroup, RenderTree};
pub use stroke::{LineCap, LineJoin, StrokeError, StrokeStyle};
pub use vector_filter::{
    VectorBlendFilter, VectorColorChannel, VectorColorInterpolation, VectorColorMatrixFilter,
    VectorColorMatrixKind, VectorComponentTransferFilter, VectorCompositeFilter,
    VectorCompositeOperator, VectorConvolveMatrixFilter, VectorDiffuseLightingFilter,
    VectorDisplacementMapFilter, VectorDropShadowFilter, VectorEdgeMode, VectorFilter,
    VectorFilterImage, VectorFilterInput, VectorFilterPrimitive, VectorFilterPrimitiveNode,
    VectorFilterResultId, VectorFloodFilter, VectorGaussianBlurFilter, VectorLightSource,
    VectorMergeFilter, VectorMorphologyFilter, VectorMorphologyOperator, VectorOffsetFilter,
    VectorSpecularLightingFilter, VectorTileFilter, VectorTransferFunction,
    VectorTurbulenceFilter, VectorTurbulenceKind,
};
pub use vector_image::{
    ImageSource, PreserveAspectRatio, PreserveAspectRatioAlign, PreserveAspectRatioMeetOrSlice,
    VectorImage, VectorImageBuilder, VectorImageDrawOptions, VectorImageError, VectorImageId,
    VectorRasterizeMode,
};
pub use vector_scene::{
    VectorBlendMode, VectorClipPath, VectorFill, VectorGroup, VectorMask, VectorMaskType,
    VectorNode, VectorPaint, VectorPaintOrder, VectorPathNode, VectorPattern, VectorRasterNode,
    VectorShapeRendering, VectorStroke,
};
