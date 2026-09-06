# Tooling status

Snapshot: 2026-09-06. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

## Current capability matrix

| Tool | State | Current capability / gap |
|---|---|---|
| `elwindui-codegen` | 🚧 | Component/ViewModel/enum/ControlTemplate parsing, diagnostics, shared semantic planning/emission, bindings, dynamic regions, ownership, environment propagation, deferred views, cleanup, explicit-target templates, and component-default templates are implemented. Targets are not inferred, the public `#[control_template]` marker is absent, and raw framework/class-managed property bridges are not synthesized. |
| `elwindui-languageserver` | 🚧 | Single-file diagnostics, member completion, and DSL semantic tokens; cross-file resolution, hover, and generated-code preview are incomplete. |
| Preview | ⬜ | Design exists; no workspace preview application. |
| `elwindui-hotreload` | 🚧 | Patch/Remount decision helper exists; artifact loading and live replacement are absent. |
| `elwindui-test` | 🚧 | Render-tree dump exists; canvas/image snapshots are absent. |
| `macos-ui-driver` | 🚧 | Process/window control, focus, Accessibility queries/actions, screenshots, coordinate clicks, real press/drag/release, and native resize gestures are implemented; full keyboard synthesis and every AX action are incomplete. |

## macOS UI driver verification

The driver must run outside the Codex workspace-write sandbox for native GUI evidence. Accessibility and Screen Recording permission checks are host properties: a false check blocks native acceptance rather than providing a partial GUI PASS. The command catalog is [`../../tools/macos-ui-driver/README.md`](../../tools/macos-ui-driver/README.md), and its operational procedure belongs in [`../agents/appkit-e2e.md`](../agents/appkit-e2e.md).

Current AppKit visual evidence exists for the control-template demo. No Accessibility-tree interaction result is claimed when the verification environment lacks the required permissions.

## External generated-component DSL

Qualified external generated components and named `elwindui::new!` construction share the local semantic planner and are covered by the downstream fixture. External properties/content, resync, two-way wiring, template dynamic regions, nested module paths, Cargo aliases, required/defaulted constructor inputs, and `Option` Props are supported. Inherited generated `Vec<Rc<T>>` content forwarding remains [#194](https://github.com/puchinya/elwindui/issues/194); same-basename path identity remains [#196](https://github.com/puchinya/elwindui/issues/196).

## Verification state

Codegen, macro, language-server, external-fixture, GUI-driver, and workspace verification follow the commands in [`../agents/testing.md`](../agents/testing.md). Platform-specific GUI results must be recorded as PASS, FAIL, or NOT RUN according to the host evidence available.
