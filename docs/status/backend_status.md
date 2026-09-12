# Backend status

Snapshot: 2026-09-12. Durable backend architecture is indexed in [`../design/README.md`](../design/README.md).

## Support matrix

| Backend | Current implementation | Current verification |
|---|---|---|
| AppKit (macOS) | ✅ primary backend | Local builds, workspace tests, screenshots, synthetic Core-backed AX projection, and text/environment paths are supported where listed below. |
| WinUI 3 (Windows) | 🚧 substantial implementation | Core snapshot and the XAML AutomationPeer bridge skeleton are implemented in source; UIA pattern providers and real Windows build/runtime verification remain incomplete. |
| GTK4 (Linux) | ⬜ stub | No functional backend or toolkit dependency. |
| UIKit / Android | ⬜ absent | No implementation. |

## AppKit current state

- Core/native input, screen-coordinate conversion, pointer cancellation, retained rendering, Window lifecycle, transparency, popup/context-menu behavior, and semantic environment/brush paths are implemented.
- Animation/transition projection uses the per-host Core runtime and stable AppKit native islands. Two clean-launch AppKit animation-demo E2E runs pass width interpolation, focused NativeControl exit visibility with recursive `AXTextField` suppression, completion removal, and Reduce Motion checks.
- `ContextMenu`/`PopupSurface` uses child-window ownership, dynamic visible-frame placement, deferred `ViewFactory` content, popup-scoped environment, and synchronous subtree teardown before ElwindUI host detach. AppKit unit and focused runtime evidence cover the supported paths.
- Native text widgets reduce unsupported gradient/image foreground brushes. TextArea character spacing does not cover every native text-storage path, and PasswordBox intentionally retains the system secure-font cascade.
- SVG filters, blend modes, masks, image-brush fills, and pattern cases use documented simpler fallbacks where Core Image or native drawing cannot represent the requested effect.
- Window hide/close, transparency, and floating/normal levels are implemented; interactive native close-button verification remains host-dependent where a test harness cannot construct an AppKit Window on the main thread.
- Core-owned synthetic AX semantics are projected from `AccessibilityRuntime`; cached elements are keyed by `AccessibilityId`, disabled state follows Core intrinsic semantics, Automatic template traversal suppresses private chrome, inactive hosted trees expose no AX children, and `NativeIslandView` remains a rendering/input containment boundary with no public AX children. The shared accessibility scenario is defined under [`../../tests/e2e/accessibility-semantics/scenario.md`](../../tests/e2e/accessibility-semantics/scenario.md). The checked-in AppKit driver completed two clean-launch runs, including direct AX text/numeric value setting, semantic state/value updates, exit removal, stale-action handling, duplicate suppression, and cleanup.

## WinUI 3 current state

- Window content-host sizing, retained layout, native controls, graphics, text/environment, and the established input/lifecycle paths are implemented. Window-level sizing is the content-host viewport authority for first show and native resize.
- Pointer/capture-loss and coordinate-topology rows remain pending real-mouse verification in [#224](https://github.com/puchinya/elwindui/issues/224). The current verification host cannot deliver the required real OS mouse input reliably.
- Popup teardown, native light-dismiss ordering, close interception, and newer Menu/icon paths are implemented or code-reviewed, but this macOS development environment cannot compile or execute the Windows-only backend; runtime verification remains in [#157](https://github.com/puchinya/elwindui/issues/157).
- SVG offscreen effect-graph work, full cross-backend parity audit, and the remaining native styling/effect gaps are incomplete.
- The Windows App Runtime registration limitation observed in the current sandbox is an environment constraint, not evidence of a product regression; Windows acceptance must run on a normal Windows host.
- The Core snapshot, stable-ID callback ABI, and Canvas-compatible AutomationPeer source are present. The current bridge exposes the semantic tree and basic properties, but UIA pattern providers are not implemented. Real Windows bridge build, pattern/action checks, duplicate suppression, and the shared scenario are delegated to [#256](https://github.com/puchinya/elwindui/issues/256) and remain unverified here.

## Platform boundaries

AppKit native evidence is valid only when run in an authenticated desktop session with the required permissions. WinUI 3 acceptance requires Windows host/build/package-registration semantics. GTK4 and mobile acceptance are not available from this repository state.

## Verification state

The repository baseline has established formatter, workspace build/check/test, and analyzer-policy verification. Backend-specific command boundaries are defined in [`../agents/appkit.md`](../agents/appkit.md) and [`../agents/winui3.md`](../agents/winui3.md); a platform result is not inferred from another backend.
