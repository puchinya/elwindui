//! Full-span interactive splitter layer for one realized docking split.

use crate::core::base::{Rect, Size, Vector};
use crate::core::layout::{GridLength, HorizontalAlignment, VerticalAlignment};
use crate::core::ui::{ControlExt, Grid, GridExt, LayoutExt, UIElementExt, VisualTransform};
use crate::runtime::metrics::SPLITTER_HIT_SIZE;
use crate::snapshot::SnapshotOrientation;
use elwindui_custom_controls::CustomGridSplitter;
use std::rc::Rc;

/// Arranges a split's track grid and places each resize handle over the complete divider.
///
/// `Grid` does not support row/column spanning, so splitters are direct children of this private
/// docking layer. Each splitter keeps a weak reference to the actual track grid it resizes.
#[elwindui::component(inherits Control)]
pub(crate) struct DockSplitView {
    #[state(default = None)]
    track_grid: Option<Rc<Grid>>,
    #[state(default = Vec::new())]
    splitters: Vec<Rc<CustomGridSplitter>>,
    #[state(default = true)]
    horizontal: bool,
    template: template_view!(|_this: Self| {
        Grid {
            rows: [GridLength::Star(1.0)]
            columns: [GridLength::Star(1.0)]
        }
    }),
}

#[elwindui::component]
impl DockSplitView {
    #[overrides]
    fn on_apply_template(&self) {
        self.sync_children();
    }

    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let Some(root) = self.__template_root() else {
            return Size::default();
        };
        root.measure(available);
        root.measured_size().unwrap_or_default()
    }

    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let Some(root) = self.__template_root() else {
            return final_size;
        };
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: final_size.width.max(0.0),
            height: final_size.height.max(0.0),
        };
        root.arrange(rect);
        let Some(grid) = self.track_grid() else {
            return final_size;
        };

        let horizontal = self.horizontal();
        let track_sizes = if horizontal {
            grid.resolved_column_sizes()
        } else {
            grid.resolved_row_sizes()
        };
        for (index, splitter) in self.splitters().iter().enumerate() {
            let Some(offset) = track_sizes
                .get(..=index)
                .map(|sizes| sizes.iter().sum::<f32>() + (index + 1) as f32 * SPLITTER_HIT_SIZE)
                .filter(|offset| offset.is_finite())
            else {
                continue;
            };
            let rect = if horizontal {
                Rect {
                    x: offset,
                    y: 0.0,
                    width: SPLITTER_HIT_SIZE,
                    height: rect.height,
                }
            } else {
                Rect {
                    x: 0.0,
                    y: offset,
                    width: rect.width,
                    height: SPLITTER_HIT_SIZE,
                }
            };
            splitter.arrange(rect);
        }
        final_size
    }
}

impl DockSplitView {
    pub(crate) fn new_view() -> Rc<Self> {
        Self::new()
    }

    #[cfg(test)]
    pub(crate) fn track_grid_for_test(&self) -> Option<Rc<Grid>> {
        self.track_grid()
    }

    pub(crate) fn configure(
        &self,
        track_grid: Rc<Grid>,
        splitters: Vec<Rc<CustomGridSplitter>>,
        orientation: SnapshotOrientation,
    ) {
        let horizontal = orientation == SnapshotOrientation::Horizontal;
        let current_grid = self.track_grid();
        let current_splitters = self.splitters();
        let structure_changed = !current_grid
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, &track_grid))
            || current_splitters.len() != splitters.len()
            || current_splitters
                .iter()
                .zip(&splitters)
                .any(|(current, next)| !Rc::ptr_eq(current, next))
            || self.horizontal() != horizontal;
        if current_grid
            .as_ref()
            .is_none_or(|current| !Rc::ptr_eq(current, &track_grid))
        {
            self.set_track_grid(Some(track_grid.clone()));
        }
        if current_splitters.len() != splitters.len()
            || current_splitters
                .iter()
                .zip(&splitters)
                .any(|(current, next)| !Rc::ptr_eq(current, next))
        {
            self.set_splitters(splitters.clone());
        }
        if self.horizontal() != horizontal {
            self.set_horizontal(horizontal);
        }
        let translation = if horizontal {
            Vector {
                x: -SPLITTER_HIT_SIZE,
                y: 0.0,
            }
        } else {
            Vector {
                x: 0.0,
                y: -SPLITTER_HIT_SIZE,
            }
        };
        let transform = VisualTransform::new(translation, 1.0, 0.0);
        for (index, splitter) in splitters.iter().enumerate() {
            splitter.set_resize_grid_owner(Some(&track_grid));
            if splitter.visual_transform() != transform {
                splitter.set_visual_transform(transform);
            }
            match orientation {
                SnapshotOrientation::Horizontal => {
                    splitter.set_attached_if_changed("Grid", "row", 0i32);
                    splitter.set_attached_if_changed("Grid", "column", (index + 1) as i32);
                    if splitter.width() != Some(SPLITTER_HIT_SIZE) {
                        splitter.set_width(SPLITTER_HIT_SIZE);
                    }
                    let base = splitter.as_ui_element();
                    if base.height.get().is_some() || base.presentation_height.get().is_some() {
                        base.height.set(None);
                        base.presentation_height.set(None);
                        splitter.invalidate_measure();
                    }
                    if splitter.horizontal_alignment() != HorizontalAlignment::Left {
                        splitter.set_horizontal_alignment(HorizontalAlignment::Left);
                    }
                    if splitter.vertical_alignment() != VerticalAlignment::Stretch {
                        splitter.set_vertical_alignment(VerticalAlignment::Stretch);
                    }
                }
                SnapshotOrientation::Vertical => {
                    splitter.set_attached_if_changed("Grid", "row", (index + 1) as i32);
                    splitter.set_attached_if_changed("Grid", "column", 0i32);
                    let base = splitter.as_ui_element();
                    if base.width.get().is_some() || base.presentation_width.get().is_some() {
                        base.width.set(None);
                        base.presentation_width.set(None);
                        splitter.invalidate_measure();
                    }
                    if splitter.height() != Some(SPLITTER_HIT_SIZE) {
                        splitter.set_height(SPLITTER_HIT_SIZE);
                    }
                    if splitter.horizontal_alignment() != HorizontalAlignment::Stretch {
                        splitter.set_horizontal_alignment(HorizontalAlignment::Stretch);
                    }
                    if splitter.vertical_alignment() != VerticalAlignment::Top {
                        splitter.set_vertical_alignment(VerticalAlignment::Top);
                    }
                }
            }
        }
        // Populate the visual tree when reconciliation runs before the first layout pass.
        self.apply_template();
        self.sync_children();
        if structure_changed {
            self.invalidate_measure();
        }
    }

    fn sync_children(&self) {
        let Some(root) = self.__template_root() else {
            return;
        };
        let Some(grid_root) = root.as_any().downcast_ref::<Grid>() else {
            return;
        };
        let mut expected = Vec::<Rc<dyn UIElementExt>>::new();
        if let Some(grid) = self.track_grid() {
            expected.push(grid as Rc<dyn UIElementExt>);
        }
        expected.extend(
            self.splitters()
                .into_iter()
                .map(|splitter| splitter as Rc<dyn UIElementExt>),
        );
        let actual = grid_root.children().to_vec();
        if actual.len() == expected.len()
            && actual
                .iter()
                .zip(&expected)
                .all(|(actual, expected)| Rc::ptr_eq(actual, expected))
        {
            return;
        }
        grid_root.children().clear();
        for child in expected {
            grid_root.children().add(child);
        }
    }
}
