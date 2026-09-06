//! Retained split-grid state and one-shot model persistence transactions.

use crate::core::layout::Orientation;
use crate::core::ui::{Grid, UIElementExt};
use crate::model::{DockLayoutModel, SplitAddress};
use std::rc::Rc;

pub(crate) struct SplitterSession {
    original: DockLayoutModel,
    address: SplitAddress,
    boundary: usize,
    extent: f32,
    captured: bool,
}

impl SplitterSession {
    pub(crate) fn begin(
        model: &DockLayoutModel,
        address: SplitAddress,
        boundary: usize,
        grid: Rc<Grid>,
        orientation: Orientation,
    ) -> Option<Self> {
        let extent = match orientation {
            Orientation::Horizontal => grid.arranged_width(),
            Orientation::Vertical => grid.arranged_height(),
        }?;
        if !extent.is_finite() || extent <= 0.0 {
            return None;
        }
        Some(Self {
            original: model.clone(),
            address,
            boundary,
            extent,
            captured: true,
        })
    }

    pub(crate) fn cancel(&mut self) {
        self.captured = false;
    }

    pub(crate) fn commit(&mut self, cumulative_delta: f32) -> Option<DockLayoutModel> {
        if !self.captured || !cumulative_delta.is_finite() {
            return None;
        }
        self.captured = false;
        self.original.with_adjacent_split_weights(
            &self.address,
            self.boundary,
            cumulative_delta,
            self.extent,
        )
    }
}
