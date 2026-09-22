# Windows UI driver design

Operational tester procedure: [`../../agents/winui3-e2e.md`](../../agents/winui3-e2e.md). AppKit
precedent this mirrors operationally (not structurally): [`../../agents/appkit-e2e.md`](../../agents/appkit-e2e.md),
[`../../../tools/macos-ui-driver/README.md`](../../../tools/macos-ui-driver/README.md).

Shared plan compilation and caching, vision checkpoints, result-classification orchestration, and
animation capture-sequence policy belong to the backend-neutral
[`native_e2e_orchestration_design.md`](native_e2e_orchestration_design.md). This document keeps
the Windows adapter-specific responsibilities and does not duplicate that shared orchestration.

## 1. Responsibility

`tools/windows-ui-driver/windows-ui-driver.ps1` is a thin, repository-owned PowerShell adapter that
gives the repository a stable, versioned command surface for Windows native E2E, while delegating
every UI Automation (UIA) query/action, real mouse/keyboard input, and screenshot capture to the
external Microsoft `winapp` CLI (`winapp ui ...`). The only direct input exception is the bounded,
cancellation-only `touch-cancel` command described in §4.1.

The adapter owns:

- stable repository-facing command names and JSON envelope shape;
- process launch/termination;
- HWND enumeration and deterministic window identity (PID, title, geometry);
- foreground request *and* foreground verification (never trusting a request's own return value);
- window bounds, DPI, and monitor metadata;
- native window move/resize (deterministic positioning, not a product interaction test);
- normalization of `winapp --json` results and of `winapp` failure categories into one fixed error
  taxonomy;
- the bounded `touch-cancel` native-touch stimulus required when `winapp` has no canceled-contact
  verb;
- resolution of the external backend executable, with a test-only override.

The adapter delegates to `winapp ui` for: UIA tree inspection, search, invoke, focus, value/property
reads, wait conditions, real mouse input (click/drag), real keyboard input, and screenshot capture
(including popup/overlay capture). It does not reimplement any of these.

## 2. Repository adapter vs. external `winapp` ownership

`winapp` is an external Microsoft-maintained tool (`winget install Microsoft.winappcli --source
winget`) and is never vendored into this repository, never checked in, and never auto-installed by
the adapter. A missing `winapp` is reported as `tool_error` with the exact install command; the
adapter must not fall back to any other automation mechanism (no Python `uiautomation`, no CUA, no
pywinauto/FlaUI/WinAppDriver/Appium, no second COM UIA tree walker).

`doctor` records `winapp --version` so evidence stays interpretable as the external tool's own
behavior evolves. `doctor` never claims real-mouse-input capability from environment inspection
alone — that capability is proven only by a live case whose application postcondition changed
(see §5).

The adapter captures external `winapp` stdout/stderr, but must drain both streams concurrently
(`ReadToEndAsync()` on both immediately after process start, before waiting for exit). Sequential
`ReadToEnd()` on redirected stdout/stderr is forbidden because either stream can fill its OS pipe
while the adapter blocks waiting for EOF on the other. This is intentionally a different ownership
model from `Cmd-Launch` (§7), which does not capture the target application's stdout/stderr at
all — `Invoke-WinApp` captures a short-lived external backend's own output as part of the driver's
JSON result; `Cmd-Launch` never redirects a long-lived launched application's output in the first
place.

## 3. Win32 helper boundary

The adapter embeds only the minimal P/Invoke needed for operations it owns: `EnumWindows`,
`GetWindowThreadProcessId`, `IsWindowVisible`, `IsWindowEnabled`, `GetWindowTextW`, `GetWindowRect`,
`GetForegroundWindow`, `ShowWindowAsync`, `SetForegroundWindow`, `SetWindowPos`/`MoveWindow`,
`GetDpiForWindow`, `MonitorFromWindow`, `GetMonitorInfoW`, `OpenInputDesktop`, and, only for
`touch-cancel`, the Windows 10 1809+ synthetic-pointer API
`CreateSyntheticPointerDevice`/`InjectSyntheticPointerInput`/`DestroySyntheticPointerDevice` plus
the legacy `InitializeTouchInjection`/`InjectTouchInput` fallback.

The adapter never adds generic Win32 input injection (`SendInput`, `mouse_event`, `keybd_event`,
`PostMessage` as a click/keystroke substitute). All normal real mouse/keyboard/touch/pen delivery
is `winapp ui`'s responsibility; duplicating it here would recreate exactly the ad-hoc,
unclassified `SendInput` path that Issue #224 already showed reports success without observable
effect on at least one host. The one documented exception is `touch-cancel`, which uses the Windows
touch-injection API solely to emit one complete canceled contact and never exposes retained contact
state.

## 4. UIA vs. real-input selection

Use a UIA pattern operation (`invoke`, `get-value`/`set-value`, `wait-for`) whenever it tests the
intended behavior — native `Button` activation, a `ValuePattern`-backed edit, or a wait/assertion.
Use real input only when the behavior under test requires it: self-drawn controls, pointer routing,
hover, drag/drop, splitters, pointer capture, right-click/context requests, keyboard routing, or
focus behavior that depends on real key/mouse delivery. WinUI 3 real-key E2E always uses `winapp ui
send-keys ... --via send-input`; `PostMessage`-style keystroke injection is never an accepted
substitute for windowless XAML controls.

Mouse drag duration is delegated to the external `winapp ui drag --duration-ms <1..60000>`
capability. The repository adapter validates and forwards the option; it does not implement a
second mouse transport or a local generic Win32 injector. `--hold-ms` is stationary before the
movement, `--duration-ms` is the monotonic movement interval, and `--dwell-ms` is stationary at
the destination before release. The external JSON fields `requestedDurationMs`,
`actualMovementDurationMs`, and `moveStepCount` are the timing evidence. While the approved
upstream release lacks this option, a pinned fork based on Microsoft winappCli v0.6.1 may be used;
native evidence records its fork commit, executable SHA-256, and version.

## 4.1 Bounded `touch-cancel` exception

`touch-cancel` exists only for Issue #267's native cancellation evidence. Its command surface is:

```text
windows-ui-driver.ps1 touch-cancel
  --hwnd <hwnd>
  --from-x <screen-physical-x> --from-y <screen-physical-y>
  [--to-x <screen-physical-x> --to-y <screen-physical-y>]
  [--hold-ms <0..2000>]
```

The adapter validates the all-or-none destination pair and bounded hold, requires a visible target
on an available interactive input desktop, makes that HWND the actual foreground window, and then
uses `CreateSyntheticPointerDevice(PT_TOUCH, 1, POINTER_FEEDBACK_NONE)` plus
`InjectSyntheticPointerInput` for exactly one contact. It emits DOWN at the origin, an UPDATE at a
distinct destination, keep-alive UPDATE frames at no more than 50 ms while holding, and
`POINTER_FLAG_CANCELED | POINTER_FLAG_UP` at the latest point. The synthetic device is destroyed
in cleanup on success and failure. The legacy `InitializeTouchInjection`/`InjectTouchInput` path
is fallback-only when the modern API entry point is unavailable or explicitly unsupported; it is
not used to hide malformed modern frames or error 87. A physical touchscreen is not required.
There is no cross-command contact handle, state file, daemon, or generic pointer/pen injection API.

The successful result includes `hwnd`, `from`, `latest`, `hold_ms`, `sequence:
"down-update-canceled"`, `injection: "windows-touch"`, and `injection_backend` identifying
`synthetic-pointer` or a legitimately selected `legacy-touch` fallback. Invalid arguments are
`usage_error`; invalid/gone HWNDs are `target_error`; unavailable desktop, foreground, access, or
explicitly unsupported API conditions are `environment_blocker`; and malformed native frames
including `ERROR_INVALID_PARAMETER` are `tool_error` with the Win32 error. RDP/VM state and absent
physical touch metrics are not capability gates. `success: true` is injection evidence only, never
a product PASS.

## 5. Error and result classification boundary

The adapter distinguishes four error categories: `tool_error` (missing/broken external tool),
`environment_blocker` (host/session/security condition — at minimum `no_interactive_desktop`,
`foreground_not_target`, and integrity/desktop-security `access_denied`-equivalent failures map
here), `target_error` (the addressed window/element/process is gone or wrong), and `usage_error`
(bad adapter invocation). A `winapp` exit code of `0` proves only that the driver operation executed
— it is never treated as proof that the application's own state changed.

This feeds a strict, repository-wide E2E result vocabulary (defined operationally in
[`winui3-e2e.md`](../../agents/winui3-e2e.md)): **PASS** requires both the delivered action and the
observed application postcondition; **FAIL** means the action reached the product but the
postcondition was wrong; **NOT RUN** means the step or its required evidence was never collected;
**BLOCKED** means a host/tool/session/security/foreground condition prevented the native action from
exercising the product at all. `no_interactive_desktop` and an unrecoverable `foreground_not_target`
are always BLOCKED, never FAIL — collapsing them into FAIL would misrepresent an environment gap as
a product defect.

For the direct touch exception, `ERROR_INVALID_PARAMETER` (87) is always `tool_error` by default.
It is not reclassified from `SM_REMOTESESSION`, `SM_DIGITIZER`, or `SM_MAXIMUMTOUCHES`; a physical
touchscreen is not required for synthetic-pointer injection. `ERROR_NOT_SUPPORTED` (50),
`ERROR_CALL_NOT_IMPLEMENTED` (120), access denial, and unavailable interactive desktop/foreground
conditions remain environment blockers where the failing operation documents that meaning.

## 6. Foreground and coordinate model

For real input, the adapter resolves the exact HWND and delegates the action to the corresponding
`winapp` real-input command. The `winapp` action owns target foreground establishment and reports
foreground/session failure through its own backend result — a separate `focus-window` call is not
a mandatory prerequisite, because Windows' anti-focus-stealing rules can reject an independent
`SetForegroundWindow` request even when the subsequent `winapp` real-input path can correctly
target the healthy window (this driver's own `focus-window` was observed to reliably report
`BLOCKED` in exactly that situation, against a target `winapp` itself could still act on correctly).
`focus-window` remains available as an explicit diagnostic or case-specific operation — for example,
a case whose own subject is foreground behavior — but is not part of the default real-input path.

PASS still always requires an independently observed application postcondition; successful
injection alone (`winapp` exit 0) is never sufficient by itself (see §5). If `winapp` itself reports
`no_interactive_desktop`, `foreground_not_target`, or an equivalent session/integrity failure, the
action is classified `environment_blocker` / BLOCKED, with at most one controlled retry after
restoring the documented precondition.

Custom/self-drawn coordinates absent from the UIA tree are always `window.left/top + case-local
offset`, recomputed from a fresh `list-windows` immediately before the action — never a coordinate
cached across a move, resize, floating-window create/close, dock/undock, monitor transition, or
DPI-affecting transition.

## 7. Process launch and stdio ownership

The driver owns process creation and PID/window discovery, not application log transport.

Before launching any child, the driver clears the inherit flag on its own standard handles
(`MakeOwnStdHandlesNonInheritable`, called once at script startup). This is the invariant that
prevents a long-lived launched process from inheriting the *driver's own* caller-facing stdout
pipe and holding its write end open past the driver's own exit — the original failure mode
observed when a caller captured this driver's stdout across a nested process invocation.

`launch` then starts the target application without any driver-owned redirected
stdin/stdout/stderr pipes (`RedirectStandardOutput`/`RedirectStandardError`/`RedirectStandardInput`
are never set on the launched process's own `ProcessStartInfo`). This is deliberate, not an
oversight: a redirected-but-unread pipe backpressures the child once the OS buffer fills, which is
a *different* deadlock class than the one the handle-inheritance fix addresses, and was rejected
as a design (no drain jobs, no `BeginOutputReadLine` event handlers, no `CopyToAsync` drain
ownership — each was tried during this driver's own development and each introduced its own
reproducible hang).

Together these two invariants mean: a caller capturing this driver's own stdout sees EOF as soon
as the driver process itself exits, regardless of whether the launched application is still
running, and the launched application's own stdout/stderr never becomes part of the driver's JSON
protocol. Application log capture is explicitly out of scope for `launch` — a durable case that
needs application logs must use an explicit case/application logging mechanism, not implicit
driver capture. This split is proven by a deterministic regression in
`tools/windows-ui-driver/tests/driver-contract.ps1` that launches a nested driver process, launches
a long-lived fake child through it, and asserts the nested driver's own stdout reaches EOF while
that child is still alive and that the child's own output never appears in the driver's captured
stdout/stderr.

## 8. Screenshot modes

`capture-window` defaults to `winapp ui screenshot` targeting the exact HWND. A case whose evidence
requires a `MenuFlyout`, popup, tooltip, dropdown, or other overlay outside the owning window's own
paint must explicitly request the `--capture-screen` path, and the recorded evidence states which
mode was actually used — a case never silently reports one mode's result as equivalent to the other.

## 9. Evidence model

Each run uses the same Issue-scoped, immutable shape already established for AppKit:
`.agent-state/issues/<issue>/e2e/<head-short>/<run-id>/`, with separate stdout/stderr per action,
repository HEAD and `origin/master`, host OS version/architecture/session metadata, `winapp
--version`, `doctor` output, PID/HWND/geometry, postcondition results, and required screenshots.
Raw logs stay under `.agent-state`; only a small reviewer-facing result set is committed, and only
when the owning Issue requires durable evidence.

## 10. External dependency lifecycle

`winapp` is versioned and updated entirely outside this repository. The adapter's contract is
therefore defined against `winapp`'s documented JSON/exit-code behavior, not against a pinned
binary — `doctor`'s recorded version is the reproducibility anchor. If a future `winapp` version
removes or changes a verb this adapter depends on, that is reported as an exact
command/version/error against this design, not silently absorbed by switching to a different
automation framework.

## 11. Non-goals

No ElwindUI public API or WinUI3 backend behavior change. No generic pointer-down/pointer-up,
pen-injection, persistent contact, or second UI Automation implementation. No vendored `winapp`.
No Rust workspace crate for the driver (this is PowerShell, matching the already-PowerShell Windows
host workflow). This design does not execute or close Issues #224, #226, or #157 — it is
infrastructure those Issues' own verification work can build on.

## 12. Product E2E ownership boundary

The Windows driver implements platform automation primitives only. It does not own product E2E
scenario definitions. Durable product scenarios live under
[`tests/e2e/`](../../../tests/e2e/README.md) and call this driver through the repository E2E
workflow (`docs/agents/winui3-e2e.md`). This driver's own `tools/windows-ui-driver/tests/` holds
only its deterministic adapter-contract tests (`driver-contract.ps1`, `fake-winapp.ps1`), never a
permanent product-specific scenario.
