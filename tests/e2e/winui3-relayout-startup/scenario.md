# WinUI3 docking relayout startup

## Scope and ownership

This is the durable Windows/WinUI3 startup acceptance case for Issue #261 and PR #262. It verifies that \`docking-demo\` launches, paints its docking content, reaches a usable UIA/input checkpoint, settles its relayout and invalidation traces, remains responsive for the required idle window, and terminates normally.

The main agent assigns the complete GUI run to one bounded tester using `gpt-6-luna` with `medium` reasoning effort. The tester does not edit product code, this scenario, Git history, or GitHub state; it does not delegate further. The tester runs as a normal non-elevated user on an unlocked interactive desktop, uses the repository \`windows-ui-driver.ps1\`, allows at most one controlled retry, and always terminates the launched process.

## Fixed setup

Run from the repository root on a committed final PR #262 HEAD. Build the executable before the run with the canonical Windows environment. Run the driver doctor once for the session and save its JSON result. Create a fresh immutable evidence directory:

\`\`\`powershell
$Root = git rev-parse --show-toplevel
$Issue = 261
$HeadShort = git rev-parse --short=12 HEAD
$RunId = (Get-Date -AsUTC).ToString('yyyyMMddTHHmmssZ')
$Run = "$Root\\.agent-state\\issues\\$Issue\\e2e\\$HeadShort\\$RunId"
New-Item -ItemType Directory -Force -Path $Run | Out-Null
$Driver = "$Root\\tools\\windows-ui-driver\\windows-ui-driver.ps1"
pwsh -NoProfile -File $Driver doctor | Tee-Object "$Run\\doctor.json"
\`\`\`

Record \`git rev-parse HEAD\`, \`git rev-parse origin/master\`, Windows version/build, the driver version/session/input-desktop probe, and the process-start UTC timestamp in the run directory.

The case-owned \`launch-docking-demo.ps1\` wrapper starts the already-built \`target\\debug\\docking-demo.exe\` and redirects application stdout/stderr to the run directory. Its single JSON result provides the PID and log paths. Driver stdout remains reserved for the driver's single JSON result.

## Exact actions

1. Record the repository/runtime setup values and run \`doctor\` once.
2. Start the wrapper and record its JSON result and process-start timestamp.
3. Read the wrapper PID, then use \`list-windows --pid <pid>\` until the exact window titled \`ElwindUI Docking Demo\` appears. Record HWND, title, left/top/width/height, and the first successful timestamp. This is \`time_to_window\`.
4. Use \`wait-for --hwnd <hwnd> --timeout-ms 30000\` and then \`search --hwnd <hwnd> --query "Document A" --max 10\`. Record every JSON result. If the stable content element is not exposed by UIA, classify the UIA checkpoint as BLOCKED rather than inventing a selector.
5. Capture a window screenshot with \`capture-window --hwnd <hwnd> --output "$Run\\docking-demo-painted.png"\`. If the window capture is blank or suspiciously small, retry once with \`--capture-screen\` and record both results. Inspect the screenshot: a visible docking surface and text/chrome must be present; a top-level window with black/empty content is not painted-ready. Record the first successful painted timestamp as \`time_to_painted\`.
6. Prove the UI is usable by resolving a known content item or status element from the immediately preceding \`search\` result and using the returned selector with the appropriate UIA operation. Prefer \`get-property\`/\`get-value\`/\`set-focus\`; use real input only if UIA cannot test the intended operation. Record the result and timestamp as \`time_to_interactive\` only after the operation reaches its expected postcondition.
7. Keep the process alive for the required 10-second idle interval. During the interval, capture process CPU samples at one-second resolution, and record whether the application remains responsive and whether the relayout/invalidation trace file grows.
8. Parse the redirected stderr log after the idle interval. Aggregate invalidation requests by caller file:line, kind, request count, and distinct render-group count. Aggregate cycles by host, \`RelayoutSource\`, and claimed strongest \`InvalidationKind\`. Record text-measure counters and cumulative time. Do not edit the trace or discard failed data.
9. Terminate with \`terminate --pid <pid> --timeout 5\`, record the JSON result and whether force was required, then verify the process has exited.

## Expected results and classification

PASS requires all of the following on the exact committed HEAD: process launch; exact top-level window; painted screenshot with non-empty docking content; a usable UIA/input checkpoint; no recurring stable-state invalidation sequence after startup; counters stable during the 10-second idle window; numeric CPU evidence; and normal termination.

FAIL means the driver reached the product but the product state or postcondition was wrong, such as black/empty content, a nonresponsive window, continuing stable-state invalidation, or abnormal termination after the retry.

BLOCKED means the host/tool/session/security/desktop or UIA surface prevented exercising the product, including missing \`winapp\`, \`no_interactive_desktop\`, an unrecoverable foreground issue, or a missing stable UIA/input surface. NOT RUN means required evidence was never collected.

Do not classify a visible top-level HWND alone as painted or interactive. Do not infer idle CPU or counter stability without the 10-second samples and log comparison.

## Evidence and cleanup

The immutable evidence directory is \`.agent-state/issues/261/e2e/<head-short>/<run-id>/\`. Store the doctor, setup, wrapper launch, list/search/wait/UIA/input/screenshot/CPU/termination JSON, redirected stdout/stderr, parsed aggregation, screenshot, and a compact \`result.md\` there. Record the exact HEAD and origin/master SHA in the result. Never overwrite an earlier run directory. The main agent consumes the result and may attach the screenshot to Issue #261 only if it materially supports the painted-ready acceptance; text/log artifacts remain local.

Cross-PR T9/T10 may be run only through their existing instructions when available, and are reported separately from this Issue #261 startup acceptance. AppKit is not exercised by this WinUI3-specific case.
