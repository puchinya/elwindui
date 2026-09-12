# ElwindUI Accessibility Specification

## 1. Scope and ownership

Accessibility is an adopted 0.1.0 public contract. `elwindui-core` owns the semantic accessibility
tree. The tree is derived from the Core `UIElement` tree and is the sole source of truth for roles,
names, values, state, bounds, children, and supported actions. AppKit AX and WinUI 3 UIA are
projections of that immutable semantic state; native widget hierarchies are never an authoritative
accessibility tree.

The contract covers individual semantic controls and presentational traversal. Composite
Menu/Tab/tree/grid/list semantics are not required by this first slice.

## 2. Public identity and data model

Every `UIElement` receives one process-wide `AccessibilityId` at construction. The identity is
stable for the lifetime of that Core element, survives property changes and reordering, and is not
reused as `render_group_id`. A removed element's ID becomes stale and is not reassigned to another
element.

The public semantic model is:

```rust
pub struct AccessibilityId(u64);

pub enum AccessibilityRole {
    Button, StaticText, TextInput, SecureTextInput, CheckBox,
    RadioButton, Switch, Slider, ComboBox, Group,
}

pub enum AccessibilityCheckState { Off, On, Mixed }

pub struct ValueRange {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
}

pub enum AccessibilityChildBehavior { Automatic, Ignore, Contain }

pub enum AccessibilityActionKind {
    Activate, Increment, Decrement, SetValue, SetText, Focus,
    Expand, Collapse, Select,
}

pub enum AccessibilityAction {
    Activate, Increment, Decrement, SetValue(f64), SetText(String),
    Focus, Expand, Collapse, Select,
}

pub struct AccessibilityState {
    pub disabled: bool,
    pub focused: bool,
    pub checked: Option<AccessibilityCheckState>,
    pub expanded: Option<bool>,
    pub selected: Option<bool>,
    pub read_only: bool,
    pub value_range: Option<ValueRange>,
}

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
```

Backends consume an immutable snapshot:

```rust
pub struct AccessibilitySnapshotNode {
    pub id: AccessibilityId,
    pub semantics: AccessibilitySemantics,
    pub bounds_in_root: Rect,
    pub children: Vec<AccessibilitySnapshotNode>,
}

pub struct AccessibilitySnapshot { pub roots: Vec<AccessibilitySnapshotNode> }
```

The public snapshot contains no native handles, raw pointers, or Core ownership objects.

## 3. UIElement customization

All `UIElement`s inherit these ordinary properties and one callback surface:

| Property | Type | Meaning |
|---|---|---|
| `accessibility_role` | `Option<AccessibilityRole>` | Explicit semantic role; makes a transparent element semantic |
| `accessibility_label` | `Option<String>` | Accessible name |
| `accessibility_value` | `Option<String>` | Accessible value |
| `accessibility_hint` | `Option<String>` | Additional usage hint |
| `accessibility_identifier` | `Option<String>` | Stable automation identifier supplied by the application |
| `accessibility_hidden` | `Option<bool>` | Excludes the element and all descendants when true |
| `accessibility_children` | `Option<AccessibilityChildBehavior>` | Child projection policy |
| `on_accessibility_action` | `fn(AccessibilityAction)` | Fallback application action callback |

These properties use the existing property/event code generation and normal setter semantics. No
accessibility-specific parser or attribute is part of the DSL contract.

An intrinsic semantic implementation supplies built-in behavior through overridable Core hooks:

```rust
fn accessibility_intrinsic_semantics(&self) -> Option<AccessibilitySemantics>;
fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool;
```

Explicit properties override or augment intrinsic semantics. An action is offered to the intrinsic
hook first; the user callback is called only when the intrinsic hook returns `false`. A single
platform action therefore cannot produce duplicate activation.

## 4. Participation and traversal

The snapshot is built from Core visual state and visual children. A node and its descendants are
excluded when the node is `Visibility::Collapsed`, belongs to an inactive hosted subtree, has
`VisualParticipation::Exiting`, or has `accessibility_hidden == true`.

Elements without intrinsic semantics or an explicit role are transparent presentation nodes. Their
semantic descendants are hoisted in order. Layout, Shape, and Canvas are transparent by default;
an explicit role makes a self-drawn element semantic without a native control.

`Automatic` hoists transparent descendants and suppresses private template chrome that would repeat
the logical control. `Ignore` keeps the current semantic node but omits semantic descendants.
`Contain` keeps the current node and its semantic descendants. There is no `Combine` behavior.

Template roots and `ContentPresenter` are presentation machinery. They do not create duplicate
public nodes for a templated control or its logical content. If traversal can reach an element via
more than one presentation path, its `AccessibilityId` is emitted once.

Logical removal removes semantics immediately. An exit transition may retain visuals, but the
exiting element is absent from the snapshot and cannot receive an accessibility action.

## 5. Built-in semantics

The minimum 0.1.0 mapping is:

| Type | Role | Required semantics and actions |
|---|---|---|
| `TextBlock` | `StaticText` | Displayed text; no action |
| `Button` | `Button` | Text label; `Activate` uses the existing click path; `Focus` when focusable |
| `TextArea` | `TextInput` | Current text; `SetText` uses normal property/event path; `Focus` |
| `PasswordBox` | `SecureTextInput` | Focus only as applicable; never expose plaintext |
| `CheckBox` | `CheckBox` | `Off`/`On`/`Mixed` where supported; `Activate` uses normal state/event path |
| `RadioButton` | `RadioButton` | Checked/selected state; `Activate`/`Select` where Core supports them |
| `ToggleSwitch` | `Switch` | Checked state; `Activate` |
| `Slider` | `Slider` | Current/min/max/step; `SetValue`/`Increment`/`Decrement` use normal value path |
| `Dropdown` | `ComboBox` | Label and selected value; only actions executable by the existing Core API |

Accessibility actions do not introduce a second clamp, state, or event policy. Intrinsic actions
must call the same public Core mutation/event paths used by ordinary input.

## 6. Focus, actions, and reentrancy

`Focus` routes through the existing per-host Core focus ownership and normal backend focus
synchronization. Accessibility does not maintain a second focus tracker.

Dispatch copies or upgrades the weak owner for the ID, releases runtime/snapshot mutable borrows,
validates current host membership and semantic participation, calls the intrinsic hook, and then
optionally calls the user callback. Callbacks may remove the target, change focus, mutate children,
or change values. Such reentrant mutation is valid and must not cause a `RefCell` borrow panic.
Unknown or stale IDs return unsupported/`false`; they never fall back to native widget actions.

## 7. Geometry

`bounds_in_root` is an axis-aligned bounding rectangle in host-root coordinates. It is computed from
Core arranged/presentation geometry, including ancestor offsets and presentation transforms, using
the same transform semantics as rendering and hit testing. Adapters convert root coordinates to
screen coordinates with their existing host coordinate conversion. Titlebar and window decoration
geometry is never estimated. Empty bounds are valid before layout or when geometry is unavailable.

## 8. Privacy and backend equivalence

`PasswordBox` plaintext must not occur in semantic values, fallback labels/descriptions, snapshots,
debug formatting, diagnostics, logs, E2E evidence, AX values, or UIA values.

AppKit and WinUI 3 must expose equivalent semantic roles, names, values, states, identifiers,
children, bounds meaning, focus behavior, and supported actions. Platform pattern names and role
constants may differ, but a backend may advertise a pattern only when the Core node advertises and
can execute the corresponding action. Adapter failure cannot alter Core semantics or fall back to
raw native accessibility.

## 9. Host lifecycle

Each hosted root owns one Core accessibility runtime and registers one weak `AccessibilityHost`
capability. The runtime keeps the current snapshot, weak `AccessibilityId` owners, a revision, and
enough effective-change information for advisory backend notifications. Property/state/layout,
focus, child-order, activation, visibility, and exit-start mutations request an update. Coalescing
is allowed, but current semantic removal is synchronous.

On teardown, platform callbacks and caches are detached before the Core host capability, snapshot,
and owner map are released. Late platform queries return unavailable safely.
