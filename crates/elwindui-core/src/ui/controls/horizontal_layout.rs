//! `builtin::HorizontalLayout` — stacks children left to right.

use super::*;

/// `HorizontalLayout`'s own class trait (docs/design/gui_framework_design.md §5.1). `spacing` lives here
/// (not on `Layout`) — see `VerticalLayout`'s own doc comment.
#[elwindui_macros::class(inherits = crate::ui::Layout)]
#[content(children)]
#[prop(spacing: Option<f32>)]
pub struct HorizontalLayout {
    spacing: Cell<f32>,
}

#[elwindui_macros::class]
impl HorizontalLayout {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        // Main axis (width) is unconstrained — see `VerticalLayout::measure_override`'s own doc
        // comment for why. Cross axis (height) is still constrained.
        let child_available = Size {
            width: f32::INFINITY,
            height: available.height,
        };
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .map(|c| {
                c.measure(child_available);
                c.measured_size().unwrap_or_default()
            })
            .collect();
        stack_natural_size(Orientation::Horizontal, self.spacing.get(), &child_sizes)
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
            Orientation::Horizontal,
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
