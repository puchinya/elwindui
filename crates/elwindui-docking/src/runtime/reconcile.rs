//! Reconciliation boundary between the value model and stable runtime item wrappers.

use crate::core::base::{Point, Rect};
use crate::core::input::PointerEventArgs;
use crate::core::layout::{GridLength, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    Grid, GridExt, LayoutExt, TextBlock, TextBlockExt, TextStyleOwner, UIElementExt,
};
use crate::model::{RootKind, SplitAddress};
use crate::snapshot::{SnapshotAutoHideEntry, SnapshotGroupKey, SnapshotNode, SnapshotOrientation};
use crate::{
    DockGroup, DockGroupId, DockItem, DockItemId, DockLayoutError, DockLayoutModel, DockSplitPanel,
};
use elwindui_custom_controls::{
    CustomSplitter, CustomSplitterExt, CustomTabView, CustomTabViewExt, CustomTabViewItem,
    CustomTabViewItemExt, SplitterDragCompletedEventArgs, SplitterDragDeltaEventArgs,
    SplitterDragStartedEventArgs, TabStripPosition,
};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::rc::Weak;

use super::drag::{DragSession, DragSourceGeometry, ResolvedDockTarget};
#[cfg(test)]
use super::floating_window::FloatingHostFactory;
use super::floating_window::{FloatingHostId, FloatingHostRegistry, PreparedFloatingHostSync};
use super::group_view::replace_group_items;
use super::split_view::SplitterSession;
use super::surface_registry::SurfaceRegistry;
use super::surface_view::{DockSurfaceView, SurfaceRuntime};
use super::themed_brush;

/// Stable registration-to-presentation map for one authored docking surface.
///
/// The map owns one `CustomTabViewItem` per authored `DockItem`. A layout reconciliation only
/// changes the parent tab view; it never reconstructs the page wrapper. This is the ownership
/// boundary that prevents a selected tab, floating host, and auto-hide overlay from each creating
/// a second logical page owner.
#[derive(Default)]
pub(crate) struct StableItemRegistry {
    items: BTreeMap<DockItemId, Rc<DockItem>>,
    wrappers: BTreeMap<DockItemId, Rc<CustomTabViewItem>>,
    group_positions: BTreeMap<DockGroupId, TabStripPosition>,
}

impl StableItemRegistry {
    pub(crate) fn from_authored(root: &dyn UIElementExt) -> Result<Self, DockLayoutError> {
        let mut registry = Self::default();
        registry.refresh_authored(root)?;
        Ok(registry)
    }

    /// Reconciles registration changes while retaining wrappers for IDs that still exist.
    pub(crate) fn refresh_authored(
        &mut self,
        root: &dyn UIElementExt,
    ) -> Result<(), DockLayoutError> {
        let mut items = BTreeMap::new();
        let mut groups = BTreeMap::new();
        collect_authored(root, &mut items, &mut groups)?;

        let removed = self
            .items
            .keys()
            .filter(|id| !items.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in removed {
            self.items.remove(&id);
            self.wrappers.remove(&id);
        }

        for (id, item) in items {
            let wrapper = self
                .wrappers
                .entry(id.clone())
                .or_insert_with(|| item.to_tab_item());
            // Metadata may be reactive even when the identity is not. Updating the existing
            // wrapper is safe because its ContentControl content remains the same logical slot.
            wrapper.set_header(item.title_value());
            wrapper.set_icon(item.icon_value());
            wrapper.set_closable(item.can_close_value());
            // Page replacement is deliberately not a V1 registration operation. Keeping the
            // existing wrapper content untouched preserves page identity and avoids an implicit
            // unmount/remount during metadata-only authored refreshes.
            self.items.insert(id, item);
        }
        self.group_positions = groups;
        Ok(())
    }

    pub(crate) fn wrapper(&self, id: &DockItemId) -> Option<Rc<CustomTabViewItem>> {
        self.wrappers.get(id).cloned()
    }

    pub(crate) fn group_position(&self, id: &DockGroupId) -> TabStripPosition {
        self.group_positions
            .get(id)
            .copied()
            .unwrap_or(TabStripPosition::Top)
    }
}

fn collect_authored(
    node: &dyn UIElementExt,
    items: &mut BTreeMap<DockItemId, Rc<DockItem>>,
    groups: &mut BTreeMap<DockGroupId, TabStripPosition>,
) -> Result<(), DockLayoutError> {
    if let Some(group) = node.as_any().downcast_ref::<DockGroup>() {
        let id = group.id_value();
        if id.as_ref().is_empty()
            || groups
                .insert(id.clone(), group.tab_strip_position_value())
                .is_some()
        {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: format!("duplicate or empty authored DockGroupId: {id}"),
            });
        }
        for item in group.authored_children() {
            let item_id = item.id_value();
            if item_id.as_ref().is_empty() || items.insert(item_id.clone(), item).is_some() {
                return Err(DockLayoutError::InvalidSnapshot {
                    reason: format!("duplicate or empty authored DockItemId: {item_id}"),
                });
            }
        }
        return Ok(());
    }
    if let Some(panel) = node.as_any().downcast_ref::<DockSplitPanel>() {
        for child in panel.authored_children() {
            collect_authored(child.as_ref(), items, groups)?;
        }
        return Ok(());
    }
    Err(DockLayoutError::InvalidSnapshot {
        reason: "authored docking root contains an unsupported element".to_owned(),
    })
}

/// The private realization tree kept by `DockingControl`.
pub(crate) enum RuntimeNode {
    Group {
        host: Rc<Grid>,
    },
    Split {
        children: Vec<RuntimeNode>,
        grid: Rc<Grid>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimePresentationOwner {
    Group(SnapshotGroupKey),
    AutoHide {
        root: RootKind,
    },
    FloatingGroup {
        root: usize,
        group: SnapshotGroupKey,
    },
    None,
}

struct FloatingRuntime {
    node: RuntimeNode,
    identity: Vec<SnapshotGroupKey>,
    surface: SurfaceRuntime,
}

impl RuntimeNode {
    fn element(&self) -> Rc<dyn UIElementExt> {
        match self {
            Self::Group { host, .. } => host.clone(),
            Self::Split { grid, .. } => grid.clone(),
        }
    }
}

#[derive(Clone)]
struct GroupRuntimeHost {
    container: Rc<Grid>,
    title_bar: Rc<Grid>,
    title: Rc<TextBlock>,
    pin_button: Rc<Grid>,
    close_button: Rc<Grid>,
}

struct PlannedGroup {
    view: Rc<CustomTabView>,
    host: GroupRuntimeHost,
    tabs: Vec<Rc<CustomTabViewItem>>,
    items: Vec<DockItemId>,
    selected: Option<DockItemId>,
    tab_position: TabStripPosition,
    title: String,
    title_visibility: Visibility,
    pin_visibility: Visibility,
    close_visibility: Visibility,
}

struct PlannedSplit {
    grid: Rc<Grid>,
    splitters: Vec<Rc<CustomSplitter>>,
    orientation: SnapshotOrientation,
    weights: Vec<f32>,
}

struct PlannedFloatingRuntime {
    bounds: crate::Rect,
    node: RuntimeNode,
    identity: Vec<SnapshotGroupKey>,
    surface: Rc<DockSurfaceView>,
}

/// A complete candidate realization. Planning only derives facts and constructs unattached
/// candidate controls; `commit_reconcile` is the sole place that changes visual ownership.
struct ReconcilePlan {
    snapshot: crate::snapshot::DockLayoutSnapshot,
    desired_owners: BTreeMap<DockItemId, RuntimePresentationOwner>,
    planned_groups: BTreeMap<SnapshotGroupKey, PlannedGroup>,
    planned_splits: BTreeMap<SplitAddress, PlannedSplit>,
    group_items: BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
    group_roots: BTreeMap<SnapshotGroupKey, RootKind>,
    group_selected: BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
    auto_hide_roots: BTreeMap<DockItemId, RootKind>,
    root: Option<RuntimeNode>,
    floating: Vec<PlannedFloatingRuntime>,
    surfaces: SurfaceRegistry,
    main_surface_child: Option<Rc<dyn UIElementExt>>,
    open_auto_hide: Vec<(RootKind, DockItemId)>,
    host_sync: PreparedFloatingHostSync,
}

pub struct RuntimeRealization {
    registry: StableItemRegistry,
    groups: BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
    group_hosts: BTreeMap<SnapshotGroupKey, GroupRuntimeHost>,
    group_items: BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
    group_roots: BTreeMap<SnapshotGroupKey, RootKind>,
    group_selected: BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
    auto_hide_roots: BTreeMap<DockItemId, RootKind>,
    owners: BTreeMap<DockItemId, RuntimePresentationOwner>,
    root: Option<RuntimeNode>,
    floating: Vec<FloatingRuntime>,
    split_views: BTreeMap<SplitAddress, (Rc<Grid>, Vec<Rc<CustomSplitter>>)>,
    drag: Option<DragSession>,
    splitter: Option<SplitterSession>,
    floating_hosts: FloatingHostRegistry,
    surfaces: SurfaceRegistry,
    main_surface: SurfaceRuntime,
    surface_root: Rc<Grid>,
    main_surface_child: Option<Rc<dyn UIElementExt>>,
    owner: Weak<crate::DockingControl>,
    reconciling: Rc<Cell<bool>>,
    #[cfg(test)]
    fail_after_reconcile_plan: bool,
    #[cfg(test)]
    full_reconcile_count: usize,
}

/// Source updates use a latest-only queue while a realization is being applied.
#[cfg(test)]
pub(crate) struct LatestOnlyQueue {
    applying: bool,
    pending: Option<DockLayoutModel>,
}

#[cfg(test)]
impl LatestOnlyQueue {
    pub(crate) fn new() -> Self {
        Self {
            applying: false,
            pending: None,
        }
    }

    pub(crate) fn request(
        &mut self,
        current: &DockLayoutModel,
        next: DockLayoutModel,
    ) -> Option<DockLayoutModel> {
        if !self.applying && current == &next {
            return None;
        }
        if self.applying {
            self.pending = Some(next);
            None
        } else {
            self.applying = true;
            Some(next)
        }
    }

    pub(crate) fn finish(&mut self) -> Option<DockLayoutModel> {
        if let Some(next) = self.pending.take() {
            Some(next)
        } else {
            self.applying = false;
            None
        }
    }
}

impl RuntimeRealization {
    pub(crate) fn from_authored(
        root: &dyn UIElementExt,
        surface: Rc<DockSurfaceView>,
        owner: Weak<crate::DockingControl>,
    ) -> Result<Self, DockLayoutError> {
        let main_surface = SurfaceRuntime::new(RootKind::Main, surface.clone(), &owner);
        let surface_root = surface.content_root();
        let mut surfaces = SurfaceRegistry::default();
        let surface_node: Rc<dyn UIElementExt> = surface.clone();
        surfaces.register(RootKind::Main, &surface_node);
        Ok(Self {
            registry: StableItemRegistry::from_authored(root)?,
            groups: BTreeMap::new(),
            group_hosts: BTreeMap::new(),
            group_items: BTreeMap::new(),
            group_roots: BTreeMap::new(),
            group_selected: BTreeMap::new(),
            auto_hide_roots: BTreeMap::new(),
            owners: BTreeMap::new(),
            root: None,
            floating: Vec::new(),
            split_views: BTreeMap::new(),
            drag: None,
            splitter: None,
            floating_hosts: FloatingHostRegistry::default(),
            surfaces,
            main_surface,
            surface_root,
            main_surface_child: None,
            owner,
            reconciling: Rc::new(Cell::new(false)),
            #[cfg(test)]
            fail_after_reconcile_plan: false,
            #[cfg(test)]
            full_reconcile_count: 0,
        })
    }

    pub(crate) fn refresh_authored(
        &mut self,
        root: &dyn UIElementExt,
    ) -> Result<(), DockLayoutError> {
        self.registry.refresh_authored(root)?;
        // An authored registration change can invalidate the item or splitter captured by a
        // native gesture. Cancel both transient sessions before the next model reconciliation;
        // this is safer than allowing a stale wrapper to commit into the new registry.
        self.drag = None;
        if let Some(mut splitter) = self.splitter.take() {
            splitter.cancel();
        }
        self.clear_previews();
        Ok(())
    }

    pub(crate) fn cancel_transient(&mut self) {
        self.drag = None;
        if let Some(mut splitter) = self.splitter.take() {
            splitter.cancel();
        }
        self.clear_previews();
    }

    /// Test-only convenience that exercises the complete staged protocol. Production callers
    /// must let `DockingControl` finalize the prepared native-host sync after owner publication.
    #[cfg(test)]
    pub(crate) fn reconcile_for_test(
        &mut self,
        model: &DockLayoutModel,
    ) -> Result<(), DockLayoutError> {
        let host_sync = self.apply_staged(model)?;
        self.floating_hosts.commit_sync(host_sync);
        Ok(())
    }

    pub(crate) fn apply_staged(
        &mut self,
        model: &DockLayoutModel,
    ) -> Result<PreparedFloatingHostSync, DockLayoutError> {
        let plan = self.prepare_reconcile(model)?;
        Ok(self.commit_reconcile(plan))
    }

    pub(crate) fn commit_floating_host_sync(&mut self, host_sync: PreparedFloatingHostSync) {
        self.floating_hosts.commit_sync(host_sync);
    }

    fn prepare_reconcile(
        &mut self,
        model: &DockLayoutModel,
    ) -> Result<ReconcilePlan, DockLayoutError> {
        let snapshot = model.snapshot();
        crate::snapshot::validate_snapshot(&snapshot)?;
        let desired_owners = desired_owners(&snapshot);
        let floating_count = snapshot.floating_roots.len();
        let mut auto_hide_roots = BTreeMap::new();
        for entry in snapshot.auto_hide.iter().flatten() {
            auto_hide_roots.insert(entry.item.clone(), auto_hide_root(entry, floating_count));
        }

        let mut groups = self.groups.clone();
        let mut group_hosts = self.group_hosts.clone();
        let mut planned_groups = BTreeMap::new();
        let mut planned_splits = BTreeMap::new();
        let mut group_items = BTreeMap::new();
        let mut group_roots = BTreeMap::new();
        let mut group_selected = BTreeMap::new();
        let mut used_groups = BTreeSet::new();
        let mut used_splits = BTreeSet::new();

        let root = snapshot
            .main_root
            .as_ref()
            .map(|node| {
                self.plan_node(
                    node,
                    &mut groups,
                    &mut group_hosts,
                    &mut planned_groups,
                    &mut planned_splits,
                    &mut group_items,
                    &mut group_roots,
                    &mut group_selected,
                    &mut used_groups,
                    &mut used_splits,
                    RootKind::Main,
                    &[],
                )
            })
            .transpose()?;

        let mut floating = Vec::with_capacity(snapshot.floating_roots.len());
        for (floating_index, floating_root) in snapshot.floating_roots.iter().enumerate() {
            let root_kind = RootKind::Floating(floating_index);
            let node = self.plan_node(
                &floating_root.root,
                &mut groups,
                &mut group_hosts,
                &mut planned_groups,
                &mut planned_splits,
                &mut group_items,
                &mut group_roots,
                &mut group_selected,
                &mut used_groups,
                &mut used_splits,
                root_kind.clone(),
                &[],
            )?;
            let identity = group_identity(&floating_root.root);
            let surface = self
                .floating
                .iter()
                .find(|runtime| runtime.identity == identity)
                .map(|runtime| runtime.surface.surface.clone())
                .unwrap_or_else(DockSurfaceView::empty_surface);
            floating.push(PlannedFloatingRuntime {
                bounds: floating_root.bounds.into(),
                node,
                identity,
                surface,
            });
        }

        groups.retain(|key, _| used_groups.contains(key));
        group_hosts.retain(|key, _| used_groups.contains(key));
        planned_groups.retain(|key, _| used_groups.contains(key));
        planned_splits.retain(|key, _| used_splits.contains(key));

        let mut surfaces = SurfaceRegistry::default();
        for (index, runtime) in floating.iter().enumerate() {
            let surface_node: Rc<dyn UIElementExt> = runtime.surface.clone();
            surfaces.register(RootKind::Floating(index), &surface_node);
        }
        let main_surface_node: Rc<dyn UIElementExt> = self.main_surface.surface.clone();
        surfaces.register(RootKind::Main, &main_surface_node);

        let open_auto_hide = snapshot
            .auto_hide
            .iter()
            .flatten()
            .filter(|entry| entry.open)
            .map(|entry| (auto_hide_root(entry, floating_count), entry.item.clone()))
            .collect::<Vec<_>>();
        let floating_specs = floating
            .iter()
            .map(|runtime| (runtime.bounds, runtime.surface.clone()))
            .collect::<Vec<_>>();
        let host_sync = self
            .floating_hosts
            .prepare_sync(&floating_specs, &self.owner)?;

        #[cfg(test)]
        if self.fail_after_reconcile_plan {
            self.fail_after_reconcile_plan = false;
            host_sync.abort();
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "injected late planning failure".to_owned(),
            });
        }

        Ok(ReconcilePlan {
            snapshot,
            desired_owners,
            planned_groups,
            planned_splits,
            group_items,
            group_roots,
            group_selected,
            auto_hide_roots,
            main_surface_child: root.as_ref().map(RuntimeNode::element),
            root,
            floating,
            surfaces,
            open_auto_hide,
            host_sync,
        })
    }

    fn commit_reconcile(&mut self, plan: ReconcilePlan) -> PreparedFloatingHostSync {
        let ReconcilePlan {
            snapshot,
            desired_owners,
            planned_groups,
            planned_splits,
            group_items,
            group_roots,
            group_selected,
            auto_hide_roots,
            root,
            floating: planned_floating,
            surfaces,
            main_surface_child,
            open_auto_hide,
            host_sync,
        } = plan;

        self.reconciling.set(true);
        let _reconciling_guard = ReconcilingGuard(self.reconciling.clone());
        self.detach_existing_tree();
        self.detach_before_attach(&desired_owners);
        let mut previous_floating = std::mem::take(&mut self.floating);

        for planned in planned_groups.values() {
            self.apply_planned_group(planned);
        }

        if let Some(root_node) = root.as_ref() {
            let _ = self.apply_planned_node(root_node, RootKind::Main, &[], &planned_splits);
        }
        self.main_surface.auto_hide.close();
        self.main_surface.preview.clear();
        self.main_surface.reset_visual_children();
        if let Some(element) = main_surface_child.clone() {
            self.main_surface.add_main_child(element);
        }

        let mut floating = Vec::with_capacity(planned_floating.len());
        for (index, planned) in planned_floating.into_iter().enumerate() {
            let root = RootKind::Floating(index);
            let mut surface = previous_floating
                .iter()
                .position(|runtime| runtime.identity == planned.identity)
                .map(|position| previous_floating.swap_remove(position).surface)
                .unwrap_or_else(|| {
                    SurfaceRuntime::new(root.clone(), planned.surface.clone(), &self.owner)
                });
            surface.set_root(root.clone());
            surface.auto_hide.close();
            surface.preview.clear();
            surface.reset_visual_children();
            let element =
                self.apply_planned_node(&planned.node, root.clone(), &[], &planned_splits);
            surface.add_main_child(element);
            floating.push(FloatingRuntime {
                node: planned.node,
                identity: planned.identity,
                surface,
            });
        }

        let registry = &self.registry;
        let floating_count = snapshot.floating_roots.len();
        let strip_titles = |root: &RootKind| {
            snapshot
                .auto_hide
                .iter()
                .enumerate()
                .flat_map(|(side, entries)| {
                    entries.iter().filter_map(move |entry| {
                        (auto_hide_root(entry, floating_count) == *root).then(|| {
                            registry.items.get(&entry.item).map(|item| {
                                (
                                    side,
                                    entry.item.clone(),
                                    item.title_value(),
                                    item.icon_value(),
                                )
                            })
                        })?
                    })
                })
                .collect::<Vec<_>>()
        };
        self.main_surface
            .render_strips(strip_titles(&RootKind::Main).into_iter(), &self.owner);
        for (index, runtime) in floating.iter().enumerate() {
            runtime.surface.render_strips(
                strip_titles(&RootKind::Floating(index)).into_iter(),
                &self.owner,
            );
        }
        for (root, item) in open_auto_hide {
            if let Some(surface) = match &root {
                RootKind::Main => Some(&mut self.main_surface),
                RootKind::Floating(index) => {
                    floating.get_mut(*index).map(|runtime| &mut runtime.surface)
                }
            } {
                surface.auto_hide.open(item.clone());
                surface
                    .auto_hide
                    .present_open_item(self.registry.wrapper(&item));
            }
        }

        self.groups = planned_groups
            .iter()
            .map(|(key, planned)| (key.clone(), planned.view.clone()))
            .collect();
        self.group_hosts = planned_groups
            .into_iter()
            .map(|(key, planned)| (key, planned.host))
            .collect();
        self.group_items = group_items;
        self.group_roots = group_roots;
        self.group_selected = group_selected;
        self.auto_hide_roots = auto_hide_roots;
        self.owners = desired_owners;
        self.root = root;
        self.floating = floating;
        self.main_surface_child = main_surface_child;
        self.surfaces = surfaces;
        self.split_views = planned_splits
            .into_iter()
            .map(|(address, planned)| (address, (planned.grid, planned.splitters)))
            .collect();
        #[cfg(test)]
        {
            self.full_reconcile_count = self.full_reconcile_count.saturating_add(1);
        }
        host_sync
    }

    pub(crate) fn begin_drag(
        &mut self,
        model: &DockLayoutModel,
        item: DockItemId,
        host_root_position: Point,
    ) -> Result<(), DockLayoutError> {
        let Some(authored) = self.registry.items.get(&item) else {
            return Err(DockLayoutError::UnknownItem(item));
        };
        if !authored.can_dock_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit docking".to_owned(),
            });
        }
        let source_root =
            self.item_root(&item)
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "dock item has no current runtime surface".to_owned(),
                })?;
        let group = self
            .group_items
            .iter()
            .find(|(_, items)| items.iter().any(|candidate| candidate == &item))
            .map(|(group, _)| group)
            .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                reason: "dock item has no current runtime group geometry".to_owned(),
            })?;
        let group_view =
            self.groups
                .get(group)
                .cloned()
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "dock item runtime group is unavailable".to_owned(),
                })?;
        let group_node: Rc<dyn UIElementExt> = group_view;
        let source_bounds_host =
            SurfaceRegistry::bounds_in_host_root(&group_node).ok_or_else(|| {
                DockLayoutError::InvalidSnapshot {
                    reason: "dock item runtime group has no arranged geometry".to_owned(),
                }
            })?;
        let pointer_offset = Point {
            x: host_root_position.x - source_bounds_host.x,
            y: host_root_position.y - source_bounds_host.y,
        };
        if !pointer_offset.x.is_finite() || !pointer_offset.y.is_finite() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock drag pointer offset is not finite".to_owned(),
            });
        }
        let source_geometry = DragSourceGeometry {
            source_root: source_root.clone(),
            source_bounds_host,
            pointer_offset,
        };
        self.drag = Some(DragSession::begin(
            model,
            item,
            source_root,
            source_geometry,
        )?);
        self.clear_previews();
        Ok(())
    }

    pub(crate) fn request_close(
        &mut self,
        model: &DockLayoutModel,
        item: &DockItemId,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let authored = self
            .registry
            .items
            .get(item)
            .ok_or_else(|| DockLayoutError::UnknownItem(item.clone()))?;
        if !authored.can_close_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit closing".to_owned(),
            });
        }
        model.with_item_closed(item)
    }

    pub(crate) fn request_pin(
        &mut self,
        model: &DockLayoutModel,
        item: &DockItemId,
        side: crate::DockSide,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let authored = self
            .registry
            .items
            .get(item)
            .ok_or_else(|| DockLayoutError::UnknownItem(item.clone()))?;
        if !authored.can_pin_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit pinning".to_owned(),
            });
        }
        model.with_item_moved(item, crate::DockPlacement::AutoHide { side })
    }

    pub(crate) fn preview_drag(
        &mut self,
        target: &ResolvedDockTarget,
        weight: f32,
    ) -> Result<(), DockLayoutError> {
        let Some(drag) = self.drag.as_mut() else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock preview requested without an active drag".to_owned(),
            });
        };
        drag.preview(target, weight)?;
        self.clear_previews();
        let Some(surface) = self.surface_runtime_mut(&target.root) else {
            return Err(DockLayoutError::InvalidFloatingRoot {
                index: match target.root {
                    RootKind::Floating(index) => index,
                    RootKind::Main => 0,
                },
            });
        };
        surface.preview.show(target);
        Ok(())
    }

    pub(crate) fn clear_drag_target(&mut self) {
        self.clear_previews();
    }

    pub(crate) fn drag_source_geometry(&self) -> Option<DragSourceGeometry> {
        self.drag
            .as_ref()
            .map(|drag| drag.source_geometry().clone())
    }

    pub(crate) fn floating_candidate(
        &mut self,
        bounds: crate::Rect,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let Some(drag) = self.drag.as_mut() else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "floating candidate requested without an active drag".to_owned(),
            });
        };
        drag.set_floating_candidate(bounds)
    }

    #[cfg(test)]
    pub(crate) fn prepare_floating_host(
        &mut self,
        bounds: crate::Rect,
    ) -> Result<super::floating_window::PreparedFloatingHost, DockLayoutError> {
        self.floating_hosts
            .prepare_new(DockSurfaceView::empty_surface(), bounds, &self.owner)
    }

    pub(crate) fn floating_root_index(&self, id: FloatingHostId) -> Option<usize> {
        self.floating_hosts.root_index_for_host(id)
    }

    #[cfg(test)]
    pub(crate) fn set_floating_host_factory_for_test(&mut self, factory: FloatingHostFactory) {
        self.floating_hosts = FloatingHostRegistry::with_factory(factory);
    }

    #[cfg(test)]
    pub(crate) fn fail_after_reconcile_plan_for_test(&mut self) {
        self.fail_after_reconcile_plan = true;
    }

    #[cfg(test)]
    pub(crate) fn full_reconcile_count_for_test(&self) -> usize {
        self.full_reconcile_count
    }

    #[cfg(test)]
    pub(crate) fn active_drag_for_test(&self) -> bool {
        self.drag.is_some()
    }

    #[cfg(test)]
    pub(crate) fn active_splitter_for_test(&self) -> bool {
        self.splitter.is_some()
    }

    #[cfg(test)]
    pub(crate) fn surface_for_test(&self, root: &RootKind) -> Option<Rc<DockSurfaceView>> {
        self.surface_runtime(root)
            .map(|runtime| runtime.surface.clone())
    }

    #[cfg(test)]
    pub(crate) fn group_for_test(&self, group: &SnapshotGroupKey) -> Option<Rc<CustomTabView>> {
        self.groups.get(group).cloned()
    }

    #[cfg(test)]
    pub(crate) fn main_runtime_root_for_test(&self) -> Option<Rc<dyn UIElementExt>> {
        self.root.as_ref().map(RuntimeNode::element)
    }

    #[cfg(test)]
    pub(crate) fn owner_for_test(&self, item: &DockItemId) -> Option<RuntimePresentationOwner> {
        self.owners.get(item).cloned()
    }

    #[cfg(test)]
    pub(crate) fn surface_chrome_for_test(
        &self,
        root: &RootKind,
    ) -> Option<(Rc<dyn UIElementExt>, Rc<dyn UIElementExt>)> {
        self.surface_runtime(root)
            .map(|runtime| (runtime.auto_hide.visual(), runtime.preview.visual()))
    }

    #[cfg(test)]
    pub(crate) fn preview_for_test(&self, root: &RootKind) -> Option<(crate::DockTarget, Rect)> {
        self.surface_runtime(root)
            .and_then(|runtime| runtime.preview.target().zip(runtime.preview.preview_rect()))
    }

    #[cfg(test)]
    pub(crate) fn show_preview_for_test(&mut self, target: ResolvedDockTarget) {
        self.clear_previews();
        if let Some(surface) = self.surface_runtime_mut(&target.root) {
            surface.preview.show(&target);
        }
    }

    #[cfg(test)]
    pub(crate) fn floating_host_count_for_test(&self) -> usize {
        self.floating_hosts.host_count()
    }

    #[cfg(test)]
    pub(crate) fn surface_registry_count_for_test(&self) -> usize {
        self.surfaces.entries().len()
    }

    pub(crate) fn target_for_drop(
        &self,
        screen_position: Option<Point>,
        host_root_position: Point,
    ) -> Option<ResolvedDockTarget> {
        let source_root = self.drag.as_ref()?.source_root();
        let (root, surface, surface_local_point) = if let Some(screen) = screen_position {
            self.surfaces
                .entries()
                .into_iter()
                .find_map(|(root, surface)| {
                    let host_root_point = surface.screen_to_root(screen)?;
                    let surface_local_point =
                        SurfaceRegistry::host_root_to_surface_local(&surface, host_root_point)?;
                    let bounds = SurfaceRegistry::surface_bounds(&surface)?;
                    contains(bounds, surface_local_point).then_some((
                        root,
                        surface,
                        surface_local_point,
                    ))
                })?
        } else {
            let surface = self.surfaces.surface_for_root(&source_root)?;
            let surface_local_point =
                SurfaceRegistry::host_root_to_surface_local(&surface, host_root_position)?;
            let bounds = SurfaceRegistry::surface_bounds(&surface)?;
            contains(bounds, surface_local_point).then_some((
                source_root,
                surface,
                surface_local_point,
            ))?
        };
        let surface_bounds = SurfaceRegistry::surface_bounds(&surface)?;

        let selected_root = root.clone();
        let groups = self.groups.iter().filter_map(|(key, group)| {
            if self.group_roots.get(key) != Some(&selected_root) {
                return None;
            }
            let group_node: Rc<dyn UIElementExt> = group.clone();
            SurfaceRegistry::bounds_in_surface_local(&group_node, &surface)
                .map(|bounds| (key.clone(), bounds))
        });
        resolve_local_target(root, surface_bounds, surface_local_point, groups)
    }

    pub(crate) fn finish_drag(&mut self, commit: bool) -> Option<DockLayoutModel> {
        let result = self.drag.take().and_then(|mut drag| {
            if commit {
                drag.commit()
            } else {
                Some(drag.cancel())
            }
        });
        self.clear_previews();
        result
    }

    pub(crate) fn begin_splitter(
        &mut self,
        model: &DockLayoutModel,
        address: SplitAddress,
        boundary: usize,
        grid: Rc<Grid>,
        orientation: crate::Orientation,
    ) -> bool {
        self.splitter = SplitterSession::begin(model, address, boundary, grid, orientation.into());
        self.splitter.is_some()
    }

    pub(crate) fn preview_splitter(&mut self, cumulative_delta: f32) {
        if let Some(splitter) = self.splitter.as_mut() {
            splitter.preview(cumulative_delta);
        }
    }

    pub(crate) fn finish_splitter(&mut self, canceled: bool) -> Option<DockLayoutModel> {
        self.splitter.take().and_then(|mut splitter| {
            if canceled {
                splitter.cancel();
                None
            } else {
                splitter.commit()
            }
        })
    }

    #[allow(dead_code)]
    pub(crate) fn open_auto_hide(&mut self, item: DockItemId) -> Option<DockItemId> {
        let root = self.auto_hide_roots.get(&item)?.clone();
        self.open_auto_hide_on(root, item)
    }

    pub(crate) fn present_auto_hide(&self, item: &DockItemId) {
        let Some(root) = self.auto_hide_roots.get(item) else {
            return;
        };
        if let Some(surface) = self.surface_runtime(root) {
            surface
                .auto_hide
                .present_open_item(self.registry.wrapper(item));
        }
    }

    pub(crate) fn open_auto_hide_on(
        &mut self,
        root: RootKind,
        item: DockItemId,
    ) -> Option<DockItemId> {
        self.clear_auto_hide_presentations();
        let wrapper = self.registry.wrapper(&item);
        self.surface_runtime_mut(&root).map(|surface| {
            let previous = surface.auto_hide.open(item.clone());
            surface.auto_hide.present_open_item(wrapper);
            previous
        })?
    }

    pub(crate) fn selected_group_item(&self, group: &SnapshotGroupKey) -> Option<DockItemId> {
        self.group_selected.get(group).and_then(Clone::clone)
    }

    pub(crate) fn selected_group_index(&self, group: &SnapshotGroupKey) -> Option<usize> {
        let selected = self.selected_group_item(group)?;
        self.group_items
            .get(group)?
            .iter()
            .position(|item| item == &selected)
    }

    /// Applies the non-structural part of a live tab selection. The caller has already changed
    /// the retained `CustomTabView` selected index, so this method only accepts a selection when
    /// the model transformation preserves every item/group/root relationship.
    pub(crate) fn apply_selection_fast_path(
        &mut self,
        model: &DockLayoutModel,
        next: &DockLayoutModel,
        group: &SnapshotGroupKey,
        index: usize,
        item: &DockItemId,
    ) -> bool {
        if self.group_item(group, index).as_ref() != Some(item)
            || !self
                .groups
                .get(group)
                .is_some_and(|view| view.selected_index() == index)
            || model.is_item_closed(item)
            || model.is_item_auto_hidden(item)
            || !same_selection_structure(&model.snapshot(), &next.snapshot())
        {
            return false;
        }
        let owner_matches = match self.owners.get(item) {
            Some(RuntimePresentationOwner::Group(current)) => current == group,
            Some(RuntimePresentationOwner::FloatingGroup { group: current, .. }) => {
                current == group
            }
            _ => false,
        };
        if !owner_matches {
            return false;
        }

        self.group_selected
            .insert(group.clone(), Some(item.clone()));
        for owner in self.owners.values_mut() {
            if matches!(owner, RuntimePresentationOwner::AutoHide { .. }) {
                *owner = RuntimePresentationOwner::None;
            }
        }
        self.clear_auto_hide_presentations();
        true
    }

    pub(crate) fn open_auto_hide_item_on(&self, root: &RootKind) -> Option<DockItemId> {
        self.surface_runtime(root)
            .and_then(|surface| surface.auto_hide.current().cloned())
    }

    pub(crate) fn can_pin(&self, item: &DockItemId) -> bool {
        self.registry
            .items
            .get(item)
            .is_some_and(|item| item.can_pin_value())
    }

    pub(crate) fn can_float(&self, item: &DockItemId) -> bool {
        self.registry
            .items
            .get(item)
            .is_some_and(|item| item.can_float_value())
    }

    pub(crate) fn dispose(&mut self) {
        self.detach_existing_tree();
        self.surface_root.children().clear();
        self.drag = None;
        self.splitter = None;
        self.clear_previews();
        self.main_surface.auto_hide.close();
        self.floating_hosts.close_empty();
        self.surfaces.unregister(RootKind::Main);
        self.surfaces = SurfaceRegistry::default();
        self.groups.clear();
        self.group_hosts.clear();
        self.owners.clear();
        self.root = None;
        self.floating.clear();
        self.split_views.clear();
        self.group_items.clear();
        self.group_roots.clear();
        self.group_selected.clear();
        self.auto_hide_roots.clear();
    }

    pub(crate) fn drag_item(&self) -> Option<DockItemId> {
        self.drag.as_ref().map(|drag| drag.item().clone())
    }

    fn plan_node(
        &self,
        node: &SnapshotNode,
        groups: &mut BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
        group_hosts: &mut BTreeMap<SnapshotGroupKey, GroupRuntimeHost>,
        planned_groups: &mut BTreeMap<SnapshotGroupKey, PlannedGroup>,
        planned_splits: &mut BTreeMap<SplitAddress, PlannedSplit>,
        group_items: &mut BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
        group_roots: &mut BTreeMap<SnapshotGroupKey, RootKind>,
        group_selected: &mut BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
        used_groups: &mut BTreeSet<SnapshotGroupKey>,
        used_splits: &mut BTreeSet<SplitAddress>,
        root_kind: RootKind,
        path: &[usize],
    ) -> Result<RuntimeNode, DockLayoutError> {
        match node {
            SnapshotNode::Group {
                group,
                items,
                selected,
            } => {
                used_groups.insert(group.clone());
                group_roots.insert(group.clone(), root_kind.clone());
                let view = if let Some(view) = groups.get(group) {
                    view.clone()
                } else {
                    let view = CustomTabView::new_view();
                    self.wire_group_callbacks(&view, group.clone());
                    groups.insert(group.clone(), view.clone());
                    view
                };
                group_items.insert(group.clone(), items.clone());
                group_selected.insert(group.clone(), selected.clone());
                let tabs = items
                    .iter()
                    .map(|id| {
                        self.registry
                            .wrapper(id)
                            .ok_or_else(|| DockLayoutError::UnknownItem(id.clone()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let tab_position = match &group {
                    SnapshotGroupKey::Authored(id) => self.registry.group_position(id),
                    SnapshotGroupKey::Generated(_) => TabStripPosition::Top,
                };
                let selected_item = selected
                    .as_ref()
                    .and_then(|selected| self.registry.items.get(selected));
                let has_pin = selected_item.is_some_and(|item| item.can_pin_value());
                let show_title_bar = tab_position == TabStripPosition::Bottom || has_pin;
                let title_visibility = if show_title_bar {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                };
                let pin_visibility = if has_pin {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                };
                let close_visibility = if selected_item.is_some_and(|item| item.can_close_value()) {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                };
                let host = group_hosts
                    .entry(group.clone())
                    .or_insert_with(|| self.new_group_host(group))
                    .clone();
                planned_groups.insert(
                    group.clone(),
                    PlannedGroup {
                        view,
                        host: host.clone(),
                        tabs,
                        items: items.clone(),
                        selected: selected.clone(),
                        tab_position,
                        title: selected_item
                            .map(|item| item.title_value())
                            .unwrap_or_default(),
                        title_visibility,
                        pin_visibility,
                        close_visibility,
                    },
                );
                Ok(RuntimeNode::Group {
                    host: host.container.clone(),
                })
            }
            SnapshotNode::Split {
                orientation,
                children: snapshot_children,
            } => {
                let split_address = SplitAddress {
                    root: root_kind.clone(),
                    path: path.to_vec(),
                };
                used_splits.insert(split_address.clone());
                let weights = snapshot_children
                    .iter()
                    .map(|child| child.weight)
                    .collect::<Vec<_>>();
                let children = snapshot_children
                    .iter()
                    .enumerate()
                    .map(|(index, child)| {
                        let mut child_path = path.to_vec();
                        child_path.push(index);
                        self.plan_node(
                            &child.node,
                            groups,
                            group_hosts,
                            planned_groups,
                            planned_splits,
                            group_items,
                            group_roots,
                            group_selected,
                            used_groups,
                            used_splits,
                            root_kind.clone(),
                            &child_path,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let (grid, mut splitters) = self
                    .split_views
                    .get(&split_address)
                    .cloned()
                    .unwrap_or_else(|| (Grid::new(), Vec::new()));
                while splitters.len() < children.len().saturating_sub(1) {
                    let index = splitters.len();
                    let splitter = CustomSplitter::new_splitter();
                    self.wire_splitter(
                        &splitter,
                        grid.clone(),
                        split_address.clone(),
                        index,
                        (*orientation).into(),
                    );
                    splitters.push(splitter);
                }
                splitters.truncate(children.len().saturating_sub(1));
                planned_splits.insert(
                    split_address,
                    PlannedSplit {
                        grid: grid.clone(),
                        splitters,
                        orientation: *orientation,
                        weights,
                    },
                );
                Ok(RuntimeNode::Split { children, grid })
            }
        }
    }

    fn new_group_host(&self, group: &SnapshotGroupKey) -> GroupRuntimeHost {
        let container = Grid::new();
        container.set_rows(vec![GridLength::Auto, GridLength::Star(1.0)]);
        container.set_columns(vec![GridLength::Star(1.0)]);

        let title_bar = Grid::new();
        title_bar.set_rows(vec![GridLength::Star(1.0)]);
        title_bar.set_columns(vec![
            GridLength::Star(1.0),
            GridLength::Fixed(28.0),
            GridLength::Fixed(28.0),
        ]);
        title_bar.set_height(30.0);
        title_bar.set_background(themed_brush(BrushStyle::Secondary));

        let title = TextBlock::new();
        title.set_foreground(themed_brush(BrushStyle::Foreground));
        title.set_margin(8.0);
        title.set_attached("Grid", "row", 0i32);
        title.set_attached("Grid", "column", 0i32);
        title_bar.children().add(title.clone());

        let pin_button = Grid::new();
        pin_button.set_background(themed_brush(BrushStyle::Tertiary));
        pin_button.set_width(22.0);
        pin_button.set_height(22.0);
        pin_button.set_attached("Grid", "row", 0i32);
        pin_button.set_attached("Grid", "column", 1i32);
        let pin_glyph = TextBlock::new();
        pin_glyph.set_text("⌖");
        pin_glyph.set_foreground(themed_brush(BrushStyle::Foreground));
        pin_glyph.set_text_alignment(crate::core::ui::TextAlignment::Center);
        pin_button.children().add(pin_glyph);
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let pin_group = group.clone();
        pin_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |_, _| {
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_group_pin(pin_group.clone());
                }
            }),
        );
        title_bar.children().add(pin_button.clone());

        let close_button = Grid::new();
        close_button.set_background(themed_brush(BrushStyle::Tertiary));
        close_button.set_width(22.0);
        close_button.set_height(22.0);
        close_button.set_attached("Grid", "row", 0i32);
        close_button.set_attached("Grid", "column", 2i32);
        let close_glyph = TextBlock::new();
        close_glyph.set_text("×");
        close_glyph.set_foreground(themed_brush(BrushStyle::Foreground));
        close_glyph.set_text_alignment(crate::core::ui::TextAlignment::Center);
        close_button.children().add(close_glyph);
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let close_group = group.clone();
        close_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |_, _| {
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_group_title_close(close_group.clone());
                }
            }),
        );
        title_bar.children().add(close_button.clone());
        title_bar.set_visibility(Visibility::Collapsed);
        title_bar.set_attached("Grid", "row", 0i32);
        title_bar.set_attached("Grid", "column", 0i32);
        container.children().add(title_bar.clone());

        GroupRuntimeHost {
            container,
            title_bar,
            title,
            pin_button,
            close_button,
        }
    }

    fn apply_planned_group(&self, planned: &PlannedGroup) {
        replace_group_items(&planned.view, planned.tabs.clone());
        planned.view.set_tab_strip_position(planned.tab_position);
        if let Some(selected) = planned.selected.as_ref() {
            if let Some(index) = planned.items.iter().position(|item| item == selected) {
                planned.view.select_index(index);
            }
        }
        planned.host.container.children().clear();
        planned
            .host
            .container
            .children()
            .add(planned.host.title_bar.clone());
        planned.host.title.set_text(&planned.title);
        planned
            .host
            .title_bar
            .set_visibility(planned.title_visibility);
        planned
            .host
            .pin_button
            .set_visibility(planned.pin_visibility);
        planned
            .host
            .close_button
            .set_visibility(planned.close_visibility);
        planned.view.set_attached("Grid", "row", 1i32);
        planned.view.set_attached("Grid", "column", 0i32);
        planned.host.container.children().add(planned.view.clone());
    }

    fn apply_planned_node(
        &self,
        node: &RuntimeNode,
        root_kind: RootKind,
        path: &[usize],
        splits: &BTreeMap<SplitAddress, PlannedSplit>,
    ) -> Rc<dyn UIElementExt> {
        match node {
            RuntimeNode::Group { host } => host.clone(),
            RuntimeNode::Split { children, grid } => {
                let address = SplitAddress {
                    root: root_kind.clone(),
                    path: path.to_vec(),
                };
                let planned = splits
                    .get(&address)
                    .expect("planned split must exist during commit");
                grid.children().clear();
                match planned.orientation {
                    SnapshotOrientation::Horizontal => {
                        grid.set_rows(vec![GridLength::Star(1.0)]);
                        let mut columns = Vec::new();
                        for (index, child) in children.iter().enumerate() {
                            columns.push(GridLength::Star(snapshot_split_weight(
                                planned.weights[index],
                            )));
                            let mut child_path = path.to_vec();
                            child_path.push(index);
                            let element = self.apply_planned_node(
                                child,
                                root_kind.clone(),
                                &child_path,
                                splits,
                            );
                            element.as_ui_element().set_attached(
                                "Grid",
                                "column",
                                (index * 2) as i32,
                            );
                            grid.children().add(element);
                            if index + 1 < children.len() {
                                columns.push(GridLength::Fixed(6.0));
                                let splitter = planned.splitters[index].clone();
                                splitter.set_orientation(crate::Orientation::Horizontal);
                                splitter.set_attached("Grid", "column", (index * 2 + 1) as i32);
                                self.wire_splitter(
                                    &splitter,
                                    grid.clone(),
                                    address.clone(),
                                    index,
                                    crate::Orientation::Horizontal,
                                );
                                grid.children().add(splitter);
                            }
                        }
                        grid.set_columns(columns);
                    }
                    SnapshotOrientation::Vertical => {
                        grid.set_columns(vec![GridLength::Star(1.0)]);
                        let mut rows = Vec::new();
                        for (index, child) in children.iter().enumerate() {
                            rows.push(GridLength::Star(snapshot_split_weight(
                                planned.weights[index],
                            )));
                            let mut child_path = path.to_vec();
                            child_path.push(index);
                            let element = self.apply_planned_node(
                                child,
                                root_kind.clone(),
                                &child_path,
                                splits,
                            );
                            element
                                .as_ui_element()
                                .set_attached("Grid", "row", (index * 2) as i32);
                            grid.children().add(element);
                            if index + 1 < children.len() {
                                rows.push(GridLength::Fixed(6.0));
                                let splitter = planned.splitters[index].clone();
                                splitter.set_orientation(crate::Orientation::Vertical);
                                splitter.set_attached("Grid", "row", (index * 2 + 1) as i32);
                                self.wire_splitter(
                                    &splitter,
                                    grid.clone(),
                                    address.clone(),
                                    index,
                                    crate::Orientation::Vertical,
                                );
                                grid.children().add(splitter);
                            }
                        }
                        grid.set_rows(rows);
                    }
                }
                grid.clone()
            }
        }
    }

    fn wire_group_callbacks(&self, view: &Rc<CustomTabView>, group: SnapshotGroupKey) {
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let selected_group = group.clone();
        view.set_on_selected_index_change(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_group_selected(selected_group.clone(), index);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let close_group = group.clone();
        view.set_on_close_request(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_group_close(close_group.clone(), index);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let start_group = group.clone();
        view.set_on_tab_drag_started(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_tab_drag_started(start_group.clone(), args);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let moved_group = group.clone();
        view.set_on_tab_drag_moved(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_tab_drag_moved(moved_group.clone(), args);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        view.set_on_tab_drag_completed(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_tab_drag_completed(group.clone(), args);
            }
        }));
    }

    fn wire_splitter(
        &self,
        splitter: &Rc<CustomSplitter>,
        grid: Rc<Grid>,
        address: SplitAddress,
        boundary: usize,
        orientation: crate::Orientation,
    ) {
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let start_grid = grid.clone();
        let start_address = address.clone();
        splitter.set_on_drag_started(Box::new(move |args: SplitterDragStartedEventArgs| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_splitter_started(
                    start_address.clone(),
                    boundary,
                    start_grid.clone(),
                    orientation,
                    args,
                );
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        splitter.set_on_drag_delta(Box::new(move |args: SplitterDragDeltaEventArgs| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_splitter_delta(args);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        splitter.set_on_drag_completed(Box::new(move |args: SplitterDragCompletedEventArgs| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_splitter_completed(args);
            }
        }));
    }

    pub(crate) fn group_item(&self, group: &SnapshotGroupKey, index: usize) -> Option<DockItemId> {
        self.group_items
            .get(group)
            .and_then(|items| items.get(index))
            .cloned()
    }

    pub(crate) fn all_closeable(&self, items: &[DockItemId]) -> bool {
        items.iter().all(|item| {
            self.registry
                .items
                .get(item)
                .is_some_and(|item| item.can_close_value())
        })
    }

    pub(crate) fn nearest_pin_side(&self, item: &DockItemId) -> Option<crate::DockSide> {
        let key = self
            .group_items
            .iter()
            .find(|(_, items)| items.iter().any(|candidate| candidate == item))
            .map(|(key, _)| key)?;
        let group_view = self.groups.get(key)?;
        let group_node: Rc<dyn UIElementExt> = group_view.clone();
        let root = self.group_roots.get(key)?;
        let surface = self.surfaces.surface_for_root(root)?;
        let bounds = SurfaceRegistry::surface_bounds(&surface)?;
        let group_bounds = SurfaceRegistry::bounds_in_surface_local(&group_node, &surface)?;
        let distances = [
            (group_bounds.x, crate::DockSide::Left),
            (group_bounds.y, crate::DockSide::Top),
            (
                bounds.width - group_bounds.x - group_bounds.width,
                crate::DockSide::Right,
            ),
            (
                bounds.height - group_bounds.y - group_bounds.height,
                crate::DockSide::Bottom,
            ),
        ];
        let mut nearest = None;
        for (distance, side) in distances {
            if !distance.is_finite() {
                continue;
            }
            if nearest.is_none_or(|(best, _): (f32, crate::DockSide)| distance < best) {
                nearest = Some((distance, side));
            }
        }
        nearest.map(|(_, side)| side)
    }

    fn item_root(&self, item: &DockItemId) -> Option<RootKind> {
        match self.owners.get(item)? {
            RuntimePresentationOwner::Group(group) => self.group_roots.get(group).cloned(),
            RuntimePresentationOwner::FloatingGroup { root, .. } => Some(RootKind::Floating(*root)),
            RuntimePresentationOwner::AutoHide { root } => Some(root.clone()),
            RuntimePresentationOwner::None => None,
        }
    }

    fn surface_runtime(&self, root: &RootKind) -> Option<&SurfaceRuntime> {
        match root {
            RootKind::Main => Some(&self.main_surface),
            RootKind::Floating(index) => self.floating.get(*index).map(|runtime| &runtime.surface),
        }
    }

    fn surface_runtime_mut(&mut self, root: &RootKind) -> Option<&mut SurfaceRuntime> {
        match root {
            RootKind::Main => Some(&mut self.main_surface),
            RootKind::Floating(index) => self
                .floating
                .get_mut(*index)
                .map(|runtime| &mut runtime.surface),
        }
    }

    fn clear_previews(&mut self) {
        self.main_surface.preview.clear();
        for runtime in &mut self.floating {
            runtime.surface.preview.clear();
        }
    }

    fn clear_auto_hide_presentations(&mut self) {
        self.main_surface.auto_hide.close();
        for runtime in &mut self.floating {
            runtime.surface.auto_hide.close();
        }
    }
}

impl RuntimeRealization {
    fn detach_existing_tree(&mut self) {
        if let Some(old_main) = self.main_surface_child.take() {
            self.surface_root.children().remove(&old_main);
        }
        if let Some(root) = self.root.as_ref() {
            detach_runtime_node(root);
        }
        self.main_surface.auto_hide.close();
        self.main_surface.preview.clear();
        for runtime in &mut self.floating {
            runtime.surface.auto_hide.close();
            runtime.surface.preview.clear();
            runtime.surface.surface.content_root().children().clear();
            detach_runtime_node(&runtime.node);
        }
    }

    fn detach_before_attach(&mut self, desired: &BTreeMap<DockItemId, RuntimePresentationOwner>) {
        // Clear every old tab parent before any desired parent is built. This makes the ownership
        // transition explicit and prevents CustomTabView's duplicate-parent guard from seeing a
        // wrapper in two visual collections during reconciliation.
        for (item, current) in &self.owners {
            if desired.get(item) == Some(current) {
                continue;
            }
            match current {
                RuntimePresentationOwner::Group(group)
                | RuntimePresentationOwner::FloatingGroup { group, .. } => {
                    if let Some(view) = self.groups.get(group) {
                        view.replace_children(Vec::new());
                    }
                }
                RuntimePresentationOwner::AutoHide { .. } | RuntimePresentationOwner::None => {}
            }
        }
    }
}

fn detach_runtime_node(node: &RuntimeNode) {
    match node {
        RuntimeNode::Group { host } => {
            host.children().clear();
        }
        RuntimeNode::Split { children, grid, .. } => {
            for child in children {
                detach_runtime_node(child);
            }
            grid.children().clear();
        }
    }
}

fn snapshot_split_weight(weight: f32) -> f32 {
    if weight.is_finite() && weight > 0.0 {
        weight
    } else {
        1.0
    }
}

fn contains(bounds: Rect, point: Point) -> bool {
    point.x >= bounds.x
        && point.y >= bounds.y
        && point.x <= bounds.x + bounds.width
        && point.y <= bounds.y + bounds.height
}

/// Resolves a pointer already expressed in one surface's local coordinate space. Keeping this
/// separate from the screen/root conversion makes the target facts and preview geometry one
/// operation: the exact object returned here is also the object used by the drag commit path.
fn resolve_local_target(
    root: RootKind,
    surface_bounds: Rect,
    surface_local_point: Point,
    groups: impl IntoIterator<Item = (SnapshotGroupKey, Rect)>,
) -> Option<ResolvedDockTarget> {
    if !valid_preview_rect(surface_bounds)
        || !surface_local_point.x.is_finite()
        || !surface_local_point.y.is_finite()
        || !contains(surface_bounds, surface_local_point)
    {
        return None;
    }
    let outer_band = (surface_bounds.width.min(surface_bounds.height) * 0.10).clamp(24.0, 64.0);
    let outer = [
        (
            surface_local_point.x <= outer_band,
            crate::DockTarget::DockLeft,
        ),
        (
            surface_local_point.y <= outer_band,
            crate::DockTarget::DockTop,
        ),
        (
            surface_local_point.x >= surface_bounds.width - outer_band,
            crate::DockTarget::DockRight,
        ),
        (
            surface_local_point.y >= surface_bounds.height - outer_band,
            crate::DockTarget::DockBottom,
        ),
    ];
    if let Some((_, target)) = outer.into_iter().find(|(inside, _)| *inside) {
        return Some(ResolvedDockTarget {
            root,
            target,
            group: None,
            preview_rect: outer_preview(surface_bounds, target)?,
        });
    }

    let mut deepest = None;
    let mut smallest_area = f32::INFINITY;
    for (key, bounds) in groups {
        if !valid_preview_rect(bounds) || !contains(bounds, surface_local_point) {
            continue;
        }
        let area = bounds.width * bounds.height;
        if area < smallest_area {
            smallest_area = area;
            deepest = Some((key, bounds));
        }
    }
    let (group, bounds) = deepest?;
    let group_band = (bounds.width.min(bounds.height) * 0.25).clamp(24.0, 64.0);
    let local = Point {
        x: surface_local_point.x - bounds.x,
        y: surface_local_point.y - bounds.y,
    };
    let target = [
        (local.x <= group_band, crate::DockTarget::SplitLeft),
        (local.y <= group_band, crate::DockTarget::SplitTop),
        (
            local.x >= bounds.width - group_band,
            crate::DockTarget::SplitRight,
        ),
        (
            local.y >= bounds.height - group_band,
            crate::DockTarget::SplitBottom,
        ),
    ]
    .into_iter()
    .find(|(inside, _)| *inside)
    .map(|(_, target)| target)
    .unwrap_or(crate::DockTarget::Center);
    Some(ResolvedDockTarget {
        root,
        target,
        group: Some(group),
        preview_rect: group_preview(bounds, target)?,
    })
}

#[cfg(test)]
pub(crate) fn resolve_local_target_for_test(
    root: RootKind,
    surface_bounds: Rect,
    surface_local_point: Point,
    groups: Vec<(SnapshotGroupKey, Rect)>,
) -> Option<ResolvedDockTarget> {
    resolve_local_target(root, surface_bounds, surface_local_point, groups)
}

fn auto_hide_root(entry: &SnapshotAutoHideEntry, floating_count: usize) -> RootKind {
    entry
        .return_state
        .floating_root
        .filter(|index| *index < floating_count)
        .map(RootKind::Floating)
        .unwrap_or(RootKind::Main)
}

fn group_identity(node: &SnapshotNode) -> Vec<SnapshotGroupKey> {
    fn visit(node: &SnapshotNode, groups: &mut BTreeSet<SnapshotGroupKey>) {
        match node {
            SnapshotNode::Group { group, .. } => {
                groups.insert(group.clone());
            }
            SnapshotNode::Split { children, .. } => {
                for child in children {
                    visit(&child.node, groups);
                }
            }
        }
    }
    let mut groups = BTreeSet::new();
    visit(node, &mut groups);
    groups.into_iter().collect()
}

fn outer_preview(surface: Rect, target: crate::DockTarget) -> Option<Rect> {
    let preview = match target {
        crate::DockTarget::DockLeft => Rect {
            x: surface.x,
            y: surface.y,
            width: surface.width * 0.25,
            height: surface.height,
        },
        crate::DockTarget::DockRight => Rect {
            x: surface.x + surface.width * 0.75,
            y: surface.y,
            width: surface.width * 0.25,
            height: surface.height,
        },
        crate::DockTarget::DockTop => Rect {
            x: surface.x,
            y: surface.y,
            width: surface.width,
            height: surface.height * 0.25,
        },
        crate::DockTarget::DockBottom => Rect {
            x: surface.x,
            y: surface.y + surface.height * 0.75,
            width: surface.width,
            height: surface.height * 0.25,
        },
        _ => return None,
    };
    valid_preview_rect(preview).then_some(preview)
}

fn group_preview(group: Rect, target: crate::DockTarget) -> Option<Rect> {
    let preview = match target {
        crate::DockTarget::Center => group,
        crate::DockTarget::SplitLeft => Rect {
            x: group.x,
            y: group.y,
            width: group.width * 0.5,
            height: group.height,
        },
        crate::DockTarget::SplitRight => Rect {
            x: group.x + group.width * 0.5,
            y: group.y,
            width: group.width * 0.5,
            height: group.height,
        },
        crate::DockTarget::SplitTop => Rect {
            x: group.x,
            y: group.y,
            width: group.width,
            height: group.height * 0.5,
        },
        crate::DockTarget::SplitBottom => Rect {
            x: group.x,
            y: group.y + group.height * 0.5,
            width: group.width,
            height: group.height * 0.5,
        },
        _ => return None,
    };
    valid_preview_rect(preview).then_some(preview)
}

fn valid_preview_rect(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}

fn desired_owners(
    snapshot: &crate::snapshot::DockLayoutSnapshot,
) -> BTreeMap<DockItemId, RuntimePresentationOwner> {
    fn visit(
        node: &SnapshotNode,
        floating_root: Option<usize>,
        out: &mut BTreeMap<DockItemId, RuntimePresentationOwner>,
    ) {
        match node {
            SnapshotNode::Group { group, items, .. } => {
                let owner = floating_root
                    .map(|root| RuntimePresentationOwner::FloatingGroup {
                        root,
                        group: group.clone(),
                    })
                    .unwrap_or_else(|| RuntimePresentationOwner::Group(group.clone()));
                for item in items {
                    out.insert(item.clone(), owner.clone());
                }
            }
            SnapshotNode::Split { children, .. } => {
                for child in children {
                    visit(&child.node, floating_root, out);
                }
            }
        }
    }
    let mut owners = BTreeMap::new();
    if let Some(root) = &snapshot.main_root {
        visit(root, None, &mut owners);
    }
    for (index, floating) in snapshot.floating_roots.iter().enumerate() {
        visit(&floating.root, Some(index), &mut owners);
    }
    for entries in &snapshot.auto_hide {
        for entry in entries {
            owners.insert(
                entry.item.clone(),
                if entry.open {
                    RuntimePresentationOwner::AutoHide {
                        root: auto_hide_root(entry, snapshot.floating_roots.len()),
                    }
                } else {
                    RuntimePresentationOwner::None
                },
            );
        }
    }
    for entry in &snapshot.closed {
        owners.insert(entry.item.clone(), RuntimePresentationOwner::None);
    }
    owners
}

fn same_selection_structure(
    left: &crate::snapshot::DockLayoutSnapshot,
    right: &crate::snapshot::DockLayoutSnapshot,
) -> bool {
    fn same_node(left: &SnapshotNode, right: &SnapshotNode) -> bool {
        match (left, right) {
            (
                SnapshotNode::Split {
                    orientation: left_orientation,
                    children: left_children,
                },
                SnapshotNode::Split {
                    orientation: right_orientation,
                    children: right_children,
                },
            ) => {
                left_orientation == right_orientation
                    && left_children.len() == right_children.len()
                    && left_children
                        .iter()
                        .zip(right_children)
                        .all(|(left, right)| {
                            left.weight == right.weight && same_node(&left.node, &right.node)
                        })
            }
            (
                SnapshotNode::Group {
                    group: left_group,
                    items: left_items,
                    ..
                },
                SnapshotNode::Group {
                    group: right_group,
                    items: right_items,
                    ..
                },
            ) => left_group == right_group && left_items == right_items,
            _ => false,
        }
    }

    let roots_match = match (&left.main_root, &right.main_root) {
        (Some(left), Some(right)) => same_node(left, right),
        (None, None) => true,
        _ => false,
    } && left.floating_roots.len() == right.floating_roots.len()
        && left
            .floating_roots
            .iter()
            .zip(&right.floating_roots)
            .all(|(left, right)| left.bounds == right.bounds && same_node(&left.root, &right.root));
    if !roots_match || left.closed != right.closed {
        return false;
    }
    left.auto_hide
        .iter()
        .zip(&right.auto_hide)
        .all(|(left, right)| {
            left.len() == right.len()
                && left.iter().zip(right).all(|(left, right)| {
                    left.item == right.item && left.return_state == right.return_state
                })
        })
}

struct ReconcilingGuard(Rc<Cell<bool>>);

impl Drop for ReconcilingGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
