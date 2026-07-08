# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Communication

When asking the user a question (clarifying questions, `AskUserQuestion`, plan checkpoints, etc.), always ask in Japanese.

## Project state

This repo is **elwindui**, the implementation project for **ElwindUIL**: a declarative, Rust-flavored layout DSL for building GUIs that compile to multiple backends (egui/iced, or native OS toolkits WinUI 3 / AppKit / GTK4). The project is currently at the **specification stage only** — `src/main.rs` is still the default `cargo new` stub and `Cargo.toml` has no dependencies. There is no parser, codegen, or runtime implemented yet.

The authoritative source of truth is `docs/elwindui_spec.md` (written in Japanese, core language:
`component`/`view`, `param`/`prop`, control flow, static verification rules, etc.), plus
`docs/elwindui_builtins_spec.md` — split out from the same doc because it had grown too large —
which covers every `builtin::`-namespace UI element and `platform::`-namespace OS API. Both are
long — read the relevant section rather than the whole file. Section map (grep each file for these
headers):

`docs/elwindui_spec.md` (core language/runtime, no builtin-widget catalog):
- §1–§15 — core language: `component`/`view` split, `param`/`prop`, control flow, `style`, constraints, `enum`, `env::*`/`once`, `bind!`, i18n (Fluent), imports, the `Element` trait, and the full list of ~24 static verification rules (§14) a future compiler/linter must implement.
- 付録A/C/D — backend abstraction: common AST → per-backend codegen, `target::backend()` compile-time constant.
- 付録E — the `builtin::` namespace and `#[overrides(builtin::X)]` override rule (static verification only; the builtins themselves are in `elwindui_builtins_spec.md` 付録F).
- 付録B — toolchain: `.elwind` → Rust via `build.rs` codegen (or proc-macro), `elwindui-languageserver` LSP, 3-tier live preview, hot-reload semantics.
- 付録H — core runtime (layout/focus/accessibility), consumed by builtins but not itself a widget.
- 付録I/J/K/O/P/R/S/U/V/W — lifecycle hooks, `store` (global/scoped shared state), keyboard shortcut *attribute*, `viewmodel`/`Command` (MVVM), async, theme/design tokens, error boundaries, undo/redo, snapshot testing, mobile lifecycle.

`docs/elwindui_builtins_spec.md` (every concrete `builtin::`/`platform::` element):
- 付録F — reference implementations of `Window`/`Row`/`Column`/`Text`/`TextArea`/`Dropdown`.
- 付録G/N — custom drawing (`Canvas`/`Painter`) and its Composition-style extensions (gradients/shadows/transforms/animation).
- 付録L — `NavigationHost`/`Route` screen navigation.
- 付録M — `Dialog`/`Menu`/`MenuItem`/`Tooltip` (dialogs, context menus, tooltips).
- 付録Q — `VirtualList` (large-list virtualization).
- 付録T — `platform::clipboard`/`platform::file_dialog`, drag & drop.
- 付録X/Y — `MenuBar`/`MenuBarItem` (native app menu bar) and `TabView`/`TabItem` (multi-document tabs), added for the notepad example.

## Core architectural rules to preserve when implementing

- **`param` vs `prop`**: `#[param]` fields are fixed at instantiation and may only use static-evaluable expressions (literals, other params, pure builtins, `env::*`, `once` values) — never `bind!`, prop references, or impure calls. Default (`prop`) fields are runtime-mutable and support `bind!`/`#[computed]`. This split is what the §14 rules exist to enforce; don't weaken it for convenience.
- **Enums are the only value-set mechanism** — no anonymous unions. `match` over an enum (including the built-in `Backend` and `Route` enums) must be exhaustive; missing arms are a compile error by design (e.g. adding a `Backend` variant should break every non-exhaustive builtin `match target::backend()`).
- **`native!` and `target::backend()` are restricted**: only reachable from `#[overrides(builtin::X)]` components or other builtins — arbitrary user components must not call into backend-specific code directly (rules 9/15).
- **`store`/`viewmodel` are never read directly from `#[param]`** — access always goes through `prop` + `bind!` (rule 12/13), and `viewmodel` internals aren't reachable from builtin view elements (rule 19), keeping MVVM's V/VM separation statically enforced.
- **Builtin shadowing must be explicit** — a user `component` sharing a name with a `builtin::` element is a static ambiguity error unless annotated `#[overrides(builtin::X)]`; there is no implicit shadowing.
- **Rust class-hierarchy convention (both codegen output and hand-written runtime code)**: for a class `Class` (with parent `SuperClass`), define `trait Class: SuperClass` + `struct ClassImpl { base: SuperClassImpl, /* own fields */ }`, with `ClassImpl` implementing `Class` and every ancestor trait (each ancestor method delegating to `self.base.method(...)`). The root class (no parent) has no `base` field. Construct via a `create_class(...)` factory function, never a bare struct literal. See docs/elwindui_spec.md 付録H.2.1a for the full rule and `elwindui-core::tree`'s `UIElement`/`Control`/etc. hierarchy for the reference implementation.

## Commands

- `cargo build` / `cargo run` — build/run the current stub binary.
- `cargo test` — no tests exist yet.
- Edition 2024; no dependencies declared yet in `Cargo.toml`.

## Taking screenshots of a running example app (AppKit backend etc.)

Always capture the specific window, not the full screen — a full-screen `screencapture` pulls in the menu bar,
desktop, and unrelated windows and wastes context. Get the target window's `CGWindowID` via a tiny Swift snippet
(no Accessibility permission needed, only Screen Recording), then pass it to `screencapture -l<id>`:

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
id=$(swift /tmp/winid.swift notepad)   # match on the app/process name
screencapture -x -l"$id" /tmp/window.png
```

Note: simulating clicks via `osascript`/System Events requires Accessibility permission, which is a separate grant
from Screen Recording and may not be available — if clicking programmatically fails with error -25211, ask the user
to perform the click manually and then capture the window screenshot afterward.