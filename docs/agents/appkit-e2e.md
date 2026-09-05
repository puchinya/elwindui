# AppKit Native E2E Tester Guide

This document is the durable procedure for native AppKit GUI acceptance. It is deliberately
separate from [`appkit.md`](appkit.md), so a fresh clone contains both the backend guide and the
tester instruction sheet. Raw GUI logs and screenshots remain Issue-scoped evidence and are not
checked in.

## Codex routing and tester ownership

This rule applies to Codex only. For every AppKit E2E request, the Codex main agent must assign
the real GUI execution to a bounded sub-agent before invoking the driver itself. Use the
`elwindui-appkit-e2e-tester` skill so the role is visibly a tester. The standard Codex E2E
sub-agent is `gpt-5.6-luna` with standard reasoning effort (`medium`). Claude Code uses its own
sub-agent mechanism and is not changed by this rule.

The assigned tester owns the complete case and must not delegate again, commit, push, or change
Issue/PR state unless explicitly assigned. The main agent reviews the source diff, evidence, and
PASS/FAIL/NOT RUN/BLOCKED classification before updating GitHub. This routing gate still applies
after context compaction and when a GUI process is already running. If no suitable sub-agent or
GUI-capable execution path is available, report BLOCKED rather than falling back to the main task.

## Fast execution and safety rules

- Use the checked-out executable at `tools/macos-ui-driver/bin/macos-ui-driver` directly.
- Run `doctor` once and require `success:true`, `accessibility:true`, and
  `screen_recording:true`.
- Launch the already-built demo once and reuse one healthy PID for compatible cases.
- Batch deterministic observations such as `list-windows`; do not rebuild, relaunch, or capture
  redundant images.
- Each driver invocation is a separate process. Run `focus_demo` immediately before every
  GUI-acting invocation (`focus-window`, `point-click`, `click`, `resize`, and `capture-window`).
- Run every driver invocation outside the Codex workspace-write sandbox so macOS TCC grants are
  visible. If `doctor` reports either permission as false, stop as BLOCKED.
- Keep stdout and stderr in separate files. A summary without the required numeric/window or
  image evidence is NOT RUN, never PASS.

For an abnormal result—non-zero driver exit, failed foreground check, missing window,
unexpected process exit, or a release that does not change the layout—retain the exact command,
stdout, and stderr in `.agent-state/issues/<issue>/logs/`, and include those paths in the report.
Do not paste high-volume raw logs into GitHub.

## Fixed tester instruction-sheet format

The main agent must give the tester a concrete, case-scoped instruction sheet rather than asking
it to design a plan. Every sheet has these sections in this order:

1. **Scope and prohibitions** — exact cases, completion ownership, and no re-delegation, commit,
   push, or Issue/PR update.
2. **Fixed setup** — absolute paths, checked-out driver, exact `doctor` and launch commands,
   launch wait option, one-PID reuse rule, and required permissions.
3. **Exact actions** — commands in execution order, fixed coordinates/selectors, explicit
   `focus_demo` before every GUI action, and placeholders only for values read from the immediately
   preceding command.
4. **Expected results and stop rules** — exact JSON fields, tolerances, and conditions for PASS,
   FAIL, NOT RUN, or BLOCKED. Never use vague instructions such as “appropriately” or “try another
   way”.
5. **Evidence and cleanup** — separate stdout/stderr paths, required screenshots and numeric
   values, compact report shape, and exact terminate command.

Keep the sheet copy/pasteable: use one shell block for helper functions and variables, avoid
hidden state, and make every design choice before handing it to the tester.

## Copy/paste example: PR #221 Snapshot and menu lifetime

The following is a complete fixed example. Replace only placeholders explicitly marked as values
read from the immediately preceding command, such as `<PID-from-demo-launch.stdout>` and
`<window_id-for-title-ElwindUI-Docking-Demo>`. Do not redesign the sequence in the sub-agent.

### Scope and prohibitions

Run exactly these two cases for PR #221:

1. Native floating bounds A -> B -> restored C using Save/Restore.
2. Main-thread native menu wrapper lifetime after dropping the caller's `Rc`.

Own both cases to completion. Do not delegate again, commit, push, or update Issue/PR state.

### Fixed setup

```zsh
ROOT=/Users/nabeshimamasataka/RustroverProjects/elwindui
BIN="$ROOT/tools/macos-ui-driver/bin/macos-ui-driver"
APP="$ROOT/target/debug/docking-demo"
LOG="$ROOT/.agent-state/issues/220/logs"
TMP=/private/tmp/pr221-e2e-logs
mkdir -p "$LOG" "$TMP"
focus_demo() {
  osascript -e 'tell application "System Events" to tell process "docking-demo" to set frontmost to true'
}
"$BIN" doctor >"$LOG/driver-doctor.stdout" 2>"$LOG/driver-doctor.stderr"
"$BIN" launch --path "$APP" --wait-window-timeout 5 >"$LOG/demo-launch.stdout" 2>"$LOG/demo-launch.stderr"
# Read PID from demo-launch.stdout once. Reuse this PID for every compatible step below.
PID=<PID-from-demo-launch.stdout>
"$BIN" list-windows --pid "$PID" >"$LOG/common-main.stdout" 2>"$LOG/common-main.stderr"
MAIN=<window_id-for-title-ElwindUI-Docking-Demo>
focus_demo
"$BIN" focus-window --pid "$PID" --window-id "$MAIN" --timeout 5 \
  >"$LOG/common-main-focus.stdout" 2>"$LOG/common-main-focus.stderr"
```

Required setup result: `doctor` JSON has `success:true`, `accessibility:true`, and
`screen_recording:true`; launch has a live PID and a `window` object; MAIN is present in
`common-main.stdout`. If any requirement fails, stop and report BLOCKED with both log paths.

### Exact actions: Snapshot native bounds

1. With MAIN focused, right-click the Document A tab at `(80,332)`:

   ```zsh
   focus_demo
   "$BIN" point-click --pid "$PID" --window-id "$MAIN" --x 80 --y 332 --button right \
     >"$LOG/snapshot-open-menu.stdout" 2>"$LOG/snapshot-open-menu.stderr"
   ```

2. Select the native Float action through Accessibility:

   ```zsh
   focus_demo
   "$BIN" click --pid "$PID" --window-title 'ElwindUI Docking Demo' --title 'Float' --via ax-press \
     >"$LOG/snapshot-float.stdout" 2>"$LOG/snapshot-float.stderr"
   ```

3. List windows, set FLOAT to the window titled `Document A`, capture A, and record
   `(x,y,width,height)` from `snapshot-a.stdout`:

   ```zsh
   "$BIN" list-windows --pid "$PID" >"$LOG/snapshot-a.stdout" 2>"$LOG/snapshot-a.stderr"
   FLOAT=<window_id-titled-Document-A>
   focus_demo
   "$BIN" capture-window --window-id "$FLOAT" --out "$TMP/closure-snapshot-bounds-a.png" \
     >"$LOG/snapshot-a-capture.stdout" 2>"$LOG/snapshot-a-capture.stderr"
   ```

4. Focus MAIN and invoke Save through Accessibility:

   ```zsh
   focus_demo
   "$BIN" focus-window --pid "$PID" --window-id "$MAIN" --timeout 5 \
     >"$LOG/snapshot-save-focus.stdout" 2>"$LOG/snapshot-save-focus.stderr"
   focus_demo
   "$BIN" click --pid "$PID" --window-title 'ElwindUI Docking Demo' --title 'Save snapshot' --via ax-press \
     >"$LOG/snapshot-save.stdout" 2>"$LOG/snapshot-save.stderr"
   ```

5. Move FLOAT to `(520,360)`, resize it by `(-120,-80)` with real native events, list B, and
   capture B:

   ```zsh
   osascript -e 'tell application "System Events" to tell process "docking-demo" to set position of window "Document A" to {520, 360}' \
     >"$LOG/snapshot-move.stdout" 2>"$LOG/snapshot-move.stderr"
   focus_demo
   "$BIN" resize --pid "$PID" --window-id "$FLOAT" --delta-width -120 --delta-height -80 \
     --steps 30 --duration 1.0 --timeout 2.0 \
     >"$LOG/snapshot-resize.stdout" 2>"$LOG/snapshot-resize.stderr"
   "$BIN" list-windows --pid "$PID" >"$LOG/snapshot-b.stdout" 2>"$LOG/snapshot-b.stderr"
   focus_demo
   "$BIN" capture-window --window-id "$FLOAT" --out "$TMP/closure-snapshot-bounds-b.png" \
     >"$LOG/snapshot-b-capture.stdout" 2>"$LOG/snapshot-b-capture.stderr"
   ```

   `snapshot-resize.stdout` must contain `success:true` and `changed:true`; otherwise report
   NOT RUN with both resize log paths. Record B from `snapshot-b.stdout`.

6. Focus MAIN, invoke Restore through Accessibility, list C, and capture C:

   ```zsh
   focus_demo
   "$BIN" focus-window --pid "$PID" --window-id "$MAIN" --timeout 5 \
     >"$LOG/snapshot-restore-focus.stdout" 2>"$LOG/snapshot-restore-focus.stderr"
   focus_demo
   "$BIN" click --pid "$PID" --window-title 'ElwindUI Docking Demo' --title 'Restore snapshot' --via ax-press \
     >"$LOG/snapshot-restore.stdout" 2>"$LOG/snapshot-restore.stderr"
   "$BIN" list-windows --pid "$PID" >"$LOG/snapshot-c.stdout" 2>"$LOG/snapshot-c.stderr"
   focus_demo
   "$BIN" capture-window --window-id "$FLOAT" --out "$TMP/closure-snapshot-bounds-c.png" \
     >"$LOG/snapshot-c-capture.stdout" 2>"$LOG/snapshot-c-capture.stderr"
   ```

   Record C from `snapshot-c.stdout`. PASS requires every C component to be within 2 points of
   A and at least one C component to differ from B. Do not report PASS without A/B/C numbers and
   all three capture paths.

### Exact actions: menu wrapper lifetime

Run the example after compiling only when it is missing or stale:

```zsh
cargo build -q -p elwindui-backend-appkit --example menu_lifetime_runtime \
  >"$LOG/menu-lifetime-build.stdout" 2>"$LOG/menu-lifetime-build.stderr"
"$ROOT/target/debug/examples/menu_lifetime_runtime" \
  >"$LOG/menu-lifetime-runtime.stdout" 2>"$LOG/menu-lifetime-runtime.stderr"
```

PASS requires runtime stdout containing `native_item_retained=true` and `callback_count=1`, with
empty runtime stderr. Build diagnostics belong to the build log and do not replace the runtime
stderr check. Any panic, missing token, or non-empty runtime stderr is FAIL.

### Expected results, report, and cleanup

Use only PASS, FAIL, NOT RUN, or BLOCKED. Report one compact table containing case, status,
PID/window IDs, numeric evidence, and log/image paths. For an abnormal result, include the exact
driver command plus both streams. Finish with:

```zsh
"$BIN" terminate --pid "$PID" --timeout 5 >"$LOG/demo-terminate.stdout" 2>"$LOG/demo-terminate.stderr"
```

The process must terminate without force when the cases are complete. The tester must not update
Issue/PR state; the main agent consumes this report and performs the GitHub workflow.
