# Docking status

Snapshot: 2026-10-04. Desired behavior is defined by the docking specification; architecture is defined by the durable design documents under [design](../design/).

## Current implementation

- elwindui-docking is a separate consumer crate with stable item/group IDs, authored defaults, dynamic registration, immutable DockLayoutModel values, version-2 snapshots, active/closed/auto-hide state, normalization, and generated groups.
- DockingControl retains one runtime host, publishes the initial default once, suppresses source echoes, and stages structural changes through a private ReconcilePlan.
- Retained wrappers, group views, split pane Grids, DockSplitView splitter overlays, tab presenters, auto-hide strips, side-aware popup panes, drop previews, and insertion markers are implemented with explicit detach-before-attach ownership. Core Grid spacing defaults to zero; Docking split Grids use 12 logical pixels on their split axis, and splitter hit targets occupy that gutter without adding a pane track.
- CustomTabView and CustomGridSplitter callbacks use weak docking owners. Selection, close, individual Document tab drag, Grid track movement, indexed context actions, capability flags, and empty-group presentation are wired through retained hosts. Docking has no group-level drag, tear-out, or cross-dock gesture; blank Top tab-strip space is not a drag handle.
- Document selection and activation remain separate. A selected tab with no DockLayoutModel active item has no active group frame or tab marker; explicit activation displays the active chrome.
- CustomTabStripPresenter derives widths from each layout pass, reuses matching measure results, fits compact headers into the finite strip, and clips header content to its own tab bounds. Unchanged TextBlock measurements reuse backend text metrics.
- AppKit and WinUI 3 floating hosts use staged prepare/commit creation, stable host IDs, logical bounds, close interception, rejected-close preservation, and empty-host cleanup. GTK model floating is valid but has no usable native Window implementation.
- Docking chrome uses cached vector geometry and transparent hit-test surfaces. The demo includes documents, nested tools, floating-window controls, auto-hide, and retained DockingControl state.
- The #279 retained-selection fast path keeps selection-only publication out of full Docking reconciliation; theme refresh is gated by the BrushStyle signature.
- Selected-page arrangement reuse checks the participating subtree's arrangement validity, including after splitter completion. WinUI Composition reconnects recreated image visuals even when node IDs and ordering remain unchanged.
- WinUI.Dock behavior alignment (2026-10-04): drop targets resolve only on drawn compass cells and root-edge targets (tab-header insertion/reorder kept); other releases float a 400 x 400 window; root Dock previews half the surface; committed drops activate the moved Document; the compass centers on the whole group frame. Auto-hide pins by preferred side or the reference shape rule, pane pin docks to the same-side root edge, dismissal releases activity, and visible strips reserve layout space. Accent chrome falls back to the platform accent when Theme `Primary` is unset. CustomGridSplitter follows the CommunityToolkit Sizers state fills.

## Current verification

- The canonical Rust formatter/analyzer gate passes: zero errors, warnings or non-exempt weak warnings; 284 intentional cfg-only inactive-code records. Workspace build and tests pass (1128 passed, 0 failed, 3 ignored).
- Normal non-elevated Windows host paired runs against the pinned WinUI.Dock example pass WDF-02/03/05–10/15/16 (light and dark); WDF-01/04/11–14 remain partially executed. Drag move handling streams at about 15 ms per move after the overlay first appears; a one-time ~0.5 s hitch on first overlay appearance remains.
- Quiet debug startup previously measured 1348–1410 ms from process start to the first content Rendering callback, accepted by the user. A comparable actual process-start measurement for the current build has not been taken.

## Platform boundaries

- WinUI 3 visual acceptance for #285 is partial: the WDF rows listed as NOT RUN above have not been executed.
- AppKit visual equivalence has not been claimed. GTK4 native floating is unavailable without a usable GTK Window implementation.
