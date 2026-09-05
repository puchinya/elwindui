//! Retained split-grid state and one-shot splitter transactions.

use crate::core::layout::GridLength;
use crate::core::layout::Orientation;
use crate::core::ui::{Grid, LayoutExt, UIElementExt};
use crate::model::{DockLayoutModel, SplitAddress};
use std::rc::Rc;

pub(crate) struct SplitterSession {
    original: DockLayoutModel,
    address: SplitAddress,
    boundary: usize,
    grid: Rc<Grid>,
    orientation: Orientation,
    extent: f32,
    original_tracks: Vec<GridLength>,
    left_track_index: usize,
    right_track_index: usize,
    left_weight: f32,
    right_weight: f32,
    left_min: f32,
    right_min: f32,
    left_max: f32,
    right_max: f32,
    left_size: f32,
    right_size: f32,
    last_cumulative_delta: f32,
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
        let original_tracks = match orientation {
            Orientation::Horizontal => grid.columns.borrow().clone(),
            Orientation::Vertical => grid.rows.borrow().clone(),
        };
        let left_track_index = boundary.checked_mul(2)?;
        let right_track_index = left_track_index.checked_add(2)?;
        let left_weight = match original_tracks.get(left_track_index)? {
            GridLength::Star(weight) if weight.is_finite() && *weight > 0.0 => *weight,
            _ => return None,
        };
        let right_weight = match original_tracks.get(right_track_index)? {
            GridLength::Star(weight) if weight.is_finite() && *weight > 0.0 => *weight,
            _ => return None,
        };
        let axis_size = |element: &Rc<dyn UIElementExt>| match orientation {
            Orientation::Horizontal => element.arranged_width(),
            Orientation::Vertical => element.arranged_height(),
        };
        let children = grid.children().to_vec();
        let left_child = children.get(boundary.checked_mul(2)?)?;
        let right_child = children.get(right_track_index)?;
        let left_size =
            axis_size(left_child).unwrap_or(extent * left_weight / (left_weight + right_weight));
        let right_size =
            axis_size(right_child).unwrap_or(extent * right_weight / (left_weight + right_weight));
        let (left_min, right_min, left_max, right_max) = match orientation {
            Orientation::Horizontal => (
                left_child.min_width().unwrap_or(0.0),
                right_child.min_width().unwrap_or(0.0),
                left_child.max_width().unwrap_or(f32::INFINITY),
                right_child.max_width().unwrap_or(f32::INFINITY),
            ),
            Orientation::Vertical => (
                left_child.min_height().unwrap_or(0.0),
                right_child.min_height().unwrap_or(0.0),
                left_child.max_height().unwrap_or(f32::INFINITY),
                right_child.max_height().unwrap_or(f32::INFINITY),
            ),
        };
        Some(Self {
            original: model.clone(),
            address,
            boundary,
            grid,
            orientation,
            extent,
            original_tracks,
            left_track_index,
            right_track_index,
            left_weight,
            right_weight,
            left_min: left_min.max(0.0),
            right_min: right_min.max(0.0),
            left_max: left_max.max(left_min.max(0.0)),
            right_max: right_max.max(right_min.max(0.0)),
            left_size,
            right_size,
            last_cumulative_delta: 0.0,
            captured: true,
        })
    }

    pub(crate) fn preview(&mut self, cumulative_delta: f32) {
        if !self.captured || !cumulative_delta.is_finite() {
            return;
        }
        let delta = self.clamped_delta(cumulative_delta);
        let Some(tracks) = self.preview_tracks(delta) else {
            return;
        };
        match self.orientation {
            Orientation::Horizontal => *self.grid.columns.borrow_mut() = tracks,
            Orientation::Vertical => *self.grid.rows.borrow_mut() = tracks,
        }
        self.grid.invalidate_arrange();
        self.grid.flush_interactive_relayout();
        self.last_cumulative_delta = delta;
    }

    pub(crate) fn cancel(&mut self) {
        self.captured = false;
        match self.orientation {
            Orientation::Horizontal => {
                *self.grid.columns.borrow_mut() = self.original_tracks.clone()
            }
            Orientation::Vertical => *self.grid.rows.borrow_mut() = self.original_tracks.clone(),
        }
        self.grid.invalidate_arrange();
    }

    pub(crate) fn commit(&mut self) -> Option<DockLayoutModel> {
        if !self.captured {
            return None;
        }
        self.captured = false;
        let Some(next) = self.original.with_adjacent_split_weights(
            &self.address,
            self.boundary,
            self.last_cumulative_delta,
            self.extent,
        ) else {
            match self.orientation {
                Orientation::Horizontal => {
                    *self.grid.columns.borrow_mut() = self.original_tracks.clone()
                }
                Orientation::Vertical => {
                    *self.grid.rows.borrow_mut() = self.original_tracks.clone()
                }
            }
            self.grid.invalidate_arrange();
            return None;
        };
        self.grid.invalidate_measure();
        Some(next)
    }

    fn preview_tracks(&self, cumulative_delta: f32) -> Option<Vec<GridLength>> {
        let total = self.left_weight + self.right_weight;
        if !total.is_finite() || total <= 0.0 {
            return None;
        }
        let shift = cumulative_delta / self.extent * total;
        if !shift.is_finite() {
            return None;
        }
        let left = (self.left_weight + shift).max(0.0001);
        let right = (self.right_weight - shift).max(0.0001);
        let pair_total = left + right;
        if !pair_total.is_finite() || pair_total <= 0.0 {
            return None;
        }
        let mut tracks = self.original_tracks.clone();
        tracks[self.left_track_index] = GridLength::Star(left / pair_total * total);
        tracks[self.right_track_index] = GridLength::Star(right / pair_total * total);
        Some(tracks)
    }

    fn clamped_delta(&self, delta: f32) -> f32 {
        let lower = (self.left_min - self.left_size).max(self.right_size - self.right_max);
        let upper = (self.left_max - self.left_size).min(self.right_size - self.right_min);
        delta.clamp(lower.min(upper), upper.max(lower))
    }
}
