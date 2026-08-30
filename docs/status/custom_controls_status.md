# Custom controls status

Snapshot: 2026-08-31. Review-remediation implementation commit:
`623a3c0`. The public contract is
[`../specs/custom_controls_spec.md`](../specs/custom_controls_spec.md).

## Implemented

- `elwindui-custom-controls` is a separate workspace crate whose runtime
  dependencies are only Core and macros; its external-consumer regression
  uses the top-level `elwindui` facade as a dev-dependency.
- `CustomTabView`, `CustomTabViewItem`, and `CustomSplitter` use
  `#[elwindui::component]` with the required Control/ContentControl
  inheritance; no new custom `#[class]` declaration was introduced.
- The controls are templated with typed `template_view!(|alias: Self| { ... })`
  subtrees use Grid, HorizontalLayout, Rectangle, TextBlock, and
  IconSourceElement. CustomTabView and CustomSplitter do not draw chrome
  through `render()`.
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
- The inherited `ContentControl::content` remains the single logical page slot,
  while the authored `template_view!` root owns the tab header presentation.
  The two ownership paths use the shared typed component-template path delivered
  by [#188](https://github.com/puchinya/elwindui/pull/188).
- The component source topology is explicit: `lib.rs` is a facade/module root,
  each component has its own source file, shared event/value types are in
  `types.rs`, and cross-cutting weak-owner support is in `support.rs`.
- Weak callback owners derive a temporary typed `Rc<T>` from the Visual owner,
  downgrade it, and drop that temporary strong reference normally. No leaked
  strong reference, raw-pointer registry, or strong callback is used.
- `TextStyleOwner::Foreground` is paint-only (`Render` invalidation). The close
  glyph is a constant `TextBlock` `×`; hover toggles only its transparent/cleared
  foreground, while `Never` changes the slot visibility and may invalidate layout.
- The generated class shape forwards own `#[prop]` and `#[content]` metadata
  across the crate boundary, so a real declarative `view!` can construct
  `CustomTabView { CustomTabViewItem { ... TextBlock { ... } } }` while
  preserving the authored item/content `Rc` identities and logical parent.
- `examples/custom-controls-demo` provides an interactive AppKit sample whose
  window, layout, page content, and custom-control nodes are composed with
  `view!`; `on_mount` supplies the external custom-control property setup and
  callback wiring. It covers tab selection, advisory close requests, tab
  dragging, and splitter-driven pane resizing.

## Verification

The focused custom-controls suite passes with 35 tests and no ignored tests.
`cargo fmt --all` and its check pass. `RUSTFLAGS="--cfg rust_analyzer" cargo
check --workspace`, `cargo check --workspace`, `cargo build --workspace`,
`cargo check -p custom-controls-demo`, `cargo test --workspace --quiet`, and
`git diff --check` pass with the remediation changes. The actual command
`rust-analyzer diagnostics .` reports zero `Error`, zero `Warning`, and zero
non-exempt `WeakWarning` diagnostics, with 132 permitted
`Ra("inactive-code", WeakWarning)` records for intentional `#[cfg(...)]`
branches, but exits 1 with the generic diagnostics failure status. Per the
verification contract this is not a PASS and remains the outstanding gate.
Windows and GTK4 runtime interaction have not been run. The existing
interactive AppKit smoke evidence was captured with `tools/macos-ui-driver`;
individual pointer behavior remains covered by the Core PointerDispatcher
host-path tests.

## Follow-up

- Docking integration remains Issue #172 and stays a downstream crate.
- Common pointer cancellation/capture-loss semantics remain owned by Issue
  #179/PR #181; this crate consumes the Core cancellation events but does not
  add a capture API.
