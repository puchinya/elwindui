# Docking status

Snapshot: 2026-09-06. Docking behavior is defined by the docking specification and its durable design documents under [`../design/`](../design/).

## Current implementation

- `elwindui-docking` is a separate consumer crate with stable item/group IDs, authored defaults, dynamic registration, immutable `DockLayoutModel` values, version-2 snapshots, active/closed/auto-hide state, normalization, and generated groups.
- `DockingControl` keeps authored declarations collapsed, owns one retained runtime host, publishes the initial default once, suppresses source echoes, and stages structural changes through a private `ReconcilePlan`.
- Retained wrappers, group views, split Grids, CustomSplitters, tab presenters, auto-hide strips, side-aware popup panes, drop previews, insertion markers, and explicit detach-before-attach ownership are implemented.
- CustomTabView and CustomSplitter callbacks use weak docking owners. Selection, close, tab drag, splitter movement, whole-group dragging, indexed context actions, capability flags, and empty-group presentation are wired through retained hosts.
- AppKit and WinUI 3 floating hosts use staged prepare/commit creation, stable host IDs, logical bounds, close interception, rejected-close preservation, and empty-host cleanup. GTK model floating remains valid but has no usable native Window implementation.
- Docking chrome uses cached vector geometry and transparent hit-test surfaces; the docking demo composes documents, nested tools, floating windows, auto-hide, and retained DockingControl state.

## Current verification state

- Focused model, reconciliation, retained-presentation, pointer-path, floating-host, snapshot, auto-hide, and weak-lifetime tests are established, together with the workspace verification baseline.
- AppKit native evidence covers the required selection, split, docking, floating, close, auto-hide, context-action, and cross-host transfer cases. Reviewer-visible captures are maintained under [`../issues/220-docking-ux-parity/evidence/`](../issues/220-docking-ux-parity/evidence/).

## Platform boundaries and blockers

- WinUI 3 native Docking acceptance is deferred to a Windows follow-up. GTK4 native floating is unavailable without a usable GTK Window implementation.
- The broader disabled-capability interaction matrix is not a required closure gate and is not represented as a PASS.
