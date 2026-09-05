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

The current workspace verification is `cargo test --workspace`: 82 suites, 1024 passed, 0 failed,
3 ignored. `cargo check --workspace` passes, `cargo fmt --all -- --check` passes, and the repository
rust-analyzer diagnostic gate passes with 222 intentional `Ra("inactive-code", WeakWarning)` records
and zero actionable diagnostics. `git diff --check` also passes. Cargo reports the repository's
existing future-incompatibility warning; it is not a new test failure.

## Platform boundaries

- AppKit runtime interaction: the elevated `macos-ui-driver doctor` check reports
  `accessibility=true` and `screen_recording=true`. A real foreground `docking-demo` session has
  confirmed initial render, rapid selection, same-group reorder, top/bottom/left/right group
  docking, cross-group tab docking, insertion-preview lifecycle, both splitter axes, V2 snapshot
  save/reset/restore, auto-hide pin transition, light/dark theme switching, item and whole-group
  tear-out, native floating move/resize, floating-to-main return, native floating close with
  process survival, and the Docking context-menu capability states. The context-menu Float and
  Close actions were re-run after retaining the menu-item wrappers: Float created a native
  `Document B` window and Close removed it from the tabs while the process stayed alive. The
  transparent chrome/icon backgrounds were also visually confirmed.
- AppKit GUI evidence is stored under `/private/tmp/pr221-e2e-logs/`; copied command stdout/stderr
  is retained under `.agent-state/issues/220/logs/`. Abnormal cases are reported with both streams,
  including the pre-fix inert context action, the selector-ambiguity diagnostic, and intermittent
  direct-driver permission-denied shell invocations. V1 snapshot compatibility is not required;
  only the V2 save/reset/restore path is part of this acceptance.
- Still not proven in this session: multiple simultaneous floating windows with independent
  interaction, auto-hide popup/unpin, and a full matrix of every context-menu action beyond the
  verified Float/Close paths. These remain NOT RUN rather than inferred from compilation or a
  screenshot. WinUI3 native Docking acceptance is DEFERRED to a separate follow-up Issue; GTK4
  native floating remains unavailable without a usable GTK Window implementation.
