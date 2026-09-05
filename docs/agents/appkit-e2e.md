# AppKit Native E2E Tester Guide

This is the durable procedure for native AppKit GUI acceptance. It is separate from
[`appkit.md`](appkit.md), so a fresh clone contains the complete tester workflow and its fixed
instruction example. Raw GUI logs remain Issue-scoped evidence; the small reviewer-facing result
set belongs under `docs/issues/<issue>-<slug>/evidence/`.

## Codex routing and tester ownership

This rule applies to Codex only. For every AppKit E2E request, the Codex main agent must assign
the real GUI execution to one bounded sub-agent before invoking the driver itself. Use the
`elwindui-appkit-e2e-tester` skill so the role is visibly a tester. The standard Codex E2E
sub-agent is `gpt-5.6-luna` with standard reasoning effort (`medium`). Claude Code uses its own
sub-agent mechanism and is not changed by this rule.

The assigned tester owns the complete case and must not delegate again, commit, push, or change
Issue/PR state unless explicitly assigned. The main agent reviews the source diff, evidence, and
PASS/FAIL/NOT RUN/BLOCKED classification before updating GitHub. This routing gate still applies
after context compaction and when a GUI process is already running. If no suitable sub-agent or
GUI-capable execution path is available, report BLOCKED rather than falling back to the main task.

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
- Launch the already-built demo once and reuse one healthy PID for compatible cases.
- Batch deterministic observations such as `list-windows`; do not relaunch or capture redundant
  images.
- Use one tester, one checked-in binary, one doctor, one demo launch, and one PID for a compatible
  batch. Refresh window IDs and geometry after floating create/close, move, resize, and restore.
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

For example, if the current MAIN origin is `<main-x>,<main-y>`, the stable Document A tab offset
`(80,127)` becomes `TAB_A_X=$((MAIN_X+80))` and `TAB_A_Y=$((MAIN_Y+127))`. Do not reuse a stale
origin after moving or resizing a window.

## Immutable evidence and session metadata

Every run uses a new directory and never overwrites earlier evidence:

```zsh
ISSUE=220
HEAD_SHORT="$(git rev-parse --short=12 HEAD)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="$ROOT/.agent-state/issues/$ISSUE/e2e/$HEAD_SHORT/$RUN_ID"
CASE="$RUN/snapshot"
mkdir -p "$CASE"
```

Record `HEAD`, `origin/master`, driver SHA-256, source fingerprint and freshness result, macOS
version, architecture, `doctor` output, and demo SHA-256 in the run directory. Raw logs go under
that run directory. Commit only the small selected result set under
`docs/issues/220-docking-ux-parity/evidence/`; do not commit `.agent-state` or full logs.

Native evidence is invalidated only by effective changes to the AppKit backend, Core layout/input/
host, Custom Controls used by the case, Docking, `docking-demo`, driver source, or checked-in
driver binary. Unrelated WinUI3 and documentation changes do not invalidate it.

## Copy/paste example: PR #221 Snapshot and menu lifetime

This is a complete fixed instruction sheet. Replace only placeholders explicitly marked as values
read from the immediately preceding command, such as `<PID>` and `<window-id>`. Do not redesign
the sequence in the tester.

### Scope and prohibitions

Run exactly these two cases:

1. Native floating bounds A -> B -> restored C using Save/Restore.
2. Main-thread native menu wrapper lifetime after dropping the caller's `Rc`.

Own both cases to completion. Do not delegate again, commit, push, or update Issue/PR state.

### Fixed setup

```zsh
set -e
ROOT="$(git rev-parse --show-toplevel)"
BIN="$ROOT/tools/macos-ui-driver/bin/macos-ui-driver"
APP="$ROOT/target/debug/docking-demo"
ISSUE=220
HEAD_SHORT="$(git rev-parse --short=12 HEAD)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="$ROOT/.agent-state/issues/$ISSUE/e2e/$HEAD_SHORT/$RUN_ID"
CASE="$RUN/snapshot"
mkdir -p "$CASE"

"$ROOT/tools/macos-ui-driver/verify-e2e-binary.sh" \
  >"$CASE/verify-binary.stdout" 2>"$CASE/verify-binary.stderr"
"$BIN" doctor >"$CASE/doctor.stdout" 2>"$CASE/doctor.stderr"
"$BIN" launch --path "$APP" --wait-window-timeout 5 \
  >"$CASE/launch.stdout" 2>"$CASE/launch.stderr"
# Read PID once from launch.stdout and reuse it for every compatible step.
PID=<PID-from-launch.stdout>
"$BIN" list-windows --pid "$PID" >"$CASE/setup-windows.stdout" 2>"$CASE/setup-windows.stderr"
MAIN=<window-id-for-title-ElwindUI-Docking-Demo>
MAIN_X=<x-from-setup-windows.stdout>
MAIN_Y=<y-from-setup-windows.stdout>
TAB_A_X=$((MAIN_X+80))
TAB_A_Y=$((MAIN_Y+127))

run_focused() {
  local target="$1" focus_stdout="$2" focus_stderr="$3" action_stdout="$4" action_stderr="$5"
  shift 5
  "$BIN" focus-window --pid "$PID" --window-id "$target" --timeout 5 \
    >"$focus_stdout" 2>"$focus_stderr" || return
  "$BIN" "$@" >"$action_stdout" 2>"$action_stderr"
}

if ! run_focused "$MAIN" "$CASE/main-focus.stdout" "$CASE/main-focus.stderr" \
    "$CASE/open-menu.stdout" "$CASE/open-menu.stderr" \
    point-click --pid "$PID" --window-id "$MAIN" --x "$TAB_A_X" --y "$TAB_A_Y" --button right; then
  exit 1
fi
```

Required setup result: freshness is `SYNCED` (or the explicitly documented baseline exception),
`doctor` has `success:true`, `accessibility:true`, and `screen_recording:true`, and launch has a
live PID plus a `window` object. If any requirement fails, stop as BLOCKED with both streams.

### Exact actions: Snapshot native bounds

1. After the setup block opens the menu, select Float through Accessibility in the same shell
   invocation as its focus check:

   ```zsh
   run_focused "$MAIN" "$CASE/float-focus.stdout" "$CASE/float-focus.stderr" \
     "$CASE/float.stdout" "$CASE/float.stderr" \
     click --pid "$PID" --window-title 'ElwindUI Docking Demo' --title 'Float' --via ax-press
   ```

2. List windows, set FLOAT to the window titled `Document A`, then focus/capture it atomically:

   ```zsh
   "$BIN" list-windows --pid "$PID" >"$CASE/a-windows.stdout" 2>"$CASE/a-windows.stderr"
   FLOAT=<window-id-titled-Document-A>
   run_focused "$FLOAT" "$CASE/a-focus.stdout" "$CASE/a-focus.stderr" \
     "$CASE/a-capture.stdout" "$CASE/a-capture.stderr" \
     capture-window --window-id "$FLOAT" --out "$CASE/snapshot-bounds-a.png"
   ```

   Record A as `(x,y,width,height)` from `a-windows.stdout`.

3. Focus MAIN and invoke Save through Accessibility in one shell invocation:

   ```zsh
   run_focused "$MAIN" "$CASE/save-focus.stdout" "$CASE/save-focus.stderr" \
     "$CASE/save.stdout" "$CASE/save.stderr" \
     click --pid "$PID" --window-title 'ElwindUI Docking Demo' --title 'Save snapshot' --via ax-press
   ```

4. Move FLOAT using its current bounds plus case-local offsets, then focus/resize and list B:

   ```zsh
   FLOAT_X=<x-from-a-windows.stdout>
   FLOAT_Y=<y-from-a-windows.stdout>
   MOVE_X=$((FLOAT_X+400))
   MOVE_Y=$((FLOAT_Y+240))
   "$BIN" focus-window --pid "$PID" --window-id "$FLOAT" --timeout 5 \
     >"$CASE/move-focus.stdout" 2>"$CASE/move-focus.stderr"
   osascript -e "tell application \"System Events\" to tell process \"docking-demo\" to set position of window \"Document A\" to {$MOVE_X, $MOVE_Y}" \
     >"$CASE/move.stdout" 2>"$CASE/move.stderr"
   run_focused "$FLOAT" "$CASE/resize-focus.stdout" "$CASE/resize-focus.stderr" \
     "$CASE/resize.stdout" "$CASE/resize.stderr" \
     resize --pid "$PID" --window-id "$FLOAT" --delta-width -120 --delta-height -80 \
       --steps 30 --duration 1.0 --timeout 2.0
   "$BIN" list-windows --pid "$PID" >"$CASE/b-windows.stdout" 2>"$CASE/b-windows.stderr"
   run_focused "$FLOAT" "$CASE/b-focus.stdout" "$CASE/b-focus.stderr" \
     "$CASE/b-capture.stdout" "$CASE/b-capture.stderr" \
     capture-window --window-id "$FLOAT" --out "$CASE/snapshot-bounds-b.png"
   ```

   `resize.stdout` must contain `success:true` and `changed:true`; otherwise report NOT RUN with
   both resize streams. Record B from `b-windows.stdout`.

5. Focus MAIN and invoke Restore, list C, and capture C atomically:

   ```zsh
   run_focused "$MAIN" "$CASE/restore-focus.stdout" "$CASE/restore-focus.stderr" \
     "$CASE/restore.stdout" "$CASE/restore.stderr" \
     click --pid "$PID" --window-title 'ElwindUI Docking Demo' --title 'Restore snapshot' --via ax-press
   "$BIN" list-windows --pid "$PID" >"$CASE/c-windows.stdout" 2>"$CASE/c-windows.stderr"
   run_focused "$FLOAT" "$CASE/c-focus.stdout" "$CASE/c-focus.stderr" \
     "$CASE/c-capture.stdout" "$CASE/c-capture.stderr" \
     capture-window --window-id "$FLOAT" --out "$CASE/snapshot-bounds-c.png"
   ```

   Record C from `c-windows.stdout`. PASS requires every C component within 2 points of A and at
   least one C component different from B. Do not report PASS without A/B/C values and all three
   capture paths.

### Exact actions: menu wrapper lifetime

Run the example after compiling only if it is missing or stale:

```zsh
cargo build -q -p elwindui-backend-appkit --example menu_lifetime_runtime \
  >"$CASE/menu-lifetime-build.stdout" 2>"$CASE/menu-lifetime-build.stderr"
"$ROOT/target/debug/examples/menu_lifetime_runtime" \
  >"$CASE/menu-lifetime-runtime.stdout" 2>"$CASE/menu-lifetime-runtime.stderr"
```

PASS requires runtime stdout containing `native_item_retained=true` and `callback_count=1`, with
empty runtime stderr. Build diagnostics belong to the build log and do not replace the runtime
stderr check. Any panic, missing token, or non-empty runtime stderr is FAIL.

### Expected results, report, and cleanup

Use only PASS, FAIL, NOT RUN, or BLOCKED. Report one compact table containing case, status,
PID/window IDs, numeric evidence, and immutable run/log/image paths. Finish with:

```zsh
"$BIN" terminate --pid "$PID" --timeout 5 \
  >"$RUN/terminate.stdout" 2>"$RUN/terminate.stderr"
```

The process must terminate without force. The tester must not update Issue/PR state; the main
agent consumes the report and performs the GitHub workflow.
