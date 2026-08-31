use crate::core::ui::{ContentControlExt, UIElementExt};
use crate::model::{DefaultDockDefinition, DockLayoutModel, Node};
use crate::{DockGroupId, DockItemId, DockLayoutError, DockPlacement, DockSide, DockTarget, Rect};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

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
    template: template_view!(|this: Self| {
        on_mount {
            this.capture_authored_default();
        }
        on_unmount {
            this.dispose_runtime();
        }
        on_update(layout) {
            this.refresh_authored_registration();
        }
        ContentPresenter {}
    }),
}

#[elwindui::component]
impl DockingControl {}

impl DockingControl {
    /// Creates a docking control with an empty layout until authored content is attached.
    pub fn new_docking() -> Rc<Self> {
        Self::new()
    }

    /// Installs the application callback used for committed user layout changes.
    /// Installs the callback raised once for each committed user layout change.
    pub fn set_on_layout_change(&self, callback: Box<dyn Fn(DockLayoutModel)>) {
        self.set_layout_change_callback(Some(Rc::from(callback)));
    }

    /// Removes the user layout-change callback.
    pub fn clear_on_layout_change(&self) {
        self.set_layout_change_callback(None);
    }

    /// Applies a complete application-authority transformation without emitting a source echo.
    /// Moves an item using an application-authority placement operation.
    pub fn apply_layout_operation(
        &self,
        item: &DockItemId,
        placement: DockPlacement,
    ) -> Result<(), DockLayoutError> {
        let next = self.layout().with_item_moved(item, placement)?;
        self.apply_model(next)
    }

    /// Activates an item and opens its auto-hide overlay when applicable.
    pub fn activate_item(&self, item: &DockItemId) -> Result<(), DockLayoutError> {
        let next = self.layout().with_item_activated(item)?;
        self.apply_model(next)
    }

    /// Closes an item through an application-authority operation.
    pub fn close_item(&self, item: &DockItemId) -> Result<(), DockLayoutError> {
        let next = self.layout().with_item_closed(item)?;
        self.apply_model(next)
    }

    /// Reopens a closed item at its recorded return position.
    pub fn reopen_item(&self, item: &DockItemId) -> Result<(), DockLayoutError> {
        let next = self.layout().with_item_reopened(item)?;
        self.apply_model(next)
    }

    /// Restores the current authored declaration as the default layout.
    pub fn reset_layout(&self) -> Result<(), DockLayoutError> {
        let next = self.layout().with_reset()?;
        self.apply_model(next)
    }

    /// Applies a value supplied by the TwoWay source with latest-only reentrancy protection.
    /// Applies a source-side TwoWay value with equal-value suppression and latest-only queuing.
    pub fn apply_layout_source(&self, model: DockLayoutModel) -> Result<(), DockLayoutError> {
        let Some(realization) = self.runtime_realization() else {
            if model != self.layout() {
                self.set_layout(model);
            }
            return Ok(());
        };
        let mut next = {
            let mut realization = realization.borrow_mut();
            realization.request_source_model(&self.layout(), model)
        };
        while let Some(candidate) = next {
            {
                let mut realization = realization.borrow_mut();
                realization.reconcile(&candidate)?;
            }
            self.set_layout(candidate);
            next = realization.borrow_mut().finish_source_model();
        }
        Ok(())
    }

    /// Starts a capability-checked interactive drag for an authored item.
    pub fn begin_drag_item(&self, item: &DockItemId) -> Result<(), DockLayoutError> {
        let realization =
            self.runtime_realization()
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "drag requested before the docking surface is mounted".to_owned(),
                })?;
        realization
            .borrow_mut()
            .begin_drag(&self.layout(), item.clone())
    }

    /// Calculates and realizes a transient drag preview.
    pub fn preview_drag_item(
        &self,
        target: DockTarget,
        group: Option<DockGroupId>,
        weight: f32,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let realization =
            self.runtime_realization()
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "drag requested before the docking surface is mounted".to_owned(),
                })?;
        let mut realization = realization.borrow_mut();
        let preview = realization.preview_drag(target, group, weight)?;
        realization.reconcile(&preview)?;
        Ok(preview)
    }

    /// Completes or cancels the current drag transaction.
    pub fn finish_drag_item(&self, commit: bool) -> Result<(), DockLayoutError> {
        let realization =
            self.runtime_realization()
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "drag requested before the docking surface is mounted".to_owned(),
                })?;
        let next = realization
            .borrow_mut()
            .finish_drag(commit)
            .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                reason: "drag completed without an active transaction".to_owned(),
            })?;
        if commit {
            self.commit_user_model(next)
        } else {
            self.apply_model(next)
        }
    }

    /// Closes an item after checking its authored close capability and notifying the source.
    pub fn request_close_item(&self, item: &DockItemId) -> Result<(), DockLayoutError> {
        let next = if let Some(realization) = self.runtime_realization() {
            realization
                .borrow_mut()
                .request_close(&self.layout(), item)?
        } else {
            self.layout().with_item_closed(item)?
        };
        self.commit_user_model(next)
    }

    /// Pins an item into an auto-hide strip after checking its authored capability.
    pub fn request_pin_item(
        &self,
        item: &DockItemId,
        side: DockSide,
    ) -> Result<(), DockLayoutError> {
        let next = if let Some(realization) = self.runtime_realization() {
            realization
                .borrow_mut()
                .request_pin(&self.layout(), item, side)?
        } else {
            self.layout()
                .with_item_moved(item, DockPlacement::AutoHide { side })?
        };
        self.commit_user_model(next)
    }

    /// Floats an item at logical desktop bounds after checking its authored capability.
    pub fn request_float_item(
        &self,
        item: &DockItemId,
        bounds: Rect,
    ) -> Result<(), DockLayoutError> {
        let next = if let Some(realization) = self.runtime_realization() {
            realization
                .borrow_mut()
                .request_float(&self.layout(), item, bounds)?
        } else {
            self.layout()
                .with_item_moved(item, DockPlacement::Floating { bounds })?
        };
        self.commit_user_model(next)
    }

    fn capture_authored_default(&self) {
        let Some(content) = self.authored_content() else {
            return;
        };
        let mut seen_items = HashSet::new();
        let mut seen_groups = HashSet::new();
        let root = authored_node(content.as_ref(), &mut seen_items, &mut seen_groups)
            .unwrap_or_else(|| {
                panic!("DockingControl content must be DockGroup or DockSplitPanel")
            });
        let model = self
            .layout()
            .attach_default(DefaultDockDefinition::new(Some(root)));
        let mut realization = crate::runtime::RuntimeRealization::from_authored(content.as_ref())
            .unwrap_or_else(|error| panic!("invalid authored docking declaration: {error}"));
        realization.reconcile(&model).unwrap_or_else(|error| {
            panic!("failed to realize authored docking declaration: {error}")
        });
        self.set_runtime_realization(Some(Rc::new(RefCell::new(realization))));
        self.set_layout(model);
    }

    pub(crate) fn authored_content(&self) -> Option<Rc<dyn UIElementExt>> {
        self.__content_opt()
    }

    fn apply_model(&self, model: DockLayoutModel) -> Result<(), DockLayoutError> {
        if let Some(realization) = self.runtime_realization() {
            realization.borrow_mut().reconcile(&model)?;
        }
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
        let mut seen_items = HashSet::new();
        let mut seen_groups = HashSet::new();
        let root = authored_node(content.as_ref(), &mut seen_items, &mut seen_groups)
            .unwrap_or_else(|| {
                panic!("DockingControl content must be DockGroup or DockSplitPanel")
            });
        let model = self
            .layout()
            .attach_default(DefaultDockDefinition::new(Some(root)));
        let mut realization = realization.borrow_mut();
        realization
            .refresh_authored(content.as_ref())
            .unwrap_or_else(|error| panic!("invalid authored docking registration: {error}"));
        realization.reconcile(&model).unwrap_or_else(|error| {
            panic!("failed to reconcile authored docking registration: {error}")
        });
        drop(realization);
        if model != self.layout() {
            self.set_layout(model);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn notify_user_layout_change(&self, model: DockLayoutModel) {
        let _ = self.commit_user_model(model);
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
