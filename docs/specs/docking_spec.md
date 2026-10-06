# Docking specification

`elwindui-docking` is a backend-neutral docking surface. It is a separate crate from the
`elwindui` facade and uses the existing Core layout, input, ContentControl, Window, and
custom-control contracts.

## Authored declarations

Applications author one `DockingControl` whose content is a `DockGroup` or `DockSplitPanel` tree.
`DockGroup` registers a stable `DockGroupId`, tab-strip position, and authored weight.
`DockGroup` and `DockSplitPanel` also accept an authored `DockSize` (optional pixel `width`,
`height`, and min/max per axis), mirroring WinUI.Dock module sizes. Inside a parent split, a node
with a fixed extent along the split axis is a fixed-size track and other nodes share the remaining
space by weight; min/max bound the track either way. A split panel's size applies to its children
across the panel's own axis, and a perpendicular split is fixed when all of its children are.
Resizing a fixed track with a splitter keeps the new pixel extent for the lifetime of the runtime.
`DockSize` is authored input only; it never enters `DockLayoutSnapshot` (V2 unchanged). `DockItem`
registers a stable `DockItemId`, title, optional `IconSource`, page content, and the capability
flags `can_close`, `can_pin`, `can_float`, and `can_dock`.

The authored declaration remains mounted so registrations and dynamic `for` changes stay live, but
its presenter is `Visibility::Collapsed`. It is not the visible workspace and a `DockItem` does
not present a second copy of its page. The visible workspace is a retained private runtime host.
Empty or duplicate IDs, unsupported declaration nodes, empty splits, and non-finite or non-positive
weights are authoring errors.

## Value model and source path

`DockLayoutModel` is an opaque, cloneable, `PartialEq` value. It owns the main root, floating
roots, auto-hide entries, closed return states, selection, and generated group identities. Model
operations return a new value and validate placements before changing anything.

The bound `DockingControl.layout` property is the only source-assignment path. A source update
normalizes against the current authored registrations, cancels transient gestures, reconciles the
retained runtime, and never invokes `set_on_layout_change`. Reentrant source updates are latest-only.
An initially empty bound value is initialized from the authored default and published once through
the property and `set_on_layout_change`; a non-empty restored value wins without an initial echo.

## Runtime interaction

Each registered item has one stable runtime `CustomTabViewItem`; selection, close requests, tab
dragging, splitter resizing, auto-hide, floating, and re-docking preserve that wrapper and its page
content identity. Runtime ownership changes use detach-before-attach.

`CustomTabView` supplies selection, close, and tab-drag callbacks, including its existing threshold,
capture, cancellation, root-relative position, and optional logical screen position.
`CustomGridSplitter` owns splitter Grid discovery, track mutation, constraints, live relayout,
rollback, and resize notifications. Split nodes with N children realize as a retained pane Grid
with N Star tracks and N-1 twelve-pixel `CustomGridSplitter` hit targets. Horizontal splits set
`column_spacing = 12`; vertical splits set `row_spacing = 12`, leaving a 12-logical-pixel gap between
adjacent pane bounds. The splitter is placed at the beginning of the following pane's track and
translated -12 logical px on the split axis, so its 12-pixel hit target occupies the gap without a
separate splitter track. Horizontal splitters use `resize_direction = Columns` and
`resize_behavior = PreviousAndCurrent`; vertical splitters use `Rows` and `PreviousAndCurrent`, so
each splitter resizes exactly the two panes on either side of its gap. The visible 4-by-24 column
grip or 24-by-4 row grip remains centered in that target.

Docking supplies pane min/max rules as Grid-owned track constraints during realization. Splitter
hit targets do not participate in pane min/max constraints. A successful splitter completion
updates adjacent normalized model weights exactly once from the effective completed cumulative delta;
Docking does not preview, clamp, restore, or reapply Grid definitions. A canceled completion
discards only Docking's model transaction and publishes no model change because the splitter has
already restored the Grid. Selection-only changes and completed adjacent split-weight changes update
the retained runtime and bound value without rebuilding unchanged structural Dock content.

Document and tool groups expose the authored tab-strip position and chrome appropriate to that
position. Top-tab groups render tabs above their content without a separate group title bar;
bottom-tab groups render tabs below their content and a content header for the selected Document;
when exactly one Document is present, the bottom tab strip collapses while the content header and
content remain visible.
The selected tab joins the content frame with the reference rounded outline treatment. The active
Document's group uses the active frame stroke, and its tab carries a distinct 4-by-16 active marker
inside the header, after the leading inset and before the title. Active-document state comes only
from `DockLayoutModel::active_item()` and is independent of tab selection: when there is no active
item, selection alone does not show the active frame or marker. Like WinUI.Dock, a left press on a
Document's tab activates that Document even when the tab is already selected, and a left press
anywhere inside a docked group (tab strip, content header or page) activates its selected Document
even when the content handles the press. The active group's frame is drawn in the accent color
(theme Primary when set, otherwise the platform accent) and, for bottom-tab groups, encloses the
content header. Closing the active Document activates its group's new selection, or leaves no
active Document when that group becomes empty; a Document of another group is never activated. Compact measurement includes the
marker when active and the title so the title remains legible. The active marker is not drawn over
page content.
Top-tab pin/close affordances appear on pointer hover; bottom-tab actions belong to the content
header.
Bottom content headers keep pin/close actions at the trailing edge with the
reference insets. The title wraps in the remaining width and the header grows
from its 40-logical-pixel minimum as necessary, without overlapping actions.
Docked headers (tab hover actions and content headers) use adjacent 24-logical-pixel action buttons
with the auto-hide glyph (a pin with a small badge) and a 10-pixel close cross, each centered in its
button. The auto-hide pane header uses adjacent 28-logical-pixel buttons with a plain push-pin
(re-dock) glyph and the same close cross. Action buttons show subtle hover/pressed fills.
All Docking drag sources represent one Document. A tab drag may reorder that Document, move it to a
group/root, split a target group, or float it, subject to the item's capability checks. Docking has
no group-level drag, tear-out, or cross-dock operation. Bottom content-header actions and drag
operate on the selected Document and never move its containing group. A floating native window
continues to move through platform-owned title dragging. Windows floating hosts
retain the native resizing border, hide the standard caption, and reserve a
32-logical-pixel custom title area above the Dock surface. This title is centered,
16 pixels and bold; it adds no minimize/maximize/close actions absent from the
pinned reference adapter. Tab/content close and system close (including Alt+F4)
retain the existing capability and close-veto lifecycle. Other backends retain
their existing native decorations. Supported tab context actions are `Close`,
`Close Others`, `Close Tabs to Left`, `Close Tabs to Right`, `Float`, and `Auto Hide / Pin`; each
action uses the same capability checks and one model transaction as its pointer equivalent.

An authored empty group remains visible only when `show_when_empty` is true, like WinUI.Dock's
`ShowWhenEmpty`. It keeps its empty tab view and frame, draws no hint text, and remains a valid drop
target. Without `show_when_empty` an empty group is removed and its split collapses.
`compact_tabs` selects the compact tab metrics for that group and defaults to `true`, matching
WinUI.Dock's compact `TabView` headers. Explicitly setting it to `false` distributes available
header width up to the 200-pixel cap. Clear/reset operations remove the live presentation without
consulting `can_close`, preserve the authored declaration, and restore the authored default
deterministically.

Drag movement changes only a custom drop-preview rectangle and candidate target. It never reparents
page content or reconciles a preview model. Like the reference, a dragged tab leaves its strip while
the drag is active, and when it was selected its group presents the neighbouring Document; both are
presentation-only and restored before the drop commits or the drag cancels. Completion commits one normalized model. A release with
no resolved target, inside or outside every surface, floats the Document when `can_float` allows it
and otherwise cancels. Every committed drop makes the moved Document the model's active item in the
same transaction. The private resolved target retains the destination root, target group,
and surface-local preview rectangle as one value, so preview, hit testing, and commit cannot diverge.
Targets resolve only where they are drawn: a root Dock target only while the pointer is inside one
of the four 36-logical-pixel root-edge target rectangles of the main surface (floating surfaces,
like the WinUI.Dock reference, draw and resolve only the group compass), and a group Center or Split target only
while the pointer is inside one of the five compass cells of the deepest containing runtime group.
The drawn rectangles and the resolution rectangles come from one geometry source. Cross-window discovery uses only Core `screen_to_root`/`root_to_screen`
conversions and arranged visual bounds, converting screen coordinates to host-root and then
surface-local coordinates by subtracting the surface origin. Without a screen position, only the
source surface is eligible.

Preview geometry is the complete target group for Center, the corresponding half for Split, and the
corresponding half of the surface for an outer Dock target, matching the committed 1:1 root wrap. The rectangle is arranged by a
retained surface-local overlay layer. Its visual uses the active/accent fill, default separator
border, four-logical-pixel border thickness, four-logical-pixel-equivalent corner radius, and 0.4
opacity. It remains non-hit-testable; this chrome does not change the resolved geometry.

The five group targets are a retained 124-logical-pixel connected cross compass
(`SplitTop`, `SplitLeft`, `Center`, `SplitRight`, `SplitBottom`) with five 36-logical-pixel target
cells. Each 36-pixel visual has a 4-pixel inset, default fill and separator stroke, and a document
glyph with active/accent stroke. Center shows a whole document; Split shows a whole document with a
dashed midline in the split direction; Dock shows a half document on the docking side and a small
square where the existing content goes. Root-edge targets sit flush against the surface edges,
centered along each edge. The
connected compass backing has a 2-pixel outer inset and joins the five cells as one rounded, stroked
cross. Target selection is conveyed by the preview rectangle, not a large selected-color cell fill.
Target visuals do not own input or resolve a drop. The compass is centered on the arranged bounds of
the resolved target group, including nested and floating groups, rather than on the surface.

The four root-edge targets (`DockLeft`, `DockTop`, `DockRight`, `DockBottom`) are a separate,
surface-relative retained target set; a root-edge target never aliases or highlights its similarly
oriented group Split target. Both sets are non-hit-testable and the source drag coordinator remains
the only input authority.

For an individual Document drag, a pointer inside the target group's arranged tab-header rectangle
resolves Center insertion for that group. This makes the actual header midpoint available for
Center insertion and same-group reorder, while the same document drag resolves compass targets only
inside their cells.

For a Center drop, the resolved target also carries an optional tab insertion index. The index is
resolved from the retained arranged header rectangles and their actual midpoints: the left side of
a midpoint inserts before that header, the right side proceeds to the next header, and a point in the
content body has no insertion index. Empty strips resolve index zero. A same-group move removes the
source once before applying the resolved index. Preview, highlight, insertion marker, and release
commit all use this same resolved target. Each drag moves one stable Document wrapper and carries
its own optional insertion index.

Center tab insertion displays one retained two-logical-pixel semantic-accent marker at the exact
resolved boundary. The marker is updated in place and cleared on target change, cancellation,
completion, unmount, and other transient cancellation paths; it never participates in layout or
hit testing.

## Auto-hide and floating

Every surface has four private custom-rendered auto-hide strips, a single overlay pane, a custom
pin affordance, and a drop-preview layer. An auto-hide entry opens in the one overlay pane; opening
another entry closes only the previous presentation. Pinning uses the item's runtime preferred side
when one exists, the side of its last root-edge drop or the side it was last unpinned from, and
appends the entry to that side's strip. Otherwise the side follows the item's arranged bounds on its surface: a narrower than
tall item chooses the nearer of Left and Right, any other item the nearer of Top and Bottom. The
pane's pin action docks the item to the root edge of the same side, makes it the active item, and
records that side as preferred. The preferred side is runtime-only state; V2 snapshot shape
is unchanged, and stored return-state fields remain accepted but are not consulted by unpinning.
Visible strips reserve their extent in the surface layout; the surface's main root is arranged
inside them and is never covered by a strip.

Auto-hide strip entries are title-first, content-sized side tabs with sixteen logical pixels between
entries and a four-logical-pixel active/hover marker. Left and Right titles are rotated; Top and
Bottom titles remain horizontal. The strip does not show a document icon. The active marker and text
follow the active/accent theme brush.

Docking's active/accent chrome (active frame and marker, target glyphs, drop preview, auto-hide
marker and pane border) uses the application Theme's `Primary` brush when it resolves to a value.
When `Primary` resolves to `PlatformDefault`, this chrome uses the platform accent color (the
Windows user accent, the macOS control accent) and re-reads it on theme refresh.

An open pane fills the usable center area on its perpendicular axis. Its initial width for Left/Right
or height for Top/Bottom is one third of the available center extent, unless that item has a remembered
extent in the current `DockingControl` runtime. A private resize grip sits on the pane's inner edge;
Right and Bottom resize directions are inverted. The extent is retained by `DockItemId` across
close/reopen and pin/unpin for the lifetime of the owning runtime, and is cleared when that runtime
is destroyed. A pane cannot consume the full surface and make its remaining content unusable.

Clicking a strip entry opens its pane and makes that item active. Clicking outside the pane and strip
on the same surface dismisses its presentation without moving or closing the item; when the item was
active, the model has no active item afterward. The pane's close action closes the item under its
`can_close` capability. Escape dismisses it when the existing input route can deliver the key. Opening another
entry, pinning, closing, or starting a drag dismisses the prior pane through the existing runtime
and model paths. The pane has an active border, rounded outer corner, and a forty-logical-pixel
title header for its Document with pin/close actions and page content below it. The non-action
portion of that header may start a drag for the open Document only.

On macOS and Windows, a floating model root is hosted by a real backend `Window` containing its
retained `DockSurfaceView`. Bounds are the model's normalized logical desktop `Rect`. A new host is
prepared with bounds, content, and a weak close handler before the candidate ownership/model is
committed; it is shown only after that commit. An interactive floating root is 400 by 400 logical
pixels, positioned from the individual drag's pointer offset, with a minimum size of 160 by 120;
only the dragged Document moves into the resulting floating root. A floating-host
failure returns `DockLayoutError::FloatingHostUnavailable` and leaves the source ownership/model
unchanged.

Native close requests are intercepted: any non-closeable contained item vetoes the close; otherwise
all contained items are closed in one model transaction and the host is removed. Host callbacks use
a private stable host identity, so removing an earlier floating root cannot redirect a later Window's
close request.

The current GTK4 baseline has no equivalent usable `Window` surface. Pure model floating snapshots
remain valid there, while an interactive request to create a floating native host reports
`FloatingHostUnavailable`.

## Snapshots and lifetime

`DockLayoutSnapshot::VERSION` is 2. Snapshots contain model state only, including the optional
globally active item; authored controls, capabilities, runtime wrappers, native windows, callbacks,
surface registry state, and remembered auto-hide pane extents are not serialized. Only version-2
snapshots are accepted. Older and
unknown versions are rejected as typed errors; there is no version-1 migration or defaulting path.
Removed authored groups are repaired during normalization: surviving items move to the current
authored default or deterministic root fallback, including closed and auto-hide return states.

Unmount cancels gestures, clears previews, clears native close handlers, closes floating hosts, and
releases the surface registry. Runtime callbacks capture weak owners and do not form retained `Rc`
cycles.
