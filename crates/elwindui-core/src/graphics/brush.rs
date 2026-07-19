use super::color::Color;
use super::image::Image;
use crate::base::{AffineTransform, Point};
use std::sync::Arc;

/// What to paint a fill/stroke with — WinUI3's `SolidColorBrush`/`LinearGradientBrush`/
/// `RadialGradientBrush`/`ImageBrush` unified under one enum (§14's "enums are the only
/// value-set mechanism" rule).
#[derive(Debug, Clone, PartialEq)]
pub enum Brush {
    Solid(Color),
    LinearGradient(LinearGradientBrush),
    RadialGradient(RadialGradientBrush),
    Image(ImageBrush),
}

impl From<Color> for Brush {
    fn from(color: Color) -> Self {
        Brush::Solid(color)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientError;

impl std::fmt::Display for GradientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gradient stop offset must be within [0.0, 1.0]")
    }
}
impl std::error::Error for GradientError {}

impl GradientStop {
    /// `offset` is normalized to `[0.0, 1.0]`; out-of-range or non-finite values are rejected
    /// rather than silently clamped, so backend differences never mask an authoring mistake
    /// (painter design doc §6).
    pub fn new(offset: f32, color: Color) -> Result<Self, GradientError> {
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
            return Err(GradientError);
        }
        Ok(Self { offset, color })
    }
}

/// Validates that stops are well-formed and returns them sorted by offset — gradient brush
/// constructors call this so an ill-formed `Vec<GradientStop>` can never reach a backend.
fn validated_stops(stops: Vec<GradientStop>) -> Result<Arc<[GradientStop]>, GradientError> {
    if stops.is_empty() {
        return Err(GradientError);
    }
    let mut stops = stops;
    stops.sort_by(|a, b| {
        a.offset
            .partial_cmp(&b.offset)
            .expect("finite, checked at construction")
    });
    Ok(stops.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientSpreadMethod {
    Pad,
    Reflect,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushMappingMode {
    Absolute,
    RelativeToBounds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradientBrush {
    pub start: Point,
    pub end: Point,
    pub stops: Arc<[GradientStop]>,
    pub spread: GradientSpreadMethod,
    pub mapping: BrushMappingMode,
    pub transform: AffineTransform,
    pub opacity: f32,
}

impl LinearGradientBrush {
    pub fn new(start: Point, end: Point, stops: Vec<GradientStop>) -> Result<Self, GradientError> {
        Ok(Self {
            start,
            end,
            stops: validated_stops(stops)?,
            spread: GradientSpreadMethod::Pad,
            mapping: BrushMappingMode::RelativeToBounds,
            transform: AffineTransform::IDENTITY,
            opacity: 1.0,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradientBrush {
    pub center: Point,
    pub gradient_origin: Point,
    pub radius_x: f32,
    pub radius_y: f32,
    pub stops: Arc<[GradientStop]>,
    pub spread: GradientSpreadMethod,
    pub mapping: BrushMappingMode,
    pub transform: AffineTransform,
    pub opacity: f32,
}

impl RadialGradientBrush {
    pub fn new(
        center: Point,
        radius_x: f32,
        radius_y: f32,
        stops: Vec<GradientStop>,
    ) -> Result<Self, GradientError> {
        Ok(Self {
            center,
            gradient_origin: center,
            radius_x,
            radius_y,
            stops: validated_stops(stops)?,
            spread: GradientSpreadMethod::Pad,
            mapping: BrushMappingMode::RelativeToBounds,
            transform: AffineTransform::IDENTITY,
            opacity: 1.0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stretch {
    None,
    Fill,
    Uniform,
    UniformToFill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentX {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentY {
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileMode {
    None,
    Tile,
    FlipX,
    FlipY,
    FlipXY,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageBrush {
    pub image: Image,
    pub source_rect: Option<crate::base::Rect>,
    pub stretch: Stretch,
    pub alignment_x: AlignmentX,
    pub alignment_y: AlignmentY,
    pub tile_mode: TileMode,
    pub opacity: f32,
    pub transform: AffineTransform,
}

impl ImageBrush {
    pub fn new(image: Image) -> Self {
        Self {
            image,
            source_rect: None,
            stretch: Stretch::Uniform,
            alignment_x: AlignmentX::Center,
            alignment_y: AlignmentY::Center,
            tile_mode: TileMode::None,
            opacity: 1.0,
            transform: AffineTransform::IDENTITY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_stop_rejects_out_of_range_offset() {
        assert!(GradientStop::new(1.5, Color::black()).is_err());
        assert!(GradientStop::new(-0.1, Color::black()).is_err());
        assert!(GradientStop::new(f32::NAN, Color::black()).is_err());
    }

    #[test]
    fn linear_gradient_sorts_stops_by_offset() {
        let brush = LinearGradientBrush::new(
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 0.0 },
            vec![
                GradientStop::new(1.0, Color::white()).unwrap(),
                GradientStop::new(0.0, Color::black()).unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(brush.stops[0].offset, 0.0);
        assert_eq!(brush.stops[1].offset, 1.0);
    }

    #[test]
    fn gradient_brush_rejects_empty_stops() {
        assert!(
            LinearGradientBrush::new(Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 0.0 }, vec![])
                .is_err()
        );
    }
}
