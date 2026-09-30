# Docking specification

`elwindui-docking` is a backend-neutral docking surface. It is a separate crate from the
`elwindui` facade and uses the existing Core layout, input, ContentControl, Window, and
custom-control contracts.

## Authored declarations

Applications author one `DockingControl` whose content is a `DockGroup` or `DockSplitPanel` tree.
`DockGroup` registers a stable `DockGroupId`, tab-strip position, and authored weight. `DockItem`
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
rollback, and resize notifications. Split nodes with N children realize as one retained Grid with N
Star pane tracks and N-1 twelve-pixel `CustomGridSplitter`s. Horizontal splits set
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
item, selection alone does not show the active frame or marker. Compact measurement includes the
marker when active and the title so the title remains legible. The active marker is not drawn over
page content.
Top-tab pin/close affordances appear on pointer hover; bottom-tab actions belong to the content
header.
All Docking drag sources represent one Document. A tab drag may reorder that Document, move it to a
group/root, split a target group, or float it, subject to the item's capability checks. Docking has
no group-level drag, tear-out, or cross-dock operation. Bottom content-header actions and drag
operate on the selected Document and never move its containing group. A floating native window
continues to move through its platform title bar. Supported tab context actions are `Close`,
`Close Others`, `Close Tabs to Left`, `Close Tabs to Right`, `Float`, and `Auto Hide / Pin`; each
action uses the same capability checks and one model transaction as its pointer equivalent.

An authored empty group remains visible only when `show_when_empty` is true. It keeps normal group
chrome and displays a centered, non-interactive `Drop here` hint while remaining a valid drop target.
`compact_tabs` selects the compact tab metrics for that group and defaults to `true`, matching
WinUI.Dock's compact `TabView` headers. Explicitly setting it to `false` distributes available
header width up to the 200-pixel cap. Clear/reset operations remove the live presentation without
consulting `can_close`, preserve the authored declaration, and restore the authored default
deterministically.

Drag movement changes only a custom drop-preview rectangle and candidate target. It never reparents
page content or reconciles a preview model. Completion commits one normalized model, or cancels when
there is no valid target. The private resolved target retains the destination root, target group,
and surface-local preview rectangle as one value, so preview, hit testing, and commit cannot diverge.
Outer surface bands provide four Dock targets; the deepest containing runtime group provides Center
or four Split targets. Cross-window discovery uses only Core `screen_to_root`/`root_to_screen`
conversions and arranged visual bounds, converting screen coordinates to host-root and then
surface-local coordinates by subtracting the surface origin. Without a screen position, only the
source surface is eligible.

Preview geometry is the complete target group for Center, the corresponding half for Split, and the
corresponding quarter of the surface for an outer Dock target. The rectangle is arranged by a
retained surface-local overlay layer. Its visual uses the active/accent fill, default separator
border, four-logical-pixel border thickness, four-logical-pixel-equivalent corner radius, and 0.4
opacity. It remains non-hit-testable; this chrome does not change the resolved geometry.

The five group targets are a retained 124-logical-pixel connected cross compass
(`SplitTop`, `SplitLeft`, `Center`, `SplitRight`, `SplitBottom`) with five 36-logical-pixel target
cells. Each 36-pixel visual has a 4-pixel inset, default fill and separator stroke, and a document
glyph with active/accent stroke. Center shows a whole document; Split shows a whole document with a
dashed midline in the split direction; Dock shows a half document and direction indicator. The
connected compass backing has a 2-pixel outer inset and joins the five cells as one rounded, stroked
cross. Target selection is conveyed by the preview rectangle, not a large selected-color cell fill.
Target visuals do not own input or resolve a drop. The compass is centered on the arranged bounds of
the resolved target group, including nested and floating groups, rather than on the surface.

The four root-edge targets (`DockLeft`, `DockTop`, `DockRight`, `DockBottom`) are a separate,
surface-relative retained target set; a root-edge target never aliases or highlights its similarly
oriented group Split target. Both sets are non-hit-testable and the source drag coordinator remains
the only input authority.

For an individual Document drag, a pointer inside the target group's arranged tab-header rectangle
takes precedence over the group's compass split bands. This makes the actual header midpoint
available for Center insertion while the same document drag resolves compass targets elsewhere in
the group.

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
another entry closes only the previous presentation. Pinning chooses the nearest surface edge with
the deterministic order Left, Top, Right, Bottom. Unpinning restores the remembered group/index,
then the current authored default, then the root fallback.

Auto-hide strip entries are title-first, content-sized side tabs with sixteen logical pixels between
entries and a four-logical-pixel active/hover marker. Left and Right titles are rotated; Top and
Bottom titles remain horizontal. The strip does not show a document icon. The active marker and text
follow the active/accent theme brush.

An open pane fills the usable center area on its perpendicular axis. Its initial width for Left/Right
or height for Top/Bottom is one third of the available center extent, unless that item has a remembered
extent in the current `DockingControl` runtime. A private resize grip sits on the pane's inner edge;
Right and Bottom resize directions are inverted. The extent is retained by `DockItemId` across
close/reopen and pin/unpin for the lifetime of the owning runtime, and is cleared when that runtime
is destroyed. A pane cannot consume the full surface and make its remaining content unusable.

Clicking outside the pane and strip on the same surface dismisses its presentation without changing
the item. Escape dismisses it when the existing input route can deliver the key. Opening another
entry, pinning, closing, or starting a drag dismisses the prior pane through the existing runtime
and model paths. The pane has an active border, rounded outer corner, and a forty-logical-pixel
title header for its Document with pin/close actions and page content below it. The non-action
portion of that header may start a drag for the open Document only.

On macOS and Windows, a floating model root is hosted by a real backend `Window` containing its
retained `DockSurfaceView`. Bounds are the model's normalized logical desktop `Rect`. A new host is
prepared with bounds, content, and a weak close handler before the candidate ownership/model is
committed; it is shown only after that commit. Interactive floating bounds derive from the source
group's arranged size and the individual drag's pointer offset, with a minimum size of 160 by 120;
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
