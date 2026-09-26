# Docking status

Snapshot: 2026-09-26. Docking behavior is defined by the docking specification and its durable design documents under [`../design/`](../design/).

## Current implementation

- `elwindui-docking` is a separate consumer crate with stable item/group IDs, authored defaults, dynamic registration, immutable `DockLayoutModel` values, version-2 snapshots, active/closed/auto-hide state, normalization, and generated groups.
- `DockingControl` keeps authored declarations collapsed, owns one retained runtime host, publishes the initial default once, suppresses source echoes, and stages structural changes through a private `ReconcilePlan`.
- Retained wrappers, group views, split Grids, CustomGridSplitters, tab presenters, auto-hide strips, side-aware popup panes, drop previews, insertion markers, and explicit detach-before-attach ownership are implemented.
- CustomTabView and CustomGridSplitter callbacks use weak docking owners. Selection, close, tab drag, Grid track movement, whole-group dragging, indexed context actions, capability flags, and empty-group presentation are wired through retained hosts.
- AppKit and WinUI 3 floating hosts use staged prepare/commit creation, stable host IDs, logical bounds, close interception, rejected-close preservation, and empty-host cleanup. GTK model floating remains valid but has no usable native Window implementation.
- Docking chrome uses cached vector geometry and transparent hit-test surfaces; the docking demo composes documents, nested tools, floating windows, auto-hide, and retained DockingControl state.
- Issue #279 keeps normal retained tab selection on a selection-only publication path: one layout publication and callback, with no DockingControl containing-tree invalidation or full runtime reconciliation. Fast-path qualification reads live model roots without snapshots. Runtime theme refresh is gated by the 11-value BrushStyle signature.

## Current verification state

- Focused model, reconciliation, retained-presentation, pointer-path, splitter, floating-host, snapshot, auto-hide, weak-lifetime, and workspace tests pass.
- Issue #259 AppKit revalidation passes exactly DNP-12, DNP-13, DNP-15, DNP-21, DNP-22, DNP-23, and DNP-24 on tested HEAD `6060a7725ddcae821da58e98576ecb420f5725e2`; DNP-25 passes through those native rows plus `window_lifetime_appkit` and the relevant deterministic `elwindui-docking` tests. The authoritative final manifest is immutable under `.agent-state/issues/259/e2e/6060a77/20260922T103912Z/final-result.md`; earlier incomplete retries remain retained as historical evidence and are superseded by this final run.
- The historical WinUI3 DNP-01..25 matrix for Issue #226 passed on tested HEAD `c589a142208bc46d2c48c29d948ec92719839756` ([final matrix result](https://github.com/puchinya/elwindui/issues/226#issuecomment-5771368668)). This evidence does not verify Issue #279 or PR #281 at its current HEAD. The backend-neutral native parity case is [`../../tests/e2e/docking-native-parity.md`](../../tests/e2e/docking-native-parity.md); the separate AppKit #259 subset is recorded above and does not imply an AppKit DNP-01..25 run.
- Issue #279 regression tests pass for zero reconcile/theme-refresh deltas, one layout callback, stable wrapper parents, and zero unrelated-sibling measure delta. The Docking and CustomTabView test suites pass. Windows DNP-01 for this change is BLOCKED before reset/input: two fresh app instances timed out on the repository tester readiness command `wait-for --selector Default` (`target_error`, `found: false`), so 0/20 clicks were delivered. Immutable evidence: `.agent-state/issues/279/e2e/3875bc73ccd3/20260923T070737Z` and `.agent-state/issues/279/e2e/3875bc73ccd3/20260923T071128Z`. No product result is inferred. macOS/AppKit host verification: deferred to #280.
- After `cargo clean` freed 2.4 GiB, the standard debug-profile `cargo build --workspace` and `cargo test --workspace` both passed without profile overrides; `cargo check --workspace` and rust-analyzer diagnostics also passed. The focused Docking and CustomTabView suites passed. Windows DNP-01 remains blocked before input as recorded above; macOS/AppKit host verification is deferred to #280.

## Platform boundaries and blockers

- WinUI 3 native Docking acceptance rows pass under [#226](https://github.com/puchinya/elwindui/issues/226), including both continuous splitter rows through the approved external fork. AppKit #259 acceptance is limited to DNP-12, DNP-13, DNP-15, DNP-21, DNP-22, DNP-23, DNP-24, and DNP-25 as recorded above; #226 remains the WinUI3 acceptance. GTK4 native floating is unavailable without a usable GTK Window implementation.
- The broader disabled-capability interaction matrix is not a required closure gate and is not represented as a PASS.
