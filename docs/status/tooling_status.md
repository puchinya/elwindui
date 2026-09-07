# Tooling status

Snapshot: 2026-09-07. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

## Current capability matrix

| Tool | State | Current capability / gap |
|---|---|---|
| `elwindui-codegen` | 🚧 | Component/ViewModel/enum/ControlTemplate parsing, diagnostics, shared semantic planning/emission, bindings, dynamic regions, ownership, environment propagation, deferred views, cleanup, explicit-target templates, and component-default templates are implemented. Targets are not inferred, the public `#[control_template]` marker is absent, and raw framework/class-managed property bridges are not synthesized. |
| `elwindui-languageserver` | 🚧 | Single-file diagnostics, member completion, and DSL semantic tokens; cross-file resolution, hover, and generated-code preview are incomplete. |
| Preview | ⬜ | Design exists; no workspace preview application. |
| `elwindui-hotreload` | 🚧 | Patch/Remount decision helper exists; artifact loading and live replacement are absent. |
| `elwindui-test` | 🚧 | Render-tree dump exists; canvas/image snapshots are absent. |
| `macos-ui-driver` | 🚧 | Process/window control, focus, Accessibility queries/actions, screenshots, coordinate clicks, real press/drag/release, and native resize gestures are implemented; full keyboard synthesis and every AX action are incomplete. |
| `windows-ui-driver` | 🚧 | Process/window control, UIA inspect/search/invoke/get-value/get-property/set-focus/wait-for, real mouse click/drag, screenshot (window and screen-capture modes), and move/resize are implemented over the external `winapp` CLI; `send-keys` is implemented but not yet exercised end to end by a live case. |

## macOS UI driver verification

The driver must run outside the Codex workspace-write sandbox for native GUI evidence. Accessibility and Screen Recording permission checks are host properties: a false check blocks native acceptance rather than providing a partial GUI PASS. The command catalog is [`../../tools/macos-ui-driver/README.md`](../../tools/macos-ui-driver/README.md), and its operational procedure belongs in [`../agents/appkit-e2e.md`](../agents/appkit-e2e.md).

Current AppKit visual evidence exists for the control-template demo. No Accessibility-tree interaction result is claimed when the verification environment lacks the required permissions.

## Windows UI driver verification

The driver must run outside any agent sandbox, as a normal non-elevated user, on an unlocked
interactive desktop for native GUI evidence. It requires the external `winapp` CLI
(`winget install Microsoft.winappcli --source winget`), never vendored or auto-installed. The
command catalog is [`../../tools/windows-ui-driver/README.md`](../../tools/windows-ui-driver/README.md),
the architecture is [`../design/tools/windows_ui_driver_design.md`](../design/tools/windows_ui_driver_design.md),
and the operational procedure is [`../agents/winui3-e2e.md`](../agents/winui3-e2e.md).

Live evidence exists for `theme-demo`: a UIA theme-button action (Ocean) and a real-mouse
theme-button action (Solarized) both verified by resulting label-text postcondition (not
injection-return-code alone), plus a `--capture-screen` screenshot and clean process termination.
Two structural gaps found relative to the removed `tools/test-theme-demo-uia.py` migration
source: current `theme-demo` no longer has a disabled-state native control or a nested `TabView`
(that content predates a since-simplified `theme-demo`) -- those cases are NOT AVAILABLE against
the current demo, not FAIL.

A genuine host-level `SetForegroundWindow`/`CreateProcess` handle-inheritance issue was found and
fixed during this driver's own development: a launched long-lived GUI process could keep a caller's
stdout pipe from ever reaching EOF (via inherited-handle propagation through nested process
invocation), and a plain `focus-window` call can legitimately report `BLOCKED` under Windows'
anti-focus-stealing restriction when invoked from a non-interactive process -- real-input driver
commands avoid this by bringing their own target to the foreground as part of delivering input.

## External generated-component DSL

Qualified external generated components and named `elwindui::new!` construction share the local semantic planner and are covered by the downstream fixture. External properties/content, resync, two-way wiring, template dynamic regions, nested module paths, Cargo aliases, required/defaulted constructor inputs, and `Option` Props are supported. Inherited generated `Vec<Rc<T>>` content forwarding remains [#194](https://github.com/puchinya/elwindui/issues/194); same-basename path identity remains [#196](https://github.com/puchinya/elwindui/issues/196).

## Verification state

Codegen, macro, language-server, external-fixture, GUI-driver, and workspace verification follow the commands in [`../agents/testing.md`](../agents/testing.md). Platform-specific GUI results must be recorded as PASS, FAIL, or NOT RUN according to the host evidence available.
