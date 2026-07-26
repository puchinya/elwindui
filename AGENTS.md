# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Communication

When asking the user a question (clarifying questions, `AskUserQuestion`, plan checkpoints, etc.), always ask in Japanese.

<!-- BEGIN ISSUE-DRIVEN AGENT WORKFLOW -->
## Issue-driven development workflow

Use this workflow only for requests expected to modify this repository. Do not create an Issue for explanation, research, exploratory design discussion, or code-reading tasks unless the user explicitly asks to track the work.

### Common rules

- Search for an existing relevant Issue before creating a new one.
- Every repository-changing task must be associated with one GitHub Issue.
- Create or locate the Issue before modifying source code or documentation.
- For Rust work, assign the Issue to the GitHub Milestone whose title exactly matches the root `Cargo.toml` version. Prefer `[workspace.package].version`, otherwise `[package].version`. Do not add a `v` prefix. Create the Milestone if it does not exist. Use `scripts/agent/ensure-version-milestone.sh <issue-number>` on macOS/Linux or `scripts/agent/ensure-version-milestone.ps1 <issue-number>` in PowerShell.
- If an exact-title Milestone exists but is closed, do not create a duplicate or reopen it silently; report the inconsistency for a release-version decision.
- For source-code changes, create a dedicated branch named `feature/<issue-number>-<short-slug>` from the current remote default branch before editing. Never edit source code directly on the default branch. Use `scripts/agent/start-feature-branch.sh <issue-number> <short-description>` on macOS/Linux or `scripts/agent/start-feature-branch.ps1 <issue-number> <short-description>` in PowerShell.
- Documentation-only or workflow-only changes may use a `docs/` or `agent/` branch instead.
- Use at most one `phase:*` label at a time:
  - `phase:requirements`
  - `phase:design`
  - `phase:ready`
  - `phase:implementation`
  - `phase:review`
- `blocked` and `needs-user-decision` are orthogonal labels; they do not replace the current phase.
- Do not update the Issue body after every conversation turn.
- Update the Issue body when requirements and design are approved, or when an approved specification materially changes.
- Do not report a GitHub write as successful unless the operation actually succeeded.
- If GitHub write access is unavailable, report the limitation instead of silently substituting a local tracking file.
- Close the Issue only after the associated Pull Request is merged into the default branch.
- Prefer `Closes #<issue-number>` in the Pull Request body.

### Phase routing

Determine the effective phase from the Issue labels, linked Pull Request state, and explicit user instructions. Repository state takes precedence over stale labels: an open associated Pull Request normally means review, and a merged Pull Request means the implementation lifecycle has finished.

Read only the workflow document needed for the effective phase. Do not load every workflow document.

`docs_only_human/` contains human-facing documentation. Do not load files from that directory during ordinary agent work unless the user explicitly requests the human-facing explanation.

- `phase:requirements`: read `docs/agent-workflow/requirements.md`
- `phase:design`: read `docs/agent-workflow/design.md`
- `phase:ready` or `phase:implementation`: read `docs/agent-workflow/implementation.md`
- `phase:review` or an open associated Pull Request: read `docs/agent-workflow/review.md`
- merged Pull Request and closed Issue: no phase workflow document is required

Reconcile stale labels before continuing.

### Required lifecycle

```text
Request
  -> Issue created or located
  -> Requirements
  -> Design
  -> Approval
  -> Ready
  -> Implementation and verification
  -> Pull Request and review
  -> Merge
  -> Issue closed
```

The initial Issue may contain only the original request and a note that planning is in progress. Keep draft requirements and design in the active conversation during a short planning session.

If planning must continue in another session and information loss would be risky, add one concise checkpoint comment with decisions, remaining questions, and the next action. Do not repeatedly rewrite the Issue body.

After approval, update the Issue with the approved requirements, non-goals, design summary, and verifiable acceptance criteria before implementation. Detailed implementation and test evidence belong primarily in the Pull Request.
<!-- END ISSUE-DRIVEN AGENT WORKFLOW -->

## Project state

This repo is **elwindui**, the implementation project for **ElwindUIL**: a declarative, Rust-flavored layout DSL for building GUIs that compile to native OS toolkit backends (WinUI 3 / AppKit / GTK4). This is a Cargo workspace (`crates/*` + `examples/*`, no root `src/`) with a real, substantial implementation — not just a spec: `elwindui-codegen` (the `.elwind` → Rust compiler, both a `build.rs`-driven path and a `component!`/`#[viewmodel]` proc-macro path), `elwindui-core` (the `UIElement` runtime), `elwindui-macros`, `elwindui-i18n`, `elwindui-languageserver`, and `elwindui-backend-appkit` (built, run, and screenshot-verified on this machine) are all real. `elwindui-backend-winui3` has code but is unverified (no Windows environment); `elwindui-backend-gtk4` and hot reload (`elwindui-hotreload`) are stubs; there is no preview-tool crate at all yet. See `docs/elwindui_implementation_status.md` for the full, regularly-stale-prone breakdown of what's implemented vs. still just spec — check it, and re-verify against `crates/` directly, before assuming a feature described in the spec docs actually exists.

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
- 付録F — reference implementations of `Window`/`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`TextArea`/`Dropdown` (the layout containers are named `VerticalLayout`/`HorizontalLayout`, not `Row`/`Column`; text display is `TextBlock`, not `Text`).
- 付録G/N — custom drawing (`Canvas`/`Painter`) and its Composition-style extensions (gradients/shadows/transforms/animation).
- 付録L — `NavigationHost`/`Route` screen navigation.
- 付録M — `Dialog`/`Menu`/`MenuItem`/`Tooltip` (dialogs, context menus, tooltips).
- 付録Q — `VirtualList` (large-list virtualization).
- 付録T — `platform::clipboard`/`platform::file_dialog`, drag & drop.
- 付録X/Y — `MenuBar`/`MenuBarItem` (native app menu bar) and `TabView`/`TabItem` (multi-document tabs), added for the notepad example.

## Core architectural rules to preserve when implementing

- **Public APIs require rustdoc**: every newly added or changed public type, trait, enum variant,
  field, function, method, macro, and generated public item must have useful `///`/`//!`
  documentation written in English. Document behavioral contracts and sentinel/reset semantics
  (for example, `PlatformDefault`) rather than merely repeating the item name; add a compilable
  example when the API is not self-explanatory.
- **`param` vs `prop`**: `#[param]` fields are fixed at instantiation and may only use static-evaluable expressions (literals, other params, pure builtins, `env::*`, `once` values) — never `bind!`, prop references, or impure calls. Default (`prop`) fields are runtime-mutable and support `bind!`/`#[computed]`. This split is what the §14 rules exist to enforce; don't weaken it for convenience.
- **Enums are the only value-set mechanism** — no anonymous unions. `match` over an enum must be exhaustive; missing arms are a compile error by design. Note: the spec's built-in `Backend` and `Route` enums (and `target::backend()`/`NavigationHost` themselves) are **not implemented yet** — see `docs/elwindui_implementation_status.md` — so this exhaustiveness rule currently only bites for user-defined enums, not those two.
- **`native!` and `target::backend()` are restricted**: only reachable from `#[overrides(builtin::X)]` components or other builtins — arbitrary user components must not call into backend-specific code directly (rules 9/15). This is a forward-looking rule: `target::backend()` itself doesn't exist in code yet (backend selection today is Cargo feature flags — `backend-appkit`/`backend-winui3`/`backend-gtk4` on the `elwindui` facade crate), so there's nothing to enforce this against currently.
- **`store`/`viewmodel` are never read directly from `#[param]`** — access always goes through `prop` + `bind!` (rule 12/13), and `viewmodel` internals aren't reachable from builtin view elements (rule 19), keeping MVVM's V/VM separation statically enforced.
- **Builtin shadowing must be explicit** — a user `component` sharing a name with a `builtin::` element is a static ambiguity error unless annotated `#[overrides(builtin::X)]`; there is no implicit shadowing.
- **Rust class-hierarchy convention (both codegen output and hand-written runtime code)**: for a class `Class` (with parent `SuperClass`), define `trait Class: SuperClass` + `struct ClassImpl { base: SuperClassImpl, /* own fields */ }`, with `ClassImpl` implementing `Class` and every ancestor trait (each ancestor method delegating to `self.base.method(...)`). The root class (no parent) has no `base` field. Construct via a `create_class(...)` factory function, never a bare struct literal. See docs/elwindui_spec.md 付録H.2.1a for the full rule and `elwindui-core::ui`'s `UIElement`/`Control`/etc. hierarchy for the reference implementation.
- **Don't unilaterally invent exceptions to an established codebase convention/rule** (e.g. `#[class]`'s normal bare-name struct declaration, the class-hierarchy convention above, or any other documented pattern) to work around a problem you haven't fully root-caused yet. If a normal-looking case seems to require a special-cased workaround, verify that the workaround is actually necessary first (re-check the mechanism in question — e.g. what name a macro actually emits, not just what's written at the call site) rather than assuming and coding around it. If a real exception does turn out to be needed, flag it to the user and get confirmation before writing it, rather than deciding and applying it silently.

## Commands

- `cargo build --workspace` / `cargo test --workspace` — build/test every crate and example.
- `cargo run -p notepad` / `cargo run -p notepad-inline` — run the example apps (AppKit backend on macOS; see the screenshot section below).
- Edition 2024. Root `Cargo.toml` is workspace-only (`members = ["crates/*", "examples/*"]`) — there is no root `src/`.

## Verifying with rust-analyzer after code changes

`cargo build`/`cargo test` passing is not the same as the IDE being clean — this workspace has proc-macros (`#[class]`, `component!`, `#[viewmodel]`) whose generated code can look fine to rustc but still misbehave under rust-analyzer's own (incremental, cross-crate-process-sharing) analysis model; see `docs/elwindui_macro_class_spec.md` §15 for a real example (a bug that only ever showed up via `rust-analyzer diagnostics`, never via `cargo build`). After a code change, run `rust-analyzer diagnostics .` (installed via `rustup component add rust-analyzer` if not already present) from the repo root — it runs the real rust-analyzer engine over the whole workspace with no editor needed and prints every diagnostic (errors and warnings) an IDE session would show. Fix whatever is fixable (real errors, `unused_variables`/similar lints, actionable style suggestions like `remove-unnecessary-else`) the same way you would for a `cargo build` warning. Ignore `"inactive-code"` diagnostics on `#[cfg(test)] mod ...` blocks — that's rust-analyzer correctly showing test code as inactive outside test-analysis mode, not a bug.

## Taking screenshots of a running example app (AppKit backend etc.)

**Preferred: `tools/macos-ui-driver`** (see `docs/elwindui_macos_gui_test_driver_status.md` for what's implemented — Phase 1 only: launch/terminate/list-windows/capture-window/doctor; Phase 2+, Accessibility-tree walking and control interaction, is not yet built). Build once with `swift build` inside that directory, then:

```bash
BIN=$(cd tools/macos-ui-driver && swift build --show-bin-path)/macos-ui-driver
"$BIN" doctor   # check Screen Recording / Accessibility permission state first
"$BIN" launch --path target/debug/notepad --wait-window-timeout 5   # polls for the window, no fixed sleep
# -> pull "pid"/"window"."window_id" out of the printed JSON
"$BIN" capture-window --window-id <id> --out /tmp/window.png   # crops to just that window, Retina-correct
"$BIN" terminate --pid <pid>
```

Every command prints one JSON object (`{"success": true, ...}` / `{"success": false, "error": "..."}`) and sets the exit code accordingly — always capture the specific window, never the full screen (a full-screen capture pulls in the menu bar, desktop, and unrelated windows and wastes context), which is exactly what `capture-window` does.

**Fallback** (no build step, useful for a one-off manual check): get the target window's `CGWindowID` via a tiny Swift snippet (no Accessibility permission needed, only Screen Recording), then pass it to `screencapture -l<id>`:

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
to perform the click manually and then capture the window screenshot afterward. `macos-ui-driver`'s own `doctor`
command is the fastest way to check both permissions' current state before attempting either path.

## Windows

When working on Windows, follow the additional instructions in [`docs/agents/windows.md`](docs/agents/windows.md).
