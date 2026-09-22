# AppKit Window lifetime and generated caller-drop

Issue [#259](https://github.com/puchinya/elwindui/issues/259) durable product E2E case for the
AppKit Window-lifetime invariant delivered by PR #257. AppKit execution is required for #259;
WinUI3 may reuse this case later. This case accepts product-visible behavior, not driver return
values.

## Fixture and scope

Use `examples/custom-controls-demo` on the final candidate HEAD. Its `main()` must retain the
ordinary caller-drop shape:

```rust
let window = CustomControlsDemoWindow::new();
window.show();
```

Do not add a keepalive, sleep, registry access, or test-only lifetime anchor to the demo. The
generated Window must remain live because the AppKit application registry owns it after `show()`.

Use the checked-in `tools/macos-ui-driver/bin/macos-ui-driver` and the procedure in
[`docs/agents/appkit-e2e.md`](../../docs/agents/appkit-e2e.md). Run the driver outside the sandbox,
with Accessibility and Screen Recording both true. Store immutable evidence in
`.agent-state/issues/259/e2e/<head-short>/<run-id>/` and exact command stdout/stderr in the
Issue-scoped log directory.

## Reused real-input mechanics

Reuse the real-input mechanics and postcondition vocabulary from
[`self-drawn-pointer-input.md`](self-drawn-pointer-input.md), especially SDP-01's `Inspector`
real-pointer selection and its fresh-geometry/screenshot rule. Do not define a second click or drag
protocol here. UIA may locate an anchor or observe a postcondition, but an AX/UIA invoke action is
not a substitute for the required real mouse input. If a self-drawn target cannot be identified
reliably from fresh visual evidence, classify the row `BLOCKED`.

## WLT-01 — generated Window remains functional after caller drop

1. Build `custom-controls-demo` on the final candidate HEAD and launch it once through
   `macos-ui-driver launch --path <absolute-executable> --cwd <repository-root> --wait-window-timeout 30`.
2. Record the PID, the AppKit CGWindowID, title, and current bounds from `list-windows --pid <pid>`.
3. Confirm the process is responsive and frontmost using the AppKit guide. Immediately before the
   real action, refresh the window list/geometry and reuse SDP-01's fresh screenshot and
   window-relative coordinate procedure to real-click the visible `Inspector` header.
4. Capture the required before/after images and re-query the current window. Observe a visible
   selected-page/content change and the supporting status text when exposed by the fixture.

`PASS` requires the same PID to remain responsive, the generated Window/TreeHost to accept the real
input, and a visible state change. Seeing the Window or a driver `success: true` alone is not PASS.

## WLT-02 — generated native close and normal termination

In the same run, refresh the current AppKit window geometry and identify the real title-bar Close
affordance for the live CGWindowID. Use a real mouse click at its current screen coordinate (or
`click --via mouse` only when the exact native AX button is uniquely discovered); do not use a
programmatic terminate or an AX invoke as the acceptance action.

`PASS` requires the generated Window to disappear from `list-windows`, the process to terminate on
its own through AppKit's last-window policy with `forced=false`/no forced cleanup, and no crash,
hang, or tester-forced exit. A missing/ambiguous close target, unavailable permission, or absent
required evidence is `BLOCKED` or `NOT RUN` according to the AppKit tester guide; a delivered close
whose product postcondition is wrong is `FAIL`.

## Reporting

Report WLT-01 and WLT-02 independently as exactly `PASS`, `FAIL`, `NOT RUN`, or `BLOCKED`, with
the final tested HEAD, PID/CGWindowID/bounds, screenshots, separate stdout/stderr paths, and cleanup
state. Do not claim WinUI3 or any unexecuted Docking row from this case.
