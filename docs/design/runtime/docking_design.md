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
content; dynamic page replacement is outside this design scope. A normal tab selection updates only the existing
group's selected bookkeeping and the bound value when its fast-path preconditions hold. It does
not rebuild groups, splitters, surfaces, wrappers, or native hosts.

Successful normal selection publishes `last_applied_model`, the bound layout, and the layout
callback in that order, without invalidating DockingControl's containing visual subtree. The
retained `CustomTabView` content presenter remains responsible for the local layout work needed to
show the selected page. Structural/source layout updates keep their containing-subtree invalidation
because they can replace native descendants and must repaint surrounding native siblings.

The runtime owns presentation only. It does not serialize wrappers, visual parents, native Window
handles, callbacks, or surface registrations in `DockLayoutSnapshot`.

Docking has no group-level drag session or group-level tear-out/cross-dock gesture. The private
group host contains no separate group title bar. Individual `CustomTabViewItem` drag remains the
only Docking drag source and moves one Document through reorder, Center, Split, root Dock, or Float
transactions. A Bottom content header presents operations for the selected Document only; its
non-action area may begin that Document's item drag, never a group drag. Native floating windows
continue to move through platform-owned title dragging. On Windows a doc-hidden
backend integration hook hides the OS caption while retaining its resizing
border and installs a 32-pixel custom title area. Native window movement remains
backend-owned and separate from the document drag coordinator. The floating
body viewport and client/screen conversions account for the title inset exactly
once, keeping Dock surfaces body-local. Native handles do not enter Core/Docking
API; staged hosts, weak close/bounds callbacks and close veto retain their
existing ownership. Other backends keep native decorations.
Tab context menus dispatch capability-checked
close, indexed-close, float, and pin operations through the same model transaction boundary as
their pointer equivalents. Empty authored groups marked `show_when_empty` retain their group host
and display a non-hit-testable drop hint; other empty groups are normalized away. Per-group
`compact_tabs` defaults to compact sizing to match the pinned WinUI.Dock `TabView` and is applied to
the retained tab view without replacing wrappers or page content; authored groups can explicitly
request equal-width headers.

## Main surface and split realization

`DockSurfaceView` is the private retained root containing the main root and the surface chrome. A
snapshot split with N children retains a pane Grid with N Star tracks and N-1 splitter controls.
Horizontal splits use columns, one Star row, and `column_spacing = 12`; vertical splits use rows,
one Star column, and `row_spacing = 12`. The 12-pixel Grid spacing separates adjacent pane bounds.
`DockSplitView` retains the pane Grid and hosts the splitter controls in a full-size single-cell
template Grid. After arranging the pane Grid, it arranges each splitter across the full orthogonal
extent, beginning at the following pane track. Each 12-pixel hit target is translated -12 pixels on
the split axis, so it occupies the spacing gap without adding a pane or splitter track. Each
splitter holds a weak resize target to the pane Grid that owns the authoritative tracks. Every
splitter records a private `SplitAddress` (main/floating root plus child path) and adjacent boundary
index.

Each realized splitter is a `CustomGridSplitter` with explicit `Columns`/`Rows` direction and
`PreviousAndCurrent` behavior. Its attached row/column records the following-track placement used
to resolve the preceding and following pane tracks in the pane Grid. Its 12-pixel hit target
occupies the spacing gap while the visible 4-by-24 or 24-by-4 grip stays centered. Realization
copies Docking pane min/max rules into the pane Grid's indexed track constraints; spacing and
splitter visuals do not become pane tracks or constraints. The splitter captures the authoritative
Grid definitions, resolved sizes, and constraints and owns all live preview,
baseline-derived cumulative resizing, relayout, and cancellation restoration.

Docking's `SplitterSession` retains only the committed model, split address,
boundary, and the weak/runtime identity needed to interpret one effective
completed cumulative delta. Start and delta callbacks do not mutate Grid
tracks. A canceled completion discards this model-only session. A successful
completion transforms adjacent normalized weights once and routes the result
through the existing reentrancy-safe value path; it never applies the delta to
Grid a second time. If validation rejects the model update, existing
authoritative-state re-realization semantics remain in force.

## Callback and source flow

The docking demo's dark comparison palette uses black window/menu space, #0f0f0f page fill,
#1f1f1f content-header fill, and a translucent white default stroke (alpha 18), matching the pinned
reference capture and its WinUI control stroke resource. The menu's File/Help labels are vertically
centered in 40 pixels with a 14-pixel leading inset; fixture tools remain available at the trailing
edge. These demo resources do not change application theme APIs or Docking ownership.

Runtime group callbacks are installed once and capture only a weak `DockingControl`. They dispatch
selection, close, and all three tab-drag events. `CustomGridSplitter` callbacks dispatch
start/delta/completion after Grid mutation or rollback. The custom controls remain the owners of
pointer threshold, capture, Grid resize, and cancellation state; Docking is an observer/persistence
consumer.

Generated groups take their tab-strip position and compact tabs from the owner's
`set_on_group_created` hook. The runtime asks it with the group's first Document when it first plans
that generated group and keeps the answer in a runtime-only map keyed by the generated group id,
pruned with the realized groups.

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

Selection fast-path qualification does not create snapshots: `DockLayoutModel` checks that the
activated item is neither closed nor auto-hidden and is present in a live group by recursively
reading the main and floating roots. `RuntimeRealization` then checks the requested group item,
retained tab selected index, and presentation owner before updating selection bookkeeping. Closed
and auto-hidden activation continue through the general model/reconcile path.

The shared layout/theme update hook applies model/layout handling first, then compares the current
theme environment signature with the last signature used by the retained runtime. The signature
contains the `BrushStyle` values for primary, secondary, tertiary, foreground, background,
window-background, tint, selection, separator, placeholder, and link. A layout-only update leaves
the signature unchanged and skips `RuntimeRealization::refresh_theme`; a changed signature is
stored before refreshing the retained runtime so a synchronous update cannot repeat the refresh.
The signature is captured after initial realization and cleared when that runtime is disposed.

Runtime-drawn accent chrome resolves through `support::accent_brush`: the Theme's `Primary` when it
resolves to a value, otherwise `core::theme::platform_accent_color()`. Backends register that query
in `init()` through the doc-hidden `set_platform_accent_provider` hook (Windows `UISettings` accent,
macOS `controlAccentColor` in sRGB), so no native type crosses Core/Docking. The query is evaluated
when chrome is created or refreshed; `CustomTabViewItem`'s active marker uses the same rule.

Authored registration callbacks are bound on every current declaration node after each traversal.
They guard reentrancy, cancel stale gestures, refresh item/group metadata, repair removed authored
group references, and publish only when the layout value actually changes.

## Drag target and preview

`SurfaceRegistry` stores a weak surface reference together with its private `RootKind` (floating
indices follow the committed model vector; the main surface is registered last for deterministic
discovery). Bounds are computed from arranged dimensions and the visual-parent chain to the hosted
visual root. Screen-position target discovery converts screen -> host-root with
`screen_to_root`, then host-root -> surface-local by subtracting the registered surface origin;
without a screen position only the source surface's root-relative point is eligible. Resolution
uses the drawn target geometry: `overlay::root_target_rect` for the four 36-pixel root-edge targets
and `overlay::group_target_rect` for the five compass cells of the deepest group containing the
pointer. Drawing and resolution share these functions, so a target resolves exactly where it is
painted. A point on neither resolves no target. Floating surfaces hide their root-target layer
(`DockTargetOverlay::set_root_targets_hidden`) and resolution skips root targets there.

`StableItemRegistry` records each authored group's `DockSize`, inheriting a split panel's
cross-axis extent. `apply_planned_node` builds a `Fixed` track for a child whose extent is fixed
along the split axis (a perpendicular split is fixed at its largest child extent when every child
is) and a `Star(weight)` track otherwise, with authored min/max as `GridTrackConstraint`s. Planned
group hosts are published before the split tree is applied so this lookup sees them. On splitter
completion `remember_fixed_tracks` stores the Grid's resulting pixel extent for fixed children in
the runtime-only `fixed_overrides`; snapshots keep only weights.

Target discovery returns one private `ResolvedDockTarget` containing the destination `RootKind`,
`DockTarget`, optional group key and arranged group rectangle, computed surface-local preview
rectangle, and an optional Center tab insertion index. Outer Dock targets use the selected surface's
root; group targets are filtered to groups belonging to that root. The preview is the group bounds,
a half-group split, or the half surface matching the committed 1:1 root-edge wrap.
`RuntimeRealization::resolve_drop` also reports the hovered group, so pointer movement keeps that
group's compass and the root targets visible while no cell is resolved. Per-move updates are idempotent:
unchanged preview, marker, and target state performs no invalidation, overlay visibility flips only
when a surface's overlay first appears or is cleared, and only surfaces other than the hovered one
are cleared, so a moving drag costs arrange-only work instead of a whole-tree Measure. Which overlay
elements are visible is decided when the overlay state changes, never inside arrange, so even the
first appearance settles in one layout pass. Release without a resolved
target follows the floating path (subject to `can_float`). The model transaction that commits any
drop also activates the moved Document. Each group view reports every tab press through the
doc-hidden `CustomTabView::set_on_tab_pressed`, so pressing an already-selected tab still activates
it; each group container also registers a handled-events-too `on_pointer_pressed` handler that
activates the selected Document for presses its content consumes. Each group container holds a body Grid (content header row
above the tab view) and, as its sibling, the active-frame overlay, so the frame spans the header.

`DockTargetOverlay` has two retained visual layers per surface. The root-target layer stays in
surface coordinates and owns the four edge targets. The group-compass layer uses the target group's
arranged rectangle, converted into that surface's coordinates, to place a 124-pixel connected cross
with five 36-pixel glyph cells at the group's center. The two layers have independent visual state;
no root Dock target aliases a similarly oriented group Split. All target cells are non-hit-testable,
and only the source drag coordinator consumes the resolved target. The retained preview uses the
resolved surface-local rectangle and active fill, default separator stroke, 4-pixel border/corner,
and 0.4 opacity. It never recomputes target semantics.

Each target cell keeps a 4-pixel inset, default fill/separator stroke, and an active/accent document
outline. Center renders a whole-document glyph; Split adds a dashed midline along the split
direction; Dock renders a half-document glyph with a small square on the opposite side. Root-edge
targets use a zero edge inset. The compass backing is a
single rounded, stroked connected cross with a 2-pixel outer inset. Preview geometry communicates
the selected target; the target cell does not become a large accent block. Compose these visuals
from existing Core shapes rather than copying the reference's path data.

Center insertion queries the retained `CustomTabStripPresenter` header arrangement, including
unequal and compact headers, rather than estimating equal widths or reconciling the tab view. It
has precedence over compass-cell resolution while the pointer is inside that arranged header,
then returns the actual midpoint-derived index and the matching retained header boundary. One retained
two-logical-pixel insertion marker is arranged at that boundary using the semantic accent brush.
Target, preview, marker, and commit consume the same resolved index for the dragged Document. The
coordinator has no group-placement drag path.

`begin_drag` collapses the dragged Document's tab wrapper and, when it was the selected tab, sets
the view's presented `selected_index` to a neighbour without the selection callback;
`restore_drag_hidden_tab` undoes both on every drag end (commit, cancel, authored refresh,
transient cancel, disposal). `CustomTabView::forward_selected_item_pointer_event` keeps a content
header gesture on the item that received the press, so the neighbour presentation cannot redirect
it.

`DragSession` retains the committed model, source `RootKind`, source group bounds in host-root
coordinates, the pointer offset used to place an individual Document's 400 x 400 floating window
(clamped inside that size), and a runtime-only
candidate item placement. Moving a tab updates only
`DropPreview`, the target highlight, and the retained insertion marker; it never applies candidate
ownership, reparents content, measures pages, or reconciles the model. Cancel, capture loss, source
removal, source application, and unmount clear every surface preview, marker, and session.

## Group chrome and tab sizing

`CustomTabView` owns tab-strip measurement and retained selection/content presentation. Non-compact
headers share available width up to 200 pixels each; compact headers measure from their text/content
under the same cap. Both modes use the same 32-pixel strip and 32-pixel item height. The active and
pointer-over header frames are template states; changing a state updates retained visuals and never
replaces a tab wrapper or page.

The generic tab view paints a neutral content frame and makes the selected header meet that frame.
Its tab row has a 6-pixel leading baseline and a trailing baseline after compact headers; header
content uses 12-pixel leading and 8-pixel trailing insets. Selection and document activation remain
separate: reconciliation drives active chrome only from `DockLayoutModel::active_item()` and never
falls back to the selected tab. With no active item, the selected header keeps its normal outline
but has no active marker or group frame. An active `CustomTabViewItem` owns its 4-by-16 marker
inside the header before the title. Its intrinsic width includes the marker slot only while active,
along with title and reserved action slot, so compact document labels remain visible. Docking's
private group host adds the active-color frame over content only for an explicitly active document;
it does not paint the document marker over the page.

Docking-specific chrome stays in the retained group realization keyed by `DockGroupId`; there is no
independent group title or group drag surface. In a Bottom group, the active item's title and
pin/close actions live in a private content-header row above the same selected page. The header
refers to the active stable item wrapper and routes requests through existing item callbacks. Its
non-action area may initiate a drag for that item only. The bottom tab strip remains interactive
when multiple items are present and is collapsed/non-hit-testable when exactly one item is present.
Neither path creates a second page presenter or changes item ownership.

## Auto-hide and native floating hosts

Bottom content headers use the pinned 8-pixel horizontal and 6-pixel vertical
insets, with adjacent 24-pixel actions and an 8-pixel title/action gap. The title
is 14-pixel semibold and wraps within the remaining track. Header height is at
least 40 pixels and grows for a wrapped title, so narrow groups do not move the
actions over text. Title and actions remain vertically centered.

`AutoHideOverlay` owns four custom strip Grids, title-first side entries, one overlay pane, resize
grip, and pin/close affordances. It attaches the stable wrapper to the pane, so auto-hide never
creates a second page. The bound model controls which entry is open. Pinning uses
`RuntimeRealization::pin_side`: the runtime preferred side recorded by the last root-edge drop or
unpin for that item, otherwise the reference shape rule over the group's surface-local bounds. The
pane's pin action commits a root-edge placement on the auto-hide root and side and activates the
item. Light dismissal notifies the owner, which commits `with_auto_hide_dismissed` (entry closed,
active item released when it was that item). Preferred sides are runtime-only and cleared with the
runtime.

The strip entries measure from their title, rotate for Left/Right, remain horizontal for Top/Bottom,
space entries by 16 pixels, and retain a 4-pixel theme marker for active/hover state. The pane fills
the usable center region on its perpendicular axis and begins at one third of that axis unless a
runtime extent exists. Resizing is local to the pane edge (inverted for Right/Bottom), remains
bounded to leave usable center content, and updates the transient extent for the item and axis.

`RuntimeRealization` owns the auto-hide extent cache, keyed by `DockItemId` with separate width and
height values so a side change does not reinterpret an extent across axes. Each surface overlay
reads/writes the same owner cache through callbacks that do not retain the owner. Ordinary model
updates, close/reopen, and pin/unpin keep these extents; owner runtime reset/disposal clears them.
They never enter `DockLayoutSnapshot` or V2 persistence. Outside dismissal is observed by the common
surface root so an overlay does not consume clicks in remaining content; Escape is handled only via
the existing routed input path. The 40-pixel pane header owns its Document title and pin/close
actions; its non-action region may start a drag for that open Document only. The page wrapper stays
stable below it.

`SurfaceRuntime` retains one surface, auto-hide controller, preview controller, target sets, and one
insertion marker for the main root and for every floating root. The surface's main root sits in the
center cell of a retained 3x3 host Grid whose edge tracks equal the visible strip extents, so strips
reserve layout space instead of covering content. `FloatingHostRegistry` maps model
floating-root positions to native
windows on AppKit and WinUI3. The adapter implements a private `FloatingWindowHost` contract for
content, logical bounds, show, activation, close, and native close interception. Native move/resize
notifications update the model's floating bounds through the same source/property transaction
path. A new host follows
prepare -> runtime commit -> owner model/property commit -> callback -> registry synchronization
-> show; preparation failure therefore does not require changing committed wrapper ownership.
GTK deliberately has no adapter in this change. Native close handlers capture only weak Docking state and a stable private
`FloatingHostId`; owner disposal clears handlers before closing every host.
