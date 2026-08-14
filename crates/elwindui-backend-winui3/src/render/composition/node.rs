//! Per-node retained state: one struct per `CompositionPrimitive` shape, holding the visuals
//! and brushes that survive between render passes.

use super::cache::*;
use super::geometry::*;
use super::*;

use crate::bindings::Microsoft::Graphics::Canvas::CanvasDevice;
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::UI::Composition::{
    CompositionBrush, CompositionClip, CompositionDrawingSurface, CompositionGeometricClip,
    CompositionGraphicsDevice, CompositionShape, CompositionSpriteShape, CompositionStretch,
    CompositionSurfaceBrush, Compositor, ICompositionSurface, ShapeVisual, SpriteVisual, Visual,
};
use crate::bindings::Microsoft::UI::Xaml::Media::LoadedImageSurface;
use elwindui_core::base::Rect;
use std::collections::HashMap;
use windows::core::{Interface, Result};

pub(crate) enum CompositionNodeState {
    Shape(ShapeNodeState),
    Image(ImageNodeState),
    VectorSurface(VectorSurfaceNode),
}

impl CompositionNodeState {
    pub(crate) fn create(
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

    pub(crate) fn can_update(
        &self,
        desired: &DesiredCompositionNode,
        rasterization_scale: f32,
    ) -> bool {
        match self {
            Self::Shape(node) => !is_image_node(desired) && node.can_update(desired),
            Self::Image(node) => {
                is_image_node(desired) && node.can_update(desired, rasterization_scale)
            }
            Self::VectorSurface(node) => node.can_update(desired, rasterization_scale),
        }
    }

    pub(crate) fn update(
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

    pub(crate) fn visual(&self) -> Result<Visual> {
        match self {
            Self::Shape(_) => unreachable!("shape nodes are owned by ShapeRunState"),
            Self::Image(node) => node.visual(),
            Self::VectorSurface(node) => node.visual.clone().cast(),
        }
    }
}

pub(crate) struct ShapeNodeState {
    shape: CompositionSpriteShape,
    _geometry: GeometryState,
    fill: Option<BrushState>,
    stroke: Option<BrushState>,
    pub(crate) snapshot: DesiredCompositionNode,
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
        let base_geometry = geometry.as_geometry().map_err(|_| "geometry cast failed")?;
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
        if (self.fill.is_none() && desired.fill.is_some()) || self.snapshot.fill != desired.fill {
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
pub(crate) enum ImageNodeState {
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
            SpriteImageNode::create(
                compositor,
                canvas_device,
                desired,
                island_bounds,
                image_surfaces,
            )
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

pub(crate) struct SpriteImageNode {
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
            let clip_interface: CompositionClip = clip
                .clip
                .clone()
                .cast()
                .map_err(|_| "SpriteVisual image clip cast failed")?;
            visual
                .SetClip(&clip_interface)
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
        self.visual.SetTransformMatrix(image_visual_matrix(
            desired.transform,
            rect,
            island_bounds,
        ))?;
        self.visual
            .SetOpacity((desired.opacity * image.opacity).clamp(0.0, 1.0))?;
        self.snapshot = desired.clone();
        Ok(())
    }
}

pub(crate) struct ImageClipState {
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
            CompositionPrimitive::RoundedRectangle { radii, .. } => {
                CompositionPrimitive::RoundedRectangle {
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        width: bounds.width,
                        height: bounds.height,
                    },
                    radii: *radii,
                }
            }
            CompositionPrimitive::Ellipse { .. } => CompositionPrimitive::Ellipse {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: bounds.width,
                    height: bounds.height,
                },
            },
            _ => return Err("image SpriteVisual primitive is not clip-compatible"),
        };
        let geometry = GeometryState::create(compositor, canvas_device, &local)?;
        let geometry_interface = geometry
            .as_geometry()
            .map_err(|_| "image clip geometry cast failed")?;
        let clip = compositor
            .CreateGeometricClipWithGeometry(&geometry_interface)
            .map_err(|_| "CreateGeometricClipWithGeometry failed")?;
        Ok(Some(Self {
            clip,
            _geometry: geometry,
        }))
    }
}

/// Win2D replay for image features that CompositionSurfaceBrush cannot express:
/// source rectangles and wrapped/mirrored texture brushes. The completed drawing
/// surface is still presented by a retained SpriteVisual, never by an XAML immediate-draw control.
pub(crate) struct DrawingSurfaceImageNode {
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
        let graphics_device =
            CanvasComposition::CreateCompositionGraphicsDevice(compositor, canvas_device)
                .map_err(|_| "CreateCompositionGraphicsDevice failed")?;
        let surface =
            create_drawing_surface(&graphics_device, surface_size(rect, rasterization_scale))
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
        brush
            .SetStretch(CompositionStretch::Fill)
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
        self.visual.SetTransformMatrix(image_visual_matrix(
            desired.transform,
            rect,
            island_bounds,
        ))?;
        self.visual
            .SetOpacity((desired.opacity * image.opacity).clamp(0.0, 1.0))?;
        self.rasterization_scale = rasterization_scale;
        self.snapshot = desired.clone();
        Ok(())
    }
}

/// A retained presentation node for complex vector documents. SVG scene lowering uses Win2D only
/// while this surface is created or invalidated; the visible object is a Composition SpriteVisual.
pub(crate) struct VectorSurfaceNode {
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
        let graphics_device =
            CanvasComposition::CreateCompositionGraphicsDevice(compositor, canvas_device)
                .map_err(|_| "CreateCompositionGraphicsDevice failed")?;
        let surface =
            create_drawing_surface(&graphics_device, surface_size(rect, rasterization_scale))
                .map_err(|_| "CreateDrawingSurface vector fallback failed")?;
        crate::render::draw_vector_image_surface(&surface, desired, rasterization_scale)
            .map_err(|_| "CompositionDrawingSurface vector replay failed")?;
        let surface_interface: ICompositionSurface = surface
            .clone()
            .cast()
            .map_err(|_| "vector CompositionDrawingSurface cast failed")?;
        let brush = compositor
            .CreateSurfaceBrushWithSurface(&surface_interface)
            .map_err(|_| "vector fallback CreateSurfaceBrush failed")?;
        brush
            .SetStretch(CompositionStretch::Fill)
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
        self.rasterization_scale == rasterization_scale
            && self.snapshot.primitive == desired.primitive
    }

    fn update(
        &mut self,
        desired: &DesiredCompositionNode,
        island_bounds: Rect,
        rasterization_scale: f32,
    ) -> Result<()> {
        let rect = desired.primitive.local_bounds();
        self.visual.SetSize(vector2(rect.width, rect.height))?;
        self.visual.SetTransformMatrix(image_visual_matrix(
            desired.transform,
            rect,
            island_bounds,
        ))?;
        self.visual.SetOpacity(desired.opacity.clamp(0.0, 1.0))?;
        self.rasterization_scale = rasterization_scale;
        self.snapshot = desired.clone();
        Ok(())
    }
}

/// Groups only adjacent shapes that can share one ShapeVisual. Any non-shape
/// visual is an ordering boundary, as is a different opacity because opacity
/// belongs to the ShapeVisual rather than to individual brushes.
pub(crate) fn shape_run_descriptors<T: Copy>(
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

pub(crate) struct ShapeRunState {
    pub(crate) visual: ShapeVisual,
    pub(crate) node_ids: Vec<RenderNodeId>,
    pub(crate) opacity: f32,
}

impl ShapeRunState {
    pub(crate) fn create(
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
            let CompositionNodeState::Shape(node) = nodes.get(id).expect("shape run node exists")
            else {
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
