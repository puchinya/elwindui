//! Transactional tab drag and drop state.

use crate::model::InternalDockPlacement;
use crate::snapshot::SnapshotGroupKey;
use crate::{DockItemId, DockLayoutError, DockLayoutModel, DockSide, DockTarget};

/// A drag keeps the last committed model separate from its preview model.
pub(crate) struct DragSession {
    item: DockItemId,
    original: DockLayoutModel,
    candidate: Option<InternalDockPlacement>,
    captured: bool,
}

impl DragSession {
    pub(crate) fn item(&self) -> &DockItemId {
        &self.item
    }

    pub(crate) fn begin(
        model: &DockLayoutModel,
        item: DockItemId,
    ) -> Result<Self, DockLayoutError> {
        if !model.contains_item(&item) {
            return Err(DockLayoutError::UnknownItem(item));
        }
        Ok(Self {
            item,
            original: model.clone(),
            candidate: None,
            captured: true,
        })
    }

    /// Calculates a candidate placement only; no runtime owner or model is mutated.
    pub(crate) fn preview(
        &mut self,
        target: DockTarget,
        group: Option<SnapshotGroupKey>,
        weight: f32,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let placement = placement_for_target(target, group, weight)?;
        let preview = self
            .original
            .with_item_moved_internal(&self.item, placement.clone())?;
        self.candidate = Some(placement);
        Ok(preview)
    }

    pub(crate) fn cancel(&mut self) -> DockLayoutModel {
        self.captured = false;
        self.original.clone()
    }

    #[allow(dead_code)]
    pub(crate) fn capture_lost(&mut self) -> DockLayoutModel {
        self.cancel()
    }

    pub(crate) fn commit(&mut self) -> Option<DockLayoutModel> {
        if !self.captured {
            return None;
        }
        self.captured = false;
        self.candidate
            .take()
            .and_then(|placement| {
                self.original
                    .with_item_moved_internal(&self.item, placement)
                    .ok()
            })
            .or_else(|| Some(self.original.clone()))
    }
}

fn placement_for_target(
    target: DockTarget,
    group: Option<SnapshotGroupKey>,
    weight: f32,
) -> Result<InternalDockPlacement, DockLayoutError> {
    let side = match target {
        DockTarget::SplitLeft | DockTarget::DockLeft => DockSide::Left,
        DockTarget::SplitTop | DockTarget::DockTop => DockSide::Top,
        DockTarget::SplitRight | DockTarget::DockRight => DockSide::Right,
        DockTarget::SplitBottom | DockTarget::DockBottom => DockSide::Bottom,
        DockTarget::Center => {
            return group
                .map(|group| InternalDockPlacement::Group {
                    group: group.into(),
                    index: None,
                })
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "center drop requires a target group".to_owned(),
                });
        }
    };
    match target {
        DockTarget::SplitLeft
        | DockTarget::SplitTop
        | DockTarget::SplitRight
        | DockTarget::SplitBottom => group
            .map(|group| InternalDockPlacement::SplitGroup {
                group: group.into(),
                side,
                weight,
            })
            .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                reason: "split drop requires a target group".to_owned(),
            }),
        DockTarget::DockLeft
        | DockTarget::DockTop
        | DockTarget::DockRight
        | DockTarget::DockBottom => Ok(InternalDockPlacement::RootEdge { side, weight }),
        DockTarget::Center => unreachable!(),
    }
}
