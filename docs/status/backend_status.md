# Backend status

Snapshot: 2026-09-12. Durable backend architecture is indexed in [`../design/README.md`](../design/README.md).

## Support matrix

| Backend | Current implementation | Current verification |
|---|---|---|
| AppKit (macOS) | ✅ primary backend | Local builds, workspace tests, screenshots, Accessibility-driven control interaction, and text/environment paths are supported where listed below. |
| WinUI 3 (Windows) | 🚧 substantial implementation | Windows builds and real interaction cover the established startup, Window lifecycle, graphics, text/environment, Button, and selection paths; the full contract and newer pointer/popup/docking rows remain incomplete or deferred. |
| GTK4 (Linux) | ⬜ stub | No functional backend or toolkit dependency. |
| UIKit / Android | ⬜ absent | No implementation. |

## AppKit current state

- Core/native input, screen-coordinate conversion, pointer cancellation, retained rendering, Window lifecycle, transparency, popup/context-menu behavior, and semantic environment/brush paths are implemented.
- Animation/transition projection uses the per-host Core runtime and stable AppKit native islands. Two clean-launch AppKit animation-demo E2E runs pass width interpolation, focused NativeControl exit visibility with recursive `AXTextField` suppression, completion removal, and Reduce Motion checks.
- `ContextMenu`/`PopupSurface` uses child-window ownership, dynamic visible-frame placement, deferred `ViewFactory` content, popup-scoped environment, and synchronous subtree teardown before ElwindUI host detach. AppKit unit and focused runtime evidence cover the supported paths.
- Native text widgets reduce unsupported gradient/image foreground brushes. TextArea character spacing does not cover every native text-storage path, and PasswordBox intentionally retains the system secure-font cascade.
- SVG filters, blend modes, masks, image-brush fills, and pattern cases use documented simpler fallbacks where Core Image or native drawing cannot represent the requested effect.
- Window hide/close, transparency, and floating/normal levels are implemented; interactive native close-button verification remains host-dependent where a test harness cannot construct an AppKit Window on the main thread.
- Issue #254: `crate::app`'s `WINDOWS` registry now strongly retains the final most-derived `Rc<dyn WindowExt>` from first `show()` until `ElwinduiWindow::windowWillClose:` releases it, closing the same class of premature-drop gap WinUI3 had (§WinUI 3 current state, below) — code-reviewed and mirrors the WinUI3 implementation exactly, but unverified: this Windows-only development session cannot compile or execute the AppKit backend (`#![cfg(target_os = "macos")]`), so T5-T8 (generated/bare Window lifetime, native/model-driven floating-Window close) remain BLOCKED pending a real macOS host run.

## WinUI 3 current state

- Window content-host sizing, retained layout, native controls, graphics, text/environment, and the established input/lifecycle paths are implemented. Window-level sizing is the content-host viewport authority for first show and native resize.
- Pointer/capture-loss and coordinate-topology rows remain pending real-mouse verification in [#224](https://github.com/puchinya/elwindui/issues/224). The current verification host cannot deliver the required real OS mouse input reliably.
- Popup teardown, native light-dismiss ordering, close interception, and newer Menu/icon paths are implemented or code-reviewed, but this macOS development environment cannot compile or execute the Windows-only backend; runtime verification remains in [#157](https://github.com/puchinya/elwindui/issues/157).
- SVG offscreen effect-graph work, full cross-backend parity audit, and the remaining native styling/effect gaps are incomplete.
- The Windows App Runtime registration limitation observed in the current sandbox is an environment constraint, not evidence of a product regression; Windows acceptance must run on a normal Windows host.
- Issue #254 implementation is present and code-reviewed: a top-level `Window`'s Rust-side wrapper — including `TreeHost`'s pointer/keyboard/context-menu callback registry — used to drop silently once every caller-side `Rc` went out of scope (e.g. after `#[elwindui::main]`'s startup closure returned), while the native `Microsoft.UI.Xaml.Window` kept rendering and receiving input with nowhere to route it. `crate::app`'s `WINDOWS` registry now retains the final `Rc<dyn WindowExt>` from first `show()` until the native `Closed` event releases it. Dedicated regressions exist for both the bare backend `Window` (`elwindui-backend-winui3::app::window_lifecycle_tests::window_registry_retention_lifecycle`) and the final generated component owner (`crates/elwindui/tests/window_mount_hide_close.rs`'s `WindowLifetimeProbe` case), but execution of both is currently BLOCKED by the build-environment issue directly below — runtime acceptance for #254 is therefore not yet verified on this checkout.
- Build-environment note (unrelated to #254, discovered while verifying it): a clean `cargo build --workspace`/`cargo check --workspace` on this checkout fails inside `elwindui-backend-winui3` — `host/replay.rs` imports `Transform`/`CompositeTransform` and calls `FrameworkElement::SetRenderTransform`, none of which the currently-generated WinUI binding projection exposes on this machine's installed WindowsAppSDK/WinUI3 NuGet packages, and `host/mod.rs` has one `RelayoutHost` trait-scope/type-mismatch pair. These lines are untouched by #254's diff (confirmed by `git diff`); this is most likely a NuGet package version / generated-binding drift specific to this development machine, exposed by the mandatory `cargo clean` on branch switch. Tracked as a separate, pre-existing blocker — not part of #254's scope — until someone reconciles the installed WindowsAppSDK/Win2D package versions with what this code expects.

## Platform boundaries

AppKit native evidence is valid only when run in an authenticated desktop session with the required permissions. WinUI 3 acceptance requires Windows host/build/package-registration semantics. GTK4 and mobile acceptance are not available from this repository state.

## Verification state

The repository baseline has established formatter, workspace build/check/test, and analyzer-policy verification. Backend-specific command boundaries are defined in [`../agents/appkit.md`](../agents/appkit.md) and [`../agents/winui3.md`](../agents/winui3.md); a platform result is not inferred from another backend.
