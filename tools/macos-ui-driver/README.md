# macos-ui-driver

AI-agent-drivable CLI for launching, inspecting, screenshotting, and interacting with elwindui (or
any) macOS app windows — see `docs/status/tooling_status.md` for what's implemented
(Phase 1: launch/terminate/list-windows/capture-window/doctor/focus-window; Phase 2:
dump-tree/find/set-focus/click/point-click/drag/resize/type-text/set-value/press-key/wait-for —
including Core-backed accessibility identifiers and direct AX value setting) versus deferred (Phase 3+:
elwindui-internal state introspection, image-diff regression testing).

Every command prints one JSON object to stdout (`{"success": true, ...}` or `{"success": false,
"error": "..."}`) and sets the process exit code accordingly (0/1). No fixed `sleep`-based waiting
anywhere — `launch --wait-window-timeout` and `terminate --timeout` both poll a real condition.

The shared native E2E orchestration target is defined in
[`docs/design/tools/native_e2e_orchestration_design.md`](../../docs/design/tools/native_e2e_orchestration_design.md).
This README remains the driver command and platform-mechanics authority; it does not implement the
shared runner or its future plan/cache protocol.

## Build

```bash
cd tools/macos-ui-driver
swift build
# binary at: $(swift build --show-bin-path)/macos-ui-driver
```

## Codex execution

When this driver is invoked by Codex for real AppKit GUI verification, request Sandbox-outside
execution for every invocation. The workspace-write sandbox can start the binary but macOS TCC
reports Accessibility and Screen Recording as unavailable, which makes foreground, Accessibility,
and screenshot evidence invalid. Do not change Codex's global default to full access just for this
driver; request the elevated execution per command and run `doctor` first.

Use the checked-out binary when available:

```bash
BIN=/absolute/path/to/tools/macos-ui-driver/bin/macos-ui-driver
"$BIN" doctor
```

The command must report `"accessibility":true` and `"screen_recording":true` before native GUI
evidence is collected. Terminal-launched execution is a valid fallback when Codex cannot provide
the elevated execution path. A `doctor` result with either value false is a blocked GUI session,
not a native PASS.

Codex must delegate the real AppKit E2E matrix to a bounded sub-agent before any driver action;
the standard sub-agent is `gpt-5.6-luna` with standard reasoning effort (`medium`). The checked-in
binary is the permission-stable E2E artifact; do not delegate a refresh or replacement sidecar for
ordinary E2E. The main agent must review the file diff and evidence before
recording results. This Codex-only routing gate and the fixed tester instruction example are
documented in [`docs/agents/appkit-e2e.md`](../../docs/agents/appkit-e2e.md) and do not change the Claude Code
workflow. If delegation is unavailable, report BLOCKED instead of performing the E2E in the main
agent.

For a fast run, perform `doctor` once, launch the prebuilt demo once, reuse its PID across the
compatible scenarios, batch window-state observations, and capture only the evidence required by
the acceptance case. Rebuild or relaunch only when the binary changed or the process became
unusable; save extra screenshots and verbose logs only for abnormal results.

Swift source is the implementation authority. The checked-in `bin/macos-ui-driver` is the normal,
permission-stable E2E executable. Ordinary E2E must not rebuild or replace it. A change under
`Package.swift` or `Sources/**/*.swift`, a missing/corrupt binary, or an explicit driver-remediation
request requires rebuilding, preserving mode `100755`, updating
[`bin/PROVENANCE.md`](bin/PROVENANCE.md), running `verify-e2e-binary.sh`, and rechecking `doctor`
in host context. Unrelated code or documentation changes must not refresh the binary.

The assigned E2E sub-agent must complete its assigned scenarios itself and must not re-delegate
them. It must return the required window values and separate stdout/stderr logs; an incomplete
summary is NOT RUN and cannot be recorded as PASS.

Each Driver CLI invocation is a separate process. Foreground is required only for operations whose
delivery depends on real frontmost input or when foreground behavior itself is under test. The
semantic operations `find`, `dump-tree`, `set-focus`, `wait-for`, and `click --via ax-press` do not
need a universal `focus-window` step. `capture-window` is screenshot capture, not user input, and
does not require `focus-window` by policy.

`click` without `--via ax-press` is real mouse delivery (`--via mouse`, the default). Use a
foreground/input-routing prerequisite immediately before `click --via mouse`, `point-click`,
`drag`, `resize`, and synthesized keyboard input when frontmost delivery matters. If focus is
needed, verify it before the real-input action; do not treat a failed focus request as permission
to continue. Do not apply a universal focus rule to captures or semantic Accessibility operations.

When a driver command fails or a GUI result is abnormal, save its exact stdout and stderr with
the command and case name. Keep high-volume logs under the Issue-scoped
`.agent-state/issues/<issue>/logs/` directory and report the path plus a short excerpt; do not
replace an error with a screenshot-only result.

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
# Screenshot capture is not user input and does not require focus-window by policy.

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
- **Element selector** (`find`/`set-focus`/`click`/`type-text`/`set-value`/`press-key`): `--role`, `--title`
  (exact), `--title-contains` (substring), `--identifier` (exact Core-backed accessibility
  identifier), `--index <n>` to disambiguate
  multiple matches. `find`/`dump-tree` never fail on 0 or 2+ matches (an empty/ambiguous result is a
  valid answer); `set-focus`/`click`/`type-text`/`set-value`/`press-key` always require exactly one match (or an
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
# Without --via ax-press, click is real mouse delivery (--via mouse, the default).
# --via mouse (default): a real CGEventPost mouse down/up pair at the element's AXPosition/AXSize
#   center — the more faithful "does this behave like a real click" test.
# --via ax-press: AXUIElementPerformAction(kAXPressAction) instead.
# There's no universal "click succeeded" AX signal, so this reports a before/after diff
# (changed.focused / changed.value) as diagnostic data rather than guessing pass/fail.
# {"success":true,"via":"mouse","click_point":{"x":...,"y":...},"before":{...},"after":{...},"changed":{"focused":true,"value":false}}

macos-ui-driver point-click --pid <pid> --window-id <id> --x <screen-x> --y <screen-y>
    [--button left|right] [--pause <seconds>]
# Sends a real click at an explicit screen coordinate for custom controls that are absent from the
# Accessibility tree. The target window must already be confirmed foreground by focus-window.
# {"success":true,"point":{"x":...,"y":...},"button":"left",...}

macos-ui-driver drag --pid <pid> --window-id <id>
    --start-x <screen-x> --start-y <screen-y> --end-x <screen-x> --end-y <screen-y>
    [--button left|right] [--steps <n>] [--duration <seconds>] [--allow-end-outside-window]
# Sends real press/drag/release events with intermediate positions. Run a several-second drag in
# one shell session and capture-window from another to inspect a live Docking preview or splitter.
# By default both endpoints must be inside --window-id. The explicit --allow-end-outside-window
# option keeps the start-point check but permits a cross-window release, such as floating -> main.
# {"success":true,"start":{"x":...,"y":...},"end":{"x":...,"y":...},"steps":N,...}

# Cross-window release (the press point is still required to be in --window-id)
macos-ui-driver drag --pid <pid> --window-id <floating-id> \
    --start-x <x> --start-y <y> --end-x <main-x> --end-y <main-y> \
    --allow-end-outside-window

macos-ui-driver resize --pid <pid> --window-id <id> \
    --delta-width <points> --delta-height <points> [--steps <n>] [--duration <seconds>] [--timeout <seconds>]
# Grabs the target window's lower-right resize handle through real mouse events, applies the
# requested deltas, and verifies the post-gesture AX size. Either delta may be zero, but not both.
# The result includes before/after width and height plus changed=true only when AppKit reported a
# different size.
# {"success":true,"before":{"width":...,"height":...},"after":{"width":...,"height":...},"changed":true,...}

macos-ui-driver type-text --pid <pid> [--window-id <id>] <selector> --text <string> [--clear]
    [--focus-via ax-attribute|click|none = ax-attribute] [--key-delay 0.02] [--timeout 1.0]
# Synthesizes real keystrokes one character at a time (CGEvent + keyboardSetUnicodeString, not one
# bulk paste-like call) after establishing focus via the chosen --focus-via mechanism (verified,
# not assumed — fails fast if focus can't be confirmed). success is gated on both focus_confirmed
# and the post-typing value matching what was requested — the decisive tool for testing whether a
# text control's focus/input wiring actually works end-to-end.
# {"success":true,"focus_confirmed":true,"before_value":"","after_value":"hello","value_matches_expected":true}

macos-ui-driver set-value --pid <pid> [--window-id <id>] <selector> (--text <string> | --value <number>)
    [--timeout 1.0]
# Sets the target's public kAXValueAttribute directly and verifies the observed value. Use this
# for semantic Value/SetText coverage; use type-text when the test specifically requires real
# keyboard delivery and native editing/focus behavior. Exactly one of --text or --value is required.
# {"success":true,"before_value":"hello","requested_text":"semantic-text","after_value":"semantic-text","value_matches_expected":true}

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
