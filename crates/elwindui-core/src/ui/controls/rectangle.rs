//! `builtin::Rectangle` — self-drawn rectangle primitive.

use super::*;

/// `builtin::Rectangle`(docs/specs/builtins_spec.md 付録G/N)。バックエンド非依存な合成 builtin
/// としてここに手書きする。`#[ancestor]`(`elwindui_macros::class`の doc comment 参照)で`Shape`
/// 自身の共通描画メソッドを`base`委譲として登録している。
#[elwindui_macros::class(inherits = crate::ui::Shape, sealed)]
#[prop(corner_radius: Option<f32>)]
pub struct Rectangle {
    stroke_width: Option<f32>,
    corner_radius: Cell<Option<f32>>,
}

#[elwindui_macros::class]
impl Rectangle {
    fn fill(&self) -> Option<Brush> {
        self.base.fill.borrow().clone()
    }
    fn stroke(&self) -> Option<Brush> {
        self.base.stroke.borrow().clone()
    }
    fn stroke_width(&self) -> Option<f32> {
        self.stroke_width.clone()
    }
    fn corner_radius(&self) -> Option<f32> {
        self.corner_radius.get()
    }
    /// Sets the radius used for all four corners.
    fn set_corner_radius(&self, corner_radius: f32) {
        self.corner_radius.set(Some(corner_radius));
        self.invalidate();
    }
    /// Restores the platform-neutral square-corner default.
    fn clear_corner_radius(&self) {
        self.corner_radius.set(None);
        self.invalidate();
    }
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: self.arranged_width().unwrap_or(0.0),
            height: self.arranged_height().unwrap_or(0.0),
        };
        let radii = CornerRadius::uniform(self.corner_radius.get().unwrap_or(0.0));
        if let Some(fill) = self.base.fill.borrow().as_ref() {
            context.fill_rounded_rect(rect, radii, fill);
        }
        if let Some(stroke) = self.base.stroke.borrow().as_ref() {
            let style = StrokeStyle {
                width: self.base.stroke_width.get(),
                ..Default::default()
            };
            context.stroke_rounded_rect(rect, radii, stroke, &style);
        }
    }
    #[inherent]
    pub fn into_node(self: Rc<Self>) -> Rc<dyn UIElementExt> {
        self
    }
    // The bare (not `Rc`-wrapped) value `#[class]`'s auto-generated `new` wraps — also what a future
    // `component X inherits Rectangle` would embed unwrapped as its own `base` field, mirroring
    // `Control`/`Shape`'s own `construct` (`Rectangle` is `#[sealed]` today, so nothing actually
    // reaches this via that path yet, but the shape stays consistent with every other builtin).
    //
    // Zero-arg, like every other builtin's `construct()` (`#[class]`'s auto-generated `new()`
    // always mirrors this signature verbatim — see `elwindui_macros::class`'s own doc comment on
    // `auto_new` — so a non-zero-arg `construct()` here means a non-zero-arg `new()`, which
    // `elwindui-codegen`'s `emit_external_construction` can never supply: with no `TypeInfo` to
    // consult (the builtin shape source was removed, Refs #14), it always calls `Type::new()` and configures
    // everything through the already-existing `set_fill`/`set_stroke`/`set_stroke_width`/
    // `set_corner_radius` setters instead).
    fn construct() -> Self {
        Self {
            base: Shape::construct(),
            stroke_width: None,
            corner_radius: Cell::new(None),
        }
    }
}
