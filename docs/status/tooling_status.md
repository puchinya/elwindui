# Tooling status

Snapshot: 2026-09-10. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

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

The Windows UI driver and deterministic adapter-contract tests are implemented. `tests/e2e/`
now holds its first durable shared scenario, [`self-drawn-pointer-input.md`](../../tests/e2e/self-drawn-pointer-input.md)
(Issue #236) — AppKit and WinUI3 consume the same case definitions.

A real-host run of that scenario's WinUI3 side (five sub-cases, `point-click`/`drag` against
`custom-controls-demo`/`docking-demo`/`controls-demo`) is currently BLOCKED for the self-drawn
sub-cases, not a coverage claim. A genuine (non-zero-distance) real `drag` over verified-blank
self-drawn `Canvas` area in `controls-demo` is proven to deliver a complete, correctly-accepted
pointer sequence, ruling out both an earlier "no WinUI3 window receives real input" claim and a
later "self-drawn controls need an invokable UIA `AutomationPeer`" hypothesis (both retired). The
narrower, still-unresolved finding is that `custom-controls-demo`'s window specifically receives
zero client-area pointer routing under every real-input variant tried, despite confirmed-correct
foreground/hwnd targeting and despite that same window's OS-native title-bar chrome demonstrably
receiving real input. The leading unverified lead is `windows-ui-driver`/`winapp`'s coordinate
handling for this window's size/position in this environment's display geometry, not a
`windows-ui-driver` protocol defect or an Issue #236 product defect; see
`docs/issues/236-treehostpanel-input-surface/evidence/README.md` for the full trail.

A genuine host-level `SetForegroundWindow`/`CreateProcess` handle-inheritance issue was found and
fixed during this driver's own development: a launched long-lived GUI process could keep a caller's
stdout pipe from ever reaching EOF (via inherited-handle propagation through nested process
invocation), and a plain `focus-window` call can legitimately report `BLOCKED` under Windows'
anti-focus-stealing restriction when invoked from a non-interactive process -- real-input driver
commands avoid this by bringing their own target to the foreground as part of delivering input.

## External generated-component DSL

Qualified external generated components and named `elwindui::new!` construction share the local semantic planner and are covered by the downstream fixture. External properties/content, resync, two-way wiring, template dynamic regions, nested module paths, Cargo aliases, required/defaulted constructor inputs, and `Option` Props are supported. Inherited generated `Vec<Rc<T>>` content forwarding remains [#194](https://github.com/puchinya/elwindui/issues/194); same-basename path identity remains [#196](https://github.com/puchinya/elwindui/issues/196).

## Verification state

Codegen, macro, language-server, external-fixture, GUI-driver, and workspace verification follow the commands in [`../agents/testing.md`](../agents/testing.md). Platform-specific GUI results must be recorded as PASS, FAIL, NOT RUN, or BLOCKED according to the host evidence available.
