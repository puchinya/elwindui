//! Matrix/rect math and the image-surface helpers shared across the composition renderer.

use super::*;

use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{CanvasImageBrush, ICanvasBrush};
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasBitmap, CanvasEdgeBehavior, CanvasImageInterpolation, ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Composition::{
    CompositionDrawingSurface, CompositionStretch, CompositionSurfaceBrush,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::UIElement;
use elwindui_core::base::{AffineTransform, Point, Rect};
use elwindui_core::graphics::{Brush, Image, ImageData, Stretch, TileMode};
use windows::Foundation::Rect as WinRect;
use windows::Foundation::Size as WinSize;
use windows::Storage::Streams::{DataWriter, IRandomAccessStream, InMemoryRandomAccessStream};
use windows::UI::Color as WinColor;
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Matrix4x4, Vector2};

pub(crate) fn island_local_matrix(transform: AffineTransform, island: Rect) -> Matrix3x2 {
    Matrix3x2 {
        M11: transform.m11,
        M12: transform.m12,
        M21: transform.m21,
        M22: transform.m22,
        M31: transform.dx - island.x,
        M32: transform.dy - island.y,
    }
}

pub(crate) fn matrix(transform: AffineTransform) -> Matrix3x2 {
    Matrix3x2 {
        M11: transform.m11,
        M12: transform.m12,
        M21: transform.m21,
        M22: transform.m22,
        M31: transform.dx,
        M32: transform.dy,
    }
}

pub(crate) fn image_visual_matrix(
    transform: AffineTransform,
    rect: Rect,
    island: Rect,
) -> Matrix4x4 {
    // SpriteVisual coordinates start at the image's top-left, unlike a
    // SpriteShape whose geometry already carries `rect.x/y`. Fold that local
    // translation into the visual transform, then convert the 2D affine matrix
    // to the row-vector Matrix4x4 ABI expected by IVisual.
    Matrix4x4 {
        M11: transform.m11,
        M12: transform.m12,
        M13: 0.0,
        M14: 0.0,
        M21: transform.m21,
        M22: transform.m22,
        M23: 0.0,
        M24: 0.0,
        M31: 0.0,
        M32: 0.0,
        M33: 1.0,
        M34: 0.0,
        M41: rect.x * transform.m11 + rect.y * transform.m21 + transform.dx - island.x,
        M42: rect.x * transform.m12 + rect.y * transform.m22 + transform.dy - island.y,
        M43: 0.0,
        M44: 1.0,
    }
}

pub(crate) fn vector2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub(crate) fn transformed_bounds(rect: Rect, transform: AffineTransform) -> Rect {
    let corners = [
        Point {
            x: rect.x,
            y: rect.y,
        },
        Point {
            x: rect.x + rect.width,
            y: rect.y,
        },
        Point {
            x: rect.x,
            y: rect.y + rect.height,
        },
        Point {
            x: rect.x + rect.width,
            y: rect.y + rect.height,
        },
    ]
    .map(|point| transform.transform_point(point));
    let min_x = corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let min_y = corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    Rect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

pub(crate) fn draw_image_surface(
    surface: &CompositionDrawingSurface,
    desired: &DesiredCompositionNode,
    rasterization_scale: f32,
) -> Result<()> {
    let image = image_brush(desired).expect("checked by requires_drawing_surface");
    let rect = desired.primitive.local_bounds();
    let session = CanvasComposition::CreateDrawingSession(surface)?;
    session.Clear(WinColor {
        A: 0,
        R: 0,
        G: 0,
        B: 0,
    })?;
    session.SetTransform(raster_scale_matrix(rasterization_scale))?;
    let creator: ICanvasResourceCreator = session.clone().cast()?;
    let bitmap = canvas_bitmap(&creator, &image.image)?;
    let size = bitmap.SizeInPixels()?;
    let source = image.source_rect.unwrap_or(Rect {
        x: 0.0,
        y: 0.0,
        width: size.Width as f32,
        height: size.Height as f32,
    });
    let local = Rect {
        x: 0.0,
        y: 0.0,
        width: rect.width,
        height: rect.height,
    };
    if image.tile_mode == TileMode::None {
        let placed = fitted_image_rect(local, (source.width, source.height), image);
        session.DrawImageToRectWithSourceRectAndOpacityAndInterpolation(
            &bitmap,
            win_rect(placed),
            win_rect(source),
            1.0,
            CanvasImageInterpolation::Linear,
        )?;
    } else {
        let image_brush = CanvasImageBrush::CreateWithImage(&creator, &bitmap)?;
        image_brush.SetExtendX(match image.tile_mode {
            TileMode::Tile => CanvasEdgeBehavior::Wrap,
            TileMode::FlipX | TileMode::FlipXY => CanvasEdgeBehavior::Mirror,
            TileMode::FlipY => CanvasEdgeBehavior::Wrap,
            TileMode::None => CanvasEdgeBehavior::Clamp,
        })?;
        image_brush.SetExtendY(match image.tile_mode {
            TileMode::Tile => CanvasEdgeBehavior::Wrap,
            TileMode::FlipY | TileMode::FlipXY => CanvasEdgeBehavior::Mirror,
            TileMode::FlipX => CanvasEdgeBehavior::Wrap,
            TileMode::None => CanvasEdgeBehavior::Clamp,
        })?;
        image_brush.SetTransform(matrix(image.transform))?;
        let brush: ICanvasBrush = image_brush.cast()?;
        match desired.primitive {
            CompositionPrimitive::Rectangle { .. } => {
                session.FillRectangleWithBrush(win_rect(local), &brush)?
            }
            CompositionPrimitive::RoundedRectangle { radii, .. } => {
                let radius =
                    (radii.top_left + radii.top_right + radii.bottom_right + radii.bottom_left)
                        / 4.0;
                session.FillRoundedRectangleWithBrush(win_rect(local), radius, radius, &brush)?;
            }
            CompositionPrimitive::Ellipse { .. } => session.FillEllipseWithBrush(
                vector2(local.width / 2.0, local.height / 2.0),
                local.width / 2.0,
                local.height / 2.0,
                &brush,
            )?,
            _ => unreachable!("image primitives are filtered by image_brush"),
        }
    }
    session.Close()
}

pub(crate) fn rasterization_scale(canvas: &Canvas) -> Result<f32> {
    let element: UIElement = canvas.clone().cast()?;
    let scale = element.RasterizationScale()? as f32;
    Ok(if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    })
}

pub(crate) fn surface_size(rect: Rect, rasterization_scale: f32) -> WinSize {
    let scale = rasterization_scale.max(0.01);
    WinSize {
        Width: (rect.width * scale).ceil().max(1.0),
        Height: (rect.height * scale).ceil().max(1.0),
    }
}

pub(crate) fn raster_scale_matrix(rasterization_scale: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: rasterization_scale,
        M12: 0.0,
        M21: 0.0,
        M22: rasterization_scale,
        M31: 0.0,
        M32: 0.0,
    }
}

pub(crate) fn canvas_bitmap(
    creator: &ICanvasResourceCreator,
    image: &Image,
) -> Result<CanvasBitmap> {
    let ImageData::Encoded { bytes, .. } = image.data() else {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80004001_u32 as i32),
            "CompositionDrawingSurface replay currently requires encoded image data",
        ));
    };
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    let stream: IRandomAccessStream = stream.cast()?;
    stream.Seek(0)?;
    CanvasBitmap::LoadAsyncFromStream(creator, &stream)?.join()
}

pub(crate) fn fitted_image_rect(
    dest: Rect,
    source_size: (f32, f32),
    image: &elwindui_core::graphics::ImageBrush,
) -> Rect {
    let (source_width, source_height) = source_size;
    let (width, height) = match image.stretch {
        Stretch::Fill => (dest.width, dest.height),
        Stretch::Uniform => {
            let scale = (dest.width / source_width).min(dest.height / source_height);
            (source_width * scale, source_height * scale)
        }
        Stretch::UniformToFill => {
            let scale = (dest.width / source_width).max(dest.height / source_height);
            (source_width * scale, source_height * scale)
        }
        Stretch::None => (source_width, source_height),
    };
    let align_x = match image.alignment_x {
        elwindui_core::graphics::AlignmentX::Left => 0.0,
        elwindui_core::graphics::AlignmentX::Center => 0.5,
        elwindui_core::graphics::AlignmentX::Right => 1.0,
    };
    let align_y = match image.alignment_y {
        elwindui_core::graphics::AlignmentY::Top => 0.0,
        elwindui_core::graphics::AlignmentY::Center => 0.5,
        elwindui_core::graphics::AlignmentY::Bottom => 1.0,
    };
    Rect {
        x: dest.x + (dest.width - width) * align_x,
        y: dest.y + (dest.height - height) * align_y,
        width,
        height,
    }
}

pub(crate) fn win_rect(rect: Rect) -> WinRect {
    WinRect {
        X: rect.x,
        Y: rect.y,
        Width: rect.width,
        Height: rect.height,
    }
}

pub(crate) fn image_brush(
    desired: &DesiredCompositionNode,
) -> Option<&elwindui_core::graphics::ImageBrush> {
    let Some(Brush::Image(image)) = desired.fill.as_ref() else {
        return None;
    };
    matches!(
        desired.primitive,
        CompositionPrimitive::Rectangle { .. }
            | CompositionPrimitive::RoundedRectangle { .. }
            | CompositionPrimitive::Ellipse { .. }
    )
    .then_some(image)
    .filter(|_| desired.stroke.is_none())
}

pub(crate) fn is_image_node(desired: &DesiredCompositionNode) -> bool {
    image_brush(desired).is_some()
}

pub(crate) fn requires_drawing_surface(desired: &DesiredCompositionNode) -> bool {
    let Some(image) = image_brush(desired) else {
        return false;
    };
    image.source_rect.is_some() || image.tile_mode != TileMode::None
}

pub(crate) fn apply_image_brush(
    brush: &CompositionSurfaceBrush,
    image: &elwindui_core::graphics::ImageBrush,
) -> Result<()> {
    brush.SetStretch(match image.stretch {
        Stretch::None => CompositionStretch::None,
        Stretch::Fill => CompositionStretch::Fill,
        Stretch::Uniform => CompositionStretch::Uniform,
        Stretch::UniformToFill => CompositionStretch::UniformToFill,
    })?;
    brush.SetHorizontalAlignmentRatio(match image.alignment_x {
        elwindui_core::graphics::AlignmentX::Left => 0.0,
        elwindui_core::graphics::AlignmentX::Center => 0.5,
        elwindui_core::graphics::AlignmentX::Right => 1.0,
    })?;
    brush.SetVerticalAlignmentRatio(match image.alignment_y {
        elwindui_core::graphics::AlignmentY::Top => 0.0,
        elwindui_core::graphics::AlignmentY::Center => 0.5,
        elwindui_core::graphics::AlignmentY::Bottom => 1.0,
    })
}
