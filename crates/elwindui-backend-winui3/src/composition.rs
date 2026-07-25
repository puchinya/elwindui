//! Retained Microsoft.UI.Composition renderer for custom-drawn RenderCommands.
//!
//! XAML remains responsible for ordering composition islands against native controls. Each island
//! owns one empty XAML Canvas with a child ContainerVisual. The island's figures are retained as
//! CompositionSpriteShapes in a shared ShapeVisual and reconciled by stable RenderNodeId.

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

pub(crate) type RenderNodeId = (u64, usize);
pub(crate) type IslandId = RenderNodeId;

// windows-bindgen 0.62 omits this method because its Microsoft.UI.Composition
// metadata signature refers to DirectX value types that are deliberately not
// emitted by our filtered bindings. The IID and method order below match the
// Windows App SDK ICompositionGraphicsDevice interface. Keep this wrapper local
// to that one missing method; all other Composition calls use generated bindings.
windows::core::imp::define_interface!(
    RawCompositionGraphicsDevice,
    RawCompositionGraphicsDeviceVtbl,
    0x3d47e3f5_f76c_5f1f_88c0_54a5f2a090d6
);

#[repr(C)]
#[allow(dead_code, reason = "the vtable is read through RawCompositionGraphicsDevice")]
pub struct RawCompositionGraphicsDeviceVtbl {
    base__: windows::core::IInspectable_Vtbl,
    create_drawing_surface: unsafe extern "system" fn(
        *mut c_void,
        WinSize,
        DirectXPixelFormat,
        DirectXAlphaMode,
        *mut *mut c_void,
    ) -> windows::core::HRESULT,
    // This event adder is not needed here, but occupies the second slot in the
    // public ICompositionGraphicsDevice vtable.
    _rendering_device_replaced: usize,
    _remove_rendering_device_replaced:
        unsafe extern "system" fn(*mut c_void, i64) -> windows::core::HRESULT,
}

pub(crate) fn create_drawing_surface(
    device: &CompositionGraphicsDevice,
    size: WinSize,
) -> Result<CompositionDrawingSurface> {
    let raw: RawCompositionGraphicsDevice = device.cast()?;
    unsafe {
        let mut result = std::ptr::null_mut();
        (Interface::vtable(&raw).create_drawing_surface)(
            Interface::as_raw(&raw),
            size,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
            &mut result,
        )
        .and_then(|| Type::from_abi(result))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CompositionClipSpec {
    Rect {
        rect: Rect,
        transform: AffineTransform,
    },
    RoundedRect {
        rect: Rect,
        radii: CornerRadius,
        transform: AffineTransform,
    },
    Path {
        commands: Vec<PathCommand>,
        rule: FillRule,
        origin: Point,
        transform: AffineTransform,
    },
}

impl CompositionClipSpec {
    fn world_bounds(&self) -> Rect {
        match self {
            Self::Rect { rect, transform } | Self::RoundedRect { rect, transform, .. } => {
                transformed_bounds(*rect, *transform)
            }
            Self::Path {
                commands,
                origin,
                transform,
                ..
            } => transformed_bounds(
                CompositionPrimitive::Path {
                    commands: commands.clone(),
                    rule: FillRule::NonZero,
                    origin: *origin,
                }
                .local_bounds(),
                *transform,
            ),
        }
    }

    fn transform(&self) -> AffineTransform {
        match self {
            Self::Rect { transform, .. }
            | Self::RoundedRect { transform, .. }
            | Self::Path { transform, .. } => *transform,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CompositionPrimitive {
    Rectangle {
        rect: Rect,
    },
    RoundedRectangle {
        rect: Rect,
        radii: CornerRadius,
    },
    Ellipse {
        rect: Rect,
    },
    Line {
        from: Point,
        to: Point,
    },
    Path {
        commands: Vec<PathCommand>,
        rule: FillRule,
        origin: Point,
    },
    /// Complex SVG is rasterized once into a tightly-sized CompositionDrawingSurface. The
    /// resulting surface remains a retained SpriteVisual in its island; it is never replayed by
    /// an XAML immediate-draw control.
    VectorImage {
        image: VectorImage,
        dest: Rect,
        source: Option<Rect>,
        options: VectorImageDrawOptions,
    },
}

impl CompositionPrimitive {
    fn local_bounds(&self) -> Rect {
        match self {
            Self::Rectangle { rect }
            | Self::RoundedRectangle { rect, .. }
            | Self::Ellipse { rect } => *rect,
            Self::Line { from, to } => Rect {
                x: from.x.min(to.x),
                y: from.y.min(to.y),
                width: (from.x - to.x).abs(),
                height: (from.y - to.y).abs(),
            },
            Self::Path {
                commands, origin, ..
            } => {
                use elwindui_core::graphics::PathBuilder;

                let mut builder = PathBuilder::new();
                for command in commands {
                    match command {
                        PathCommand::MoveTo(point) => builder.move_to(*point),
                        PathCommand::LineTo(point) => builder.line_to(*point),
                        PathCommand::QuadTo { control, to } => builder.quad_to(*control, *to),
                        PathCommand::CubicTo {
                            control1,
                            control2,
                            to,
                        } => builder.cubic_to(*control1, *control2, *to),
                        PathCommand::ArcTo(arc) => builder.arc_to(*arc),
                        PathCommand::Close => builder.close(),
                    };
                }
                let bounds = builder.build().map(|path| path.bounds()).unwrap_or(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                });
                Rect {
                    x: bounds.x + origin.x,
                    y: bounds.y + origin.y,
                    ..bounds
                }
            }
            Self::VectorImage { dest, .. } => *dest,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DesiredCompositionNode {
    pub id: RenderNodeId,
    pub primitive: CompositionPrimitive,
    pub fill: Option<Brush>,
    pub stroke: Option<(Brush, StrokeStyle)>,
    pub transform: AffineTransform,
    pub opacity: f32,
}

impl DesiredCompositionNode {
    pub(crate) fn world_bounds(&self) -> Rect {
        let mut rect = self.primitive.local_bounds();
        if let Some((_, stroke)) = &self.stroke {
            let outset = stroke.width * 0.5;
            rect.x -= outset;
            rect.y -= outset;
            rect.width += outset * 2.0;
            rect.height += outset * 2.0;
        }
        transformed_bounds(rect, self.transform)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DesiredCompositionIsland {
    pub id: IslandId,
    pub nodes: Vec<DesiredCompositionNode>,
    pub bounds: Rect,
    pub clips: Vec<CompositionClipSpec>,
}

impl DesiredCompositionIsland {
    pub(crate) fn from_nodes(
        nodes: Vec<DesiredCompositionNode>,
        clips: Vec<CompositionClipSpec>,
    ) -> Option<Self> {
        let id = nodes.first()?.id;
        let mut bounds = nodes[0].world_bounds();
        for node in nodes.iter().skip(1) {
            bounds = union_rect(bounds, node.world_bounds());
        }
        for clip in &clips {
            bounds = intersect_rect(bounds, clip.world_bounds())?;
        }
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
        {
            return None;
        }
        bounds.width = bounds.width.max(1.0);
        bounds.height = bounds.height.max(1.0);
        Some(Self {
            id,
            nodes,
            bounds,
            clips,
        })
    }
}

#[derive(Debug)]
pub(crate) struct UnsupportedCompositionNode {
    pub id: RenderNodeId,
    pub reason: &'static str,
}

pub(crate) struct CompositionRenderer {
    compositor: Compositor,
    canvas_device: CanvasDevice,
    islands: HashMap<IslandId, CompositionIslandState>,
    image_surfaces: ImageSurfaceCache,
}

/// Keeps the WinRT stream alive until WinUI has finished decoding the corresponding surface.
/// Entries are owned by the renderer, so a retained Composition node never recreates an image
/// surface during an ordinary layout pass.
struct LoadedSurface {
    surface: LoadedImageSurface,
    _stream: IRandomAccessStream,
    load_completed_token: Option<i64>,
}

#[derive(Default)]
struct ImageSurfaceCache {
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

impl CompositionRenderer {
    pub(crate) fn new(canvas: &Canvas) -> Result<Self> {
        let host: UIElement = canvas.clone().cast()?;
        let element_visual = ElementCompositionPreview::GetElementVisual(&host)?;
        let compositor = element_visual.Compositor()?;
        let canvas_device = CanvasDevice::GetSharedDevice()?;
        let graphics_device = CanvasComposition::CreateCompositionGraphicsDevice(
            &compositor,
            &canvas_device,
        )?;
        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            // Exercise the one raw ABI call during explicit diagnostics. This is
            // deliberately a tiny, detached surface: it verifies the fallback
            // factory without changing normal retained-island rendering.
            let probe = create_drawing_surface(
                &graphics_device,
                WinSize {
                    Width: 1.0,
                    Height: 1.0,
                },
            )?;
            let _ = probe.Close();
            eprintln!("elwindui-winui3: CompositionDrawingSurface ABI fallback is available");
        }
        Ok(Self {
            compositor,
            canvas_device,
            islands: HashMap::new(),
            image_surfaces: ImageSurfaceCache::default(),
        })
    }

    pub(crate) fn reconcile(
        &mut self,
        canvas: &Canvas,
        wanted: Vec<DesiredCompositionIsland>,
    ) -> Result<(Vec<(IslandId, UIElement)>, Vec<UnsupportedCompositionNode>)> {
        // Composition visuals use device-independent pixels, while a drawing
        // surface's extent is physical pixels. Recreate only raster fallbacks
        // when the XAML rasterization scale changes; shape and image-surface
        // visuals remain in DIPs.
        let rasterization_scale = rasterization_scale(canvas)?;
        self.image_surfaces.retain_for(&wanted);
        let mut live = HashSet::new();
        let mut ordered_hosts = Vec::with_capacity(wanted.len());
        let mut unsupported = Vec::new();

        let (islands, image_surfaces) = (&mut self.islands, &mut self.image_surfaces);
        for island in wanted {
            live.insert(island.id);
            if !islands.contains_key(&island.id) {
                let state = CompositionIslandState::new(canvas, &self.compositor)?;
                islands.insert(island.id, state);
            }
            let state = islands.get_mut(&island.id).expect("inserted above");
            state.reconcile(
                &self.compositor,
                &self.canvas_device,
                image_surfaces,
                &island,
                rasterization_scale,
                &mut unsupported,
            )?;
            ordered_hosts.push((island.id, state.host_element()?));
        }

        let removed: Vec<_> = islands
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in removed {
            if let Some(state) = islands.remove(&id) {
                state.detach(canvas)?;
            }
        }

        Ok((ordered_hosts, unsupported))
    }

}

impl Drop for CompositionRenderer {
    fn drop(&mut self) {
        self.image_surfaces.clear();
    }
}

struct CompositionIslandState {
    host: Canvas,
    root: ContainerVisual,
    nodes: HashMap<RenderNodeId, CompositionNodeState>,
    shape_runs: Vec<ShapeRunState>,
    order: Vec<RenderNodeId>,
    bounds: Rect,
    clip: Option<ClipState>,
    clip_snapshot: Vec<CompositionClipSpec>,
}

impl CompositionIslandState {
    fn new(canvas: &Canvas, compositor: &Compositor) -> Result<Self> {
        let host = Canvas::new()?;
        let host_ui: UIElement = host.clone().cast()?;
        host_ui.SetIsHitTestVisible(false)?;

        let root = compositor.CreateContainerVisual()?;
        let root_visual: Visual = root.clone().cast()?;
        ElementCompositionPreview::SetElementChildVisual(&host_ui, &root_visual)?;
        canvas.Children()?.Append(&host_ui)?;

        Ok(Self {
            host,
            root,
            nodes: HashMap::new(),
            shape_runs: Vec::new(),
            order: Vec::new(),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            clip: None,
            clip_snapshot: Vec::new(),
        })
    }

    fn host_element(&self) -> Result<UIElement> {
        self.host.clone().cast()
    }

    fn detach(self, canvas: &Canvas) -> Result<()> {
        let host_ui: UIElement = self.host.clone().cast()?;
        ElementCompositionPreview::SetElementChildVisual(
            &host_ui,
            Option::<&Visual>::None,
        )?;
        let children = canvas.Children()?;
        let mut index = 0;
        if children.IndexOf(&host_ui, &mut index)? {
            children.RemoveAt(index)?;
        }
        Ok(())
    }

    fn reconcile(
        &mut self,
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        image_surfaces: &mut ImageSurfaceCache,
        wanted: &DesiredCompositionIsland,
        rasterization_scale: f32,
        unsupported: &mut Vec<UnsupportedCompositionNode>,
    ) -> Result<()> {
        let bounds_changed = self.bounds != wanted.bounds;
        if bounds_changed {
            self.update_bounds(wanted.bounds)?;
        }
        if self.clip_snapshot != wanted.clips {
            self.clip = (!wanted.clips.is_empty())
                .then_some(())
                .map(|_| ClipState::create(compositor, canvas_device, &wanted.clips, wanted.bounds))
                .transpose()
                .map_err(|reason| {
                    windows::core::Error::new(
                        windows::core::HRESULT(0x80004001_u32 as i32),
                        reason,
                    )
                })?;
            let clip = self
                .clip
                .as_ref()
                .map(ClipState::as_clip)
                .transpose()?;
            self.root.SetClip(clip.as_ref())?;
            self.clip_snapshot = wanted.clips.clone();
        }

        let wanted_ids: HashSet<_> = wanted.nodes.iter().map(|node| node.id).collect();
        let removed: Vec<_> = self
            .nodes
            .keys()
            .copied()
            .filter(|id| !wanted_ids.contains(id))
            .collect();
        for id in removed {
            self.nodes.remove(&id);
        }

        let mut next_order = Vec::with_capacity(wanted.nodes.len());
        for desired in &wanted.nodes {
            match self.nodes.get_mut(&desired.id) {
                Some(existing) if existing.can_update(desired, rasterization_scale) => {
                    match existing.update(
                        compositor,
                        desired,
                        self.bounds,
                        bounds_changed,
                        rasterization_scale,
                        image_surfaces,
                    ) {
                        Ok(()) => next_order.push(desired.id),
                        Err(error) => {
                            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                                eprintln!(
                                    "elwindui-winui3: Composition node {:?} update failed: {error:?}",
                                    desired.id,
                                );
                            }
                            self.nodes.remove(&desired.id);
                            unsupported.push(UnsupportedCompositionNode {
                                id: desired.id,
                                reason: "Composition node property update failed",
                            });
                        }
                    }
                }
                _ => match CompositionNodeState::create(
                    compositor,
                    canvas_device,
                    desired,
                    self.bounds,
                    rasterization_scale,
                    image_surfaces,
                ) {
                    Ok(state) => {
                        self.nodes.insert(desired.id, state);
                        next_order.push(desired.id);
                    }
                    Err(reason) => {
                        self.nodes.remove(&desired.id);
                        unsupported.push(UnsupportedCompositionNode {
                            id: desired.id,
                            reason,
                        });
                    }
                },
            }
        }

        let shape_runs_changed = self.reconcile_shape_runs(compositor, &next_order)?;
        if self.order != next_order || shape_runs_changed {
            let children = self.root.Children()?;
            children.RemoveAll()?;
            let runs_by_first_id: HashMap<_, _> = self
                .shape_runs
                .iter()
                .enumerate()
                .map(|(index, run)| (run.node_ids[0], index))
                .collect();
            for id in &next_order {
                let visual = match self.nodes.get(id).expect("ordered node exists") {
                    CompositionNodeState::Shape(_) => {
                        let Some(index) = runs_by_first_id.get(id) else {
                            continue;
                        };
                        self.shape_runs[*index].visual.clone().cast()?
                    }
                    node => node.visual()?,
                };
                children.InsertAtTop(&visual)?;
            }
            self.order = next_order;
        }
        Ok(())
    }

    fn reconcile_shape_runs(
        &mut self,
        compositor: &Compositor,
        order: &[RenderNodeId],
    ) -> Result<bool> {
        let wanted = shape_run_descriptors(order.iter().map(|id| {
            match self.nodes.get(id).expect("ordered node exists") {
                CompositionNodeState::Shape(shape) => {
                    Some((*id, shape.snapshot.opacity.clamp(0.0, 1.0)))
                }
                _ => None,
            }
        }));

        let unchanged = wanted.len() == self.shape_runs.len()
            && wanted.iter().zip(&self.shape_runs).all(|((ids, opacity), run)| {
                *ids == run.node_ids && *opacity == run.opacity
            });
        if unchanged {
            for run in &self.shape_runs {
                run.visual.SetSize(vector2(self.bounds.width, self.bounds.height))?;
            }
            return Ok(false);
        }

        // Sprite shapes are retained by RenderNode. Only their containing
        // ShapeVisual is rebuilt when a run boundary changes (for example, an
        // image is inserted or a node's opacity requires a distinct visual).
        // Detach them first because a CompositionShape can have only one parent.
        for run in &self.shape_runs {
            let shapes = run.visual.Shapes()?;
            while shapes.Size()? != 0 {
                shapes.RemoveAt(0)?;
            }
        }
        self.shape_runs.clear();
        for (node_ids, opacity) in wanted {
            self.shape_runs.push(ShapeRunState::create(
                compositor,
                &self.nodes,
                node_ids,
                opacity,
                self.bounds,
            )?);
        }
        Ok(true)
    }

    fn update_bounds(&mut self, bounds: Rect) -> Result<()> {
        let host: FrameworkElement = self.host.clone().cast()?;
        host.SetWidth(bounds.width as f64)?;
        host.SetHeight(bounds.height as f64)?;
        Canvas::SetLeft(&host, bounds.x as f64)?;
        Canvas::SetTop(&host, bounds.y as f64)?;
        let size = Vector2 {
            X: bounds.width,
            Y: bounds.height,
        };
        self.root.SetSize(size)?;
        self.bounds = bounds;
        Ok(())
    }
}

/// Groups only adjacent shapes that can share one ShapeVisual. Any non-shape
/// visual is an ordering boundary, as is a different opacity because opacity
/// belongs to the ShapeVisual rather than to individual brushes.
fn shape_run_descriptors<T: Copy>(
    entries: impl IntoIterator<Item = Option<(T, f32)>>,
) -> Vec<(Vec<T>, f32)> {
    let mut runs: Vec<(Vec<T>, f32)> = Vec::new();
    let mut previous_was_shape = false;
    for entry in entries {
        let Some((id, opacity)) = entry else {
            previous_was_shape = false;
            continue;
        };
        match runs.last_mut() {
            Some((ids, run_opacity)) if previous_was_shape && *run_opacity == opacity => {
                ids.push(id)
            }
            _ => runs.push((vec![id], opacity)),
        }
        previous_was_shape = true;
    }
    runs
}

struct ShapeRunState {
    visual: ShapeVisual,
    node_ids: Vec<RenderNodeId>,
    opacity: f32,
}

impl ShapeRunState {
    fn create(
        compositor: &Compositor,
        nodes: &HashMap<RenderNodeId, CompositionNodeState>,
        node_ids: Vec<RenderNodeId>,
        opacity: f32,
        island_bounds: Rect,
    ) -> Result<Self> {
        let visual = compositor.CreateShapeVisual()?;
        visual.SetSize(vector2(island_bounds.width, island_bounds.height))?;
        visual.SetOpacity(opacity)?;
        let shapes = visual.Shapes()?;
        for id in &node_ids {
            let CompositionNodeState::Shape(node) = nodes.get(id).expect("shape run node exists") else {
                unreachable!("shape run contains only shape nodes");
            };
            let shape: CompositionShape = node.shape.clone().cast()?;
            shapes.Append(&shape)?;
        }
        Ok(Self {
            visual,
            node_ids,
            opacity,
        })
    }
}

enum CompositionNodeState {
    Shape(ShapeNodeState),
    Image(ImageNodeState),
    VectorSurface(VectorSurfaceNode),
}

impl CompositionNodeState {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
        image_surfaces: &mut ImageSurfaceCache,
    ) -> std::result::Result<Self, &'static str> {
        if matches!(desired.primitive, CompositionPrimitive::VectorImage { .. }) {
            return VectorSurfaceNode::create(
                compositor,
                canvas_device,
                desired,
                island_bounds,
                rasterization_scale,
            )
            .map(Self::VectorSurface);
        }
        if is_image_node(desired) {
            return ImageNodeState::create(
                compositor,
                canvas_device,
                desired,
                island_bounds,
                rasterization_scale,
                image_surfaces,
            )
            .map(Self::Image);
        }
        ShapeNodeState::create(compositor, canvas_device, desired, island_bounds).map(Self::Shape)
    }

    fn can_update(&self, desired: &DesiredCompositionNode, rasterization_scale: f32) -> bool {
        match self {
            Self::Shape(node) => !is_image_node(desired) && node.can_update(desired),
            Self::Image(node) => {
                is_image_node(desired) && node.can_update(desired, rasterization_scale)
            }
            Self::VectorSurface(node) => node.can_update(desired, rasterization_scale),
        }
    }

    fn update(
        &mut self,
        compositor: &Compositor,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        island_bounds_changed: bool,
        rasterization_scale: f32,
        image_surfaces: &mut ImageSurfaceCache,
    ) -> Result<()> {
        match self {
            Self::Shape(node) => {
                node.update(compositor, desired, island_bounds, island_bounds_changed)
            }
            Self::Image(node) => {
                node.update(desired, island_bounds, rasterization_scale, image_surfaces)
            }
            Self::VectorSurface(node) => node.update(desired, island_bounds, rasterization_scale),
        }
    }

    fn visual(&self) -> Result<Visual> {
        match self {
            Self::Shape(_) => unreachable!("shape nodes are owned by ShapeRunState"),
            Self::Image(node) => node.visual(),
            Self::VectorSurface(node) => node.visual.clone().cast(),
        }
    }
}

struct ShapeNodeState {
    shape: CompositionSpriteShape,
    _geometry: GeometryState,
    fill: Option<BrushState>,
    stroke: Option<BrushState>,
    snapshot: DesiredCompositionNode,
}

impl ShapeNodeState {
    fn can_update(&self, desired: &DesiredCompositionNode) -> bool {
        match (&self.snapshot.primitive, &desired.primitive) {
            (CompositionPrimitive::Rectangle { .. }, CompositionPrimitive::Rectangle { .. })
            | (
                CompositionPrimitive::RoundedRectangle { .. },
                CompositionPrimitive::RoundedRectangle { .. },
            )
            | (CompositionPrimitive::Ellipse { .. }, CompositionPrimitive::Ellipse { .. })
            | (CompositionPrimitive::Line { .. }, CompositionPrimitive::Line { .. }) => true,
            (CompositionPrimitive::Path { .. }, CompositionPrimitive::Path { .. }) => {
                self.snapshot.primitive == desired.primitive
            }
            _ => false,
        }
    }

    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
    ) -> std::result::Result<Self, &'static str> {
        let geometry = GeometryState::create(compositor, canvas_device, &desired.primitive)?;
        let base_geometry = geometry
            .as_geometry()
            .map_err(|_| "geometry cast failed")?;
        let shape = compositor
            .CreateSpriteShapeWithGeometry(&base_geometry)
            .map_err(|_| "CreateSpriteShapeWithGeometry failed")?;
        let mut state = Self {
            shape,
            _geometry: geometry,
            fill: None,
            stroke: None,
            snapshot: desired.clone(),
        };
        // Transform is retained by the SpriteShape. Opacity is held by the
        // containing ShapeVisual run so multiple adjacent figures share one
        // visual without pushing alpha into their brushes.
        state
            .shape
            .SetTransformMatrix(island_local_matrix(desired.transform, island_bounds))
            .map_err(|_| "sprite shape transform initialization failed")?;
        if let Err(error) = state.update(compositor, desired, island_bounds, false) {
            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                eprintln!(
                    "elwindui-winui3: Composition node {:?} creation failed: {error:?}",
                    desired.id,
                );
            }
            return Err("Composition node property update failed");
        }
        Ok(state)
    }

    fn update(
        &mut self,
        compositor: &Compositor,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        island_bounds_changed: bool,
    ) -> Result<()> {
        if self.snapshot.primitive != desired.primitive {
            self._geometry.update(&desired.primitive)?;
        }
        if (self.fill.is_none() && desired.fill.is_some())
            || self.snapshot.fill != desired.fill
        {
            self.fill = desired
                .fill
                .as_ref()
                .map(|brush| BrushState::create(compositor, brush, 1.0))
                .transpose()?;
            let brush = match &self.fill {
                Some(brush) => Some(brush.as_brush()?),
                None => None,
            };
            self.shape.SetFillBrush(brush.as_ref())?;
        }
        if (self.stroke.is_none() && desired.stroke.is_some())
            || self.snapshot.stroke != desired.stroke
        {
            self.stroke = desired
                .stroke
                .as_ref()
                .map(|(brush, _)| BrushState::create(compositor, brush, 1.0))
                .transpose()?;
            let brush = match &self.stroke {
                Some(brush) => Some(brush.as_brush()?),
                None => None,
            };
            self.shape.SetStrokeBrush(brush.as_ref())?;
        }
        if let Some((_, stroke)) = &desired.stroke {
            apply_stroke(&self.shape, stroke)?;
        } else {
            self.shape.SetStrokeThickness(0.0)?;
        }
        if self.snapshot.transform != desired.transform || island_bounds_changed {
            self.shape
                .SetTransformMatrix(island_local_matrix(desired.transform, island_bounds))?;
        }
        self.snapshot = desired.clone();
        Ok(())
    }
}

/// Image brushes cannot be assigned to `CompositionSpriteShape::FillBrush`.
/// They are retained as their own SpriteVisual, which is ordered alongside the
/// shape visuals in the island root.
enum ImageNodeState {
    Sprite(SpriteImageNode),
    DrawingSurface(DrawingSurfaceImageNode),
}

impl ImageNodeState {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
        image_surfaces: &mut ImageSurfaceCache,
    ) -> std::result::Result<Self, &'static str> {
        if requires_drawing_surface(desired) {
            DrawingSurfaceImageNode::create(
                compositor,
                canvas_device,
                desired,
                island_bounds,
                rasterization_scale,
            )
            .map(Self::DrawingSurface)
        } else {
            SpriteImageNode::create(compositor, canvas_device, desired, island_bounds, image_surfaces)
                .map(Self::Sprite)
        }
    }

    fn can_update(&self, desired: &DesiredCompositionNode, rasterization_scale: f32) -> bool {
        match self {
            Self::Sprite(node) => !requires_drawing_surface(desired) && node.can_update(desired),
            Self::DrawingSurface(node) => {
                requires_drawing_surface(desired) && node.can_update(desired, rasterization_scale)
            }
        }
    }

    fn update(
        &mut self,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
        image_surfaces: &mut ImageSurfaceCache,
    ) -> Result<()> {
        match self {
            Self::Sprite(node) => node.update(desired, island_bounds, image_surfaces),
            Self::DrawingSurface(node) => node.update(desired, island_bounds, rasterization_scale),
        }
    }

    fn visual(&self) -> Result<Visual> {
        match self {
            Self::Sprite(node) => node.visual.clone().cast(),
            Self::DrawingSurface(node) => node.visual.clone().cast(),
        }
    }
}

struct SpriteImageNode {
    visual: SpriteVisual,
    _brush: CompositionSurfaceBrush,
    _surface: LoadedImageSurface,
    _clip: Option<ImageClipState>,
    snapshot: DesiredCompositionNode,
}

impl SpriteImageNode {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        image_surfaces: &mut ImageSurfaceCache,
    ) -> std::result::Result<Self, &'static str> {
        let image = image_brush(desired).expect("checked by is_image_node");
        let surface = image_surfaces.surface_for(&image.image)?;
        let surface_interface: ICompositionSurface = surface
            .clone()
            .cast()
            .map_err(|_| "LoadedImageSurface cast failed")?;
        let brush = compositor
            .CreateSurfaceBrushWithSurface(&surface_interface)
            .map_err(|_| "CreateSurfaceBrushWithSurface failed")?;
        apply_image_brush(&brush, image).map_err(|_| "SurfaceBrush setup failed")?;
        let visual = compositor
            .CreateSpriteVisual()
            .map_err(|_| "CreateSpriteVisual failed")?;
        let brush_interface: CompositionBrush = brush
            .clone()
            .cast()
            .map_err(|_| "CompositionSurfaceBrush cast failed")?;
        visual
            .SetBrush(&brush_interface)
            .map_err(|_| "SpriteVisual brush assignment failed")?;
        let clip = ImageClipState::create(compositor, canvas_device, &desired.primitive)
            .map_err(|_| "SpriteVisual image clip creation failed")?;
        if let Some(clip) = &clip {
            let clip_interface: CompositionClip = clip.clip.clone().cast()
                .map_err(|_| "SpriteVisual image clip cast failed")?;
            visual.SetClip(&clip_interface)
                .map_err(|_| "SpriteVisual image clip assignment failed")?;
        }
        let mut state = Self {
            visual,
            _brush: brush,
            _surface: surface,
            _clip: clip,
            snapshot: desired.clone(),
        };
        state
            .update(desired, island_bounds, image_surfaces)
            .map_err(|_| "SpriteVisual image update failed")?;
        Ok(state)
    }

    fn can_update(&self, desired: &DesiredCompositionNode) -> bool {
        // Changing the source image or sampling configuration recreates the
        // surface brush. Position, transform, and opacity update in place.
        self.snapshot.fill == desired.fill && self.snapshot.primitive == desired.primitive
    }

    fn update(
        &mut self,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        _image_surfaces: &mut ImageSurfaceCache,
    ) -> Result<()> {
        let image = image_brush(desired).expect("checked by is_image_node");
        let rect = desired.primitive.local_bounds();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80070057_u32 as i32),
                "image has an empty destination rectangle",
            ));
        }
        self.visual.SetSize(vector2(rect.width, rect.height))?;
        self.visual
            .SetTransformMatrix(image_visual_matrix(desired.transform, rect, island_bounds))?;
        self.visual.SetOpacity((desired.opacity * image.opacity).clamp(0.0, 1.0))?;
        self.snapshot = desired.clone();
        Ok(())
    }
}

struct ImageClipState {
    clip: CompositionGeometricClip,
    _geometry: GeometryState,
}

impl ImageClipState {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        primitive: &CompositionPrimitive,
    ) -> std::result::Result<Option<Self>, &'static str> {
        let bounds = primitive.local_bounds();
        let local = match primitive {
            CompositionPrimitive::Rectangle { .. } => return Ok(None),
            CompositionPrimitive::RoundedRectangle { radii, .. } => CompositionPrimitive::RoundedRectangle {
                rect: Rect { x: 0.0, y: 0.0, width: bounds.width, height: bounds.height },
                radii: *radii,
            },
            CompositionPrimitive::Ellipse { .. } => CompositionPrimitive::Ellipse {
                rect: Rect { x: 0.0, y: 0.0, width: bounds.width, height: bounds.height },
            },
            _ => return Err("image SpriteVisual primitive is not clip-compatible"),
        };
        let geometry = GeometryState::create(compositor, canvas_device, &local)?;
        let geometry_interface = geometry.as_geometry().map_err(|_| "image clip geometry cast failed")?;
        let clip = compositor
            .CreateGeometricClipWithGeometry(&geometry_interface)
            .map_err(|_| "CreateGeometricClipWithGeometry failed")?;
        Ok(Some(Self { clip, _geometry: geometry }))
    }
}

/// Win2D replay for image features that CompositionSurfaceBrush cannot express:
/// source rectangles and wrapped/mirrored texture brushes. The completed drawing
/// surface is still presented by a retained SpriteVisual, never by an XAML immediate-draw control.
struct DrawingSurfaceImageNode {
    visual: SpriteVisual,
    _brush: CompositionSurfaceBrush,
    _surface: CompositionDrawingSurface,
    // A CompositionDrawingSurface is backed by this device. Retain both for
    // the whole node lifetime; dropping the device leaves the surface unusable.
    _graphics_device: CompositionGraphicsDevice,
    rasterization_scale: f32,
    snapshot: DesiredCompositionNode,
}

impl DrawingSurfaceImageNode {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
    ) -> std::result::Result<Self, &'static str> {
        let rect = desired.primitive.local_bounds();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Err("image has an empty destination rectangle");
        }
        let graphics_device = CanvasComposition::CreateCompositionGraphicsDevice(compositor, canvas_device)
            .map_err(|_| "CreateCompositionGraphicsDevice failed")?;
        let surface = create_drawing_surface(
            &graphics_device,
            surface_size(rect, rasterization_scale),
        )
        .map_err(|_| "CreateDrawingSurface fallback failed")?;
        if let Err(error) = draw_image_surface(&surface, desired, rasterization_scale) {
            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                eprintln!("elwindui-winui3: CompositionDrawingSurface replay failed: {error:?}");
            }
            return Err("CompositionDrawingSurface replay failed");
        }
        let surface_interface: ICompositionSurface = surface
            .clone()
            .cast()
            .map_err(|_| "CompositionDrawingSurface cast failed")?;
        let brush = compositor
            .CreateSurfaceBrushWithSurface(&surface_interface)
            .map_err(|_| "fallback CreateSurfaceBrush failed")?;
        brush.SetStretch(CompositionStretch::Fill)
            .map_err(|_| "fallback SurfaceBrush stretch failed")?;
        let brush_interface: CompositionBrush = brush
            .clone()
            .cast()
            .map_err(|_| "fallback SurfaceBrush cast failed")?;
        let visual = compositor
            .CreateSpriteVisual()
            .map_err(|_| "fallback CreateSpriteVisual failed")?;
        visual
            .SetBrush(&brush_interface)
            .map_err(|_| "fallback SpriteVisual brush assignment failed")?;
        let mut state = Self {
            visual,
            _brush: brush,
            _surface: surface,
            _graphics_device: graphics_device,
            rasterization_scale,
            snapshot: desired.clone(),
        };
        state
            .update(desired, island_bounds, rasterization_scale)
            .map_err(|_| "fallback SpriteVisual update failed")?;
        Ok(state)
    }

    fn can_update(&self, desired: &DesiredCompositionNode, rasterization_scale: f32) -> bool {
        // A changed primitive changes the surface's pixel dimensions and must be
        // recreated. Transform/opacity updates remain retained properties.
        self.rasterization_scale == rasterization_scale
            && self.snapshot.fill == desired.fill
            && self.snapshot.primitive == desired.primitive
    }

    fn update(
        &mut self,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
    ) -> Result<()> {
        let image = image_brush(desired).expect("checked by is_image_node");
        let rect = desired.primitive.local_bounds();
        self.visual.SetSize(vector2(rect.width, rect.height))?;
        self.visual
            .SetTransformMatrix(image_visual_matrix(desired.transform, rect, island_bounds))?;
        self.visual.SetOpacity((desired.opacity * image.opacity).clamp(0.0, 1.0))?;
        self.rasterization_scale = rasterization_scale;
        self.snapshot = desired.clone();
        Ok(())
    }
}

/// A retained presentation node for complex vector documents. SVG scene lowering uses Win2D only
/// while this surface is created or invalidated; the visible object is a Composition SpriteVisual.
struct VectorSurfaceNode {
    visual: SpriteVisual,
    _brush: CompositionSurfaceBrush,
    _surface: CompositionDrawingSurface,
    _graphics_device: CompositionGraphicsDevice,
    rasterization_scale: f32,
    snapshot: DesiredCompositionNode,
}

impl VectorSurfaceNode {
    fn create(
        compositor: &Compositor,
        canvas_device: &CanvasDevice,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
    ) -> std::result::Result<Self, &'static str> {
        let rect = desired.primitive.local_bounds();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Err("vector image has an empty destination rectangle");
        }
        let graphics_device = CanvasComposition::CreateCompositionGraphicsDevice(compositor, canvas_device)
            .map_err(|_| "CreateCompositionGraphicsDevice failed")?;
        let surface = create_drawing_surface(
            &graphics_device,
            surface_size(rect, rasterization_scale),
        )
        .map_err(|_| "CreateDrawingSurface vector fallback failed")?;
        crate::inner::draw_vector_image_surface(&surface, desired, rasterization_scale)
            .map_err(|_| "CompositionDrawingSurface vector replay failed")?;
        let surface_interface: ICompositionSurface = surface
            .clone()
            .cast()
            .map_err(|_| "vector CompositionDrawingSurface cast failed")?;
        let brush = compositor
            .CreateSurfaceBrushWithSurface(&surface_interface)
            .map_err(|_| "vector fallback CreateSurfaceBrush failed")?;
        brush.SetStretch(CompositionStretch::Fill)
            .map_err(|_| "vector fallback SurfaceBrush stretch failed")?;
        let brush_interface: CompositionBrush = brush
            .clone()
            .cast()
            .map_err(|_| "vector fallback SurfaceBrush cast failed")?;
        let visual = compositor
            .CreateSpriteVisual()
            .map_err(|_| "vector fallback CreateSpriteVisual failed")?;
        visual
            .SetBrush(&brush_interface)
            .map_err(|_| "vector fallback SpriteVisual brush assignment failed")?;
        let mut state = Self {
            visual,
            _brush: brush,
            _surface: surface,
            _graphics_device: graphics_device,
            rasterization_scale,
            snapshot: desired.clone(),
        };
        state
            .update(desired, island_bounds, rasterization_scale)
            .map_err(|_| "vector fallback SpriteVisual update failed")?;
        Ok(state)
    }

    fn can_update(&self, desired: &DesiredCompositionNode, rasterization_scale: f32) -> bool {
        // A changed document, source rectangle, or destination changes the raster contents or
        // its pixel extent, so reconciliation recreates the surface. Transform and opacity remain
        // retained visual properties.
        self.rasterization_scale == rasterization_scale && self.snapshot.primitive == desired.primitive
    }

    fn update(
        &mut self,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
    ) -> Result<()> {
        let rect = desired.primitive.local_bounds();
        self.visual.SetSize(vector2(rect.width, rect.height))?;
        self.visual
            .SetTransformMatrix(image_visual_matrix(desired.transform, rect, island_bounds))?;
        self.visual.SetOpacity(desired.opacity.clamp(0.0, 1.0))?;
        self.rasterization_scale = rasterization_scale;
        self.snapshot = desired.clone();
        Ok(())
    }
}

fn draw_image_surface(
    surface: &CompositionDrawingSurface,
    desired: &DesiredCompositionNode,
    rasterization_scale: f32,
) -> Result<()> {
    let image = image_brush(desired).expect("checked by requires_drawing_surface");
    let rect = desired.primitive.local_bounds();
    let session = CanvasComposition::CreateDrawingSession(surface)?;
    session.Clear(WinColor { A: 0, R: 0, G: 0, B: 0 })?;
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
            CompositionPrimitive::Rectangle { .. } => session.FillRectangleWithBrush(win_rect(local), &brush)?,
            CompositionPrimitive::RoundedRectangle { radii, .. } => {
                let radius = (radii.top_left + radii.top_right + radii.bottom_right + radii.bottom_left) / 4.0;
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

fn rasterization_scale(canvas: &Canvas) -> Result<f32> {
    let element: UIElement = canvas.clone().cast()?;
    let scale = element.RasterizationScale()? as f32;
    Ok(if scale.is_finite() && scale > 0.0 { scale } else { 1.0 })
}

fn surface_size(rect: Rect, rasterization_scale: f32) -> WinSize {
    let scale = rasterization_scale.max(0.01);
    WinSize {
        Width: (rect.width * scale).ceil().max(1.0),
        Height: (rect.height * scale).ceil().max(1.0),
    }
}

fn raster_scale_matrix(rasterization_scale: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: rasterization_scale,
        M12: 0.0,
        M21: 0.0,
        M22: rasterization_scale,
        M31: 0.0,
        M32: 0.0,
    }
}

fn canvas_bitmap(creator: &ICanvasResourceCreator, image: &Image) -> Result<CanvasBitmap> {
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

fn fitted_image_rect(
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

fn win_rect(rect: Rect) -> WinRect {
    WinRect {
        X: rect.x,
        Y: rect.y,
        Width: rect.width,
        Height: rect.height,
    }
}

fn image_brush(desired: &DesiredCompositionNode) -> Option<&elwindui_core::graphics::ImageBrush> {
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

fn is_image_node(desired: &DesiredCompositionNode) -> bool {
    image_brush(desired).is_some()
}

fn requires_drawing_surface(desired: &DesiredCompositionNode) -> bool {
    let Some(image) = image_brush(desired) else {
        return false;
    };
    image.source_rect.is_some() || image.tile_mode != TileMode::None
}

fn apply_image_brush(
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

#[cfg_attr(rust_analyzer, allow(dead_code))]
enum GeometryState {
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
fn composition_path_geometry(
    compositor: &Compositor,
    canvas_geometry: &CanvasGeometry,
) -> Result<(CompositionPath, CompositionPathGeometry)> {
    let source: windows::Graphics::IGeometrySource2D = canvas_geometry.clone().cast()?;
    let path: CompositionPath = CompositionPath::Create(&source)?;
    let geometry: CompositionPathGeometry = compositor.CreatePathGeometryWithPath(&path)?;
    Ok((path, geometry))
}

struct ClipState {
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

fn canvas_clip_geometry(
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

enum BrushState {
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

fn apply_gradient_common(
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

fn apply_stroke(shape: &CompositionSpriteShape, stroke: &StrokeStyle) -> Result<()> {
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

fn create_canvas_path(
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

fn uniform_radius(radii: CornerRadius) -> Option<f32> {
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

fn stroke_cap(cap: LineCap) -> CompositionStrokeCap {
    match cap {
        LineCap::Butt => CompositionStrokeCap::Flat,
        LineCap::Round => CompositionStrokeCap::Round,
        LineCap::Square => CompositionStrokeCap::Square,
    }
}

fn color_with_opacity(
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

fn island_local_matrix(transform: AffineTransform, island: Rect) -> Matrix3x2 {
    Matrix3x2 {
        M11: transform.m11,
        M12: transform.m12,
        M21: transform.m21,
        M22: transform.m22,
        M31: transform.dx - island.x,
        M32: transform.dy - island.y,
    }
}

fn matrix(transform: AffineTransform) -> Matrix3x2 {
    Matrix3x2 {
        M11: transform.m11,
        M12: transform.m12,
        M21: transform.m21,
        M22: transform.m22,
        M31: transform.dx,
        M32: transform.dy,
    }
}

fn image_visual_matrix(transform: AffineTransform, rect: Rect, island: Rect) -> Matrix4x4 {
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

fn vector2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn transformed_bounds(rect: Rect, transform: AffineTransform) -> Rect {
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

fn union_rect(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > left && bottom > top).then_some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformed_bounds_include_rotation() {
        let bounds = transformed_bounds(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
            AffineTransform::rotation(std::f32::consts::FRAC_PI_2),
        );
        assert!((bounds.x + 10.0).abs() < 0.001);
        assert!(bounds.y.abs() < 0.001);
        assert!((bounds.width - 10.0).abs() < 0.001);
        assert!((bounds.height - 20.0).abs() < 0.001);
    }

    #[test]
    fn island_id_is_first_stable_node_id() {
        let nodes = vec![DesiredCompositionNode {
            id: (42, 3),
            primitive: CompositionPrimitive::Rectangle {
                rect: Rect {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                },
            },
            fill: None,
            stroke: None,
            transform: AffineTransform::IDENTITY,
            opacity: 1.0,
        }];
        let island =
            DesiredCompositionIsland::from_nodes(nodes, Vec::new()).expect("non-empty island");
        assert_eq!(island.id, (42, 3));
        assert_eq!(
            island.bounds,
            Rect {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
            }
        );
    }

    #[test]
    fn nested_clips_intersect_island_bounds() {
        let nodes = vec![DesiredCompositionNode {
            id: (7, 1),
            primitive: CompositionPrimitive::Rectangle {
                rect: Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
            },
            fill: None,
            stroke: None,
            transform: AffineTransform::IDENTITY,
            opacity: 1.0,
        }];
        let clips = vec![
            CompositionClipSpec::Rect {
                rect: Rect { x: 10.0, y: 10.0, width: 80.0, height: 80.0 },
                transform: AffineTransform::IDENTITY,
            },
            CompositionClipSpec::Rect {
                rect: Rect { x: 25.0, y: 20.0, width: 50.0, height: 60.0 },
                transform: AffineTransform::IDENTITY,
            },
        ];
        let island = DesiredCompositionIsland::from_nodes(nodes, clips).expect("clipped island");
        assert_eq!(
            island.bounds,
            Rect { x: 25.0, y: 20.0, width: 50.0, height: 60.0 },
        );
        assert_eq!(island.clips.len(), 2);
    }

    #[test]
    fn non_uniform_radii_require_fallback() {
        assert_eq!(
            uniform_radius(CornerRadius {
                top_left: 2.0,
                top_right: 2.0,
                bottom_right: 3.0,
                bottom_left: 2.0,
            }),
            None
        );
    }

    #[test]
    fn drawing_surface_size_uses_physical_pixels() {
        let size = surface_size(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 101.0,
                height: 20.0,
            },
            1.25,
        );
        assert_eq!(size.Width, 127.0);
        assert_eq!(size.Height, 25.0);
    }

    #[test]
    fn raster_scale_matrix_scales_coordinates() {
        let matrix = raster_scale_matrix(1.5);
        assert_eq!(matrix.M11, 1.5);
        assert_eq!(matrix.M22, 1.5);
        assert_eq!(matrix.M31, 0.0);
        assert_eq!(matrix.M32, 0.0);
    }

    #[test]
    fn shape_runs_preserve_non_shape_z_order_boundaries() {
        let runs = shape_run_descriptors([
            Some((1_u32, 1.0)),
            Some((2_u32, 1.0)),
            None,
            Some((3_u32, 1.0)),
            Some((4_u32, 0.5)),
            Some((5_u32, 0.5)),
        ]);
        assert_eq!(
            runs,
            vec![(vec![1, 2], 1.0), (vec![3], 1.0), (vec![4, 5], 0.5)]
        );
    }

    #[test]
    fn drawing_surface_sizes_cover_supported_dpi_scales() {
        let rect = Rect { x: 0.0, y: 0.0, width: 80.0, height: 40.0 };
        for (scale, width, height) in [
            (1.0, 80.0, 40.0),
            (1.25, 100.0, 50.0),
            (1.5, 120.0, 60.0),
            (2.0, 160.0, 80.0),
        ] {
            let size = surface_size(rect, scale);
            assert_eq!((size.Width, size.Height), (width, height));
        }
    }
}
