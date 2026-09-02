use crate::id::{DockGroupId, DockItemId};
use crate::placement::DockLayoutError;
use elwindui_core::layout::Orientation;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub(crate) const SNAPSHOT_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockLayoutSnapshot {
    pub(crate) version: u32,
    pub(crate) main_root: Option<SnapshotNode>,
    pub(crate) floating_roots: Vec<SnapshotFloatingRoot>,
    pub(crate) auto_hide: [Vec<SnapshotAutoHideEntry>; 4],
    pub(crate) closed: Vec<SnapshotClosedEntry>,
    pub(crate) next_generated_group_id: u64,
    /// The globally active item.
    pub(crate) active_item: Option<DockItemId>,
}

impl DockLayoutSnapshot {
    /// The currently emitted snapshot schema version.
    pub const VERSION: u32 = SNAPSHOT_VERSION;

    /// Returns the schema version encoded by this snapshot.
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum SnapshotNode {
    Split {
        orientation: SnapshotOrientation,
        children: Vec<SnapshotWeightedNode>,
    },
    Group {
        group: SnapshotGroupKey,
        items: Vec<DockItemId>,
        selected: Option<DockItemId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SnapshotWeightedNode {
    pub weight: f32,
    pub node: SnapshotNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SnapshotFloatingRoot {
    pub bounds: SnapshotRect,
    pub root: SnapshotNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SnapshotAutoHideEntry {
    pub item: DockItemId,
    pub open: bool,
    pub return_state: SnapshotReturnState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SnapshotClosedEntry {
    pub item: DockItemId,
    pub return_state: SnapshotReturnState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SnapshotReturnState {
    pub group: SnapshotGroupKey,
    pub index: usize,
    pub floating_root: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SnapshotRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub(crate) enum SnapshotGroupKey {
    Authored(DockGroupId),
    Generated(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SnapshotOrientation {
    Horizontal,
    Vertical,
}

impl From<Orientation> for SnapshotOrientation {
    fn from(value: Orientation) -> Self {
        match value {
            Orientation::Horizontal => Self::Horizontal,
            Orientation::Vertical => Self::Vertical,
        }
    }
}

impl From<SnapshotOrientation> for Orientation {
    fn from(value: SnapshotOrientation) -> Self {
        match value {
            SnapshotOrientation::Horizontal => Self::Horizontal,
            SnapshotOrientation::Vertical => Self::Vertical,
        }
    }
}

impl SnapshotRect {
    pub(crate) fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }
}

impl From<elwindui_core::base::Rect> for SnapshotRect {
    fn from(value: elwindui_core::base::Rect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

impl From<SnapshotRect> for elwindui_core::base::Rect {
    fn from(value: SnapshotRect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

pub(crate) fn validate_snapshot(snapshot: &DockLayoutSnapshot) -> Result<(), DockLayoutError> {
    if snapshot.version != SNAPSHOT_VERSION {
        return Err(DockLayoutError::UnknownSnapshotVersion {
            version: snapshot.version,
        });
    }
    let mut item_ids = HashSet::new();
    let mut group_ids = HashSet::new();
    fn visit(
        node: &SnapshotNode,
        item_ids: &mut HashSet<DockItemId>,
        group_ids: &mut HashSet<SnapshotGroupKey>,
    ) -> Result<(), DockLayoutError> {
        match node {
            SnapshotNode::Split { children, .. } => {
                if children.is_empty() {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "split has no children".to_owned(),
                    });
                }
                for child in children {
                    if !child.weight.is_finite() || child.weight <= 0.0 {
                        return Err(DockLayoutError::InvalidSnapshot {
                            reason: "split weight is not finite and positive".to_owned(),
                        });
                    }
                    visit(&child.node, item_ids, group_ids)?;
                }
            }
            SnapshotNode::Group {
                group,
                items,
                selected,
            } => {
                if !group_ids.insert(group.clone()) {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "group identity appears more than once".to_owned(),
                    });
                }
                for item in items {
                    if item.as_ref().is_empty() || !item_ids.insert(item.clone()) {
                        return Err(DockLayoutError::InvalidSnapshot {
                            reason: "item identity is empty or appears more than once".to_owned(),
                        });
                    }
                }
                if selected
                    .as_ref()
                    .is_some_and(|selected| !items.contains(selected))
                {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "selected item is not in its group".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
    if let Some(root) = &snapshot.main_root {
        visit(root, &mut item_ids, &mut group_ids)?;
    }
    for floating in &snapshot.floating_roots {
        if !floating.bounds.is_valid() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "floating bounds are invalid".to_owned(),
            });
        }
        visit(&floating.root, &mut item_ids, &mut group_ids)?;
    }
    let mut open_auto_hide = false;
    for side in &snapshot.auto_hide {
        for entry in side {
            if entry.item.as_ref().is_empty() || !item_ids.insert(entry.item.clone()) {
                return Err(DockLayoutError::InvalidSnapshot {
                    reason: "auto-hide item identity is empty or duplicated".to_owned(),
                });
            }
            if entry.open {
                if open_auto_hide {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "more than one auto-hide overlay is open".to_owned(),
                    });
                }
                open_auto_hide = true;
            }
            if !entry.return_state.group_is_valid()
                || entry
                    .return_state
                    .floating_root
                    .is_some_and(|index| index >= snapshot.floating_roots.len())
            {
                return Err(DockLayoutError::InvalidSnapshot {
                    reason: "auto-hide return state is invalid".to_owned(),
                });
            }
        }
    }
    for entry in &snapshot.closed {
        if entry.item.as_ref().is_empty() || !item_ids.insert(entry.item.clone()) {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "closed item identity is empty or duplicated".to_owned(),
            });
        }
        if !entry.return_state.group_is_valid()
            || entry
                .return_state
                .floating_root
                .is_some_and(|index| index >= snapshot.floating_roots.len())
        {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "closed return state is invalid".to_owned(),
            });
        }
    }
    Ok(())
}

impl SnapshotReturnState {
    fn group_is_valid(&self) -> bool {
        match &self.group {
            SnapshotGroupKey::Authored(id) => !id.as_ref().is_empty(),
            SnapshotGroupKey::Generated(id) => *id > 0,
        }
    }
}
