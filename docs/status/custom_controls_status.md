# Custom controls status

Snapshot: 2026-08-24. The public contract is
[`../specs/custom_controls_spec.md`](../specs/custom_controls_spec.md).

## Implemented

- `elwindui-custom-controls` is a separate workspace crate depending only on
  Core and macros.
- `CustomTabView`, `CustomTabViewItem`, and `CustomSplitter` use
  `#[elwindui::component]` with the required Control/ContentControl
  inheritance; no new custom `#[class]` declaration was introduced.
- `CustomTabView.children()` exposes the established
  `ListExt<dyn CustomTabViewItemExt>` surface, with `set_children` as the
  concrete replacement convenience path.
- Typed item ownership, source-vs-user selection paths, close capability and
  presentation state, 4-pixel tab drag cancellation, splitter axis/delta
  semantics, weak callbacks, and Core `IconSourceElement` realization are
  implemented.
- Close presses are resolved before header selection, hover transitions use
  equality-guarded paint invalidation, drag-start callbacks re-resolve item
  identity/index, and cancellation reconciliation restarts after reentrant
  child replacement.
- Core-only tests cover ownership, metadata/icon realization, selection and
  callback behavior, tab gestures, cancellation, reentrant drag mutations,
  splitter gestures, host layout/render dispatch, and PointerDispatcher
  implicit capture outside the original control bounds.
- The generic component override bridge from [#185](https://github.com/puchinya/elwindui/issues/185)
  is merged. Custom control layout, render, and hit-test behavior is now
  exercised through the normal `layout_root`/`RenderTree`/`UIElementExt` paths;
  no ignored C-class render test remains.

## Verification

The focused crate test command passes with 21 tests and no ignored tests. Core,
codegen, AppKit-enabled facade check, inheritance-demo check, workspace
check/build, and `git diff --check` also pass. Workspace-wide
`cargo fmt --all -- --check` still reports pre-existing formatting differences
outside this crate; the changed Rust test file passes a direct rustfmt check.
The full AppKit facade suite, `control_template` integration test, and
workspace test suite start successfully but may time out in the current GUI
test environment; any timeout is reported against the exact command rather
than treated as a pass. `rust-analyzer diagnostics .` remains a baseline-wide
nonzero command (the clean master archive has the same class of
macro/import diagnostics). AppKit interactive, WinUI3, and GTK4 runtime
interaction have not been run; Windows verification remains outside #173 in
#178/#180.

## Follow-up

- Docking integration remains Issue #172 and must stay a downstream crate.
- This crate does not add or own common pointer-cancellation infrastructure.
