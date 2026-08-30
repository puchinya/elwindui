# Custom controls status

Snapshot: 2026-08-31. Final verified remediation implementation commit:
`f31ad651f2e4fd9b7c89a7e90f89baad294c3b63`. The public contract is
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
  each component has its own source file, `types.rs` contains only public
  shared value/event types and aliases, and component/presenter-private state
  is colocated with its owner (`TabGestureKind`, `TabGesture`, and
  `TabItemPointerEvent` in `custom_tab_view.rs`; `ContentEntry` in
  `custom_tab_content_presenter.rs`; `SplitterGesture` in
  `custom_splitter.rs`). Cross-cutting weak-owner support remains in
  `support.rs`.
- The crate-root `types` export is explicit. There is no `pub use types::*`,
  and the five implementation types above are not crate-root API. The owning
  component modules remain private; their generated component metadata needs
  the declarations to be nameable inside those private modules, but no
  external import path is exposed.
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

The focused custom-controls suite passes with 35 tests and no ignored tests;
the external `declarative_content` regression passes with 1 test. `cargo fmt
--all`, `cargo fmt --all -- --check`, and `git diff --check` pass.
`cargo check -p custom-controls-demo`, `cargo check --workspace`, and `cargo
build --workspace` exit 0, but they emit respectively 5, 22, and 22 compiler
warnings. All remaining warnings are `unreachable statement` diagnostics
originating in the component proc-macro expansion; 17 of the workspace
locations are already present on clean `origin/master` and 5 are the custom
controls' corresponding expansions. The `SystemIcon::ALL` warning was removed
by keeping that test-only constant under `cfg(test)`. The warning-free Issue
acceptance is therefore blocked; no warning suppression or codegen change was
introduced.

The exact `rust-analyzer diagnostics .` command exits 0 on the final
implementation snapshot with 0 `Error`, 0 `Warning`, 133 permitted
`Ra("inactive-code", WeakWarning)` records, and 0 non-exempt
`WeakWarning` records. Against clean `origin/master`
(`f2412f7ea807e66d780be57480c5be86453f07e6`) in the same environment it also
exits 0 with 0 `Error`, 0 `Warning`, 125 permitted inactive-code
`WeakWarning` records, and 0 non-exempt records. The rust-analyzer gate is
resolved. `cargo test --workspace --quiet` passes; its test-target compilation
still reports the same compiler warning family. Windows and GTK4 runtime
interaction have not been run. The existing interactive AppKit smoke evidence
was captured with `tools/macos-ui-driver`; individual pointer behavior remains
covered by the Core PointerDispatcher host-path tests.

## Follow-up

- Docking integration remains Issue #172 and stays a downstream crate.
- Common pointer cancellation/capture-loss semantics remain owned by Issue
  #179/PR #181; this crate consumes the Core cancellation events but does not
  add a capture API.
