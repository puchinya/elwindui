# Accessibility runtime design

Related specification: [`../../specs/accessibility_spec.md`](../../specs/accessibility_spec.md).

## Ownership and graph separation

`elwindui-core` owns semantic accessibility. A hosted `UIElement` tree is traversed independently
from the retained `RenderTree` and from backend native children. `AccessibilityId` is allocated by a
process-wide atomic counter at Core construction and is never derived from `render_group_id`.
Native AppKit and XAML objects are projections and never cross the Core public API.

The runtime is per host, not global:

```text
hosted Core root
  -> AccessibilityRuntime
       -> immutable AccessibilitySnapshot
       -> AccessibilityId -> Weak<dyn UIElementExt>
       -> revision / effective diff
  -> weak AccessibilityHost scheduling route
```

The root receives `AccessibilityHost` in parallel with `RelayoutHost`, `FocusHost`,
`CoordinateHost`, and `PointerGestureHost`. Backend implementations keep only weak routes back to
their host. Clearing or unmounting the root clears every capability and breaks the cycle.

## Snapshot construction

The runtime walks `visual_children()` from the hosted root after Core layout has produced arranged
geometry. Each visit first checks `Visibility::Collapsed`, hosted activation, `Exiting`, and
`accessibility_hidden`. A transparent node recurses into children and hoists semantic descendants;
an effective intrinsic or explicit role creates a snapshot node. `Automatic`, `Ignore`, and
`Contain` determine descendant emission. A visited `AccessibilityId` set prevents duplicate
emission if presentation topology reaches the same Core element more than once.

Bounds are accumulated from `arranged_offset()` and the existing local presentation transform,
including transform origin. The transform/hit-test affine helper is shared or refactored rather
than reimplemented for accessibility. The resulting transformed rectangle is normalized to an
axis-aligned root-relative `Rect`. Missing geometry produces an empty rectangle. Coordinate-to-
screen conversion remains a backend responsibility.

The snapshot is immutable for adapter reads. Internal flat indexes may point to snapshot nodes, but
the public snapshot contains only values and child vectors. Runtime mutable borrows are never held
while invoking application or platform callbacks.

## Intrinsic semantics and action flow

Core controls provide intrinsic semantics and actions through the ordinary class override mechanism.
Built-ins call existing public setters, routed events, click paths, and `focus()`; they do not
mutate a native handle directly. Explicit UIElement properties are merged after intrinsic defaults:
role is explicit when present, text/value fields override only when set, and child behavior follows
the explicit property when supplied.

An adapter dispatch follows this sequence:

1. Resolve the `AccessibilityId` to a weak owner without retaining a mutable runtime borrow.
2. Verify the owner is still attached to this host and participates in the current snapshot.
3. Call `perform_accessibility_action`.
4. If it returns `false`, take the registered `on_accessibility_action` callback and invoke it.
5. Request/coalesce a semantic update after the action.

The callback is invoked after all runtime borrows are released. A callback can remove itself,
rebuild children, change focus, or update a value. A missing owner, host mismatch, or absent current
semantic node returns `false` and cannot invoke a native child as a fallback.

## Invalidation and effective changes

Accessibility setters request semantic invalidation in addition to any layout invalidation their
property requires. Intrinsic value/state changes use the same notification path as their ordinary
Core mutation. Focus transitions request updates after the existing focus tracker updates
`focus_state`. Child collection mutations, activation/deactivation, visibility changes, and
`begin_exit` request updates immediately.

The host may coalesce repeated requests in one event-loop turn. Before publishing a new snapshot it
compares effective node structure and semantic fields, including order, bounds, focus, value, and
actions. Backend notifications are advisory and are emitted only for effective structural, focus,
or value changes. Correctness is always obtained by querying the current snapshot.

## Templates and participation

The semantic walk uses the Core visual tree, but recognizes template ownership boundaries. A
templated `Control` has one public semantic node when its intrinsic or explicit role makes it
semantic. Private template chrome is not independently exposed under `Automatic`. `ContentPresenter`
places logical content visually but does not create a second semantic content node. Transparent
layout/presentation elements hoist descendants. `Ignore` and `Contain` are applied only to the
effective current semantic node.

`VisualParticipation::Exiting` is excluded even though render traversal retains it until transition
completion. This gives immediate semantic removal with deferred visual cleanup. Normal collection
removal and host deactivation clear owners from the next/current runtime state; stale adapter
objects remain safe but unqueryable.

## AppKit projection

`TreeHostView` owns a cache of synthetic `NSAccessibilityElement` instances keyed by
`AccessibilityId`. The host exposes its Core snapshot as semantic children with stable parent,
child, and navigation order. Each cached object reads role, label, value, hint, identifier,
enabled/focus state, and root-to-screen frame from the snapshot at query time. Hit testing searches
the snapshot bounds and returns the synthetic object.

Actions call the Core runtime by ID. The cache is pruned after effective snapshot changes and fully
cleared before host teardown. `NativeIslandView` remains the transform/input/native containment
boundary, but returns no public accessibility children in both Active and Exiting states once the
synthetic host is active. Its raw inner native controls cannot appear as duplicate public children.

## WinUI 3 projection and C++ boundary

The actual `TreeHostPanel` backing element is a Canvas-compatible C++/WinRT XAML subclass. Its
`OnCreateAutomationPeer` returns a root `FrameworkElementAutomationPeer` implementation whose
`GetChildrenCore` reads only the Core snapshot through a narrow Rust C ABI. Virtual semantic child
peers are cached by integer `AccessibilityId`; peer identity is not tied to native child identity.

C++ owns only composable host subclassing, AutomationPeer/pattern plumbing, value copying, and the
callback context lifetime. It does not choose roles, traverse Core elements, hold semantic policy,
own IDs, mutate controls, or invoke user callbacks directly. Rust owns the runtime, traversal,
semantic values, and dispatch. No `Rc`, Rust reference, native pointer, or borrowed string crosses
the ABI; values are copied into caller-owned buffers or structs.

The bridge detaches its callback context before XAML host destruction. Late UIA calls return empty
or unavailable values. UI mutation is scheduled on the WinUI UI thread. The narrowest XAML
accessibility-view suppression is applied to native projection children so the custom host peer is
the public tree. A raw HWND-wide `WM_GETOBJECT` provider is not introduced.

The root exposes name/control type/bounds/enabled/focus/children and focus action. Invoke, Toggle,
RangeValue, Value, SelectionItem, and ExpandCollapse are advertised only when matching Core action
kinds are present and executable.

## Teardown and failure semantics

Host teardown order is platform callback detach, AX/peer cache clear, Core host capability clear,
runtime snapshot/owner-map release, then native host release. Backend adapter errors are isolated
from layout, render, and input. An adapter may expose an empty/unavailable projection, but it never
reconstructs semantics from native widgets to mask a projection failure.

## Test strategy

Core tests cover ID stability, transparent traversal, explicit Canvas semantics, hidden/collapsed/
inactive/exiting exclusion, Ignore/Contain, template/content de-duplication, reorder, secure text,
reentrant removal, and built-in action paths. AppKit tests cover cache identity, mapping, order,
action/stale-ID dispatch, teardown, and native duplicate suppression. WinUI tests cover custom peer
creation, snapshot-only children, cache identity, pattern gating, native duplicate suppression,
and ABI teardown on Windows. One backend-neutral durable scenario under `tests/e2e/` is consumed by
platform-specific AppKit and WinUI drivers.
