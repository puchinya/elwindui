//! Retained split-grid state and one-shot splitter transactions.

use crate::core::layout::GridLength;
use crate::core::layout::Orientation;
use crate::core::ui::{Grid, GridExt, UIElementExt};
use crate::model::{DockLayoutModel, RootKind, SplitAddress};
use std::rc::Rc;

pub(crate) struct SplitterSession {
    original: DockLayoutModel,
    transient: DockLayoutModel,
    address: SplitAddress,
    boundary: usize,
    grid: Rc<Grid>,
    orientation: Orientation,
    extent: f32,
    original_tracks: Vec<GridLength>,
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
        Some(Self {
            original: model.clone(),
            transient: model.clone(),
            address,
            boundary,
            grid,
            orientation,
            extent,
            original_tracks,
            captured: true,
        })
    }

    pub(crate) fn preview(&mut self, cumulative_delta: f32) {
        if !self.captured {
            return;
        }
        let Some(next) = self.original.with_adjacent_split_weights(
            &self.address,
            self.boundary,
            cumulative_delta,
            self.extent,
        ) else {
            return;
        };
        if let Some(tracks) = split_tracks(&next, &self.address) {
            match self.orientation {
                Orientation::Horizontal => self.grid.set_columns(tracks),
                Orientation::Vertical => self.grid.set_rows(tracks),
            }
        }
        self.transient = next;
    }

    pub(crate) fn cancel(&mut self) -> DockLayoutModel {
        self.captured = false;
        match self.orientation {
            Orientation::Horizontal => self.grid.set_columns(self.original_tracks.clone()),
            Orientation::Vertical => self.grid.set_rows(self.original_tracks.clone()),
        }
        self.original.clone()
    }

    pub(crate) fn commit(&mut self) -> Option<DockLayoutModel> {
        if !self.captured {
            return None;
        }
        self.captured = false;
        Some(self.transient.clone())
    }
}

fn split_tracks(model: &DockLayoutModel, address: &SplitAddress) -> Option<Vec<GridLength>> {
    let snapshot = model.snapshot();
    let mut node = match address.root {
        RootKind::Main => snapshot.main_root.as_ref(),
        RootKind::Floating(index) => snapshot.floating_roots.get(index).map(|root| &root.root),
    }?;
    for index in &address.path {
        let crate::snapshot::SnapshotNode::Split { children, .. } = node else {
            return None;
        };
        node = &children.get(*index)?.node;
    }
    let crate::snapshot::SnapshotNode::Split { children, .. } = node else {
        return None;
    };
    let mut tracks = Vec::with_capacity(children.len() * 2 - 1);
    for (index, child) in children.iter().enumerate() {
        tracks.push(GridLength::Star(child.weight));
        if index + 1 < children.len() {
            tracks.push(GridLength::Fixed(6.0));
        }
    }
    Some(tracks)
}
