# Custom controls status

Snapshot: 2026-08-24. The public contract is
[`../specs/custom_controls_spec.md`](../specs/custom_controls_spec.md).

## Implemented

- `elwindui-custom-controls` is a separate workspace crate depending only on
  Core and macros.
- `CustomTabView`, `CustomTabViewItem`, and `CustomSplitter` use
  `#[elwindui::component]` with the required Control/ContentControl
  inheritance; no new custom `#[class]` declaration was introduced.
- The controls are templated: authored `body: view!` subtrees use Grid,
  HorizontalLayout, Rectangle, TextBlock, and IconSourceElement. CustomTabView
  and CustomSplitter do not draw chrome through `render()`.
- CustomTabView owns the typed ordered item list and TwoWay selection. Private
  strip/content presenters preserve item identity and keep all current logical
  contents visually attached while selection changes.
- Header text, optional icons, selected indicator, and close affordance are
  ordinary template visuals. The close helper uses a fixed 20-pixel slot and
  Core implicit pointer capture; no SystemIcon geometry or direct X drawing is
  duplicated here.
- Close capability/presentation, tab drag cancellation/reentrancy, splitter
  axis/delta semantics, weak callbacks, content replacement/removal, and Core
  IconSourceElement realization are implemented.
- The generic component override bridge from [#185](https://github.com/puchinya/elwindui/issues/185)
  is merged. Host-path tests exercise `layout_root`, `RenderTree`, routed input,
  and PointerDispatcher implicit capture; no ignored test remains for the old
  override or direct-render architectures.

## Verification

The focused custom-controls suite passes with 26 tests and no ignored tests.
Core, codegen, AppKit-enabled facade, inheritance-demo, workspace build/check,
and workspace test results are recorded in the PR completion report against
the final head. Workspace-wide formatter or rust-analyzer diagnostics are
reported separately when their baseline macro/environment diagnostics remain.
Windows and GTK4 runtime interaction have not been run. Interactive AppKit
verification remains unverified unless explicitly listed in the final report.

## Follow-up

- Docking integration remains Issue #172 and stays a downstream crate.
- Common pointer cancellation/capture-loss semantics remain owned by Issue
  #179/PR #181; this crate consumes the Core cancellation events but does not
  add a capture API.
