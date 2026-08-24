# Control implementation status

Snapshot: 2026-08-24. Public behavior is defined by [`../specs/ui_spec.md`](../specs/ui_spec.md).

| Control | AppKit | WinUI 3 | GTK4 | Notes |
|---|---|---|---|---|
| Window | ✅ | ✅ | ⬜ | lifecycle plus transparent client surface and normal/topmost Z-order; Issue #150 AppKit runtime verified, WinUI 3 additions not recompiled on macOS |
| TextBlock | ✅ | ✅ | ⬜ | backend text measurement and rendering |
| TextArea | ✅ | ✅ | ⬜ | native editable multiline control |
| TextBox | ✅ | ✅ | ⬜ | value, placeholder, submit/focus paths |
| PasswordBox | ✅ | ✅ | ⬜ | secure entry; AppKit uses system secure font cascade |
| ScrollView | ✅ | ✅ | ⬜ | native viewport plus ElwindUI content host |
| Button | ✅ | ✅ | ⬜ | role/default/tooltip verified |
| CheckBox | ✅ | ✅ | ⬜ | tri-state behavior verified |
| RadioButton | ✅ | ✅ | ⬜ | group behavior verified |
| ToggleSwitch | ✅ | ✅ | ⬜ | TwoWay state verified |
| Dropdown / DropdownItem | ✅ | ✅ | ⬜ | selection and dynamic item rebuild verified |
| MenuBar / Menu / MenuItem | ✅ | 🚧 | ⬜ | menu bar and native/custom context menu attached via `context_menu` / `context_menu_presentation`; AppKit verified, WinUI 3 runtime verification in [#157](https://github.com/puchinya/elwindui/issues/157) ([#152](https://github.com/puchinya/elwindui/issues/152)). `MenuItem.icon` (`IconSource`/`SystemIcon`) added for both Native and Custom presentation ([#170](https://github.com/puchinya/elwindui/issues/170)). AppKit: SF Symbol system icons, user raster **and vector** icons, and the Custom canonical vector fallback are all screenshot-verified on `controls-demo`. WinUI 3: every `ImageSource` case (`SymbolIcon`, `Encoded` fast path, `Rgba8`/Win2D-`CanvasBitmap`-backed `Backend` via the existing `win2d_bitmap` conversion, and `Vector` via a `CanvasRenderTarget` rasterize-to-PNG bridge) is implemented, and regression test code has been added for each case, but — same #157 macOS-only-environment gap as the rest of Menu — none of it has actually been compiled, let alone executed or runtime-verified, on Windows; build/test execution/runtime verification for the icon paths is tracked in [#157](https://github.com/puchinya/elwindui/issues/157) alongside the rest of Menu's Windows verification |
| PopupSurface / context_popup | ✅ | 🚧 | ⬜ | arbitrary UIElement popup surface with auto-flip placement, light dismiss, and above-native-control elevation; `ViewTemplate`-based deferred build (owner captured `Weak`, popup-scoped derived Environment, declarative `#[environment(popup_dismiss)]`-resolvable `PopupDismissAction`) and `unmount_subtree` teardown-before-detach on close, both backends ([#161](https://github.com/puchinya/elwindui/issues/161)) — portable guarantee is unmount before ElwindUI's own host-tree detach on every path; framework-initiated close additionally unmounts before native visibility/detach on both backends, but WinUI3 native light-dismiss (`Popup.Closed`, which fires only after WinUI itself sets `IsOpen=false`) is a documented exception to that stronger ordering, not to the portable one; declarative `context_popup: view! { .. }` DSL sugar not yet implemented (low-level `ViewTemplate::new(...)` only) — see [#162](https://github.com/puchinya/elwindui/issues/162); AppKit verified, WinUI 3 runtime verification (including native light-dismiss ordering) in [#157](https://github.com/puchinya/elwindui/issues/157) ([#152](https://github.com/puchinya/elwindui/issues/152)) |
| TabView / TabViewItem | ✅ | ✅ | ⬜ | hosted page activation and native child reconciliation; AppKit tab chrome (layerless chip drawing, system-symbol close/new-tab, closable live sync, shrink-to-fit overflow) screenshot-verified on `controls-demo`/`notepad` ([#167](https://github.com/puchinya/elwindui/issues/167)) — Accessibility-driven interaction verification (`find`/`click`) not run, no Accessibility permission granted to the verification environment |
| Rectangle / Ellipse / Image | ✅ | ✅ | ⬜ | backend-neutral self-rendered controls |
| IconElement / IconSourceElement | ✅ | ✅ | ⬜ | backend-neutral self-rendered icon base/value wrapper; Core unit and cross-crate DSL tests, no backend-specific control path ([#176](https://github.com/puchinya/elwindui/issues/176)) |
| ControlTemplate / ContentPresenter | ✅ | ✅ | ⬜ | mount-time typed Environment selection, implicit ContentControl default body templates, and logical/Visual separation; backend-neutral runtime; authored roots use metadata-driven template-root ownership while caller bare content remains the inherited `content` slot |

## Current gaps

- Runtime re-template、per-instance template property、TemplatePart、VisualStateは初期`ControlTemplate`の対象外である ([#83](https://github.com/puchinya/elwindui/issues/83))。
- ContentControl-derived components use their authored body as the default visual template; a `ContentPresenter` is opt-in in that body, while raw `ContentControl` retains direct presentation.
- `tooltip` is implemented for NativeControl descendants, not backend-neutral self-rendered elements.
- Native control support has no GTK4 implementation.
- Accessibility scaffolds and the NavigationHost/VirtualList/ErrorBoundary surface require an explicit public-contract decision ([#85](https://github.com/puchinya/elwindui/issues/85)).
- Additional planned controls remain backlog items until their public contract and design are approved.

## Verification

`examples/controls-demo` covers TextBox, PasswordBox, ScrollView, Button, selection controls, Dropdown, Slider, existing TextArea/Button regressions, and (Context Menu tab) `MenuItem.icon` — Native `SystemIcon` items, a Native user-vector-icon item, a Custom Context Menu mixing a `SystemIcon` item, a disabled `SystemIcon` item, a user raster `IconSource::Image` item, a user vector `IconSource::Image` item, and an icon-less item to verify leading-column alignment. `examples/control-template-demo` covers typed Environment override, capturing factory, reactive `templated_parent`, and `ContentPresenter`. `examples/mascot-demo` covers a draggable transparent always-on-top Window with a real alpha PNG. AppKit uses `tools/macos-ui-driver`; WinUI 3 verification uses Windows UI Automation and real input.
