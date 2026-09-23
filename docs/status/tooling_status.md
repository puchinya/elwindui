# Tooling status

Snapshot: 2026-09-21. Tool architecture is indexed in [`../design/README.md`](../design/README.md).

## Current capability matrix

| Tool | State | Current capability / gap |
|---|---|---|
| `elwindui-codegen` | 🚧 | Component/ViewModel/enum/ControlTemplate parsing, diagnostics, shared semantic planning/emission, bindings, dynamic regions, ownership, environment propagation, deferred views, cleanup, explicit-target templates, and component-default templates are implemented. Targets are not inferred, the public `#[control_template]` marker is absent, and raw framework/class-managed property bridges are not synthesized. |
| `elwindui-languageserver` | 🚧 | Single-file diagnostics, member completion, and DSL semantic tokens; cross-file resolution, hover, and generated-code preview are incomplete. |
| Preview | ⬜ | Design exists; no workspace preview application. |
| `elwindui-hotreload` | 🚧 | Patch/Remount decision helper exists; artifact loading and live replacement are absent. |
| `elwindui-test` | 🚧 | Render-tree dump exists; canvas/image snapshots are absent. |
| `macos-ui-driver` | 🚧 | Process/window control, focus, Accessibility queries/actions, screenshots, coordinate clicks, Core-backed identifiers, direct AX text/numeric value setting, real press/drag/release, and native resize gestures are implemented; full keyboard synthesis and every AX action are incomplete. |
| `windows-ui-driver` | 🚧 | Process/window control, UIA inspect/search/invoke/get-value/set-value/get-property/set-focus/wait-for, real mouse click/drag, duration-controlled real mouse drag pass-through, screenshot (window and screen-capture modes), move/resize, and the bounded cancellation-only `touch-cancel` Windows synthetic-pointer stimulus are implemented over the external `winapp` CLI/Windows API; deterministic adapter contract tests pass. The driver contains no generic mouse injector. The duration acceptance build is the pinned `puchinya/winappCli` fork based on Microsoft winappCli v0.6.1; native evidence records its commit, executable SHA-256, and version. The modern synthetic-pointer backend is primary, legacy touch injection is fallback-only, and a physical touchscreen is not required. |
| Shared native E2E orchestration | ⬜ | Backend-neutral durable cases, deterministic compilation, reusable local plan cache, batch runner, bounded vision checkpoints, and animation capture sequence are planned but not implemented. |

## Native E2E orchestration state

The shared architecture is [`native_e2e_orchestration_design.md`](../design/tools/native_e2e_orchestration_design.md).
The platform drivers above exist, but the shared durable case runner/compiler/cache is planned and
not implemented. No durable shared product E2E case exists yet; durable cases remain deferred to
[`tests/e2e/`](../../tests/e2e/README.md). The animation `capture-sequence` capability is planned,
not implemented, and no AI image-recognition pipeline is claimed.

The required Codex tester routing policy is `gpt-6-luna` with child reasoning effort explicitly
`medium`. The repository does not currently prove explicit child-effort enforcement or attestation;
the policy must not be described as enforced, and no speculative `.codex/config.toml` key is used.

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

The Windows UI driver and deterministic adapter-contract tests are implemented. The first durable
shared product case is [`tests/e2e/self-drawn-pointer-input.md`](../../tests/e2e/self-drawn-pointer-input.md)
and is executed through this driver for WinUI3 Issue #236. The final remediation run against
implementation HEAD `98e5f9e204bfc0b5ae120c56b5a72f57e674fcde` reports SDP-01 `PASS`, SDP-02
`PASS`, SDP-03 `PASS`, SDP-04 `PASS`, and SDP-05 `PASS`. For SDP-05, the requested `1100x850`
resize did not fit the usable desktop, so the largest safe `908x476` window was used; a fresh
post-resize screenshot and one-Tab focus location identified `Normal` at local `(50,120)` / screen
`(102,172)`, and one real mouse click appended exactly one `Normal clicked` line. UIA returned no
match, but Issue #260 UIA discoverability is not an Issue #236 acceptance dependency. Raw evidence
is under `.agent-state/issues/236/e2e/98e5f9e204bfc0b5ae120c56b5a72f57e674fcde/20260916T142131Z/`.
This does not claim the shared runner/compiler/cache, which is separate from the durable case
acceptance recorded above.

Issue #267 adds the durable native cancellation/capture-loss case at
[`tests/e2e/pointer-cancellation-capture-loss.md`](../../tests/e2e/pointer-cancellation-capture-loss.md),
the bounded `touch-cancel` command, and private WinUI3 trace/capture-loss instrumentation. The
driver contract test suite, including backend selection, error taxonomy, lifecycle cleanup shape,
touch-cancel usage, and one-object JSON checks, passes. The modern synthetic-pointer API is
primary; legacy `InjectTouchInput` is fallback-only, and physical touch hardware is not a
capability gate. The prior RDP error-87 classification as `environment_blocker` is superseded:
error 87 is `tool_error` unless a separately documented API/session condition proves otherwise.
The implementation/tooling capability is complete in Issue #267 / PR #269. The NC-01..NC-09 and
NC-11 local native matrix remains pending under [Issue #270](https://github.com/puchinya/elwindui/issues/270)
and is not claimed by #267. The pre-remediation RDP legacy Error-87 classification is historical
and superseded, not acceptance evidence. On the new remediation working-tree run, Windows 10 Pro
build 19045 / `SM_REMOTESESSION=1` accepted the modern synthetic-pointer sequence
(`injection_backend:"synthetic-pointer"`), but the application observed a normal release rather
than cancellation; the capture-loss diagnostic was also blocked by foreground ownership. A normal
local interactive Windows host must still produce both trace and visible-probe evidence.

For PR #241 remediation, the comparable `rust-analyzer diagnostics .` run with the repository
Visual Studio environment passed on both base `766c2a9ab24632e639e02e232fd2e861d834caad` and
implementation HEAD: base had 248 allowed `Ra("inactive-code", WeakWarning)` records and the
implementation had 249, with no Error or Warning diagnostics. The hosted-XAML focused test
passed on both base and implementation after the implementation's narrow test-only lifecycle
cleanup. `cargo check --workspace`, `cargo build --workspace`, the three package test commands,
and `cargo test --workspace` all passed.

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
