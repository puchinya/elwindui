# Windows UI driver design

Operational tester procedure: [`../../agents/winui3-e2e.md`](../../agents/winui3-e2e.md). AppKit
precedent this mirrors operationally (not structurally): [`../../agents/appkit-e2e.md`](../../agents/appkit-e2e.md),
[`../../../tools/macos-ui-driver/README.md`](../../../tools/macos-ui-driver/README.md).

## 1. Responsibility

`tools/windows-ui-driver/windows-ui-driver.ps1` is a thin, repository-owned PowerShell adapter that
gives the repository a stable, versioned command surface for Windows native E2E, while delegating
every UI Automation (UIA) query/action, real mouse/keyboard input, and screenshot capture to the
external Microsoft `winapp` CLI (`winapp ui ...`).

The adapter owns:

- stable repository-facing command names and JSON envelope shape;
- process launch/termination;
- HWND enumeration and deterministic window identity (PID, title, geometry);
- foreground request *and* foreground verification (never trusting a request's own return value);
- window bounds, DPI, and monitor metadata;
- native window move/resize (deterministic positioning, not a product interaction test);
- normalization of `winapp --json` results and of `winapp` failure categories into one fixed error
  taxonomy;
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

## 3. Win32 helper boundary

The adapter embeds only the minimal P/Invoke needed for operations it owns: `EnumWindows`,
`GetWindowThreadProcessId`, `IsWindowVisible`, `IsWindowEnabled`, `GetWindowTextW`, `GetWindowRect`,
`GetForegroundWindow`, `ShowWindowAsync`, `SetForegroundWindow`, `SetWindowPos`/`MoveWindow`,
`GetDpiForWindow`, `MonitorFromWindow`, `GetMonitorInfoW`.

The adapter never adds Win32 input injection (`SendInput`, `mouse_event`, `keybd_event`, `PostMessage`
as a click/keystroke substitute). All real mouse/keyboard delivery is `winapp ui`'s responsibility;
duplicating it here would recreate exactly the ad-hoc, unclassified `SendInput` path that Issue #224
already showed reports success without observable effect on at least one host.

## 4. UIA vs. real-input selection

Use a UIA pattern operation (`invoke`, `get-value`/`set-value`, `wait-for`) whenever it tests the
intended behavior — native `Button` activation, a `ValuePattern`-backed edit, or a wait/assertion.
Use real input only when the behavior under test requires it: self-drawn controls, pointer routing,
hover, drag/drop, splitters, pointer capture, right-click/context requests, keyboard routing, or
focus behavior that depends on real key/mouse delivery. WinUI 3 real-key E2E always uses `winapp ui
send-keys ... --via send-input`; `PostMessage`-style keystroke injection is never an accepted
substitute for windowless XAML controls.

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

## 7. Screenshot modes

`capture-window` defaults to `winapp ui screenshot` targeting the exact HWND. A case whose evidence
requires a `MenuFlyout`, popup, tooltip, dropdown, or other overlay outside the owning window's own
paint must explicitly request the `--capture-screen` path, and the recorded evidence states which
mode was actually used — a case never silently reports one mode's result as equivalent to the other.

## 8. Evidence model

Each run uses the same Issue-scoped, immutable shape already established for AppKit:
`.agent-state/issues/<issue>/e2e/<head-short>/<run-id>/`, with separate stdout/stderr per action,
repository HEAD and `origin/master`, host OS version/architecture/session metadata, `winapp
--version`, `doctor` output, PID/HWND/geometry, postcondition results, and required screenshots.
Raw logs stay under `.agent-state`; only a small reviewer-facing result set is committed, and only
when the owning Issue requires durable evidence.

## 9. External dependency lifecycle

`winapp` is versioned and updated entirely outside this repository. The adapter's contract is
therefore defined against `winapp`'s documented JSON/exit-code behavior, not against a pinned
binary — `doctor`'s recorded version is the reproducibility anchor. If a future `winapp` version
removes or changes a verb this adapter depends on, that is reported as an exact
command/version/error against this design, not silently absorbed by switching to a different
automation framework.

## 10. Non-goals

No ElwindUI public API or WinUI3 backend behavior change. No second UI Automation implementation.
No vendored `winapp`. No Rust workspace crate for the driver (this is PowerShell, matching the
already-PowerShell Windows host workflow). This design does not execute or close Issues #224, #226,
or #157 — it is infrastructure those Issues' own verification work can build on.
