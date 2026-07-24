# macos-ui-driver

AI-agent-drivable CLI for launching, inspecting, and screenshotting elwindui (or any) macOS app
windows — see `docs/elwindui_macos_gui_test_driver_status.md` for what's implemented (Phase 1:
launch/terminate/list-windows/capture-window/doctor) versus deferred (Phase 2+: Accessibility-tree
walking and control interaction, elwindui-internal state introspection, image-diff regression
testing).

Every command prints one JSON object to stdout (`{"success": true, ...}` or `{"success": false,
"error": "..."}`) and sets the process exit code accordingly (0/1). No fixed `sleep`-based waiting
anywhere — `launch --wait-window-timeout` and `terminate --timeout` both poll a real condition.

## Build

```bash
cd tools/macos-ui-driver
swift build
# binary at: $(swift build --show-bin-path)/macos-ui-driver
```

## Commands

```bash
macos-ui-driver doctor
# {"accessibility":true,"screen_recording":true,"macos_version":"...","success":true}

macos-ui-driver launch --path <executable> [--arg <a>]* [--cwd <dir>] [--wait-window-timeout <seconds>]
# {"pid":1234,"success":true,"window":{...}}   (window field only present if --wait-window-timeout given)

macos-ui-driver list-windows [--pid <pid>] [--name <substring>]
# {"success":true,"windows":[{"window_id":..., "pid":..., "owner_name":..., "title":..., "layer":..., "x":..., "y":..., "width":..., "height":...}]}

macos-ui-driver capture-window --window-id <id> --out <path.png>
# {"success":true,"window_id":...,"path":"...","width":...,"height":...}

macos-ui-driver terminate --pid <pid> [--timeout <seconds>]
# {"success":true,"pid":...,"terminated":true,"forced":false}

macos-ui-driver focus-window --pid <pid> [--title <substring>] [--timeout <seconds>]
# Two-stage foreground request (NSRunningApplication.activate() then AXRaise on the target
# window) followed by verifying 4 real postconditions (isActive / frontmost app / AXMain /
# AXFocusedWindow) — never trusts activate()/AXRaise return values alone. On success or failure,
# reports rich diagnostics (frontmost app, activation policy, macOS version, etc.). If the
# environment refuses to actually foreground the app (observed in this project's own sandboxed
# agent shell — see docs/elwindui_macos_gui_test_driver_status.md §1.5), this reports
# success:false with full diagnostics rather than retrying or claiming success.
# {"success":true,"pid":...,"is_active":true,"ax_main":true,...}
```

## Example: launch, screenshot, terminate

```bash
BIN=$(swift build --show-bin-path)/macos-ui-driver
cargo build -p controls-demo
"$BIN" launch --path ../../target/debug/controls-demo --wait-window-timeout 5
# -> pull "window_id" out of the JSON
"$BIN" capture-window --window-id <id> --out /tmp/shot.png
"$BIN" terminate --pid <pid>
```

`doctor` requires Screen Recording (for `capture-window`) and Accessibility (for future Phase 2
control interaction) permissions granted to whatever process actually runs this binary — it only
*checks* those permissions, never prompts for them.
