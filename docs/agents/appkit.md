# AppKit Backend Agent Guidelines

Guidelines for AI agents modifying `elwindui-backend-appkit` or testing on macOS.

## Related Status Documents

- [`docs/status/macos_ui_driver_status.md`](../status/macos_ui_driver_status.md) — CLI driver commands and status.
- [`docs/status/appkit_memory_baseline.md`](../status/appkit_memory_baseline.md) — AppKit baseline memory measurements.
- [`docs/status/appkit_graphics_demo_rss_breakdown.md`](../status/appkit_graphics_demo_rss_breakdown.md) — AppKit graphics-demo footprint breakdown.

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
