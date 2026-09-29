# WinUI.Dock visual and interaction fidelity

Issue [#285](https://github.com/puchinya/elwindui/issues/285) compares ElwindUI Docking with the
pinned `qian-o/WinUI.Dock@7949f84a8da28f8e4af13ce582bd5a99fd97b5e3` reference. Run this matrix on a
real Windows host using the existing Windows UI driver and the tester procedure in
[`winui3-e2e.md`](../../docs/agents/winui3-e2e.md). Each row has one result: `PASS`, `FAIL`,
`NOT RUN`, or `BLOCKED`. A driver result alone is not product evidence.

The Issue #285 approved amendment removes all group-level drag, tear-out, and cross-dock
operations. Every drag in this matrix starts from one individual Document. Bottom content-header
actions and drag apply only to the selected Document. Moving a floating window uses its native
window title bar.

## Fixture and evidence

Use the docking demo and its authored capability fixtures. Capture fresh ElwindUI and pinned
reference screenshots at equivalent scale for visual rows. Record numeric geometry, visible state,
and the observed transition; do not require raster-identical text rendering. Store immutable run
evidence under `.agent-state/issues/285/e2e/<head-short>/<run-id>/` and attach useful comparison
screenshots to Issue #285. Record exact HEAD, host/driver versions, screenshot paths, and cleanup in
the run manifest.

## Results

| ID | State and action | PASS requires | Result / evidence |
|---|---|---|---|
| WDF-01 | Top tabs: select a Document, hover a capable tab, then leave it. | Selected tab joins the active content frame; pin/close appear on hover and disappear on exit; group has no separate title bar. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-02 | Bottom multi-tab group: select another Document and inspect its content header and tabs. | Header title/actions belong to the selected Document; actions are not duplicated in tabs; the selected Document remains the only drag source. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-03 | Bottom single-tab group: inspect the tab strip, frame, header, and page. | Tab strip is collapsed/noninteractive; content header and full content frame remain visible and usable. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-04 | Compare compact and non-compact strips with short and long titles. | Normal widths equalize up to 200 logical px; compact widths follow content up to the same computed cap; heights match. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-05 | Drag one Document tab over an off-center nested target group, then move between its header and compass regions. | Compass follows that group's arranged center; individual tab drag resolves header insertion and group Split targets; no group-level drag is available. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-06 | Inspect the group compass during an individual Document drag. | Connected 124 logical px cross contains five directional 36 logical px target glyphs with Center/Split semantics. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-07 | Inspect surface-edge targets and nearby group targets during an individual Document drag. | Root target visuals remain surface-relative and distinct from group Split targets. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-08 | Resolve Center, Split, and root Dock targets. | Preview uses resolver geometry, 4 logical px border, rounded corner, 0.4 opacity, and does not intercept input. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-09 | Resize a horizontal split with a real pointer. | 12 logical px gutter; live geometry tracks movement and release does not jump. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-10 | Resize a vertical split with a real pointer. | 12 logical px gutter; live geometry tracks movement and release does not jump. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-11 | Inspect and activate auto-hide items on Left, Top, Right, and Bottom. | Titles are content-sized; left/right rotate, top/bottom remain horizontal; entries have 16 logical px spacing and a 4 logical px active/hover marker. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-12 | Open Left and Right auto-hide items, resize, close, and reopen each. | Initial width is one third of usable center area; resize remains on the chosen side, trailing-edge direction is inverted, and each item's extent is remembered. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-13 | Open Top and Bottom auto-hide items, resize, close, and reopen each. | Initial height is one third of usable center area; resize remains on the chosen side, Bottom direction is inverted, and each item's extent is remembered. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-14 | Dismiss an open auto-hide pane by clicking outside and Escape; then exercise pin and close. | Dismissal changes presentation only; pin/close follow capability and model lifecycle rules. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-15 | Start an individual Document drag on a floating surface and inspect its compass/preview; move the floating window by its native title bar. | Compass/preview follow the floating target group; native title-bar movement updates window bounds without creating a group drag source. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
| WDF-16 | Inspect tab, target, preview, and auto-hide chrome in light and dark themes. | Theme resources update all listed chrome and preserve target geometry, action visibility, and interaction states. | BLOCKED — pinned reference app produced no HWND; see `.agent-state/issues/285/e2e/55f0a852cb8d/20260928T165331Z/report.md`. |
