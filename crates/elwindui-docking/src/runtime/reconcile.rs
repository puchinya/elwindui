//! Reconciliation boundary between the value model and stable runtime item wrappers.

use crate::core::ui::{ContentControlExt, UIElementExt};
use crate::snapshot::{SnapshotGroupKey, SnapshotNode};
use crate::{
    DockGroup, DockGroupId, DockItem, DockItemId, DockLayoutError, DockLayoutModel, DockSplitPanel,
};
use elwindui_custom_controls::{
    CustomSplitter, CustomSplitterExt, CustomTabView, CustomTabViewExt, CustomTabViewItem,
    CustomTabViewItemExt, TabStripPosition,
};
use std::collections::BTreeMap;
use std::rc::Rc;

use super::auto_hide::AutoHideOverlay;
use super::drag::DragSession;
use super::floating_window::FloatingHostRegistry;
use super::group_view::replace_group_items;
use super::overlay::DropPreview;
use super::split_view::SplitterSession;
use super::surface_registry::SurfaceRegistry;

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
            if let Some(content) = item.as_content() {
                wrapper.set_content(content);
            }
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
#[allow(dead_code)]
pub(crate) enum RuntimeNode {
    Group(Rc<CustomTabView>),
    Split {
        orientation: crate::Orientation,
        children: Vec<RuntimeNode>,
        splitter: Rc<CustomSplitter>,
    },
}

pub struct RuntimeRealization {
    registry: StableItemRegistry,
    groups: BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
    root: Option<RuntimeNode>,
    floating: Vec<(crate::Rect, RuntimeNode)>,
    drag: Option<DragSession>,
    splitter: Option<SplitterSession>,
    auto_hide: AutoHideOverlay,
    preview: DropPreview,
    floating_hosts: FloatingHostRegistry,
    surfaces: SurfaceRegistry,
    source_queue: LatestOnlyQueue,
}

/// Source updates use a latest-only queue while a realization is being applied.
pub(crate) struct LatestOnlyQueue {
    applying: bool,
    pending: Option<DockLayoutModel>,
}

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
    pub(crate) fn from_authored(root: &dyn UIElementExt) -> Result<Self, DockLayoutError> {
        Ok(Self {
            registry: StableItemRegistry::from_authored(root)?,
            groups: BTreeMap::new(),
            root: None,
            floating: Vec::new(),
            drag: None,
            splitter: None,
            auto_hide: AutoHideOverlay::default(),
            preview: DropPreview::new(),
            floating_hosts: FloatingHostRegistry::default(),
            surfaces: SurfaceRegistry::default(),
            source_queue: LatestOnlyQueue::new(),
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
        self.splitter = None;
        self.preview.clear();
        Ok(())
    }

    /// Applies a model transaction to the realization. All tab list changes happen after the
    /// complete model has been accepted, so a failed snapshot cannot partially mutate the UI.
    pub(crate) fn reconcile(&mut self, model: &DockLayoutModel) -> Result<(), DockLayoutError> {
        let snapshot = model.snapshot();
        let mut used_groups = BTreeMap::new();
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
            .map(|node| self.build_node(node, &mut groups, &mut used_groups))
            .transpose()?;
        let mut floating_roots = Vec::new();
        for floating in snapshot.floating_roots {
            let bounds = floating.bounds.into();
            let node = self.build_node(&floating.root, &mut groups, &mut used_groups)?;
            floating_roots.push((bounds, node));
        }
        groups.retain(|key, _| used_groups.contains_key(key));
        self.groups = groups;
        self.root = root;
        self.floating = floating_roots;
        self.preview.clear();
        self.auto_hide.close();
        for item in open_auto_hide {
            self.auto_hide.open(item);
        }
        let floating_bounds = self
            .floating
            .iter()
            .map(|(bounds, _)| *bounds)
            .collect::<Vec<_>>();
        self.floating_hosts.sync(&floating_bounds);
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
        model.with_item_moved(item, crate::DockPlacement::Floating { bounds })
    }

    pub(crate) fn preview_drag(
        &mut self,
        target: crate::DockTarget,
        group: Option<DockGroupId>,
        weight: f32,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let Some(drag) = self.drag.as_mut() else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock preview requested without an active drag".to_owned(),
            });
        };
        let model = drag.preview(target, group, weight)?.clone();
        self.preview.set_target(target);
        Ok(model)
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

    #[allow(dead_code)]
    pub(crate) fn begin_splitter(&mut self, model: &DockLayoutModel) {
        self.splitter = Some(SplitterSession::begin(model));
    }

    #[allow(dead_code)]
    pub(crate) fn preview_splitter(&mut self, model: DockLayoutModel) {
        if let Some(splitter) = self.splitter.as_mut() {
            splitter.preview(model);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn finish_splitter(&mut self, commit: bool) -> Option<DockLayoutModel> {
        self.splitter.take().and_then(|mut splitter| {
            if commit {
                splitter.commit()
            } else {
                Some(splitter.cancel())
            }
        })
    }

    #[allow(dead_code)]
    pub(crate) fn open_auto_hide(&mut self, item: DockItemId) -> Option<DockItemId> {
        self.auto_hide.open(item)
    }

    #[allow(dead_code)]
    pub(crate) fn close_auto_hide(&mut self) -> Option<DockItemId> {
        self.auto_hide.close()
    }

    #[allow(dead_code)]
    pub(crate) fn register_surface(&mut self, surface: &Rc<dyn UIElementExt>) {
        self.surfaces.register(surface);
    }

    pub(crate) fn dispose(&mut self) {
        self.drag = None;
        self.splitter = None;
        self.preview.clear();
        self.auto_hide.close();
        self.floating_hosts.close_empty();
        self.surfaces = SurfaceRegistry::default();
        self.groups.clear();
        self.root = None;
        self.floating.clear();
        self.source_queue = LatestOnlyQueue::new();
    }

    pub(crate) fn request_source_model(
        &mut self,
        current: &DockLayoutModel,
        next: DockLayoutModel,
    ) -> Option<DockLayoutModel> {
        self.source_queue.request(current, next)
    }

    pub(crate) fn finish_source_model(&mut self) -> Option<DockLayoutModel> {
        self.source_queue.finish()
    }

    fn build_node(
        &self,
        node: &SnapshotNode,
        groups: &mut BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
        used_groups: &mut BTreeMap<SnapshotGroupKey, ()>,
    ) -> Result<RuntimeNode, DockLayoutError> {
        match node {
            SnapshotNode::Group {
                group,
                items,
                selected,
            } => {
                used_groups.insert(group.clone(), ());
                let view = groups
                    .entry(group.clone())
                    .or_insert_with(CustomTabView::new_view)
                    .clone();
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
                Ok(RuntimeNode::Group(view))
            }
            SnapshotNode::Split {
                orientation,
                children,
            } => {
                let splitter = CustomSplitter::new_splitter();
                splitter.set_orientation((*orientation).into());
                let children = children
                    .iter()
                    .map(|child| self.build_node(&child.node, groups, used_groups))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(RuntimeNode::Split {
                    orientation: (*orientation).into(),
                    children,
                    splitter,
                })
            }
        }
    }
}
