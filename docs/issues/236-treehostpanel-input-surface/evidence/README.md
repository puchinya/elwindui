# Issue #236 reviewer evidence — TreeHostPanel input surface

Windows: build 26100 (Windows App SDK / Windows 10-family). `winapp` version: `0.6.1`. This file
was last updated against commit `9282b85980e8` (`feature/236-treehostpanel-input-surface`).

## Result: BLOCKED, not PASS — real root cause confirmed as [#254](https://github.com/puchinya/elwindui/issues/254), independent of #236

Scenario definitions: [`tests/e2e/self-drawn-pointer-input.md`](../../../../tests/e2e/self-drawn-pointer-input.md).

| Scenario | Action type | Classification |
|---|---|---|
| SDP-01 — CustomTabView real-pointer selection | real `point-click` | BLOCKED on [#254](https://github.com/puchinya/elwindui/issues/254) |
| SDP-02 — CustomGridSplitter real-pointer drag | real `drag` | BLOCKED on #254 (same `custom-controls-demo` window) |
| SDP-03 — Docking real-pointer tab selection | real `point-click` | NOT RUN (likely blocked upstream by #254 too — `docking-demo` not yet checked) |
| SDP-04 — Docking real-pointer tab drag/reorder | real `drag` | NOT RUN (same as SDP-03) |
| SDP-05 — NativeControl exactly-once | real `point-click` | Not re-verified this session; native-control real-click delivery is confirmed working generally (see below), but SDP-05's exact case was not rerun |

## Superseded earlier conclusion

An earlier version of this document (commit `0be1f74c76b8`) concluded that *no* WinUI3 window,
self-drawn or native, could receive real synthetic pointer input on this host — based on the
driver's original coordinate-based `point-click` (a zero-distance real `drag`), which failed
uniformly. That conclusion is superseded: `point-click --selector` (added after that point,
delegating to `winapp ui click` instead of a zero-distance drag) *does* deliver working real input
to native WinUI3 controls, and a genuine (non-zero-distance) `drag` delivers working real input to
self-drawn `Canvas` content too — see the positive control below.

## `input_surface` itself is proven correct on a real host (positive control)

A real drag with genuine movement (`winapp ui drag`, 40px, `--hold-ms 150 --dwell-ms 150` — not a
zero-distance click substitute) over a screenshot-verified-blank self-drawn `Canvas` region inside
`controls-demo` (no UIA element anywhere near it) produced a complete, correct routed sequence in
the env-gated pointer-routing diagnostic added this session
(`ELWINDUI_WINUI3_DIAGNOSTICS_LOG=<path>` in `TreeHostPanel::dispatch_pointer_routed`):

```
PointerRouted kind=Moved original_source=Some("Rectangle") accepted=true
PointerRouted kind=Pressed(Left) original_source=Some("Rectangle") accepted=true
PointerRouted kind=Moved original_source=Some("Rectangle") accepted=true   (×23)
PointerRouted kind=Released(Left) original_source=Some("Rectangle") accepted=true
PointerRouted kind=Canceled original_source=Some("Canvas") accepted=true
```

This is a clean, direct, real-host proof that Issue #236's `input_surface`/hit-testing fix works
exactly as designed: a real OS pointer over blank self-drawn area resolves to `input_surface` and
is correctly accepted and dispatched. This rules out an entire earlier line of hypotheses (see
below) — the fix itself is not in question.

## Retired hypothesis: invokable vs. non-invokable UIA targets

An earlier pass this session hypothesized that `winapp`'s real-click delivery required an
invokable UIA automation peer somewhere in the target's ancestry (native `Button`/`TabView`
clicks worked; a self-drawn `CustomTabView` header, exposing only a non-invokable `TextBlock`
projection, did not). **This is disproven** by the positive control above: a plain `Rectangle`
(`input_surface`) with no automation peer at all received a full, correct real-input sequence. A
follow-up code review also confirmed `custom-controls-demo` contains **no** native XAML
interactive element anywhere in its tree that could plausibly explain an invokability-based
distinction — the close "✕" affordance is a core (self-drawn) `Grid`/glyph with its own routed-event
handler, not a native `Button`; `CustomGridSplitter`'s template is a single core `Rectangle`; there
is no `ContentControl`/native-child construction path in this example at all. Do not re-open the
AutomationPeer hypothesis without new evidence.

## Current state: `custom-controls-demo`'s window receives no client-area real input at all, for reasons still unresolved

With `input_surface` proven correct and no native-overlay explanation available, the same
screenshot-verified-blank-area methodology was repeated against `custom-controls-demo` itself,
under increasingly strict controls, all in the same run/session:

- Diagnostics log path set in the exact same shell invocation as `launch` (ruling out env-var
  propagation loss across separate shell calls).
- `focus-window --hwnd <hwnd>` called explicitly before the action, and `doctor`'s
  `foreground_hwnd` independently re-checked immediately before the drag — both confirmed the
  target window was genuinely foreground at click time.
- The target `--hwnd` passed explicitly (never `--pid`, avoiding any window-selection ambiguity).
- Click coordinates re-derived from each run's own `launch` JSON `window.left`/`window.top` (never
  reused across launches, since the window position cascades per launch), and deliberately kept
  well inside the reported `work_area` bounds (an early attempt used a point close to the window's
  bottom edge that turned out to exceed `work_area.bottom` — i.e., in taskbar territory — a
  confound that was identified and controlled for in the final run).
- A genuine (40px, `--hold-ms 150 --dwell-ms 150`) drag, same primitive as the working positive
  control above.

Result: `success: true` from the driver every time, **zero diagnostics-log lines of any kind** —
not even a stray `Canceled`, the kind of line the positive-control run above produced even from
incidental/unrelated pointer activity.

Separately, a real click aimed at the window's OS-native title-bar minimize button (not app
content at all — plain non-client-area window chrome) landed close enough to the close button
instead and **actually closed the running process**. This is important: it proves real synthetic
input from this same driver/session unambiguously reaches this exact window at the OS level. The
failure is therefore specific to client-area (XAML content) pointer routing for this window/app,
not "this window never receives input."

## Confirmed root cause: [#254](https://github.com/puchinya/elwindui/issues/254) — the top-level `Window` component is not retained after `main()`'s startup closure returns

`GetCursorPos` logged immediately before/after an injected drag confirmed the OS cursor lands at
the exact intended pixel, inside `custom-controls-demo`'s reported client rectangle, with zero
error — ruling out the coordinate-handling hypothesis above. No native-overlay or product-code
explanation survived scrutiny either (window-creation/activation code is byte-for-byte equivalent
between `controls-demo` and `custom-controls-demo`).

The actual cause, found by adding thread-id/lifecycle logging to
`crates/elwindui-backend-winui3/src/ffi.rs`'s callback registry and `TreeHostPanel::new()`: for
`custom-controls-demo`, `UiCallbackRegistryOwnerInner::drop` fires **immediately at startup, before
any input is ever sent** — removing the exact callback ids `TreeHostPanel::new()` had just
registered for pointer/keyboard/context routing. `#[elwindui::main]`'s generated `main()` runs the
user's whole body (`let window = SomeWindow::new(); window.show();`) as a single native
`OnLaunched`-triggered closure; once that closure returns, native `Application::Start` owns the
message loop independently, and `window` (the local `Rc<Self>`) is dropped unless something
external holds another strong reference. `InnerWindow::show()`'s `retain_window()` only retains the
**native** `Microsoft.UI.Xaml.Window` COM object, not the Rust-side component wrapper — so the
native window (and its `Canvas`) survives and keeps receiving real OS input forever, while every
native pointer-routed event resolves via `invoke_ui_pointer_event_callback` to an id no longer in
the registry and is silently, permanently no-op'd. `controls-demo`'s window happens to survive
(diagnostically confirmed: zero drops logged across its 18 `TreeHostPanel` instances) because its
ViewModel-bound construction (`elwindui::new!(ControlsDemoWindow(vm: vm))`) incidentally keeps an
external strong reference alive — not a documented guarantee. See #254 for the full analysis and
fix tracking; this is an independent framework-level lifetime bug, not a defect in #236's
`input_surface` fix.

## What is verified

- Structural/live-XAML correctness of the `input_surface` architecture (construction, z-order
  persistence, viewport sizing including the unconstrained-axis case, transparency independence,
  active/inactive `Canvas` hit-test gating, and source classification — including rejecting a real
  native `Button`, not just an unrelated `TextBlock`) is covered by
  `crate::host::live_input_surface_tests`, exercised in a live hosted WinUI XAML session as part of
  `cargo test --workspace`.
- **Real-host proof that Issue #236's fix itself works correctly**: a real OS drag over blank
  self-drawn `Canvas` area in `controls-demo` is accepted by `input_surface` and dispatched
  correctly (see positive control above).
- `custom-controls-demo`'s window loses all Rust-side event routing at startup due to [#254](https://github.com/puchinya/elwindui/issues/254), an independent framework lifetime bug — SDP-01/02
  never got a chance to exercise #236's fix at all. Real-host verification of those scenarios is
  blocked on #254, not on any further work in #236.
- The AutomationPeer/invokability and coordinate-handling hypotheses from earlier passes this
  session are both retired.

## Illustrative screenshot

`custom-controls-demo-blocked-state.png` (from the superseded `0be1f74c76b8` run, kept for
continuity) — a full-screen capture of `custom-controls-demo` after repeated real-pointer click
attempts against the "Inspector" tab header: the underline selection indicator remains under
"Overview". The visible symptom is fully explained by #254 (all Rust-side pointer routing for this
window was already gone before any click was ever sent).

## Raw evidence

Per-attempt JSON/screenshots/diagnostics-log captures for this session's runs are under
`.agent-state/issues/236/e2e/9282b85980e8/` (`sdp01-*`, `diag-baseline-*`, `button-tab-retest/`,
`blank-probe-*`, `final-decisive*`, `titlebar-test-*`, not committed; local/CI working-tree only).
Earlier superseded evidence remains under `.agent-state/issues/236/e2e/0be1f74c76b8/` and
`d8759f795785/`.
