//! Core-owned semantic accessibility.
//!
//! The types in this module deliberately contain no AppKit, WinUI, or other native handles. A
//! backend projects the immutable snapshot produced here into its platform accessibility API.

use crate::base::{AffineTransform, Point, Rect, Size};
use crate::ui::UIElementExt;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ACCESSIBILITY_ID: AtomicU64 = AtomicU64::new(1);

/// Stable semantic identity. It is intentionally independent from `render_group_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessibilityId(u64);

impl AccessibilityId {
    pub fn new() -> Self {
        Self(NEXT_ACCESSIBILITY_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl Default for AccessibilityId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityRole {
    Button,
    StaticText,
    TextInput,
    SecureTextInput,
    CheckBox,
    RadioButton,
    Switch,
    Slider,
    ComboBox,
    Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityCheckState {
    Off,
    On,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueRange {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessibilityChildBehavior {
    #[default]
    Automatic,
    Ignore,
    Contain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityActionKind {
    Activate,
    Increment,
    Decrement,
    SetValue,
    SetText,
    Focus,
    Expand,
    Collapse,
    Select,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccessibilityAction {
    Activate,
    Increment,
    Decrement,
    SetValue(f64),
    SetText(String),
    Focus,
    Expand,
    Collapse,
    Select,
}

impl AccessibilityAction {
    pub fn kind(&self) -> AccessibilityActionKind {
        match self {
            Self::Activate => AccessibilityActionKind::Activate,
            Self::Increment => AccessibilityActionKind::Increment,
            Self::Decrement => AccessibilityActionKind::Decrement,
            Self::SetValue(_) => AccessibilityActionKind::SetValue,
            Self::SetText(_) => AccessibilityActionKind::SetText,
            Self::Focus => AccessibilityActionKind::Focus,
            Self::Expand => AccessibilityActionKind::Expand,
            Self::Collapse => AccessibilityActionKind::Collapse,
            Self::Select => AccessibilityActionKind::Select,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilityState {
    pub disabled: bool,
    pub focused: bool,
    pub checked: Option<AccessibilityCheckState>,
    pub expanded: Option<bool>,
    pub selected: Option<bool>,
    pub read_only: bool,
    pub value_range: Option<ValueRange>,
}

impl Default for AccessibilityState {
    fn default() -> Self {
        Self {
            disabled: false,
            focused: false,
            checked: None,
            expanded: None,
            selected: None,
            read_only: false,
            value_range: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilitySemantics {
    pub role: AccessibilityRole,
    pub label: Option<String>,
    pub value: Option<String>,
    pub hint: Option<String>,
    pub identifier: Option<String>,
    pub state: AccessibilityState,
    pub child_behavior: AccessibilityChildBehavior,
    pub actions: Vec<AccessibilityActionKind>,
}

impl AccessibilitySemantics {
    pub fn new(role: AccessibilityRole) -> Self {
        Self {
            role,
            label: None,
            value: None,
            hint: None,
            identifier: None,
            state: AccessibilityState::default(),
            child_behavior: AccessibilityChildBehavior::Automatic,
            actions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilitySnapshotNode {
    pub id: AccessibilityId,
    pub semantics: AccessibilitySemantics,
    pub bounds_in_root: Rect,
    pub children: Vec<AccessibilitySnapshotNode>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessibilitySnapshot {
    pub roots: Vec<AccessibilitySnapshotNode>,
}

/// Backend-owned route for coalesced semantic invalidation.
pub trait AccessibilityHost {
    fn request_accessibility_update(&self);
}

/// Per-host Core accessibility state. The snapshot and owner index are replaced as one logical
/// operation; action dispatch copies the weak owner before invoking application code, so callbacks
/// may mutate or remove the target without holding a runtime borrow.
pub struct AccessibilityRuntime {
    snapshot: RefCell<AccessibilitySnapshot>,
    owners: RefCell<HashMap<AccessibilityId, Weak<dyn UIElementExt>>>,
    root_id: Cell<Option<AccessibilityId>>,
    revision: Cell<u64>,
}

impl AccessibilityRuntime {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            snapshot: RefCell::new(AccessibilitySnapshot::default()),
            owners: RefCell::new(HashMap::new()),
            root_id: Cell::new(None),
            revision: Cell::new(0),
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub fn snapshot(&self) -> AccessibilitySnapshot {
        self.snapshot.borrow().clone()
    }

    pub fn clear(&self) {
        let changed = !self.snapshot.borrow().roots.is_empty() || !self.owners.borrow().is_empty();
        self.snapshot.borrow_mut().roots.clear();
        self.owners.borrow_mut().clear();
        self.root_id.set(None);
        if changed {
            self.revision.set(self.revision.get().wrapping_add(1));
        }
    }

    pub fn rebuild(&self, root: &Rc<dyn UIElementExt>) -> AccessibilitySnapshot {
        let mut seen = HashSet::new();
        let mut owners = HashMap::new();
        let roots = semantic_children(root, &mut seen, &mut owners);
        let snapshot = AccessibilitySnapshot { roots };
        let changed = *self.snapshot.borrow() != snapshot;
        *self.snapshot.borrow_mut() = snapshot.clone();
        *self.owners.borrow_mut() = owners;
        self.root_id.set(Some(root.accessibility_id()));
        if changed {
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        snapshot
    }

    pub fn dispatch_action(&self, id: AccessibilityId, action: AccessibilityAction) -> bool {
        let owner = self.owners.borrow().get(&id).and_then(Weak::upgrade);
        let Some(owner) = owner else { return false };

        // Revalidate against current participation. A stale platform object must not activate a
        // node that has become hidden, collapsed, inactive, or exiting since the last rebuild.
        if !is_accessible_participant(&owner) {
            return false;
        }
        if !self
            .root_id
            .get()
            .is_some_and(|root_id| belongs_to_root(&owner, root_id))
        {
            return false;
        }
        let Some(semantics) = owner.accessibility_semantics() else {
            return false;
        };
        // A built-in may only be reached through an action it advertises at the platform adapter,
        // while an ordinary `on_accessibility_action` callback is intentionally open-ended: the
        // callback receives the action value itself and is the Core-owned extension point for
        // explicit semantic elements. Disabled nodes remain unsupported in both paths.
        if semantics.state.disabled {
            return false;
        }
        if owner.perform_accessibility_action(action.clone()) {
            return true;
        }
        owner.invoke_accessibility_action(action)
    }
}

fn is_accessible_participant(node: &Rc<dyn UIElementExt>) -> bool {
    node.visibility() == crate::layout::Visibility::Visible
        && node.visual_participation() == crate::ui::VisualParticipation::Active
        && !node.accessibility_hidden()
}

fn belongs_to_root(node: &Rc<dyn UIElementExt>, root_id: AccessibilityId) -> bool {
    let mut current = Some(Rc::clone(node));
    while let Some(item) = current {
        if item.accessibility_id() == root_id {
            return true;
        }
        current = item.visual_parent();
    }
    false
}

fn semantic_children(
    node: &Rc<dyn UIElementExt>,
    seen: &mut HashSet<AccessibilityId>,
    owners: &mut HashMap<AccessibilityId, Weak<dyn UIElementExt>>,
) -> Vec<AccessibilitySnapshotNode> {
    if !is_accessible_participant(node) || !seen.insert(node.accessibility_id()) {
        return Vec::new();
    }

    let semantics = node.accessibility_semantics();
    // A transparent node has no public semantics from which Ignore or Contain could be
    // resolved. Its effective policy is therefore Automatic, which is important for a templated
    // Control: the private template boundary must still be respected while projected content is
    // hoisted through the transparent owner.
    let effective_child_behavior = semantics
        .as_ref()
        .map(|value| value.child_behavior)
        .unwrap_or(AccessibilityChildBehavior::Automatic);
    let child_nodes = match effective_child_behavior {
        AccessibilityChildBehavior::Ignore => Vec::new(),
        AccessibilityChildBehavior::Automatic => node
            .__accessibility_template_children()
            .unwrap_or_else(|| node.visual_children()),
        AccessibilityChildBehavior::Contain => node.visual_children(),
    };
    let semantic_children = child_nodes
        .iter()
        .flat_map(|child| semantic_children(child, seen, owners))
        .collect::<Vec<_>>();

    let Some(semantics) = semantics else {
        return semantic_children;
    };

    let children = semantic_children;
    let id = node.accessibility_id();
    owners.insert(id, Rc::downgrade(node));
    vec![AccessibilitySnapshotNode {
        id,
        semantics,
        bounds_in_root: bounds_in_root(node),
        children,
    }]
}

/// Finds logical/projected content inside a Control's private template without depending on the
/// concrete template primitive (`ContentPresenter`, a layout, or a user component). The template
/// root and its presentation chrome have no logical edge back to the Control; projected content
/// retains the owning Control's logical parent edge. Returning that first edge preserves public
/// content while suppressing private template descendants.
pub(crate) fn template_accessibility_children(
    owner: &Rc<dyn UIElementExt>,
    template_root: &Rc<dyn UIElementExt>,
) -> Vec<Rc<dyn UIElementExt>> {
    let owner_id = owner.accessibility_id();
    let mut children = Vec::new();
    let mut seen = HashSet::new();
    collect_template_accessibility_children(template_root, owner_id, &mut seen, &mut children);
    children
}

fn collect_template_accessibility_children(
    node: &Rc<dyn UIElementExt>,
    owner_id: AccessibilityId,
    seen: &mut HashSet<AccessibilityId>,
    children: &mut Vec<Rc<dyn UIElementExt>>,
) {
    let mut logical_parent = node.parent();
    while let Some(parent) = logical_parent {
        if parent.accessibility_id() == owner_id {
            if seen.insert(node.accessibility_id()) {
                children.push(Rc::clone(node));
            }
            return;
        }
        logical_parent = parent.parent();
    }

    for child in node.visual_children() {
        collect_template_accessibility_children(&child, owner_id, seen, children);
    }
}

fn bounds_in_root(node: &Rc<dyn UIElementExt>) -> Rect {
    let mut chain = Vec::new();
    let mut current = Some(Rc::clone(node));
    while let Some(item) = current {
        current = item.visual_parent();
        chain.push(item);
    }
    chain.reverse();

    let mut transform = AffineTransform::IDENTITY;
    let mut size = Size::default();
    for item in chain {
        size = Size {
            width: item.arranged_width().unwrap_or(0.0),
            height: item.arranged_height().unwrap_or(0.0),
        };
        let offset = item.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let layout = AffineTransform::translation(offset.x, offset.y);
        let local = crate::ui::local_transform(
            item.presentation_visual_transform(),
            item.transform_origin(),
            size,
        );
        transform = transform.concat(&layout.concat(&local));
    }

    let corners = [
        Point { x: 0.0, y: 0.0 },
        Point {
            x: size.width,
            y: 0.0,
        },
        Point {
            x: 0.0,
            y: size.height,
        },
        Point {
            x: size.width,
            y: size.height,
        },
    ];
    let first = transform.transform_point(corners[0]);
    corners[1..].iter().fold(
        Rect {
            x: first.x,
            y: first.y,
            width: 0.0,
            height: 0.0,
        },
        |bounds, corner| {
            let point = transform.transform_point(*corner);
            bounds.union(Rect {
                x: point.x,
                y: point.y,
                width: 0.0,
                height: 0.0,
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Orientation, Visibility};
    use crate::ui::testsupport::{native, stack};
    use crate::ui::{ContentControl, ContentControlExt, ContentPresenter, ControlExt, TextBlock};

    fn button(label: &str) -> Rc<dyn UIElementExt> {
        let node = native(
            "accessibility-test",
            Size {
                width: 40.0,
                height: 20.0,
            },
        );
        node.set_accessibility_role(AccessibilityRole::Button);
        node.set_accessibility_label(label);
        node
    }

    #[test]
    fn semantic_id_survives_snapshot_rebuild_and_mutation() {
        let node = button("before");
        let runtime = AccessibilityRuntime::new();
        let first = runtime.rebuild(&node);
        let id = first.roots[0].id;

        node.set_accessibility_label("after");
        node.set_accessibility_value("ready");
        let second = runtime.rebuild(&node);

        assert_eq!(id, second.roots[0].id);
        assert_eq!(second.roots[0].semantics.label.as_deref(), Some("after"));
        assert_eq!(second.roots[0].semantics.value.as_deref(), Some("ready"));
    }

    #[test]
    fn transparent_presentation_nodes_hoist_semantic_descendants() {
        let child = button("child");
        let middle = stack(Orientation::Vertical, 0.0, vec![child.clone()]);
        let root = stack(Orientation::Vertical, 0.0, vec![middle]);
        let snapshot = AccessibilityRuntime::new().rebuild(&root);

        assert_eq!(snapshot.roots.len(), 1);
        assert_eq!(snapshot.roots[0].id, child.accessibility_id());
    }

    #[test]
    fn explicit_canvas_semantics_and_child_behavior_are_honored() {
        let parent = button("parent");
        let child = button("child");
        parent.set_accessibility_children(AccessibilityChildBehavior::Ignore);
        parent.as_ui_element().visual_collection.add(child.clone());
        let runtime = AccessibilityRuntime::new();
        let ignored = runtime.rebuild(&parent);
        assert_eq!(ignored.roots[0].children.len(), 0);

        parent.set_accessibility_children(AccessibilityChildBehavior::Contain);
        let contained = runtime.rebuild(&parent);
        assert_eq!(contained.roots[0].children.len(), 1);
        assert_eq!(contained.roots[0].children[0].id, child.accessibility_id());
    }

    #[test]
    fn hidden_and_exiting_subtrees_are_removed_immediately() {
        let child = button("hidden");
        let root = stack(Orientation::Vertical, 0.0, vec![child.clone()]);
        let runtime = AccessibilityRuntime::new();
        assert_eq!(runtime.rebuild(&root).roots.len(), 1);

        child.set_accessibility_hidden(true);
        assert!(runtime.rebuild(&root).roots.is_empty());
        child.set_accessibility_hidden(false);
        assert_eq!(runtime.rebuild(&root).roots.len(), 1);

        child.set_visibility(Visibility::Collapsed);
        assert!(runtime.rebuild(&root).roots.is_empty());
    }

    #[test]
    fn reentrant_action_can_remove_its_owner_without_borrow_panic() {
        let child = button("remove me");
        let root = stack(Orientation::Vertical, 0.0, vec![child.clone()]);
        let weak_child = Rc::downgrade(&child);
        let root_for_callback = root.clone();
        child.set_on_accessibility_action(Box::new(move |_| {
            if let Some(child) = weak_child.upgrade() {
                root_for_callback
                    .as_ui_element()
                    .visual_collection
                    .remove(&child);
            }
        }));
        let runtime = AccessibilityRuntime::new();
        let id = runtime.rebuild(&root).roots[0].id;

        assert!(runtime.dispatch_action(id, AccessibilityAction::Activate));
        assert!(runtime.rebuild(&root).roots.is_empty());
        assert!(!runtime.dispatch_action(id, AccessibilityAction::Activate));
    }

    #[test]
    fn secure_text_semantics_do_not_store_or_debug_plaintext() {
        let node = native(
            "secure",
            Size {
                width: 80.0,
                height: 20.0,
            },
        );
        node.set_accessibility_role(AccessibilityRole::SecureTextInput);
        node.set_accessibility_label("Password");
        let snapshot = AccessibilityRuntime::new().rebuild(&node);
        let debug = format!("{:?}", node.as_ui_element());

        assert!(snapshot.roots[0].semantics.value.is_none());
        assert!(!debug.contains("hunter2"));
    }

    #[test]
    fn automatic_template_traversal_hides_private_chrome_but_keeps_projected_content() {
        let control = ContentControl::new();
        control.set_accessibility_role(AccessibilityRole::Button);
        control.set_accessibility_label("Save");

        let private_text = TextBlock::new();
        private_text.set_accessibility_role(AccessibilityRole::StaticText);
        private_text.set_accessibility_label("Save");
        let private_chrome = stack(Orientation::Vertical, 0.0, vec![private_text]);

        let content = TextBlock::new();
        content.set_accessibility_role(AccessibilityRole::StaticText);
        content.set_accessibility_label("public content");
        control.set_content(content.clone());
        control.__enable_template_presentation();
        let presenter = ContentPresenter::new();
        ContentPresenter::__bind_templated_parent(&presenter, &control);
        let template_root = stack(
            Orientation::Vertical,
            0.0,
            vec![private_chrome, presenter as Rc<dyn UIElementExt>],
        );
        control.__set_template_root(template_root);

        let control_node: Rc<dyn UIElementExt> = control;
        let snapshot = AccessibilityRuntime::new().rebuild(&control_node);

        assert_eq!(snapshot.roots.len(), 1);
        assert_eq!(snapshot.roots[0].semantics.role, AccessibilityRole::Button);
        assert_eq!(snapshot.roots[0].children.len(), 1);
        assert_eq!(snapshot.roots[0].children[0].id, content.accessibility_id());
        assert_eq!(
            snapshot.roots[0].children[0].semantics.label.as_deref(),
            Some("public content")
        );
    }

    #[test]
    fn transparent_templated_control_hides_private_chrome_and_hoists_content() {
        let control = ContentControl::new();

        let private_text = TextBlock::new();
        private_text.set_accessibility_role(AccessibilityRole::StaticText);
        private_text.set_accessibility_label("decorative");
        let private_chrome = stack(Orientation::Vertical, 0.0, vec![private_text]);

        let content = button("Save");
        control.set_content(content.clone());
        control.__enable_template_presentation();
        let presenter = ContentPresenter::new();
        ContentPresenter::__bind_templated_parent(&presenter, &control);
        let template_root = stack(
            Orientation::Vertical,
            0.0,
            vec![private_chrome, presenter as Rc<dyn UIElementExt>],
        );
        control.__set_template_root(template_root);

        let owner: Rc<dyn UIElementExt> = control;
        let snapshot = AccessibilityRuntime::new().rebuild(&owner);

        assert_eq!(snapshot.roots.len(), 1);
        assert_eq!(snapshot.roots[0].id, content.accessibility_id());
        assert_eq!(snapshot.roots[0].semantics.role, AccessibilityRole::Button);
        assert_eq!(snapshot.roots[0].semantics.label.as_deref(), Some("Save"));
    }
}
