# Self-drawn pointer input

This is the durable, backend-neutral product scenario for real pointer input over ElwindUI
self-drawn content. It is required on WinUI3 for Issue [#236](https://github.com/puchinya/elwindui/issues/236)
and may be consumed by AppKit when that backend is scheduled for the same behavior. GTK4 is not
currently supported for this case.

The action under test must be real mouse input through the repository platform driver. UIA may
locate an anchor or observe a postcondition, but UIA `InvokePattern` is not a substitute for a
self-drawn click or drag. When self-drawn content is absent from the UIA tree, use the current
window-relative coordinate and screenshot procedure instead of weakening the product assertion.
Refresh HWND, window geometry, UIA bounds, and DPI immediately before every real action. Convert
ElwindUI logical offsets with `screen_delta = logical_delta * dpi / 96.0`; do not use desktop-global
constants.

Each case is classified as `PASS`, `FAIL`, `NOT RUN`, or `BLOCKED` according to
[`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md). A successful driver command is
not a product PASS without its required state or geometry postcondition.

## SDP-01 — CustomTabView real-pointer selection

Application: `target/debug/custom-controls-demo.exe`.

Launch a fresh process and wait for the initial status and the `Inspector` header to be visible.
Refresh the current window geometry and the exact `Inspector` header bounds immediately before the
action. Use `point-click --hwnd <hwnd> --x <screen-x> --y <screen-y>` at the center of that header
using the coordinate procedure above. Do not use UIA invoke.

PASS requires one delivered real click and an observed application state containing:

```text
Selected tab: Inspector · selected_index callback received 1
```

The selected `CustomTabViewItem` page must also have non-zero visible bounds. A driver success
without the status/page change is `FAIL` if the action reached the application.

## SDP-02 — CustomGridSplitter real-pointer drag

Use a fresh or restored `custom-controls-demo.exe` state. Refresh the exact window geometry, DPI,
and the `Interaction surface` anchor. On the current demo layout, the splitter center is derived
from the anchor as:

```text
splitter_x = interaction_surface.left - (18 + 3) * dpi / 96.0
splitter_y = center_y(interaction_surface)
```

Perform a real drag with the current driver from that point to
`splitter_x + 40 * dpi / 96.0, splitter_y`. Re-query the anchor and the status after the drag.

PASS requires both:

- status matching `Grid resize completed: cumulative delta=<non-zero>px canceled=false · panes resized`;
- the `Interaction surface` left coordinate moving at least `20 * dpi / 96.0` pixels in the
  drag direction.

The geometry assertion distinguishes an actual pane resize from callback-only completion.

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
Refresh both exact-name header bounds and verify `Document A.left < Document B.left`. Start a real
driver drag at the center of `Document B` and release in the left half of the `Document A` header,
beyond the four-logical-pixel drag threshold after DPI conversion.

PASS requires `Committed a live layout change` and, after re-querying both headers,
`Document B.left < Document A.left`. A generic status message without the spatial reversal is not
enough. If current Docking behavior intentionally contradicts this required reorder, report the
exact repository conflict rather than changing Docking semantics in this case.

## SDP-05 — NativeControl remains native-only / exactly once

Application: `target/debug/controls-demo.exe`.

Navigate to the native `TabViewItem` whose exact header is `Button`, confirm that page is visible,
and verify the event log is empty. Refresh the exact `Normal` button bounds and perform exactly one
real coordinate `point-click` on its center.

PASS requires the application event log to contain exactly one appended line:

```text
Normal clicked
```

Two lines after one gesture is a `FAIL`: it indicates duplicate ownership between the native
control and Core. The direct hosted source-classification assertion for a real XAML `Button` is
required alongside this runtime isolation case.

## Cleanup and evidence

Terminate every launched process with the platform driver's `terminate --pid <pid> --timeout 5`,
including after a failed or blocked action. Store raw command JSON, screenshots, environment
details, and the final commit SHA under the owning Issue's immutable `.agent-state` E2E run
directory. A reviewer-facing summary or final-state screenshot may be committed under the Issue's
`docs/issues/236-treehostpanel-input-surface/evidence/` directory when the acceptance workflow
requires it; raw logs do not belong in the repository.
