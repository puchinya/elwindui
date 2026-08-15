# Control implementation status

Snapshot: 2026-08-15. Public behavior is defined by [`../specs/ui_spec.md`](../specs/ui_spec.md).

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
| Slider | ✅ | ✅ | ⬜ | value/range and explicit-width behavior verified |
| MenuBar / Menu / MenuItem | ✅ | ✅ | ⬜ | context-menu attachment remains absent |
| TabView / TabViewItem | ✅ | ✅ | ⬜ | hosted page activation and native child reconciliation |
| Rectangle / Ellipse / Image | ✅ | ✅ | ⬜ | backend-neutral self-rendered controls |
| ControlTemplate / ContentPresenter | ✅ | ✅ | ⬜ | mount-time typed Environment selection and ContentControl logical/Visual separation; backend-neutral runtime |

## Current gaps

- Runtime re-template、per-instance template property、TemplatePart、VisualStateは初期`ControlTemplate`の対象外である ([#83](https://github.com/puchinya/elwindui/issues/83))。
- `tooltip` is implemented for NativeControl descendants, not backend-neutral self-rendered elements.
- Native control support has no GTK4 implementation.
- Accessibility scaffolds and the NavigationHost/VirtualList/ErrorBoundary surface require an explicit public-contract decision ([#85](https://github.com/puchinya/elwindui/issues/85)).
- Additional planned controls remain backlog items until their public contract and design are approved.

## Verification

`examples/controls-demo` covers TextBox, PasswordBox, ScrollView, Button, selection controls, Dropdown, Slider, and existing TextArea/Button regressions. `examples/control-template-demo` covers typed Environment override, capturing factory, reactive `templated_parent`, and `ContentPresenter`. `examples/mascot-demo` covers a draggable transparent always-on-top Window with a real alpha PNG. AppKit uses `tools/macos-ui-driver`; WinUI 3 verification uses Windows UI Automation and real input.
