# Docking status

## Implemented

- `elwindui-docking` is a separate consumer crate with stable `DockItemId` and `DockGroupId`
  registrations, authored-default metadata, and dynamic registration callbacks.
- `DockLayoutModel` is an opaque immutable value for main/floating roots, global active selection,
  closed return state, auto-hide state, generated groups, normalization, and version-2 snapshots;
  version-1 input is rejected without migration or defaulting.
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
- Whole-group title-bar dragging, indexed tab context actions, compact tab metrics, and authored
  `show_when_empty` groups with a non-interactive `Drop here` hint are wired through retained
  runtime hosts.
- Normal tab selection uses a retained value-only path, and live splitter movement updates retained
  Grid tracks with arrange invalidation. CustomTabView caches private presenter references and the
  content presenter measures only the selected page while retaining hidden page ownership.
- Four custom auto-hide strips, icon/title entries, a single wrapper-hosting overlay pane, and pin
  affordances are present per Dock surface.
- Drop discovery carries the selected `RootKind`, target group, and exact surface-local preview
  rectangle together. Surface registration stores floating-root identity, corrects non-zero
  `DockSurfaceView` offsets, filters group hit testing by root, and keeps only the target surface's
  positioned preview visible. The retained five-button group compass and four-button root Dock set
  are visually distinct and never alias highlights; both are non-hit-testable. Center tab insertion
  uses actual retained arranged header midpoints, carries `tab_insert_index` in the resolved target,
  and paints one retained two-logical-pixel semantic-accent insertion marker without preview
  reconciliation or page measurement.
- AppKit and WinUI3 have private native floating Window adapters with logical bounds, retained
  surface content, stable host IDs, staged prepare/commit creation, close interception, and
  empty-host cleanup. A native close callback marks the host as closing, hands the original OS
  close back without reentrant `close()`, and commits the model on the following UI turn; rejected
  closes keep both the model and native host intact. Interactive bounds preserve the source
  group's arranged size and pointer offset, subject to a 160x120 minimum. GTK model floating
  remains valid but interactive native floating reports `FloatingHostUnavailable` because the
  baseline has no usable Window implementation.
- Docking chrome close, pin, and floating affordances use a shared cached 16x16 vector geometry with
  round caps/joins and centered hit-test-transparent presentation. Their hosting title-bar and
  auto-hide button surfaces use an alpha-zero brush, preserving the full hit area without a
  contrasting rectangle. The former 3x3 rectangle mosaics are no longer used.
- `examples/docking-demo` visibly composes documents, nested tools, and the retained DockingControl
  runtime; it exposes active-item, floating-window-count, and latest-layout status values, plus
  authored `can_close=false`, `can_float=false`, and `can_dock=false` capability examples.

## Tests and verification state

The focused `elwindui-docking` suite currently has 84 passing tests covering default
initialization/reset, activation, close/reopen, all four split/edge sides, snapshot round-trip,
auto-hide state, typed invalid values, latest-only source logic, removed-authored-group repair,
adjacent split-weight transformation, generated-group drag targets, retained runtime presentation,
callback-driven selection/close, initial publication, dynamic registration, cross-window target
root/geometry and offset conversion, positioned previews, floating source geometry, staged host
failure/success, stable host identity, native close veto/accept, final-root cleanup, actual tab and
splitter pointer paths, prepare/commit ordering, and unmount/weak-lifetime cleanup, including V2
active-item round-trip, V1 rejection, clear/reset, whole-group movement, empty-group presentation,
indexed context-close actions, exact root/group target geometry, and resolved center insertion.
The focused `elwindui-custom-controls` suite has 9 library tests and 38 integration tests, including
the 24-tab retained-selection operation budget, structural-selection counters, hidden-page measure
checks, and compact presentation. Native GUI behavior still requires platform-host verification
where noted below.

The current workspace verification is `1024 passed, 3 ignored`; `cargo check --workspace`, the
`rust_analyzer`-cfg check, and the repository rust-analyzer diagnostic gate all pass. The latter
reports only intentional `inactive-code` WeakWarnings (222 records, zero actionable diagnostics).

## Platform boundaries

- AppKit runtime interaction: the rebuilt real `docking-demo` rendered successfully and its captured
  GUI shows the retained Docking layout, live status row, and alpha-transparent chrome surfaces.
  The required interactive rerun is BLOCKED: `macos-ui-driver focus-window` could not confirm that
  the demo became frontmost (`frontmost_application_name` remained outside the demo), so native
  pointer, floating move/resize, context, auto-hide, and long-duration splitter results are not
  claimed. The prior observed native PASS items remain historical evidence only; they do not replace
  the blocked final matrix.
- WinUI3 runtime interaction: Issues #207 and #217 are closed and current master includes the
  Windows gate recovery. No usable Windows GUI host was available in this macOS run, so canonical
  WinUI3 build/native Docking execution was not run; compile-only evidence is not a native PASS.
- GTK4: compile as required by the workspace. Do not claim native floating runtime support without a
  real GTK Window implementation.
