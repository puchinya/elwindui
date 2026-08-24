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
- Core-only tests cover ownership, metadata/icon realization, selection and
  callback behavior, tab gestures, cancellation, and splitter gestures.

## Known implementation gap (C — newly discovered requirement)

The current `#[component]` composed-target generator marks companion
`#[overrides]` methods as inherent helpers. It does not route those methods into
the inherited `UIElementExt` virtual layout/render/hit-test dispatch table.
Consequently the custom `measure_override`, `arrange_override`, `render`, and
`hit_test_content` implementations can be exercised directly, but the normal
host `layout_root`/`RenderTree` path still uses the inherited Control behavior.
The close-geometry render regression is therefore kept as an ignored test with
an explicit C-class reason. Fixing the component override bridge requires a
separate approved codegen/core prerequisite; this PR does not silently refactor
codegen or fall back to new `#[class]` controls.

## Verification

The focused crate test command passes with 12 tests and one ignored C-class
render test. Core, codegen, AppKit-enabled facade check, inheritance-demo
check, workspace check/build, and `git diff --check` also pass. Workspace-wide
`cargo fmt --all -- --check` still reports pre-existing formatting differences
outside this crate; the new crate passes a direct rustfmt check. The full
AppKit facade suite, `control_template` integration test, and workspace test
suite start successfully but time out in the current GUI test environment;
they produced no test failure before the timeout. `rust-analyzer diagnostics .`
remains a baseline-wide nonzero command (the clean master archive has the same
class of macro/import diagnostics). AppKit/WinUI3 GUI interaction has not been
run; Windows runtime verification remains outside #173 in #178/#180.

## Follow-up

- Open/track the C-class component override-vtable prerequisite before claiming
  host-level custom layout/render parity.
- Docking integration remains Issue #172 and must stay a downstream crate.
- This crate does not add or own common pointer-cancellation infrastructure.
