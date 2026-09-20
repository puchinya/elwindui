# WinUI3 NativeControl first stable layout

## Scope and ownership

This is the durable Windows/WinUI3 acceptance case for Issue #265. It verifies the first
NativeControl layout from framework lifecycle events alone. This case is deliberately visual and
diagnostic: it uses the process, top-level HWND geometry, screenshots, redirected application
output, and bounded WinUI3 diagnostics. Semantic-accessibility acceptance remains owned by #256 /
PR #264 after this fix is integrated.

The main agent assigns each complete GUI run to one bounded tester using GPT-5.6 Luna with medium
reasoning. The tester does not edit product code, this scenario, Git history, or GitHub state; it
does not delegate further. The tester runs as a normal non-elevated user on an unlocked interactive
desktop, uses the repository `windows-ui-driver.ps1`, allows at most one controlled retry, and
always terminates the launched process.

## Fixed setup

Run twice from clean launches at the exact final committed Issue #265 HEAD. Build
`examples/accessibility-semantics-demo` with the canonical Windows environment before the runs.
Run `doctor` once per session and create a fresh immutable evidence directory for each run. The
case-owned launcher is required because the driver launch result does not capture application
stdout/stderr.

```powershell
$Root = git rev-parse --show-toplevel
$Issue = 265
$Head = git rev-parse HEAD
$HeadShort = git rev-parse --short=12 HEAD
$RunId = (Get-Date -AsUTC).ToString('yyyyMMddTHHmmssZ')
$Run = "$Root\.agent-state\issues\$Issue\e2e\$HeadShort\$RunId"
New-Item -ItemType Directory -Force -Path $Run | Out-Null
$Driver = "$Root\tools\windows-ui-driver\windows-ui-driver.ps1"
$Launcher = "$Root\tests\e2e\winui3-native-control-first-layout\launch-accessibility-semantics.ps1"
pwsh -NoProfile -File $Driver doctor | Tee-Object "$Run\doctor.json"
pwsh -NoProfile -File $Launcher `
  -Executable "$Root\target\debug\accessibility-semantics-demo.exe" `
  -EvidenceDirectory $Run | Tee-Object "$Run\launch.json"
```

Record `$Head`, `origin/master`, Windows version/build, driver version, session/input-desktop
probe, PID, top-level HWND, process-start UTC timestamp, and the redirected stdout/stderr paths.
Do not reuse PID, HWND, or evidence directory between runs.

## Exact actions

1. Resolve the exact top-level window from the launcher PID with `list-windows --pid <pid>`. Record
   the first geometry and require the requested presentation to be `620x560` within the driver's
   normal window-reporting convention.
2. Before any user-generated action, poll only the redirected logs and `list-windows` result until
   the explicit diagnostic record `native_load_batch_complete` is present and the target window is
   non-empty. Polling is bounded and only waits for those named conditions; it is not a timer-based
   layout repair. Record each bounded poll result and the first-stable timestamp.
3. Parse the captured WinUI3 diagnostics. In this repository revision the fixture's initial
   declarative tree is realized by two framework-owned reconciliation batches: a five-member
   primary batch (Activate Button, TextArea, CheckBox, Slider, and removal Button), followed by
   a one-member conditional/transition batch for the exiting TextArea. This is an observed fixture
   lifecycle detail, not a user-generated event. Require exactly those two
   `native_load_batch`/`native_load_batch_complete` pairs, one completion with `saw_loaded=true`
   per batch, and exactly one `relayout_realization` for the same host with
   `source=InteractiveFlush`, `kind=Measure`, and `realized=true` per batch after its bootstrap/
   queued records. The acceptance is per reconciliation batch: it must not count bootstrap or
   queued realization as readiness work, and must not permit more than one readiness realization
   for either batch.
4. Parse the final `native_projection_rect` records for that host. Require positive width and
   height for all six NativeControl rectangles, monotonically increasing row Y positions, and no
   overlapping row intervals. The self-drawn Canvas row is checked visually in the screenshot; it
   is not counted as a native projection rectangle.
5. Capture the first stable window:

   ```powershell
   pwsh -NoProfile -File $Driver capture-window --hwnd <hwnd> --output "$Run\first-stable.png"
   ```

   Inspect the image for a complete, non-black presentation: title/result area, separated native
   rows, the self-drawn Canvas row, removal row, and exiting TextArea must be visible in vertical
   order. If the window-owned capture is blank or suspiciously small, retry once with
   `--capture-screen` and record both results. The screenshot is the visual acceptance evidence;
   no semantic-tree query is part of this case.
6. After the initial acceptance, perform one normal resize and reacquire the window geometry:

   ```powershell
   pwsh -NoProfile -File $Driver resize-window --hwnd <hwnd> --width 760 --height 620
   pwsh -NoProfile -File $Driver list-windows --pid <pid>
   ```

   Wait only for the explicit post-resize diagnostic/projection change, capture
   `$Run\after-resize.png`, and require positive ordered non-overlapping rectangles and a coherent
   screenshot after resize. Record the resize `relayout_realization` source/kind and final
   projected rectangles.
7. Terminate normally and verify exit:

   ```powershell
   pwsh -NoProfile -File $Driver terminate --pid <pid> --timeout 5
   ```

   Record whether force was required and the final process-exit result.

The case does not invoke controls, set focus, send pointer/keyboard input, hide/show the window,
or create a synthetic size event. The single post-acceptance mutation is the normal Window resize
required to preserve #225 viewport tracking coverage.

## Expected results and classification

PASS requires: clean launch at the exact final HEAD; requested 620x560 fixture; the two observed
startup reconciliation batches (5 + 1); exactly one readiness-driven authoritative full-host
Measure realization per batch (two total for this fixture); six positive, ordered,
non-overlapping NativeControl rectangles; a valid complete screenshot; coherent normal resize
evidence; no recurring relayout cascade; and normal process termination. Correctness must be
established before the resize and without a user-generated second event.

FAIL means the product launched but the diagnostics show a missing/multiple readiness batch, more
than one readiness-driven full-host Measure realization, zero/overlapping/incorrect projected
rectangles, an incomplete screenshot, incorrect resize tracking, a relayout storm, or abnormal
termination.

BLOCKED means the host/tool/session/security/desktop or capture surface prevented exercising the
product, including missing `winapp`, `no_interactive_desktop`, or an unrecoverable foreground/
session issue.

NOT RUN means required evidence was never collected.

## Evidence and cleanup

Store `doctor.json`, `launch.json`, `list-windows` JSON, redirected stdout/stderr, bounded poll
records, `first-stable.png`, optional screen-capture retry, `after-resize.png`, resize/window JSON,
termination JSON, parsed diagnostics, and a compact `result.md` under
`.agent-state/issues/265/e2e/<head-short>/<run-id>/`. `result.md` must include:

* exact HEAD and requested/observed window geometry;
* fresh PID/HWND and process-start UTC;
* startup batch hosts and member counts;
* readiness completion record;
* readiness-driven full-host pass count per reconciliation batch, which must be exactly one;
* final NativeControl rectangles, positive-size/order/overlap calculations;
* native leaf measure count as supporting evidence only, never as the full-host pass assertion;
* screenshot paths and visual inspection result;
* post-resize diagnostics/screenshot result;
* termination and force-kill result;
* `UIA commands used: none` and `semantic-bounds acceptance: deferred to PR #264`.

Never overwrite an earlier run. Raw logs remain Issue-scoped evidence. Attach a valid screenshot to
Issue #265 only from the main agent after the run has passed. AppKit is not exercised by this
WinUI3-specific case.
