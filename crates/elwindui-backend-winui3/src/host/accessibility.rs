//! WinUI 3 projection state for the Core-owned accessibility snapshot.
//!
//! The XAML peer bridge is deliberately a projection only. It receives stable integer IDs and
//! copied snapshot values from this state; semantic traversal, policy, and action dispatch stay in
//! `elwindui-core`.

use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityCheckState, AccessibilityHost,
    AccessibilityId, AccessibilityRole, AccessibilityRuntime, AccessibilitySnapshot,
    AccessibilitySnapshotNode,
};
use elwindui_core::ui::UIElementExt;
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::{Rc, Weak};

#[cfg(windows)]
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
#[cfg(windows)]
use elwindui_core::base::{Point, Rect};
#[cfg(windows)]
use windows::core::{IInspectable, Interface};

#[cfg(windows)]
#[repr(C)]
struct AccessibilityNodeRecord {
    id: u64,
    role: u32,
    state_flags: u32,
    actions_mask: u32,
    patterns_mask: u32,
    value: f64,
    minimum: f64,
    maximum: f64,
    step: f64,
    has_range: u8,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    label_length: u32,
    label: [u16; 256],
    value_length: u32,
    value_text: [u16; 256],
    identifier_length: u32,
    identifier: [u16; 256],
}

#[cfg(windows)]
#[repr(C)]
struct AccessibilityCallbacks {
    context: *mut c_void,
    revision: extern "C" fn(*mut c_void) -> u64,
    child_count: extern "C" fn(*mut c_void, u64) -> u32,
    child_id: extern "C" fn(*mut c_void, u64, u32) -> u64,
    get_node: extern "C" fn(*mut c_void, u64, *mut AccessibilityNodeRecord) -> u32,
    dispatch_action: extern "C" fn(*mut c_void, u64, u32, f64, *const u16, u32) -> u32,
}

#[cfg(windows)]
unsafe extern "C" {
    fn elwindui_winui3_accessibility_canvas_create() -> *mut c_void;
    fn elwindui_winui3_accessibility_canvas_set_callbacks(
        bridge_key: *mut c_void,
        callbacks: *const AccessibilityCallbacks,
    ) -> u32;
    fn elwindui_winui3_accessibility_canvas_refresh(
        bridge_key: *mut c_void,
        structure_changed: u32,
    );
    fn elwindui_winui3_accessibility_canvas_detach(bridge_key: *mut c_void);
}

#[cfg(windows)]
struct CppAccessibilityBridge {
    bridge_key: std::cell::Cell<*mut c_void>,
}

#[cfg(windows)]
impl CppAccessibilityBridge {
    fn new() -> Self {
        Self {
            bridge_key: std::cell::Cell::new(std::ptr::null_mut()),
        }
    }
}

#[cfg(windows)]
impl Drop for CppAccessibilityBridge {
    fn drop(&mut self) {
        let bridge_key = self.bridge_key.replace(std::ptr::null_mut());
        if !bridge_key.is_null() {
            // The C++ side only erases a map entry here; it does not dereference the released
            // XAML object. This remains safe even when the final Canvas clone is dropped first.
            unsafe { elwindui_winui3_accessibility_canvas_detach(bridge_key) };
        }
    }
}

pub(crate) struct WinUI3AccessibilityState {
    pub(crate) runtime: Rc<AccessibilityRuntime>,
    tree: Weak<RefCell<Option<Rc<dyn UIElementExt>>>>,
    #[cfg(windows)]
    canvas: RefCell<Option<windows::core::Weak<Canvas>>>,
    #[cfg(windows)]
    cpp_bridge: CppAccessibilityBridge,
}

#[cfg(windows)]
fn canonical_bridge_key(canvas: &Canvas) -> Option<*mut c_void> {
    let inspectable: IInspectable = canvas.cast().ok()?;
    Some(Interface::as_raw(&inspectable) as *mut c_void)
}

impl WinUI3AccessibilityState {
    pub(crate) fn new(tree: Weak<RefCell<Option<Rc<dyn UIElementExt>>>>) -> Rc<Self> {
        Rc::new(Self {
            runtime: AccessibilityRuntime::new(),
            tree,
            #[cfg(windows)]
            canvas: RefCell::new(None),
            #[cfg(windows)]
            cpp_bridge: CppAccessibilityBridge::new(),
        })
    }

    #[cfg(windows)]
    pub(crate) fn bind_canvas(self: &Rc<Self>, canvas: &Canvas) -> bool {
        *self.canvas.borrow_mut() = canvas.downgrade().ok();
        self.cpp_bridge.bridge_key.set(std::ptr::null_mut());
        let Some(bridge_key) = canonical_bridge_key(canvas) else {
            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                eprintln!("[elwindui-winui3] accessibility Canvas -> IInspectable cast failed");
            }
            return false;
        };
        self.cpp_bridge.bridge_key.set(bridge_key);
        let callbacks = AccessibilityCallbacks {
            context: Rc::as_ptr(self) as *mut c_void,
            revision: callback_revision,
            child_count: callback_child_count,
            child_id: callback_child_id,
            get_node: callback_get_node,
            dispatch_action: callback_dispatch_action,
        };
        let result =
            unsafe { elwindui_winui3_accessibility_canvas_set_callbacks(bridge_key, &callbacks) };
        let bound = result == 1;
        if !bound && std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            eprintln!("[elwindui-winui3] accessibility callback binding failed");
        }
        bound
    }

    pub(crate) fn rebuild(&self) {
        let old_revision = self.runtime.revision();
        let old_snapshot = self.runtime.snapshot();
        let tree: Option<Rc<RefCell<Option<Rc<dyn UIElementExt>>>>> = self.tree.upgrade();
        let Some(tree) = tree.and_then(|tree| tree.borrow().clone()) else {
            self.runtime.clear();
            self.refresh_after_update(old_revision, old_snapshot);
            return;
        };
        self.runtime.rebuild(&tree);
        self.refresh_after_update(old_revision, old_snapshot);
    }

    pub(crate) fn clear(&self) {
        let old_revision = self.runtime.revision();
        let old_snapshot = self.runtime.snapshot();
        self.runtime.clear();
        self.refresh_after_update(old_revision, old_snapshot);
    }

    fn refresh_after_update(&self, old_revision: u64, old_snapshot: AccessibilitySnapshot) {
        let new_snapshot = self.runtime.snapshot();
        let Some(structure_changed) = refresh_decision(
            old_revision,
            self.runtime.revision(),
            &old_snapshot,
            &new_snapshot,
        ) else {
            return;
        };

        #[cfg(windows)]
        self.refresh_tree(structure_changed);
        #[cfg(not(windows))]
        let _ = structure_changed;
    }

    #[cfg(windows)]
    fn refresh_tree(&self, structure_changed: bool) {
        let bridge_key = self.cpp_bridge.bridge_key.get();
        if !bridge_key.is_null() {
            unsafe {
                elwindui_winui3_accessibility_canvas_refresh(
                    bridge_key,
                    u32::from(structure_changed),
                )
            };
        }
    }
}

fn same_semantic_structure(old: &AccessibilitySnapshot, new: &AccessibilitySnapshot) -> bool {
    fn same_nodes(old: &[AccessibilitySnapshotNode], new: &[AccessibilitySnapshotNode]) -> bool {
        old.len() == new.len()
            && old
                .iter()
                .zip(new)
                .all(|(old, new)| old.id == new.id && same_nodes(&old.children, &new.children))
    }

    same_nodes(&old.roots, &new.roots)
}

fn refresh_decision(
    old_revision: u64,
    new_revision: u64,
    old_snapshot: &AccessibilitySnapshot,
    new_snapshot: &AccessibilitySnapshot,
) -> Option<bool> {
    (old_revision != new_revision).then(|| !same_semantic_structure(old_snapshot, new_snapshot))
}

/// Weak host capability installed on the Core root. A Core property mutation can therefore
/// refresh the immutable snapshot without retaining the XAML host or creating a Rust reference
/// cycle.
pub(crate) struct WinUI3AccessibilityHost {
    state: Weak<WinUI3AccessibilityState>,
}

impl WinUI3AccessibilityHost {
    pub(crate) fn new(state: Weak<WinUI3AccessibilityState>) -> Self {
        Self { state }
    }
}

impl AccessibilityHost for WinUI3AccessibilityHost {
    fn request_accessibility_update(&self) {
        let state: Option<Rc<WinUI3AccessibilityState>> = self.state.upgrade();
        if let Some(state) = state {
            state.rebuild();
        }
    }
}

#[cfg(windows)]
fn state_from_context<'a>(context: *mut c_void) -> Option<&'a WinUI3AccessibilityState> {
    if context.is_null() {
        None
    } else {
        // SAFETY: the C++ peer cache is detached before the Rc owning this state is released.
        Some(unsafe { &*(context as *const WinUI3AccessibilityState) })
    }
}

#[cfg(windows)]
fn snapshot_node(
    state: &WinUI3AccessibilityState,
    id: u64,
) -> Option<elwindui_core::accessibility::AccessibilitySnapshotNode> {
    fn find(
        nodes: &[elwindui_core::accessibility::AccessibilitySnapshotNode],
        id: AccessibilityId,
    ) -> Option<elwindui_core::accessibility::AccessibilitySnapshotNode> {
        for node in nodes {
            if node.id == id {
                return Some(node.clone());
            }
            if let Some(found) = find(&node.children, id) {
                return Some(found);
            }
        }
        None
    }
    find(
        &state.runtime.snapshot().roots,
        AccessibilityId::from_raw(id),
    )
}

#[cfg(windows)]
fn children_for(state: &WinUI3AccessibilityState, parent: u64) -> Vec<AccessibilityId> {
    fn find(
        nodes: &[elwindui_core::accessibility::AccessibilitySnapshotNode],
        id: AccessibilityId,
    ) -> Option<Vec<AccessibilityId>> {
        for node in nodes {
            if node.id == id {
                return Some(node.children.iter().map(|child| child.id).collect());
            }
            if let Some(found) = find(&node.children, id) {
                return Some(found);
            }
        }
        None
    }
    let snapshot = state.runtime.snapshot();
    if parent == 0 {
        snapshot.roots.iter().map(|node| node.id).collect()
    } else {
        find(&snapshot.roots, AccessibilityId::from_raw(parent)).unwrap_or_default()
    }
}

#[cfg(windows)]
fn role_code(role: AccessibilityRole) -> u32 {
    match role {
        AccessibilityRole::Button => 1,
        AccessibilityRole::StaticText => 2,
        AccessibilityRole::TextInput => 3,
        AccessibilityRole::SecureTextInput => 4,
        AccessibilityRole::CheckBox => 5,
        AccessibilityRole::RadioButton => 6,
        AccessibilityRole::Switch => 7,
        AccessibilityRole::Slider => 8,
        AccessibilityRole::ComboBox => 9,
        AccessibilityRole::Group => 10,
    }
}

#[cfg(windows)]
fn copy_utf16(value: Option<&str>, destination: &mut [u16; 256]) -> u32 {
    let Some(value) = value else { return 0 };
    let count = value.encode_utf16().take(destination.len()).count();
    for (slot, code_unit) in destination.iter_mut().zip(value.encode_utf16()).take(count) {
        *slot = code_unit;
    }
    count as u32
}

#[cfg(windows)]
const PATTERN_INVOKE: u32 = 1 << 0;
#[cfg(windows)]
const PATTERN_TOGGLE: u32 = 1 << 1;
#[cfg(windows)]
const PATTERN_RANGE_VALUE: u32 = 1 << 2;
#[cfg(windows)]
const PATTERN_VALUE: u32 = 1 << 3;
#[cfg(windows)]
const PATTERN_SELECTION_ITEM: u32 = 1 << 4;
#[cfg(windows)]
const PATTERN_EXPAND_COLLAPSE: u32 = 1 << 5;

#[cfg(windows)]
fn action_is_present(actions_mask: u32, action: AccessibilityActionKind) -> bool {
    actions_mask & (1u32 << action_kind_code(action)) != 0
}

#[cfg(windows)]
fn patterns_mask(
    role: AccessibilityRole,
    checked: Option<AccessibilityCheckState>,
    expanded: Option<bool>,
    selected: Option<bool>,
    value_range: bool,
    actions_mask: u32,
) -> u32 {
    let mut mask = 0;
    if role == AccessibilityRole::Button
        && action_is_present(actions_mask, AccessibilityActionKind::Activate)
    {
        mask |= PATTERN_INVOKE;
    }
    if matches!(
        role,
        AccessibilityRole::CheckBox | AccessibilityRole::Switch
    ) && checked.is_some()
        && action_is_present(actions_mask, AccessibilityActionKind::Activate)
    {
        mask |= PATTERN_TOGGLE;
    }
    if role == AccessibilityRole::Slider
        && value_range
        && action_is_present(actions_mask, AccessibilityActionKind::SetValue)
    {
        mask |= PATTERN_RANGE_VALUE;
    }
    if role == AccessibilityRole::TextInput
        && action_is_present(actions_mask, AccessibilityActionKind::SetText)
    {
        mask |= PATTERN_VALUE;
    }
    if role == AccessibilityRole::RadioButton
        && selected.is_some()
        && action_is_present(actions_mask, AccessibilityActionKind::Select)
    {
        mask |= PATTERN_SELECTION_ITEM;
    }
    if expanded.is_some()
        && (action_is_present(actions_mask, AccessibilityActionKind::Expand)
            || action_is_present(actions_mask, AccessibilityActionKind::Collapse))
    {
        mask |= PATTERN_EXPAND_COLLAPSE;
    }
    mask
}

#[cfg(windows)]
fn fill_record(
    record: &mut AccessibilityNodeRecord,
    node: &elwindui_core::accessibility::AccessibilitySnapshotNode,
) {
    const STATE_DISABLED: u32 = 1 << 0;
    const STATE_FOCUSED: u32 = 1 << 1;
    const STATE_CHECKED_ON: u32 = 1 << 2;
    const STATE_EXPANDED: u32 = 1 << 3;
    const STATE_SELECTED: u32 = 1 << 4;
    const STATE_READ_ONLY: u32 = 1 << 5;
    const STATE_CHECKED_MIXED: u32 = 1 << 6;

    record.id = node.id.raw();
    record.role = role_code(node.semantics.role);
    record.state_flags = if node.semantics.state.disabled {
        STATE_DISABLED
    } else {
        0
    } | if node.semantics.state.focused {
        STATE_FOCUSED
    } else {
        0
    } | match node.semantics.state.checked {
        Some(AccessibilityCheckState::On) => STATE_CHECKED_ON,
        Some(AccessibilityCheckState::Mixed) => STATE_CHECKED_MIXED,
        Some(AccessibilityCheckState::Off) | None => 0,
    } | if node.semantics.state.expanded == Some(true) {
        STATE_EXPANDED
    } else {
        0
    } | if node.semantics.state.selected == Some(true) {
        STATE_SELECTED
    } else {
        0
    } | if node.semantics.state.read_only {
        STATE_READ_ONLY
    } else {
        0
    };
    record.actions_mask = node.semantics.actions.iter().fold(0u32, |mask, action| {
        mask | (1u32 << action_kind_code(*action))
    });
    record.patterns_mask = patterns_mask(
        node.semantics.role,
        node.semantics.state.checked,
        node.semantics.state.expanded,
        node.semantics.state.selected,
        node.semantics.state.value_range.is_some(),
        record.actions_mask,
    );
    record.value = 0.0;
    record.minimum = 0.0;
    record.maximum = 0.0;
    record.step = f64::NAN;
    record.has_range = 0;
    if let Some(range) = node.semantics.state.value_range {
        record.value = range.value;
        record.minimum = range.min;
        record.maximum = range.max;
        record.step = range.step.unwrap_or(f64::NAN);
        record.has_range = 1;
    }
    record.x = node.bounds_in_root.x;
    record.y = node.bounds_in_root.y;
    record.width = node.bounds_in_root.width;
    record.height = node.bounds_in_root.height;
    record.label_length = copy_utf16(node.semantics.label.as_deref(), &mut record.label);
    // Password semantics never cross the Value/UIA projection boundary, even though the Core
    // snapshot may carry an internal value for the native PasswordBox implementation.
    record.value_length = if node.semantics.role == AccessibilityRole::SecureTextInput {
        0
    } else {
        copy_utf16(node.semantics.value.as_deref(), &mut record.value_text)
    };
    record.identifier_length =
        copy_utf16(node.semantics.identifier.as_deref(), &mut record.identifier);
}

#[cfg(windows)]
fn physical_bounds_from_points(top_left: Point, bottom_right: Point) -> Rect {
    Rect {
        x: top_left.x.min(bottom_right.x),
        y: top_left.y.min(bottom_right.y),
        width: (bottom_right.x - top_left.x).abs(),
        height: (bottom_right.y - top_left.y).abs(),
    }
}

#[cfg(windows)]
fn physical_bounds_for(
    state: &WinUI3AccessibilityState,
    node: &elwindui_core::accessibility::AccessibilitySnapshotNode,
) -> Option<Rect> {
    let canvas = {
        let weak = state.canvas.borrow();
        weak.as_ref()?.upgrade()?
    };
    let top_left = super::TreeHost::canvas_to_screen_physical_point(
        &canvas,
        Point {
            x: node.bounds_in_root.x,
            y: node.bounds_in_root.y,
        },
    )?;
    let bottom_right = super::TreeHost::canvas_to_screen_physical_point(
        &canvas,
        Point {
            x: node.bounds_in_root.x + node.bounds_in_root.width,
            y: node.bounds_in_root.y + node.bounds_in_root.height,
        },
    )?;
    Some(physical_bounds_from_points(top_left, bottom_right))
}

#[cfg(windows)]
fn physical_canvas_bounds(state: &WinUI3AccessibilityState) -> Option<Rect> {
    let canvas = {
        let weak = state.canvas.borrow();
        weak.as_ref()?.upgrade()?
    };
    let width = canvas.ActualWidth().ok()? as f32;
    let height = canvas.ActualHeight().ok()? as f32;
    let top_left =
        super::TreeHost::canvas_to_screen_physical_point(&canvas, Point { x: 0.0, y: 0.0 })?;
    let bottom_right = super::TreeHost::canvas_to_screen_physical_point(
        &canvas,
        Point {
            x: width,
            y: height,
        },
    )?;
    Some(physical_bounds_from_points(top_left, bottom_right))
}

#[cfg(windows)]
fn action_kind_code(action: AccessibilityActionKind) -> u32 {
    match action {
        AccessibilityActionKind::Activate => 0,
        AccessibilityActionKind::Increment => 1,
        AccessibilityActionKind::Decrement => 2,
        AccessibilityActionKind::SetValue => 3,
        AccessibilityActionKind::SetText => 4,
        AccessibilityActionKind::Focus => 5,
        AccessibilityActionKind::Expand => 6,
        AccessibilityActionKind::Collapse => 7,
        AccessibilityActionKind::Select => 8,
    }
}

#[cfg(windows)]
extern "C" fn callback_revision(context: *mut c_void) -> u64 {
    state_from_context(context).map_or(0, |state| state.runtime.revision())
}

#[cfg(windows)]
extern "C" fn callback_child_count(context: *mut c_void, parent: u64) -> u32 {
    state_from_context(context).map_or(0, |state| children_for(state, parent).len() as u32)
}

#[cfg(windows)]
extern "C" fn callback_child_id(context: *mut c_void, parent: u64, index: u32) -> u64 {
    state_from_context(context)
        .and_then(|state| children_for(state, parent).get(index as usize).copied())
        .map_or(0, AccessibilityId::raw)
}

#[cfg(windows)]
extern "C" fn callback_get_node(
    context: *mut c_void,
    id: u64,
    record: *mut AccessibilityNodeRecord,
) -> u32 {
    let Some(state) = state_from_context(context) else {
        return 0;
    };
    let Some(record) = (unsafe { record.as_mut() }) else {
        return 0;
    };
    if id == 0 {
        let Some(bounds) = physical_canvas_bounds(state) else {
            return 0;
        };
        *record = unsafe { std::mem::zeroed() };
        record.x = bounds.x;
        record.y = bounds.y;
        record.width = bounds.width;
        record.height = bounds.height;
        return 1;
    }
    let Some(node) = snapshot_node(state, id) else {
        return 0;
    };
    *record = unsafe { std::mem::zeroed() };
    fill_record(record, &node);
    if let Some(bounds) = physical_bounds_for(state, &node) {
        record.x = bounds.x;
        record.y = bounds.y;
        record.width = bounds.width;
        record.height = bounds.height;
    } else {
        record.x = 0.0;
        record.y = 0.0;
        record.width = 0.0;
        record.height = 0.0;
    }
    1
}

#[cfg(windows)]
extern "C" fn callback_dispatch_action(
    context: *mut c_void,
    id: u64,
    kind: u32,
    numeric_value: f64,
    text: *const u16,
    text_length: u32,
) -> u32 {
    let Some(state) = state_from_context(context) else {
        return 0;
    };
    let action = match kind {
        0 => AccessibilityAction::Activate,
        1 => AccessibilityAction::Increment,
        2 => AccessibilityAction::Decrement,
        3 => AccessibilityAction::SetValue(numeric_value),
        4 => {
            let text = if text.is_null() {
                String::new()
            } else {
                String::from_utf16_lossy(unsafe {
                    std::slice::from_raw_parts(text, text_length as usize)
                })
            };
            AccessibilityAction::SetText(text)
        }
        5 => AccessibilityAction::Focus,
        6 => AccessibilityAction::Expand,
        7 => AccessibilityAction::Collapse,
        8 => AccessibilityAction::Select,
        _ => return 0,
    };
    u32::from(
        state
            .runtime
            .dispatch_action(AccessibilityId::from_raw(id), action),
    )
}

#[cfg(not(windows))]
pub(crate) fn create_canvas() -> crate::bindings::Microsoft::UI::Xaml::Controls::Canvas {
    crate::bindings::Microsoft::UI::Xaml::Controls::Canvas::new().expect("Canvas::new")
}

#[cfg(windows)]
pub(crate) fn create_canvas() -> Canvas {
    let raw = unsafe { elwindui_winui3_accessibility_canvas_create() };
    assert!(
        !raw.is_null(),
        "elwindui_winui3_accessibility_canvas_create"
    );
    unsafe { windows::core::Type::from_abi(raw) }
        .expect("elwindui_winui3_accessibility_canvas_create returned invalid Canvas")
}

#[cfg(test)]
mod tests {
    use super::*;
    use elwindui_core::accessibility::AccessibilitySemantics;
    use elwindui_core::base::Rect;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn node(id: u64, children: Vec<AccessibilitySnapshotNode>) -> AccessibilitySnapshotNode {
        AccessibilitySnapshotNode {
            id: AccessibilityId::from_raw(id),
            semantics: AccessibilitySemantics::new(AccessibilityRole::StaticText),
            bounds_in_root: rect(0.0, 0.0, 10.0, 10.0),
            children,
        }
    }

    fn snapshot(roots: Vec<AccessibilitySnapshotNode>) -> AccessibilitySnapshot {
        AccessibilitySnapshot { roots }
    }

    #[test]
    fn semantic_structure_ignores_non_topology_changes() {
        let old = snapshot(vec![node(1, vec![node(2, vec![])])]);
        let mut changed = old.clone();
        changed.roots[0].semantics.label = Some("updated label".to_owned());
        changed.roots[0].semantics.value = Some("updated value".to_owned());
        changed.roots[0].semantics.state.focused = true;
        changed.roots[0].bounds_in_root = rect(5.0, 6.0, 20.0, 21.0);
        changed.roots[0]
            .semantics
            .actions
            .push(AccessibilityActionKind::Focus);

        assert!(same_semantic_structure(&old, &old));
        assert!(same_semantic_structure(&old, &changed));
    }

    #[test]
    fn refresh_decision_skips_noop_and_refreshes_effective_non_structural_change() {
        let old = snapshot(vec![node(1, vec![])]);
        let mut changed = old.clone();
        changed.roots[0].semantics.label = Some("updated label".to_owned());
        let mut structural = old.clone();
        structural.roots[0].children.push(node(2, vec![]));

        assert_eq!(refresh_decision(7, 7, &old, &old), None);
        assert_eq!(refresh_decision(7, 8, &old, &old), Some(false));
        assert_eq!(refresh_decision(7, 8, &old, &changed), Some(false));
        assert_eq!(refresh_decision(7, 8, &old, &structural), Some(true));
    }

    #[test]
    fn semantic_structure_detects_child_add_remove_and_reorder() {
        let old = snapshot(vec![node(1, vec![node(2, vec![]), node(3, vec![])])]);

        let mut added = old.clone();
        added.roots[0].children.push(node(4, vec![]));
        assert!(!same_semantic_structure(&old, &added));

        let mut removed = old.clone();
        removed.roots[0].children.remove(1);
        assert!(!same_semantic_structure(&old, &removed));

        let mut reordered = old.clone();
        reordered.roots[0].children.swap(0, 1);
        assert!(!same_semantic_structure(&old, &reordered));
    }

    #[test]
    fn semantic_structure_detects_nested_reparenting() {
        let old = snapshot(vec![node(
            1,
            vec![node(2, vec![node(3, vec![])]), node(4, vec![])],
        )]);
        let new = snapshot(vec![node(
            1,
            vec![node(2, vec![]), node(4, vec![node(3, vec![])])],
        )]);

        assert!(!same_semantic_structure(&old, &new));
    }

    #[test]
    fn semantic_structure_detects_clear_but_not_repeated_empty_state() {
        let non_empty = snapshot(vec![node(1, vec![])]);
        let empty = AccessibilitySnapshot::default();

        assert!(!same_semantic_structure(&non_empty, &empty));
        assert!(same_semantic_structure(&empty, &empty));
    }

    #[cfg(windows)]
    #[test]
    fn fill_record_keeps_focus_checked_and_focus_action_bits_independent() {
        let mut node = node(42, vec![]);
        node.semantics.state.checked = Some(AccessibilityCheckState::On);
        node.semantics.actions = vec![AccessibilityActionKind::Focus];

        let mut record: AccessibilityNodeRecord = unsafe { std::mem::zeroed() };
        fill_record(&mut record, &node);

        assert_eq!(record.state_flags & (1 << 1), 0);
        assert_ne!(record.state_flags & (1 << 2), 0);
        assert_ne!(record.actions_mask & (1 << 5), 0);

        node.semantics.state.focused = true;
        let mut focused_record: AccessibilityNodeRecord = unsafe { std::mem::zeroed() };
        fill_record(&mut focused_record, &node);
        assert_ne!(focused_record.state_flags & (1 << 1), 0);
        assert_ne!(focused_record.state_flags & (1 << 2), 0);
        assert_ne!(focused_record.actions_mask & (1 << 5), 0);
    }

    #[cfg(windows)]
    #[test]
    fn patterns_mask_matches_role_state_and_executable_action() {
        let mut button = node(1, vec![]);
        button.semantics.role = AccessibilityRole::Button;
        button.semantics.actions = vec![AccessibilityActionKind::Activate];
        assert_eq!(patterns_mask_for(&button), PATTERN_INVOKE);

        let mut check_box = node(2, vec![]);
        check_box.semantics.role = AccessibilityRole::CheckBox;
        check_box.semantics.state.checked = Some(AccessibilityCheckState::On);
        check_box.semantics.actions = vec![AccessibilityActionKind::Activate];
        assert_eq!(patterns_mask_for(&check_box), PATTERN_TOGGLE);

        let mut slider = node(3, vec![]);
        slider.semantics.role = AccessibilityRole::Slider;
        slider.semantics.state.value_range = Some(elwindui_core::accessibility::ValueRange {
            value: 2.0,
            min: 0.0,
            max: 10.0,
            step: Some(0.5),
        });
        slider.semantics.actions = vec![AccessibilityActionKind::SetValue];
        assert_eq!(patterns_mask_for(&slider), PATTERN_RANGE_VALUE);

        let mut text_input = node(4, vec![]);
        text_input.semantics.role = AccessibilityRole::TextInput;
        text_input.semantics.actions = vec![AccessibilityActionKind::SetText];
        assert_eq!(patterns_mask_for(&text_input), PATTERN_VALUE);

        let mut secure_text = text_input.clone();
        secure_text.semantics.role = AccessibilityRole::SecureTextInput;
        assert_eq!(patterns_mask_for(&secure_text), 0);
        secure_text.semantics.value = Some("secret".into());
        let mut secure_record: AccessibilityNodeRecord = unsafe { std::mem::zeroed() };
        fill_record(&mut secure_record, &secure_text);
        assert_eq!(secure_record.value_length, 0);

        let mut radio = node(5, vec![]);
        radio.semantics.role = AccessibilityRole::RadioButton;
        radio.semantics.state.selected = Some(true);
        radio.semantics.actions = vec![AccessibilityActionKind::Select];
        assert_eq!(patterns_mask_for(&radio), PATTERN_SELECTION_ITEM);

        let mut expandable = node(6, vec![]);
        expandable.semantics.state.expanded = Some(false);
        expandable.semantics.actions = vec![AccessibilityActionKind::Expand];
        assert_eq!(patterns_mask_for(&expandable), PATTERN_EXPAND_COLLAPSE);

        let mut dropdown = node(7, vec![]);
        dropdown.semantics.role = AccessibilityRole::ComboBox;
        dropdown.semantics.actions = vec![AccessibilityActionKind::Focus];
        assert_eq!(patterns_mask_for(&dropdown), 0);

        let mut missing_action = check_box.clone();
        missing_action.semantics.actions.clear();
        assert_eq!(patterns_mask_for(&missing_action), 0);
        assert_eq!(patterns_mask_for(&radio), PATTERN_SELECTION_ITEM);
    }

    #[cfg(windows)]
    fn patterns_mask_for(node: &AccessibilitySnapshotNode) -> u32 {
        let actions_mask = node.semantics.actions.iter().fold(0u32, |mask, action| {
            mask | (1u32 << action_kind_code(*action))
        });
        patterns_mask(
            node.semantics.role,
            node.semantics.state.checked,
            node.semantics.state.expanded,
            node.semantics.state.selected,
            node.semantics.state.value_range.is_some(),
            actions_mask,
        )
    }

    #[cfg(windows)]
    #[test]
    fn fill_record_copies_identifier_mixed_state_and_range_without_aliasing_bits() {
        let mut semantic = node(42, vec![]);
        semantic.semantics.identifier = Some("a11y-slider".to_owned());
        semantic.semantics.role = AccessibilityRole::CheckBox;
        semantic.semantics.state.checked = Some(AccessibilityCheckState::Mixed);
        semantic.semantics.state.focused = true;
        semantic.semantics.state.value_range = Some(elwindui_core::accessibility::ValueRange {
            value: 2.5,
            min: -1.0,
            max: 8.0,
            step: None,
        });
        semantic.semantics.actions = vec![AccessibilityActionKind::Activate];

        let mut record: AccessibilityNodeRecord = unsafe { std::mem::zeroed() };
        fill_record(&mut record, &semantic);

        assert_eq!(record.identifier_length, 11);
        assert_eq!(
            String::from_utf16_lossy(&record.identifier[..11]),
            "a11y-slider"
        );
        assert_ne!(record.state_flags & (1 << 6), 0);
        assert_eq!(record.state_flags & (1 << 2), 0);
        assert_ne!(record.state_flags & (1 << 1), 0);
        assert_eq!(record.patterns_mask, PATTERN_TOGGLE);
        assert_eq!(record.value, 2.5);
        assert_eq!(record.minimum, -1.0);
        assert_eq!(record.maximum, 8.0);
        assert!(record.step.is_nan());
        assert_eq!(record.has_range, 1);
    }

    #[cfg(windows)]
    #[test]
    fn physical_bounds_normalize_converted_screen_points() {
        let bounds =
            physical_bounds_from_points(Point { x: 120.0, y: 80.0 }, Point { x: 240.0, y: 200.0 });
        assert_eq!(
            bounds,
            Rect {
                x: 120.0,
                y: 80.0,
                width: 120.0,
                height: 120.0
            }
        );
    }
}
