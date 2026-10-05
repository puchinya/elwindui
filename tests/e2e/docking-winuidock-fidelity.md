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

The splitter content/outline regression probe passed on a normal non-elevated
Windows host with the candidate executable SHA-256
`CFD4CD6DC7E50B7B74431A6048756041DAA53F8D014290F7B46D151759A50217`.
Row-down, row-back and column drags had 4000/4006/4000 ms actual movement and
250 steps each. All four pane contents, tab outlines and separators remained
correct after release, selection and resize. Evidence and attached images:
[#285 verification comment](https://github.com/puchinya/elwindui/issues/285#issuecomment-5970902836).
This candidate regression probe does not replace the full pinned-reference
comparison required by WDF-09 and WDF-10 below.

Use the docking demo and its authored capability fixtures. Capture fresh ElwindUI and pinned
reference screenshots at equivalent scale for visual rows. Record numeric geometry, visible state,
and the observed transition; do not require raster-identical text rendering. Store immutable run
evidence under `.agent-state/issues/285/e2e/<head-short>/<run-id>/` and attach useful comparison
screenshots to Issue #285. Record exact HEAD, host/driver versions, screenshot paths, and cleanup in
the run manifest.

## Results

| ID | State and action | PASS requires | Result / evidence |
|---|---|---|---|
| WDF-01 | Top tabs: select a Document, hover a capable tab, then leave it. | Selected tab joins the active content frame; pin/close appear on hover and disappear on exit; group has no separate title bar. | NOT RUN — selected frame and active marker observed on the candidate; hover reveal is not drivable (the Windows driver has no hover verb). |
| WDF-02 | Bottom multi-tab group: select another Document and inspect its content header and tabs. | Header title/actions belong to the selected Document; actions are not duplicated in tabs; the selected Document remains the only drag source. | PASS (candidate) — selecting Git Changes moves the content header title/actions to it (run `20261004T124556Z-wdf` W02). Reference: NOT RUN. |
| WDF-03 | Bottom single-tab group: inspect the tab strip, frame, header, and page. | Tab strip is collapsed/noninteractive; content header and full content frame remain visible and usable. | PASS — Error List single-tab group: strip collapsed, header and frame visible, non-closable pin packed at the trailing edge (run `20261004T155144Z-icons`; reference `20261004T090659Z`). |
| WDF-04 | Compare compact and non-compact strips with short and long titles. | Normal widths equalize up to 200 logical px; compact widths follow content up to the same computed cap; heights match. | NOT RUN — narrow-window sizing with DockSize matches the reference (runs `20261004T140645Z-size`, `20261004T124556Z-wdf`), but the compact vs non-compact long-title comparison was not executed. |
| WDF-05 | Drag one Document tab over an off-center nested target group, then move between its header and compass regions. | Compass follows that group's arranged frame center (including a bottom-tab content header) and stays visible over the group body; targets resolve only on drawn cells, header insertion stays available; no group-level drag is available. | PASS — compass follows the hovered group frame (including bottom-tab header) and Center drops join that group in both apps (run `20261004T124556Z-wdf` S2). |
| WDF-06 | Inspect the group compass during an individual Document drag. | Connected 124 logical px cross contains five directional 36 logical px target glyphs with Center/Split semantics. | PASS — connected 124 px cross with five 36 px glyphs in both apps (comparison images 02/11 on Issue #285). |
| WDF-07 | Inspect surface-edge targets and nearby group targets during an individual Document drag. | Root target visuals remain surface-relative and distinct from group Split targets. | PASS — flush surface-edge root targets with half-document/square glyphs distinct from the group compass (run `20261004T235902Z-dragsrc`). |
| WDF-08 | Resolve Center, Split, and root Dock targets. | Preview uses resolver geometry (half surface for root Dock), 4 logical px border, rounded corner, 0.4 opacity, and does not intercept input; releasing off every drawn target floats a 400 x 400 window. | PASS — Center, SplitRight and root DockLeft previews (half surface) resolve only on drawn targets; results match the reference (runs `20261004T124556Z-wdf`, `20261004T235902Z-dragsrc`). |
| WDF-09 | Resize a horizontal split with a real pointer. | 12 logical px gutter; live geometry tracks movement and release does not jump. | PASS — 4 s row-splitter drag tracks continuously and keeps the 12 px gutter in both apps (run `20261004T124556Z-wdf` W09). |
| WDF-10 | Resize a vertical split with a real pointer. | 12 logical px gutter; live geometry tracks movement and release does not jump. | PASS — 4 s column-splitter drag tracks continuously in both apps (run `20261004T124556Z-wdf` W10). |
| WDF-11 | Inspect and activate auto-hide items on Left, Top, Right, and Bottom. | Titles are content-sized; left/right rotate, top/bottom remain horizontal; entries have 16 logical px spacing and a 4 logical px active/hover marker. | NOT RUN — Right side verified in both apps (rotated title, marker, open/dismiss); Left/Top/Bottom were not exercised. |
| WDF-12 | Open Left and Right auto-hide items, resize, close, and reopen each. | Initial width is one third of usable center area; resize remains on the chosen side, trailing-edge direction is inverted, and each item's extent is remembered. | NOT RUN — Right: one-third initial width, inner-edge resize and remembered width on reopen pass on the candidate (run `20261004T141248Z-pane`); Left side and reference resize were not exercised. |
| WDF-13 | Open Top and Bottom auto-hide items, resize, close, and reopen each. | Initial height is one third of usable center area; resize remains on the chosen side, Bottom direction is inverted, and each item's extent is remembered. | NOT RUN — Top/Bottom auto-hide items were not exercised. |
| WDF-14 | Dismiss an open auto-hide pane by clicking outside and Escape; then exercise pin and close. | Dismissal closes the pane without moving the item and releases its active state; pane pin docks to the same-side root edge as the active item; close follows `can_close`. | NOT RUN — outside dismissal and pane pin-back to the same-side root edge pass in both apps (runs `20261004T140907Z-pane`, `20261004T141248Z-pane`); Escape (no focused recipient in the demo) and pane close were not exercised. |
| WDF-15 | Start an individual Document drag on a floating surface and inspect its compass/preview; move the floating window by its native title bar. | Compass/preview follow the floating target group; native title-bar movement updates window bounds without creating a group drag source. | PASS — floating surface shows only the group compass and a Center drop joins the floating group in both apps (run `20261004T140645Z-size` W15). |
| WDF-16 | Inspect tab, target, preview, and auto-hide chrome in light and dark themes. | Theme resources update all listed chrome and preserve target geometry, action visibility, and interaction states. | PASS — light and dark theme chrome after a runtime theme switch matches the reference (runs `20261004T162856Z-light`, `20261004T162502Z-light`; images 10–12 on Issue #285). |
