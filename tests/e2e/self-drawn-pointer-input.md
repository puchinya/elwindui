# Self-drawn pointer input

This is the durable, backend-neutral product scenario for real pointer input over ElwindUI
self-drawn content. It is required on WinUI3 for Issue [#236](https://github.com/puchinya/elwindui/issues/236)
and may be consumed by AppKit when that backend is scheduled for the same behavior. GTK4 is not
currently supported for this case.

The action under test must be real mouse input through the repository platform driver. UIA may
locate an anchor or observe a postcondition when the target is discoverable, but UIA `InvokePattern`
is not a substitute for a self-drawn click or drag. Issue #260's semantic UIA discoverability is
not an acceptance dependency for Issue #236. When UIA cannot locate a self-drawn or native target,
use a fresh screenshot and current HWND/window geometry to identify the visible target, derive and
record a window-relative coordinate, and use that coordinate for the real action. If the target
cannot be identified reliably from current visual evidence, classify the case as `BLOCKED`.
Refresh HWND, window geometry, any available UIA bounds, and DPI immediately before every real
action. Convert ElwindUI logical offsets with `screen_delta = logical_delta * dpi / 96.0`; do not
use desktop-global constants or stale coordinates.

For WinUI3 acceptance, resolve the repository root with `git rev-parse --show-toplevel` and launch
each application through `tools/windows-ui-driver` using its absolute executable path,
`--cwd <repository-root>`, and `--wait-window-timeout 30`. The acceptance run must not depend on an
application asynchronously started outside the driver; a one-off external launch is diagnostic
only and is not evidence. Wait for the PID/HWND returned by `launch` before querying or acting.

When a screenshot supplies the target geometry, record the current window rectangle and the
window-local target point, then derive `screen_x = window.left + local_x` and
`screen_y = window.top + local_y`. Recompute this after every move, resize, or layout/topology
change.

Each case is classified as `PASS`, `FAIL`, `NOT RUN`, or `BLOCKED` according to
[`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md). A successful driver command is
not a product PASS without its required state or geometry postcondition.

## SDP-01 — CustomTabView real-pointer selection

Application: `target/debug/custom-controls-demo.exe`.

Launch a fresh process and wait for the `Inspector` header to be visibly identified. Refresh the
current window geometry and exact `Inspector` bounds immediately before the action when UIA exposes
them. Otherwise capture a fresh screenshot, identify the visible header in the current window, and
record its window-local center before deriving the screen point with the coordinate procedure
above. Use `point-click --hwnd <hwnd> --x <screen-x> --y <screen-y>` at that point. Do not use UIA
invoke.

PASS requires one delivered real click, a visibly selected `Inspector` page, and non-zero visible
selected-page bounds.

The status string is supporting evidence when observable:

```text
Selected tab: Inspector · selected_index callback received 1
```

A driver success without the visible page change is `FAIL` if the action reached the application.

## SDP-02 — CustomGridSplitter real-pointer drag

Use a fresh or restored `custom-controls-demo.exe` state. Refresh the current window geometry and
DPI. Use the `Interaction surface` anchor when UIA exposes it. Otherwise capture a fresh screenshot,
identify the visible splitter, and record its window-local center from that screenshot. On the
current demo layout, the splitter center may be derived from a discoverable anchor as:

```text
splitter_x = interaction_surface.left - (18 + 3) * dpi / 96.0
splitter_y = center_y(interaction_surface)
```

Perform a real drag with the current driver from that point to
`splitter_x + 40 * dpi / 96.0, splitter_y`. Capture a fresh after-screenshot and compare the
visible pane boundary with the before-screenshot. Re-query the anchor and status after the drag
when they are discoverable.

PASS requires both:

- a real delivered drag; and
- visible pane geometry moving at least `20 * dpi / 96.0` screen pixels in the drag direction.

The status is supporting evidence when observable:

```text
Grid resize completed: cumulative delta=<non-zero>px canceled=false · panes resized
```

The geometry assertion distinguishes an actual pane resize from callback-only completion without
making #236 depend on #260's semantic UIA tree.

## SDP-03 — Docking real-pointer tab selection

Application: `target/debug/docking-demo.exe`.

Launch a fresh process and wait for exact-name `Document A` and `Document B` header elements. Do
not use a partial match such as `Document B editor`. Confirm that `Document A editor` is the
visible selected content, refresh the current `Document B` header bounds, and use one real
coordinate `point-click` at its center.

PASS requires visible non-zero `Document B editor` content and an observed active-layout change.
When exposed by the demo, the status must also contain `Committed a live layout change`. Driver
success alone is not sufficient.

## SDP-04 — Docking real-pointer tab drag/reorder

Use a fresh/reset documents-group state whose header order is `Document A`, then `Document B`.
Refresh both exact-name header bounds when discoverable. Otherwise capture a fresh screenshot and
record the window-local centers of the visible `Document A` and `Document B` headers. Start a real
driver drag at the center of `Document B` and release in the left half of the visible `Document A`
header, beyond the four-logical-pixel drag threshold after DPI conversion.

PASS requires a real delivered drag, a visible valid Docking layout mutation, and evidence that the
dragged document changed docking placement or grouping. The current designed vertical-group result
is valid; horizontal header reversal is not required by this case. `Committed a live layout change`
remains supporting evidence. Do not change Docking semantics merely to force a particular header
order. If the intended Docking product specification requires a different result, report that
specification conflict separately.

## SDP-05 — NativeControl remains native-only / exactly once

Application: `target/debug/controls-demo.exe`.

Navigate to the visible native `TabViewItem` whose header is `Button`, using a real native pointer
action or another repository-approved native-control action. Confirm that page is visible and the
event log is empty. Capture a fresh screenshot and current window geometry, identify the visible
native `Normal` button, record its window-local center, and perform exactly one real coordinate
`point-click` on its derived screen point. Do not modify accessibility exposure merely to locate
the button.

PASS requires the application event log to contain exactly one appended line:

```text
Normal clicked
```

Two lines after one gesture is a `FAIL`: it indicates duplicate ownership between the native
control and Core. A missing UIA match for the native Button is not by itself a #236 failure; if the
button cannot be identified reliably from the fresh screenshot, classify the case as `BLOCKED`.
The direct hosted source-classification assertion for a real XAML `Button` is required alongside
this runtime isolation case.

## Cleanup and evidence

Terminate every launched process with the platform driver's `terminate --pid <pid> --timeout 5`,
including after a failed or blocked action. Store raw command JSON, screenshots, environment
details, and the final commit SHA under the owning Issue's immutable `.agent-state` E2E run
directory. A reviewer-facing summary or final-state screenshot may be committed under the Issue's
`docs/issues/236-treehostpanel-input-surface/evidence/` directory when the acceptance workflow
requires it; raw logs do not belong in the repository.
