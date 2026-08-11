//! `elwindui::ui::HorizontalLayout` — stacks children left to right.

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
        // See `VerticalLayout::measure_override`'s own doc comment: every child is still measured,
        // but only participating children's sizes feed `stack_natural_size` — otherwise a zero-
        // sized non-participating child would still strand one `spacing` gap.
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .filter_map(|c| {
                c.measure(child_available);
                c.participates_in_layout()
                    .then(|| c.measured_size().unwrap_or_default())
            })
            .collect();
        stack_natural_size(Orientation::Horizontal, self.spacing.get(), &child_sizes)
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let children = self.visual_children();
        let child_sizes: Vec<Size> = children
            .iter()
            .filter(|c| c.participates_in_layout())
            .map(|c| c.measured_size().unwrap_or_default())
            .collect();
        let child_rects = stack_arrange(
            final_size,
            Orientation::Horizontal,
            self.spacing.get(),
            &child_sizes,
        );
        // See `VerticalLayout::arrange_override`'s own doc comment: a non-participating child
        // still gets `arrange` called on it, with a placeholder rect it never reads.
        let mut rects = child_rects.into_iter();
        for child in children.iter() {
            if child.participates_in_layout() {
                child.arrange(rects.next().expect(
                    "stack_arrange returns exactly one rect per participating child size",
                ));
            } else {
                child.arrange(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                });
            }
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
    fn horizontal_layout_measures_children_with_unconstrained_main_axis() {
        let probe = MeasureProbe::new(size(20.0, 10.0));
        let child: Rc<dyn UIElementExt> = probe.clone();
        let root = HorizontalLayout::new();
        root.children().add(child);
        root.measure(size(50.0, 200.0));
        let last = probe.last_available();
        assert_eq!(
            last.height, 200.0,
            "cross axis (height) must stay constrained to the container's own available height"
        );
        assert!(
            last.width.is_infinite() && last.width > 0.0,
            "main axis (width) must be unconstrained, got {:?}",
            last.width
        );
    }
}
