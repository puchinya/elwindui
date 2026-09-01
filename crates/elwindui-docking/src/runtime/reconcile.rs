//! Reconciliation boundary between the value model and stable runtime item wrappers.

use crate::core::base::{Point, Rect};
use crate::core::input::PointerEventArgs;
use crate::core::layout::GridLength;
use crate::core::ui::{Grid, GridExt, LayoutExt, Rectangle, ShapeExt, UIElementExt};
use crate::model::{RootKind, SplitAddress};
use crate::snapshot::{SnapshotGroupKey, SnapshotNode};
use crate::{
    DockGroup, DockGroupId, DockItem, DockItemId, DockLayoutError, DockLayoutModel, DockSplitPanel,
};
use elwindui_custom_controls::{
    CustomSplitter, CustomSplitterExt, CustomTabView, CustomTabViewExt, CustomTabViewItem,
    CustomTabViewItemExt, SplitterDragCompletedEventArgs, SplitterDragDeltaEventArgs,
    SplitterDragStartedEventArgs, TabStripPosition,
};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::rc::Weak;

use super::auto_hide::AutoHideOverlay;
use super::drag::DragSession;
use super::floating_window::{FloatingHostRegistry, floating_host_available};
use super::group_view::replace_group_items;
use super::overlay::DropPreview;
use super::split_view::SplitterSession;
use super::surface_registry::SurfaceRegistry;
use super::surface_view::DockSurfaceView;

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
    AutoHide,
    FloatingGroup {
        root: usize,
        group: SnapshotGroupKey,
    },
    None,
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
    pin_button: Rc<Rectangle>,
}

pub struct RuntimeRealization {
    registry: StableItemRegistry,
    groups: BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
    group_hosts: BTreeMap<SnapshotGroupKey, GroupRuntimeHost>,
    group_items: BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
    group_selected: BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
    owners: BTreeMap<DockItemId, RuntimePresentationOwner>,
    root: Option<RuntimeNode>,
    floating: Vec<(crate::Rect, RuntimeNode, Rc<DockSurfaceView>)>,
    split_views: BTreeMap<SplitAddress, (Rc<Grid>, Vec<Rc<CustomSplitter>>)>,
    drag: Option<DragSession>,
    splitter: Option<SplitterSession>,
    auto_hide: AutoHideOverlay,
    preview: DropPreview,
    floating_hosts: FloatingHostRegistry,
    surfaces: SurfaceRegistry,
    surface: Rc<DockSurfaceView>,
    surface_root: Rc<Grid>,
    main_surface_child: Option<Rc<dyn UIElementExt>>,
    owner: Weak<crate::DockingControl>,
    reconciling: Rc<Cell<bool>>,
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
        let auto_hide = AutoHideOverlay::new();
        let preview = DropPreview::new();
        auto_hide.bind_pin_handler(&owner);
        let surface_root = surface.content_root();
        surface_root.children().add(auto_hide.visual());
        surface_root.children().add(preview.visual());
        let mut surfaces = SurfaceRegistry::default();
        let surface_node: Rc<dyn UIElementExt> = surface.clone();
        surfaces.register(&surface_node);
        Ok(Self {
            registry: StableItemRegistry::from_authored(root)?,
            groups: BTreeMap::new(),
            group_hosts: BTreeMap::new(),
            group_items: BTreeMap::new(),
            group_selected: BTreeMap::new(),
            owners: BTreeMap::new(),
            root: None,
            floating: Vec::new(),
            split_views: BTreeMap::new(),
            drag: None,
            splitter: None,
            auto_hide,
            preview,
            floating_hosts: FloatingHostRegistry::default(),
            surfaces,
            surface,
            surface_root,
            main_surface_child: None,
            owner,
            reconciling: Rc::new(Cell::new(false)),
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
        self.preview.clear();
        Ok(())
    }

    pub(crate) fn cancel_transient(&mut self) {
        self.drag = None;
        if let Some(mut splitter) = self.splitter.take() {
            splitter.cancel();
        }
        self.preview.clear();
    }

    /// Applies a model transaction to the realization. All tab list changes happen after the
    /// complete model has been accepted, so a failed snapshot cannot partially mutate the UI.
    pub(crate) fn reconcile(&mut self, model: &DockLayoutModel) -> Result<(), DockLayoutError> {
        let snapshot = model.snapshot();
        self.detach_existing_tree();
        self.auto_hide.close();
        let desired_owners = desired_owners(&snapshot);
        self.detach_before_attach(&desired_owners);
        self.owners = desired_owners;
        self.reconciling.set(true);
        let _reconciling_guard = ReconcilingGuard(self.reconciling.clone());
        self.group_items.clear();
        self.group_selected.clear();
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
        for (floating_index, floating) in snapshot.floating_roots.into_iter().enumerate() {
            let bounds = floating.bounds.into();
            let node = self.build_node(
                &floating.root,
                &mut groups,
                &mut used_groups,
                &mut used_splits,
                RootKind::Floating(floating_index),
                &[],
            )?;
            let floating_surface = DockSurfaceView::empty_surface();
            let floating_surface_root = floating_surface.content_root();
            floating_surface_root.children().add(node.element());
            let floating_auto_hide = AutoHideOverlay::new();
            floating_auto_hide.bind_pin_handler(&self.owner);
            floating_surface_root
                .children()
                .add(floating_auto_hide.visual());
            let floating_preview = DropPreview::new();
            floating_surface_root
                .children()
                .add(floating_preview.visual());
            let floating_surface_node: Rc<dyn UIElementExt> = floating_surface.clone();
            self.surfaces.register(&floating_surface_node);
            floating_roots.push((bounds, node, floating_surface));
        }
        groups.retain(|key, _| used_groups.contains_key(key));
        self.group_items
            .retain(|key, _| used_groups.contains_key(key));
        self.split_views
            .retain(|key, _| used_splits.contains_key(key));
        if let Some(root) = root.as_ref() {
            let element = root.element();
            self.surface_root.children().insert(0, element.clone());
            self.main_surface_child = Some(element);
        }
        self.groups = groups;
        self.group_hosts
            .retain(|key, _| used_groups.contains_key(key));
        self.root = root;
        self.floating = floating_roots;
        self.preview.clear();
        self.auto_hide.close();
        let registry = &self.registry;
        let strip_titles = snapshot
            .auto_hide
            .iter()
            .enumerate()
            .flat_map(move |(side, entries)| {
                entries.iter().filter_map(move |entry| {
                    registry.items.get(&entry.item).map(|item| {
                        (
                            side,
                            entry.item.clone(),
                            item.title_value(),
                            item.icon_value(),
                        )
                    })
                })
            })
            .collect::<Vec<_>>();
        self.auto_hide
            .render_strips(strip_titles.into_iter(), &self.owner);
        for item in open_auto_hide {
            self.auto_hide.open(item.clone());
            self.auto_hide
                .present_open_item(self.registry.wrapper(&item));
        }
        let floating_specs = self
            .floating
            .iter()
            .map(|(bounds, _, surface)| (*bounds, surface.clone()))
            .collect::<Vec<_>>();
        self.floating_hosts.sync(&floating_specs, &self.owner)?;
        Ok(())
    }

    pub(crate) fn begin_drag(
        &mut self,
        model: &DockLayoutModel,
        item: DockItemId,
    ) -> Result<(), DockLayoutError> {
        let Some(authored) = self.registry.items.get(&item) else {
            return Err(DockLayoutError::UnknownItem(item));
        };
        if !authored.can_dock_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit docking".to_owned(),
            });
        }
        self.drag = Some(DragSession::begin(model, item)?);
        self.preview.clear();
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

    pub(crate) fn request_float(
        &mut self,
        model: &DockLayoutModel,
        item: &DockItemId,
        bounds: crate::Rect,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let authored = self
            .registry
            .items
            .get(item)
            .ok_or_else(|| DockLayoutError::UnknownItem(item.clone()))?;
        if !authored.can_float_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit floating".to_owned(),
            });
        }
        if !floating_host_available() {
            return Err(DockLayoutError::FloatingHostUnavailable {
                reason: "the current platform has no Docking Window implementation".to_owned(),
            });
        }
        model.with_item_moved(item, crate::DockPlacement::Floating { bounds })
    }

    pub(crate) fn preview_drag(
        &mut self,
        target: crate::DockTarget,
        group: Option<SnapshotGroupKey>,
        weight: f32,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let Some(drag) = self.drag.as_mut() else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock preview requested without an active drag".to_owned(),
            });
        };
        let model = drag.preview(target, group, weight)?;
        self.preview.set_target(target);
        Ok(model)
    }

    pub(crate) fn clear_drag_target(&mut self) {
        self.preview.clear();
    }

    pub(crate) fn target_for_drop(
        &self,
        screen_position: Option<Point>,
        source_position: Point,
    ) -> Option<(crate::DockTarget, Option<SnapshotGroupKey>)> {
        let (surface, point) = if let Some(screen) = screen_position {
            self.surfaces
                .roots_for_screen_point(screen)
                .into_iter()
                .filter_map(|surface| surface.screen_to_root(screen).map(|point| (surface, point)))
                .find_map(|(surface, point)| {
                    let bounds = SurfaceRegistry::surface_bounds(&surface)?;
                    contains(bounds, point).then_some((surface, point))
                })?
        } else {
            let surface: Rc<dyn UIElementExt> = self.surface_root.clone();
            (surface, source_position)
        };
        let surface_bounds = SurfaceRegistry::surface_bounds(&surface)?;
        if !contains(surface_bounds, point) {
            return None;
        }
        let band = (surface_bounds.width.min(surface_bounds.height) * 0.10).clamp(24.0, 64.0);
        let outer = [
            (point.x <= band, crate::DockTarget::DockLeft),
            (point.y <= band, crate::DockTarget::DockTop),
            (
                point.x >= surface_bounds.width - band,
                crate::DockTarget::DockRight,
            ),
            (
                point.y >= surface_bounds.height - band,
                crate::DockTarget::DockBottom,
            ),
        ];
        if let Some((_, target)) = outer.into_iter().find(|(inside, _)| *inside) {
            return Some((target, None));
        }

        let mut deepest = None;
        let mut smallest_area = f32::INFINITY;
        for (key, group) in &self.groups {
            let group_node: Rc<dyn UIElementExt> = group.clone();
            let Some(bounds) = SurfaceRegistry::bounds_in_surface_root(&group_node, &surface)
            else {
                continue;
            };
            if !contains(bounds, point) {
                continue;
            }
            let area = bounds.width * bounds.height;
            if area < smallest_area {
                smallest_area = area;
                deepest = Some((key, bounds));
            }
        }
        let (key, bounds) = deepest?;
        let band = (bounds.width.min(bounds.height) * 0.25).clamp(24.0, 64.0);
        let local = Point {
            x: point.x - bounds.x,
            y: point.y - bounds.y,
        };
        let target = [
            (local.x <= band, crate::DockTarget::SplitLeft),
            (local.y <= band, crate::DockTarget::SplitTop),
            (
                local.x >= bounds.width - band,
                crate::DockTarget::SplitRight,
            ),
            (
                local.y >= bounds.height - band,
                crate::DockTarget::SplitBottom,
            ),
        ]
        .into_iter()
        .find(|(inside, _)| *inside)
        .map(|(_, target)| target)
        .unwrap_or(crate::DockTarget::Center);
        Some((target, Some(key.clone())))
    }

    pub(crate) fn finish_drag(&mut self, commit: bool) -> Option<DockLayoutModel> {
        let result = self.drag.take().and_then(|mut drag| {
            if commit {
                drag.commit()
            } else {
                Some(drag.cancel())
            }
        });
        self.preview.clear();
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
        self.auto_hide.open(item)
    }

    pub(crate) fn present_auto_hide(&self, item: &DockItemId) {
        self.auto_hide
            .present_open_item(self.registry.wrapper(item));
    }

    pub(crate) fn selected_group_item(&self, group: &SnapshotGroupKey) -> Option<DockItemId> {
        self.group_selected.get(group).and_then(Clone::clone)
    }

    pub(crate) fn open_auto_hide_item(&self) -> Option<DockItemId> {
        self.auto_hide.current().cloned()
    }

    pub(crate) fn can_pin(&self, item: &DockItemId) -> bool {
        self.registry
            .items
            .get(item)
            .is_some_and(|item| item.can_pin_value())
    }

    pub(crate) fn dispose(&mut self) {
        self.detach_existing_tree();
        self.surface_root.children().clear();
        self.drag = None;
        self.splitter = None;
        self.preview.clear();
        self.auto_hide.close();
        self.floating_hosts.close_empty();
        let surface_node: Rc<dyn UIElementExt> = self.surface.clone();
        self.surfaces.unregister(&surface_node);
        self.surfaces = SurfaceRegistry::default();
        self.groups.clear();
        self.group_hosts.clear();
        self.owners.clear();
        self.root = None;
        self.floating.clear();
        self.split_views.clear();
        self.group_items.clear();
        self.group_selected.clear();
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
                let host = self.group_hosts.entry(group.clone()).or_insert_with(|| {
                    let container = Grid::new();
                    container.set_rows(vec![GridLength::Star(1.0)]);
                    let pin_button = Rectangle::new();
                    pin_button.set_fill(Some(crate::core::graphics::Brush::from("#606060")));
                    pin_button.set_width(18.0);
                    pin_button.set_height(18.0);
                    pin_button.set_attached("Grid", "row", 0i32);
                    pin_button.set_attached("Grid", "column", 0i32);
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
                    GroupRuntimeHost {
                        container,
                        pin_button,
                    }
                });
                host.container.children().clear();
                host.container.children().add(view.clone());
                host.container.children().add(host.pin_button.clone());
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
        self.surfaces
            .all_surfaces()
            .into_iter()
            .find_map(|surface| {
                let bounds = SurfaceRegistry::surface_bounds(&surface)?;
                let group_bounds = SurfaceRegistry::bounds_in_surface_root(&group_node, &surface)?;
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
                distances
                    .into_iter()
                    .min_by(|(left, _), (right, _)| left.total_cmp(right))
                    .map(|(_, side)| side)
            })
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
        for (_, root, surface) in &self.floating {
            surface.content_root().children().clear();
            detach_runtime_node(root);
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
                RuntimePresentationOwner::AutoHide | RuntimePresentationOwner::None => {}
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
                    RuntimePresentationOwner::AutoHide
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
