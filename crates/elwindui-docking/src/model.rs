use crate::id::{DockGroupId, DockItemId};
use crate::placement::{DockLayoutError, DockPlacement, DockSide, valid_bounds, valid_weight};
use crate::snapshot::{
    DockLayoutSnapshot, SnapshotAutoHideEntry, SnapshotClosedEntry, SnapshotFloatingRoot,
    SnapshotGroupKey, SnapshotNode, SnapshotReturnState, SnapshotWeightedNode, validate_snapshot,
};
use elwindui_core::base::Rect;
use elwindui_core::layout::Orientation;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InternalDockGroupKey {
    Authored(DockGroupId),
    Generated(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub(crate) enum RootKind {
    Main,
    Floating(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub(crate) struct SplitAddress {
    pub(crate) root: RootKind,
    pub(crate) path: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DefaultDockDefinition {
    pub(crate) root: Option<Node>,
    pub(crate) item_groups: BTreeMap<DockItemId, DockGroupId>,
    pub(crate) groups: BTreeSet<DockGroupId>,
    pub(crate) keep_empty_groups: BTreeSet<DockGroupId>,
}

impl DefaultDockDefinition {
    pub(crate) fn new(root: Option<Node>) -> Self {
        let mut item_groups = BTreeMap::new();
        let mut groups = BTreeSet::new();
        if let Some(root) = &root {
            collect_authored_defaults(root, &mut item_groups, &mut groups);
        }
        Self {
            root,
            item_groups,
            keep_empty_groups: BTreeSet::new(),
            groups,
        }
    }

    pub(crate) fn with_keep_empty_groups(
        mut self,
        keep_empty_groups: impl IntoIterator<Item = DockGroupId>,
    ) -> Self {
        self.keep_empty_groups = keep_empty_groups.into_iter().collect();
        self
    }
}

fn collect_authored_defaults(
    node: &Node,
    item_groups: &mut BTreeMap<DockItemId, DockGroupId>,
    groups: &mut BTreeSet<DockGroupId>,
) {
    match node {
        Node::Split { children, .. } => {
            for child in children {
                collect_authored_defaults(&child.node, item_groups, groups);
            }
        }
        Node::Group { group, items, .. } => {
            if let InternalDockGroupKey::Authored(group_id) = group {
                groups.insert(group_id.clone());
                for item in items {
                    item_groups
                        .entry(item.clone())
                        .or_insert_with(|| group_id.clone());
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WeightedNode {
    pub(crate) weight: f32,
    pub(crate) node: Node,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Split {
        orientation: Orientation,
        children: Vec<WeightedNode>,
    },
    Group {
        group: InternalDockGroupKey,
        items: Vec<DockItemId>,
        selected: Option<DockItemId>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FloatingRoot {
    pub(crate) bounds: Rect,
    pub(crate) root: Node,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AutoHideEntry {
    pub(crate) item: DockItemId,
    pub(crate) open: bool,
    pub(crate) return_state: ReturnState,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClosedEntry {
    pub(crate) item: DockItemId,
    pub(crate) return_state: ReturnState,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReturnState {
    pub(crate) group: InternalDockGroupKey,
    pub(crate) index: usize,
    pub(crate) floating_root: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
struct Workspace {
    main_root: Option<Node>,
    floating_roots: Vec<FloatingRoot>,
    auto_hide: [Vec<AutoHideEntry>; 4],
    closed: Vec<ClosedEntry>,
    next_generated_group_id: u64,
    active_item: Option<DockItemId>,
}

impl Workspace {
    fn empty() -> Self {
        Self {
            main_root: None,
            floating_roots: Vec::new(),
            auto_hide: std::array::from_fn(|_| Vec::new()),
            closed: Vec::new(),
            next_generated_group_id: 1,
            active_item: None,
        }
    }
}

/// Opaque, value-semantic current state of one Docking surface and its floating roots.
///
/// The authored declaration and the mutable runtime realization are intentionally not exposed by
/// this type. Every transformation returns a new value, which makes model updates safe to use as a
/// normal TwoWay property value.
#[derive(Clone)]
pub struct DockLayoutModel {
    workspace: Workspace,
    default_definition: Option<Rc<DefaultDockDefinition>>,
}

/// Runtime-only placement target. Unlike [`DockPlacement`], this can address a generated group
/// created by an earlier docking operation without exposing generated identities publicly.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InternalDockPlacement {
    Group {
        group: InternalDockGroupKey,
        index: Option<usize>,
    },
    SplitGroup {
        group: InternalDockGroupKey,
        side: DockSide,
        weight: f32,
    },
    RootEdge {
        root: RootKind,
        side: DockSide,
        weight: f32,
    },
    Floating {
        bounds: Rect,
    },
    AutoHide {
        side: DockSide,
    },
}

/// Runtime-only placement target for moving an entire realized group as one unit.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InternalDockGroupPlacement {
    Center {
        group: InternalDockGroupKey,
    },
    SplitGroup {
        group: InternalDockGroupKey,
        side: DockSide,
        weight: f32,
    },
    RootEdge {
        root: RootKind,
        side: DockSide,
        weight: f32,
    },
    Floating {
        bounds: Rect,
    },
}

impl PartialEq for DockLayoutModel {
    fn eq(&self, other: &Self) -> bool {
        self.workspace == other.workspace
    }
}

impl fmt::Debug for DockLayoutModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockLayoutModel")
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

impl DockLayoutModel {
    /// Creates an empty model with no authored default attached.
    pub fn empty() -> Self {
        Self {
            workspace: Workspace::empty(),
            default_definition: None,
        }
    }

    /// Returns whether the model contains no registered or transient item.
    pub fn is_empty(&self) -> bool {
        self.item_ids().is_empty()
    }

    /// Returns whether an item is present in a live, auto-hidden, or closed state.
    pub fn contains_item(&self, item: &DockItemId) -> bool {
        self.item_ids().iter().any(|candidate| candidate == item)
    }

    /// Returns whether an item is currently closed.
    pub fn is_item_closed(&self, item: &DockItemId) -> bool {
        self.workspace
            .closed
            .iter()
            .any(|entry| &entry.item == item)
    }

    /// Returns whether an item is selected or owns the active auto-hide overlay.
    pub fn is_item_active(&self, item: &DockItemId) -> bool {
        if self.workspace.active_item.as_ref() == Some(item) {
            return true;
        }
        if self.workspace.active_item.is_some() {
            return false;
        }
        if self
            .workspace
            .auto_hide
            .iter()
            .flatten()
            .any(|entry| &entry.item == item && entry.open)
        {
            return true;
        }
        self.group_is_active(&self.workspace.main_root, item)
            || self
                .workspace
                .floating_roots
                .iter()
                .any(|root| self.group_is_active(&Some(root.root.clone()), item))
    }

    /// Returns the globally active item, if one has been activated.
    pub fn active_item(&self) -> Option<DockItemId> {
        self.workspace.active_item.clone()
    }

    /// Returns the number of live floating roots in the workspace.
    pub fn floating_root_count(&self) -> usize {
        self.workspace.floating_roots.len()
    }

    /// Returns whether the item is currently represented by an auto-hide entry.
    pub fn is_item_auto_hidden(&self, item: &DockItemId) -> bool {
        self.workspace
            .auto_hide
            .iter()
            .flatten()
            .any(|entry| &entry.item == item)
    }

    /// Returns a model with the item selected, reopening it when it is closed.
    pub fn with_item_activated(&self, item: &DockItemId) -> Result<Self, DockLayoutError> {
        let mut next = self.clone();
        if !next.contains_item(item) {
            return Err(DockLayoutError::UnknownItem(item.clone()));
        }
        if next.is_item_closed(item) {
            next.reopen_internal(item);
        }
        let mut changed = false;
        changed |= select_item(&mut next.workspace.main_root, item);
        for floating in &mut next.workspace.floating_roots {
            changed |= select_node(&mut floating.root, item);
        }
        for side in &mut next.workspace.auto_hide {
            for entry in side {
                let should_open = entry.item == *item;
                if entry.open != should_open {
                    entry.open = should_open;
                    changed = true;
                }
            }
        }
        if next.workspace.active_item.as_ref() != Some(item) {
            next.workspace.active_item = Some(item.clone());
            changed = true;
        }
        if !changed && next.is_item_active(item) {
            return Ok(next);
        }
        next.normalize();
        Ok(next)
    }

    /// Returns a model with the item closed and its return position recorded.
    pub fn with_item_closed(&self, item: &DockItemId) -> Result<Self, DockLayoutError> {
        let mut next = self.clone();
        if !next.contains_item(item) {
            return Err(DockLayoutError::UnknownItem(item.clone()));
        }
        if next.is_item_closed(item) {
            return Ok(next);
        }
        let was_active = next.is_item_active(item);
        let return_state = next
            .remove_live_item(item)
            .unwrap_or_else(|| next.fallback_return_state());
        let return_group = return_state.group.clone();
        next.workspace.closed.push(ClosedEntry {
            item: item.clone(),
            return_state,
        });
        if was_active {
            next.workspace.active_item = None;
        }
        next.normalize();
        if was_active {
            next.workspace.active_item = next
                .selected_item_in_group(&return_group)
                .or_else(|| next.selected_item_id());
        }
        Ok(next)
    }

    /// Returns a model with a closed item restored to its recorded position.
    pub fn with_item_reopened(&self, item: &DockItemId) -> Result<Self, DockLayoutError> {
        let mut next = self.clone();
        if !next.contains_item(item) {
            return Err(DockLayoutError::UnknownItem(item.clone()));
        }
        if !next.is_item_closed(item) {
            return Ok(next);
        }
        next.reopen_internal(item);
        next.normalize();
        Ok(next)
    }

    /// Returns a model with an auto-hidden item restored to its remembered return position.
    pub fn with_item_unpinned(&self, item: &DockItemId) -> Result<Self, DockLayoutError> {
        let mut next = self.clone();
        let Some(side) = DockSide::ALL.into_iter().find(|side| {
            next.workspace.auto_hide[side.index()]
                .iter()
                .any(|entry| &entry.item == item)
        }) else {
            return Ok(next);
        };
        let entries = &mut next.workspace.auto_hide[side.index()];
        let position = entries
            .iter()
            .position(|entry| &entry.item == item)
            .expect("auto-hide side was found above");
        let return_state = entries.remove(position).return_state;
        next.restore_at(item.clone(), return_state);
        if next.workspace.active_item.is_none() {
            next.workspace.active_item = Some(item.clone());
        }
        next.normalize();
        Ok(next)
    }

    /// Returns a model with the item placed according to a backend-neutral placement.
    pub fn with_item_moved(
        &self,
        item: &DockItemId,
        placement: DockPlacement,
    ) -> Result<Self, DockLayoutError> {
        validate_placement(&placement)?;
        let placement = match placement {
            DockPlacement::Group { group, index } => InternalDockPlacement::Group {
                group: InternalDockGroupKey::Authored(group),
                index,
            },
            DockPlacement::SplitGroup {
                group,
                side,
                weight,
            } => InternalDockPlacement::SplitGroup {
                group: InternalDockGroupKey::Authored(group),
                side,
                weight,
            },
            DockPlacement::RootEdge { side, weight } => InternalDockPlacement::RootEdge {
                root: RootKind::Main,
                side,
                weight,
            },
            DockPlacement::Floating { bounds } => InternalDockPlacement::Floating { bounds },
            DockPlacement::AutoHide { side } => InternalDockPlacement::AutoHide { side },
        };
        self.with_item_moved_internal(item, placement)
    }

    pub(crate) fn with_item_moved_internal(
        &self,
        item: &DockItemId,
        placement: InternalDockPlacement,
    ) -> Result<Self, DockLayoutError> {
        if let InternalDockPlacement::SplitGroup { weight, .. }
        | InternalDockPlacement::RootEdge { weight, .. } = placement
            && !valid_weight(weight)
        {
            return Err(DockLayoutError::InvalidWeight);
        }
        if let InternalDockPlacement::Floating { bounds } = placement
            && !valid_bounds(bounds)
        {
            return Err(DockLayoutError::InvalidBounds);
        }
        if let InternalDockPlacement::RootEdge {
            root: RootKind::Floating(index),
            ..
        } = &placement
            && self.workspace.floating_roots.get(*index).is_none()
        {
            return Err(DockLayoutError::InvalidFloatingRoot { index: *index });
        }
        let mut next = self.clone();
        if !next.contains_item(item) {
            return Err(DockLayoutError::UnknownItem(item.clone()));
        }
        let return_state = next
            .remove_live_item(item)
            .or_else(|| next.remove_closed_item(item))
            .unwrap_or_else(|| next.fallback_return_state());
        let source_group = return_state.group.clone();
        let placement = match placement {
            InternalDockPlacement::Group {
                group,
                index: Some(index),
            } if group == source_group => InternalDockPlacement::Group {
                group,
                index: Some(if return_state.index < index {
                    index.saturating_sub(1)
                } else {
                    index
                }),
            },
            other => other,
        };
        next.place_item(item.clone(), placement, return_state)?;
        next.normalize();
        Ok(next)
    }

    pub(crate) fn validate_item_placement(
        &self,
        item: &DockItemId,
        placement: &InternalDockPlacement,
    ) -> Result<(), DockLayoutError> {
        if !self.contains_item(item) {
            return Err(DockLayoutError::UnknownItem(item.clone()));
        }
        match placement {
            InternalDockPlacement::Group { group, .. }
            | InternalDockPlacement::SplitGroup { group, .. } => {
                if !self.contains_group(group) {
                    return Err(match group {
                        InternalDockGroupKey::Authored(group) => {
                            DockLayoutError::UnknownGroup(group.clone())
                        }
                        InternalDockGroupKey::Generated(_) => DockLayoutError::InvalidSnapshot {
                            reason: "generated target group is no longer present".to_owned(),
                        },
                    });
                }
            }
            InternalDockPlacement::RootEdge { root, weight, .. } => {
                if !valid_weight(*weight) {
                    return Err(DockLayoutError::InvalidWeight);
                }
                if let RootKind::Floating(index) = root
                    && self.workspace.floating_roots.get(*index).is_none()
                {
                    return Err(DockLayoutError::InvalidFloatingRoot { index: *index });
                }
            }
            InternalDockPlacement::Floating { bounds } => {
                if !valid_bounds(*bounds) {
                    return Err(DockLayoutError::InvalidBounds);
                }
            }
            InternalDockPlacement::AutoHide { .. } => {}
        }
        if let InternalDockPlacement::SplitGroup { weight, .. } = placement
            && !valid_weight(*weight)
        {
            return Err(DockLayoutError::InvalidWeight);
        }
        Ok(())
    }

    pub(crate) fn validate_group_placement(
        &self,
        group: &InternalDockGroupKey,
        placement: &InternalDockGroupPlacement,
    ) -> Result<(), DockLayoutError> {
        if !self.contains_group(group) {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "source group is no longer present".to_owned(),
            });
        }
        match placement {
            InternalDockGroupPlacement::Center { group: target }
            | InternalDockGroupPlacement::SplitGroup { group: target, .. }
                if !self.contains_group(target) =>
            {
                Err(DockLayoutError::InvalidSnapshot {
                    reason: "target group is no longer present".to_owned(),
                })
            }
            InternalDockGroupPlacement::SplitGroup { weight, .. }
            | InternalDockGroupPlacement::RootEdge { weight, .. }
                if !valid_weight(*weight) =>
            {
                Err(DockLayoutError::InvalidWeight)
            }
            InternalDockGroupPlacement::RootEdge {
                root: RootKind::Floating(index),
                ..
            } if self.workspace.floating_roots.get(*index).is_none() => {
                Err(DockLayoutError::InvalidFloatingRoot { index: *index })
            }
            InternalDockGroupPlacement::Floating { bounds } if !valid_bounds(*bounds) => {
                Err(DockLayoutError::InvalidBounds)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn with_group_moved_internal(
        &self,
        group: &InternalDockGroupKey,
        placement: InternalDockGroupPlacement,
    ) -> Result<Self, DockLayoutError> {
        self.validate_group_placement(group, &placement)?;
        if matches!(&placement, InternalDockGroupPlacement::Center { group: target } if target == group)
        {
            return Ok(self.clone());
        }

        let mut next = self.clone();
        let mut source_node = None;
        if let Some(root) = next.workspace.main_root.take() {
            let (remaining, detached) = detach_group(root, group);
            next.workspace.main_root = remaining;
            source_node = detached;
        }
        let mut removed_floating_index = None;
        if source_node.is_none() {
            let mut index = 0;
            while index < next.workspace.floating_roots.len() {
                let floating = next.workspace.floating_roots.remove(index);
                let (remaining, detached) = detach_group(floating.root, group);
                if let Some(root) = remaining {
                    next.workspace.floating_roots.insert(
                        index,
                        FloatingRoot {
                            bounds: floating.bounds,
                            root,
                        },
                    );
                } else if detached.is_some() {
                    removed_floating_index = Some(index);
                }
                if detached.is_some() {
                    source_node = detached;
                    break;
                }
                index += 1;
            }
        }
        let Some(source_node) = source_node else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "source group is no longer present".to_owned(),
            });
        };

        let placement = if let Some(source_index) = removed_floating_index {
            match placement {
                InternalDockGroupPlacement::RootEdge {
                    root: RootKind::Floating(index),
                    ..
                } if index == source_index => {
                    return Ok(self.clone());
                }
                InternalDockGroupPlacement::RootEdge {
                    root: RootKind::Floating(index),
                    side,
                    weight,
                } => InternalDockGroupPlacement::RootEdge {
                    root: RootKind::Floating(if index > source_index {
                        index - 1
                    } else {
                        index
                    }),
                    side,
                    weight,
                },
                other => other,
            }
        } else {
            placement
        };

        match placement {
            InternalDockGroupPlacement::Center { group: target } => {
                let Node::Group {
                    items, selected, ..
                } = source_node
                else {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "group source is not a group node".to_owned(),
                    });
                };
                let mut placed = next.workspace.main_root.as_mut().is_some_and(|root| {
                    append_group_items(root, &target, &items, selected.clone())
                });
                if !placed {
                    for floating in &mut next.workspace.floating_roots {
                        if append_group_items(&mut floating.root, &target, &items, selected.clone())
                        {
                            placed = true;
                            break;
                        }
                    }
                }
                if !placed {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "target group is no longer present".to_owned(),
                    });
                }
            }
            InternalDockGroupPlacement::SplitGroup {
                group: target,
                side,
                weight,
            } => {
                let mut placed = next.workspace.main_root.as_mut().is_some_and(|root| {
                    split_group(root, &target, source_node.clone(), side, weight)
                });
                if !placed {
                    for floating in &mut next.workspace.floating_roots {
                        if split_group(
                            &mut floating.root,
                            &target,
                            source_node.clone(),
                            side,
                            weight,
                        ) {
                            placed = true;
                            break;
                        }
                    }
                }
                if !placed {
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "target group is no longer present".to_owned(),
                    });
                }
            }
            InternalDockGroupPlacement::RootEdge { root, side, weight } => {
                let wrap = |old_root: Option<Node>| match old_root {
                    None => source_node.clone(),
                    Some(old_root) => Node::Split {
                        orientation: side.orientation(),
                        children: if side.is_leading() {
                            vec![
                                WeightedNode {
                                    weight,
                                    node: source_node.clone(),
                                },
                                WeightedNode {
                                    weight: 1.0,
                                    node: old_root,
                                },
                            ]
                        } else {
                            vec![
                                WeightedNode {
                                    weight: 1.0,
                                    node: old_root,
                                },
                                WeightedNode {
                                    weight,
                                    node: source_node.clone(),
                                },
                            ]
                        },
                    },
                };
                match root {
                    RootKind::Main => {
                        next.workspace.main_root = Some(wrap(next.workspace.main_root.take()));
                    }
                    RootKind::Floating(index) => {
                        let old_root = std::mem::replace(
                            &mut next.workspace.floating_roots[index].root,
                            source_node.clone(),
                        );
                        next.workspace.floating_roots[index].root = wrap(Some(old_root));
                    }
                }
            }
            InternalDockGroupPlacement::Floating { bounds } => {
                next.workspace.floating_roots.push(FloatingRoot {
                    bounds,
                    root: source_node,
                });
            }
        }
        next.normalize();
        Ok(next)
    }

    /// Returns a model restored to the currently attached authored declaration.
    pub fn with_reset(&self) -> Result<Self, DockLayoutError> {
        let Some(default_definition) = &self.default_definition else {
            return Err(DockLayoutError::DefaultLayoutUnavailable);
        };
        let mut next = self.clone();
        next.workspace = Workspace::empty();
        next.workspace.main_root = default_definition.root.clone();
        next.normalize();
        Ok(next)
    }

    /// Clears the current presentation while retaining the authored declaration for reset.
    /// This operation intentionally does not consult item close capabilities.
    pub fn with_cleared_layout(&self) -> Result<Self, DockLayoutError> {
        let mut next = self.clone();
        let items = next.item_ids();
        let mut closed = Vec::new();
        for item in &items {
            if next.is_item_closed(item) {
                continue;
            }
            // Capture every return index against the same pre-clear workspace. Removing items
            // while collecting these states would shift later same-group indices and make a
            // deterministic reopen sequence reverse the original tab order.
            let mut probe = next.clone();
            let mut return_state = probe
                .remove_live_item(item)
                .unwrap_or_else(|| next.fallback_return_state());
            return_state.floating_root = None;
            closed.push(ClosedEntry {
                item: item.clone(),
                return_state,
            });
        }
        for item in &items {
            next.remove_live_item(item);
        }
        next.workspace.main_root = next
            .default_definition
            .as_ref()
            .and_then(|definition| definition.root.as_ref().map(empty_authored_root));
        next.workspace.floating_roots.clear();
        next.workspace.auto_hide = std::array::from_fn(|_| Vec::new());
        next.workspace.closed.extend(closed);
        next.workspace.active_item = None;
        next.normalize();
        Ok(next)
    }

    pub(crate) fn with_floating_bounds(
        &self,
        index: usize,
        bounds: Rect,
    ) -> Result<Self, DockLayoutError> {
        if !valid_bounds(bounds) {
            return Err(DockLayoutError::InvalidBounds);
        }
        let mut next = self.clone();
        let Some(root) = next.workspace.floating_roots.get_mut(index) else {
            return Err(DockLayoutError::InvalidFloatingRoot { index });
        };
        if root.bounds == bounds {
            return Ok(next);
        }
        root.bounds = bounds;
        Ok(next)
    }

    /// Serializes the current value state without authored UI or capability metadata.
    pub fn snapshot(&self) -> DockLayoutSnapshot {
        DockLayoutSnapshot {
            version: crate::snapshot::SNAPSHOT_VERSION,
            main_root: self.workspace.main_root.as_ref().map(Into::into),
            floating_roots: self
                .workspace
                .floating_roots
                .iter()
                .map(|root| SnapshotFloatingRoot {
                    bounds: root.bounds.into(),
                    root: (&root.root).into(),
                })
                .collect(),
            auto_hide: std::array::from_fn(|side| {
                self.workspace.auto_hide[side]
                    .iter()
                    .map(|entry| SnapshotAutoHideEntry {
                        item: entry.item.clone(),
                        open: entry.open,
                        return_state: (&entry.return_state).into(),
                    })
                    .collect()
            }),
            closed: self
                .workspace
                .closed
                .iter()
                .map(|entry| SnapshotClosedEntry {
                    item: entry.item.clone(),
                    return_state: (&entry.return_state).into(),
                })
                .collect(),
            next_generated_group_id: self.workspace.next_generated_group_id,
            active_item: self.workspace.active_item.clone(),
        }
    }

    /// Restores a model from a validated version-2 snapshot without an authored default.
    pub fn from_snapshot(snapshot: DockLayoutSnapshot) -> Result<Self, DockLayoutError> {
        validate_snapshot(&snapshot)?;
        let DockLayoutSnapshot {
            main_root,
            floating_roots,
            auto_hide,
            closed,
            next_generated_group_id,
            active_item,
            ..
        } = snapshot;
        let mut model = Self {
            workspace: Workspace {
                main_root: main_root.map(TryInto::try_into).transpose()?,
                floating_roots: floating_roots
                    .into_iter()
                    .map(|root| {
                        Ok(FloatingRoot {
                            bounds: root.bounds.into(),
                            root: root.root.try_into()?,
                        })
                    })
                    .collect::<Result<_, DockLayoutError>>()?,
                auto_hide: {
                    let mut restored = std::array::from_fn(|_| Vec::new());
                    for side in DockSide::ALL {
                        restored[side.index()] = auto_hide[side.index()]
                            .iter()
                            .map(TryInto::try_into)
                            .collect::<Result<Vec<_>, _>>()?;
                    }
                    restored
                },
                closed: closed
                    .iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()?,
                next_generated_group_id: next_generated_group_id.max(1),
                active_item,
            },
            default_definition: None,
        };
        if model.workspace.next_generated_group_id <= model.max_generated_group_id() {
            model.workspace.next_generated_group_id = model.max_generated_group_id() + 1;
        }
        model.normalize();
        Ok(model)
    }

    pub(crate) fn attach_default(&self, definition: DefaultDockDefinition) -> Self {
        let mut next = self.clone();
        next.default_definition = Some(Rc::new(definition));
        if next.is_empty() {
            next.workspace.main_root = next
                .default_definition
                .as_ref()
                .and_then(|definition| definition.root.clone());
        }
        next.normalize();
        next
    }

    /// Returns a transient model with only the adjacent panes at a retained split resized.
    pub(crate) fn with_adjacent_split_weights(
        &self,
        address: &SplitAddress,
        boundary: usize,
        cumulative_delta: f32,
        arranged_extent: f32,
    ) -> Option<Self> {
        if !arranged_extent.is_finite() || arranged_extent <= 0.0 || !cumulative_delta.is_finite() {
            return None;
        }
        let mut next = self.clone();
        let root = match address.root {
            RootKind::Main => next.workspace.main_root.as_mut(),
            RootKind::Floating(index) => next
                .workspace
                .floating_roots
                .get_mut(index)
                .map(|root| &mut root.root),
        }?;
        let node = node_at_path_mut(root, &address.path)?;
        let Node::Split { children, .. } = node else {
            return None;
        };
        let (left, right) = (children.get(boundary)?, children.get(boundary + 1)?);
        let total = left.weight + right.weight;
        if !total.is_finite() || total <= 0.0 {
            return None;
        }
        let shift = cumulative_delta / arranged_extent * total;
        let left_weight = (left.weight + shift).max(0.0001);
        let right_weight = (right.weight - shift).max(0.0001);
        let pair_total = left_weight + right_weight;
        children[boundary].weight = left_weight / pair_total * total;
        children[boundary + 1].weight = right_weight / pair_total * total;
        Some(next)
    }

    pub(crate) fn floating_item_ids(&self, index: usize) -> Vec<DockItemId> {
        fn collect(node: &Node, out: &mut Vec<DockItemId>) {
            match node {
                Node::Group { items, .. } => out.extend(items.iter().cloned()),
                Node::Split { children, .. } => {
                    for child in children {
                        collect(&child.node, out);
                    }
                }
            }
        }
        let mut items = Vec::new();
        if let Some(root) = self.workspace.floating_roots.get(index) {
            collect(&root.root, &mut items);
        }
        items
    }

    /// Removes a floating root after a native close has closed every item it contained.
    ///
    /// Authored groups may intentionally remain visible when empty, but an empty native
    /// floating host has no useful presentation and must not outlive its model root. Return
    /// states are remapped because floating-root indices are positional in the snapshot model.
    pub(crate) fn without_empty_floating_root(
        &self,
        index: usize,
    ) -> Result<Self, DockLayoutError> {
        // Generated roots are already removed by `normalize` when their last item is closed.
        // An authored root, on the other hand, is intentionally retained until this method
        // removes the empty native host explicitly.
        let Some(_) = self.workspace.floating_roots.get(index) else {
            return Ok(self.clone());
        };
        if !self.floating_item_ids(index).is_empty() {
            return Ok(self.clone());
        }

        fn remap(index: &mut Option<usize>, removed: usize) {
            *index = match *index {
                Some(value) if value == removed => None,
                Some(value) if value > removed => Some(value - 1),
                other => other,
            };
        }

        let mut next = self.clone();
        next.workspace.floating_roots.remove(index);
        for side in &mut next.workspace.auto_hide {
            for entry in side {
                remap(&mut entry.return_state.floating_root, index);
            }
        }
        for entry in &mut next.workspace.closed {
            remap(&mut entry.return_state.floating_root, index);
        }
        Ok(next)
    }

    pub(crate) fn selected_item_id(&self) -> Option<DockItemId> {
        fn selected(node: &Node) -> Option<DockItemId> {
            match node {
                Node::Group { selected, .. } => selected.clone(),
                Node::Split { children, .. } => {
                    children.iter().find_map(|child| selected(&child.node))
                }
            }
        }
        self.workspace
            .main_root
            .as_ref()
            .and_then(selected)
            .or_else(|| {
                self.workspace
                    .floating_roots
                    .iter()
                    .find_map(|root| selected(&root.root))
            })
    }

    fn selected_item_in_group(&self, target: &InternalDockGroupKey) -> Option<DockItemId> {
        fn find(node: &Node, target: &InternalDockGroupKey) -> Option<DockItemId> {
            match node {
                Node::Group {
                    group, selected, ..
                } if group == target => selected.clone(),
                Node::Group { .. } => None,
                Node::Split { children, .. } => {
                    children.iter().find_map(|child| find(&child.node, target))
                }
            }
        }
        self.workspace
            .main_root
            .as_ref()
            .and_then(|root| find(root, target))
            .or_else(|| {
                self.workspace
                    .floating_roots
                    .iter()
                    .find_map(|root| find(&root.root, target))
            })
    }

    #[cfg(test)]
    pub(crate) fn from_default(definition: DefaultDockDefinition) -> Self {
        Self::empty().attach_default(definition)
    }

    fn item_ids(&self) -> Vec<DockItemId> {
        let mut ids = Vec::new();
        collect_node_items(&self.workspace.main_root, &mut ids);
        for floating in &self.workspace.floating_roots {
            collect_node_items(&Some(floating.root.clone()), &mut ids);
        }
        for side in &self.workspace.auto_hide {
            ids.extend(side.iter().map(|entry| entry.item.clone()));
        }
        ids.extend(self.workspace.closed.iter().map(|entry| entry.item.clone()));
        ids
    }

    fn contains_group(&self, target: &InternalDockGroupKey) -> bool {
        let roots = self
            .workspace
            .main_root
            .iter()
            .chain(self.workspace.floating_roots.iter().map(|root| &root.root));
        roots.into_iter().any(|root| contains_group(root, target))
    }

    fn group_is_active(&self, root: &Option<Node>, item: &DockItemId) -> bool {
        fn visit(node: &Node, item: &DockItemId) -> bool {
            match node {
                Node::Group { selected, .. } => selected.as_ref() == Some(item),
                Node::Split { children, .. } => {
                    children.iter().any(|child| visit(&child.node, item))
                }
            }
        }
        root.as_ref().is_some_and(|root| visit(root, item))
    }

    fn reopen_internal(&mut self, item: &DockItemId) {
        let Some(position) = self
            .workspace
            .closed
            .iter()
            .position(|entry| &entry.item == item)
        else {
            return;
        };
        let entry = self.workspace.closed.remove(position);
        self.restore_at(item.clone(), entry.return_state);
    }

    fn remove_closed_item(&mut self, item: &DockItemId) -> Option<ReturnState> {
        self.workspace
            .closed
            .iter()
            .position(|entry| &entry.item == item)
            .map(|index| self.workspace.closed.remove(index).return_state)
    }

    fn remove_live_item(&mut self, item: &DockItemId) -> Option<ReturnState> {
        if let Some((group, index)) = self
            .workspace
            .main_root
            .as_mut()
            .and_then(|root| remove_from_node(root, item))
        {
            return Some(ReturnState {
                group,
                index,
                floating_root: None,
            });
        }
        for (floating_index, floating) in self.workspace.floating_roots.iter_mut().enumerate() {
            if let Some((group, index)) = remove_from_node(&mut floating.root, item) {
                return Some(ReturnState {
                    group,
                    index,
                    floating_root: Some(floating_index),
                });
            }
        }
        for side in DockSide::ALL {
            if let Some(index) = self.workspace.auto_hide[side.index()]
                .iter()
                .position(|entry| &entry.item == item)
            {
                return Some(
                    self.workspace.auto_hide[side.index()]
                        .remove(index)
                        .return_state,
                );
            }
        }
        None
    }

    fn fallback_return_state(&self) -> ReturnState {
        let group = self
            .default_definition
            .as_ref()
            .and_then(|definition| definition.groups.iter().next().cloned())
            .map(InternalDockGroupKey::Authored)
            .unwrap_or_else(|| InternalDockGroupKey::Generated(0));
        ReturnState {
            group,
            index: usize::MAX,
            floating_root: None,
        }
    }

    fn restore_at(&mut self, item: DockItemId, state: ReturnState) {
        if let Some(root_index) = state.floating_root {
            if let Some(floating) = self.workspace.floating_roots.get_mut(root_index) {
                if append_to_group(
                    &mut floating.root,
                    &state.group,
                    item.clone(),
                    Some(state.index),
                ) {
                    return;
                }
            }
        }
        if self.workspace.main_root.is_none() {
            self.workspace.main_root = self
                .default_definition
                .as_ref()
                .and_then(|definition| definition.root.as_ref().map(empty_authored_root));
        }
        if let Some(default_definition) = &self.default_definition {
            let empty_root = default_definition.root.as_ref().map(empty_authored_root);
            ensure_authored_group(
                &mut self.workspace.main_root,
                &state.group,
                empty_root.as_ref(),
            );
        }
        if self.workspace.main_root.as_mut().is_some_and(|root| {
            append_to_group(root, &state.group, item.clone(), Some(state.index))
        }) {
            return;
        }
        if let Some(default_definition) = &self.default_definition {
            if let Some(group) = default_definition.item_groups.get(&item) {
                if self.workspace.main_root.as_mut().is_some_and(|root| {
                    append_to_group(
                        root,
                        &InternalDockGroupKey::Authored(group.clone()),
                        item.clone(),
                        None,
                    )
                }) {
                    return;
                }
            }
        }
        self.insert_root_fallback(item);
    }

    fn insert_root_fallback(&mut self, item: DockItemId) {
        let key = self.next_generated_key();
        let new_group = Node::Group {
            group: key,
            items: vec![item.clone()],
            selected: Some(item),
        };
        match self.workspace.main_root.take() {
            None => self.workspace.main_root = Some(new_group),
            Some(root) => {
                self.workspace.main_root = Some(Node::Split {
                    orientation: Orientation::Horizontal,
                    children: vec![
                        WeightedNode {
                            weight: 1.0,
                            node: root,
                        },
                        WeightedNode {
                            weight: 1.0,
                            node: new_group,
                        },
                    ],
                });
            }
        }
    }

    fn place_item(
        &mut self,
        item: DockItemId,
        placement: InternalDockPlacement,
        return_state: ReturnState,
    ) -> Result<(), DockLayoutError> {
        match placement {
            InternalDockPlacement::Group {
                group: target,
                index,
            } => {
                let mut placed = self
                    .workspace
                    .main_root
                    .as_mut()
                    .is_some_and(|root| append_to_group(root, &target, item.clone(), index));
                if !placed {
                    for floating in &mut self.workspace.floating_roots {
                        if append_to_group(&mut floating.root, &target, item.clone(), index) {
                            placed = true;
                            break;
                        }
                    }
                }
                if !placed {
                    if let InternalDockGroupKey::Authored(group) = target {
                        return Err(DockLayoutError::UnknownGroup(group));
                    }
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "generated target group is no longer present".to_owned(),
                    });
                }
            }
            InternalDockPlacement::SplitGroup {
                group,
                side,
                weight,
            } => {
                let key = self.next_generated_key();
                let new_group = Node::Group {
                    group: key,
                    items: vec![item.clone()],
                    selected: Some(item),
                };
                let mut placed = false;
                if let Some(root) = self.workspace.main_root.as_mut() {
                    placed = split_group(root, &group, new_group.clone(), side, weight);
                }
                if !placed {
                    for floating in &mut self.workspace.floating_roots {
                        if split_group(&mut floating.root, &group, new_group.clone(), side, weight)
                        {
                            placed = true;
                            break;
                        }
                    }
                }
                if !placed {
                    if let InternalDockGroupKey::Authored(group) = group {
                        return Err(DockLayoutError::UnknownGroup(group));
                    }
                    return Err(DockLayoutError::InvalidSnapshot {
                        reason: "generated target group is no longer present".to_owned(),
                    });
                }
            }
            InternalDockPlacement::RootEdge { root, side, weight } => {
                let key = self.next_generated_key();
                let new_group = Node::Group {
                    group: key,
                    items: vec![item.clone()],
                    selected: Some(item),
                };
                let wrap_root = |old_root: Option<Node>, new_group: Node| match old_root {
                    None => new_group,
                    Some(root) => {
                        let orientation = side.orientation();
                        let new_child = WeightedNode {
                            weight,
                            node: new_group,
                        };
                        let old_child = WeightedNode {
                            weight: 1.0,
                            node: root,
                        };
                        Node::Split {
                            orientation,
                            children: if side.is_leading() {
                                vec![new_child, old_child]
                            } else {
                                vec![old_child, new_child]
                            },
                        }
                    }
                };
                match root {
                    RootKind::Main => {
                        self.workspace.main_root =
                            Some(wrap_root(self.workspace.main_root.take(), new_group));
                    }
                    RootKind::Floating(index) => {
                        let old_root = std::mem::replace(
                            &mut self.workspace.floating_roots[index].root,
                            new_group.clone(),
                        );
                        self.workspace.floating_roots[index].root =
                            wrap_root(Some(old_root), new_group);
                    }
                }
            }
            InternalDockPlacement::Floating { bounds } => {
                let key = self.next_generated_key();
                self.workspace.floating_roots.push(FloatingRoot {
                    bounds,
                    root: Node::Group {
                        group: key,
                        items: vec![item.clone()],
                        selected: Some(item),
                    },
                });
            }
            InternalDockPlacement::AutoHide { side } => {
                self.workspace.auto_hide[side.index()].push(AutoHideEntry {
                    item,
                    open: false,
                    return_state,
                });
            }
        }
        Ok(())
    }

    fn next_generated_key(&mut self) -> InternalDockGroupKey {
        let id = self.workspace.next_generated_group_id.max(1);
        self.workspace.next_generated_group_id = id.saturating_add(1);
        InternalDockGroupKey::Generated(id)
    }

    fn max_generated_group_id(&self) -> u64 {
        fn visit(node: &Node, max: &mut u64) {
            match node {
                Node::Group { group, .. } => {
                    if let InternalDockGroupKey::Generated(id) = group {
                        *max = (*max).max(*id);
                    }
                }
                Node::Split { children, .. } => {
                    for child in children {
                        visit(&child.node, max);
                    }
                }
            }
        }
        let mut max = 0;
        if let Some(root) = &self.workspace.main_root {
            visit(root, &mut max);
        }
        for floating in &self.workspace.floating_roots {
            visit(&floating.root, &mut max);
        }
        for side in &self.workspace.auto_hide {
            for entry in side {
                if let InternalDockGroupKey::Generated(id) = entry.return_state.group {
                    max = max.max(id);
                }
            }
        }
        for entry in &self.workspace.closed {
            if let InternalDockGroupKey::Generated(id) = entry.return_state.group {
                max = max.max(id);
            }
        }
        max
    }

    fn normalize(&mut self) {
        if self.is_empty() {
            if let Some(default_definition) = &self.default_definition {
                if self.workspace.main_root.is_none() {
                    self.workspace.main_root = default_definition.root.clone();
                }
            }
        }
        let registered = self.default_definition.as_ref().map(|definition| {
            definition
                .item_groups
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>()
        });
        let valid_groups = self
            .default_definition
            .as_ref()
            .map(|definition| &definition.groups);
        let keep_empty_groups = self
            .default_definition
            .as_ref()
            .map(|definition| &definition.keep_empty_groups);
        let mut orphaned = Vec::new();
        let mut seen = HashSet::new();
        self.workspace.main_root = self.workspace.main_root.take().and_then(|root| {
            normalize_node(
                root,
                registered.as_ref(),
                valid_groups,
                keep_empty_groups,
                &mut seen,
                &mut orphaned,
            )
        });
        let mut floating_roots = Vec::new();
        let mut floating_index_map = Vec::new();
        for floating in self.workspace.floating_roots.drain(..) {
            if let Some(root) = normalize_node(
                floating.root,
                registered.as_ref(),
                valid_groups,
                keep_empty_groups,
                &mut seen,
                &mut orphaned,
            ) {
                floating_index_map.push(Some(floating_roots.len()));
                floating_roots.push(FloatingRoot {
                    bounds: floating.bounds,
                    root,
                });
            } else {
                floating_index_map.push(None);
            }
        }
        self.workspace.floating_roots = floating_roots;
        for entry in self.workspace.auto_hide.iter_mut().flatten() {
            repair_return_state(
                &mut entry.return_state,
                &entry.item,
                self.default_definition.as_deref(),
            );
            entry.return_state.floating_root = entry
                .return_state
                .floating_root
                .and_then(|index| floating_index_map.get(index).copied().flatten());
        }
        for entry in &mut self.workspace.closed {
            repair_return_state(
                &mut entry.return_state,
                &entry.item,
                self.default_definition.as_deref(),
            );
            entry.return_state.floating_root = entry
                .return_state
                .floating_root
                .and_then(|index| floating_index_map.get(index).copied().flatten());
        }

        let mut open_seen = false;
        for side in DockSide::ALL {
            let entries = std::mem::take(&mut self.workspace.auto_hide[side.index()]);
            self.workspace.auto_hide[side.index()] = entries
                .into_iter()
                .filter_map(|mut entry| {
                    if registered
                        .as_ref()
                        .is_some_and(|registered| !registered.contains(&entry.item))
                        || !seen.insert(entry.item.clone())
                    {
                        return None;
                    }
                    if entry.open {
                        if open_seen {
                            entry.open = false;
                        } else {
                            open_seen = true;
                        }
                    }
                    Some(entry)
                })
                .collect();
        }
        self.workspace.closed.retain(|entry| {
            registered
                .as_ref()
                .is_none_or(|registered| registered.contains(&entry.item))
                && seen.insert(entry.item.clone())
        });

        if let Some(default_definition) = self.default_definition.clone() {
            let mut missing = default_definition
                .item_groups
                .keys()
                .filter(|item| !seen.contains(*item))
                .cloned()
                .collect::<Vec<_>>();
            missing.extend(orphaned.into_iter().filter(|item| !seen.contains(item)));
            missing.sort();
            missing.dedup();
            for item in missing {
                let Some(group) = default_definition.item_groups.get(&item) else {
                    continue;
                };
                let key = InternalDockGroupKey::Authored(group.clone());
                ensure_authored_group(
                    &mut self.workspace.main_root,
                    &key,
                    default_definition.root.as_ref(),
                );
                if !self
                    .workspace
                    .main_root
                    .as_mut()
                    .is_some_and(|root| append_to_group(root, &key, item.clone(), None))
                {
                    self.insert_root_fallback(item);
                }
            }
        }
        if let Some(active) = self.workspace.active_item.clone() {
            let active_is_live = !self.is_item_closed(&active)
                && (self
                    .workspace
                    .main_root
                    .as_ref()
                    .is_some_and(|root| node_contains_item(root, &active))
                    || self
                        .workspace
                        .floating_roots
                        .iter()
                        .any(|root| node_contains_item(&root.root, &active))
                    || self
                        .workspace
                        .auto_hide
                        .iter()
                        .flatten()
                        .any(|entry| entry.item == active));
            if !active_is_live {
                let fallback_group = self
                    .workspace
                    .closed
                    .iter()
                    .find(|entry| entry.item == active)
                    .map(|entry| entry.return_state.group.clone());
                self.workspace.active_item = fallback_group
                    .as_ref()
                    .and_then(|group| self.selected_item_in_group(group))
                    .or_else(|| self.selected_item_id());
            }
        }
        if self.workspace.next_generated_group_id <= self.max_generated_group_id() {
            self.workspace.next_generated_group_id = self.max_generated_group_id() + 1;
        }
    }
}

fn validate_placement(placement: &DockPlacement) -> Result<(), DockLayoutError> {
    match placement {
        DockPlacement::SplitGroup { weight, .. } | DockPlacement::RootEdge { weight, .. }
            if !valid_weight(*weight) =>
        {
            Err(DockLayoutError::InvalidWeight)
        }
        DockPlacement::Floating { bounds } if !valid_bounds(*bounds) => {
            Err(DockLayoutError::InvalidBounds)
        }
        _ => Ok(()),
    }
}

fn collect_node_items(root: &Option<Node>, out: &mut Vec<DockItemId>) {
    fn visit(node: &Node, out: &mut Vec<DockItemId>) {
        match node {
            Node::Group { items, .. } => out.extend(items.iter().cloned()),
            Node::Split { children, .. } => {
                for child in children {
                    visit(&child.node, out);
                }
            }
        }
    }
    if let Some(root) = root {
        visit(root, out);
    }
}

fn node_contains_item(node: &Node, item: &DockItemId) -> bool {
    match node {
        Node::Group { items, .. } => items.iter().any(|candidate| candidate == item),
        Node::Split { children, .. } => children
            .iter()
            .any(|child| node_contains_item(&child.node, item)),
    }
}

fn node_at_path_mut<'a>(root: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    if path.is_empty() {
        return Some(root);
    }
    let Node::Split { children, .. } = root else {
        return None;
    };
    let child = children.get_mut(path[0])?;
    node_at_path_mut(&mut child.node, &path[1..])
}

fn select_item(root: &mut Option<Node>, item: &DockItemId) -> bool {
    root.as_mut().is_some_and(|root| select_node(root, item))
}

fn select_node(node: &mut Node, item: &DockItemId) -> bool {
    fn visit(node: &mut Node, item: &DockItemId) -> bool {
        match node {
            Node::Group {
                items, selected, ..
            } => {
                if items.contains(item) {
                    let changed = selected.as_ref() != Some(item);
                    *selected = Some(item.clone());
                    changed
                } else {
                    false
                }
            }
            Node::Split { children, .. } => children
                .iter_mut()
                .any(|child| visit(&mut child.node, item)),
        }
    }
    visit(node, item)
}

fn remove_from_node(node: &mut Node, item: &DockItemId) -> Option<(InternalDockGroupKey, usize)> {
    match node {
        Node::Group {
            group,
            items,
            selected,
        } => {
            let index = items.iter().position(|candidate| candidate == item)?;
            items.remove(index);
            if selected.as_ref() == Some(item) {
                *selected = items.first().cloned();
            }
            Some((group.clone(), index))
        }
        Node::Split { children, .. } => {
            for child in children {
                if let Some(found) = remove_from_node(&mut child.node, item) {
                    return Some(found);
                }
            }
            None
        }
    }
}

fn append_to_group(
    node: &mut Node,
    target: &InternalDockGroupKey,
    item: DockItemId,
    index: Option<usize>,
) -> bool {
    match node {
        Node::Group {
            group,
            items,
            selected,
        } if group == target => {
            let index = index.unwrap_or(items.len()).min(items.len());
            items.insert(index, item.clone());
            *selected = Some(item);
            true
        }
        Node::Group { .. } => false,
        Node::Split { children, .. } => children
            .iter_mut()
            .any(|child| append_to_group(&mut child.node, target, item.clone(), index)),
    }
}

fn append_group_items(
    node: &mut Node,
    target: &InternalDockGroupKey,
    source_items: &[DockItemId],
    source_selected: Option<DockItemId>,
) -> bool {
    match node {
        Node::Group {
            group,
            items,
            selected,
        } if group == target => {
            items.extend(source_items.iter().cloned());
            if source_selected.is_some() {
                *selected = source_selected;
            }
            true
        }
        Node::Group { .. } => false,
        Node::Split { children, .. } => children.iter_mut().any(|child| {
            append_group_items(
                &mut child.node,
                target,
                source_items,
                source_selected.clone(),
            )
        }),
    }
}

fn detach_group(node: Node, target: &InternalDockGroupKey) -> (Option<Node>, Option<Node>) {
    match node {
        Node::Group {
            group,
            items,
            selected,
        } if &group == target => (
            None,
            Some(Node::Group {
                group,
                items,
                selected,
            }),
        ),
        Node::Group { .. } => (Some(node), None),
        Node::Split {
            orientation,
            mut children,
        } => {
            let mut index = 0;
            while index < children.len() {
                let child = children.remove(index);
                let (remaining, detached) = detach_group(child.node, target);
                if let Some(node) = remaining {
                    children.insert(
                        index,
                        WeightedNode {
                            weight: child.weight,
                            node,
                        },
                    );
                }
                if detached.is_some() {
                    return (
                        Some(Node::Split {
                            orientation,
                            children,
                        }),
                        detached,
                    );
                }
                index += 1;
            }
            (
                Some(Node::Split {
                    orientation,
                    children,
                }),
                None,
            )
        }
    }
}

fn split_group(
    node: &mut Node,
    target: &InternalDockGroupKey,
    new_group: Node,
    side: DockSide,
    weight: f32,
) -> bool {
    match node {
        Node::Group { group, .. } if group == target => {
            let old = std::mem::replace(node, new_group.clone());
            let orientation = side.orientation();
            let first = WeightedNode {
                weight: if side.is_leading() { weight } else { 1.0 },
                node: if side.is_leading() {
                    new_group.clone()
                } else {
                    old.clone()
                },
            };
            let second = WeightedNode {
                weight: if side.is_leading() { 1.0 } else { weight },
                node: if side.is_leading() { old } else { new_group },
            };
            *node = Node::Split {
                orientation,
                children: vec![first, second],
            };
            true
        }
        Node::Group { .. } => false,
        Node::Split { children, .. } => children
            .iter_mut()
            .any(|child| split_group(&mut child.node, target, new_group.clone(), side, weight)),
    }
}

fn normalize_node(
    node: Node,
    registered: Option<&BTreeSet<DockItemId>>,
    valid_groups: Option<&BTreeSet<DockGroupId>>,
    keep_empty_groups: Option<&BTreeSet<DockGroupId>>,
    seen: &mut HashSet<DockItemId>,
    orphaned: &mut Vec<DockItemId>,
) -> Option<Node> {
    match node {
        Node::Group {
            group,
            items,
            selected,
        } => {
            if let InternalDockGroupKey::Authored(group_id) = &group
                && valid_groups.is_some_and(|groups| !groups.contains(group_id))
            {
                for item in items {
                    if registered.is_none_or(|registered| registered.contains(&item))
                        && !seen.contains(&item)
                    {
                        orphaned.push(item);
                    }
                }
                return None;
            }
            let items = items
                .into_iter()
                .filter(|item| {
                    registered.is_none_or(|registered| registered.contains(item))
                        && seen.insert(item.clone())
                })
                .collect::<Vec<_>>();
            let selected = selected
                .filter(|item| items.contains(item))
                .or_else(|| items.first().cloned());
            let remove_empty = match &group {
                InternalDockGroupKey::Generated(_) => items.is_empty(),
                InternalDockGroupKey::Authored(group) => {
                    items.is_empty()
                        && keep_empty_groups.is_some_and(|groups| !groups.contains(group))
                }
            };
            (!remove_empty).then_some(Node::Group {
                group,
                items,
                selected,
            })
        }
        Node::Split {
            orientation,
            children,
        } => {
            let mut children = children
                .into_iter()
                .filter_map(|child| {
                    normalize_node(
                        child.node,
                        registered,
                        valid_groups,
                        keep_empty_groups,
                        seen,
                        orphaned,
                    )
                    .map(|node| WeightedNode {
                        weight: child.weight,
                        node,
                    })
                })
                .collect::<Vec<_>>();
            if children.is_empty() {
                return None;
            }
            if children.len() == 1 {
                return children.pop().map(|child| child.node);
            }
            let total = children.iter().map(|child| child.weight).sum::<f32>();
            if total.is_finite() && total > 0.0 {
                for child in &mut children {
                    child.weight = child.weight / total;
                }
            } else {
                let weight = 1.0 / children.len() as f32;
                for child in &mut children {
                    child.weight = weight;
                }
            }
            Some(Node::Split {
                orientation,
                children,
            })
        }
    }
}

fn repair_return_state(
    state: &mut ReturnState,
    item: &DockItemId,
    default_definition: Option<&DefaultDockDefinition>,
) {
    let Some(default_definition) = default_definition else {
        return;
    };
    let valid = match &state.group {
        InternalDockGroupKey::Authored(group) => default_definition.groups.contains(group),
        InternalDockGroupKey::Generated(_) => true,
    };
    if !valid {
        state.group = default_definition
            .item_groups
            .get(item)
            .cloned()
            .map(InternalDockGroupKey::Authored)
            .unwrap_or(InternalDockGroupKey::Generated(0));
        state.index = usize::MAX;
        state.floating_root = None;
    }
}

fn find_authored_group(root: &Node, target: &DockGroupId) -> Option<Node> {
    match root {
        Node::Group { group, .. } => (group == &InternalDockGroupKey::Authored(target.clone()))
            .then(|| Node::Group {
                group: InternalDockGroupKey::Authored(target.clone()),
                items: Vec::new(),
                selected: None,
            }),
        Node::Split { children, .. } => children
            .iter()
            .find_map(|child| find_authored_group(&child.node, target)),
    }
}

fn empty_authored_root(root: &Node) -> Node {
    match root {
        Node::Group { group, .. } => Node::Group {
            group: group.clone(),
            items: Vec::new(),
            selected: None,
        },
        Node::Split {
            orientation,
            children,
        } => Node::Split {
            orientation: *orientation,
            children: children
                .iter()
                .map(|child| WeightedNode {
                    weight: child.weight,
                    node: empty_authored_root(&child.node),
                })
                .collect(),
        },
    }
}

fn contains_group(root: &Node, target: &InternalDockGroupKey) -> bool {
    match root {
        Node::Group { group, .. } => group == target,
        Node::Split { children, .. } => children
            .iter()
            .any(|child| contains_group(&child.node, target)),
    }
}

fn ensure_authored_group(
    root: &mut Option<Node>,
    target: &InternalDockGroupKey,
    default_root: Option<&Node>,
) -> bool {
    if root
        .as_ref()
        .is_some_and(|root| contains_group(root, target))
    {
        return true;
    }
    let InternalDockGroupKey::Authored(group_id) = target else {
        return false;
    };
    let Some(default_root) = default_root else {
        return false;
    };
    let Some(group) = find_authored_group(default_root, group_id) else {
        return false;
    };
    match root.take() {
        None => *root = Some(group),
        Some(existing) => {
            *root = Some(Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    WeightedNode {
                        weight: 1.0,
                        node: existing,
                    },
                    WeightedNode {
                        weight: 1.0,
                        node: group,
                    },
                ],
            });
        }
    }
    true
}

impl DockSide {
    fn is_leading(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }

    fn orientation(self) -> Orientation {
        match self {
            Self::Left | Self::Right => Orientation::Horizontal,
            Self::Top | Self::Bottom => Orientation::Vertical,
        }
    }
}

impl From<&Node> for SnapshotNode {
    fn from(value: &Node) -> Self {
        match value {
            Node::Split {
                orientation,
                children,
            } => Self::Split {
                orientation: (*orientation).into(),
                children: children
                    .iter()
                    .map(|child| SnapshotWeightedNode {
                        weight: child.weight,
                        node: (&child.node).into(),
                    })
                    .collect(),
            },
            Node::Group {
                group,
                items,
                selected,
            } => Self::Group {
                group: group.into(),
                items: items.clone(),
                selected: selected.clone(),
            },
        }
    }
}

impl TryFrom<SnapshotNode> for Node {
    type Error = DockLayoutError;

    fn try_from(value: SnapshotNode) -> Result<Self, Self::Error> {
        Ok(match value {
            SnapshotNode::Split {
                orientation,
                children,
            } => Node::Split {
                orientation: orientation.into(),
                children: children
                    .into_iter()
                    .map(|child| {
                        Ok(WeightedNode {
                            weight: child.weight,
                            node: child.node.try_into()?,
                        })
                    })
                    .collect::<Result<_, DockLayoutError>>()?,
            },
            SnapshotNode::Group {
                group,
                items,
                selected,
            } => Node::Group {
                group: group.into(),
                items,
                selected,
            },
        })
    }
}

impl From<&InternalDockGroupKey> for SnapshotGroupKey {
    fn from(value: &InternalDockGroupKey) -> Self {
        match value {
            InternalDockGroupKey::Authored(id) => Self::Authored(id.clone()),
            InternalDockGroupKey::Generated(id) => Self::Generated(*id),
        }
    }
}

impl From<SnapshotGroupKey> for InternalDockGroupKey {
    fn from(value: SnapshotGroupKey) -> Self {
        match value {
            SnapshotGroupKey::Authored(id) => Self::Authored(id),
            SnapshotGroupKey::Generated(id) => Self::Generated(id),
        }
    }
}

impl From<&ReturnState> for SnapshotReturnState {
    fn from(value: &ReturnState) -> Self {
        Self {
            group: (&value.group).into(),
            index: value.index,
            floating_root: value.floating_root,
        }
    }
}

impl TryFrom<&SnapshotReturnState> for ReturnState {
    type Error = DockLayoutError;

    fn try_from(value: &SnapshotReturnState) -> Result<Self, Self::Error> {
        Ok(Self {
            group: value.group.clone().into(),
            index: value.index,
            floating_root: value.floating_root,
        })
    }
}

impl TryFrom<&SnapshotAutoHideEntry> for AutoHideEntry {
    type Error = DockLayoutError;

    fn try_from(value: &SnapshotAutoHideEntry) -> Result<Self, Self::Error> {
        Ok(Self {
            item: value.item.clone(),
            open: value.open,
            return_state: (&value.return_state).try_into()?,
        })
    }
}

impl TryFrom<&SnapshotClosedEntry> for ClosedEntry {
    type Error = DockLayoutError;

    fn try_from(value: &SnapshotClosedEntry) -> Result<Self, Self::Error> {
        Ok(Self {
            item: value.item.clone(),
            return_state: (&value.return_state).try_into()?,
        })
    }
}
