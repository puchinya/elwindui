//! Transactional tab drag and drop state.

use crate::{
    DockGroupId, DockItemId, DockLayoutError, DockLayoutModel, DockPlacement, DockSide, DockTarget,
};

/// A drag keeps the last committed model separate from its preview model.
pub(crate) struct DragSession {
    item: DockItemId,
    original: DockLayoutModel,
    preview: DockLayoutModel,
    captured: bool,
}

impl DragSession {
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
            preview: model.clone(),
            captured: true,
        })
    }

    /// Calculates a preview only; the original model remains untouched until `commit`.
    pub(crate) fn preview(
        &mut self,
        target: DockTarget,
        group: Option<DockGroupId>,
        weight: f32,
    ) -> Result<&DockLayoutModel, DockLayoutError> {
        let placement = placement_for_target(target, group, weight)?;
        self.preview = self.original.with_item_moved(&self.item, placement)?;
        Ok(&self.preview)
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
        Some(self.preview.clone())
    }
}

fn placement_for_target(
    target: DockTarget,
    group: Option<DockGroupId>,
    weight: f32,
) -> Result<DockPlacement, DockLayoutError> {
    let side = match target {
        DockTarget::SplitLeft | DockTarget::DockLeft => DockSide::Left,
        DockTarget::SplitTop | DockTarget::DockTop => DockSide::Top,
        DockTarget::SplitRight | DockTarget::DockRight => DockSide::Right,
        DockTarget::SplitBottom | DockTarget::DockBottom => DockSide::Bottom,
        DockTarget::Center => {
            return group
                .map(|group| DockPlacement::Group { group, index: None })
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
            .map(|group| DockPlacement::SplitGroup {
                group,
                side,
                weight,
            })
            .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                reason: "split drop requires a target group".to_owned(),
            }),
        DockTarget::DockLeft
        | DockTarget::DockTop
        | DockTarget::DockRight
        | DockTarget::DockBottom => Ok(DockPlacement::RootEdge { side, weight }),
        DockTarget::Center => unreachable!(),
    }
}
