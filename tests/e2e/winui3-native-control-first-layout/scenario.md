# WinUI3 NativeControl first stable layout

## Scope and ownership

This is the durable Windows/WinUI3 acceptance case for Issue #265. It verifies that the existing
`accessibility-semantics-demo` reaches its first stable native-control layout from framework
lifecycle events alone, with no manual resize, hide/show, focus, pointer/keyboard input, synthetic
`SizeChanged`, recurring `LayoutUpdated`, or arbitrary timer/sleep repair.

The main agent assigns each complete GUI run to one bounded tester using GPT-5.6 Luna with medium
reasoning. The tester does not edit product code, this scenario, Git history, or GitHub state; it
does not delegate further. The tester runs as a normal non-elevated user on an unlocked interactive
desktop, uses the repository `windows-ui-driver.ps1`, allows at most one controlled retry, and
always terminates the launched process.

## Fixed setup

Run twice from clean launches at the exact final committed Issue #265 HEAD. Build
`examples/accessibility-semantics-demo` with the canonical Windows environment before the runs.
Run `doctor` once per session and create a fresh immutable evidence directory for each run:

```powershell
$Root = git rev-parse --show-toplevel
$Issue = 265
$HeadShort = git rev-parse --short=12 HEAD
$RunId = (Get-Date -AsUTC).ToString('yyyyMMddTHHmmssZ')
$Run = "$Root\.agent-state\issues\$Issue\e2e\$HeadShort\$RunId"
New-Item -ItemType Directory -Force -Path $Run | Out-Null
$Driver = "$Root\tools\windows-ui-driver\windows-ui-driver.ps1"
pwsh -NoProfile -File $Driver doctor | Tee-Object "$Run\doctor.json"
pwsh -NoProfile -File $Driver launch --path "$Root\target\debug\accessibility-semantics-demo.exe" | Tee-Object "$Run\launch.json"
```

Record `git rev-parse HEAD`, `git rev-parse origin/master`, Windows version/build, driver version,
session/input-desktop probe, PID, HWND, and process-start UTC timestamp. Do not reuse the PID,
HWND, UIA identity, or evidence directory between runs.

## Exact actions

1. Launch the clean fixture and resolve the exact top-level window from the launch result or
   `list-windows --pid <pid>`. Record the first window geometry and require the requested client
   presentation to be `620x560` within the driver's normal window-reporting convention.
2. Before any user-generated action, use `wait-for --hwnd <hwnd> --timeout-ms 30000` for the
   fixture's semantic root and search for `a11y-text-area`, `a11y-check-box`, `a11y-slider`,
   `a11y-canvas`, and `a11y-exiting-text-area`. Record every JSON result and the first stable
   checkpoint timestamp. No resize, hide/show, focus, pointer, keyboard, tab, or synthetic event
   may occur before this checkpoint.
3. From the immediately preceding search results, inspect the semantic/native properties and bounds
   for each required identifier. Record screen/root-relative bounds, role, and availability. The
   TextArea, CheckBox, Slider, Canvas/removal rows must have nonzero required native/semantic bounds,
   monotonically increasing vertical positions, and no overlapping row intervals. Do not use a
   delayed screenshot or arbitrary sleep as the condition that makes this pass.
4. Capture the first stable window with `capture-window --hwnd <hwnd> --output
   "$Run\first-stable.png"`. Inspect it for complete separated rows in the expected order. If the
   window-owned capture is blank or suspiciously small, retry once with `--capture-screen` and record
   both results; a screenshot is supporting evidence, not a substitute for numeric bounds.
5. After the initial acceptance, perform one normal `resize-window` operation to a bounded larger
   size, reacquire window geometry, wait for the semantic root, and repeat the bounds/order check.
   Record that the same rows remain ordered, non-overlapping, and updated for the new viewport.
6. Terminate with `terminate --pid <pid> --timeout 5`, record the result and whether force was
   required, then verify normal process exit.

The case does not invoke buttons, focus, text entry, slider actions, or removal actions; those
behaviors belong to the shared accessibility scenario. The only post-acceptance mutation is the
single normal Window resize required to preserve #225 viewport tracking coverage.

## Expected results and classification

PASS requires: clean launch; exact 620x560 requested fixture; first stable checkpoint reached without
a user-generated second event; nonzero and non-overlapping ordered TextArea/CheckBox/Slider/Canvas/
removal geometry; a valid complete screenshot; successful normal resize tracking; and normal process
termination. The evidence must show that readiness caused one settled presentation rather than a
per-control relayout cascade when diagnostics/counters are enabled by the test harness.

FAIL means the driver reached the product but initial bounds are zero/overlapping, the expected rows
are missing, the screenshot is incomplete, resize tracking is wrong, or termination is abnormal.
BLOCKED means the host/tool/session/security/desktop or UIA surface prevented exercising the product,
including missing `winapp`, `no_interactive_desktop`, or an unrecoverable foreground/session issue.
NOT RUN means required evidence was never collected.

## Evidence and cleanup

Store doctor, setup, launch, window, wait/search/property/bounds, screenshot, resize, and termination
JSON plus a compact `result.md` under `.agent-state/issues/265/e2e/<head-short>/<run-id>/`. Include
the exact HEAD, fresh PID/HWND/UIA identity, window geometry, first-stable timestamp, numeric row
bounds, overlap/order calculation, screenshot path, resize result, and process-exit result. Never
overwrite an earlier run. The main agent may attach a valid screenshot to Issue #265 with `gh
--attach`; raw logs remain Issue-scoped evidence. AppKit is not exercised by this WinUI3-specific
case.
