//! Retained Microsoft.UI.Composition renderer for custom-drawn RenderCommands.
//!
//! XAML remains responsible for ordering composition islands against native controls. Each island
//! owns one empty XAML Canvas with a child ContainerVisual. The island's figures are retained as
//! CompositionSpriteShapes in a shared ShapeVisual and reconciled by stable RenderNodeId.


mod cache;
mod geometry;
mod node;

use cache::*;
use geometry::*;
use node::*;

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
            bounds = bounds.union(node.world_bounds());
        }
        for clip in &clips {
            bounds = bounds.intersect(clip.world_bounds())?;
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

pub(crate) struct CompositionIslandState {
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
