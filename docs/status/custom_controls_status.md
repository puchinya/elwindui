# Custom controls status

Snapshot: 2026-09-06. The public contract is [`../specs/custom_controls_spec.md`](../specs/custom_controls_spec.md); durable design is indexed in [`../design/README.md`](../design/README.md).

## Current implementation

- `elwindui-custom-controls` is a separate consumer crate with Core and macro runtime dependencies. `CustomTabView`, `CustomTabViewItem`, and `CustomSplitter` are typed `#[elwindui::component]` controls with the required Control/ContentControl inheritance.
- Templates use Grid, HorizontalLayout, Rectangle, TextBlock, and IconSourceElement. Chrome is composed through ordinary template visuals rather than duplicated direct rendering.
- CustomTabView owns typed ordered items and TwoWay selection. Retained presenters preserve item identity and keep logical contents attached while selection changes; the inherited `ContentControl::content` remains the logical page slot.
- Close presentation, tab drag cancellation/reentrancy, splitter axis/delta semantics, weak callbacks, content replacement/removal, pointer capture, and IconSourceElement realization are implemented.
- The component override bridge, source-local module topology, explicit public type exports, weak-owner callback lifetime, paint-only close-glyph updates, and cross-crate generated shape forwarding are implemented.
- `examples/custom-controls-demo` provides an AppKit sample covering tab selection, advisory close requests, tab dragging, and splitter-driven pane resizing.

## Current gaps and follow-up

- Native runtime interaction for CustomTabView, CustomTabViewItem, and CustomSplitter is not established for every target backend.
- Docking integration is implemented in the separate `elwindui-docking` crate; this crate remains the reusable custom-control layer.
- Custom controls consume the common Core pointer cancellation and capture-loss semantics; this crate does not add a separate public capture API.

## Verification state

Focused custom-control, external declarative-content, formatter, workspace build/check, and warning-regression verification is established on the current repository baseline. AppKit smoke evidence exists through `tools/macos-ui-driver`; Windows and GTK4 runtime interaction remain unverified.
