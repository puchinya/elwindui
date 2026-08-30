# Issue #178 — WinUI3 pointer and coordinate verification evidence

Verification was attempted on branch `feature/178-winui3-pointer-coordinate-verification` at commit `f2412f7ea807e66d780be57480c5be86453f07e6` (the Issue #178 baseline). This remained a verification-only change: no pointer or coordinate production code was changed.

## Environment

- Windows edition/build: unavailable. `Get-CimInstance Win32_OperatingSystem` returned `Access denied` in this environment.
- Architecture: `AMD64` / `x86_64-pc-windows-msvc`.
- `rustc -Vv`: `rustc 1.97.1 (8bab26f4f 2026-07-14)`, commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452`, LLVM `22.1.6`.
- `cargo -V`: `cargo 1.97.1 (c980f4866 2026-06-30)`.
- Real input method: none completed. The temporary public-API probe was prepared, but the WinUI3 crate failed to compile before a window could launch. No UI Automation pointer or capture result is claimed.
- Display enumeration exposed one primary display, `\\.\DISPLAY17`, bounds `0,0–1108,652` and working area `0,0–1108,612`. DPI/scaling was unavailable; no runtime scenario used this display. No secondary monitor or negative-coordinate monitor was available to the probe.

## Probe and ownership surface

The ignored temporary standalone Cargo application at `.agent-state/issues/178/verification-app/` uses only public ElwindUI APIs. It contains a self-drawn `TextBlock` glyph probe, routed pressed/moved/released/right-tapped handlers, bounded event traces, root/screen conversion calls, and adjacent native Button and TextBox controls. `cargo fmt --manifest-path .agent-state/issues/178/verification-app/Cargo.toml` passed. `cargo run` did not reach application launch because it hit the same backend compile failure recorded below.

Source inspection confirmed that the renderer proxy sets `IsHitTestVisible(false)` for self-drawn `TextBlock` content and that native-control branches remain native-owned. This is code evidence only; no click counts were collected.

## Build and test results

| Command | Result | Observation |
|---|---|---|
| `. .\tools\setup-vs-env.ps1; cargo check -p elwindui-backend-winui3` | FAIL | 49 WinUI3 compile errors; generated bindings also reported 7,688 omitted unsupported metadata members. |
| `. .\tools\setup-vs-env.ps1; cargo test -p elwindui-backend-winui3` | FAIL | Same 49-error backend compile blocker; tests did not run. |
| `cargo test -p elwindui-core pointer_dispatch_preserves_backend_screen_position_during_capture` | PASS | 1 passed, 0 failed. |
| `cargo build -p controls-demo` | FAIL | Same WinUI3 backend compile blocker; demo did not launch. |
| `cargo run --manifest-path .agent-state/issues/178/verification-app/Cargo.toml` | FAIL | Probe compilation stopped at the same backend blocker; no window launched. |
| `cargo fmt --all` | PASS | No formatting changes. |
| `cargo fmt --all -- --check` | PASS | Clean. |
| `rust-analyzer diagnostics .` | FAIL | Exited 1 with actionable `E0282` inference errors, inactive-code `WeakWarning`s, and lint warnings. |
| `cargo check --workspace` | FAIL | Same 49 WinUI3 backend compile errors. |
| `cargo build --workspace` | FAIL | Same 49 WinUI3 backend compile errors. |
| `cargo test --workspace` | FAIL | Same 49 WinUI3 backend compile errors. |
| `git diff --check` | PASS | No whitespace errors. |

The compile regression is tracked separately in [Issue #207](https://github.com/puchinya/elwindui/issues/207). Representative errors include missing `UI_Composition` bindings, missing generated WinUI types such as `IPropertySet`/`BitmapSource`, and unresolved `Point`, `Rect`, `UIElementExt`, `Microsoft`, and `AppWindowClosingEventArgs` references. No production fix was made in #178.

## Pointer routing and ownership results

No real pointer event trace was collected because the application could not launch. Therefore event counts, glyph/blank-area counts, drag-outside-element behavior, native capture release, right-tap delivery, and native-control before/after Core counts are all unavailable rather than passing.

## Coordinate table

No `P`, `S`, `R`, or `P'` sample exists because no window was launched. `—` means not collected; no tolerance result is inferred.

| Monitor/scenario | `P` | `S` (`screen_position`) | `R` (`root_to_screen(P)`) | `P'` (`screen_to_root(S)`) | `|S-R|` x/y | `|P'-P|` x/y | Result (≤1.0 DIP/axis) |
|---|---|---|---|---|---|---|---|
| Primary `DISPLAY17` | — | — | — | — | — | — | FOLLOW-UP — [#207](https://github.com/puchinya/elwindui/issues/207) |
| Secondary monitor | — | — | — | — | — | — | FOLLOW-UP — [#207](https://github.com/puchinya/elwindui/issues/207); no secondary display exposed |
| Negative desktop coordinates | — | — | — | — | — | — | FOLLOW-UP — [#207](https://github.com/puchinya/elwindui/issues/207); no negative-coordinate display exposed |
| Different-DPI monitor | — | — | — | — | — | — | FOLLOW-UP — [#207](https://github.com/puchinya/elwindui/issues/207); DPI unavailable |

## Regression and acceptance matrix

All runtime rows below are linked to #207 because the build failure prevented the required Windows application launch. This is not a claim that the product behavior failed at runtime.

| Issue #178 item | Result | Evidence / follow-up |
|---|---|---|
| PointerPressed | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| PointerMoved capture | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| PointerReleased capture | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| Drag outside element | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| Native capture released | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| TextBlock glyph hit | FOLLOW-UP | Probe prepared but not launched; [#207](https://github.com/puchinya/elwindui/issues/207) |
| No renderer dead zone | FOLLOW-UP | No runtime sample; code inspection only; [#207](https://github.com/puchinya/elwindui/issues/207) |
| NativeControl no duplicate | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| root/screen round trip | FOLLOW-UP | No `P/S/R/P'` samples; [#207](https://github.com/puchinya/elwindui/issues/207) |
| negative desktop coordinates | FOLLOW-UP | Topology unavailable and no runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| secondary monitor | FOLLOW-UP | No secondary display exposed and no runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| different DPI | FOLLOW-UP | DPI unavailable and no runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| screen_position match | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| context-menu placement | FOLLOW-UP | `controls-demo` did not build; [#207](https://github.com/puchinya/elwindui/issues/207) |
| `on_right_tapped` | FOLLOW-UP | No runtime sample; [#207](https://github.com/puchinya/elwindui/issues/207) |
| Button | FOLLOW-UP | `controls-demo` did not build; [#207](https://github.com/puchinya/elwindui/issues/207) |
| TextArea/TextBox | FOLLOW-UP | `controls-demo` did not build; [#207](https://github.com/puchinya/elwindui/issues/207) |
| TabView | FOLLOW-UP | `controls-demo` did not build; [#207](https://github.com/puchinya/elwindui/issues/207) |

## Findings and related work

- Implementation regression: the current #178 baseline cannot compile the WinUI3 backend. Follow-up: [Issue #207](https://github.com/puchinya/elwindui/issues/207).
- Environment limitation: OS edition/build and DPI could not be queried; display enumeration exposed only one primary display. These limitations are recorded, not treated as runtime passes.
- Cancellation/capture-loss work remains separate in [Issue #180](https://github.com/puchinya/elwindui/issues/180); it was not absorbed into #178.
- Related implementation and verification context: [Issue #174](https://github.com/puchinya/elwindui/issues/174), [PR #175](https://github.com/puchinya/elwindui/pull/175), [Issue #172](https://github.com/puchinya/elwindui/issues/172), and [Issue #173](https://github.com/puchinya/elwindui/issues/173).

No screenshots were committed because the probe never launched and there was no useful runtime state to capture.
