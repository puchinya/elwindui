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
