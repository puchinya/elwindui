# WinUI3 pointer cancellation and capture-loss evidence

Implementation/tooling owner: Issue [#267](https://github.com/puchinya/elwindui/issues/267)
Local runtime acceptance owner: Issue [#270](https://github.com/puchinya/elwindui/issues/270)

This durable case covers only native-stimulus-dependent pointer cancellation evidence transferred
from #180. Core/custom-control deterministic behavior and the normal WinUI3 pointer path remain
owned by #180 / PR #268. Issue #267 owns the implementation/tooling capability; Issue #270 owns
the deferred local non-RDP runtime execution. The case must not synthesize Core cancellation or
change the public pointer contract.

## Fixed setup

- Use a normal, non-elevated Windows user on an unlocked interactive desktop.
- Run the driver outside the agent sandbox.
- Run `windows-ui-driver.ps1 doctor` once and retain its `winapp_version`, `session_id`, and
  `input_desktop_probe`.
- Build and launch `custom-controls-demo` with a fresh PID/HWND and fresh geometry for each case
  group. The demo must show Probe A, Probe B, the existing CustomGridSplitter, and the existing
  CustomTabView status line.
- For every trace-enabled group, set
  `ELWINDUI_WINUI3_POINTER_TRACE_PATH` to an absolute JSONL path under
  `.agent-state/issues/267/e2e/<runtime-evidence-head>/<run-id>/` before launching the product.
- Set `ELWINDUI_WINUI3_E2E_RELEASE_CAPTURE_ON_PRESS=1` only for NC-02. The hook is effective only
  when the trace path is also set.
- Record repository HEAD, `origin/master`, Windows version/build/session, driver version, PID,
  HWND, geometry, every driver JSON result, trace JSONL, and useful screenshots. Never reuse an old
  run directory.
- End every run with `terminate --pid <pid> --timeout 5` and record whether forced cleanup was
  needed. Reap a background `touch-cancel` process on every outcome.

## Driver stimulus

Resolve fresh screen-physical coordinates from the current HWND and case-local probe geometry. For
an ordinary cancellation:

```powershell
pwsh -NoProfile -File $D touch-cancel `
  --hwnd <hwnd> `
  --from-x <probe-x> --from-y <probe-y> `
  --to-x <displaced-x> --to-y <displaced-y> `
  --hold-ms 250
```

The successful driver result must contain `success:true`, `hwnd`, `from`, `latest`, `hold_ms`,
`sequence:"down-update-canceled"`, `injection:"windows-touch"`, and
`injection_backend:"synthetic-pointer"` unless the documented legacy fallback was selected.
The primary backend is `CreateSyntheticPointerDevice` / `InjectSyntheticPointerInput` /
`DestroySyntheticPointerDevice` on Windows 10 1809+. Legacy `InitializeTouchInjection` /
`InjectTouchInput` is fallback-only for an unavailable or explicitly unsupported modern API; a
malformed modern frame and error 87 are `tool_error`. A physical touchscreen is not required.
This proves injection only; product PASS requires the native trace and visible application state
below. Use the complete bounded command with `--hold-ms 1500` as a background PowerShell process
only for NC-11.

## Evidence rules

The trace is the native evidence layer. It must show the native event, `pointer_id`,
`source_classification`, `forwarded_to_core`, root/screen positions, and monotonic `order`. Press
records must be followed by a `NativeCapture` record with success. Cancellation records must make
the sequence distinguishable as native event -> `NativeCaptureRelease` with
`native_capture_release_success` recorded after the actual release call -> optional later
`PointerCaptureLost`. The trace does not emit or claim `CoreCancellation`; exactly-once Core
cancellation is proved by combining forwarded native events with the Probe canceled count. Do not
fabricate `PointerCaptureLost` when Windows does not emit it.

The visible demo is the Core/application evidence layer. Probe A and Probe B independently display
`pressed`, `moved`, `released`, `canceled`, `tapped`, `double_tapped`, `right_tapped`, last root
position, last screen position, and last button. After cancellation, the canceled line must show
`button=None`; a canceled gesture must not produce a normal release or tap.

## Acceptance matrix

### NC-01 — PC-01: native `PointerCanceled` and latest payload

Fresh trace-enabled process. Run `touch-cancel` on Probe A with distinct `from` and `to` points.
PASS requires one native `PointerCanceled` forwarded to Core, Probe A `canceled=1`,
`released=0`, `button=None`, and root/screen positions matching the latest `to` point rather than
the press point.

### NC-02 — PC-02: independent native `PointerCaptureLost`

Fresh process with both trace and `ELWINDUI_WINUI3_E2E_RELEASE_CAPTURE_ON_PRESS=1`. Perform one
normal real point-click on Probe A. PASS requires accepted Core press, successful native capture,
the hook's native release, one native `PointerCaptureLost` forwarded to Core, `canceled=1`,
`released=0`, and `tapped=0`. The hook must not call Core cancellation directly.

### NC-03 — PC-03: canceled then capture-lost idempotence

Fresh process with the hook disabled. Run NC-01. PASS requires one native `PointerCanceled` and,
only if Windows emits it, a later `PointerCaptureLost`; Probe A remains exactly `canceled=1` with no
normal release/tap.

### NC-04 — PC-04: capture released and fresh routing

Without restarting after NC-01, refresh geometry and real-click Probe B. PASS requires no B gesture
to reach A, a fresh B press/release/tap, and trace evidence that the canceled A capture is gone.

### NC-05 — native PC-05 portion: no active Core gesture

Fresh trace-enabled process. Run `touch-cancel` on an actual native XAML child instead of the Core
input surface. PASS requires a native routed cancellation with native-child classification,
`forwarded_to_core=false`, and no Core cancellation or tap duplicate.

### NC-06 — native PC-06 portion: recognition suppression

Run `touch-cancel` on Probe A. PASS requires `canceled=1`, `released=0`, `tapped=0`,
`double_tapped=0`, and `right_tapped=0`.

### NC-07 — native PC-07 portion: recovery

Cancel A, refresh geometry, then normal real-click B. PASS requires fresh normal B input and no
stale A gesture.

### NC-08 — native PC-08 portion: CustomGridSplitter rollback

Run `touch-cancel` from the splitter to a sufficiently displaced point. PASS requires provisional
resize, final splitter status `canceled=true`, return to the pre-gesture size within the existing
tolerance, and no normal commit. A separate normal real splitter drag must still finish with
`canceled=false`.

### NC-09 — native PC-09 portion: CustomTabView canceled drag

Run `touch-cancel` from a tab header to a displaced point. PASS requires tab-drag completion
`canceled=true`, no canceled-drag commit, and no unintended selected-index mutation. A separate
normal tab drag regression must still pass.

### NC-11 — native PC-11 portion: teardown during an active contact

Start `touch-cancel ... --hold-ms 1500` as a bounded background PowerShell process. Once the trace
proves press acceptance and native capture, terminate the product normally. PASS requires teardown
while the real contact is active, exactly one Core cancellation before teardown completes, native
release/capture-loss evidence if Windows emits it, no later release/tap to the old target, and a
normal product exit without forced kill. Reap the background driver process.

## Supporting regression evidence

The native rows above do not replace the deterministic and normal-pointer evidence already owned
by the predecessor verification:

- The deterministic Core cancellation and custom-control tests remain required for the stable
  public semantics: cancellation with no active gesture, recognition suppression, fresh hit test,
  subtree/unmount ordering, splitter rollback, tab-drag cancellation, and close-request behavior.
- The existing hosted-XAML test `hosted_button_text_and_window_lifecycle_regressions_work` and its
  adjacent `live_input_surface_creation_persistence_viewport_and_source_classification()` helper
  remain the single-Application evidence for renderer-created projection input transparency,
  exact root/surface source classification, native-child rejection, and lifecycle behavior.
- The normal real-pointer rows in [`self-drawn-pointer-input.md`](self-drawn-pointer-input.md)
  remain the regression control for tab selection, splitter drag, and native-control click. A
  normal regression is not a substitute for the native cancellation trace.

Run the focused deterministic commands against the runtime-evidence HEAD:

```text
cargo test -p elwindui-core cancel
cargo test -p elwindui-custom-controls --test controls pointer_dispatcher_implicit_capture_completes_tab_outside_and_cancels
cargo test -p elwindui-custom-controls --test controls pointer_dispatcher_cancellation_restores_grid_before_notification
cargo test -p elwindui-custom-controls --test controls close_pointer_canceled_does_not_request_close_or_select
cargo test -p elwindui-backend-winui3 hosted_button_text_and_window_lifecycle_regressions_work
```

These commands prove Core/custom-control ordering and hosted-XAML structure; they are not evidence
that a WinUI3 native Canvas cancellation event was raised.

## Result classification

`PASS` requires both native trace and visible postcondition. `FAIL` means the native action reached
the product but the postcondition is wrong. `NOT RUN` means required evidence was not collected.
`BLOCKED` means the desktop, foreground, access, touch-injection capability, `winapp`, or other
host/tool condition prevented the action; do not relabel it as a product failure.
