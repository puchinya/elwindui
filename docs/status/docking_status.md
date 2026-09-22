# Docking status

Snapshot: 2026-09-22. Docking behavior is defined by the docking specification and its durable design documents under [`../design/`](../design/).

## Current implementation

- `elwindui-docking` is a separate consumer crate with stable item/group IDs, authored defaults, dynamic registration, immutable `DockLayoutModel` values, version-2 snapshots, active/closed/auto-hide state, normalization, and generated groups.
- `DockingControl` keeps authored declarations collapsed, owns one retained runtime host, publishes the initial default once, suppresses source echoes, and stages structural changes through a private `ReconcilePlan`.
- Retained wrappers, group views, split Grids, CustomGridSplitters, tab presenters, auto-hide strips, side-aware popup panes, drop previews, insertion markers, and explicit detach-before-attach ownership are implemented.
- CustomTabView and CustomGridSplitter callbacks use weak docking owners. Selection, close, tab drag, Grid track movement, whole-group dragging, indexed context actions, capability flags, and empty-group presentation are wired through retained hosts.
- AppKit and WinUI 3 floating hosts use staged prepare/commit creation, stable host IDs, logical bounds, close interception, rejected-close preservation, and empty-host cleanup. GTK model floating remains valid but has no usable native Window implementation.
- Docking chrome uses cached vector geometry and transparent hit-test surfaces; the docking demo composes documents, nested tools, floating windows, auto-hide, and retained DockingControl state.

## Current verification state

- Focused model, reconciliation, retained-presentation, pointer-path, splitter, floating-host, snapshot, auto-hide, weak-lifetime, and workspace tests pass. The current WinUI3 native run passes DNP-01 through DNP-17, DNP-18 short/long, DNP-19 short/long, and DNP-20 through DNP-25. DNP-18/19 long use the pinned `puchinya/winappCli` fork based on v0.6.1 and report 4000 ms movement, 250 steps, and the expected live splitter displacement without a release-time jump.
- The backend-neutral native parity case is [`../../tests/e2e/docking-native-parity.md`](../../tests/e2e/docking-native-parity.md). The WinUI3 DNP matrix is complete for the current implementation; the shared-runtime Docking evidence still requires the corresponding AppKit invalidation rerun before cross-backend parity can be claimed on this Windows host.

## Platform boundaries and blockers

- WinUI 3 native Docking acceptance rows pass under [#226](https://github.com/puchinya/elwindui/issues/226), including both continuous splitter rows through the approved external fork. Final cross-backend delivery remains gated by the required AppKit invalidation rerun and repository review workflow. GTK4 native floating is unavailable without a usable GTK Window implementation.
- The broader disabled-capability interaction matrix is not a required closure gate and is not represented as a PASS.
