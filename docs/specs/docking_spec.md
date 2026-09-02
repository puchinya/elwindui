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
capture, cancellation, root-relative position, and optional logical screen position. `CustomSplitter`
supplies splitter gestures. Split nodes with N children realize as one retained Grid with N panes and
N-1 six-pixel `CustomSplitter`s. Splitter movement changes only transient Grid tracks; a successful
completion writes adjacent normalized weights once, while cancellation restores the original tracks.
Selection-only changes and completed adjacent split-weight changes update the retained runtime and
bound value without rebuilding unchanged structural Dock content; live splitter movement only
updates retained tracks and arrange state.

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
retained surface-local overlay layer.

## Auto-hide and floating

Every surface has four private custom-rendered auto-hide strips, a single overlay pane, a custom
pin affordance, and a drop-preview layer. An auto-hide entry opens in the one overlay pane; opening
another entry closes only the previous presentation. Pinning chooses the nearest surface edge with
the deterministic order Left, Top, Right, Bottom. Unpinning restores the remembered group/index,
then the current authored default, then the root fallback.

On macOS and Windows, a floating model root is hosted by a real backend `Window` containing its
retained `DockSurfaceView`. Bounds are the model's normalized logical desktop `Rect`. A new host is
prepared with bounds, content, and a weak close handler before the candidate ownership/model is
committed; it is shown only after that commit. Interactive floating bounds derive from the dragged
group's arranged size and pointer offset, with a minimum size of 160 by 120. A floating-host
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

`DockLayoutSnapshot::VERSION` is 1. Snapshots contain model state only; authored controls,
capabilities, runtime wrappers, native windows, callbacks, and surface registry state are not
serialized. Removed authored groups are repaired during normalization: surviving items move to the
current authored default or deterministic root fallback, including closed and auto-hide return
states.

Unmount cancels gestures, clears previews, clears native close handlers, closes floating hosts, and
releases the surface registry. Runtime callbacks capture weak owners and do not form retained `Rc`
cycles.
