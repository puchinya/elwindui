//! The abstract base for the self-drawn shape primitives (`Rectangle`/`Ellipse`).

use super::*;

/// `Rectangle`/`Ellipse`. A pure leaf, like `TextBlock` — no children of its own (matching real
/// WinUI3's `Shape`, which likewise has no `Children`/content property; see docs/design/gui_framework_design.md
/// §5.2), so its natural size is just its own drawn bounds.
/// `Shape`'s own class trait (docs/design/gui_framework_design.md §5.1); `Shape` has no further
/// DSL-level subclass today.
#[elwindui_macros::class(inherits = crate::ui::UIElement, abstract_class)]
#[prop(fill: Option<crate::graphics::Brush>)]
#[prop(stroke: Option<crate::graphics::Brush>)]
#[prop(stroke_width: Option<f32>)]
pub struct Shape {
    pub fill: RefCell<Option<Brush>>,
    pub stroke: RefCell<Option<Brush>>,
    pub stroke_width: Cell<f32>,
}

#[elwindui_macros::class]
impl Shape {
    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        Size {
            width: 0.0,
            height: 0.0,
        }
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        final_size
    }
    /// A shape with neither `fill` nor `stroke` set paints nothing, so it isn't hit-testable
    /// either (WinUI3/WPF's `Shape.Fill == null` rule) — see
    /// `UIElement::hit_test_content`'s own doc comment. A simplification vs. real path/stroke-
    /// aware hit-testing: this is a whole-bounding-rect yes/no, not per-pixel.
    #[overrides]
    fn hit_test_content(&self) -> bool {
        self.fill.borrow().is_some() || self.stroke.borrow().is_some()
    }
    fn set_fill(&self, fill: Option<Brush>) {
        *self.fill.borrow_mut() = fill;
        self.invalidate();
    }
    /// Removes the explicit fill.
    fn clear_fill(&self) {
        self.set_fill(None);
    }
    fn set_stroke(&self, stroke: Option<Brush>) {
        *self.stroke.borrow_mut() = stroke;
        self.invalidate();
    }
    /// Removes the explicit stroke.
    fn clear_stroke(&self) {
        self.set_stroke(None);
    }
    fn set_stroke_width(&self, stroke_width: f32) {
        self.stroke_width.set(stroke_width);
        self.invalidate();
    }
    /// Restores zero stroke width.
    fn clear_stroke_width(&self) {
        self.set_stroke_width(0.0);
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            fill: RefCell::new(None),
            stroke: RefCell::new(None),
            stroke_width: Cell::new(0.0),
        }
    }
}
