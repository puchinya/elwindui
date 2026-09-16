//! WinUI 3 projection state for the Core-owned accessibility snapshot.
//!
//! The XAML peer bridge is deliberately a projection only. It receives stable integer IDs and
//! copied snapshot values from this state; semantic traversal, policy, and action dispatch stay in
//! `elwindui-core`.

use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityActionKind, AccessibilityCheckState, AccessibilityHost,
    AccessibilityId, AccessibilityRole, AccessibilityRuntime,
};
use elwindui_core::ui::UIElementExt;
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::{Rc, Weak};

#[cfg(windows)]
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
#[cfg(windows)]
use windows::core::{IInspectable, Interface};

#[cfg(windows)]
#[repr(C)]
struct AccessibilityNodeRecord {
    id: u64,
    role: u32,
    state_flags: u32,
    actions_mask: u32,
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
    fn elwindui_winui3_accessibility_canvas_notify_tree_changed(bridge_key: *mut c_void);
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
            cpp_bridge: CppAccessibilityBridge::new(),
        })
    }

    #[cfg(windows)]
    pub(crate) fn bind_canvas(self: &Rc<Self>, canvas: &Canvas) -> bool {
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
        let tree: Option<Rc<RefCell<Option<Rc<dyn UIElementExt>>>>> = self.tree.upgrade();
        let Some(tree) = tree.and_then(|tree| tree.borrow().clone()) else {
            self.runtime.clear();
            #[cfg(windows)]
            self.notify_tree_changed();
            return;
        };
        self.runtime.rebuild(&tree);
        #[cfg(windows)]
        self.notify_tree_changed();
    }

    pub(crate) fn clear(&self) {
        self.runtime.clear();
        #[cfg(windows)]
        self.notify_tree_changed();
    }

    #[cfg(windows)]
    fn notify_tree_changed(&self) {
        let bridge_key = self.cpp_bridge.bridge_key.get();
        if !bridge_key.is_null() {
            unsafe { elwindui_winui3_accessibility_canvas_notify_tree_changed(bridge_key) };
        }
    }
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
fn fill_record(
    record: &mut AccessibilityNodeRecord,
    node: &elwindui_core::accessibility::AccessibilitySnapshotNode,
) {
    record.id = node.id.raw();
    record.role = role_code(node.semantics.role);
    record.state_flags = u32::from(node.semantics.state.disabled)
        | (u32::from(node.semantics.state.focused) << 1)
        | (u32::from(node.semantics.state.checked == Some(AccessibilityCheckState::On)) << 2)
        | (u32::from(node.semantics.state.expanded == Some(true)) << 3)
        | (u32::from(node.semantics.state.selected == Some(true)) << 4)
        | (u32::from(node.semantics.state.read_only) << 5);
    record.actions_mask = node.semantics.actions.iter().fold(0u32, |mask, action| {
        mask | (1u32 << action_kind_code(*action))
    });
    if let Some(range) = node.semantics.state.value_range {
        record.value = range.value;
        record.minimum = range.min;
        record.maximum = range.max;
        record.step = range.step.unwrap_or(0.0);
        record.has_range = 1;
    }
    record.x = node.bounds_in_root.x;
    record.y = node.bounds_in_root.y;
    record.width = node.bounds_in_root.width;
    record.height = node.bounds_in_root.height;
    record.label_length = copy_utf16(node.semantics.label.as_deref(), &mut record.label);
    record.value_length = copy_utf16(node.semantics.value.as_deref(), &mut record.value_text);
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
    let Some(node) = snapshot_node(state, id) else {
        return 0;
    };
    *record = unsafe { std::mem::zeroed() };
    fill_record(record, &node);
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
