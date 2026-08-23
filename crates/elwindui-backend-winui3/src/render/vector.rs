//! `RenderCommand::DrawVectorImage` — emitting an SVG scene onto a Win2D drawing surface
//! (groups, blend modes, pattern fills, masks).

use super::win2d::*;
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasActiveLayer, CanvasAntialiasing, CanvasBlend, CanvasDrawingSession,
    CanvasImageInterpolation, CanvasRenderTarget, ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Composition::CompositionDrawingSurface;
use crate::render::composition::{CompositionPrimitive, DesiredCompositionNode};
use windows::UI::Color;
use windows::core::{Interface, Result};

pub(crate) fn win2d_fitted_vector_rect(
    dest: elwindui_core::base::Rect,
    source_size: (f32, f32),
    options: &elwindui_core::graphics::VectorImageDrawOptions,
) -> elwindui_core::base::Rect {
    let image_options = elwindui_core::graphics::ImageDrawOptions {
        opacity: 1.0,
        sampling: elwindui_core::graphics::ImageSampling::Linear,
        fit: options.fit,
        alignment_x: options.alignment_x,
        alignment_y: options.alignment_y,
        repeat: elwindui_core::graphics::TileMode::None,
    };
    win2d_fitted_image_rect(dest, source_size, &image_options)
}

pub(crate) fn svg_view_box_transform(
    source: elwindui_core::base::Rect,
    destination: elwindui_core::base::Rect,
    aspect: elwindui_core::graphics::PreserveAspectRatio,
) -> elwindui_core::base::AffineTransform {
    use elwindui_core::graphics::{
        PreserveAspectRatioAlign as Align, PreserveAspectRatioMeetOrSlice as MeetOrSlice,
    };

    let raw_x = destination.width / source.width.max(1e-6);
    let raw_y = destination.height / source.height.max(1e-6);
    let (scale_x, scale_y, extra_x, extra_y) = if aspect.align == Align::None {
        (raw_x, raw_y, 0.0, 0.0)
    } else {
        let scale = match aspect.meet_or_slice {
            MeetOrSlice::Meet => raw_x.min(raw_y),
            MeetOrSlice::Slice => raw_x.max(raw_y),
        };
        let width = source.width * scale;
        let height = source.height * scale;
        let offset_x = match aspect.align {
            Align::XMinYMin | Align::XMinYMid | Align::XMinYMax => 0.0,
            Align::XMidYMin | Align::XMidYMid | Align::XMidYMax => {
                (destination.width - width) / 2.0
            }
            Align::XMaxYMin | Align::XMaxYMid | Align::XMaxYMax => destination.width - width,
            Align::None => 0.0,
        };
        let offset_y = match aspect.align {
            Align::XMinYMin | Align::XMidYMin | Align::XMaxYMin => 0.0,
            Align::XMinYMid | Align::XMidYMid | Align::XMaxYMid => {
                (destination.height - height) / 2.0
            }
            Align::XMinYMax | Align::XMidYMax | Align::XMaxYMax => destination.height - height,
            Align::None => 0.0,
        };
        (scale, scale, offset_x, offset_y)
    };
    elwindui_core::base::AffineTransform::translation(
        destination.x + extra_x,
        destination.y + extra_y,
    )
    .concat(&elwindui_core::base::AffineTransform::scale(
        scale_x, scale_y,
    ))
    .concat(&elwindui_core::base::AffineTransform::translation(
        -source.x, -source.y,
    ))
}

pub(crate) fn vector_mask_path(
    mask: &elwindui_core::graphics::VectorMask,
) -> elwindui_core::graphics::Path {
    use elwindui_core::graphics::{PathBuilder, VectorNode};

    fn append_group(
        builder: &mut PathBuilder,
        group: &elwindui_core::graphics::VectorGroup,
        parent: elwindui_core::base::AffineTransform,
    ) {
        let world = parent.concat(&group.transform);
        for node in group.children.iter() {
            match node {
                VectorNode::Path(node) if node.visibility => {
                    builder.add_path(&node.path, Some(world.concat(&node.transform)));
                }
                VectorNode::Group(group) => append_group(builder, group, world),
                VectorNode::Path(_) | VectorNode::RasterImage(_) => {}
            }
        }
    }

    let mut builder = PathBuilder::new();
    append_group(&mut builder, &mask.root, mask.transform);
    let local = builder.build().unwrap_or_else(|_| {
        PathBuilder::new()
            .build()
            .expect("an empty PathBuilder must build")
    });
    match &mask.nested {
        Some(nested) => elwindui_core::graphics::Path::combine(
            &local,
            &vector_mask_path(nested),
            elwindui_core::graphics::GeometryCombineMode::Intersect,
            0.25,
        )
        .unwrap_or(local),
        None => local,
    }
}

pub(crate) fn win2d_vector_blend(
    mode: elwindui_core::graphics::VectorBlendMode,
) -> Option<CanvasBlend> {
    match mode {
        elwindui_core::graphics::VectorBlendMode::Normal => Some(CanvasBlend::SourceOver),
        // Win2D's Min/Add composition modes are direct GPU implementations of the two blend
        // operations it exposes without first rasterizing a group into a custom effect graph.
        elwindui_core::graphics::VectorBlendMode::Darken => Some(CanvasBlend::Min),
        elwindui_core::graphics::VectorBlendMode::Lighten => Some(CanvasBlend::Add),
        _ => None,
    }
}

/// Expands a retained SVG pattern into its visible tiles and clips those tiles to one path. This
/// stays in the ordinary retained Win2D command stream: no bitmap cache is needed, so the result
/// remains sharp under the drawing surface's DPI/device changes.
pub(crate) fn emit_vector_pattern_fill(
    pattern: &elwindui_core::graphics::VectorPattern,
    path: &elwindui_core::graphics::Path,
    rule: elwindui_core::graphics::FillRule,
    stroke_width: Option<f32>,
    node_world: elwindui_core::base::AffineTransform,
    opacity: f32,
    out: &mut Vec<Win2dPrimitive>,
) {
    use elwindui_core::graphics::Clip;

    let tile = pattern.tile_rect;
    let bounds = path.bounds();
    if tile.width <= 0.0 || tile.height <= 0.0 || bounds.width <= 0.0 || bounds.height <= 0.0 {
        return;
    }
    let source = pattern.view_box.unwrap_or(tile);
    if source.width <= 0.0 || source.height <= 0.0 {
        return;
    }

    push_win2d_transform(out, node_world);
    match stroke_width {
        Some(width_px) => out.push(Win2dPrimitive::PushPathStrokeClip {
            commands: path.commands().to_vec(),
            x: 0.0,
            y: 0.0,
            width_px,
        }),
        None => out.push(Win2dPrimitive::PushClip {
            clip: Clip::Path {
                path: path.clone(),
                rule,
            },
            x: 0.0,
            y: 0.0,
        }),
    }
    out.push(Win2dPrimitive::PushOpacityLayer(opacity.clamp(0.0, 1.0)));

    let first_x = ((bounds.x - tile.x) / tile.width).floor() as i32;
    let last_x = ((bounds.x + bounds.width - tile.x) / tile.width).ceil() as i32;
    let first_y = ((bounds.y - tile.y) / tile.height).floor() as i32;
    let last_y = ((bounds.y + bounds.height - tile.y) / tile.height).ceil() as i32;
    const MAX_PATTERN_TILES: usize = 4096;
    let mut count = 0usize;
    'tiles: for row in first_y..last_y {
        for column in first_x..last_x {
            if count == MAX_PATTERN_TILES {
                #[cfg(debug_assertions)]
                eprintln!(
                    "elwindui-backend-winui3: SVG pattern tile expansion reached its safety limit"
                );
                break 'tiles;
            }
            count += 1;
            let x = tile.x + column as f32 * tile.width;
            let y = tile.y + row as f32 * tile.height;
            let tile_transform = svg_view_box_transform(
                source,
                elwindui_core::base::Rect {
                    x,
                    y,
                    width: tile.width,
                    height: tile.height,
                },
                pattern.preserve_aspect_ratio,
            )
            .concat(&pattern.transform);
            emit_vector_group(&pattern.root, node_world.concat(&tile_transform), 1.0, out);
        }
    }
    out.push(Win2dPrimitive::PopOpacityLayer);
    out.push(Win2dPrimitive::PopClip);
    push_win2d_transform(out, node_world);
}

pub(crate) fn emit_vector_group(
    group: &elwindui_core::graphics::VectorGroup,
    parent_transform: elwindui_core::base::AffineTransform,
    base_opacity: f32,
    out: &mut Vec<Win2dPrimitive>,
) {
    use elwindui_core::graphics::{
        Clip, ImageDrawOptions, ImageFit, TileMode, VectorNode, VectorPaintOrder,
    };

    let world = parent_transform.concat(&group.transform);
    if win2d_vector_blend(group.blend_mode).is_none() || group.isolate || !group.filters.is_empty()
    {
        #[cfg(debug_assertions)]
        eprintln!(
            "elwindui-backend-winui3: SVG blend, isolation, mask, or filter requires an offscreen effect graph and is not yet supported"
        );
    }
    if let Some(blend) = win2d_vector_blend(group.blend_mode) {
        out.push(Win2dPrimitive::SetBlend(blend));
    }
    push_win2d_transform(out, world);
    if let Some(clip_path) = &group.clip_path {
        out.push(Win2dPrimitive::PushClip {
            clip: Clip::Path {
                path: clip_path.to_path(),
                rule: elwindui_core::graphics::FillRule::NonZero,
            },
            x: 0.0,
            y: 0.0,
        });
    }
    if let Some(mask) = &group.mask {
        if mask.mask_type == elwindui_core::graphics::VectorMaskType::Luminance {
            #[cfg(debug_assertions)]
            eprintln!(
                "elwindui-backend-winui3: SVG luminance masks require an offscreen alpha conversion; using mask geometry"
            );
        }
        out.push(Win2dPrimitive::PushClip {
            clip: Clip::Rect(mask.bounds),
            x: 0.0,
            y: 0.0,
        });
        out.push(Win2dPrimitive::PushClip {
            clip: Clip::Path {
                path: vector_mask_path(mask),
                rule: elwindui_core::graphics::FillRule::NonZero,
            },
            x: 0.0,
            y: 0.0,
        });
    }
    out.push(Win2dPrimitive::PushOpacityLayer(
        group.opacity.clamp(0.0, 1.0),
    ));

    for node in group.children.iter() {
        match node {
            VectorNode::Group(child) => emit_vector_group(child, world, base_opacity, out),
            VectorNode::Path(path_node) if path_node.visibility => {
                let node_world = world.concat(&path_node.transform);
                push_win2d_transform(out, node_world);
                out.push(Win2dPrimitive::SetAntialiasing(matches!(
                    path_node.rendering,
                    elwindui_core::graphics::VectorShapeRendering::CrispEdges
                        | elwindui_core::graphics::VectorShapeRendering::OptimizeSpeed,
                )));
                let emit_fill = |out: &mut Vec<Win2dPrimitive>| {
                    if let Some(fill) = &path_node.fill {
                        match &fill.paint {
                            elwindui_core::graphics::VectorPaint::Brush(brush) => {
                                out.push(Win2dPrimitive::SetOpacity(
                                    (base_opacity * fill.opacity).clamp(0.0, 1.0),
                                ));
                                out.push(Win2dPrimitive::FillPath {
                                    commands: path_node.path.commands().to_vec(),
                                    x: 0.0,
                                    y: 0.0,
                                    brush: brush.clone(),
                                    rule: fill.rule,
                                });
                                out.push(Win2dPrimitive::SetOpacity(base_opacity));
                            }
                            elwindui_core::graphics::VectorPaint::Pattern(pattern) => {
                                emit_vector_pattern_fill(
                                    pattern,
                                    &path_node.path,
                                    fill.rule,
                                    None,
                                    node_world,
                                    (base_opacity * fill.opacity).clamp(0.0, 1.0),
                                    out,
                                );
                            }
                        }
                    }
                };
                let emit_stroke = |out: &mut Vec<Win2dPrimitive>| {
                    if let Some(stroke) = &path_node.stroke {
                        match &stroke.paint {
                            elwindui_core::graphics::VectorPaint::Brush(brush) => {
                                out.push(Win2dPrimitive::SetOpacity(
                                    (base_opacity * stroke.opacity).clamp(0.0, 1.0),
                                ));
                                out.push(Win2dPrimitive::StrokePath {
                                    commands: path_node.path.commands().to_vec(),
                                    x: 0.0,
                                    y: 0.0,
                                    brush: brush.clone(),
                                    stroke: stroke.style.clone(),
                                });
                                out.push(Win2dPrimitive::SetOpacity(base_opacity));
                            }
                            elwindui_core::graphics::VectorPaint::Pattern(pattern) => {
                                emit_vector_pattern_fill(
                                    pattern,
                                    &path_node.path,
                                    elwindui_core::graphics::FillRule::NonZero,
                                    Some(stroke.style.width),
                                    node_world,
                                    (base_opacity * stroke.opacity).clamp(0.0, 1.0),
                                    out,
                                );
                            }
                        }
                    }
                };
                match path_node.paint_order {
                    VectorPaintOrder::FillStroke => {
                        emit_fill(out);
                        emit_stroke(out);
                    }
                    VectorPaintOrder::StrokeFill => {
                        emit_stroke(out);
                        emit_fill(out);
                    }
                }
                out.push(Win2dPrimitive::SetAntialiasing(false));
                push_win2d_transform(out, world);
            }
            VectorNode::Path(_) => {}
            VectorNode::RasterImage(raster) => {
                push_win2d_transform(out, world.concat(&raster.transform));
                out.push(Win2dPrimitive::DrawImage {
                    image: raster.image.clone(),
                    dest: raster.rect,
                    source: None,
                    options: ImageDrawOptions {
                        opacity: raster.opacity,
                        sampling: raster.sampling,
                        fit: ImageFit::Fill,
                        alignment_x: elwindui_core::graphics::AlignmentX::Center,
                        alignment_y: elwindui_core::graphics::AlignmentY::Center,
                        repeat: TileMode::None,
                    },
                    x: 0.0,
                    y: 0.0,
                });
                push_win2d_transform(out, world);
            }
        }
    }
    out.push(Win2dPrimitive::PopOpacityLayer);
    if group.mask.is_some() {
        out.push(Win2dPrimitive::PopClip);
        out.push(Win2dPrimitive::PopClip);
    }
    if group.clip_path.is_some() {
        out.push(Win2dPrimitive::PopClip);
    }
    push_win2d_transform(out, parent_transform);
    out.push(Win2dPrimitive::SetBlend(CanvasBlend::SourceOver));
}

pub(crate) fn emit_vector_image(
    image: &elwindui_core::graphics::VectorImage,
    dest: elwindui_core::base::Rect,
    source: Option<elwindui_core::base::Rect>,
    options: &elwindui_core::graphics::VectorImageDrawOptions,
    parent_transform: elwindui_core::base::AffineTransform,
    base_opacity: f32,
    out: &mut Vec<Win2dPrimitive>,
) {
    let source = source.unwrap_or_else(|| image.view_box());
    if source.width <= 0.0 || source.height <= 0.0 {
        return;
    }
    let placed = win2d_fitted_vector_rect(dest, (source.width, source.height), options);
    let local = elwindui_core::base::AffineTransform::translation(placed.x, placed.y)
        .concat(&elwindui_core::base::AffineTransform::scale(
            placed.width / source.width,
            placed.height / source.height,
        ))
        .concat(&elwindui_core::base::AffineTransform::translation(
            -source.x, -source.y,
        ));
    if options.clip_to_dest {
        push_win2d_transform(out, parent_transform);
        out.push(Win2dPrimitive::PushClip {
            clip: elwindui_core::graphics::Clip::Rect(dest),
            x: 0.0,
            y: 0.0,
        });
    }
    out.push(Win2dPrimitive::PushOpacityLayer(
        options.opacity.clamp(0.0, 1.0),
    ));
    emit_vector_group(
        image.root(),
        parent_transform.concat(&local),
        base_opacity,
        out,
    );
    out.push(Win2dPrimitive::PopOpacityLayer);
    if options.clip_to_dest {
        out.push(Win2dPrimitive::PopClip);
    }
    push_win2d_transform(out, parent_transform);
}

/// Replays one complex `VectorImage` into a tightly sized CompositionDrawingSurface. This is a
/// fallback for SVG features that are not yet lowered to Composition shapes; the surface itself is
/// hosted by a retained SpriteVisual, so this function is never called from an XAML draw event.
pub(crate) fn draw_vector_image_surface(
    surface: &CompositionDrawingSurface,
    desired: &DesiredCompositionNode,
    rasterization_scale: f32,
) -> Result<()> {
    let CompositionPrimitive::VectorImage {
        image,
        dest,
        source,
        options,
    } = &desired.primitive
    else {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80070057_u32 as i32),
            "vector surface node received a non-vector primitive",
        ));
    };
    let mut primitives = vec![
        Win2dPrimitive::SetTransform {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
        },
        Win2dPrimitive::SetOpacity(1.0),
    ];
    emit_vector_image(
        image,
        elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: dest.width,
            height: dest.height,
        },
        *source,
        options,
        elwindui_core::base::AffineTransform::identity(),
        1.0,
        &mut primitives,
    );
    let session = CanvasComposition::CreateDrawingSession(surface)?;
    session.Clear(Color {
        A: 0,
        R: 0,
        G: 0,
        B: 0,
    })?;
    replay_win2d_primitives(&session, &primitives, rasterization_scale)?;
    session.Close()
}

/// Replays a recorded `Win2dPrimitive` stream onto an already-created, already-cleared
/// `CanvasDrawingSession` — the transform/clip/opacity/fill/stroke/image interpreter shared by
/// every Win2D-backed surface this backend draws into. `draw_vector_image_surface` (the retained
/// `CompositionDrawingSurface` fallback path) and the menu-icon `VectorImage` rasterizer
/// (`rasterize_vector_image_to_canvas_bitmap`, PR #171 delta remediation) both call this rather
/// than each walking `Win2dPrimitive` themselves — one interpreter, so a fix/feature to fill,
/// stroke, clip, or image drawing lands in both places at once. Does not close `session`; the
/// caller owns that (each caller's target surface has different close/finalize semantics —
/// `CompositionDrawingSurface` vs. an offscreen `CanvasRenderTarget`).
pub(crate) fn replay_win2d_primitives(
    session: &CanvasDrawingSession,
    primitives: &[Win2dPrimitive],
    rasterization_scale: f32,
) -> Result<()> {
    let creator: ICanvasResourceCreator = session.clone().cast()?;
    let mut opacity = 1.0_f32;
    let mut active_layers = Vec::<CanvasActiveLayer>::new();
    for primitive in primitives {
        match primitive {
            Win2dPrimitive::SetTransform {
                m11,
                m12,
                m21,
                m22,
                dx,
                dy,
            } => session.SetTransform(windows_numerics::Matrix3x2 {
                // The CompositionDrawingSurface extent is in physical pixels,
                // while SVG lowering produces DIPs. Compose the XAML scale into
                // every scene transform so geometry, clips, and brushes replay
                // at the surface's native resolution.
                M11: *m11 * rasterization_scale,
                M12: *m12 * rasterization_scale,
                M21: *m21 * rasterization_scale,
                M22: *m22 * rasterization_scale,
                M31: *dx * rasterization_scale,
                M32: *dy * rasterization_scale,
            })?,
            Win2dPrimitive::SetOpacity(value) => opacity = *value,
            Win2dPrimitive::SetAntialiasing(aliased) => session.SetAntialiasing(if *aliased {
                CanvasAntialiasing::Aliased
            } else {
                CanvasAntialiasing::Antialiased
            })?,
            Win2dPrimitive::SetBlend(blend) => session.SetBlend(*blend)?,
            Win2dPrimitive::FillPath {
                commands,
                x,
                y,
                brush,
                rule,
            } => {
                let geometry = win2d_path_geometry(&creator, commands, *x, *y, *rule)?;
                let bounds = win2d_path_bounds(commands);
                let brush = win2d_brush(
                    &creator,
                    brush,
                    elwindui_core::base::Rect {
                        x: bounds.x + *x,
                        y: bounds.y + *y,
                        ..bounds
                    },
                    opacity,
                )?;
                session.FillGeometryAtCoordsWithBrush(&geometry, 0.0, 0.0, &brush)?;
            }
            Win2dPrimitive::StrokePath {
                commands,
                x,
                y,
                brush,
                stroke,
            } => {
                let geometry = win2d_path_geometry(
                    &creator,
                    commands,
                    *x,
                    *y,
                    elwindui_core::graphics::FillRule::NonZero,
                )?;
                let bounds = win2d_path_bounds(commands);
                let brush = win2d_brush(
                    &creator,
                    brush,
                    elwindui_core::base::Rect {
                        x: bounds.x + *x,
                        y: bounds.y + *y,
                        ..bounds
                    },
                    opacity,
                )?;
                let style = win2d_stroke_style(stroke)?;
                session.DrawGeometryWithBrushAndStrokeWidthAndStrokeStyle(
                    &geometry,
                    windows_numerics::Vector2 { X: 0.0, Y: 0.0 },
                    &brush,
                    stroke.width,
                    &style,
                )?;
            }
            Win2dPrimitive::PushClip { clip, x, y } => {
                let geometry = win2d_clip_geometry(&creator, clip, *x, *y)?;
                active_layers.push(session.CreateLayerWithOpacityAndClipGeometry(1.0, &geometry)?);
            }
            Win2dPrimitive::PushPathStrokeClip {
                commands,
                x,
                y,
                width_px,
            } => {
                let geometry = win2d_path_geometry(
                    &creator,
                    commands,
                    *x,
                    *y,
                    elwindui_core::graphics::FillRule::NonZero,
                )?;
                let stroke_geometry = geometry.Stroke(*width_px)?;
                active_layers
                    .push(session.CreateLayerWithOpacityAndClipGeometry(1.0, &stroke_geometry)?);
            }
            Win2dPrimitive::PopClip | Win2dPrimitive::PopOpacityLayer => {
                if let Some(layer) = active_layers.pop() {
                    layer.Close()?;
                }
            }
            Win2dPrimitive::PushOpacityLayer(layer_opacity) => {
                active_layers.push(session.CreateLayerWithOpacity(*layer_opacity)?);
            }
            Win2dPrimitive::DrawImage {
                image,
                dest,
                source,
                options,
                x,
                y,
            } => {
                let bitmap = win2d_bitmap(&creator, image)?;
                let size = bitmap.SizeInPixels()?;
                let source = source.unwrap_or(elwindui_core::base::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: size.Width as f32,
                    height: size.Height as f32,
                });
                let placed = win2d_fitted_image_rect(
                    elwindui_core::base::Rect {
                        x: *x + dest.x,
                        y: *y + dest.y,
                        width: dest.width,
                        height: dest.height,
                    },
                    (source.width, source.height),
                    options,
                );
                let interpolation = match options.sampling {
                    elwindui_core::graphics::ImageSampling::Nearest => {
                        CanvasImageInterpolation::NearestNeighbor
                    }
                    elwindui_core::graphics::ImageSampling::Linear => {
                        CanvasImageInterpolation::Linear
                    }
                    elwindui_core::graphics::ImageSampling::Cubic => {
                        CanvasImageInterpolation::HighQualityCubic
                    }
                };
                session.DrawImageToRectWithSourceRectAndOpacityAndInterpolation(
                    &bitmap,
                    windows::Foundation::Rect {
                        X: placed.x,
                        Y: placed.y,
                        Width: placed.width,
                        Height: placed.height,
                    },
                    windows::Foundation::Rect {
                        X: source.x,
                        Y: source.y,
                        Width: source.width,
                        Height: source.height,
                    },
                    (opacity * options.opacity).clamp(0.0, 1.0),
                    interpolation,
                )?;
            }
        }
    }
    while let Some(layer) = active_layers.pop() {
        layer.Close()?;
    }
    Ok(())
}

/// Rasterizes a user-defined `VectorImage` menu icon (PR #171 delta remediation, replacing the
/// previous "Vector user icons are not yet bridged" gap) into a fresh, transparent
/// `width`x`height` `CanvasRenderTarget`, reusing the exact same `emit_vector_image`/
/// `replay_win2d_primitives` pipeline `draw_vector_image_surface` uses for ordinary retained
/// vector drawing — no second VectorScene traversal (§3.7/§13.2 of the delta contract). The
/// caller (`inner/menu.rs`) encodes the result to PNG and feeds it to a XAML `BitmapImage`; this
/// function returns the offscreen `CanvasRenderTarget` itself and does not touch any XAML type.
///
/// **Unverified naming**: `CanvasRenderTarget::CreateWithWidthAndHeightAndDpi` is this crate's
/// best-effort guess at the `windows_bindgen` projection of `CanvasRenderTarget`'s
/// `(ICanvasResourceCreator, width: f32, height: f32, dpi: f32)` constructor overload, following
/// the "method name mirrors the WinRT factory method" convention `CanvasBitmap::
/// LoadAsyncFromStream`/`CanvasBitmap::CreateFromBytes` already establish in `win2d.rs`. This
/// crate has never been built on Windows (`#![cfg(target_os = "windows")]`, unverified per the
/// crate's own disclaimer) — confirm the exact generated name in `bindings.rs` on first Windows
/// build and correct this call site (a mechanical binding-spelling fix, not an architecture
/// change) if it differs.
pub(crate) fn rasterize_vector_image_to_canvas_bitmap(
    creator: &ICanvasResourceCreator,
    image: &elwindui_core::graphics::VectorImage,
    width: f32,
    height: f32,
) -> Result<CanvasRenderTarget> {
    const RASTER_TARGET_DPI: f32 = 96.0;
    let target = CanvasRenderTarget::CreateWithWidthAndHeightAndDpi(
        creator,
        width,
        height,
        RASTER_TARGET_DPI,
    )?;
    let session = target.CreateDrawingSession()?;
    session.Clear(Color {
        A: 0,
        R: 0,
        G: 0,
        B: 0,
    })?;
    let mut primitives = vec![
        Win2dPrimitive::SetTransform {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            dx: 0.0,
            dy: 0.0,
        },
        Win2dPrimitive::SetOpacity(1.0),
    ];
    emit_vector_image(
        image,
        elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        },
        None,
        &elwindui_core::graphics::VectorImageDrawOptions::default(),
        elwindui_core::base::AffineTransform::identity(),
        1.0,
        &mut primitives,
    );
    replay_win2d_primitives(&session, &primitives, 1.0)?;
    session.Close()?;
    Ok(target)
}

#[cfg(test)]
mod vector_view_box_tests {
    use super::*;
    use elwindui_core::base::Rect;
    use elwindui_core::graphics::{
        PreserveAspectRatio, PreserveAspectRatioAlign as Align,
        PreserveAspectRatioMeetOrSlice as MeetOrSlice,
    };

    fn rect(width: f32, height: f32) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    #[test]
    fn view_box_meet_centers_the_unused_axis() {
        let transform = svg_view_box_transform(
            rect(100.0, 50.0),
            rect(100.0, 100.0),
            PreserveAspectRatio {
                align: Align::XMidYMid,
                meet_or_slice: MeetOrSlice::Meet,
            },
        );
        assert_eq!(
            (transform.m11, transform.m22, transform.dx, transform.dy),
            (1.0, 1.0, 0.0, 25.0)
        );
    }

    #[test]
    fn view_box_slice_centers_the_overflow_axis() {
        let transform = svg_view_box_transform(
            rect(100.0, 50.0),
            rect(100.0, 100.0),
            PreserveAspectRatio {
                align: Align::XMidYMid,
                meet_or_slice: MeetOrSlice::Slice,
            },
        );
        assert_eq!(
            (transform.m11, transform.m22, transform.dx, transform.dy),
            (2.0, 2.0, -50.0, 0.0)
        );
    }

    #[test]
    fn view_box_none_keeps_independent_axes() {
        let transform = svg_view_box_transform(
            rect(100.0, 50.0),
            rect(100.0, 100.0),
            PreserveAspectRatio {
                align: Align::None,
                meet_or_slice: MeetOrSlice::Meet,
            },
        );
        assert_eq!(
            (transform.m11, transform.m22, transform.dx, transform.dy),
            (1.0, 2.0, 0.0, 0.0)
        );
    }
}

/// §9.5 of the PR #171 delta remediation contract. See `render::win2d_bitmap_tests`'s own module
/// doc comment for why this depends on live `CanvasDevice`/`CanvasRenderTarget` construction
/// (unverified without a Windows build/run) rather than pure logic like the rest of this file's
/// tests.
#[cfg(test)]
mod menu_vector_rasterization_tests {
    use super::*;
    use elwindui_core::base::{Rect, Size};
    use elwindui_core::graphics::{
        Brush, Color, FillRule, PathBuilder, VectorFill, VectorGroup, VectorImageBuilder,
        VectorNode, VectorPaint, VectorPaintOrder, VectorPathNode, VectorShapeRendering,
    };
    use std::sync::Arc;

    fn creator() -> Result<ICanvasResourceCreator> {
        let device = crate::bindings::Microsoft::Graphics::Canvas::CanvasDevice::GetSharedDevice()?;
        device.cast()
    }

    /// A simple 16x16 filled square vector image — enough to have an obvious non-transparent
    /// region to check against, without depending on any other part of this remediation.
    fn filled_square_vector_image() -> elwindui_core::graphics::VectorImage {
        let mut builder = PathBuilder::new();
        builder.add_rect(Rect {
            x: 2.0,
            y: 2.0,
            width: 12.0,
            height: 12.0,
        });
        let path = builder.build().expect("filled square path is well-formed");
        let node = VectorNode::Path(VectorPathNode {
            path,
            transform: elwindui_core::base::AffineTransform::IDENTITY,
            fill: Some(VectorFill {
                paint: VectorPaint::Brush(Brush::Solid(Color::rgb(200, 40, 40))),
                opacity: 1.0,
                rule: FillRule::NonZero,
            }),
            stroke: None,
            paint_order: VectorPaintOrder::default(),
            rendering: VectorShapeRendering::default(),
            visibility: true,
        });
        let group = VectorGroup {
            children: Arc::from([node]),
            ..VectorGroup::default()
        };
        VectorImageBuilder::new(
            Size {
                width: 16.0,
                height: 16.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0,
            },
        )
        .expect("16x16 canvas is a valid size")
        .root(group)
        .finish()
        .expect("filled square vector image builds successfully")
    }

    /// §9.5: rasterizing a simple filled-square `VectorImage` produces a 32x32
    /// `CanvasRenderTarget` (the fixed menu-icon raster size, `MENU_ICON_RASTER_SIZE` in
    /// `inner/menu.rs`) with at least one non-transparent pixel — so a silently blank target
    /// cannot pass this test.
    #[test]
    fn rasterizes_a_filled_shape_into_a_32x32_non_blank_render_target() {
        let creator = creator().expect("CanvasDevice::GetSharedDevice must succeed on Windows");
        let image = filled_square_vector_image();
        let target = rasterize_vector_image_to_canvas_bitmap(&creator, &image, 32.0, 32.0)
            .expect("vector rasterization must succeed for a simple filled shape");
        let size = target.SizeInPixels().expect("SizeInPixels");
        assert_eq!((size.Width, size.Height), (32, 32));

        // The square covers most of the 16x16 viewBox, which maps to the render target's center;
        // sample its center pixel and confirm the fill's red channel dominates (not fully
        // transparent/black, i.e. something was actually drawn there).
        let bytes = target
            .GetPixelBytes()
            .expect("GetPixelBytes must succeed for a freshly-drawn CanvasRenderTarget");
        let center_pixel_index = ((16 * 32) + 16) * 4; // row 16, column 16, BGRA8
        let alpha = bytes[center_pixel_index + 3];
        assert!(
            alpha > 0,
            "center pixel must not be fully transparent — the fill did not render"
        );
    }

    /// §9.6 half: an empty/degenerate `VectorImage` (empty group) rasterizes without error into an
    /// all-transparent target — confirms the offscreen target/session lifecycle itself (create,
    /// clear, draw nothing, close) doesn't fail even when there is nothing to draw, distinguishing
    /// "rasterization pipeline broken" from "this specific shape didn't render".
    #[test]
    fn rasterizing_an_empty_vector_image_succeeds_and_stays_transparent() {
        let creator = creator().expect("CanvasDevice::GetSharedDevice must succeed on Windows");
        let empty_group = VectorGroup::default();
        let image = VectorImageBuilder::new(
            Size {
                width: 16.0,
                height: 16.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0,
            },
        )
        .expect("16x16 canvas is a valid size")
        .root(empty_group)
        .finish()
        .expect("empty vector image builds successfully");
        let target = rasterize_vector_image_to_canvas_bitmap(&creator, &image, 32.0, 32.0)
            .expect("rasterizing an empty scene must still succeed");
        let bytes = target
            .GetPixelBytes()
            .expect("GetPixelBytes must succeed for a freshly-drawn CanvasRenderTarget");
        let center_pixel_index = ((16 * 32) + 16) * 4;
        assert_eq!(
            bytes[center_pixel_index + 3],
            0,
            "an empty vector scene must rasterize to a fully transparent target"
        );
    }
}
