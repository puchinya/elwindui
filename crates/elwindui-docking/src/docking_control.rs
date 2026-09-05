use crate::core::base::Point;
use crate::core::input::PointerEventArgs;
use crate::core::ui::Grid;
use crate::core::ui::{ContentControlExt, UIElementExt};
use crate::model::{DefaultDockDefinition, DockLayoutModel, InternalDockGroupPlacement, Node};
use crate::model::{RootKind, SplitAddress};
use crate::runtime::DragSourceGeometry;
use crate::runtime::FloatingHostId;
use crate::runtime::metrics::{FLOATING_MIN_HEIGHT, FLOATING_MIN_WIDTH};
use crate::snapshot::SnapshotGroupKey;
use crate::{DockItemId, DockLayoutError, DockPlacement};
use elwindui_custom_controls::{
    SplitterDragCompletedEventArgs, SplitterDragDeltaEventArgs, SplitterDragStartedEventArgs,
    TabDragCompletedEventArgs, TabDragMovedEventArgs, TabDragStartedEventArgs,
};
#[cfg(all(target_os = "macos", not(test)))]
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
#[cfg(all(target_os = "macos", not(test)))]
use std::future::poll_fn;
use std::rc::{Rc, Weak};
#[cfg(all(target_os = "macos", not(test)))]
use std::task::Poll;

#[cfg(all(target_os = "macos", not(test)))]
async fn wait_for_next_ui_turn() {
    let started = Rc::new(Cell::new(false));
    let started_on_poll = Rc::clone(&started);
    poll_fn(move |context| {
        if started_on_poll.replace(true) {
            Poll::Ready(())
        } else {
            let waker = context.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1));
                waker.wake();
            });
            Poll::Pending
        }
    })
    .await;
}

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
    #[environment(primary)]
    primary_brush: crate::core::theme::BrushStyle,
    #[environment(secondary)]
    secondary_brush: crate::core::theme::BrushStyle,
    #[environment(tertiary)]
    tertiary_brush: crate::core::theme::BrushStyle,
    #[environment(foreground)]
    foreground_brush: crate::core::theme::BrushStyle,
    #[environment(background)]
    background_brush: crate::core::theme::BrushStyle,
    #[environment(window_background)]
    window_background_brush: crate::core::theme::BrushStyle,
    #[environment(tint)]
    tint_brush: crate::core::theme::BrushStyle,
    #[environment(selection)]
    selection_brush: crate::core::theme::BrushStyle,
    #[environment(separator)]
    separator_brush: crate::core::theme::BrushStyle,
    #[environment(placeholder)]
    placeholder_brush: crate::core::theme::BrushStyle,
    #[environment(link)]
    link_brush: crate::core::theme::BrushStyle,
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
        on_update(
            layout,
            primary_brush,
            secondary_brush,
            tertiary_brush,
            foreground_brush,
            background_brush,
            window_background_brush,
            tint_brush,
            selection_brush,
            separator_brush,
            placeholder_brush,
            link_brush
        ) {
            this.handle_layout_update(layout);
            this.refresh_runtime_theme();
        }
        Grid {
            rows: [crate::core::layout::GridLength::Star(1.0)]
            columns: [crate::core::layout::GridLength::Star(1.0)]
            ContentPresenter {
                visibility: crate::core::layout::Visibility::Collapsed
            }
            DockRuntimeHost {
                content: runtime_surface
            }
        }
    }),
}

fn invalidate_visual_subtree(node: &Rc<dyn UIElementExt>) {
    node.invalidate_measure();
    for child in node.visual_children() {
        invalidate_visual_subtree(&child);
    }
}

fn visual_tree_root(node: &Rc<dyn UIElementExt>) -> Rc<dyn UIElementExt> {
    let mut root = Rc::clone(node);
    while let Some(parent) = root.visual_parent() {
        root = parent;
    }
    root
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Clone, Copy)]
pub(crate) enum DockTabContextAction {
    Close,
    CloseOthers,
    CloseTabsToLeft,
    CloseTabsToRight,
    Float,
    Pin,
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

    /// Publishes the current realized layout after a containing view has finished wiring its
    /// two-way binding. The authored default can be realized while the parent is still being
    /// constructed, before that binding callback exists.
    #[doc(hidden)]
    pub fn synchronize_layout_source(&self) {
        let model = self.attach_current_default(self.layout());
        self.set_layout(model.clone());
        if let Some(callback) = self.layout_change_callback() {
            callback(model);
        }
    }

    /// Removes the user layout-change callback.
    pub fn clear_on_layout_change(&self) {
        self.set_layout_change_callback(None);
    }

    #[cfg(test)]
    pub(crate) fn realization_for_test(
        &self,
    ) -> Option<Rc<RefCell<crate::runtime::RuntimeRealization>>> {
        self.runtime_realization()
    }

    #[cfg(test)]
    pub(crate) fn install_floating_host_factory_for_test(
        &self,
        factory: crate::runtime::FloatingHostFactory,
    ) {
        if let Some(realization) = self.runtime_realization() {
            realization
                .borrow_mut()
                .set_floating_host_factory_for_test(factory);
        }
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
        let model = self.layout().attach_default(
            DefaultDockDefinition::new(Some(root))
                .with_keep_empty_groups(authored_keep_empty_groups(content.as_ref())),
        );
        let realization = Rc::new(RefCell::new(
            crate::runtime::RuntimeRealization::from_authored(
                content.as_ref(),
                self.runtime_surface(),
                crate::runtime::weak_self_from_visual_owner(self),
            )
            .unwrap_or_else(|error| panic!("invalid authored docking declaration: {error}")),
        ));
        self.set_runtime_realization(Some(realization.clone()));
        let staged = realization.borrow_mut().apply_staged(&model);
        let host_sync = match staged {
            Ok(host_sync) => host_sync,
            Err(error) => {
                realization.borrow_mut().dispose();
                if self
                    .runtime_realization()
                    .is_some_and(|current| Rc::ptr_eq(&current, &realization))
                {
                    self.set_runtime_realization(None);
                }
                panic!("failed to realize authored docking declaration: {error}");
            }
        };
        self.bind_registration_callbacks(content.as_ref());
        self.set_last_applied_model(model.clone());
        self.set_layout(model.clone());
        if was_empty && !self.initial_publication_done() {
            self.set_initial_publication_done(true);
            if let Some(callback) = self.layout_change_callback() {
                callback(model.clone());
            }
        }
        self.finalize_staged_host_sync(Some(realization), &model, Some(host_sync));
    }

    pub(crate) fn authored_content(&self) -> Option<Rc<dyn UIElementExt>> {
        self.__content_opt()
    }

    fn refresh_runtime_theme(&self) {
        if let Some(realization) = self.runtime_realization() {
            realization.borrow().refresh_theme();
        }
    }

    fn apply_model(
        &self,
        model: DockLayoutModel,
    ) -> Result<
        (
            Option<Rc<RefCell<crate::runtime::RuntimeRealization>>>,
            Option<crate::runtime::PreparedFloatingHostSync>,
        ),
        DockLayoutError,
    > {
        let runtime_before = self.runtime_realization();
        let host_sync = runtime_before
            .as_ref()
            .map(|realization| realization.borrow_mut().apply_staged(&model))
            .transpose()?;
        self.set_last_applied_model(model.clone());
        self.set_layout(model.clone());
        Ok((runtime_before, host_sync))
    }

    fn finalize_staged_host_sync(
        &self,
        runtime_before: Option<Rc<RefCell<crate::runtime::RuntimeRealization>>>,
        expected_model: &DockLayoutModel,
        host_sync: Option<crate::runtime::PreparedFloatingHostSync>,
    ) {
        let Some(host_sync) = host_sync else {
            return;
        };
        let runtime_is_current = runtime_before.as_ref().is_some_and(|runtime| {
            self.runtime_realization()
                .is_some_and(|current| Rc::ptr_eq(runtime, &current))
        });
        if runtime_is_current && self.last_applied_model() == *expected_model {
            runtime_before
                .expect("runtime was present when the staged model was prepared")
                .borrow_mut()
                .commit_floating_host_sync(host_sync);
        } else {
            // A callback or generated source update may have replaced the owner/runtime before
            // the native resources were committed. They belong to the abandoned transaction.
            host_sync.abort();
        }
    }

    fn commit_user_model(&self, model: DockLayoutModel) -> Result<(), DockLayoutError> {
        let (runtime_before, host_sync) = self.apply_model(model.clone())?;
        if let Some(callback) = self.layout_change_callback() {
            callback(model.clone());
        }
        self.finalize_staged_host_sync(runtime_before, &model, host_sync);
        Ok(())
    }

    fn commit_user_value_only(&self, model: DockLayoutModel) {
        self.set_last_applied_model(model.clone());
        self.set_layout(model.clone());
        if let Some(callback) = self.layout_change_callback() {
            callback(model);
        }
        self.invalidate_containing_visual_subtree();
    }

    fn commit_source_model(&self, model: DockLayoutModel) -> Result<(), DockLayoutError> {
        let (runtime_before, host_sync) = self.apply_model(model.clone())?;
        self.finalize_staged_host_sync(runtime_before, &model, host_sync);
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
        let model = current.attach_default(
            DefaultDockDefinition::new(Some(root))
                .with_keep_empty_groups(authored_keep_empty_groups(content.as_ref())),
        );
        let runtime_before = realization.clone();
        let host_sync = {
            let mut realization = runtime_before.borrow_mut();
            realization
                .refresh_authored(content.as_ref())
                .unwrap_or_else(|error| panic!("invalid authored docking registration: {error}"));
            realization.apply_staged(&model).unwrap_or_else(|error| {
                panic!("failed to reconcile authored docking registration: {error}")
            })
        };
        self.bind_registration_callbacks(content.as_ref());
        if model != current {
            self.set_last_applied_model(model.clone());
            self.set_layout(model.clone());
            if let Some(callback) = self.layout_change_callback() {
                callback(model.clone());
            }
        }
        self.finalize_staged_host_sync(Some(runtime_before), &model, Some(host_sync));
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
            if let Err(error) = self.commit_source_model(normalized.clone()) {
                self.set_applying_source(false);
                panic!("failed to apply source DockLayoutModel: {error}");
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
        // A DockingControl layout update can replace several native descendants while its
        // containing view remains otherwise unchanged. In AppKit the containing layout group is
        // also responsible for repainting sibling native controls (for example, a toolbar above
        // the docking surface). Invalidate that containing subtree so those siblings get their
        // native presentation refreshed even when their measured sizes did not change.
        self.invalidate_containing_visual_subtree();
    }

    fn invalidate_containing_visual_subtree(&self) {
        if let Some(parent) = self.visual_parent() {
            let root = visual_tree_root(&parent);
            invalidate_visual_subtree(&root);
        } else {
            self.invalidate_measure();
        }
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
        model.attach_default(
            DefaultDockDefinition::new(Some(root))
                .with_keep_empty_groups(authored_keep_empty_groups(content.as_ref())),
        )
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
            let weak_group: Weak<crate::DockingControl> = weak.clone();
            group.bind_registration_callback(Some(Rc::new(move || {
                let owner: Option<Rc<crate::DockingControl>> = weak_group.upgrade();
                if let Some(owner) = owner {
                    owner.on_registration_changed();
                }
            })));
            for item in group.authored_children() {
                let weak_item: Weak<crate::DockingControl> = weak.clone();
                item.bind_registration_callback(Some(Rc::new(move || {
                    let owner: Option<Rc<crate::DockingControl>> = weak_item.upgrade();
                    if let Some(owner) = owner {
                        owner.on_registration_changed();
                    }
                })));
            }
        } else if let Some(panel) = root.as_any().downcast_ref::<crate::DockSplitPanel>() {
            let weak_panel: Weak<crate::DockingControl> = weak.clone();
            panel.bind_registration_callback(Some(Rc::new(move || {
                let owner: Option<Rc<crate::DockingControl>> = weak_panel.upgrade();
                if let Some(owner) = owner {
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
        let current = self.layout();
        let Ok(next) = current.with_item_activated(&item) else {
            return;
        };
        if next != current {
            let fast_path = self.runtime_realization().is_some_and(|realization| {
                realization
                    .borrow_mut()
                    .apply_selection_fast_path(&current, &next, &group, index, &item)
            });
            if fast_path {
                self.commit_user_value_only(next);
            } else {
                let _ = self.commit_user_model(next);
            }
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

    pub(crate) fn handle_group_title_close(&self, group: SnapshotGroupKey) {
        let Some(index) = self
            .runtime_realization()
            .and_then(|realization| realization.borrow().selected_group_index(&group))
        else {
            return;
        };
        self.handle_group_close(group, index);
    }

    pub(crate) fn handle_group_float(&self, group: SnapshotGroupKey) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        if !realization.borrow().can_float_group(&group) {
            return;
        }
        let Some(bounds) = realization.borrow().context_group_floating_bounds(&group) else {
            return;
        };
        let model = self.layout();
        let Ok(next) = model.with_group_moved_internal(
            &group.clone().into(),
            InternalDockGroupPlacement::Floating { bounds },
        ) else {
            return;
        };
        if next != model {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_tab_context_action(&self, item: DockItemId, action: DockTabContextAction) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let Some((group, index)) = realization.borrow().group_for_item(&item) else {
            return;
        };
        let items = realization
            .borrow()
            .group_items_for(&group)
            .unwrap_or_default();
        match action {
            DockTabContextAction::Close => {
                if realization.borrow().can_close(&item) {
                    self.close_context_items(vec![item]);
                }
            }
            DockTabContextAction::CloseOthers => {
                let targets = items
                    .into_iter()
                    .enumerate()
                    .filter(|(candidate, _)| *candidate != index)
                    .map(|(_, item)| item)
                    .collect();
                self.close_context_items(targets);
            }
            DockTabContextAction::CloseTabsToLeft => {
                self.close_context_items(items.into_iter().take(index).collect());
            }
            DockTabContextAction::CloseTabsToRight => {
                self.close_context_items(items.into_iter().skip(index + 1).collect());
            }
            DockTabContextAction::Float => {
                if !realization.borrow().can_float(&item) {
                    return;
                }
                let Some(bounds) = realization.borrow().context_floating_bounds(&item) else {
                    return;
                };
                let model = self.layout();
                let Ok(next) = model.with_item_moved(&item, DockPlacement::Floating { bounds })
                else {
                    return;
                };
                if next != model {
                    let _ = self.commit_user_model(next);
                }
            }
            DockTabContextAction::Pin => self.commit_pin_gesture(item),
        }
    }

    fn close_context_items(&self, items: Vec<DockItemId>) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let closeable = items
            .into_iter()
            .filter(|item| realization.borrow().can_close(item))
            .collect::<Vec<_>>();
        if closeable.is_empty() {
            return;
        }
        let mut next = self.layout();
        for item in closeable {
            let Ok(updated) = next.with_item_closed(&item) else {
                return;
            };
            next = updated;
        }
        if next != self.layout() {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_auto_hide_open(&self, root: RootKind, item: DockItemId) {
        let current = self.layout();
        let Ok(next) = current.with_item_activated(&item) else {
            return;
        };
        if next == current {
            if let Some(realization) = self.runtime_realization() {
                let mut realization = realization.borrow_mut();
                realization.open_auto_hide_on(root, item.clone());
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

    pub(crate) fn handle_group_drag_started(
        &self,
        group: SnapshotGroupKey,
        event: PointerEventArgs,
    ) {
        if let Some(realization) = self.runtime_realization() {
            let _ =
                realization
                    .borrow_mut()
                    .begin_group_drag(&self.layout(), group, event.position);
        }
    }

    pub(crate) fn handle_group_drag_moved(&self, group: SnapshotGroupKey, event: PointerEventArgs) {
        if let Some(realization) = self.runtime_realization() {
            let mut realization = realization.borrow_mut();
            if !realization.can_dock_group(&group) {
                realization.clear_drag_target();
                return;
            }
            let Some(target) = realization.target_for_drop(event.screen_position, event.position)
            else {
                realization.clear_drag_target();
                return;
            };
            let _ = realization.preview_drag(&target, 1.0);
        }
    }

    pub(crate) fn handle_group_drag_completed(
        &self,
        group: SnapshotGroupKey,
        event: PointerEventArgs,
        canceled: bool,
    ) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        if canceled {
            realization.borrow_mut().finish_drag(false);
            return;
        }

        let target = realization
            .borrow()
            .target_for_drop(event.screen_position, event.position);
        if let Some(target) = target {
            if !realization.borrow().can_dock_group(&group) {
                realization.borrow_mut().finish_drag(false);
                return;
            }
            let next = {
                let mut current = realization.borrow_mut();
                let _ = current.preview_drag(&target, 1.0);
                current.finish_drag(true)
            };
            if let Some(next) = next
                && next != self.layout()
            {
                let _ = self.commit_user_model(next);
            }
            return;
        }

        let Some(screen) = event.screen_position else {
            realization.borrow_mut().finish_drag(false);
            return;
        };
        let group = realization.borrow().drag_group();
        let geometry = realization.borrow().drag_source_geometry();
        let (Some(group), Some(geometry)) = (group, geometry) else {
            realization.borrow_mut().finish_drag(false);
            return;
        };
        if !realization.borrow().can_float_group(&group) {
            realization.borrow_mut().finish_drag(false);
            return;
        }
        let Some(bounds) = floating_bounds(&geometry, screen) else {
            realization.borrow_mut().finish_drag(false);
            return;
        };
        let next = {
            let mut current = realization.borrow_mut();
            let Ok(next) = current.group_floating_candidate(bounds) else {
                current.finish_drag(false);
                return;
            };
            next
        };
        let result = self.commit_user_model(next);
        realization.borrow_mut().finish_drag(result.is_ok());
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
            let _ = realization
                .borrow_mut()
                .begin_drag(&self.layout(), item, args.position);
        }
    }

    pub(crate) fn handle_tab_drag_moved(
        &self,
        group: SnapshotGroupKey,
        args: TabDragMovedEventArgs,
    ) {
        if let Some(realization) = self.runtime_realization() {
            let mut realization = realization.borrow_mut();
            let Some(target) = realization.target_for_drop(args.screen_position, args.position)
            else {
                realization.clear_drag_target();
                return;
            };
            // The realization owns the target/adornment state. No model reconciliation occurs in
            // this path; CustomTabView remains the sole threshold/capture state machine.
            let _ = realization.preview_drag(&target, 1.0);
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
        if args.canceled {
            realization.borrow_mut().finish_drag(false);
            return;
        }

        let target = {
            realization
                .borrow()
                .target_for_drop(args.screen_position, args.position)
        };
        if let Some(target) = target {
            let next = {
                let mut current = realization.borrow_mut();
                let _ = current.preview_drag(&target, 1.0);
                current.finish_drag(true)
            };
            if let Some(next) = next
                && next != self.layout()
            {
                let _ = self.commit_user_model(next);
            }
            return;
        }

        let Some(screen) = args.screen_position else {
            realization.borrow_mut().finish_drag(false);
            return;
        };
        let item = realization.borrow().drag_item();
        let geometry = realization.borrow().drag_source_geometry();
        let (Some(item), Some(geometry)) = (item, geometry) else {
            realization.borrow_mut().finish_drag(false);
            return;
        };
        let Some(bounds) = floating_bounds(&geometry, screen) else {
            realization.borrow_mut().finish_drag(false);
            return;
        };
        if !realization.borrow().can_float(&item) {
            realization.borrow_mut().finish_drag(false);
            return;
        }

        let next = {
            let mut current = realization.borrow_mut();
            let Ok(next) = current.floating_candidate(bounds) else {
                current.finish_drag(false);
                return;
            };
            next
        };
        let result = self.commit_user_model(next);
        realization.borrow_mut().finish_drag(result.is_ok());
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
            self.commit_user_value_only(next);
        }
    }

    pub(crate) fn handle_floating_close_host(&self, host_id: FloatingHostId) -> bool {
        let Some(realization) = self.runtime_realization() else {
            return false;
        };
        let Some(index) = realization.borrow().floating_root_index(host_id) else {
            return false;
        };
        let current = self.layout();
        let items = current.floating_item_ids(index);
        if items.is_empty() || !realization.borrow().all_closeable(&items) {
            return true;
        }
        if !realization
            .borrow_mut()
            .begin_native_floating_close(host_id)
        {
            return true;
        }

        let mut next = current.clone();
        for item in &items {
            let Ok(updated) = next.with_item_closed(item) else {
                realization
                    .borrow_mut()
                    .cancel_native_floating_close(host_id);
                return true;
            };
            next = updated;
        }
        let Ok(next_without_empty_host) = next.without_empty_floating_root(index) else {
            realization
                .borrow_mut()
                .cancel_native_floating_close(host_id);
            return true;
        };
        next = next_without_empty_host;
        if next == current {
            realization
                .borrow_mut()
                .cancel_native_floating_close(host_id);
            return false;
        }

        #[cfg(all(target_os = "macos", not(test)))]
        {
            let weak_owner = crate::runtime::weak_self_from_visual_owner(self);
            elwindui::core::task::spawn_local(async move {
                // AppKit's close delegate calls this handler before the native window starts its
                // close transaction. Let that callback return first; otherwise the main host can
                // reconcile its sibling native islands while AppKit still owns the closing host.
                wait_for_next_ui_turn().await;
                let Some(owner) = weak_owner.upgrade() else {
                    return;
                };
                if owner.layout() != current {
                    owner.runtime_realization().map(|realization| {
                        realization
                            .borrow_mut()
                            .cancel_native_floating_close(host_id)
                    });
                    return;
                }
                match owner.commit_user_model(next) {
                    Ok(())
                        if owner.runtime_realization().is_some_and(|realization| {
                            realization.borrow().floating_root_index(host_id).is_none()
                        }) => {}
                    Ok(()) | Err(_) => {
                        if let Some(realization) = owner.runtime_realization() {
                            realization
                                .borrow_mut()
                                .cancel_native_floating_close(host_id);
                        }
                    }
                }
            });
            false
        }

        #[cfg(any(not(target_os = "macos"), test))]
        match self.commit_user_model(next) {
            Ok(()) if realization.borrow().floating_root_index(host_id).is_none() => false,
            Ok(()) | Err(_) => {
                realization
                    .borrow_mut()
                    .cancel_native_floating_close(host_id);
                true
            }
        }
    }

    pub(crate) fn handle_floating_bounds_changed(
        &self,
        host_id: FloatingHostId,
        bounds: crate::Rect,
    ) {
        let Some(realization) = self.runtime_realization() else {
            return;
        };
        let index = {
            // AppKit may synchronously report the bounds while a native host sync still holds
            // the realization mutably. That callback is part of the sync transaction and must not
            // recursively borrow the same RefCell.
            let Ok(realization) = realization.try_borrow() else {
                return;
            };
            if realization.native_bounds_syncing() {
                return;
            }
            realization.floating_root_index(host_id)
        };
        let Some(index) = index else { return };
        let model = self.layout();
        let Ok(next) = model.with_floating_bounds(index, bounds) else {
            return;
        };
        if next != model {
            let _ = self.commit_user_model(next);
        }
    }

    pub(crate) fn handle_pin_gesture(&self, root: RootKind) {
        let model = self.layout();
        let item = self
            .runtime_realization()
            .and_then(|realization| realization.borrow().open_auto_hide_item_on(&root))
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

fn floating_bounds(source: &DragSourceGeometry, screen_position: Point) -> Option<crate::Rect> {
    if !source.source_bounds_host.x.is_finite()
        || !source.source_bounds_host.y.is_finite()
        || !source.source_bounds_host.width.is_finite()
        || !source.source_bounds_host.height.is_finite()
        || source.source_bounds_host.width < 0.0
        || source.source_bounds_host.height < 0.0
        || !source.pointer_offset.x.is_finite()
        || !source.pointer_offset.y.is_finite()
        || !screen_position.x.is_finite()
        || !screen_position.y.is_finite()
    {
        return None;
    }
    let width = source.source_bounds_host.width.max(FLOATING_MIN_WIDTH);
    let height = source.source_bounds_host.height.max(FLOATING_MIN_HEIGHT);
    let bounds = crate::Rect {
        x: screen_position.x - source.pointer_offset.x,
        y: screen_position.y - source.pointer_offset.y,
        width,
        height,
    };
    (bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width >= FLOATING_MIN_WIDTH
        && bounds.height >= FLOATING_MIN_HEIGHT)
        .then_some(bounds)
}

#[cfg(test)]
pub(crate) fn floating_bounds_for_test(
    source: &DragSourceGeometry,
    screen_position: Point,
) -> Option<crate::Rect> {
    floating_bounds(source, screen_position)
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

fn authored_keep_empty_groups(element: &dyn UIElementExt) -> BTreeSet<crate::DockGroupId> {
    fn collect(element: &dyn UIElementExt, groups: &mut BTreeSet<crate::DockGroupId>) {
        if let Some(group) = element.as_any().downcast_ref::<crate::DockGroup>() {
            if group.show_when_empty_value() {
                groups.insert(group.id_value());
            }
        } else if let Some(split) = element.as_any().downcast_ref::<crate::DockSplitPanel>() {
            for child in split.authored_children() {
                collect(child.as_ref(), groups);
            }
        }
    }
    let mut groups = BTreeSet::new();
    collect(element, &mut groups);
    groups
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
