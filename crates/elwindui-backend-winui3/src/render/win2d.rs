//! The Win2D command list primitives a `RenderCommand` is translated into: paths, clips,
//! brushes, gradients, strokes, and bitmaps. Knows nothing about `UIElement` or any `Inner*`
//! control.

use crate::bindings;
use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{
    CanvasGradientStop, CanvasImageBrush, CanvasLinearGradientBrush, CanvasRadialGradientBrush,
    CanvasSolidColorBrush, ICanvasBrush,
};
use crate::bindings::Microsoft::Graphics::Canvas::Geometry::{
    CanvasArcSize, CanvasFigureLoop, CanvasFilledRegionDetermination, CanvasGeometry,
    CanvasPathBuilder, CanvasSweepDirection,
};
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasBitmap, CanvasBlend, CanvasEdgeBehavior, CanvasImageInterpolation, ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use windows::Foundation::PropertyValue;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Storage::Streams::{DataWriter, IRandomAccessStream, InMemoryRandomAccessStream};
use windows::UI::Color;
use windows::core::{Interface, Result};

#[derive(Clone)]
pub(crate) enum Win2dPrimitive {
    SetTransform {
        m11: f32,
        m12: f32,
        m21: f32,
        m22: f32,
        dx: f32,
        dy: f32,
    },
    SetOpacity(f32),
    SetAntialiasing(bool),
    SetBlend(CanvasBlend),
    FillPath {
        commands: Vec<elwindui_core::graphics::PathCommand>,
        x: f32,
        y: f32,
        brush: elwindui_core::graphics::Brush,
        rule: elwindui_core::graphics::FillRule,
    },
    StrokePath {
        commands: Vec<elwindui_core::graphics::PathCommand>,
        x: f32,
        y: f32,
        brush: elwindui_core::graphics::Brush,
        stroke: elwindui_core::graphics::StrokeStyle,
    },
    PushClip {
        clip: elwindui_core::graphics::Clip,
        x: f32,
        y: f32,
    },
    PushPathStrokeClip {
        commands: Vec<elwindui_core::graphics::PathCommand>,
        x: f32,
        y: f32,
        width_px: f32,
    },
    PopClip,
    /// Keeps group opacity as an off-screen Win2D composition operation. Applying this opacity
    /// to each child separately is observably different when children overlap.
    PushOpacityLayer(f32),
    PopOpacityLayer,
    DrawImage {
        image: elwindui_core::graphics::Image,
        dest: elwindui_core::base::Rect,
        source: Option<elwindui_core::base::Rect>,
        options: elwindui_core::graphics::ImageDrawOptions,
        x: f32,
        y: f32,
    },
}

pub(crate) fn win2d_path_geometry(
    creator: &ICanvasResourceCreator,
    commands: &[elwindui_core::graphics::PathCommand],
    origin_x: f32,
    origin_y: f32,
    rule: elwindui_core::graphics::FillRule,
) -> Result<CanvasGeometry> {
    use elwindui_core::graphics::{PathCommand, SweepDirection};

    let builder = CanvasPathBuilder::Create(creator)?;
    let determination = match rule {
        elwindui_core::graphics::FillRule::EvenOdd => CanvasFilledRegionDetermination::Alternate,
        elwindui_core::graphics::FillRule::NonZero => CanvasFilledRegionDetermination::Winding,
    };
    builder.SetFilledRegionDetermination(determination)?;
    let mut open = false;
    for command in commands {
        match command {
            PathCommand::MoveTo(point) => {
                if open {
                    builder.EndFigure(CanvasFigureLoop::Open)?;
                }
                builder.BeginFigureAtCoords(point.x + origin_x, point.y + origin_y)?;
                open = true;
            }
            PathCommand::LineTo(point) if open => {
                builder.AddLineWithCoords(point.x + origin_x, point.y + origin_y)?;
            }
            PathCommand::QuadTo { control, to } if open => {
                builder.AddQuadraticBezier(
                    windows_numerics::Vector2 {
                        X: control.x + origin_x,
                        Y: control.y + origin_y,
                    },
                    windows_numerics::Vector2 {
                        X: to.x + origin_x,
                        Y: to.y + origin_y,
                    },
                )?;
            }
            PathCommand::CubicTo {
                control1,
                control2,
                to,
            } if open => {
                builder.AddCubicBezier(
                    windows_numerics::Vector2 {
                        X: control1.x + origin_x,
                        Y: control1.y + origin_y,
                    },
                    windows_numerics::Vector2 {
                        X: control2.x + origin_x,
                        Y: control2.y + origin_y,
                    },
                    windows_numerics::Vector2 {
                        X: to.x + origin_x,
                        Y: to.y + origin_y,
                    },
                )?;
            }
            PathCommand::ArcTo(arc) if open => {
                builder.AddArcToPoint(
                    windows_numerics::Vector2 {
                        X: arc.to.x + origin_x,
                        Y: arc.to.y + origin_y,
                    },
                    arc.radii.width,
                    arc.radii.height,
                    arc.x_axis_rotation,
                    match arc.sweep {
                        SweepDirection::Clockwise => CanvasSweepDirection::Clockwise,
                        SweepDirection::CounterClockwise => CanvasSweepDirection::CounterClockwise,
                    },
                    if arc.large_arc {
                        CanvasArcSize::Large
                    } else {
                        CanvasArcSize::Small
                    },
                )?;
            }
            PathCommand::Close if open => {
                builder.EndFigure(CanvasFigureLoop::Closed)?;
                open = false;
            }
            _ => {}
        }
    }
    if open {
        builder.EndFigure(CanvasFigureLoop::Open)?;
    }
    CanvasGeometry::CreatePath(&builder)
}

pub(crate) fn win2d_clip_geometry(
    creator: &ICanvasResourceCreator,
    clip: &elwindui_core::graphics::Clip,
    origin_x: f32,
    origin_y: f32,
) -> Result<CanvasGeometry> {
    match clip {
        elwindui_core::graphics::Clip::Rect(rect) => CanvasGeometry::CreateRectangleAtCoords(
            creator,
            rect.x + origin_x,
            rect.y + origin_y,
            rect.width,
            rect.height,
        ),
        elwindui_core::graphics::Clip::RoundedRect { rect, radii } => {
            let radius =
                (radii.top_left + radii.top_right + radii.bottom_right + radii.bottom_left) / 4.0;
            CanvasGeometry::CreateRoundedRectangleAtCoords(
                creator,
                rect.x + origin_x,
                rect.y + origin_y,
                rect.width,
                rect.height,
                radius,
                radius,
            )
        }
        elwindui_core::graphics::Clip::Path { path, rule } => {
            win2d_path_geometry(creator, path.commands(), origin_x, origin_y, *rule)
        }
    }
}

pub(crate) fn win2d_brush_matrix(
    transform: elwindui_core::base::AffineTransform,
) -> windows_numerics::Matrix3x2 {
    windows_numerics::Matrix3x2 {
        M11: transform.m11,
        M12: transform.m12,
        M21: transform.m21,
        M22: transform.m22,
        M31: transform.dx,
        M32: transform.dy,
    }
}

pub(crate) fn win2d_gradient_point(
    point: elwindui_core::base::Point,
    mapping: elwindui_core::graphics::BrushMappingMode,
    bounds: elwindui_core::base::Rect,
) -> windows_numerics::Vector2 {
    use elwindui_core::graphics::BrushMappingMode;
    match mapping {
        BrushMappingMode::RelativeToBounds => windows_numerics::Vector2 {
            X: bounds.x + point.x * bounds.width,
            Y: bounds.y + point.y * bounds.height,
        },
        BrushMappingMode::Absolute => windows_numerics::Vector2 {
            X: point.x,
            Y: point.y,
        },
    }
}

pub(crate) fn win2d_gradient_radius(
    radius: f32,
    mapping: elwindui_core::graphics::BrushMappingMode,
    extent: f32,
) -> f32 {
    match mapping {
        elwindui_core::graphics::BrushMappingMode::RelativeToBounds => radius * extent,
        elwindui_core::graphics::BrushMappingMode::Absolute => radius,
    }
}

pub(crate) fn win2d_gradient_stops(
    stops: &[elwindui_core::graphics::GradientStop],
) -> Vec<CanvasGradientStop> {
    stops
        .iter()
        .map(|stop| CanvasGradientStop {
            Position: stop.offset,
            Color: graphics_color_to_winui_color(stop.color),
        })
        .collect()
}

pub(crate) fn win2d_path_bounds(
    commands: &[elwindui_core::graphics::PathCommand],
) -> elwindui_core::base::Rect {
    use elwindui_core::graphics::{PathBuilder, PathCommand};

    let mut builder = PathBuilder::new();
    for command in commands {
        match command {
            PathCommand::MoveTo(point) => {
                builder.move_to(*point);
            }
            PathCommand::LineTo(point) => {
                builder.line_to(*point);
            }
            PathCommand::QuadTo { control, to } => {
                builder.quad_to(*control, *to);
            }
            PathCommand::CubicTo {
                control1,
                control2,
                to,
            } => {
                builder.cubic_to(*control1, *control2, *to);
            }
            PathCommand::ArcTo(arc) => {
                builder.arc_to(*arc);
            }
            PathCommand::Close => {
                builder.close();
            }
        }
    }
    builder
        .build()
        .map(|path| path.bounds())
        .unwrap_or(elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        })
}

/// Materializes one core brush inside a transient Win2D drawing session. Win2D resources
/// are deliberately not retained in `Win2dPrimitive`: that keeps the retained scene device-loss
/// safe and ensures every resource belongs to the drawing session's own device.
pub(crate) fn win2d_brush(
    creator: &ICanvasResourceCreator,
    brush: &elwindui_core::graphics::Brush,
    bounds: elwindui_core::base::Rect,
    opacity: f32,
) -> Result<ICanvasBrush> {
    use elwindui_core::graphics::Brush;

    match brush {
        Brush::Solid(color) => {
            let native =
                CanvasSolidColorBrush::Create(creator, graphics_color_to_winui_color(*color))?;
            native.SetOpacity(opacity.clamp(0.0, 1.0))?;
            native.cast()
        }
        Brush::LinearGradient(gradient) => {
            let stops = win2d_gradient_stops(&gradient.stops);
            let native = CanvasLinearGradientBrush::CreateWithStops(creator, &stops)?;
            native.SetStartPoint(win2d_gradient_point(
                gradient.start,
                gradient.mapping,
                bounds,
            ))?;
            native.SetEndPoint(win2d_gradient_point(gradient.end, gradient.mapping, bounds))?;
            native.SetTransform(win2d_brush_matrix(gradient.transform))?;
            native.SetOpacity((gradient.opacity * opacity).clamp(0.0, 1.0))?;
            native.cast()
        }
        Brush::RadialGradient(gradient) => {
            let stops = win2d_gradient_stops(&gradient.stops);
            let native = CanvasRadialGradientBrush::CreateWithStops(creator, &stops)?;
            let center = win2d_gradient_point(gradient.center, gradient.mapping, bounds);
            let origin = win2d_gradient_point(gradient.gradient_origin, gradient.mapping, bounds);
            native.SetCenter(center)?;
            native.SetOriginOffset(windows_numerics::Vector2 {
                X: origin.X - center.X,
                Y: origin.Y - center.Y,
            })?;
            native.SetRadiusX(win2d_gradient_radius(
                gradient.radius_x,
                gradient.mapping,
                bounds.width,
            ))?;
            native.SetRadiusY(win2d_gradient_radius(
                gradient.radius_y,
                gradient.mapping,
                bounds.height,
            ))?;
            native.SetTransform(win2d_brush_matrix(gradient.transform))?;
            native.SetOpacity((gradient.opacity * opacity).clamp(0.0, 1.0))?;
            native.cast()
        }
        Brush::Image(image_brush) => {
            let bitmap = win2d_bitmap(creator, &image_brush.image)?;
            let native = CanvasImageBrush::CreateWithImage(creator, &bitmap)?;
            let bitmap_size = bitmap.SizeInPixels()?;
            let source = image_brush
                .source_rect
                .unwrap_or(elwindui_core::base::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: bitmap_size.Width as f32,
                    height: bitmap_size.Height as f32,
                });
            if image_brush.source_rect.is_some() {
                let source = windows::Foundation::Rect {
                    X: source.x,
                    Y: source.y,
                    Width: source.width,
                    Height: source.height,
                };
                let reference: windows::Foundation::IReference<windows::Foundation::Rect> =
                    PropertyValue::CreateRect(source)?.cast()?;
                native.SetSourceRectangle(&reference)?;
            }
            let (extend_x, extend_y) = match image_brush.tile_mode {
                elwindui_core::graphics::TileMode::None => {
                    (CanvasEdgeBehavior::Clamp, CanvasEdgeBehavior::Clamp)
                }
                elwindui_core::graphics::TileMode::Tile => {
                    (CanvasEdgeBehavior::Wrap, CanvasEdgeBehavior::Wrap)
                }
                elwindui_core::graphics::TileMode::FlipX => {
                    (CanvasEdgeBehavior::Mirror, CanvasEdgeBehavior::Clamp)
                }
                elwindui_core::graphics::TileMode::FlipY => {
                    (CanvasEdgeBehavior::Clamp, CanvasEdgeBehavior::Mirror)
                }
                elwindui_core::graphics::TileMode::FlipXY => {
                    (CanvasEdgeBehavior::Mirror, CanvasEdgeBehavior::Mirror)
                }
            };
            native.SetExtendX(extend_x)?;
            native.SetExtendY(extend_y)?;
            native.SetInterpolation(CanvasImageInterpolation::Linear)?;
            let fit = match image_brush.stretch {
                elwindui_core::graphics::Stretch::None => elwindui_core::graphics::ImageFit::None,
                elwindui_core::graphics::Stretch::Fill => elwindui_core::graphics::ImageFit::Fill,
                elwindui_core::graphics::Stretch::Uniform => {
                    elwindui_core::graphics::ImageFit::Contain
                }
                elwindui_core::graphics::Stretch::UniformToFill => {
                    elwindui_core::graphics::ImageFit::Cover
                }
            };
            let layout = win2d_fitted_image_rect(
                bounds,
                (source.width, source.height),
                &elwindui_core::graphics::ImageDrawOptions {
                    opacity: 1.0,
                    sampling: elwindui_core::graphics::ImageSampling::Linear,
                    fit,
                    alignment_x: image_brush.alignment_x,
                    alignment_y: image_brush.alignment_y,
                    repeat: image_brush.tile_mode,
                },
            );
            let layout_transform =
                elwindui_core::base::AffineTransform::translation(layout.x, layout.y)
                    .concat(&elwindui_core::base::AffineTransform::scale(
                        layout.width / source.width.max(1e-6),
                        layout.height / source.height.max(1e-6),
                    ))
                    .concat(&elwindui_core::base::AffineTransform::translation(
                        -source.x, -source.y,
                    ));
            native.SetTransform(win2d_brush_matrix(
                layout_transform.concat(&image_brush.transform),
            ))?;
            native.SetOpacity((image_brush.opacity * opacity).clamp(0.0, 1.0))?;
            native.cast()
        }
    }
}

#[allow(dead_code)]
pub(crate) fn win2d_stroke_style(
    stroke: &elwindui_core::graphics::StrokeStyle,
) -> Result<crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasStrokeStyle> {
    let native = crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasStrokeStyle::new()?;
    native.SetMiterLimit(stroke.miter_limit)?;
    let cap = |cap| match cap {
        elwindui_core::graphics::LineCap::Butt => {
            crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasCapStyle::Flat
        }
        elwindui_core::graphics::LineCap::Round => {
            crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasCapStyle::Round
        }
        elwindui_core::graphics::LineCap::Square => {
            crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasCapStyle::Square
        }
    };
    native.SetStartCap(cap(stroke.start_cap))?;
    native.SetEndCap(cap(stroke.end_cap))?;
    native.SetDashCap(cap(stroke.dash_cap))?;
    native.SetDashOffset(stroke.dash_offset)?;
    native.SetCustomDashStyle(&stroke.dash_pattern)?;
    native.SetLineJoin(match stroke.line_join {
        elwindui_core::graphics::LineJoin::Miter => {
            crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasLineJoin::Miter
        }
        elwindui_core::graphics::LineJoin::Round => {
            crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasLineJoin::Round
        }
        elwindui_core::graphics::LineJoin::Bevel => {
            crate::bindings::Microsoft::Graphics::Canvas::Geometry::CanvasLineJoin::Bevel
        }
    })?;
    Ok(native)
}

pub(crate) fn win2d_bitmap(
    creator: &ICanvasResourceCreator,
    image: &elwindui_core::graphics::Image,
) -> Result<CanvasBitmap> {
    use elwindui_core::graphics::{AlphaMode, ImageData};

    match image.data() {
        ImageData::Encoded { bytes, .. } => {
            let stream = InMemoryRandomAccessStream::new()?;
            let writer = DataWriter::CreateDataWriter(&stream)?;
            writer.WriteBytes(bytes)?;
            writer.StoreAsync()?.join()?;
            let stream: IRandomAccessStream = stream.cast()?;
            stream.Seek(0)?;
            CanvasBitmap::LoadAsyncFromStream(creator, &stream)?.join()
        }
        ImageData::Rgba8 {
            width,
            height,
            stride,
            pixels,
            alpha,
        } => {
            let mut bgra = Vec::with_capacity((*width as usize) * (*height as usize) * 4);
            for row in 0..*height as usize {
                let row_start = row * *stride as usize;
                for pixel in pixels[row_start..row_start + *width as usize * 4].chunks_exact(4) {
                    let (r, g, b, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);
                    let (r, g, b) = match alpha {
                        AlphaMode::Straight => (
                            ((r as u16 * a as u16 + 127) / 255) as u8,
                            ((g as u16 * a as u16 + 127) / 255) as u8,
                            ((b as u16 * a as u16 + 127) / 255) as u8,
                        ),
                        AlphaMode::Premultiplied => (r, g, b),
                        AlphaMode::Opaque => (r, g, b),
                    };
                    bgra.extend_from_slice(&[
                        b,
                        g,
                        r,
                        if *alpha == AlphaMode::Opaque { 255 } else { a },
                    ]);
                }
            }
            CanvasBitmap::CreateFromBytes(
                creator,
                &bgra,
                (*width).try_into().unwrap(),
                (*height).try_into().unwrap(),
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
            )
        }
        ImageData::Backend(handle) => {
            handle
                .0
                .downcast_ref::<CanvasBitmap>()
                .cloned()
                .ok_or_else(|| {
                    windows::core::Error::new(
                        windows::core::HRESULT(0x80070057_u32 as i32),
                        "Image backend handle is not a Win2D CanvasBitmap",
                    )
                })
        }
    }
}

pub(crate) fn win2d_fitted_image_rect(
    dest: elwindui_core::base::Rect,
    image_size: (f32, f32),
    options: &elwindui_core::graphics::ImageDrawOptions,
) -> elwindui_core::base::Rect {
    use elwindui_core::graphics::{AlignmentX, AlignmentY, ImageFit};
    let (iw, ih) = image_size;
    let (width, height) = if iw <= 0.0 || ih <= 0.0 {
        (dest.width, dest.height)
    } else {
        match options.fit {
            ImageFit::Fill => (dest.width, dest.height),
            ImageFit::None => (iw, ih),
            ImageFit::Contain => {
                let scale = (dest.width / iw).min(dest.height / ih);
                (iw * scale, ih * scale)
            }
            ImageFit::Cover => {
                let scale = (dest.width / iw).max(dest.height / ih);
                (iw * scale, ih * scale)
            }
        }
    };
    let x = match options.alignment_x {
        AlignmentX::Left => dest.x,
        AlignmentX::Center => dest.x + (dest.width - width) / 2.0,
        AlignmentX::Right => dest.x + dest.width - width,
    };
    let y = match options.alignment_y {
        AlignmentY::Top => dest.y,
        AlignmentY::Center => dest.y + (dest.height - height) / 2.0,
        AlignmentY::Bottom => dest.y + dest.height - height,
    };
    elwindui_core::base::Rect {
        x,
        y,
        width,
        height,
    }
}

pub(crate) fn push_win2d_transform(
    out: &mut Vec<Win2dPrimitive>,
    transform: elwindui_core::base::AffineTransform,
) {
    out.push(Win2dPrimitive::SetTransform {
        m11: transform.m11,
        m12: transform.m12,
        m21: transform.m21,
        m22: transform.m22,
        dx: transform.dx,
        dy: transform.dy,
    });
}

/// Converts our own `elwindui_core::graphics::Color` (RGBA field order) into a `Windows::UI::Color`
/// (ARGB field order) — a plain field re-shuffle, no hex round-trip needed now that `Color` is a
/// real value type rather than a backend-agnostic hex string (painter design doc §18).
pub(crate) fn graphics_color_to_winui_color(c: elwindui_core::graphics::Color) -> Color {
    Color {
        A: c.a,
        R: c.r,
        G: c.g,
        B: c.b,
    }
}

pub(crate) fn solid_color_brush(color: elwindui_core::graphics::Color) -> Result<SolidColorBrush> {
    let brush = SolidColorBrush::new()?;
    brush.SetColor(graphics_color_to_winui_color(color))?;
    Ok(brush)
}

/// `elwindui_core::ui::TextAlignment` -> `Microsoft.UI.Xaml.TextAlignment`.
pub(crate) fn xaml_text_alignment(
    alignment: elwindui_core::ui::TextAlignment,
) -> bindings::Microsoft::UI::Xaml::TextAlignment {
    match alignment {
        elwindui_core::ui::TextAlignment::Left => {
            bindings::Microsoft::UI::Xaml::TextAlignment::Left
        }
        elwindui_core::ui::TextAlignment::Center => {
            bindings::Microsoft::UI::Xaml::TextAlignment::Center
        }
        elwindui_core::ui::TextAlignment::Right => {
            bindings::Microsoft::UI::Xaml::TextAlignment::Right
        }
    }
}
