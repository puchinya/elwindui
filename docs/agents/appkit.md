# AppKit Backend Agent Guidelines

Guidelines for AI agents modifying `elwindui-backend-appkit` or testing on macOS.

## Related documents

- Architecture: [`docs/design/backends/appkit_backend_design.md`](../design/backends/appkit_backend_design.md)
- Backend state: [`docs/status/backend_status.md`](../status/backend_status.md)
- Control state: [`docs/status/control_status.md`](../status/control_status.md)
- Tool state: [`docs/status/tooling_status.md`](../status/tooling_status.md)

Raw memory reports and GUI logs belong under `.agent-state/issues/<issue>/` according to [`docs/agent-workflow/evidence.md`](../agent-workflow/evidence.md), not in durable status documents.

## Visual & Screenshot Verification

Always capture the target window directly (never the full screen).

### Preferred Method: `tools/macos-ui-driver`

Build once with `swift build` in `tools/macos-ui-driver`:

```bash
BIN=$(cd tools/macos-ui-driver && swift build --show-bin-path)/macos-ui-driver
"$BIN" doctor                                                       # Verify permissions
"$BIN" launch --path target/debug/notepad --wait-window-timeout 5   # Launch app
"$BIN" capture-window --window-id <id> --out /tmp/window.png        # Window-cropped capture
"$BIN" terminate --pid <pid>                                        # Clean shutdown
```

When these commands are run through Codex, request Sandbox-outside execution on every command
that invokes `macos-ui-driver`. The workspace-write sandbox does not expose the macOS TCC grants
needed by Accessibility and Screen Recording, even when the same binary succeeds from Terminal.
Run `"$BIN" doctor` first and require both checks to be `true`; otherwise report native GUI
acceptance as BLOCKED. Keep the normal Codex sandbox default unchanged and use per-command
elevation rather than switching the global profile to full access.

For Docking headers, tab strips, compass targets, and other custom surfaces that are absent from
the Accessibility tree, use the driver's coordinate commands after focus-window has succeeded:

~~~bash
"$BIN" point-click --pid <pid> --window-id <id> --x <screen-x> --y <screen-y> [--button left|right]
"$BIN" drag --pid <pid> --window-id <id> \
  --start-x <screen-x> --start-y <screen-y> --end-x <screen-x> --end-y <screen-y> \
  [--button left|right] [--steps <n>] [--duration <seconds>]
~~~

Both commands verify that the target window is still frontmost/main/focused and reject points
outside its AX bounds. A long drag can remain running while a separate elevated invocation uses
capture-window to record the real mid-drag preview.

### Codex delegation for AppKit E2E

This delegation rule applies to Codex only. The Codex main agent may assign a bounded
sub-agent the preparation and registration of test artifacts (for example, rebuilding the
driver and placing the executable at `tools/macos-ui-driver/bin/macos-ui-driver`) and may
assign a separate bounded sub-agent the real AppKit E2E execution. The main agent remains
responsible for reviewing the resulting file diff, evidence, and PASS/FAIL/NOT RUN/BLOCKED
classification before updating the Issue or Pull Request. Delegated agents must not commit,
push, or change Issue/PR state unless that scope is explicitly assigned.

The E2E sub-agent must follow the same per-command Sandbox-outside requirement and `doctor`
preflight above, use the checked-out driver directly, record concrete PID/window/capture
evidence, and report unsupported or unobservable cases as NOT RUN rather than inferring a
PASS. This does not alter the Claude Code workflow; Claude Code already uses its own
sub-agent mechanism.

For an abnormal result—non-zero driver exit, a failed foreground check, a missing window,
an unexpected process exit, or a release that does not change the layout—retain the exact
command, stdout, and stderr in the Issue-scoped `.agent-state/issues/<issue>/logs/` directory
(or a temporary path when the log is only needed during the live session). Include the log
path and a short diagnostic excerpt in the evidence report; do not paste high-volume raw logs
into the Issue or Pull Request.

### Fallback Method

Get target window `CGWindowID` via Swift, then use `screencapture`:

```bash
cat > /tmp/winid.swift << 'EOF'
import CoreGraphics
import Foundation
let target = CommandLine.arguments[1]
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
for w in list {
    let owner = w[kCGWindowOwnerName as String] as? String ?? ""
    let layer = w[kCGWindowLayer as String] as? Int ?? -1
    if layer == 0, owner.localizedCaseInsensitiveContains(target), let num = w[kCGWindowNumber as String] as? Int {
        print(num)
    }
}
EOF
id=$(swift /tmp/winid.swift notepad)
screencapture -x -l"$id" /tmp/window.png
```
