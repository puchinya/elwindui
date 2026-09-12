# Backend status

Snapshot: 2026-09-10. Durable backend architecture is indexed in [`../design/README.md`](../design/README.md).

## Support matrix

| Backend | Current implementation | Current verification |
|---|---|---|
| AppKit (macOS) | ✅ primary backend | Local builds, workspace tests, screenshots, Accessibility-driven control interaction, and text/environment paths are supported where listed below. |
| WinUI 3 (Windows) | 🚧 substantial implementation | Windows builds and real interaction cover the established startup, Window lifecycle, graphics, text/environment, Button, and selection paths; the full contract and newer pointer/popup/docking rows remain incomplete or deferred. |
| GTK4 (Linux) | ⬜ stub | No functional backend or toolkit dependency. |
| UIKit / Android | ⬜ absent | No implementation. |

## AppKit current state

- Core/native input, screen-coordinate conversion, pointer cancellation, retained rendering, Window lifecycle, transparency, popup/context-menu behavior, and semantic environment/brush paths are implemented.
- `ContextMenu`/`PopupSurface` uses child-window ownership, dynamic visible-frame placement, deferred `ViewFactory` content, popup-scoped environment, and synchronous subtree teardown before ElwindUI host detach. AppKit unit and focused runtime evidence cover the supported paths.
- Native text widgets reduce unsupported gradient/image foreground brushes. TextArea character spacing does not cover every native text-storage path, and PasswordBox intentionally retains the system secure-font cascade.
- SVG filters, blend modes, masks, image-brush fills, and pattern cases use documented simpler fallbacks where Core Image or native drawing cannot represent the requested effect.
- Window hide/close, transparency, and floating/normal levels are implemented; interactive native close-button verification remains host-dependent where a test harness cannot construct an AppKit Window on the main thread.

## WinUI 3 current state

- Window content-host sizing, retained layout, native controls, graphics, text/environment, and the established input/lifecycle paths are implemented. Window-level sizing is the content-host viewport authority for first show and native resize.
- Issue #236's dedicated `TreeHostPanel` input-surface architecture (permanent transparent hit-test `Rectangle`, exact root-Canvas/input-surface source classification, native-child rejection including a real `Button`) is implemented and covered by hosted live-XAML structural tests (`crate::host::live_input_surface_tests`).
- Real-OS-pointer acceptance for Issue #236 (`tests/e2e/self-drawn-pointer-input.md`, scenarios SDP-01..SDP-05) was attempted through the [#242](https://github.com/puchinya/elwindui/issues/242) `windows-ui-driver` and remains **BLOCKED** for the self-drawn scenarios, not FAIL, now root-caused as an independent framework bug, [#254](https://github.com/puchinya/elwindui/issues/254). `input_surface` itself is proven correct on a real host: a real, genuine-movement `drag` over verified-blank self-drawn `Canvas` area in `controls-demo` produced a complete, correctly-accepted `Pressed`/`Moved`/`Released` sequence in an env-gated pointer-routing diagnostic. Earlier hypotheses (an invokable UIA `AutomationPeer` requirement; `windows-ui-driver`/`winapp` coordinate handling) are both retired — `GetCursorPos` confirmed pixel-exact landing, and a code review found no native XAML element in `custom-controls-demo` that could explain an invokability-based distinction. The actual cause: `custom-controls-demo`'s top-level `Window` component is never retained past `#[elwindui::main]`'s startup closure returning (`InnerWindow::show()`'s `retain_window()` retains only the native `Microsoft.UI.Xaml.Window`, not the Rust-side component/`TreeHostPanel`), so its `UiCallbackRegistryOwner` — and every pointer/keyboard/context callback it owns — is dropped at startup, before any input is ever sent, while the native window keeps rendering and receiving real OS input forever with nothing left to deliver it to. `controls-demo` happens to survive only because its ViewModel-bound construction incidentally holds an external reference. Not a defect in Issue #236's `input_surface` fix, which is independently verified correct above. Evidence: `docs/issues/236-treehostpanel-input-surface/evidence/README.md`, `.agent-state/issues/236/e2e/9282b85980e8/` and `1edff32625c6/`.
- Pointer/capture-loss and coordinate-topology rows remain pending real-mouse verification in [#224](https://github.com/puchinya/elwindui/issues/224). The current verification host still cannot deliver required real OS mouse input into a WinUI3 window reliably.
- Popup teardown, native light-dismiss ordering, close interception, and newer Menu/icon paths are implemented or code-reviewed, but this macOS development environment cannot compile or execute the Windows-only backend; runtime verification remains in [#157](https://github.com/puchinya/elwindui/issues/157).
- SVG offscreen effect-graph work, full cross-backend parity audit, and the remaining native styling/effect gaps are incomplete.
- The Windows App Runtime registration limitation observed in the current sandbox is an environment constraint, not evidence of a product regression; Windows acceptance must run on a normal Windows host.

## Platform boundaries

AppKit native evidence is valid only when run in an authenticated desktop session with the required permissions. WinUI 3 acceptance requires Windows host/build/package-registration semantics. GTK4 and mobile acceptance are not available from this repository state.

## Verification state

The repository baseline has established formatter, workspace build/check/test, and analyzer-policy verification. Backend-specific command boundaries are defined in [`../agents/appkit.md`](../agents/appkit.md) and [`../agents/winui3.md`](../agents/winui3.md); a platform result is not inferred from another backend.
