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
