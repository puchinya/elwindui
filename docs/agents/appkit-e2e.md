# AppKit Native E2E Tester Guide

This guide defines the durable AppKit tester procedure and fixed tester instruction-sheet format.
It is separate from [`appkit.md`](appkit.md), so a fresh clone contains the complete tester
workflow. Any command snippet in this guide is a non-authoritative mechanics illustration, not a
fixed product instruction example -- durable product cases originate under
[`tests/e2e/`](../../tests/e2e/README.md). Raw GUI logs remain Issue-scoped evidence under the
owning Issue's immutable `.agent-state` run directory; commit only a small reviewer-facing evidence
subset when the owning Issue/workflow explicitly requires it.

## Codex and Claude Code routing and tester ownership

This tester routing is provider-neutral (see also `docs/agents/winui3-e2e.md`'s own copy of this
policy for WinUI3): both providers use the same bounded tester contract, evidence obligations,
retry rules, and PASS/FAIL/NOT RUN/BLOCKED semantics. Only the selected tester model and
provider-specific sub-agent mechanism differ:

```text
Codex:        GPT-5.6 Luna, standard reasoning effort (medium)
Claude Code:  Claude Haiku 4.5, normal/default reasoning configuration
              (do not enable extended thinking for routine E2E execution)
```

For every AppKit E2E request, the main agent must assign the real GUI execution to one bounded
sub-agent before invoking the driver itself, using its own provider's sub-agent mechanism (Codex:
the `elwindui-appkit-e2e-tester` skill, so the role is visibly a tester).

The assigned tester owns the complete case and must not delegate again, commit, push, or change
Issue/PR state unless explicitly assigned. The main agent reviews the source diff, evidence, and
PASS/FAIL/NOT RUN/BLOCKED classification before updating GitHub. This routing gate still applies
after context compaction and when a GUI process is already running. If no suitable sub-agent or
GUI-capable execution path is available, report BLOCKED rather than falling back to the main task.

## Durable case ownership

When executing a permanent repository E2E scenario, the scenario must originate under
[`tests/e2e/`](../../tests/e2e/README.md). Do not create AppKit-only permanent product scenarios
under `tools/macos-ui-driver/` or `docs/agents/`.

## Stable driver artifact and rebuild policy

Swift source is the development authority. The checked-in binary is the permission-stable native
E2E artifact:

```zsh
ROOT="$(git rev-parse --show-toplevel)"
BIN="$ROOT/tools/macos-ui-driver/bin/macos-ui-driver"
```

Use the checked-in binary for ordinary E2E. Do not rebuild or replace it during ordinary E2E or
because an unrelated PR is under review. Run the freshness check before native work:

```zsh
"$ROOT/tools/macos-ui-driver/verify-e2e-binary.sh"
"$BIN" doctor
```

Rebuild only when `Package.swift` or `Sources/**/*.swift` changes, the binary is missing/corrupt,
or the user explicitly requests driver remediation. After replacement, preserve mode `100755`,
update [`bin/PROVENANCE.md`](../../tools/macos-ui-driver/bin/PROVENANCE.md), rerun the freshness
check, and run `doctor` outside the workspace sandbox. If TCC permission is lost, native E2E is
BLOCKED until it is re-established. Never use a refresh sidecar for unrelated work.

## Fast execution and safety rules

- Run every driver invocation outside the Codex workspace-write sandbox.
- Run `doctor` once and require `success:true`, `accessibility:true`, and `screen_recording:true`.
- Launch the already-built target application/example once and reuse one healthy PID for
  compatible cases.
- Batch deterministic observations such as `list-windows`; do not relaunch or capture redundant
  images.
- Use one tester, one checked-in binary, one doctor, one target launch, and one PID for a
  compatible batch. Refresh window IDs and geometry after floating create/close, move, resize, and
  restore.
- Use one controlled retry at most, only after restoring foreground, target identity, geometry, and
  the expected precondition. After a second abnormal result, classify behavior mismatch as FAIL,
  host permission/session failure as BLOCKED, and an unexecuted case as NOT RUN.
- Keep stdout and stderr separate. A summary without the required numeric/window or image evidence
  is NOT RUN, never PASS.

## Fixed tester instruction-sheet format

The main agent must give the tester a concrete, case-scoped instruction sheet rather than asking
it to design a plan. Every sheet has these sections in this order:

1. **Scope and prohibitions** — exact cases, completion ownership, and no re-delegation, commit,
   push, or Issue/PR update.
2. **Fixed setup** — clone-relative paths, checked-in driver, exact freshness/`doctor`/launch
   commands, launch wait option, one-PID reuse rule, and required permissions.
3. **Exact actions** — commands in execution order, fixed case-local offsets, explicit focus/action
   grouping, and placeholders only for values read from the immediately preceding command.
4. **Expected results and stop rules** — exact JSON fields, tolerances, and conditions for PASS,
   FAIL, NOT RUN, or BLOCKED. Do not transfer design decisions to the tester.
5. **Evidence and cleanup** — immutable per-run directory, separate stdout/stderr paths, required
   screenshots and numeric values, compact report shape, and exact terminate command.

## Foreground/action grouping

Each driver process can leave the Codex window frontmost. For every GUI action or capture, run the
checked-in driver's `focus-window` and the action sequentially in the same host-context shell
invocation. If focus fails, do not run the action. The following helper makes that boundary
explicit:

```zsh
run_focused() {
  local target="$1" focus_stdout="$2" focus_stderr="$3" action_stdout="$4" action_stderr="$5"
  shift 5
  "$BIN" focus-window --pid "$PID" --window-id "$target" --timeout 5 \
    >"$focus_stdout" 2>"$focus_stderr" || return
  "$BIN" "$@" >"$action_stdout" 2>"$action_stderr"
}
```

This applies to `point-click`, `click`, `drag`, `resize`, `capture-window`, and keyboard input.
For a cross-window drag, focus the source window immediately before the drag.

## Window-relative coordinates

Custom-control points are not portable desktop-global coordinates. Derive screen coordinates from
the latest `list-windows` result immediately before the action:

```text
screen_x = current_window.x + case_local_x
screen_y = current_window.y + case_local_y
```

`case_local_x`/`case_local_y` are supplied by the selected durable case under `tests/e2e/`, not by
this guide -- for example, if the current window origin is `<window-x>,<window-y>` and the case
defines a target offset `<case-local-x>,<case-local-y>`, compute
`TARGET_X=$((WINDOW_X+CASE_LOCAL_X))` and `TARGET_Y=$((WINDOW_Y+CASE_LOCAL_Y))`. Do not reuse a
stale origin after moving or resizing a window.

## Immutable evidence and session metadata

Every run uses a new directory and never overwrites earlier evidence:

```zsh
ISSUE=<owning-issue-number>
CASE_ID=<case-id>
HEAD_SHORT="$(git rev-parse --short=12 HEAD)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="$ROOT/.agent-state/issues/$ISSUE/e2e/$HEAD_SHORT/$RUN_ID"
CASE="$RUN/$CASE_ID"
mkdir -p "$CASE"
```

Record `HEAD`, `origin/master`, driver SHA-256, source fingerprint and freshness result, macOS
version, architecture, `doctor` output, and target application/example SHA-256 in the run
directory. Raw logs remain
under the owning Issue's immutable `.agent-state` run directory; commit only a small
reviewer-facing evidence subset when the owning Issue/workflow explicitly requires it -- do not
invent a universal committed-evidence directory, and do not commit full `.agent-state` logs.

Native evidence is invalidated by effective changes to: the selected durable case definition or
its declared dependencies; AppKit/backend/core behavior relevant to that case; the target
example/application used by that case; or `macos-ui-driver` source or the checked-in driver
binary. Unrelated WinUI3-only or documentation-only changes do not invalidate AppKit evidence
unless the selected case explicitly depends on them.

## Executing a durable case

1. Select the durable case from [`tests/e2e/`](../../tests/e2e/README.md).
2. The main agent resolves that case into the fixed five-section tester instruction sheet.
3. The tester executes it through `macos-ui-driver`.
4. Evidence is stored under the owning Issue's immutable run directory.
5. The tester reports PASS / FAIL / NOT RUN / BLOCKED.
6. The tester does not modify the durable case definition during execution.

The following is a non-authoritative command illustration of driver mechanics, not a durable
product E2E test case:

```zsh
ROOT="$(git rev-parse --show-toplevel)"
BIN="$ROOT/tools/macos-ui-driver/bin/macos-ui-driver"
"$ROOT/tools/macos-ui-driver/verify-e2e-binary.sh"
"$BIN" doctor
"$BIN" launch --path "$ROOT/target/debug/<example>" --wait-window-timeout 5
# Read PID once from launch's own output and reuse it for every compatible step.
"$BIN" list-windows --pid "$PID"
run_focused() {
  local target="$1" focus_stdout="$2" focus_stderr="$3" action_stdout="$4" action_stderr="$5"
  shift 5
  "$BIN" focus-window --pid "$PID" --window-id "$target" --timeout 5 \
    >"$focus_stdout" 2>"$focus_stderr" || return
  "$BIN" "$@" >"$action_stdout" 2>"$action_stderr"
}
"$BIN" terminate --pid "$PID" --timeout 5
```

Use only PASS, FAIL, NOT RUN, or BLOCKED. Report one compact table containing case, status,
PID/window IDs, numeric evidence, and immutable run/log/image paths. The process must terminate
without force under normal conditions. The tester must not update Issue/PR state; the main agent
consumes the report and performs the GitHub workflow.

Historical evidence from prior durable AppKit cases (e.g. PR #221 / Issue #220's floating-bounds
and menu-lifetime verification) remains under `docs/issues/220-docking-ux-parity/evidence/` and
`.agent-state/issues/220/`; it is retained as historical record, not as a current permanent case
definition -- current durable case definitions originate from `tests/e2e/`.
