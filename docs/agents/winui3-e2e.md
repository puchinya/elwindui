# WinUI3 Native E2E Tester Guide

This is the durable procedure for native WinUI3 GUI acceptance. It is separate from
[`winui3.md`](winui3.md), so a fresh clone contains the complete tester workflow and its fixed
instruction example. Raw GUI logs remain Issue-scoped evidence under
`.agent-state/issues/<issue>/e2e/<head>/<run-id>/`; a small reviewer-facing result set may be
committed when the owning Issue requires durable evidence.

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

`focus-window` and real-input commands (`point-click`, `drag`, `send-keys --via send-input`) are
each their own separate `windows-ui-driver.ps1` process invocation, so nothing about "focus, then
act in the same shell" is implicit -- for UIA-pattern commands this does not matter (they run
headless); for real input, deliver the action immediately after resolving the exact HWND, since
`winapp`'s own real-input verbs bring their target to the foreground as part of injecting input
and fail fast (`environment_blocker`) rather than acting on the wrong window.

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
$Issue = 242
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

## Copy/paste example: theme-demo Ocean/Solarized transitions

This is a complete fixed instruction sheet. Replace only placeholders explicitly marked as values
read from the immediately preceding command.

### Scope and prohibitions

Run exactly these two cases against `theme-demo`: a UIA theme transition (Ocean) and a real-mouse
theme transition (Solarized). Own both cases to completion; do not delegate again, commit, push,
or update Issue/PR state.

### Fixed setup

```powershell
$Root = git rev-parse --show-toplevel
$D = "$Root\tools\windows-ui-driver\windows-ui-driver.ps1"
pwsh -NoProfile -File $D doctor
pwsh -NoProfile -File $D launch --path "$Root\target\debug\theme-demo.exe" --wait-window-timeout 10
```

Read `pid` and `window.hwnd` from `launch`'s own JSON output and reuse them for every following
step. Required setup result: `doctor.success == true`, `launch.success == true` with exactly one
`window`. If either fails, stop as BLOCKED with both JSON results.

```powershell
pwsh -NoProfile -File $D wait-for --hwnd <hwnd> --selector Default --timeout-ms 5000
```

A freshly-appeared HWND does not guarantee its UIA tree is populated yet -- wait for a known
element before the first UIA action rather than querying immediately. If this times out, stop as
BLOCKED.

### Exact actions

1. UIA theme transition:

   ```powershell
   pwsh -NoProfile -File $D search --hwnd <hwnd> --query Ocean
   ```

   Read the `Button`-typed match's `selector` from the result.

   ```powershell
   pwsh -NoProfile -File $D invoke --hwnd <hwnd> --selector <ocean-button-selector>
   pwsh -NoProfile -File $D search --hwnd <hwnd> --query Ocean
   ```

   PASS requires the second `search`'s matches to include a `Text`-typed element named exactly
   `Ocean`. `invoke` reporting `success: true` alone is NOT RUN/insufficient without this
   postcondition check.

2. Real-mouse theme transition:

   ```powershell
   pwsh -NoProfile -File $D search --hwnd <hwnd> --query Solarized
   ```

   Read the `Button`-typed match's `x`/`y`/`width`/`height`; compute the center point.

   ```powershell
   pwsh -NoProfile -File $D point-click --hwnd <hwnd> --x <center-x> --y <center-y>
   pwsh -NoProfile -File $D search --hwnd <hwnd> --query Solarized
   ```

   PASS requires the second `search`'s matches to include a `Text`-typed element named exactly
   `Solarized`. If `point-click` itself fails with `category: "environment_blocker"`, classify
   BLOCKED and cross-reference #224 rather than FAIL. If it reports success but the label never
   changes, classify FAIL -- this is the exact defect class #224 documents (injection success
   without observed delivery).

### Expected results, report, and cleanup

Use only PASS, FAIL, NOT RUN, or BLOCKED. Report one compact table containing case, status,
PID/HWND, the resolved selectors/coordinates, and the immutable run directory path. Finish with:

```powershell
pwsh -NoProfile -File $D terminate --pid <pid> --timeout 5
```

The process must terminate without force under normal conditions; if force was required, report
that explicitly. The tester must not update Issue/PR state -- the main agent consumes the report
and performs the GitHub workflow.
