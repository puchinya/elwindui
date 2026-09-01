//! Reconciliation boundary between the value model and stable runtime item wrappers.

use crate::core::base::{Point, Rect};
use crate::core::input::PointerEventArgs;
use crate::core::layout::{GridLength, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    Grid, GridExt, LayoutExt, TextBlock, TextBlockExt, TextStyleOwner, UIElementExt,
};
use crate::model::{RootKind, SplitAddress};
use crate::snapshot::{SnapshotAutoHideEntry, SnapshotGroupKey, SnapshotNode};
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
use super::floating_window::{FloatingHostId, FloatingHostRegistry};
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
    bounds: crate::Rect,
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

struct GroupRuntimeHost {
    container: Rc<Grid>,
    title_bar: Rc<Grid>,
    title: Rc<TextBlock>,
    pin_button: Rc<Grid>,
    close_button: Rc<Grid>,
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
    fail_next_reconcile: bool,
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
            fail_next_reconcile: false,
        })
    }

    pub(crate) fn refresh_authored(
        &mut self,
        root: &dyn UIElementExt,
    ) -> Result<(), DockLayoutError> {
        self.detach_existing_tree();
        self.registry.refresh_authored(root)?;
        // An authored registration change can invalidate the item or splitter captured by a
        // native gesture. Cancel both transient sessions before the next model reconciliation;
        // this is safer than allowing a stale wrapper to commit into the new registry.
        self.drag = None;
        self.splitter = None;
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

    /// Applies a model transaction to the realization. All tab list changes happen after the
    /// complete model has been accepted, so a failed snapshot cannot partially mutate the UI.
    pub(crate) fn reconcile(&mut self, model: &DockLayoutModel) -> Result<(), DockLayoutError> {
        self.reconcile_internal(model, None)
    }

    /// Applies a candidate that already has a prepared native host. The prepared surface is used
    /// for the newly appended floating root, while the host registry remains untouched until the
    /// caller commits the transaction.
    pub(crate) fn reconcile_with_prepared(
        &mut self,
        model: &DockLayoutModel,
        prepared_surface: Rc<DockSurfaceView>,
    ) -> Result<(), DockLayoutError> {
        self.reconcile_internal(model, Some(prepared_surface))
    }

    fn reconcile_internal(
        &mut self,
        model: &DockLayoutModel,
        prepared_surface: Option<Rc<DockSurfaceView>>,
    ) -> Result<(), DockLayoutError> {
        #[cfg(test)]
        if self.fail_next_reconcile {
            self.fail_next_reconcile = false;
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "injected reconciliation failure".to_owned(),
            });
        }
        let snapshot = model.snapshot();
        self.detach_existing_tree();
        let mut previous_floating = std::mem::take(&mut self.floating);
        self.surfaces = SurfaceRegistry::default();
        let desired_owners = desired_owners(&snapshot);
        self.detach_before_attach(&desired_owners);
        self.owners = desired_owners;
        self.reconciling.set(true);
        let _reconciling_guard = ReconcilingGuard(self.reconciling.clone());
        self.group_items.clear();
        self.group_roots.clear();
        self.group_selected.clear();
        self.auto_hide_roots.clear();
        let floating_count = snapshot.floating_roots.len();
        for entry in snapshot.auto_hide.iter().flatten() {
            self.auto_hide_roots
                .insert(entry.item.clone(), auto_hide_root(entry, floating_count));
        }
        let mut used_groups = BTreeMap::new();
        let mut used_splits = BTreeMap::new();
        let mut groups = self.groups.clone();
        let open_auto_hide = snapshot
            .auto_hide
            .iter()
            .flatten()
            .filter(|entry| entry.open)
            .map(|entry| entry.item.clone())
            .collect::<Vec<_>>();
        let root = snapshot
            .main_root
            .as_ref()
            .map(|node| {
                self.build_node(
                    node,
                    &mut groups,
                    &mut used_groups,
                    &mut used_splits,
                    RootKind::Main,
                    &[],
                )
            })
            .transpose()?;
        let mut floating_roots = Vec::new();
        for (floating_index, floating) in snapshot.floating_roots.iter().enumerate() {
            let bounds = floating.bounds.into();
            let node = self.build_node(
                &floating.root,
                &mut groups,
                &mut used_groups,
                &mut used_splits,
                RootKind::Floating(floating_index),
                &[],
            )?;
            let identity = group_identity(&floating.root);
            let mut surface_runtime = if floating_index + 1 == snapshot.floating_roots.len() {
                prepared_surface
                    .as_ref()
                    .map(|surface| {
                        SurfaceRuntime::new(
                            RootKind::Floating(floating_index),
                            surface.clone(),
                            &self.owner,
                        )
                    })
                    .or_else(|| {
                        previous_floating
                            .iter()
                            .position(|runtime| runtime.identity == identity)
                            .map(|index| previous_floating.swap_remove(index).surface)
                    })
            } else {
                previous_floating
                    .iter()
                    .position(|runtime| runtime.identity == identity)
                    .map(|index| previous_floating.swap_remove(index).surface)
            }
            .unwrap_or_else(|| {
                SurfaceRuntime::new(
                    RootKind::Floating(floating_index),
                    DockSurfaceView::empty_surface(),
                    &self.owner,
                )
            });
            surface_runtime.set_root(RootKind::Floating(floating_index));
            surface_runtime.auto_hide.close();
            surface_runtime.preview.clear();
            surface_runtime.reset_visual_children();
            surface_runtime.add_main_child(node.element());
            floating_roots.push(FloatingRuntime {
                bounds,
                node,
                identity,
                surface: surface_runtime,
            });
        }
        groups.retain(|key, _| used_groups.contains_key(key));
        self.group_items
            .retain(|key, _| used_groups.contains_key(key));
        self.group_roots
            .retain(|key, _| used_groups.contains_key(key));
        self.split_views
            .retain(|key, _| used_splits.contains_key(key));
        self.main_surface.auto_hide.close();
        self.main_surface.preview.clear();
        self.main_surface.reset_visual_children();
        if let Some(root) = root.as_ref() {
            let element = root.element();
            self.main_surface.add_main_child(element.clone());
            self.main_surface_child = Some(element);
        }
        self.groups = groups;
        self.group_hosts
            .retain(|key, _| used_groups.contains_key(key));
        self.root = root;
        self.floating = floating_roots;
        for (index, runtime) in self.floating.iter().enumerate() {
            let surface_node: Rc<dyn UIElementExt> = runtime.surface.surface.clone();
            self.surfaces
                .register(RootKind::Floating(index), &surface_node);
        }
        let main_surface_node: Rc<dyn UIElementExt> = self.main_surface.surface.clone();
        self.surfaces.register(RootKind::Main, &main_surface_node);
        let registry = &self.registry;
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
        for (index, runtime) in self.floating.iter().enumerate() {
            let root = RootKind::Floating(index);
            runtime
                .surface
                .render_strips(strip_titles(&root).into_iter(), &self.owner);
        }
        for item in open_auto_hide {
            let root = snapshot
                .auto_hide
                .iter()
                .flatten()
                .find(|entry| entry.item == item)
                .map(|entry| auto_hide_root(entry, floating_count))
                .unwrap_or(RootKind::Main);
            let wrapper = self.registry.wrapper(&item);
            let surface = self.surface_runtime_mut(&root);
            if let Some(surface) = surface {
                surface.auto_hide.open(item.clone());
                surface.auto_hide.present_open_item(wrapper);
            }
        }
        let floating_specs = self
            .floating
            .iter()
            .map(|runtime| (runtime.bounds, runtime.surface.surface.clone()))
            .collect::<Vec<_>>();
        if prepared_surface.is_none() {
            self.floating_hosts.sync(&floating_specs, &self.owner)?;
        }
        Ok(())
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

    pub(crate) fn prepare_floating_host(
        &mut self,
        bounds: crate::Rect,
    ) -> Result<super::floating_window::PreparedFloatingHost, DockLayoutError> {
        self.floating_hosts
            .prepare_new(DockSurfaceView::empty_surface(), bounds, &self.owner)
    }

    pub(crate) fn commit_prepared_host(
        &mut self,
        prepared: super::floating_window::PreparedFloatingHost,
        root_index: usize,
    ) -> super::floating_window::FloatingHostId {
        self.floating_hosts.commit_prepared(prepared, root_index)
    }

    pub(crate) fn show_floating_host(&self, id: super::floating_window::FloatingHostId) {
        self.floating_hosts.show(id);
    }

    pub(crate) fn floating_root_index(&self, id: FloatingHostId) -> Option<usize> {
        self.floating_hosts.root_index_for_host(id)
    }

    #[cfg(test)]
    pub(crate) fn set_floating_host_factory_for_test(&mut self, factory: FloatingHostFactory) {
        self.floating_hosts = FloatingHostRegistry::with_factory(factory);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_reconcile_for_test(&mut self) {
        self.fail_next_reconcile = true;
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
                .filter_map(|(root, surface)| {
                    let host_root_point = surface.screen_to_root(screen)?;
                    let surface_local_point =
                        SurfaceRegistry::host_root_to_surface_local(&surface, host_root_point)?;
                    let bounds = SurfaceRegistry::surface_bounds(&surface)?;
                    contains(bounds, surface_local_point).then_some((
                        root,
                        surface,
                        surface_local_point,
                    ))
                })
                .next()?
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
                Some(splitter.cancel())
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

    fn build_node(
        &mut self,
        node: &SnapshotNode,
        groups: &mut BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
        used_groups: &mut BTreeMap<SnapshotGroupKey, ()>,
        used_splits: &mut BTreeMap<SplitAddress, ()>,
        root_kind: RootKind,
        path: &[usize],
    ) -> Result<RuntimeNode, DockLayoutError> {
        match node {
            SnapshotNode::Group {
                group,
                items,
                selected,
            } => {
                used_groups.insert(group.clone(), ());
                self.group_roots.insert(group.clone(), root_kind.clone());
                let view = if let Some(view) = groups.get(group) {
                    view.clone()
                } else {
                    let view = CustomTabView::new_view();
                    self.wire_group_callbacks(&view, group.clone());
                    groups.insert(group.clone(), view.clone());
                    view
                };
                self.group_items.insert(group.clone(), items.clone());
                self.group_selected.insert(group.clone(), selected.clone());
                let tabs = items
                    .iter()
                    .map(|id| {
                        self.registry
                            .wrapper(id)
                            .ok_or_else(|| DockLayoutError::UnknownItem(id.clone()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                replace_group_items(&view, tabs);
                if let SnapshotGroupKey::Authored(id) = group {
                    view.set_tab_strip_position(self.registry.group_position(id));
                }
                if let Some(selected) = selected {
                    if let Some(index) = items.iter().position(|id| id == selected) {
                        view.select_index(index);
                    }
                }
                let tab_position = match &group {
                    SnapshotGroupKey::Authored(id) => self.registry.group_position(id),
                    SnapshotGroupKey::Generated(_) => TabStripPosition::Top,
                };
                let host = self.group_hosts.entry(group.clone()).or_insert_with(|| {
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
                    let weak_owner = self.owner.clone();
                    let pin_group = group.clone();
                    pin_button.register_routed_handler::<PointerEventArgs>(
                        "on_pointer_released",
                        Box::new(move |_, _| {
                            if let Some(owner) = weak_owner.upgrade() {
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
                    let weak_owner = self.owner.clone();
                    let close_group = group.clone();
                    close_button.register_routed_handler::<PointerEventArgs>(
                        "on_pointer_released",
                        Box::new(move |_, _| {
                            if let Some(owner) = weak_owner.upgrade() {
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
                });
                host.container.children().clear();
                host.container.children().add(host.title_bar.clone());
                let selected_item = selected
                    .as_ref()
                    .and_then(|selected| self.registry.items.get(selected));
                host.title.set_text(
                    &selected_item
                        .map(|item| item.title_value())
                        .unwrap_or_default(),
                );
                let has_pin = selected_item.is_some_and(|item| item.can_pin_value());
                let show_title_bar = tab_position == TabStripPosition::Bottom || has_pin;
                host.title_bar.set_visibility(if show_title_bar {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                });
                host.pin_button.set_visibility(if has_pin {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                });
                host.close_button.set_visibility(
                    if selected_item.is_some_and(|item| item.can_close_value()) {
                        Visibility::Visible
                    } else {
                        Visibility::Collapsed
                    },
                );
                view.set_attached("Grid", "row", 1i32);
                view.set_attached("Grid", "column", 0i32);
                host.container.children().add(view.clone());
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
                used_splits.insert(split_address.clone(), ());
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
                        self.build_node(
                            &child.node,
                            groups,
                            used_groups,
                            used_splits,
                            root_kind.clone(),
                            &child_path,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let (grid, mut splitters) = self
                    .split_views
                    .remove(&split_address)
                    .unwrap_or_else(|| (Grid::new(), Vec::new()));
                grid.children().clear();
                match orientation {
                    crate::snapshot::SnapshotOrientation::Horizontal => {
                        grid.set_rows(vec![GridLength::Star(1.0)]);
                        let mut columns = Vec::new();
                        for (index, child) in children.iter().enumerate() {
                            columns.push(GridLength::Star(snapshot_split_weight(weights[index])));
                            let element = child.element();
                            element.as_ui_element().set_attached(
                                "Grid",
                                "column",
                                (index * 2) as i32,
                            );
                            grid.children().add(element);
                            if index + 1 < children.len() {
                                columns.push(GridLength::Fixed(6.0));
                                let splitter = splitters
                                    .get(index)
                                    .cloned()
                                    .unwrap_or_else(CustomSplitter::new_splitter);
                                splitter.set_orientation((*orientation).into());
                                splitter.set_attached("Grid", "column", (index * 2 + 1) as i32);
                                self.wire_splitter(
                                    &splitter,
                                    grid.clone(),
                                    SplitAddress {
                                        root: root_kind.clone(),
                                        path: path.to_vec(),
                                    },
                                    index,
                                    (*orientation).into(),
                                );
                                grid.children().add(splitter.clone());
                                if index >= splitters.len() {
                                    splitters.push(splitter);
                                }
                            }
                        }
                        grid.set_columns(columns);
                    }
                    crate::snapshot::SnapshotOrientation::Vertical => {
                        grid.set_columns(vec![GridLength::Star(1.0)]);
                        let mut rows = Vec::new();
                        for (index, child) in children.iter().enumerate() {
                            rows.push(GridLength::Star(snapshot_split_weight(weights[index])));
                            let element = child.element();
                            element
                                .as_ui_element()
                                .set_attached("Grid", "row", (index * 2) as i32);
                            grid.children().add(element);
                            if index + 1 < children.len() {
                                rows.push(GridLength::Fixed(6.0));
                                let splitter = splitters
                                    .get(index)
                                    .cloned()
                                    .unwrap_or_else(CustomSplitter::new_splitter);
                                splitter.set_orientation((*orientation).into());
                                splitter.set_attached("Grid", "row", (index * 2 + 1) as i32);
                                self.wire_splitter(
                                    &splitter,
                                    grid.clone(),
                                    SplitAddress {
                                        root: root_kind.clone(),
                                        path: path.to_vec(),
                                    },
                                    index,
                                    (*orientation).into(),
                                );
                                grid.children().add(splitter.clone());
                                if index >= splitters.len() {
                                    splitters.push(splitter);
                                }
                            }
                        }
                        grid.set_rows(rows);
                    }
                }
                splitters.truncate(children.len().saturating_sub(1));
                self.split_views
                    .insert(split_address, (grid.clone(), splitters.clone()));
                Ok(RuntimeNode::Split { children, grid })
            }
        }
    }

    fn wire_group_callbacks(&self, view: &Rc<CustomTabView>, group: SnapshotGroupKey) {
        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let selected_group = group.clone();
        view.set_on_selected_index_change(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
                owner.handle_group_selected(selected_group.clone(), index);
            }
        }));

        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let close_group = group.clone();
        view.set_on_close_request(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
                owner.handle_group_close(close_group.clone(), index);
            }
        }));

        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let start_group = group.clone();
        view.set_on_tab_drag_started(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
                owner.handle_tab_drag_started(start_group.clone(), args);
            }
        }));

        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let moved_group = group.clone();
        view.set_on_tab_drag_moved(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
                owner.handle_tab_drag_moved(moved_group.clone(), args);
            }
        }));

        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        view.set_on_tab_drag_completed(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
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
        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let start_grid = grid.clone();
        let start_address = address.clone();
        splitter.set_on_drag_started(Box::new(move |args: SplitterDragStartedEventArgs| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
                owner.handle_splitter_started(
                    start_address.clone(),
                    boundary,
                    start_grid.clone(),
                    orientation,
                    args,
                );
            }
        }));

        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        splitter.set_on_drag_delta(Box::new(move |args: SplitterDragDeltaEventArgs| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
                owner.handle_splitter_delta(args);
            }
        }));

        let weak_owner = self.owner.clone();
        let reconciling = self.reconciling.clone();
        splitter.set_on_drag_completed(Box::new(move |args: SplitterDragCompletedEventArgs| {
            if reconciling.get() {
                return;
            }
            if let Some(owner) = weak_owner.upgrade() {
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
    if let RuntimeNode::Split { children, grid, .. } = node {
        for child in children {
            detach_runtime_node(child);
        }
        grid.children().clear();
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

struct ReconcilingGuard(Rc<Cell<bool>>);

impl Drop for ReconcilingGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
