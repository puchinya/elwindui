# Issue #236 reviewer evidence — TreeHostPanel input surface

Windows: build 26100 (Windows App SDK / Windows 10-family). Repository commit at time of this
verification run: `0be1f74c76b8` (`feature/236-treehostpanel-input-surface`, rebased onto
`origin/master` `f9c178d`). `winapp` version: `0.6.1`.

## Result: BLOCKED, not PASS — real synthetic pointer input does not reach WinUI3 windows on this host

Scenario definitions: [`tests/e2e/self-drawn-pointer-input.md`](../../../../tests/e2e/self-drawn-pointer-input.md).

| Scenario | Action type | Classification |
|---|---|---|
| SDP-01 — CustomTabView real-pointer selection | real `point-click` | BLOCKED |
| SDP-02 — CustomGridSplitter real-pointer drag | real `drag` | BLOCKED |
| SDP-03 — Docking real-pointer tab selection | real `point-click` | NOT RUN (blocked upstream) |
| SDP-04 — Docking real-pointer tab drag/reorder | real `drag` | NOT RUN (blocked upstream) |
| SDP-05 — NativeControl exactly-once | real `point-click` | BLOCKED |

## Root cause

Four full attempts at SDP-01 each reported the driver's own `point-click`/`drag` as
`success: true` (correct HWND, correct foreground, cursor confirmed via `GetCursorPos` to land
exactly on the intended target — no coordinate/DPI/virtual-screen mapping error), yet the
application never visibly reacted (tab selection never changed).

A follow-up control test isolated the cause: a real `point-click` was issued against a genuine
**native** WinUI3 `Button` (`controls-demo`'s "Normal" button, entirely unrelated to Issue #236's
self-drawn code path) at its correct UIA-reported bounds. It also produced zero effect — the
button's own click handler never fired, and the event log stayed empty. A UIA `invoke` on the
*same* button, immediately after, worked correctly and appended `Normal clicked` to the event log.
A parallel test against a classic Win32 window (Notepad) showed both real mouse clicks and real
keyboard input landing and registering correctly.

Conclusion: this verification host's current session cannot deliver real synthetic pointer input
(`SendInput`) into *any* WinUI3/XAML-Islands window — self-drawn or native — while the same
mechanism works normally against a classic Win32 window, and UIA's own `invoke` pathway works
normally against WinUI3 native controls. This is a limitation of the current verification
host/session for WinUI3 real-input delivery, not evidence of a defect in Issue #236's
`TreeHostPanel` input-surface implementation, and not a driver bug demonstrated against a
non-WinUI3 target.

## What is verified

- Structural/live-XAML correctness of the `input_surface` architecture (construction, z-order
  persistence, viewport sizing including the unconstrained-axis case, transparency independence,
  and source classification — including rejecting a real native `Button`, not just an unrelated
  `TextBlock`) is covered by `crate::host::live_input_surface_tests`, exercised in a live hosted
  WinUI XAML session as part of `cargo test --workspace`.
- Real-OS-pointer acceptance for the self-drawn tab/splitter/docking scenarios remains unverified
  pending a verification host that can deliver real synthetic pointer input into a WinUI3 window
  (tracked as an open item; see the owning Issue for current status rather than this frozen
  snapshot).

## Illustrative screenshot

`custom-controls-demo-blocked-state.png` — full-screen capture (`--capture-screen`) of
`custom-controls-demo` after repeated real-pointer click attempts against the "Inspector" tab
header: the underline selection indicator remains under "Overview", confirming no visible state
change occurred despite the driver reporting successful input delivery.

## Raw evidence

Full command JSON, additional screenshots, and the per-attempt investigation trail (four SDP-01
attempts plus the Notepad/native-Button control tests) are under
`.agent-state/issues/236/e2e/0be1f74c76b8/` (not committed; local/CI working-tree only).
