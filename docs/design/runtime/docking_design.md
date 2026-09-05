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
content; dynamic page replacement is outside V1. A normal tab selection updates only the existing
group's selected bookkeeping and the bound value when its fast-path preconditions hold. It does
not rebuild groups, splitters, surfaces, wrappers, or native hosts.

The runtime owns presentation only. It does not serialize wrappers, visual parents, native Window
handles, callbacks, or surface registrations in `DockLayoutSnapshot`.

## Main surface and split realization

`DockSurfaceView` is the private retained root containing the main root and the surface chrome. A
snapshot split with N children is realized as one Grid with N Star pane tracks and N-1 Fixed(6)
splitter tracks. Horizontal splits use columns and one Star row; vertical splits use rows and one
Star column. Every splitter records a private `SplitAddress` (main/floating root plus child path)
and adjacent boundary index.

On splitter start, the retained Grid extent and committed model are captured. Delta updates derive
a transient track vector directly from the captured adjacent weights and update only the retained
Grid's rows or columns with arrange invalidation. Completion either restores the captured tracks
or performs one adjacent-weight model transformation, one model commit, and one notification;
the completed split-weight value update does not structurally reconcile the Dock runtime.

## Callback and source flow

Runtime group callbacks are installed once and capture only a weak `DockingControl`. They dispatch
selection, close, and all three tab-drag events. Splitter callbacks dispatch start/delta/completion.
The custom controls remain the owners of pointer threshold and capture state.

The generated `layout` update callback routes to one internal source-application method. It compares
against `last_applied_model`, cancels transient state, attaches authored metadata, normalizes, and
applies only the latest reentrant pending value. Structural user changes use a `ReconcilePlan`:
preparation derives all candidate runtime/native work without touching committed ownership, and an
infallible commit performs the single ownership transition. The bound property is updated after
runtime commit, then the user callback is notified exactly once; a shared owner-level finalizer
commits staged floating-host synchronization only after the owner/runtime and published model are
still the same transaction, otherwise it aborts the staged resources. This ordering is used for
source application, initial/default capture, registration refresh, and user structural commits.
There is no reconcile-the-old-model rollback path and no production runtime reconcile shortcut.
Selection-only changes and completed adjacent split-weight changes use retained value/layout fast
paths because neither changes ownership or topology. The subsequent generated property update is
suppressed by equality with `last_applied_model`.

Authored registration callbacks are bound on every current declaration node after each traversal.
They guard reentrancy, cancel stale gestures, refresh item/group metadata, repair removed authored
group references, and publish only when the layout value actually changes.

## Drag target and preview

`SurfaceRegistry` stores a weak surface reference together with its private `RootKind` (floating
indices follow the committed model vector; the main surface is registered last for deterministic
discovery). Bounds are computed from arranged dimensions and the visual-parent chain to the hosted
visual root. Screen-position target discovery converts screen -> host-root with
`screen_to_root`, then host-root -> surface-local by subtracting the registered surface origin;
without a screen position only the source surface's root-relative point is eligible. Surface edge
bands are 10% of the smaller extent clamped to 24..64 pixels. Group split bands are 25% with the
same clamp. Edge ties use Left, Top, Right, Bottom.

Target discovery returns one private `ResolvedDockTarget` containing the destination `RootKind`,
`DockTarget`, optional group key, and computed surface-local preview rectangle. Outer Dock targets
use the selected surface's root; group targets are filtered to groups belonging to that root. The
preview is the group bounds, a half-group split, or a quarter-surface outer band as appropriate.

`DragSession` retains the committed model, source `RootKind`, source group bounds in host-root
coordinates, pointer offset, and a runtime-only candidate placement. Moving a tab updates only
`DropPreview`, whose retained layer arranges a rectangle in surface-local coordinates. It never
applies candidate ownership. Cancel, capture loss, source removal, source application, and unmount
clear every surface preview/session.

## Auto-hide and native floating hosts

`AutoHideOverlay` owns four custom strip Grids, custom icon/title entries, one overlay pane, and a
pin affordance. It attaches the stable wrapper to the pane, so auto-hide never creates a second page.
The bound model controls which entry is open and which remembered return state is used.

`SurfaceRuntime` retains one surface, auto-hide controller, and preview controller for the main root
and for every floating root. `FloatingHostRegistry` maps model floating-root positions to native
windows on AppKit and WinUI3. The adapter implements a private `FloatingWindowHost` contract for
content, logical bounds, show, close, and native close interception. A new host follows
prepare -> runtime commit -> owner model/property commit -> callback -> registry synchronization
-> show; preparation failure therefore does not require changing committed wrapper ownership.
GTK deliberately has no adapter in this change. Native close handlers capture only weak Docking state and a stable private
`FloatingHostId`; owner disposal clears handlers before closing every host.
