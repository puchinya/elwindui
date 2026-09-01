use crate::core::ui::Grid;
use crate::core::ui::{ContentControlExt, UIElementExt};
use crate::model::SplitAddress;
use crate::model::{DefaultDockDefinition, DockLayoutModel, Node};
use crate::snapshot::SnapshotGroupKey;
use crate::{DockItemId, DockLayoutError};
use elwindui_custom_controls::{
    SplitterDragCompletedEventArgs, SplitterDragDeltaEventArgs, SplitterDragStartedEventArgs,
    TabDragCompletedEventArgs, TabDragMovedEventArgs, TabDragStartedEventArgs,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Private marker used to distinguish the visible retained runtime host from the collapsed
/// authored declaration presenter. It is exported only as a hidden macro support type.
#[elwindui::component(inherits ContentControl)]
#[doc(hidden)]
pub struct DockRuntimeHost {
    template: template_view!(|_this: Self| { ContentPresenter {} }),
}

#[elwindui::component]
impl DockRuntimeHost {}

/// Root authored declaration and runtime coordinator for one dock surface.
#[elwindui::component(inherits ContentControl)]
pub struct DockingControl {
    #[prop(default = crate::dock_layout_model::empty())]
    #[two_way]
    layout: crate::dock_layout_model,
    #[state(default = None)]
    layout_change_callback: Option<Rc<dyn Fn(DockLayoutModel)>>,
    #[state(default = None)]
    runtime_realization: Option<Rc<RefCell<crate::runtime::RuntimeRealization>>>,
    #[state(default = crate::runtime::DockSurfaceView::empty_surface())]
    runtime_surface: Rc<crate::runtime::DockSurfaceView>,
    #[state(default = crate::dock_layout_model::empty())]
    last_applied_model: crate::dock_layout_model,
    #[state(default = false)]
    applying_source: bool,
    #[state(default = None)]
    pending_source: Option<crate::dock_layout_model>,
    #[state(default = false)]
    registration_refreshing: bool,
    #[state(default = false)]
    initial_publication_done: bool,
    template: template_view!(|this: Self| {
        on_mount {
            this.capture_authored_default();
        }
        on_unmount {
            this.dispose_runtime();
        }
        on_update(layout) {
            this.handle_layout_update(layout);
        }
        Grid {
            ContentPresenter {
                visibility: crate::core::layout::Visibility::Collapsed
            }
            DockRuntimeHost {
                content: runtime_surface
            }
        }
    }),
}

#[elwindui::component]
impl DockingControl {}

impl DockingControl {
    /// Creates a docking control with an empty layout until authored content is attached.
    pub fn new_docking() -> Rc<Self> {
        Self::new()
    }

    /// Installs the callback raised once for each committed user layout change.
    pub fn set_on_layout_change(&self, callback: Box<dyn Fn(DockLayoutModel)>) {
        self.set_layout_change_callback(Some(Rc::from(callback)));
    }

    /// Removes the user layout-change callback.
    pub fn clear_on_layout_change(&self) {
        self.set_layout_change_callback(None);
    }

    fn capture_authored_default(&self) {
        let Some(content) = self.authored_content() else {
            return;
        };
        apply_authored_templates(content.as_ref());
        let was_empty = self.layout().is_empty();
        let mut seen_items = HashSet::new();
        let mut seen_groups = HashSet::new();
        let root = authored_node(content.as_ref(), &mut seen_items, &mut seen_groups)
            .unwrap_or_else(|| {
                panic!("DockingControl content must be DockGroup or DockSplitPanel")
            });
        let model = self
            .layout()
            .attach_default(DefaultDockDefinition::new(Some(root)));
        let mut realization = crate::runtime::RuntimeRealization::from_authored(
            content.as_ref(),
            self.runtime_surface(),
            crate::runtime::weak_self_from_visual_owner(self),
        )
        .unwrap_or_else(|error| panic!("invalid authored docking declaration: {error}"));
        realization.reconcile(&model).unwrap_or_else(|error| {
            panic!("failed to realize authored docking declaration: {error}")
        });
        self.set_runtime_realization(Some(Rc::new(RefCell::new(realization))));
        self.bind_registration_callbacks(content.as_ref());
        self.set_last_applied_model(model.clone());
        self.set_layout(model.clone());
        if was_empty && !self.initial_publication_done() {
            self.set_initial_publication_done(true);
            if let Some(callback) = self.layout_change_callback() {
                callback(model);
            }
        }
    }

    pub(crate) fn authored_content(&self) -> Option<Rc<dyn UIElementExt>> {
        self.__content_opt()
    }

    fn apply_model(&self, model: DockLayoutModel) -> Result<(), DockLayoutError> {
        if let Some(realization) = self.runtime_realization() {
            let previous = self.last_applied_model();
            if let Err(error) = realization.borrow_mut().reconcile(&model) {
                // Native floating-host creation happens at the end of reconciliation. If it
                // fails, restore the committed projection before returning the typed error so
                // the source model and its retained ownership remain unchanged.
                let _ = realization.borrow_mut().reconcile(&previous);
                return Err(error);
            }
        }
        self.set_last_applied_model(model.clone());
        self.set_layout(model);
        Ok(())
    }

    fn commit_user_model(&self, model: DockLayoutModel) -> Result<(), DockLayoutError> {
        self.apply_model(model.clone())?;
        if let Some(callback) = self.layout_change_callback() {
            callback(model);
        }
        Ok(())
    }

    fn dispose_runtime(&self) {
        if let Some(realization) = self.runtime_realization() {
            realization.borrow_mut().dispose();
        }
        self.set_runtime_realization(None);
    }

    fn refresh_authored_registration(&self) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let Some(content) = self.authored_content() else {
            return;
        };
        apply_authored_templates(content.as_ref());
        let mut seen_items = HashSet::new();
        let mut seen_groups = HashSet::new();
        let root = authored_node(content.as_ref(), &mut seen_items, &mut seen_groups)
            .unwrap_or_else(|| {
                panic!("DockingControl content must be DockGroup or DockSplitPanel")
            });
        let current = self.layout();
        let model = current.attach_default(DefaultDockDefinition::new(Some(root)));
        let mut realization = realization.borrow_mut();
        realization
            .refresh_authored(content.as_ref())
            .unwrap_or_else(|error| panic!("invalid authored docking registration: {error}"));
        realization.reconcile(&model).unwrap_or_else(|error| {
            panic!("failed to reconcile authored docking registration: {error}")
        });
        self.bind_registration_callbacks(content.as_ref());
        drop(realization);
        if model != current {
            self.set_last_applied_model(model.clone());
            self.set_layout(model.clone());
            if let Some(callback) = self.layout_change_callback() {
                callback(model);
            }
        }
    }

    fn handle_layout_update(&self, incoming: DockLayoutModel) {
        if incoming == self.last_applied_model() {
            return;
        }
        if self.applying_source() {
            self.set_pending_source(Some(incoming));
            return;
        }
        self.set_applying_source(true);
        let mut candidate = incoming;
        loop {
            if let Some(realization) = self.runtime_realization() {
                realization.borrow_mut().cancel_transient();
            }
            let normalized = self.attach_current_default(candidate);
            let result = self
                .runtime_realization()
                .map(|realization| realization.borrow_mut().reconcile(&normalized));
            if let Some(Err(error)) = result {
                self.set_applying_source(false);
                panic!("failed to apply source DockLayoutModel: {error}");
            }
            self.set_last_applied_model(normalized.clone());
            if normalized != self.layout() {
                self.set_layout(normalized);
            }
            let pending = self.pending_source();
            self.set_pending_source(None);
            if let Some(next) = pending {
                candidate = next;
            } else {
                break;
            }
        }
        self.set_applying_source(false);
    }

    fn attach_current_default(&self, model: DockLayoutModel) -> DockLayoutModel {
        let Some(content) = self.authored_content() else {
            return model;
        };
        let mut seen_items = HashSet::new();
        let mut seen_groups = HashSet::new();
        let root = authored_node(content.as_ref(), &mut seen_items, &mut seen_groups)
            .unwrap_or_else(|| {
                panic!("DockingControl content must be DockGroup or DockSplitPanel")
            });
        model.attach_default(DefaultDockDefinition::new(Some(root)))
    }

    fn on_registration_changed(&self) {
        if self.registration_refreshing() {
            return;
        }
        self.set_registration_refreshing(true);
        self.refresh_authored_registration();
        self.set_registration_refreshing(false);
    }

    fn bind_registration_callbacks(&self, root: &dyn UIElementExt) {
        let weak = crate::runtime::weak_self_from_visual_owner(self);
        if let Some(group) = root.as_any().downcast_ref::<crate::DockGroup>() {
            let weak_group = weak.clone();
            group.bind_registration_callback(Some(Rc::new(move || {
                if let Some(owner) = weak_group.upgrade() {
                    owner.on_registration_changed();
                }
            })));
            for item in group.authored_children() {
                let weak_item = weak.clone();
                item.bind_registration_callback(Some(Rc::new(move || {
                    if let Some(owner) = weak_item.upgrade() {
                        owner.on_registration_changed();
                    }
                })));
            }
        } else if let Some(panel) = root.as_any().downcast_ref::<crate::DockSplitPanel>() {
            let weak_panel = weak.clone();
            panel.bind_registration_callback(Some(Rc::new(move || {
                if let Some(owner) = weak_panel.upgrade() {
                    owner.on_registration_changed();
                }
            })));
            for child in panel.authored_children() {
                self.bind_registration_callbacks(child.as_ref());
            }
        }
    }

    pub(crate) fn handle_group_selected(&self, group: SnapshotGroupKey, index: usize) {
        let Some(item) = self
            .runtime_realization()
            .and_then(|realization| realization.borrow().group_item(&group, index))
        else {
            return;
        };
        let Ok(next) = self.layout().with_item_activated(&item) else {
            return;
        };
        if next != self.layout() {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_group_close(&self, group: SnapshotGroupKey, index: usize) {
        let Some(item) = self
            .runtime_realization()
            .and_then(|realization| realization.borrow().group_item(&group, index))
        else {
            return;
        };
        let next = if let Some(realization) = self.runtime_realization() {
            realization
                .borrow_mut()
                .request_close(&self.layout(), &item)
                .ok()
        } else {
            self.layout().with_item_closed(&item).ok()
        };
        if let Some(next) = next
            && next != self.layout()
        {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_auto_hide_open(&self, item: DockItemId) {
        let current = self.layout();
        let Ok(next) = current.with_item_activated(&item) else {
            return;
        };
        if next == current {
            if let Some(realization) = self.runtime_realization() {
                let mut realization = realization.borrow_mut();
                realization.open_auto_hide(item.clone());
                realization.present_auto_hide(&item);
            }
            return;
        }
        let _ = self.commit_user_model(next);
    }

    pub(crate) fn handle_group_pin(&self, group: SnapshotGroupKey) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let Some(item) = realization.borrow().selected_group_item(&group) else {
            return;
        };
        self.commit_pin_gesture(item);
    }

    pub(crate) fn handle_tab_drag_started(
        &self,
        group: SnapshotGroupKey,
        args: TabDragStartedEventArgs,
    ) {
        let Some(item) = self
            .runtime_realization()
            .and_then(|realization| realization.borrow().group_item(&group, args.index))
        else {
            return;
        };
        if let Some(realization) = self.runtime_realization() {
            let _ = realization.borrow_mut().begin_drag(&self.layout(), item);
        }
    }

    pub(crate) fn handle_tab_drag_moved(
        &self,
        group: SnapshotGroupKey,
        args: TabDragMovedEventArgs,
    ) {
        if let Some(realization) = self.runtime_realization() {
            let mut realization = realization.borrow_mut();
            let Some((target, target_group)) =
                realization.target_for_drop(args.screen_position, args.position)
            else {
                realization.clear_drag_target();
                return;
            };
            // The realization owns the target/adornment state. No model reconciliation occurs in
            // this path; CustomTabView remains the sole threshold/capture state machine.
            let _ = realization.preview_drag(target, target_group, 1.0);
        }
        let _ = group;
    }

    pub(crate) fn handle_tab_drag_completed(
        &self,
        _group: SnapshotGroupKey,
        args: TabDragCompletedEventArgs,
    ) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let mut floating_fallback = None;
        if !args.canceled {
            let mut current = realization.borrow_mut();
            let Some((target, target_group)) =
                current.target_for_drop(args.screen_position, args.position)
            else {
                if let (Some(screen), Some(item)) = (args.screen_position, current.drag_item()) {
                    let bounds = crate::Rect {
                        x: screen.x,
                        y: screen.y,
                        width: 480.0,
                        height: 320.0,
                    };
                    floating_fallback = current.request_float(&self.layout(), &item, bounds).ok();
                }
                current.clear_drag_target();
                current.finish_drag(false);
                if floating_fallback.is_none() {
                    return;
                }
                drop(current);
                if let Some(next) = floating_fallback {
                    let _ = self.commit_user_model(next);
                }
                return;
            };
            let _ = current.preview_drag(target, target_group, 1.0);
        }
        let next = realization.borrow_mut().finish_drag(!args.canceled);
        if args.canceled {
            return;
        }
        if let Some(next) = next
            && next != self.layout()
        {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_splitter_started(
        &self,
        address: SplitAddress,
        boundary: usize,
        grid: Rc<Grid>,
        orientation: crate::Orientation,
        _args: SplitterDragStartedEventArgs,
    ) {
        if let Some(realization) = self.runtime_realization() {
            let _ = realization.borrow_mut().begin_splitter(
                &self.layout(),
                address,
                boundary,
                grid,
                orientation,
            );
        }
    }

    pub(crate) fn handle_splitter_delta(&self, args: SplitterDragDeltaEventArgs) {
        if let Some(realization) = self.runtime_realization() {
            realization
                .borrow_mut()
                .preview_splitter(args.cumulative_delta);
        }
    }

    pub(crate) fn handle_splitter_completed(&self, args: SplitterDragCompletedEventArgs) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let next = realization.borrow_mut().finish_splitter(args.canceled);
        if args.canceled {
            return;
        }
        if let Some(next) = next
            && next != self.layout()
        {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_floating_close(&self, floating_index: usize) -> bool {
        let items = self.layout().floating_item_ids(floating_index);
        let Some(realization) = self.runtime_realization() else {
            return false;
        };
        if !realization.borrow().all_closeable(&items) {
            return true;
        }
        let mut next = self.layout();
        for item in &items {
            let Ok(updated) = next.with_item_closed(item) else {
                return false;
            };
            next = updated;
        }
        if next != self.layout() {
            let _ = self.commit_user_model(next);
        }
        true
    }

    pub(crate) fn handle_pin_gesture(&self) {
        let model = self.layout();
        let item = self
            .runtime_realization()
            .and_then(|realization| realization.borrow().open_auto_hide_item())
            .or_else(|| model.selected_item_id());
        let Some(item) = item else {
            return;
        };
        self.commit_pin_gesture(item);
    }

    fn commit_pin_gesture(&self, item: DockItemId) {
        let model = self.layout();
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        if !realization.borrow().can_pin(&item) {
            return;
        }
        let next = if model.is_item_auto_hidden(&item) {
            model.with_item_unpinned(&item)
        } else {
            let Some(side) = realization.borrow().nearest_pin_side(&item) else {
                return;
            };
            realization.borrow_mut().request_pin(&model, &item, side)
        };
        if let Ok(next) = next
            && next != model
        {
            let _ = self.commit_user_model(next);
        }
    }
}

fn authored_node(
    element: &dyn UIElementExt,
    seen_items: &mut HashSet<DockItemId>,
    seen_groups: &mut HashSet<crate::DockGroupId>,
) -> Option<Node> {
    if let Some(group) = element.as_any().downcast_ref::<crate::DockGroup>() {
        let id = group.id_value();
        assert!(!id.as_ref().is_empty(), "DockGroupId must not be empty");
        assert!(
            seen_groups.insert(id.clone()),
            "duplicate DockGroupId: {id}"
        );
        let children = group.authored_children();
        let items = children
            .iter()
            .map(|item| {
                let id = item.id_value();
                assert!(!id.as_ref().is_empty(), "DockItemId must not be empty");
                assert!(seen_items.insert(id.clone()), "duplicate DockItemId: {id}");
                id
            })
            .collect::<Vec<_>>();
        let selected = items.first().cloned();
        return Some(Node::Group {
            group: crate::model::InternalDockGroupKey::Authored(id),
            items,
            selected,
        });
    }
    if let Some(split) = element.as_any().downcast_ref::<crate::DockSplitPanel>() {
        let mut children = Vec::new();
        for child in split.authored_children() {
            let weight = if let Some(group) = child.as_any().downcast_ref::<crate::DockGroup>() {
                group.weight_value()
            } else if let Some(nested) = child.as_any().downcast_ref::<crate::DockSplitPanel>() {
                nested.weight_value()
            } else {
                panic!("DockSplitPanel children must be DockGroup or DockSplitPanel");
            };
            assert!(
                weight.is_finite() && weight > 0.0,
                "Dock declaration weight must be finite and positive"
            );
            let node = authored_node(child.as_ref(), seen_items, seen_groups)
                .unwrap_or_else(|| panic!("invalid DockSplitPanel child"));
            children.push(crate::model::WeightedNode { weight, node });
        }
        assert!(
            !children.is_empty(),
            "DockSplitPanel must have at least one child"
        );
        return Some(Node::Split {
            orientation: split.orientation_value(),
            children,
        });
    }
    None
}

fn apply_authored_templates(element: &dyn UIElementExt) {
    element.apply_template();
    if let Some(group) = element.as_any().downcast_ref::<crate::DockGroup>() {
        for item in group.authored_children() {
            item.apply_template();
        }
    } else if let Some(panel) = element.as_any().downcast_ref::<crate::DockSplitPanel>() {
        for child in panel.authored_children() {
            apply_authored_templates(child.as_ref());
        }
    }
}
