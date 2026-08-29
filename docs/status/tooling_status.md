# Tooling status

Snapshot: 2026-08-29. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

| Tool | State | Current capability / gap |
|---|---|---|
| `elwindui-codegen` | 🚧 | component/ViewModel/enum/ControlTemplate frontend, parser, diagnostics, and one shared semantic planner/emitter for ordinary `view!` plus template construction, property/content lowering, lifecycle, bindings, dynamic regions, ownership, Environment propagation, deferred views, and cleanup; `body: view!` remains ordinary composition, while `template: template_view!` and standalone `template_view!` expressions compile to typed `ControlTemplate<C>` values through the same lowerer. Template-only work is limited to typed-parent acquisition, capability bounds, factory wrapping, and template-root replacement. Property-free templates accept raw `ControlExt` targets; typed parent property paths are emitted only through `TemplateProperty`/`WritableTemplateProperty` capability bounds, and raw framework/class-managed property bridges are not synthesized. Bare children and dynamic regions lower from effective `#[content(field)]` metadata plus field shape (scalar setter or collection surface); Layout is not a special host category. Control-specific type-name lowering, standalone compiler/type lists, and hidden body-presentation metadata are absent. |
| `elwindui-languageserver` | 🚧 | single-file diagnostics, member completion, and DSL semantic tokens; no cross-file resolution, hover, or generated-code preview |
| Preview | ⬜ | design exists; no workspace preview application |
| `elwindui-hotreload` | 🚧 | tested Patch/Remount decision helper exists; artifact loading and live replacement pipeline are absent |
| `elwindui-test` | 🚧 | render-tree dump exists; canvas/image snapshots absent |
| `macos-ui-driver` | 🚧 | process/window control, focus, Accessibility tree queries/actions, and screenshots are implemented; full keyboard/mouse synthesis and every AX action are not complete |

## macOS UI driver verification

Implemented commands cover launching/locating a process or window, waiting for window state, bringing a window to the front, querying the Accessibility tree, setting supported values, and invoking supported actions. Accessibility permission and foreground restrictions remain environment constraints.

The command catalog and operational precautions belong in [`../agents/appkit.md`](../agents/appkit.md) and [`../../tools/macos-ui-driver/README.md`](../../tools/macos-ui-driver/README.md), not in status.

## External generated-component DSL (#191)

The open PR #192 branch accepts qualified external generated-component paths in `view!`, keeps the
authored type path for construction and extension traits, and resolves the `#[macro_export]` props
shape at the defining crate root. Ordinary, template, dynamic, event, two-way, semantic-brush,
and resync lowering share this path-origin decision. The real downstream fixture depends on
`elwindui` and `elwindui-external-component-fixture` independently and covers external properties,
collection/scalar content, property resync, two-way wiring, template dynamic `if`/`for`, nested
module paths, and a Cargo alias.

Issue #193's named construction surface is implemented in the working tree: `elwindui::new!` routes
local, builtin, and qualified external generated components through one construction planner. Required
`#[param]`/`#[bindable]`, defaulted `#[param(default = ...)]`, ordinary mutable Props, full `Option`
storage, pre-mount initial values, external root constructor ABI macros, exact diagnostics, and the
required-before-mount-before-runtime-resync order are covered by focused codegen and downstream tests.
Directly-declared generated `Vec<Rc<T>>` content hosts now separate raw slot mutation from one
post-reconciliation property-change commit, so computed/template dependents observe the final
collection state. A derived component that only inherits the `#[content]` declaration does not
receive that generated host forwarding in #192; this capability boundary is tracked in follow-up
Issue #194 and is not replaced with a fake reactive bridge.
Unqualified imported shorthand and a defining-crate `pub mod ui` facade are not required. PR #192
is not merged yet; PR #184 remains dependent on it. The inherited `Vec<Rc<T>>` content forwarding
boundary remains follow-up Issue #194.
