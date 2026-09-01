# Docking runtime design

## Projection layers

Docking has three deliberately separate layers:

```text
authored DockGroup/DockSplitPanel/DockItem tree
    -> DefaultDockDefinition + StableItemRegistry
    -> DockLayoutModel
    -> retained DockSurfaceView projection
```

The authored tree remains mounted only as registration input. `DockingControl` presents it through a
collapsed presenter and installs one private `DockRuntimeHost` for the visible projection. The
runtime host contains the main `DockSurfaceView`; each native floating host contains another
surface. Authored controls are never registered as target-discovery surfaces.

## Retained ownership

`StableItemRegistry` keeps one `CustomTabViewItem` per authored item ID. A reconciliation computes a
complete desired ownership map, detaches wrappers from old group/overlay parents before rebuilding
structural children, then attaches the same wrappers to their desired parents. Group views,
splitter collections, and split Grids are retained by authored/generated group identity and
`SplitAddress`. Metadata refresh updates header/icon/capability chrome without replacing page
content; dynamic page replacement is outside V1.

The runtime owns presentation only. It does not serialize wrappers, visual parents, native Window
handles, callbacks, or surface registrations in `DockLayoutSnapshot`.

## Main surface and split realization

`DockSurfaceView` is the private retained root containing the main root and the surface chrome. A
snapshot split with N children is realized as one Grid with N Star pane tracks and N-1 Fixed(6)
splitter tracks. Horizontal splits use columns and one Star row; vertical splits use rows and one
Star column. Every splitter records a private `SplitAddress` (main/floating root plus child path)
and adjacent boundary index.

On splitter start, the retained Grid extent and committed model are captured. Delta updates derive
a transient model from the original and update only the adjacent Grid tracks. Completion either
restores the captured tracks or performs one model commit and one notification.

## Callback and source flow

Runtime group callbacks are installed once and capture only a weak `DockingControl`. They dispatch
selection, close, and all three tab-drag events. Splitter callbacks dispatch start/delta/completion.
The custom controls remain the owners of pointer threshold and capture state.

The generated `layout` update callback routes to one internal source-application method. It compares
against `last_applied_model`, cancels transient state, attaches authored metadata, normalizes, and
applies only the latest reentrant pending value. User callbacks instead calculate a complete
normalized model, reconcile the retained projection, update the bound property, and notify exactly
once. The subsequent generated property update is suppressed by equality with
`last_applied_model`.

Authored registration callbacks are bound on every current declaration node after each traversal.
They guard reentrancy, cancel stale gestures, refresh item/group metadata, repair removed authored
group references, and publish only when the layout value actually changes.

## Drag target and preview

`SurfaceRegistry` stores weak surface references. Bounds are computed from arranged dimensions and
the visual-parent chain up to that registered surface. Screen-position target discovery first
converts through each registered surface; without a screen position only the source surface's
root-relative point is eligible. Surface edge bands are 10% of the smaller extent clamped to 24..64
pixels. Group split bands are 25% with the same clamp. Edge ties use Left, Top, Right, Bottom.

`DragSession` retains the committed model and a runtime-only candidate placement. Moving a tab updates only
`DropPreview`, a real custom rectangle. It never applies candidate ownership. Cancel, capture loss,
source removal, source application, and unmount clear the preview/session.

## Auto-hide and native floating hosts

`AutoHideOverlay` owns four custom strip Grids, custom icon/title entries, one overlay pane, and a
pin affordance. It attaches the stable wrapper to the pane, so auto-hide never creates a second page.
The bound model controls which entry is open and which remembered return state is used.

`FloatingHostRegistry` maps model floating-root positions to native windows on AppKit and WinUI3.
The adapter implements a private `FloatingWindowHost` contract for content, logical bounds, show,
close, and native close interception. GTK deliberately has no adapter in this change. Native close
handlers capture only weak Docking state; owner disposal clears handlers before closing every host.
