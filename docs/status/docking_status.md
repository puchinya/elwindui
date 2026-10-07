# Docking status

Snapshot: 2026-10-07. Desired behavior is defined by the docking specification; architecture is defined by the durable design documents under [design](../design/).

## Current implementation

- elwindui-docking is a separate consumer crate with stable item/group IDs, authored defaults, dynamic registration, immutable DockLayoutModel values, version-2 snapshots, active/closed/auto-hide state, normalization, and generated groups.
- DockingControl retains one runtime host, publishes the initial default once, suppresses source echoes, and stages structural changes through a private ReconcilePlan.
- Retained wrappers, group views, split pane Grids, DockSplitView splitter overlays, tab presenters, auto-hide strips, side-aware popup panes, drop previews, and insertion markers are implemented with explicit detach-before-attach ownership. Core Grid spacing defaults to zero; Docking split Grids use 12 logical pixels on their split axis, and splitter hit targets occupy that gutter without adding a pane track.
- CustomTabView and CustomGridSplitter callbacks use weak docking owners. Selection, close, individual Document tab drag, Grid track movement, indexed context actions, capability flags, and empty-group presentation are wired through retained hosts. Docking has no group-level drag, tear-out, or cross-dock gesture; blank Top tab-strip space is not a drag handle.
- Document selection and activation remain separate. A selected tab with no DockLayoutModel active item has no active group frame or tab marker; explicit activation displays the active chrome.
- CustomTabStripPresenter measures each header once in Measure, reads compact natural widths from the retained measurement, reuses them across strip-width-only changes, and never measures in Arrange; it fits compact headers into the finite strip and clips header content to its own tab bounds. The group active frame is painted by its overlay without Measure invalidation, and auto-hide titles are measured once by normal layout. Unchanged TextBlock measurements reuse backend text metrics.
- AppKit and WinUI 3 floating hosts use staged prepare/commit creation, stable host IDs, logical bounds, close interception, rejected-close preservation, and empty-host cleanup. GTK model floating is valid but has no usable native Window implementation.
- Docking chrome uses cached vector geometry and transparent hit-test surfaces. The demo includes documents, nested tools, floating-window controls, auto-hide, and retained DockingControl state.
- The #279 retained-selection fast path keeps selection-only publication out of full Docking reconciliation; theme refresh is gated by the BrushStyle signature.
- Selected-page arrangement reuse checks the participating subtree's arrangement validity, including after splitter completion. WinUI Composition reconnects recreated image visuals even when node IDs and ordering remain unchanged.
- Active selected tabs use the accent outline and an open content-frame contour; retained header text changes notify the host before reconciliation detaches the tree. Auto-hide extents are shared across surfaces within one runtime and are remembered on resizing, dismissal, replacement and teardown.
- WinUI.Dock behavior alignment (2026-10-04): drop targets resolve only on drawn compass cells and root-edge targets (tab-header insertion/reorder kept); other releases float a 400 x 400 window; root Dock previews half the surface; committed drops activate the moved Document; the compass centers on the whole group frame. Auto-hide pins by preferred side or the reference shape rule, pane pin docks to the same-side root edge, dismissal releases activity, and visible strips reserve layout space. Accent chrome falls back to the platform accent when Theme `Primary` is unset. CustomGridSplitter follows the CommunityToolkit Sizers state fills.

## Current verification

- The canonical Rust gate passes: formatting, analyzer diagnostics (0 errors/warnings/non-exempt weak warnings; 289 intentional cfg-only inactive records), workspace check/build, and workspace tests (1145 passed, 0 failed, 3 ignored). Custom Controls passes 89 tests and Docking passes 123.
- Fresh normal non-elevated Windows comparisons pass the dark selected-tab outline and hover states, normal/compact narrow widths, and all four auto-hide resize/reopen cases. Updated paired splitter/header acceptance and the light-theme pass (initial layout, split drop with the joined accent frame, auto-hide pane) also pass; detailed results live in the WDF matrix and Issue evidence.
- Quiet debug startup (process start to first content Rendering, line-tables-only, startup trace only) measures 1482–1641 ms (median 1538) over five launches of the latest executable, against 1538–2714 ms (median 1651) for the pre-remediation build on the same host. Backend `measure_text` calls up to first content rendering fell from 44 to 40 per launch (three traced launches each); the review target of a 30% reduction is not met.

## Platform boundaries

- WinUI 3 acceptance covers all WDF rows on the pinned example, including Escape dismissal from a focused field inside an open auto-hide pane.
- AppKit visual equivalence has not been claimed. GTK4 native floating is unavailable without a usable GTK Window implementation.
