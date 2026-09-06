# Custom controls status

Snapshot: 2026-09-06. The public contract is [`../specs/custom_controls_spec.md`](../specs/custom_controls_spec.md); durable design is indexed in [`../design/README.md`](../design/README.md).

## Current implementation

- `elwindui-custom-controls` is a separate consumer crate with Core and macro runtime dependencies. `CustomTabView`, `CustomTabViewItem`, and `CustomGridSplitter` are typed `#[elwindui::component]` controls with the required Control/ContentControl inheritance.
- Templates use Grid, HorizontalLayout, Rectangle, TextBlock, and IconSourceElement. Chrome is composed through ordinary template visuals rather than duplicated direct rendering; CustomGridSplitter presents Fluent-style neutral, hover/focus, and pressed states while preserving its six-logical-pixel natural surface.
- CustomTabView owns typed ordered items and TwoWay selection. Retained presenters preserve item identity and keep logical contents attached while selection changes; the inherited `ContentControl::content` remains the logical page slot.
- Close presentation, tab drag cancellation/reentrancy, Grid-aware splitter resizing, weak callbacks, content replacement/removal, pointer capture, and IconSourceElement realization are implemented.
- The component override bridge, source-local module topology, explicit public type exports, weak-owner callback lifetime, paint-only close-glyph updates, and cross-crate generated shape forwarding are implemented.
- `examples/custom-controls-demo` provides an AppKit sample covering tab selection, advisory close requests, tab dragging, and splitter-driven pane resizing.
- Docking integration is implemented in the separate `elwindui-docking` crate; this crate remains the reusable custom-control layer.
- Custom controls consume the common Core pointer cancellation and capture-loss semantics; this crate does not add a separate public capture API.

## Current gaps and follow-up

- Native runtime interaction for CustomTabView, CustomTabViewItem, and CustomGridSplitter is not established for every target backend.

## Verification state

Focused custom-control, external declarative-content, formatter, workspace build/check/test, and warning-regression verification passes on the current implementation. Final-head AppKit runtime interaction for CustomGridSplitter and Docking splitter resizing is verified on the current implementation. WinUI3 and GTK4 runtime interaction remain unverified.
