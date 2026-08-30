# Tooling status

Snapshot: 2026-08-30. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

| Tool | State | Current capability / gap |
|---|---|---|
| `elwindui-codegen` | 🚧 | component/ViewModel/enum/ControlTemplate frontend, parser, diagnostics, and one shared semantic planner/emitter for ordinary `view!` plus template construction, property/content lowering, lifecycle, bindings, dynamic regions, ownership, Environment propagation, deferred views, and cleanup; `body: view!` remains ordinary composition, while explicit-target `template_view!(|alias: Target| { ... })` and component `template: template_view!(|alias: Self| { ... })` compile to typed `ControlTemplate<T>` values through the same lowerer. Template-only work is limited to declared-parent acquisition, capability bounds, factory wrapping, and template-root replacement. Targets are not inferred from expected types, `Self` is component-default-only, and reusable templates are ordinary Rust functions; the public `#[control_template]` marker API is absent. Property-free templates accept raw `ControlExt` targets; typed parent property paths are emitted only through `TemplateProperty`/`WritableTemplateProperty` capability bounds, with source-local analysis-only shadows preserving exact associated value types and writable/read-only capability; raw framework/class-managed property bridges are not synthesized. Bare children and dynamic regions lower from effective `#[content(field)]` metadata plus field shape (scalar setter or collection surface); Layout is not a special host category. Control-specific type-name lowering, standalone compiler/type lists, and hidden body-presentation metadata are absent. |
| `elwindui-languageserver` | 🚧 | single-file diagnostics, member completion, and DSL semantic tokens; no cross-file resolution, hover, or generated-code preview |
| Preview | ⬜ | design exists; no workspace preview application |
| `elwindui-hotreload` | 🚧 | tested Patch/Remount decision helper exists; artifact loading and live replacement pipeline are absent |
| `elwindui-test` | 🚧 | render-tree dump exists; canvas/image snapshots absent |
| `macos-ui-driver` | 🚧 | process/window control, focus, Accessibility tree queries/actions, and screenshots are implemented; full keyboard/mouse synthesis and every AX action are not complete |

## macOS UI driver verification

Implemented commands cover launching/locating a process or window, waiting for window state, bringing a window to the front, querying the Accessibility tree, setting supported values, and invoking supported actions. Accessibility permission and foreground restrictions remain environment constraints.

The command catalog and operational precautions belong in [`../agents/appkit.md`](../agents/appkit.md) and [`../../tools/macos-ui-driver/README.md`](../../tools/macos-ui-driver/README.md), not in status.

On 2026-08-30, `cargo run -p control-template-demo` reached executable
startup after the public-path remediation. Computer Use could not inspect the
window because the Mac was locked and automatic unlock failed; no screenshot or
Accessibility-tree result was recorded, so this run is `BLOCKED`, not a runtime
PASS.

## External generated-component DSL (#191/#193/#194)

Merged PR #192 provides qualified external generated-component paths in `view!`, keeps the authored
type path for construction and extension traits, and resolves the `#[macro_export]` props shape at
the defining crate root. Ordinary, template, dynamic, event, two-way, semantic-brush, and resync
lowering share this path-origin decision. The real downstream fixture depends on `elwindui` and
`elwindui-external-component-fixture` independently and covers external properties, collection/scalar
content, property resync, two-way wiring, template dynamic `if`/`for`, nested module paths, and a
Cargo alias.

Issue #193's named construction surface is implemented by PR #195: `elwindui::new!` routes local,
builtin, and qualified external generated components through one construction planner. Required
`#[param]`/`#[bindable]`, defaulted `#[param(default = ...)]`, ordinary mutable Props, full `Option`
storage, pre-mount initial values, external root constructor ABI macros, exact diagnostics, and the
required-before-mount-before-runtime-resync order are covered by focused codegen and downstream tests.
Directly-declared generated `Vec<Rc<T>>` content hosts now separate raw slot mutation from one
post-reconciliation property-change commit, so computed/template dependents observe the final
collection state. A derived component that only inherits the `#[content]` declaration does not
receive that generated host forwarding in #192; this capability boundary is tracked in follow-up
Issue #194 and is not replaced with a fake reactive bridge. Unqualified imported shorthand and a
defining-crate `pub mod ui` facade are not required. The inherited `Vec<Rc<T>>` content forwarding
boundary remains follow-up Issue #194; #194 is intentionally outside PR #195.

The current generated-component shape identity is basename-based within a crate: the same-crate
registry and the hidden #192/#193 shape macros do not yet provide a path-aware identity for distinct
components with the same basename in different modules. This is a current ElwindUI compile-time
ABI/registry limitation, not a fundamental Rust limitation, and is tracked from requirements/design
in [follow-up Issue #196](https://github.com/puchinya/elwindui/issues/196). PR #195 deliberately does
not partially redesign that registry or either hidden shape ABI.
