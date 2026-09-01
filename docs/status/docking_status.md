# Docking status

## Implemented

- `elwindui-docking` is a separate consumer crate with stable `DockItemId` and `DockGroupId`
  registrations, authored-default metadata, and dynamic registration callbacks.
- `DockLayoutModel` is an opaque immutable value for main/floating roots, selection, closed return
  state, auto-hide state, generated groups, normalization, and version-1 snapshots.
- `DockingControl` keeps authored declarations mounted but collapsed, installs one retained private
  runtime host, applies the actual TwoWay layout update path, publishes an empty initial default
  once, and suppresses source echoes/latest-only reentrant updates.
- The runtime retains item wrappers, group views, split Grids, and splitter instances. It realizes
  N-pane splits with N-1 `CustomSplitter`s and uses explicit detach-before-attach ownership.
- Structural model application is staged through a private `ReconcilePlan`; native floating hosts
  are prepared outside the committed registry and are inserted/shown only after the runtime/model
  commit. Failed preparation leaves committed wrapper parents, runtime maps, and existing hosts
  unchanged; no old-model rollback is used.
- CustomTabView selection/close/tab-drag callbacks and CustomSplitter callbacks are wired through
  weak Docking owners. Drag preview is a real visual adornment and does not reparent page content.
- Normal tab selection uses a retained value-only path, and live splitter movement updates retained
  Grid tracks with arrange invalidation. CustomTabView caches private presenter references and the
  content presenter measures only the selected page while retaining hidden page ownership.
- Four custom auto-hide strips, icon/title entries, a single wrapper-hosting overlay pane, and pin
  affordances are present per Dock surface.
- Drop discovery carries the selected `RootKind`, target group, and exact surface-local preview
  rectangle together. Surface registration stores floating-root identity, corrects non-zero
  DockSurfaceView offsets, filters group hit testing by root, and keeps only the target surface's
  positioned preview visible.
- AppKit and WinUI3 have private native floating Window adapters with logical bounds, retained
  surface content, stable host IDs, staged prepare/commit creation, close interception, and
  empty-host cleanup. Interactive bounds preserve the source group's arranged size and pointer
  offset, subject to a 160x120 minimum. GTK model floating remains valid but interactive native
  floating reports `FloatingHostUnavailable` because the baseline has no usable Window
  implementation.
- `examples/docking-demo` visibly composes documents, nested tools, and the retained DockingControl
  runtime; it no longer only serializes an empty model.

## Tests and verification state

The focused `elwindui-docking` suite currently has 58 passing tests covering default
initialization/reset, activation, close/reopen, all four split/edge sides, snapshot round-trip,
auto-hide state, typed invalid values, latest-only source logic, removed-authored-group repair,
adjacent split-weight transformation, generated-group drag targets, retained runtime presentation,
callback-driven selection/close, initial publication, dynamic registration, cross-window target
root/geometry and offset conversion, positioned previews, floating source geometry, staged host
failure/success, stable host identity, native close veto/accept, final-root cleanup, actual tab and
splitter pointer paths, and unmount/weak-lifetime cleanup. Native GUI behavior still requires
platform-host verification where noted below.

The canonical command results are recorded in the PR #218 remediation report. Workspace failures
are kept separate from Docking-specific results; in particular, the existing AppKit
`control_template_window_rt4` fixture must not be treated as a Docking failure without change-specific
evidence.

## Platform boundaries

- AppKit runtime interaction: run selection/close, tab drag targets/cancellation, splitter
  completion/cancellation, pin/auto-hide/unpin, floating/re-dock, and native close accept/reject
  when macOS UI permissions are available.
- WinUI3 runtime interaction: run the equivalent matrix only when the separate Issue #207/#217
  Windows integration state permits it; those fixes are not part of #172/#218.
- GTK4: compile as required by the workspace. Do not claim native floating runtime support without a
  real GTK Window implementation.
