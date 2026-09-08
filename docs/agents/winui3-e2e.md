# WinUI3 Native E2E Tester Guide

This is the complete tester workflow, fixed instruction-sheet format, and non-authoritative driver
command illustration for native WinUI3 GUI acceptance. It is separate from
[`winui3.md`](winui3.md), so a fresh clone contains the complete tester workflow. Raw GUI logs
remain Issue-scoped evidence under `.agent-state/issues/<issue>/e2e/<head>/<run-id>/`; a small
reviewer-facing result set may be committed when the owning Issue requires durable evidence.

## Codex and Claude Code routing and tester ownership

This routing is provider-neutral: Codex and Claude Code use the same bounded tester contract,
instruction-sheet structure, evidence obligations, retry rules, and PASS/FAIL/NOT RUN/BLOCKED
semantics. Only the selected tester model and each provider's own sub-agent mechanism differ.

```text
Codex:        GPT-5.6 Luna, standard reasoning effort (medium)
Claude Code:  Claude Haiku 4.5, normal/default reasoning configuration
              (do not enable extended thinking for routine E2E execution)
```

For every WinUI3 E2E request, the main agent must assign the real GUI execution to one bounded
tester sub-agent before invoking the driver itself, using its provider's own sub-agent mechanism.
The assigned tester owns the complete case and must not delegate again, commit, push, or change
Issue/PR state unless explicitly assigned; it must not modify product code, relax or reinterpret
acceptance criteria, or invent a replacement command/mechanism when the supplied instruction
conflicts with repository/host reality -- it stops the affected case and reports BLOCKED with the
exact conflict instead. The main agent reviews the source diff, evidence, and PASS/FAIL/NOT
RUN/BLOCKED classification before updating GitHub; failure diagnosis, architecture judgment,
remediation design, and product-code modification belong to the main agent, not the tester. If no
suitable sub-agent or GUI-capable execution path is available, report BLOCKED rather than
performing the E2E in the main agent. This routing gate still applies after context compaction and
when a GUI process is already running.

## External prerequisite and `doctor`

```powershell
winget install Microsoft.winappcli --source winget
```

Run `windows-ui-driver.ps1 doctor` once per session and record its `winapp_version`,
`session_id`, and `input_desktop_probe`. A missing `winapp` is `category: "tool_error"` with the
install command above -- that state blocks native E2E; it is not silently worked around with
another automation framework.

## Host-context execution

Run every driver invocation outside any agent sandbox, as a normal non-elevated user, on an
unlocked interactive desktop -- see [`winui3.md`](winui3.md). Sandbox-only failures (missing
interactive desktop, package/App Runtime access errors) are not product failures until reproduced
in host context.

## Fast execution rules

- Run `doctor` once per session, not once per case.
- Launch the already-built demo once and reuse one healthy PID/HWND for a compatible batch of
  cases. Refresh window geometry after move, resize, or any topology change (floating window
  create/close, dock/undock, monitor/DPI transition) -- never reuse a stale coordinate.
- Prefer UIA pattern commands (`invoke`, `get-value`, `wait-for`) over real input wherever they
  test the intended behavior; they need no foreground and run headless. Reserve real input
  (`point-click`, `drag`, `send-keys`) for what actually requires it (see the driver
  [`README.md`](../../tools/windows-ui-driver/README.md)'s "UIA vs. real input" section).
- Use one controlled retry at most, only after restoring foreground, target identity, geometry, and
  the expected precondition. After a second abnormal result, classify a behavior mismatch as FAIL,
  a host/session/security/foreground condition as BLOCKED, and an unexecuted case as NOT RUN.
- Keep stdout and stderr separate. A summary without the required numeric/window evidence or
  screenshot is NOT RUN, never PASS.

## Fixed tester instruction-sheet format

The main agent must give the tester a concrete, case-scoped instruction sheet, not a request to
design a plan. Every sheet has these five sections in this order:

1. **Scope and prohibitions** -- exact cases, completion ownership, no re-delegation/commit/push/
   Issue-PR update.
2. **Fixed setup** -- clone-relative driver path, exact `doctor`/`launch` commands, one-PID/HWND
   reuse rule, required host-context conditions.
3. **Exact actions** -- commands in execution order, fixed case-local coordinates/selectors, and
   placeholders only for values read from the immediately preceding command's own output.
4. **Expected results and stop rules** -- exact JSON fields, tolerances, and the conditions for
   PASS, FAIL, NOT RUN, or BLOCKED. Do not transfer design decisions to the tester.
5. **Evidence and cleanup** -- immutable per-run directory, required screenshots/numeric values,
   compact report shape, and the exact `terminate` command.

## Foreground and action grouping

`focus-window` is not a mandatory prerequisite for real-input commands (`point-click`, `drag`,
`send-keys --via send-input`) -- `winapp`'s own real-input verbs bring their target to the
foreground themselves as part of injecting input, and fail fast (`environment_blocker`) rather than
acting on the wrong window. This driver's own `focus-window` (a plain `SetForegroundWindow`) is
separately subject to Windows' anti-focus-stealing restriction when called from a non-interactive
process and was observed to reliably report `BLOCKED` in exactly that situation even against a
healthy, responsive target `winapp` could still act on correctly. Use it only as an explicit
diagnostic, or for a case whose own subject is foreground behavior. UIA-pattern commands need no
foreground at all -- they run headless. For real input, deliver the action immediately after
resolving the exact HWND from the most recent `list-windows`/`search`.

## Window-relative coordinates

Custom/self-drawn `point-click`/`drag` coordinates are never portable desktop-global constants.
Derive them from the most recent `list-windows`/`search` result immediately before the action:

```text
screen_x = current_window.left + case_local_x
screen_y = current_window.top  + case_local_y
```

Recompute after every move, resize, or topology change; never reuse a coordinate captured before a
preceding UIA action or focus change.

## Immutable evidence

```powershell
$Issue = <owning-issue-number>
$HeadShort = (git rev-parse --short=12 HEAD)
$RunId = (Get-Date -AsUTC).ToString('yyyyMMddTHHmmssZ')
$Run = "$Root\.agent-state\issues\$Issue\e2e\$HeadShort\$RunId"
New-Item -ItemType Directory -Force -Path $Run | Out-Null
```

Record repository HEAD, `origin/master`, Windows version/build, `winapp --version`, `doctor`
output, PID/HWND/geometry, and every action's JSON result and required screenshot under that run
directory. Never overwrite an earlier run's directory.

## PASS / FAIL / NOT RUN / BLOCKED

- **PASS** -- the action was delivered/executed and the required application postcondition or
  evidence was actually observed (a driver command's own `success: true` is not sufficient by
  itself -- see the design doc's Section 5).
- **FAIL** -- the action reached the product (delivered input, or a UIA command that resolved its
  target) but the resulting product state was wrong.
- **NOT RUN** -- the case, or its required evidence, was never executed/collected.
- **BLOCKED** -- a host/tool/session/security/foreground condition (`environment_blocker`,
  `tool_error`) prevented the action from exercising the product at all. `no_interactive_desktop`
  and an unrecoverable `foreground_not_target` are always BLOCKED, never FAIL.

## Cleanup

Always end a batch with `terminate --pid <pid> --timeout 5`, even after an abnormal result, and
record whether it needed to force-kill. Do not leave a healthy launched app running after a
completed run.

## Durable case ownership

This guide owns the WinUI3 tester procedure, not the set of permanent product E2E cases. Durable
product/application E2E scenarios originate under [`tests/e2e/`](../../tests/e2e/README.md); this
driver and guide only execute them. Do not create WinUI3-only permanent product scenarios under
`tools/windows-ui-driver/` or this `docs/agents/` guide.

## Executing a durable case

1. Select the durable case from `tests/e2e/`.
2. The main agent resolves that case into a fixed tester instruction sheet (the five-section format
   above), filling in the case's concrete setup/actions/expected-results/evidence.
3. The tester executes the instruction sheet through `windows-ui-driver.ps1`, per the fast
   execution and foreground rules above.
4. Evidence is stored under the owning Issue's run directory (see "Immutable evidence" above).
5. The tester returns PASS / FAIL / NOT RUN / BLOCKED for each case, per the classification above.
6. The tester does not modify the durable case definition during execution -- a case defect or gap
   is reported back to the main agent, not silently patched by the tester.

The following is a non-authoritative command illustration of driver mechanics, not a durable
product E2E test case:

```powershell
$Root = git rev-parse --show-toplevel
$D = "$Root\tools\windows-ui-driver\windows-ui-driver.ps1"
pwsh -NoProfile -File $D doctor
pwsh -NoProfile -File $D launch --path "$Root\target\debug\<example>.exe" --wait-window-timeout 10
# Read pid and window.hwnd from launch's own JSON output and reuse them for every following step.
pwsh -NoProfile -File $D wait-for --hwnd <hwnd> --selector Default --timeout-ms 5000
pwsh -NoProfile -File $D search --hwnd <hwnd> --query <element-name>
pwsh -NoProfile -File $D invoke --hwnd <hwnd> --selector <selector-from-search>
pwsh -NoProfile -File $D terminate --pid <pid> --timeout 5
```

A freshly-appeared HWND does not guarantee its UIA tree is populated yet -- wait for a known
element before the first UIA action rather than querying immediately. The process must terminate
without force under normal conditions; if force was required, report that explicitly. The tester
must not update Issue/PR state -- the main agent consumes the report and performs the GitHub
workflow.
