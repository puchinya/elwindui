//! `builtin::VerticalLayout` — stacks children top to bottom.

use super::*;

/// `VerticalLayout`'s own class trait (docs/design/gui_framework_design.md §5.1). `spacing` lives here
/// (not on `Layout`) since it's meaningless to `Grid`, `Layout`'s other concrete subclass — see
/// `Layout`'s own doc comment.
#[elwindui_macros::class(inherits = crate::ui::Layout)]
#[content(children)]
#[prop(spacing: Option<f32>)]
pub struct VerticalLayout {
    spacing: Cell<f32>,
}

#[elwindui_macros::class]
impl VerticalLayout {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        // Main axis (height) is unconstrained: a content-sized `VerticalLayout` must size itself
        // from each child's own natural height, not from whatever finite `available` its own
        // parent happened to hand it — passing `available.height` straight through here would let
        // a large parent silently inflate every child's measured height. Cross axis (width) is
        // still constrained, since `stack_arrange` gives every child the container's full cross
        // extent as its slot.
        let child_available = Size {
            width: available.width,
            height: f32::INFINITY,
        };
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| {
                c.measure(child_available);
                c.measured_size().unwrap_or_default()
            })
            .collect();
        stack_natural_size(Orientation::Vertical, self.spacing.get(), &child_sizes)
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();
        let child_rects = stack_arrange(
            final_size,
            Orientation::Vertical,
            self.spacing.get(),
            &child_sizes,
        );
        for (child, rect) in self.visual_children().iter().zip(child_rects) {
            child.arrange(rect);
        }
        final_size
    }
    fn set_spacing(&self, spacing: f32) {
        self.spacing.set(spacing);
        self.invalidate_measure();
    }
    /// Restores zero spacing.
    fn clear_spacing(&self) {
        self.set_spacing(0.0);
    }
    fn construct() -> Self {
        Self {
            base: Layout::construct(),
            spacing: Cell::new(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::*;

    #[test]
    fn vertical_layout_measures_children_with_unconstrained_main_axis() {
        // A content-sized `VerticalLayout` must size itself from each child's own natural height,
        // not from whatever finite `available` its own parent happened to hand it — passing
        // `available.height` straight through to children would let a large parent silently
        // inflate every child's measured height.
        let probe = MeasureProbe::new(size(10.0, 20.0));
        let child: Rc<dyn UIElementExt> = probe.clone();
        let root = VerticalLayout::new();
        root.children().add(child);
        root.measure(size(200.0, 50.0));
        let last = probe.last_available();
        assert_eq!(
            last.width, 200.0,
            "cross axis (width) must stay constrained to the container's own available width"
        );
        assert!(
            last.height.is_infinite() && last.height > 0.0,
            "main axis (height) must be unconstrained, got {:?}",
            last.height
        );
    }
}
