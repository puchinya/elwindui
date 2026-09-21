# Docking status

Snapshot: 2026-09-21. Docking behavior is defined by the docking specification and its durable design documents under [`../design/`](../design/).

## Current implementation

- `elwindui-docking` is a separate consumer crate with stable item/group IDs, authored defaults, dynamic registration, immutable `DockLayoutModel` values, version-2 snapshots, active/closed/auto-hide state, normalization, and generated groups.
- `DockingControl` keeps authored declarations collapsed, owns one retained runtime host, publishes the initial default once, suppresses source echoes, and stages structural changes through a private `ReconcilePlan`.
- Retained wrappers, group views, split Grids, CustomGridSplitters, tab presenters, auto-hide strips, side-aware popup panes, drop previews, insertion markers, and explicit detach-before-attach ownership are implemented.
- CustomTabView and CustomGridSplitter callbacks use weak docking owners. Selection, close, tab drag, Grid track movement, whole-group dragging, indexed context actions, capability flags, and empty-group presentation are wired through retained hosts.
- AppKit and WinUI 3 floating hosts use staged prepare/commit creation, stable host IDs, logical bounds, close interception, rejected-close preservation, and empty-host cleanup. GTK model floating remains valid but has no usable native Window implementation.
- Docking chrome uses cached vector geometry and transparent hit-test surfaces; the docking demo composes documents, nested tools, floating windows, auto-hide, and retained DockingControl state.

## Current verification state

- Focused model, reconciliation, retained-presentation, pointer-path, splitter, floating-host, snapshot, auto-hide, weak-lifetime, and workspace tests pass. Final-head AppKit runtime interaction for CustomGridSplitter-owned Docking splitter resizing is verified on the current implementation.
- The backend-neutral native parity case is [`../../tests/e2e/docking-native-parity.md`](../../tests/e2e/docking-native-parity.md). The bounded WinUI3 run for Issue #226 verified rapid selection, reorder, Center and directional group/root docking, main-to-floating docking, context actions, short splitter drags, theme preservation, allowed native close, programmatic removal, and repeated host cleanup. The acceptance is not closed: the fresh outside-bounds item tear-out row remains a native FAIL, auto-hide strip/overlay exposure and continuous three-second splitter tracking are blocked, and several multi-floating, snapshot, capability, veto, and whole-group rows remain unverified.

## Platform boundaries and blockers

- WinUI 3 native Docking acceptance remains open under [#226](https://github.com/puchinya/elwindui/issues/226); the remaining native failures/blockers and unverified rows must be resolved before phase review. GTK4 native floating is unavailable without a usable GTK Window implementation.
- The broader disabled-capability interaction matrix is not a required closure gate and is not represented as a PASS.
