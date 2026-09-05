//! Transactional tab drag and drop state.

use crate::core::base::{Point, Rect};
use crate::model::{InternalDockGroupPlacement, InternalDockPlacement, RootKind};
use crate::snapshot::SnapshotGroupKey;
use crate::{DockItemId, DockLayoutError, DockLayoutModel, DockSide, DockTarget};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedDockTarget {
    pub(crate) root: RootKind,
    pub(crate) target: DockTarget,
    pub(crate) group: Option<SnapshotGroupKey>,
    pub(crate) preview_rect: Rect,
    pub(crate) tab_insert_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DragSourceGeometry {
    pub(crate) source_root: RootKind,
    pub(crate) source_bounds_host: Rect,
    pub(crate) pointer_offset: Point,
}

/// A drag keeps the last committed model separate from its preview model.
pub(crate) struct DragSession {
    item: DockItemId,
    original: DockLayoutModel,
    source_root: RootKind,
    source_geometry: DragSourceGeometry,
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
        source_root: RootKind,
        source_geometry: DragSourceGeometry,
    ) -> Result<Self, DockLayoutError> {
        if !model.contains_item(&item) {
            return Err(DockLayoutError::UnknownItem(item));
        }
        Ok(Self {
            item,
            original: model.clone(),
            source_root,
            source_geometry,
            candidate: None,
            captured: true,
        })
    }

    pub(crate) fn source_root(&self) -> RootKind {
        self.source_root.clone()
    }

    pub(crate) fn source_geometry(&self) -> &DragSourceGeometry {
        &self.source_geometry
    }

    /// Calculates a candidate placement only; no runtime owner or model is mutated.
    pub(crate) fn preview(
        &mut self,
        target: &ResolvedDockTarget,
        weight: f32,
    ) -> Result<(), DockLayoutError> {
        let placement = placement_for_target(target, weight)?;
        // Validate the private placement without cloning or normalizing the model. Pointer
        // movement owns only this candidate and the visual adornment; the model transforms once
        // on release.
        self.original
            .validate_item_placement(&self.item, &placement)?;
        self.candidate = Some(placement);
        Ok(())
    }

    pub(crate) fn set_floating_candidate(
        &mut self,
        bounds: Rect,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let placement = InternalDockPlacement::Floating { bounds };
        let candidate = self
            .original
            .with_item_moved_internal(&self.item, placement.clone())?;
        self.candidate = Some(placement);
        Ok(candidate)
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
    target: &ResolvedDockTarget,
    weight: f32,
) -> Result<InternalDockPlacement, DockLayoutError> {
    let side = match target.target {
        DockTarget::SplitLeft | DockTarget::DockLeft => DockSide::Left,
        DockTarget::SplitTop | DockTarget::DockTop => DockSide::Top,
        DockTarget::SplitRight | DockTarget::DockRight => DockSide::Right,
        DockTarget::SplitBottom | DockTarget::DockBottom => DockSide::Bottom,
        DockTarget::Center => {
            return target
                .group
                .clone()
                .map(|group| InternalDockPlacement::Group {
                    group: group.into(),
                    index: target.tab_insert_index,
                })
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "center drop requires a target group".to_owned(),
                });
        }
    };
    match target.target {
        DockTarget::SplitLeft
        | DockTarget::SplitTop
        | DockTarget::SplitRight
        | DockTarget::SplitBottom => target
            .group
            .clone()
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
        | DockTarget::DockBottom => Ok(InternalDockPlacement::RootEdge {
            root: target.root.clone(),
            side,
            weight,
        }),
        DockTarget::Center => unreachable!(),
    }
}

/// Transactional coordinator for dragging one complete tab group.
pub(crate) struct GroupDragSession {
    group: SnapshotGroupKey,
    original: DockLayoutModel,
    source_root: RootKind,
    source_geometry: DragSourceGeometry,
    candidate: Option<InternalDockGroupPlacement>,
    captured: bool,
}

impl GroupDragSession {
    pub(crate) fn begin(
        model: &DockLayoutModel,
        group: SnapshotGroupKey,
        source_root: RootKind,
        source_geometry: DragSourceGeometry,
    ) -> Result<Self, DockLayoutError> {
        model.validate_group_placement(
            &group.clone().into(),
            &InternalDockGroupPlacement::Center {
                group: group.clone().into(),
            },
        )?;
        Ok(Self {
            group,
            original: model.clone(),
            source_root,
            source_geometry,
            candidate: None,
            captured: true,
        })
    }

    pub(crate) fn group(&self) -> &SnapshotGroupKey {
        &self.group
    }

    pub(crate) fn source_root(&self) -> RootKind {
        self.source_root.clone()
    }

    pub(crate) fn source_geometry(&self) -> &DragSourceGeometry {
        &self.source_geometry
    }

    pub(crate) fn preview(
        &mut self,
        target: &ResolvedDockTarget,
        weight: f32,
    ) -> Result<(), DockLayoutError> {
        let placement = group_placement_for_target(target, weight)?;
        self.original
            .validate_group_placement(&self.group.clone().into(), &placement)?;
        self.candidate = Some(placement);
        Ok(())
    }

    pub(crate) fn set_floating_candidate(
        &mut self,
        bounds: Rect,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let placement = InternalDockGroupPlacement::Floating { bounds };
        let candidate = self
            .original
            .with_group_moved_internal(&self.group.clone().into(), placement.clone())?;
        self.candidate = Some(placement);
        Ok(candidate)
    }

    pub(crate) fn cancel(&mut self) -> DockLayoutModel {
        self.captured = false;
        self.original.clone()
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
                    .with_group_moved_internal(&self.group.clone().into(), placement)
                    .ok()
            })
            .or_else(|| Some(self.original.clone()))
    }
}

fn group_placement_for_target(
    target: &ResolvedDockTarget,
    weight: f32,
) -> Result<InternalDockGroupPlacement, DockLayoutError> {
    let side = match target.target {
        DockTarget::SplitLeft | DockTarget::DockLeft => DockSide::Left,
        DockTarget::SplitTop | DockTarget::DockTop => DockSide::Top,
        DockTarget::SplitRight | DockTarget::DockRight => DockSide::Right,
        DockTarget::SplitBottom | DockTarget::DockBottom => DockSide::Bottom,
        DockTarget::Center => {
            return target
                .group
                .clone()
                .map(|group| InternalDockGroupPlacement::Center {
                    group: group.into(),
                })
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "center drop requires a target group".to_owned(),
                });
        }
    };
    match target.target {
        DockTarget::SplitLeft
        | DockTarget::SplitTop
        | DockTarget::SplitRight
        | DockTarget::SplitBottom => target
            .group
            .clone()
            .map(|group| InternalDockGroupPlacement::SplitGroup {
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
        | DockTarget::DockBottom => Ok(InternalDockGroupPlacement::RootEdge {
            root: target.root.clone(),
            side,
            weight,
        }),
        DockTarget::Center => unreachable!(),
    }
}
