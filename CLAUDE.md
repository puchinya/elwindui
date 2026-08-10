# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Communication

When asking the user a question (clarifying questions, `AskUserQuestion`, plan checkpoints, etc.), always ask in Japanese.

<!-- BEGIN ISSUE-DRIVEN AGENT WORKFLOW -->
## Issue-driven development workflow

Use this workflow only for requests expected to modify this repository. Do not create an Issue for explanation, research, exploratory design discussion, or code-reading tasks unless the user explicitly asks to track the work.

### Common rules

- Before starting a new requirement or separate work item, check out `master` and run
  `git pull --ff-only origin master`. Only then create or locate its Issue and start work on the
  appropriate branch.
- Search for an existing relevant Issue before creating a new one.
- Every repository-changing task must be associated with one GitHub Issue.
- Create or locate the Issue before modifying source code or documentation.
- When a feature has backend-specific implementation or verification work that can progress
  independently, keep shared API/specification decisions in the parent Issue and create one
  sub-issue per in-scope backend before changing that backend. Do not create empty sub-issues for
  explicitly out-of-scope backends; record the exclusion and reason in the parent instead.
- Treat each backend sub-issue as its own lifecycle unit: give it acceptance criteria and one
  `phase:*` label, name its branch from the sub-issue number, close it from its associated Pull
  Request, and keep the parent Issue's sub-issue roll-up current. Close the parent only after every
  required backend sub-issue is merged and the shared acceptance criteria are satisfied.
- Prefer one Pull Request per backend sub-issue. A single atomic Pull Request may close multiple
  backend sub-issues only when the user approves the combined scope and the Pull Request explains
  why the shared change cannot be reviewed or landed independently. If backend-specific work is
  discovered while implementing an older broad Issue, create and link the sub-issue before
  continuing and leave only the roll-up summary in the parent.
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
<!-- BEGIN LOCAL STATE AND EVIDENCE -->
### Local state and evidence

- For resuming or pausing incomplete Issue work, read `docs/agent-workflow/checkpoint.md`; otherwise do not load it.
- Store local state under `.agent-state/issues/<issue-number>/`; never commit `.agent-state/`.
- On resume, compare the checkpoint with Issue/PR, branch, HEAD, and worktree. Git and GitHub override stale local state.
- Before pausing, record completed work, current state, one concrete next action, checks, uncommitted files, and blockers.
- For screenshots or logs, read `docs/agent-workflow/evidence.md`; otherwise do not load it.
- Keep temporary screenshots and logs under `.agent-state/`. Commit only small durable review evidence; use CI artifacts for large data.
- Local state is not shared between macOS and Windows. Add one concise Issue checkpoint comment before cross-machine handoff.
<!-- END LOCAL STATE AND EVIDENCE -->
<!-- END ISSUE-DRIVEN AGENT WORKFLOW -->

## Project state

This repo is **elwindui**, the implementation project for **ElwindUIL**: a declarative, Rust-flavored layout DSL for building GUIs that compile to native OS toolkit backends (WinUI 3 / AppKit / GTK4). This is a Cargo workspace (`crates/*` + `examples/*`, no root `src/`) with a real, substantial implementation — not just a spec: `elwindui-codegen` (the compiler backing three Rust proc-macros — `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` — the only supported input form), `elwindui-core` (the `UIElement` runtime), `elwindui-macros`, `elwindui-i18n`, `elwindui-languageserver` (operates on a single `.rs` file at a time), and `elwindui-backend-appkit` (built, run, and screenshot-verified on this machine) are all real. `elwindui-backend-winui3` has code and selected NativeControls have been built and interaction-tested on Windows, but backend-wide verification remains incomplete; `elwindui-backend-gtk4` and hot reload (`elwindui-hotreload`) are stubs; there is no preview-tool crate at all yet. See `docs/status/implementation_status.md` for the full, regularly-stale-prone breakdown of what's implemented vs. still just spec — check it, and re-verify against `crates/` directly, before assuming a feature described in the spec docs actually exists.

`docs/README.md` is the entry point: it indexes every doc, defines the `docs/specs` (normative spec) /
`docs/design` (how it is built) / `docs/status` (what actually works today) split, and explains the
✅/🚧/📋 implementation-status badges carried on section headings in specs and design docs.

The authoritative source of truth is split across three Japanese-language docs, each scoped to a
different concern. All three are long — read the relevant section rather than the whole file.
Section map (grep each file for these headers):

`docs/specs/dsl_spec.md` (ElwindUIL **DSL syntax only** — no backend/runtime/state-management content):
- §1–§14 — core language: `component`/`view` split and namespace (§2 — builtins resolve to the real Rust path `elwindui::ui::*`, ordinary Rust crate-namespace rules apply throughout, no `builtin::`/`platform::` pseudo-namespace), `param`/`prop`/`state`, control flow, `style`, constraints, `enum`, `env::*`/`once!`, reactive attribute expressions and `<=>`, i18n (Fluent), imports, an overview of the `UIElement` tree-exploration contract, and the full list of static verification rules (§13, numbered 1-32 with 2 vacant slots) a future compiler/linter must implement.
- 付録A — the `#[sealed]`/`#[abstract]`/`#[text_style]`/`#[content(field_name)]` component-level attributes (static verification only; the builtins themselves are in `docs/specs/builtins_spec.md` 付録F).

`docs/design/gui_framework_design.md` (GUI framework implementation — backend abstraction, runtime, state management; **not** a summary of the DSL spec, this is the primary source for these topics):
- §1/§3 — backend abstraction: common AST → per-backend codegen, `target::backend()` compile-time constant, OS-native toolkit mapping.
- §5 — core runtime: the `UIElement`/`UIElementExt` class hierarchy (§5.1/§5.1a — the Rust class-hierarchy convention lives here), Logical/Visual tree split, layout engine, focus, accessibility, `Canvas`/`Painter`, routing events.
- §6 — lifecycle hooks (`on_mount`/`on_unmount`/`on_update`, app-level `on_foreground`/`on_background`/`on_terminate`).
- §7 — `store` (global/scoped shared state), `viewmodel`/`Command` (MVVM), async, undo/redo.
- §8 — keyboard shortcuts, navigation, dialogs, virtual lists, theme/design tokens, error boundaries, platform APIs, mobile.
- §9 — snapshot testing.

`docs/specs/builtins_spec.md` (every concrete `builtin::`/`platform::` element):
- 付録F — reference implementations of `Window`/`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`TextArea`/`Dropdown` (the layout containers are named `VerticalLayout`/`HorizontalLayout`, not `Row`/`Column`; text display is `TextBlock`, not `Text`).
- 付録G/N — custom drawing (`Canvas`/`Painter`) and its Composition-style extensions (gradients/shadows/transforms/animation).
- 付録L — `NavigationHost`/`Route` screen navigation.
- 付録M — `Dialog`/`Menu`/`MenuItem`/`Tooltip` (dialogs, context menus, tooltips).
- 付録Q — `VirtualList` (large-list virtualization).
- 付録T — `platform::clipboard`/`platform::file_dialog`, drag & drop.
- 付録X/Y — `MenuBar`/`MenuBarItem` (native app menu bar) and `TabView`/`TabItem` (multi-document tabs), added for the notepad example.

Toolchain design (proc-macro codegen, LSP, live preview, hot-reload) lives in
`docs/design/tools/*.md`, not in the three docs above. `docs/specs/macro_class_spec.md`
is the authoritative spec for `#[elwindui_macros::class]` and takes precedence over
`docs/design/gui_framework_design.md`'s §5.1a summary if the two ever disagree.

## Core architectural rules to preserve when implementing

- **Public APIs require rustdoc**: every newly added or changed public type, trait, enum variant,
  field, function, method, macro, and generated public item must have useful `///`/`//!`
  documentation written in English. Document behavioral contracts and sentinel/reset semantics
  (for example, `PlatformDefault`) rather than merely repeating the item name; add a compilable
  example when the API is not self-explanatory.
- **`param` vs `prop`**: `#[param]` fields are fixed at instantiation and may only use static-evaluable expressions (literals, other params, pure builtins, `env::*`, `once!` values) — never reactive prop references or impure calls. Default (`prop`) fields are runtime-mutable and support reactive attribute expressions/`#[computed]`. This split is what the §13 rules exist to enforce; don't weaken it for convenience.
- **Enums are the only value-set mechanism** — no anonymous unions. `match` over an enum must be exhaustive; missing arms are a compile error by design. Note: the spec's built-in `Backend` and `Route` enums (and `target::backend()`/`NavigationHost` themselves) are **not implemented yet** — see `docs/status/implementation_status.md` — so this exhaustiveness rule currently only bites for user-defined enums, not those two.
- **`native!` and `target::backend()` are restricted**: only reachable from builtin definitions — arbitrary user components must not call into backend-specific code directly (rules 9/15). This is a forward-looking rule: `target::backend()` itself doesn't exist in code yet (backend selection today is Cargo feature flags — `backend-appkit`/`backend-winui3`/`backend-gtk4` on the `elwindui` facade crate), so there's nothing to enforce this against currently.
- **`store`/`viewmodel` are never read directly from `#[param]`** — access goes through reactive `prop` expressions or explicit `<=>` on a writable target (rule 12/13), and `viewmodel` internals aren't reachable from builtin view elements (rule 19), keeping MVVM's V/VM separation statically enforced.
- **Builtin naming follows ordinary Rust scoping — there is no DSL-level shadowing mechanism**: `view!` bodies auto-import `elwindui::ui::*` (a glob, `docs/specs/dsl_spec.md` §2), and Rust's own name resolution always prefers a local/explicit item over a glob import. A user `component` sharing a name with a builtin (e.g. `Button`) is therefore not ambiguous — the local one wins in that scope, with no special annotation or static rule needed to declare the shadowing intentional.
- **Rust class-hierarchy convention (both codegen output and hand-written runtime code)**: for a class `Class` (with parent `SuperClass`), define `struct Class { base: SuperClass, /* own fields */ }` (bare struct name, no suffix) + `trait ClassExt: SuperClassExt`, with `Class` implementing `ClassExt` and every ancestor trait (each ancestor method delegating to `self.base.method(...)`). The root class (no parent) has no `base` field. Construct via a `create_class(...)` factory function, never a bare struct literal. See `docs/specs/macro_class_spec.md` (authoritative) and `docs/design/gui_framework_design.md` §5.1a for the full rule, and `elwindui-core::ui`'s `UIElement`/`Control`/etc. hierarchy for the reference implementation.
- **Backend crates share one layered module structure** — `elwindui-backend-appkit` and `elwindui-backend-winui3` are laid out file-for-file the same way, and dependencies run strictly one direction: `native_ui -> inner -> host -> render -> ffi`. `native_ui/` is the public façade (one `#[class]` per builtin, delegating to its `inner` twin, no toolkit calls); `inner/` is raw per-control plumbing, one file per control family; `host/` owns the tree host view; `render/` does drawing only and must not know about `UIElement`, focus, or any control; `ffi.rs` is the toolkit seam holding `AnyView`. Adding a helper means putting it at the layer that owns the concept, not wherever the caller happens to be — a `render` file importing from `host` is a bug in the layering. See `docs/status/implementation_status.md` §6.9. Logic that is pure `elwindui_core` value math (rect/geometry/image-fit) belongs in `elwindui-core`, not duplicated per backend.
- **Don't unilaterally invent exceptions to an established codebase convention/rule** (e.g. `#[class]`'s normal bare-name struct declaration, the class-hierarchy convention above, or any other documented pattern) to work around a problem you haven't fully root-caused yet. If a normal-looking case seems to require a special-cased workaround, verify that the workaround is actually necessary first (re-check the mechanism in question — e.g. what name a macro actually emits, not just what's written at the call site) rather than assuming and coding around it. If a real exception does turn out to be needed, flag it to the user and get confirmation before writing it, rather than deciding and applying it silently.

## Commands

- `cargo build --workspace` / `cargo test --workspace` — build/test every crate and example.
- `cargo run -p notepad` — run the example apps (AppKit backend on macOS; see the screenshot section below).
- `cargo run -p graphics-demo` — run the standing visual verification tool for `elwindui_core::graphics` (a `TabView` of labeled feature demos, one tab per submodule area); re-run and screenshot this whenever `graphics` changes.
- Edition 2024. Root `Cargo.toml` is workspace-only (`members = ["crates/*", "examples/*"]`) — there is no root `src/`.

## Verifying with rust-analyzer after code changes

`cargo build`/`cargo test` passing is not the same as the IDE being clean — this workspace has proc-macros (`#[class]`, `#[elwindui::component]`, `#[elwindui::viewmodel]`) whose generated code can look fine to rustc but still misbehave under rust-analyzer's own (incremental, cross-crate-process-sharing) analysis model; see `docs/specs/macro_class_spec.md` §15 for a real example (a bug that only ever showed up via `rust-analyzer diagnostics`, never via `cargo build`). After a code change, run `rust-analyzer diagnostics .` (installed via `rustup component add rust-analyzer` if not already present) from the repo root — it runs the real rust-analyzer engine over the whole workspace with no editor needed and prints every diagnostic (errors and warnings) an IDE session would show. Fix whatever is fixable (real errors, `unused_variables`/similar lints, actionable style suggestions like `remove-unnecessary-else`) the same way you would for a `cargo build` warning. Ignore `"inactive-code"` diagnostics on `#[cfg(test)] mod ...` blocks — that's rust-analyzer correctly showing test code as inactive outside test-analysis mode, not a bug.

## Taking screenshots of a running example app (AppKit backend etc.)

**Preferred: `tools/macos-ui-driver`** (see `docs/status/macos_ui_driver_status.md` for what's implemented — Phase 1 only: launch/terminate/list-windows/capture-window/doctor; Phase 2+, Accessibility-tree walking and control interaction, is not yet built). Build once with `swift build` inside that directory, then:

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
