# Backend status

Snapshot: 2026-08-14. Architecture is indexed in [`../design/README.md`](../design/README.md).

## Support matrix

| Backend | Build/runtime state | Verification state |
|---|---|---|
| AppKit (macOS) | ✅ primary backend | Local builds, core examples, screenshots, Accessibility-driven control interaction, text/Environment paths |
| WinUI 3 (Windows) | 🚧 substantial implementation | Windows builds and real interaction verified for application startup, Window lifecycle, graphics, text/Environment, Button and selection controls; requirement-by-requirement audit remains incomplete |
| GTK4 (Linux) | ⬜ stub | No functional backend or toolkit dependency |
| UIKit / Android | ⬜ absent | No code |

## AppKit current gaps

- Host-level pointer/keyboard dispatch covers self-drawn elements; native leaves receive OS events directly and require control-specific focus/input wiring.
- Gradient/Image foreground brushes are reduced when a native text widget cannot represent the full brush.
- Editable TextArea character spacing does not cover every native text-storage path.
- Secure PasswordBox intentionally retains the system secure font cascade instead of applying ordinary font synthesis.
- SVG filter replay passes through turbulence, diffuse/specular lighting, displacement maps, unsupported convolve kernels, and Table/Discrete/Gamma component-transfer functions. Group blend modes fall back when no Core Image filter is available.
- SVG nested masks with mismatched bounds use the outer mask only; image-brush path fills, pattern strokes, and degenerate/offscreen patterns use documented simpler fallbacks.
- Memory investigations for Issue #60 found no durable evidence that retained ElwindUI render data directly owns the previously observed multi-megabyte process delta. Raw measurements remain Issue evidence rather than status documents.
- Window's new `hide()`/`close()` (Issue #80 CI-8: `NSWindow::orderOut`/`NSWindow::close`) are implemented and covered by the workspace test suite (constructed, type-checked; interactive on-screen confirmation was attempted but inconclusive in this sandboxed development environment — no crash or early-exit was observed running a real example either before or after the change, but a spawned GUI process's window did not appear in a `screencapture` capture, a pre-existing environment characteristic unrelated to this change).

## WinUI 3 current gaps

- SVG group blend modes without direct `CanvasBlend` mappings, isolation, filters, and luminance-mask rasterization need an offscreen effect graph.
- Cross-backend parity has been verified for the controls recorded in [`control_status.md`](control_status.md), but the entire backend contract has not been re-audited.
- Whole-workspace rust-analyzer diagnostics have pre-existing failures and were not established as a clean Windows backend gate.

## Verification baseline

- AppKit: macOS native execution, screenshot, Accessibility interaction, and workspace tests.
- WinUI 3: Windows package/bootstrap builds, real window screenshots/input, automated Window `show()`/`hide()`/re-show/`close()` with `AppWindow.IsVisible` and retained-window bookkeeping assertions, final-window close, graphics-demo tabs, and controls-demo UI Automation.
- Backend-specific environment commands live in [`../agents/appkit.md`](../agents/appkit.md) and [`../agents/winui3.md`](../agents/winui3.md).
