//! `builtin::Ellipse` — self-drawn ellipse primitive.

use super::*;

/// `builtin::Ellipse`(docs/specs/builtins_spec.md 付録G/N)。`Rectangle`の doc comment 参照。
#[elwindui_macros::class(inherits = crate::ui::Shape, sealed)]
pub struct Ellipse {
    stroke_width: Option<f32>,
}

#[elwindui_macros::class]
impl Ellipse {
    fn fill(&self) -> Option<Brush> {
        self.base.fill.borrow().clone()
    }
    fn stroke(&self) -> Option<Brush> {
        self.base.stroke.borrow().clone()
    }
    fn stroke_width(&self) -> Option<f32> {
        self.stroke_width.clone()
    }
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: self.arranged_width().unwrap_or(0.0),
            height: self.arranged_height().unwrap_or(0.0),
        };
        if let Some(fill) = self.base.fill.borrow().as_ref() {
            context.fill_ellipse(rect, fill);
        }
        if let Some(stroke) = self.base.stroke.borrow().as_ref() {
            let style = StrokeStyle {
                width: self.base.stroke_width.get(),
                ..Default::default()
            };
            context.stroke_ellipse(rect, stroke, &style);
        }
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // Zero-arg — see `Rectangle`'s own `construct` doc comment for why. `stroke_width` (this own
    // field, distinct from `base.stroke_width`) has no setter of its own and nothing reads it
    // internally (`render` above uses `self.base.stroke_width`, already configured through the
    // inherited `Shape::set_stroke_width`) — it only ever echoed back whatever `construct` was
    // originally given, so defaulting it to `None` here changes no observable behavior.
    fn construct() -> Self {
        Self {
            base: Shape::construct(),
            stroke_width: None,
        }
    }
}
