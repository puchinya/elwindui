# windows-ui-driver

A thin, repository-owned PowerShell adapter for Windows native E2E — process/window control, UI
Automation (UIA) inspection/actions, real mouse/keyboard input, and screenshot capture — for
`elwindui` (or any Windows app). It gives the repository a stable command surface while delegating
all UIA/input/capture work to the external Microsoft `winapp` CLI. See
[`docs/design/tools/windows_ui_driver_design.md`](../../docs/design/tools/windows_ui_driver_design.md)
for the architecture and [`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md) for the
operational tester procedure.

## Prerequisite: `winapp`

This driver never vendors or auto-installs `winapp`. Install it once, manually:

```powershell
winget install Microsoft.winappcli --source winget
```

Verify with `doctor` (below). A missing `winapp` is reported as `category: "tool_error"` with this
exact install command, not a stack trace.

## `doctor`

Run this first. It reports `winapp`'s version, an input-desktop probe, and the current foreground
HWND -- but never claims real-mouse-input capability from environment inspection alone; that is
only proven by a live case whose application postcondition actually changed.

```powershell
pwsh -NoProfile -File .\windows-ui-driver.ps1 doctor
```

## Command examples

```powershell
$D = '.\windows-ui-driver.ps1'
pwsh -NoProfile -File $D launch --path C:\path\to\app.exe --wait-window-timeout 10
pwsh -NoProfile -File $D list-windows --pid <pid>
pwsh -NoProfile -File $D focus-window --hwnd <hwnd> --timeout 3
pwsh -NoProfile -File $D search --hwnd <hwnd> --query "Ocean"
pwsh -NoProfile -File $D invoke --hwnd <hwnd> --selector <selector-from-search>
pwsh -NoProfile -File $D point-click --hwnd <hwnd> --x <screen-x> --y <screen-y>
pwsh -NoProfile -File $D drag --hwnd <hwnd> --from-x <x1> --from-y <y1> --to-x <x2> --to-y <y2>
pwsh -NoProfile -File $D capture-window --hwnd <hwnd> --output shot.png
pwsh -NoProfile -File $D capture-window --hwnd <hwnd> --output shot.png --capture-screen
pwsh -NoProfile -File $D move-window --hwnd <hwnd> --left 100 --top 100
pwsh -NoProfile -File $D resize-window --hwnd <hwnd> --width 800 --height 600
pwsh -NoProfile -File $D terminate --pid <pid> --timeout 5
```

Every command prints exactly one JSON object to stdout (`{"success": true, ...}` or
`{"success": false, "category": "...", "error": "..."}`) and sets the process exit code
accordingly (0/1). `category` is one of `tool_error`, `environment_blocker`, `target_error`,
`usage_error` -- see the design doc's Section 5 for the full boundary. A command's own
`success: true` proves only that the driver operation executed; it is never proof that the target
application's state actually changed -- verify that separately (`search`/`get-value`/`wait-for`).

## UIA vs. real input

Use a UIA pattern command (`invoke`, `get-value`/`set-focus`, `wait-for`) whenever it tests the
intended behavior -- it works headless and needs no foreground. Use real input (`point-click`,
`drag`, `send-keys`) only when the behavior itself requires it: self-drawn controls, pointer
routing, drag/drop, splitters, right-click/context requests, or keyboard routing. Real-input
commands bring their target to the foreground themselves as part of delivering input (and fail
fast, classified `environment_blocker`, if they can't) -- do not call `focus-window` first as a
matter of course; it uses a plain `SetForegroundWindow`, which is subject to Windows' anti-focus-
stealing restriction when called from a non-interactive/background process and was observed to
reliably report `BLOCKED` in exactly that situation even against a healthy target window.

## Host-context requirement

Real GUI verification (`launch` targeting a live app, any real-input command, `capture-window`)
must run outside any sandbox, as a normal non-elevated user, on an unlocked interactive desktop --
see [`docs/agents/winui3.md`](../../docs/agents/winui3.md) and
[`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md).

## Exact HWND targeting

Once a window's HWND is known (from `launch` or `list-windows`), address it with `--hwnd`, not
`--pid` -- a process can own more than one top-level window, and HWND is stable across title/tab
changes. Custom/self-drawn coordinates for `point-click`/`drag` must be recomputed from a fresh
`list-windows`/`search` result immediately before the action -- never reused across a move,
resize, or window-topology change.

## Popup/overlay screenshots

`capture-window` defaults to `winapp ui screenshot`'s own window-owned (WGC) capture path, which
does not include popups, flyouts, tooltips, or other overlays outside the target window's own
paint. Pass `--capture-screen` for those. On at least one host in this repository's own testing,
the default WGC path returned a blank/near-empty PNG while `--capture-screen` captured correctly --
if a capture looks suspiciously small or blank, try `--capture-screen` before assuming the target
state is wrong.

## Tests

```powershell
pwsh -NoProfile -File .\tests\driver-contract.ps1
```

Deterministic adapter-contract tests against `tests\fake-winapp.ps1` (`ELWINDUI_WINAPP_PATH`
override) -- no real `winapp`, no real GUI process. Run this before live GUI testing.

```powershell
pwsh -NoProfile -File .\tests\theme-demo-e2e.ps1 -Issue <issue-number>
```

Live smoke test against the real `theme-demo` example -- host-context only. See
[`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md) for the full tester procedure this
is meant to be run under.

## No-vendoring rule

`winapp` is external, Microsoft-maintained, and versioned independently of this repository. Do not
check its executable into this repository and do not have this driver install or update it as a
side effect of running -- see the design doc's Section 9 for why the external tool's own `doctor`
version output, not a pinned binary, is this driver's reproducibility anchor.
