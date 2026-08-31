# Docking specification

elwindui-docking provides a backend-neutral docking surface. The crate is separate from the
elwindui facade and consumes the existing component, ContentControl, and custom-control APIs.

## Authored surface

The public authored controls are:

- DockingControl, whose layout: DockLayoutModel property is TwoWay and whose content is one
  DockGroup or DockSplitPanel;
- DockSplitPanel, with orientation, weight, and DockGroup/DockSplitPanel children;
- DockGroup, with an authored DockGroupId, weight, tab_strip_position, and DockItem children;
- DockItem, with an authored DockItemId, title, optional icon, capability flags, and content.

An empty or duplicate authored ID, an unsupported root, an empty split, or a non-positive/non-finite
weight is a deterministic authoring error. IDs are not silently renamed.

## Value model

DockLayoutModel is opaque, Clone, and PartialEq. It owns the main root, floating roots, auto-hide
entries, closed entries, selection, and generated group identity. Model operations are immutable:
activation, close/reopen, group/split/root-edge/floating/auto-hide moves, and reset all return a
new model or a typed DockLayoutError.

DockSide, DockTarget, and DockPlacement are backend-neutral. Programmatic weights and floating
bounds must be finite and positive. A failed operation does not change the live value.

## Capabilities and interaction

can_close, can_pin, can_float, and can_dock constrain user gestures. Selection remains possible for
an item that cannot be docked. Center, four group split, and four outer-edge targets are distinct
target kinds. A canceled drag, lost capture, unavailable coordinate conversion, or failed
floating-host creation preserves the committed model.

Splitter movement is transient until pointer completion and writes one normalized adjacent-weight
update. Auto-hide keeps one open overlay, preserves its return group/index, and restores that
position when unpinned. A floating root belongs to the same model as the main root.

## Snapshot

DockLayoutSnapshot::VERSION is 1. Snapshots contain current layout state only: authored content,
capabilities, and UI element values are not serialized. Unknown versions and malformed
weights/bounds/selection/return state produce typed errors without replacing live state. Generated
group IDs resume above every restored generated identity.
