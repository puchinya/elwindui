//! `elwindui::ui::VerticalLayout` — stacks children top to bottom.

use super::*;

/// `VerticalLayout`'s own class trait (docs/design/runtime/ui_tree_design.md). `spacing` lives here
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
        // Every child is still measured (a non-participating child's own `measure` short-circuits
        // to zero-size internally — see `UIElementExt::participates_in_layout`'s own doc comment —
        // but it must still run so `measured_size()` isn't left stale). Only *participating*
        // children's sizes feed `stack_natural_size`, though: including a zero-sized non-
        // participating child there would still add one `spacing` gap for it, stranding a visible
        // gap where a Collapsed/suppressed child used to be.
        let child_sizes: Vec<Size> = self
            .visual_children()
            .iter()
            .filter_map(|c| {
                c.measure(child_available);
                c.participates_in_layout()
                    .then(|| c.measured_size().unwrap_or_default())
            })
            .collect();
        stack_natural_size(Orientation::Vertical, self.spacing.get(), &child_sizes)
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
            Orientation::Vertical,
            self.spacing.get(),
            &child_sizes,
        );
        // Zipped back up against only the participating children, in the same order `child_sizes`
        // collected them in — a non-participating child still gets `arrange` called on it (so its
        // own `arranged_*` reset to zero rather than staying stale), just with a placeholder rect
        // it ignores (`UIElement::arrange`'s non-participating branch returns before ever reading
        // `final_rect`).
        let mut rects = child_rects.into_iter();
        for child in children.iter() {
            if child.participates_in_layout() {
                child.arrange(
                    rects.next().expect(
                        "stack_arrange returns exactly one rect per participating child size",
                    ),
                );
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
