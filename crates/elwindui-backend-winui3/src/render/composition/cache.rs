//! The caches that let a render pass reuse geometry, brushes, clips, and image surfaces
//! instead of rebuilding them every frame.


use super::geometry::*;
use super::node::*;
use super::*;

use crate::bindings::Microsoft::Graphics::Canvas::Geometry::{
    CanvasArcSize, CanvasFigureLoop, CanvasFilledRegionDetermination, CanvasGeometry,
    CanvasGeometryCombine,
    CanvasPathBuilder, CanvasSweepDirection,
};
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasBitmap, CanvasDevice, CanvasEdgeBehavior, CanvasImageInterpolation,
    ICanvasResourceCreator,
};
use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{CanvasImageBrush, ICanvasBrush};
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::UI::Composition::{
    Compositor, CompositionBrush, CompositionClip, CompositionColorBrush,
    CompositionColorGradientStopCollection, CompositionEllipseGeometry,
    CompositionGeometricClip, CompositionGeometry, CompositionGradientBrush,
    CompositionDrawingSurface, CompositionGraphicsDevice,
    CompositionGradientExtendMode, CompositionLineGeometry, CompositionLinearGradientBrush,
    CompositionMappingMode, CompositionPath, CompositionPathGeometry,
    CompositionRadialGradientBrush, CompositionRectangleGeometry,
    CompositionRoundedRectangleGeometry, CompositionShape, CompositionSpriteShape,
    CompositionStrokeCap, CompositionStrokeLineJoin, CompositionStretch,
    CompositionSurfaceBrush, ContainerVisual, ICompositionSurface, ShapeVisual, SpriteVisual,
    Visual,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::Hosting::ElementCompositionPreview;
use crate::bindings::Microsoft::UI::Xaml::Media::{
    LoadedImageSourceLoadCompletedEventArgs, LoadedImageSurface,
};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
use elwindui_core::base::{AffineTransform, CornerRadius, Point, Rect};
use elwindui_core::graphics::{
    Brush, BrushMappingMode, FillRule, GradientSpreadMethod, LineCap, LineJoin, PathCommand,
    StrokeStyle, Image, ImageData, Stretch, TileMode, VectorImage, VectorImageDrawOptions,
};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use windows::core::{Interface, Result, Type};
use windows::Foundation::{Size as WinSize, TypedEventHandler};
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Foundation::Rect as WinRect;
use windows::UI::Color as WinColor;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, IRandomAccessStream};
use windows_numerics::{Matrix3x2, Matrix4x4, Vector2};

/// Keeps the WinRT stream alive until WinUI has finished decoding the corresponding surface.
/// Entries are owned by the renderer, so a retained Composition node never recreates an image
/// surface during an ordinary layout pass.
pub(crate) struct LoadedSurface {
    surface: LoadedImageSurface,
    _stream: IRandomAccessStream,
    load_completed_token: Option<i64>,
}

#[derive(Default)]
pub(crate) struct ImageSurfaceCache {
    entries: HashMap<usize, LoadedSurface>,
}

impl ImageSurfaceCache {
    fn retain_for(&mut self, islands: &[DesiredCompositionIsland]) {
        let mut live = HashSet::new();
        for node in islands.iter().flat_map(|island| &island.nodes) {
            for brush in std::iter::once(node.fill.as_ref())
                .chain(std::iter::once(node.stroke.as_ref().map(|(brush, _)| brush)))
                .flatten()
            {
                if let Brush::Image(image) = brush {
                    live.insert(image.image.data() as *const ImageData as usize);
                }
            }
        }
        let removed: Vec<_> = self
            .entries
            .keys()
            .copied()
            .filter(|key| !live.contains(key))
            .collect();
        for key in removed {
            if let Some(entry) = self.entries.remove(&key) {
                if let Some(token) = entry.load_completed_token {
                    let _ = entry.surface.RemoveLoadCompleted(token);
                }
                let _ = entry.surface.Close();
            }
        }
    }

    fn clear(&mut self) {
        for (_, entry) in self.entries.drain() {
            if let Some(token) = entry.load_completed_token {
                let _ = entry.surface.RemoveLoadCompleted(token);
            }
            let _ = entry.surface.Close();
        }
    }

    fn surface_for(&mut self, image: &Image) -> std::result::Result<LoadedImageSurface, &'static str> {
        let key = image.data() as *const ImageData as usize;
        if let Some(entry) = self.entries.get(&key) {
            return Ok(entry.surface.clone());
        }

        let ImageData::Encoded { bytes, .. } = image.data() else {
            return Err("RGBA and backend image handles require CompositionDrawingSurface fallback");
        };
        let stream = InMemoryRandomAccessStream::new().map_err(|_| "image stream creation failed")?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(|_| "image writer creation failed")?;
        writer.WriteBytes(bytes).map_err(|_| "image stream write failed")?;
        writer.StoreAsync()
            .map_err(|_| "image stream flush failed")?
            .join()
            .map_err(|_| "image stream flush failed")?;
        let stream: IRandomAccessStream = stream.cast().map_err(|_| "image stream cast failed")?;
        stream.Seek(0).map_err(|_| "image stream seek failed")?;
        let surface = LoadedImageSurface::StartLoadFromStream(&stream)
            .map_err(|_| "LoadedImageSurface creation failed")?;
        let load_completed_token = if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            let handler = TypedEventHandler::new(move |
                loaded: windows::core::Ref<'_, LoadedImageSurface>,
                args: windows::core::Ref<'_, LoadedImageSourceLoadCompletedEventArgs>,
            | {
                let status = args
                    .ok()?
                    .Status()?;
                let size = loaded
                    .ok()?
                    .DecodedSize()?;
                eprintln!(
                    "elwindui-winui3: LoadedImageSurface completed: status={}, decoded_size={}x{}",
                    status.0, size.Width, size.Height,
                );
                Ok(())
            });
            Some(surface.LoadCompleted(&handler).map_err(|_| "LoadedImageSurface event registration failed")?)
        } else {
            None
        };
        self.entries.insert(
            key,
            LoadedSurface {
                surface: surface.clone(),
                _stream: stream,
                load_completed_token,
            },
        );
        Ok(surface)
    }
}

#[cfg_attr(rust_analyzer, allow(dead_code))]
pub(crate) enum GeometryState {
    Rectangle(CompositionRectangleGeometry),
    RoundedRectangle(CompositionRoundedRectangleGeometry),
    Ellipse(CompositionEllipseGeometry),
    Line(CompositionLineGeometry),
    Path {
        geometry: CompositionPathGeometry,
        _path: CompositionPath,
        _canvas_geometry: CanvasGeometry,
    },
}

/// Creates the Composition path objects from one retained Win2D path geometry.
/// Keeping this in a `windows::core::Result` helper preserves the concrete
/// WinRT error type across the QueryInterface and factory calls.
pub(crate) fn composition_path_geometry(
    compositor: &Compositor,
    canvas_geometry: &CanvasGeometry,
) -> Result<(CompositionPath, CompositionPathGeometry)> {
    let source: windows::Graphics::IGeometrySource2D = canvas_geometry.clone().cast()?;
    let path: CompositionPath = CompositionPath::Create(&source)?;
    let geometry: CompositionPathGeometry = compositor.CreatePathGeometryWithPath(&path)?;
    Ok((path, geometry))
}

pub(crate) struct ClipState {
    clip: CompositionGeometricClip,
    _geometry: CompositionPathGeometry,
    _path: CompositionPath,
    _canvas_geometry: CanvasGeometry,
}

impl ClipState {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        specs: &[CompositionClipSpec],
        island_bounds: Rect,
    ) -> std::result::Result<Self, &'static str> {
        let creator: ICanvasResourceCreator = canvas_device
            .clone()
            .cast()
            .map_err(|_| "CanvasDevice is not an ICanvasResourceCreator")?;
        let universe = CanvasGeometry::CreateRectangleAtCoords(
            &creator,
            -16_777_216.0,
            -16_777_216.0,
            33_554_432.0,
            33_554_432.0,
        )
        .map_err(|_| "clip universe geometry creation failed")?;
        let mut combined = universe;
        for spec in specs {
            let geometry = canvas_clip_geometry(&creator, spec)?;
            combined = combined
                .CombineWith(
                    &geometry,
                    matrix(spec.transform()),
                    CanvasGeometryCombine::Intersect,
                )
                .map_err(|_| "clip geometry intersection failed")?;
        }
        let (path, geometry) = composition_path_geometry(compositor, &combined)
            .map_err(|_| "clip CompositionPath creation failed")?;
        let base: CompositionGeometry = geometry
            .clone()
            .cast()
            .map_err(|_| "clip path geometry cast failed")?;
        let clip = compositor
            .CreateGeometricClipWithGeometry(&base)
            .map_err(|_| "CreateGeometricClipWithGeometry failed")?;
        clip.SetTransformMatrix(Matrix3x2 {
            M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0,
            M31: -island_bounds.x, M32: -island_bounds.y,
        })
            .map_err(|_| "clip transform failed")?;
        Ok(Self {
            clip,
            _geometry: geometry,
            _path: path,
            _canvas_geometry: combined,
        })
    }

    fn as_clip(&self) -> Result<CompositionClip> {
        self.clip.clone().cast()
    }
}

pub(crate) fn canvas_clip_geometry(
    creator: &ICanvasResourceCreator,
    spec: &CompositionClipSpec,
) -> std::result::Result<CanvasGeometry, &'static str> {
    match spec {
        CompositionClipSpec::Rect { rect, .. } => CanvasGeometry::CreateRectangleAtCoords(
            creator, rect.x, rect.y, rect.width, rect.height,
        ).map_err(|_| "clip rectangle geometry creation failed"),
        CompositionClipSpec::RoundedRect { rect, radii, .. } => {
            let radius = uniform_radius(*radii)
                .ok_or("per-corner rounded clip requires surface fallback")?;
            CanvasGeometry::CreateRoundedRectangleAtCoords(
                creator, rect.x, rect.y, rect.width, rect.height, radius, radius,
            ).map_err(|_| "clip rounded geometry creation failed")
        }
        CompositionClipSpec::Path { commands, rule, origin, .. } => {
            create_canvas_path(creator, commands, *origin, *rule)
        }
    }
}

impl GeometryState {
    fn update(&self, primitive: &CompositionPrimitive) -> Result<()> {
        match (self, primitive) {
            (Self::Rectangle(geometry), CompositionPrimitive::Rectangle { rect }) => {
                geometry.SetOffset(vector2(rect.x, rect.y))?;
                geometry.SetSize(vector2(rect.width, rect.height))
            }
            (
                Self::RoundedRectangle(geometry),
                CompositionPrimitive::RoundedRectangle { rect, radii },
            ) => {
                let radius = uniform_radius(*radii).ok_or_else(|| {
                    windows::core::Error::new(
                        windows::core::HRESULT(0x80004001_u32 as i32),
                        "per-corner rounded rectangles require surface fallback",
                    )
                })?;
                geometry.SetOffset(vector2(rect.x, rect.y))?;
                geometry.SetSize(vector2(rect.width, rect.height))?;
                geometry.SetCornerRadius(vector2(radius, radius))
            }
            (Self::Ellipse(geometry), CompositionPrimitive::Ellipse { rect }) => {
                geometry.SetCenter(vector2(
                    rect.x + rect.width * 0.5,
                    rect.y + rect.height * 0.5,
                ))?;
                geometry.SetRadius(vector2(rect.width * 0.5, rect.height * 0.5))
            }
            (Self::Line(geometry), CompositionPrimitive::Line { from, to }) => {
                geometry.SetStart(vector2(from.x, from.y))?;
                geometry.SetEnd(vector2(to.x, to.y))
            }
            (Self::Path { .. }, CompositionPrimitive::Path { .. }) => Ok(()),
            _ => Err(windows::core::Error::new(
                windows::core::HRESULT(0x80070057_u32 as i32),
                "Composition geometry kind changed",
            )),
        }
    }

    #[cfg(not(rust_analyzer))]
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        primitive: &CompositionPrimitive,
    ) -> std::result::Result<Self, &'static str> {
        match primitive {
            CompositionPrimitive::Rectangle { rect } => {
                let geometry = compositor
                    .CreateRectangleGeometry()
                    .map_err(|_| "CreateRectangleGeometry failed")?;
                geometry
                    .SetOffset(vector2(rect.x, rect.y))
                    .map_err(|_| "rectangle offset failed")?;
                geometry
                    .SetSize(vector2(rect.width, rect.height))
                    .map_err(|_| "rectangle size failed")?;
                Ok(Self::Rectangle(geometry))
            }
            CompositionPrimitive::RoundedRectangle { rect, radii } => {
                let radius = uniform_radius(*radii)
                    .ok_or("per-corner rounded rectangles require surface fallback")?;
                let geometry = compositor
                    .CreateRoundedRectangleGeometry()
                    .map_err(|_| "CreateRoundedRectangleGeometry failed")?;
                geometry
                    .SetOffset(vector2(rect.x, rect.y))
                    .map_err(|_| "rounded rectangle offset failed")?;
                geometry
                    .SetSize(vector2(rect.width, rect.height))
                    .map_err(|_| "rounded rectangle size failed")?;
                geometry
                    .SetCornerRadius(vector2(radius, radius))
                    .map_err(|_| "rounded rectangle radius failed")?;
                Ok(Self::RoundedRectangle(geometry))
            }
            CompositionPrimitive::Ellipse { rect } => {
                let geometry = compositor
                    .CreateEllipseGeometry()
                    .map_err(|_| "CreateEllipseGeometry failed")?;
                geometry
                    .SetCenter(vector2(
                        rect.x + rect.width * 0.5,
                        rect.y + rect.height * 0.5,
                    ))
                    .map_err(|_| "ellipse center failed")?;
                geometry
                    .SetRadius(vector2(rect.width * 0.5, rect.height * 0.5))
                    .map_err(|_| "ellipse radius failed")?;
                Ok(Self::Ellipse(geometry))
            }
            CompositionPrimitive::Line { from, to } => {
                let geometry = compositor
                    .CreateLineGeometry()
                    .map_err(|_| "CreateLineGeometry failed")?;
                geometry
                    .SetStart(vector2(from.x, from.y))
                    .map_err(|_| "line start failed")?;
                geometry
                    .SetEnd(vector2(to.x, to.y))
                    .map_err(|_| "line end failed")?;
                Ok(Self::Line(geometry))
            }
            CompositionPrimitive::Path {
                commands,
                rule,
                origin,
            } => {
                let creator: ICanvasResourceCreator = canvas_device
                    .clone()
                    .cast()
                    .map_err(|_| "CanvasDevice is not an ICanvasResourceCreator")?;
                let canvas_geometry: CanvasGeometry = match create_canvas_path(
                    &creator,
                    commands,
                    *origin,
                    *rule,
                ) {
                    Ok(geometry) => geometry,
                    Err(reason) => return Err(reason),
                };
                let (path, geometry): (CompositionPath, CompositionPathGeometry) =
                    match composition_path_geometry(compositor, &canvas_geometry) {
                        Ok(result) => result,
                        Err(_) => return Err("CompositionPath creation failed"),
                    };
                Ok(Self::Path {
                    geometry,
                    _path: path,
                    _canvas_geometry: canvas_geometry,
                })
            }
            CompositionPrimitive::VectorImage { .. } => Err("vector images require a CompositionDrawingSurface"),
        }
    }

    // rust-analyzer 1.97 cannot solve several generic `windows-core::Param`
    // calls in the generated WinRT bindings used above, even though rustc
    // accepts the same code. The IDE needs only the method signature to type
    // the retained renderer; real builds always compile the implementation
    // above. This mirrors the workspace's established rust_analyzer shadows.
    #[cfg(rust_analyzer)]
    fn create(
        _compositor: &Compositor,
        _canvas_device: &CanvasDevice,
        _primitive: &CompositionPrimitive,
    ) -> std::result::Result<Self, &'static str> {
        Err("GeometryState construction is evaluated only by the real WinRT build")
    }

    fn as_geometry(&self) -> Result<CompositionGeometry> {
        match self {
            Self::Rectangle(value) => value.clone().cast(),
            Self::RoundedRectangle(value) => value.clone().cast(),
            Self::Ellipse(value) => value.clone().cast(),
            Self::Line(value) => value.clone().cast(),
            Self::Path { geometry, .. } => geometry.clone().cast(),
        }
    }
}

pub(crate) enum BrushState {
    Solid(CompositionColorBrush),
    Linear(CompositionLinearGradientBrush),
    Radial(CompositionRadialGradientBrush),
}

impl BrushState {
    fn create(
        compositor: &Compositor,
        brush: &Brush,
        opacity: f32,
    ) -> Result<Self> {
        match brush {
            Brush::Solid(color) => Ok(Self::Solid(
                compositor.CreateColorBrushWithColor(color_with_opacity(*color, opacity))?,
            )),
            Brush::LinearGradient(gradient) => {
                let native = compositor.CreateLinearGradientBrush()?;
                let common: CompositionGradientBrush = native.clone().cast()?;
                apply_gradient_common(
                    compositor,
                    &common,
                    &gradient.stops,
                    gradient.spread,
                    gradient.mapping,
                    gradient.transform,
                    opacity * gradient.opacity,
                )?;
                native.SetStartPoint(vector2(gradient.start.x, gradient.start.y))?;
                native.SetEndPoint(vector2(gradient.end.x, gradient.end.y))?;
                Ok(Self::Linear(native))
            }
            Brush::RadialGradient(gradient) => {
                let native = compositor.CreateRadialGradientBrush()?;
                let common: CompositionGradientBrush = native.clone().cast()?;
                apply_gradient_common(
                    compositor,
                    &common,
                    &gradient.stops,
                    gradient.spread,
                    gradient.mapping,
                    gradient.transform,
                    opacity * gradient.opacity,
                )?;
                native.SetEllipseCenter(vector2(gradient.center.x, gradient.center.y))?;
                native.SetGradientOriginOffset(vector2(
                    gradient.gradient_origin.x - gradient.center.x,
                    gradient.gradient_origin.y - gradient.center.y,
                ))?;
                native.SetEllipseRadius(vector2(gradient.radius_x, gradient.radius_y))?;
                Ok(Self::Radial(native))
            }
            Brush::Image(_) => Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004001_u32 as i32),
                "image brush requires SpriteVisual or CompositionDrawingSurface fallback",
            )),
        }
    }

    fn as_brush(&self) -> Result<CompositionBrush> {
        match self {
            Self::Solid(value) => value.clone().cast(),
            Self::Linear(value) => value.clone().cast(),
            Self::Radial(value) => value.clone().cast(),
        }
    }
}

pub(crate) fn apply_gradient_common(
    compositor: &Compositor,
    brush: &CompositionGradientBrush,
    stops: &[elwindui_core::graphics::GradientStop],
    spread: GradientSpreadMethod,
    mapping: BrushMappingMode,
    transform: AffineTransform,
    opacity: f32,
) -> Result<()> {
    brush.SetExtendMode(match spread {
        GradientSpreadMethod::Pad => CompositionGradientExtendMode::Clamp,
        GradientSpreadMethod::Repeat => CompositionGradientExtendMode::Wrap,
        GradientSpreadMethod::Reflect => CompositionGradientExtendMode::Mirror,
    })?;
    brush.SetMappingMode(match mapping {
        BrushMappingMode::Absolute => CompositionMappingMode::Absolute,
        BrushMappingMode::RelativeToBounds => CompositionMappingMode::Relative,
    })?;
    brush.SetTransformMatrix(matrix(transform))?;
    let collection: CompositionColorGradientStopCollection = brush.ColorStops()?;
    collection.Clear()?;
    for stop in stops {
        let native = compositor
            .CreateColorGradientStopWithOffsetAndColor(
                stop.offset,
                color_with_opacity(stop.color, opacity),
            )?;
        collection.Append(&native)?;
    }
    Ok(())
}

pub(crate) fn apply_stroke(shape: &CompositionSpriteShape, stroke: &StrokeStyle) -> Result<()> {
    shape.SetStrokeThickness(stroke.width)?;
    shape.SetStrokeStartCap(stroke_cap(stroke.start_cap))?;
    shape.SetStrokeEndCap(stroke_cap(stroke.end_cap))?;
    shape.SetStrokeDashCap(stroke_cap(stroke.dash_cap))?;
    shape.SetStrokeLineJoin(match stroke.line_join {
        LineJoin::Miter => CompositionStrokeLineJoin::Miter,
        LineJoin::Round => CompositionStrokeLineJoin::Round,
        LineJoin::Bevel => CompositionStrokeLineJoin::Bevel,
    })?;
    shape.SetStrokeMiterLimit(stroke.miter_limit)?;
    shape.SetStrokeDashOffset(stroke.dash_offset)?;
    let dashes = shape.StrokeDashArray()?;
    dashes.Clear()?;
    for value in stroke.dash_pattern.iter().copied() {
        dashes.Append(value)?;
    }
    Ok(())
}

pub(crate) fn create_canvas_path(
    creator: &ICanvasResourceCreator,
    commands: &[PathCommand],
    origin: Point,
    rule: FillRule,
) -> std::result::Result<CanvasGeometry, &'static str> {
    use elwindui_core::graphics::SweepDirection;

    let builder = CanvasPathBuilder::Create(creator).map_err(|_| "CanvasPathBuilder::Create failed")?;
    builder
        .SetFilledRegionDetermination(match rule {
            FillRule::EvenOdd => CanvasFilledRegionDetermination::Alternate,
            FillRule::NonZero => CanvasFilledRegionDetermination::Winding,
        })
        .map_err(|_| "path fill rule failed")?;
    let mut open = false;
    for command in commands {
        match command {
            PathCommand::MoveTo(point) => {
                if open {
                    builder
                        .EndFigure(CanvasFigureLoop::Open)
                        .map_err(|_| "end open path failed")?;
                }
                builder
                    .BeginFigureAtCoords(point.x + origin.x, point.y + origin.y)
                    .map_err(|_| "begin path failed")?;
                open = true;
            }
            PathCommand::LineTo(point) if open => builder
                .AddLineWithCoords(point.x + origin.x, point.y + origin.y)
                .map_err(|_| "line path segment failed")?,
            PathCommand::QuadTo { control, to } if open => builder
                .AddQuadraticBezier(
                    vector2(control.x + origin.x, control.y + origin.y),
                    vector2(to.x + origin.x, to.y + origin.y),
                )
                .map_err(|_| "quadratic path segment failed")?,
            PathCommand::CubicTo {
                control1,
                control2,
                to,
            } if open => builder
                .AddCubicBezier(
                    vector2(control1.x + origin.x, control1.y + origin.y),
                    vector2(control2.x + origin.x, control2.y + origin.y),
                    vector2(to.x + origin.x, to.y + origin.y),
                )
                .map_err(|_| "cubic path segment failed")?,
            PathCommand::ArcTo(arc) if open => builder
                .AddArcToPoint(
                    vector2(arc.to.x + origin.x, arc.to.y + origin.y),
                    arc.radii.width,
                    arc.radii.height,
                    arc.x_axis_rotation,
                    match arc.sweep {
                        SweepDirection::Clockwise => CanvasSweepDirection::Clockwise,
                        SweepDirection::CounterClockwise => {
                            CanvasSweepDirection::CounterClockwise
                        }
                    },
                    if arc.large_arc {
                        CanvasArcSize::Large
                    } else {
                        CanvasArcSize::Small
                    },
                )
                .map_err(|_| "arc path segment failed")?,
            PathCommand::Close if open => {
                builder
                    .EndFigure(CanvasFigureLoop::Closed)
                    .map_err(|_| "close path failed")?;
                open = false;
            }
            _ => return Err("path segment appears before MoveTo"),
        }
    }
    if open {
        builder
            .EndFigure(CanvasFigureLoop::Open)
            .map_err(|_| "end path failed")?;
    }
    CanvasGeometry::CreatePath(&builder).map_err(|_| "CanvasGeometry::CreatePath failed")
}

pub(crate) fn uniform_radius(radii: CornerRadius) -> Option<f32> {
    let radius = radii.top_left;
    let values = [
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ];
    values
        .into_iter()
        .all(|value| (value - radius).abs() <= f32::EPSILON)
        .then_some(radius)
}

pub(crate) fn stroke_cap(cap: LineCap) -> CompositionStrokeCap {
    match cap {
        LineCap::Butt => CompositionStrokeCap::Flat,
        LineCap::Round => CompositionStrokeCap::Round,
        LineCap::Square => CompositionStrokeCap::Square,
    }
}

pub(crate) fn color_with_opacity(
    color: elwindui_core::graphics::Color,
    opacity: f32,
) -> windows::UI::Color {
    windows::UI::Color {
        A: ((color.a as f32) * opacity.clamp(0.0, 1.0)).round() as u8,
        R: color.r,
        G: color.g,
        B: color.b,
    }
}
