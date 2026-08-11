//! `elwindui::ui::Image` — self-drawn raster/vector image, plus the `Stretch` -> `ImageFit` mapping it arranges with.

use super::*;

/// `Stretch` -> `ImageFit` — the same four cases under a different name per surface
/// (`ImageBrush`/`Image` use WinUI3's `Stretch` vocabulary, `ImageDrawOptions`/
/// `VectorImageDrawOptions` use `Fill`/`Contain`/`Cover`/`None`); each backend's own AppKit
/// `stretch_to_image_fit` duplicates this identically rather than being reused, since
/// `elwindui-core` cannot depend on a backend crate (SVG読み込み・ベクター描画対応 実装指示書§24).
pub(crate) fn stretch_to_image_fit(stretch: Stretch) -> ImageFit {
    match stretch {
        Stretch::None => ImageFit::None,
        Stretch::Fill => ImageFit::Fill,
        Stretch::Uniform => ImageFit::Contain,
        Stretch::UniformToFill => ImageFit::Cover,
    }
}

/// `elwindui::ui::Image`(SVG読み込み・ベクター描画対応 実装指示書§24) — a self-drawing leaf like `Shape`,
/// not backed by any native widget on any backend: `render()` calls `RenderContext::draw_image`/
/// `draw_vector_image` directly depending on which `ImageSource` variant `source` holds, so no
/// per-backend construction code is needed at all (unlike `NativeControl`-family builtins).
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
#[prop(source: Option<crate::graphics::ImageSource>)]
#[prop(stretch: Option<crate::graphics::Stretch>)]
#[prop(rasterize: Option<crate::graphics::VectorRasterizeMode>)]
pub struct Image {
    source: RefCell<Option<ImageSource>>,
    stretch: Cell<Stretch>,
    rasterize: Cell<VectorRasterizeMode>,
}

#[elwindui_macros::class]
impl Image {
    fn source(&self) -> Option<ImageSource> {
        self.source.borrow().clone()
    }
    fn stretch(&self) -> Stretch {
        self.stretch.get()
    }
    fn rasterize(&self) -> VectorRasterizeMode {
        self.rasterize.get()
    }
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        match &*self.source.borrow() {
            Some(ImageSource::Raster(image)) => image
                .pixel_size()
                .map(|(width, height)| Size {
                    width: width as f32,
                    height: height as f32,
                })
                .unwrap_or_default(),
            Some(ImageSource::Vector(vector)) => vector.intrinsic_size(),
            None => Size::default(),
        }
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
    }
    /// Bounding-box precision only, matching `Shape::hit_test_content`'s own simplification —
    /// see that method's doc comment and 実装指示書 discussion for why a path-shaped or
    /// alpha-aware hit test isn't attempted here (it would need `UIElement::hit_test_content`'s
    /// own signature extended to take the hit point, a wider change than this feature's scope).
    #[overrides]
    fn hit_test_content(&self) -> bool {
        self.source.borrow().is_some()
    }
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: self.arranged_width().unwrap_or(0.0),
            height: self.arranged_height().unwrap_or(0.0),
        };
        let fit = stretch_to_image_fit(self.stretch.get());
        match &*self.source.borrow() {
            Some(ImageSource::Raster(image)) => {
                context.draw_image(
                    image,
                    rect,
                    None,
                    ImageDrawOptions {
                        fit,
                        ..Default::default()
                    },
                );
            }
            Some(ImageSource::Vector(vector)) => {
                context.draw_vector_image(
                    vector,
                    rect,
                    None,
                    VectorImageDrawOptions {
                        fit,
                        rasterize: self.rasterize.get(),
                        ..Default::default()
                    },
                );
            }
            None => {}
        }
    }
    fn set_source(&self, source: Option<ImageSource>) {
        *self.source.borrow_mut() = source;
        self.invalidate_measure();
    }
    fn set_stretch(&self, stretch: Stretch) {
        self.stretch.set(stretch);
        self.invalidate();
    }
    fn set_rasterize(&self, rasterize: VectorRasterizeMode) {
        self.rasterize.set(rasterize);
        self.invalidate();
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // Zero-arg — see `Rectangle`'s own `construct` doc comment for why.
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            source: RefCell::new(None),
            stretch: Cell::new(Stretch::Uniform),
            rasterize: Cell::new(VectorRasterizeMode::default()),
        }
    }
}
