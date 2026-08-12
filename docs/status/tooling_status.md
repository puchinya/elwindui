# Tooling status

Snapshot: 2026-08-12. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

| Tool | State | Current capability / gap |
|---|---|---|
| `elwindui-codegen` | ✅ | component/ViewModel/enum frontend, parser, diagnostics, binding and dynamic-region code generation; a consumer component that `inherits` a genuinely external (no local `TypeInfo`) builtin and bare-forwards one of its attribute values (`padding: padding`, dsl_spec.md §3's `ContentControl` pattern) now compiles — previously panicked (Refs #90) |
| `elwindui-languageserver` | 🚧 | single-file diagnostics, member completion, and DSL semantic tokens; no cross-file resolution, hover, or generated-code preview |
| Preview | ⬜ | design exists; no workspace preview application |
| `elwindui-hotreload` | 🚧 | tested Patch/Remount decision helper exists; artifact loading and live replacement pipeline are absent |
| `elwindui-test` | 🚧 | render-tree dump exists; canvas/image snapshots absent |
| `macos-ui-driver` | 🚧 | process/window control, focus, Accessibility tree queries/actions, and screenshots are implemented; full keyboard/mouse synthesis and every AX action are not complete |

## macOS UI driver verification

Implemented commands cover launching/locating a process or window, waiting for window state, bringing a window to the front, querying the Accessibility tree, setting supported values, and invoking supported actions. Accessibility permission and foreground restrictions remain environment constraints.

The command catalog and operational precautions belong in [`../agents/appkit.md`](../agents/appkit.md) and [`../../tools/macos-ui-driver/README.md`](../../tools/macos-ui-driver/README.md), not in status.
