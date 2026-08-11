# macos-ui-driver

AI-agent-drivable CLI for launching, inspecting, screenshotting, and interacting with elwindui (or
any) macOS app windows — see `docs/status/tooling_status.md` for what's implemented
(Phase 1: launch/terminate/list-windows/capture-window/doctor/focus-window; Phase 2:
dump-tree/find/set-focus/click/type-text/press-key/wait-for — driver-side only, see that doc for the
Rust-side accessibility-identifier wiring left out of scope) versus deferred (Phase 3+:
elwindui-internal state introspection, image-diff regression testing).

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
# agent shell — see docs/status/tooling_status.md), this reports
# success:false with full diagnostics rather than retrying or claiming success.
# {"success":true,"pid":...,"is_active":true,"ax_main":true,...}
```

## Phase 2: Accessibility-tree walking and control interaction

Every Phase 2 command shares two flag groups:

- **Window locator** (`--pid <pid>` required; `--window-id <id>` and/or `--window-title <substring>`
  optional). With neither given: the app's sole AX window is used if there's exactly one, otherwise
  the command fails rather than guessing (stricter than `focus-window`'s `windows[0]` fallback,
  since these commands cause real side effects). `--window-id` is the same `CGWindowID` returned by
  `list-windows`/`capture-window`; resolving it to the matching AX window uses only public API
  (title+geometry matching against `listOnScreenWindows()` when the app has more than one window —
  no private `_AXUIElementGetWindow`).
- **Element selector** (`find`/`set-focus`/`click`/`type-text`/`press-key`): `--role`, `--title`
  (exact), `--title-contains` (substring), `--identifier` (exact — currently inert, since no
  elwindui control sets a custom `accessibilityIdentifier`; standard AppKit `role`/`title`/`value`
  attributes are populated automatically and are enough to select on), `--index <n>` to disambiguate
  multiple matches. `find`/`dump-tree` never fail on 0 or 2+ matches (an empty/ambiguous result is a
  valid answer); `set-focus`/`click`/`type-text`/`press-key` always require exactly one match (or an
  explicit `--index`) since they cause a real side effect.

```bash
macos-ui-driver dump-tree --pid <pid> [--window-id <id>] [--window-title <substring>] [--max-depth 40]
# {"success":true,"pid":...,"node_count":N,"truncated":false,"root":{"role":"AXWindow","title":"...","children":[...]}}

macos-ui-driver find --pid <pid> [--window-id <id>] [--role <r>] [--title <t>] [--title-contains <t>] [--identifier <i>]
# {"success":true,"match_count":N,"matches":[{"role":...,"title":...,"value":...,"position":{...},"size":{...},...}]}
# always success:true, even for 0 matches — this is an existence check, not an action

macos-ui-driver set-focus --pid <pid> [--window-id <id>] <selector> [--timeout 1.0]
# Sets keyboard focus directly via AXUIElementSetAttributeValue(kAXFocusedAttribute), bypassing
# mouse hit-testing entirely. Exists to distinguish "click doesn't focus this control" (a
# mouse/hit-test bug) from "nothing can focus this control at all" (a deeper wiring bug) — try
# both `click` and `set-focus` against the identical selector. Request-then-verify, like
# focus-window: the AX call's return value is recorded but not trusted; only a re-read of
# AXFocused counts.
# {"success":true,"focus_confirmed":true,"set_attribute_status_ok":true,"before":{...},"after":{...}}

macos-ui-driver click --pid <pid> [--window-id <id>] <selector> [--via mouse|ax-press = mouse] [--timeout 1.0]
# --via mouse (default): a real CGEventPost mouse down/up pair at the element's AXPosition/AXSize
#   center — the more faithful "does this behave like a real click" test.
# --via ax-press: AXUIElementPerformAction(kAXPressAction) instead.
# There's no universal "click succeeded" AX signal, so this reports a before/after diff
# (changed.focused / changed.value) as diagnostic data rather than guessing pass/fail.
# {"success":true,"via":"mouse","click_point":{"x":...,"y":...},"before":{...},"after":{...},"changed":{"focused":true,"value":false}}

macos-ui-driver type-text --pid <pid> [--window-id <id>] <selector> --text <string> [--clear]
    [--focus-via ax-attribute|click|none = ax-attribute] [--key-delay 0.02] [--timeout 1.0]
# Synthesizes real keystrokes one character at a time (CGEvent + keyboardSetUnicodeString, not one
# bulk paste-like call) after establishing focus via the chosen --focus-via mechanism (verified,
# not assumed — fails fast if focus can't be confirmed). success is gated on both focus_confirmed
# and the post-typing value matching what was requested — the decisive tool for testing whether a
# text control's focus/input wiring actually works end-to-end.
# {"success":true,"focus_confirmed":true,"before_value":"","after_value":"hello","value_matches_expected":true}

macos-ui-driver press-key --pid <pid> [--window-id <id>] [selector optional]
    --key <enter|tab|escape|backspace|delete|forward-delete|space|left|right|up|down>
    [--modifiers cmd,shift,alt,ctrl] [--focus-via ax-attribute|click|none = none] [--timeout 1.0]
# With no selector, sends the key to whatever currently holds focus. Reports the window's
# kAXFocusedUIElementAttribute before/after (not the originally targeted element — e.g. Tab is
# expected to move focus elsewhere).
# {"success":true,"key":"tab","focused_element_before":{...},"focused_element_after":{...}}

macos-ui-driver wait-for --pid <pid> [--window-id <id>] [selector]
    --condition exists|not-exists|enabled|focused|value-equals [--value <v>] [--timeout 5.0] [--interval 0.1]
# Polls (never a fixed sleep) until the condition holds or --timeout elapses.
# {"success":true,"matched":true,"timed_out":false,"elapsed_seconds":0.11,"match_count":1}
```

## Example: launch, screenshot, click, type, terminate

```bash
BIN=$(swift build --show-bin-path)/macos-ui-driver
cargo build -p controls-demo
"$BIN" launch --path ../../target/debug/controls-demo --wait-window-timeout 5
# -> pull "pid" / "window"."window_id" out of the JSON
"$BIN" capture-window --window-id <id> --out /tmp/shot.png
"$BIN" find --pid <pid> --window-id <id> --title-contains PasswordBox
"$BIN" click --pid <pid> --window-id <id> --title-contains PasswordBox --via ax-press
"$BIN" set-focus --pid <pid> --window-id <id> --role AXTextField
"$BIN" type-text --pid <pid> --window-id <id> --role AXTextField --text "hello" --focus-via none
"$BIN" terminate --pid <pid>
```

`doctor` requires Screen Recording (for `capture-window`) and Accessibility (for `focus-window` and
every Phase 2 command above) permissions granted to whatever process actually runs this binary — it
only *checks* those permissions, never prompts for them.
