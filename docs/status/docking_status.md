# Docking status

## Implemented

- Separate elwindui-docking crate with transparent DockItemId/DockGroupId newtypes.
- Opaque immutable DockLayoutModel, typed placements/errors, default attachment, normalization,
  generated group IDs, and version-1 serde snapshots.
- Declarative DockingControl, DockSplitPanel, DockGroup, and DockItem components.
- Stable item-wrapper registry and private model-to-CustomTabView/CustomSplitter realization
  boundary, including transactional drag/preview, auto-hide, floating-host, and coordinate
  registry seams.
- Declarative docking-demo with two document tabs, two tool windows, and nested horizontal and
  vertical split declarations.

## Verification

The focused model tests cover default/reset, activation, close/reopen, group and all four-side
split/edge placement, floating/auto-hide, normalization, transparent IDs, snapshot round-trip,
typed invalid values, and latest-only source queuing.

- `cargo fmt --all -- --check`: PASS.
- `rust-analyzer diagnostics .`: PASS; 0 Error, 0 Warning, and 0 non-exempt WeakWarning records.
  The 142 reported `Ra("inactive-code", WeakWarning)` records are intentional conditional code.
- `cargo check --workspace`, `cargo build --workspace`, and
  `RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace`: PASS.
- `cargo check -p elwindui-docking -p docking-demo`, `cargo build -p elwindui-docking -p
  docking-demo`, and `cargo test -p elwindui-docking`: PASS; 13 tests passed.
- `cargo test --workspace`: FAIL in the existing AppKit `control_template_window_rt4` fixture:
  its Window-hosted target reported `measured=229x48` and `arranged=1x0`, then aborted. This is
  outside the changed files and prevents treating the workspace test command as fully passing.
- AppKit GUI proof: NOT RUN. `macos-ui-driver doctor` succeeded but reported both Accessibility
  and Screen Recording permissions as false; the demo launch consequently timed out waiting for a
  window and `list-windows` returned no windows. WinUI3 and GTK4 interactive runs are also NOT RUN.

Backend compilation and GUI interaction remain platform-dependent; compilation is not reported as
GUI proof.
