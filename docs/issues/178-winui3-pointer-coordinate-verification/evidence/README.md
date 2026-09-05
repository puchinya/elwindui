# Issue #178 — WinUI3 pointer and coordinate verification evidence

## Current verification — 2026-09-05

**Status: BLOCKED by `ENVIRONMENT_LIMITATION`.** The latest-master build and automated WinUI3 gates pass, but the required live matrix could not be executed because the approved CUA surface exposed no native Windows apps or windows. No runtime `PASS` is claimed.

This remains verification-only. No product source, test, or backend implementation file was changed.

## Environment and execution context

- Branch: `feature/178-winui3-pointer-coordinate-verification`
- Required `origin/master` merge commit: `d96cd7a4fa2c992a2c3ffb49be8bbd0fbd7d6b14`
- Current verification commit: `b379620` (`docs: record Issue 178 runtime environment limitation`)
- `origin/master`: `1b398f213744...`
- Host: Windows `x86_64-pc-windows-msvc`, `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `cargo 1.97.1 (c980f4866 2026-06-30)`
- User token/integrity: standard interactive user, `Medium Mandatory Level`; no administrator-only success was used as acceptance.
- Active display enumeration (`System.Windows.Forms.Screen::AllScreens`): primary `\\.\DISPLAY49`, bounds `0,0–1020,651`, working area `0,0–1020,611`, 32-bit color.
- WMI exposed `192` logical pixels per inch for the generic monitor entries (200% scale), but no usable per-monitor mapping was available for a live probe window. No secondary or negative-coordinate desktop was available to the UI automation surface.
- CUA state while the probe and `controls-demo` were running: `apps: []`; only the Codex in-app browser was listed. The WinUI3 processes were alive and responsive but had `MainWindowHandle = 0`, no discoverable title/rectangle, and no selectable UI surface.

## Probe and trace

The ignored agent-local probe at `.agent-state/issues/178/verification-app/` was recreated with public ElwindUI APIs only. It contains:

- one self-drawn target with rendered glyph text and a blank interior;
- routed `pressed` / `moved` / `released` / `right_tapped` handlers;
- `root_to_screen` / `screen_to_root` calls;
- adjacent native Button and TextBox controls;
- a bounded JSONL trace under `.agent-state/issues/178/results/trace.jsonl`.

The probe compiled successfully and entered its executable process. The trace contains only its metadata line; no pointer or native-control event was generated because no UI surface could be selected. The process was then stopped with Ctrl+C. `controls-demo` was also built and reached its executable process, but had the same `MainWindowHandle = 0` / CUA `apps: []` limitation and was stopped without input.

## Automated build and test evidence

All commands below were run after `. .\tools\setup-vs-env.ps1` unless noted otherwise.

| Command | Result | Evidence |
|---|---|---|
| `cargo check -p elwindui-backend-winui3` | PASS | Exit 0; only the known binding-generation omission warning. |
| `cargo build -p controls-demo` | PASS | Exit 0. |
| `cargo test -p elwindui-core pointer_dispatch_preserves_backend_screen_position_during_capture` | PASS | 1 passed, 0 failed. |
| `cargo fmt --all -- --check` | PASS | Clean. |
| `rust-analyzer diagnostics .` | PASS | Exit 0 after setup-vs-env; only cfg-only inactive-code warnings were reported. |
| `cargo check --workspace` | PASS | Exit 0. |
| `cargo build --workspace` | PASS | Exit 0. |
| `cargo test -p elwindui-backend-winui3` | PASS | 42 passed, 0 failed; hosted XAML Button/Text/Window assertions executed. |
| `cargo test --workspace` | PASS | Exit 0; all workspace tests and doctests passed. |
| `cargo fmt --manifest-path .agent-state/issues/178/verification-app/Cargo.toml -- --check` | PASS | Probe formatting clean. |
| `cargo check --manifest-path .agent-state/issues/178/verification-app/Cargo.toml` | PASS | Probe compiles in WinUI3 host context. |
| `git diff --check` | PASS | No whitespace errors. |

The previous #207 compile blocker is not present on this branch: the focused backend check, `controls-demo` build, backend suite, workspace check/build/test, and probe check all pass. #207 is retained below only as historical context.

## Pointer matrix

The contract requires real OS mouse input against the actual native window. Because CUA could not select a WinUI3 window, every live row is explicitly classified as `ENVIRONMENT_LIMITATION` and linked to follow-up [Issue #224](https://github.com/puchinya/elwindui/issues/224).

| Case | pressed | moved outside | released | target / result |
|---|---:|---:|---:|---|
| P1 blank-area click | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P2 glyph click | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| P3 captured drag outside | — | — | — | target identity and capture not collected — `ENVIRONMENT_LIMITATION`, [#224](https://github.com/puchinya/elwindui/issues/224) |
| P4 post-release / native Button | — | — | — | stuck capture and duplicate Core events not collected — `ENVIRONMENT_LIMITATION`, [#224](https://github.com/puchinya/elwindui/issues/224) |
| P5 native TextBox/TextArea ownership | — | — | — | native input/focus and self-drawn counter not collected — `ENVIRONMENT_LIMITATION`, [#224](https://github.com/puchinya/elwindui/issues/224) |
| P6 right tap | — | — | — | `on_right_tapped` / context-menu result not collected — `ENVIRONMENT_LIMITATION`, [#224](https://github.com/puchinya/elwindui/issues/224) |

Issue #180 cancellation/capture-loss behavior was not intentionally exercised or claimed.

## Coordinate and topology matrix

No `P`, `S`, `R`, or `P'` values are synthesized. The ≤1.0 DIP/axis comparison is therefore not evaluated.

| Scenario | Monitor / bounds / DPI | P | S (`screen_position`) | R (`root_to_screen(P)`) | P' (`screen_to_root(S)`) | `|S-R|` x/y | `|P'-P|` x/y | Result |
|---|---|---|---|---|---|---|---|---|
| Primary | `\\.\DISPLAY49`, `0,0–1020,651`, WMI 192 PPI | — | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| Secondary | no selectable secondary display in active topology | — | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| Negative desktop coordinates | no selectable negative-coordinate monitor | — | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |
| Different DPI | no selectable second monitor / per-monitor mapping; WMI generic entries reported 192 PPI | — | — | — | — | — | — | `ENVIRONMENT_LIMITATION` — [#224](https://github.com/puchinya/elwindui/issues/224) |

## Acceptance mapping

| Requirement | Result | Owning evidence / follow-up |
|---|---|---|
| self-drawn blank and glyph hit ownership | `ENVIRONMENT_LIMITATION` | No CUA-selectable native window; [#224](https://github.com/puchinya/elwindui/issues/224) |
| pressed/moved/released cardinality and captured drag | `ENVIRONMENT_LIMITATION` | Trace contains metadata only; [#224](https://github.com/puchinya/elwindui/issues/224) |
| post-release capture release and native Button ownership | `ENVIRONMENT_LIMITATION` | `controls-demo` process had no selectable UI surface; [#224](https://github.com/puchinya/elwindui/issues/224) |
| native TextBox/TextArea input ownership | `ENVIRONMENT_LIMITATION` | No real input delivered; [#224](https://github.com/puchinya/elwindui/issues/224) |
| right tap and context-menu placement | `ENVIRONMENT_LIMITATION` | No real input delivered; [#224](https://github.com/puchinya/elwindui/issues/224) |
| root/screen round trip and screen-position match | `ENVIRONMENT_LIMITATION` | No `P/S/R/P'` sample; [#224](https://github.com/puchinya/elwindui/issues/224) |
| secondary, negative-coordinate, and mixed-DPI topology | `ENVIRONMENT_LIMITATION` | Topology not available to the UI surface; [#224](https://github.com/puchinya/elwindui/issues/224) |
| #180 cancellation/capture-loss separation | PASS (scope separation) | No cancellation behavior was intentionally exercised. |

## Findings and history

- `ENVIRONMENT_LIMITATION`: live WinUI3 OS-input verification is blocked by the missing CUA/native-window surface. The focused environment follow-up is [Issue #224](https://github.com/puchinya/elwindui/issues/224), milestone `0.1.0`, `phase:requirements` + `blocked`.
- `PASS`: latest-master automated build, backend tests, workspace tests, and probe compilation succeed in host context.
- Historical #207 blocker: the earlier evidence recorded a 49-error WinUI3 compile failure and routed all runtime rows to [Issue #207](https://github.com/puchinya/elwindui/issues/207). That blocker is resolved on the current branch; it is not the reason for the present runtime limitation.
- Issue #180 remains separate and is not absorbed into #178.

## Workflow state

```text
Issue #178: OPEN, phase:review
PR #208: OPEN; verification docs updated with current environment limitation
Issue #180: separate; not exercised
topology / interactive-host follow-up: #224
implementation regression issues: none
```

Completion is intentionally not claimed: PR #208 must not be merged and Issue #178 must not be closed until the required live matrix is executed, or the project owner explicitly accepts the classified environment follow-up.
