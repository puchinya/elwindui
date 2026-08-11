# Control implementation status

Snapshot: 2026-08-11. Public behavior is defined by [`../specs/ui_spec.md`](../specs/ui_spec.md).

| Control | AppKit | WinUI 3 | GTK4 | Notes |
|---|---|---|---|---|
| Window | ✅ | ✅ | ⬜ | startup/window close verified on primary backends |
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

## Current gaps

- `ControlTemplate<Self>` does not have a complete runtime replacement pipeline.
- `tooltip` is implemented for NativeControl descendants, not backend-neutral self-rendered elements.
- Native control support has no GTK4 implementation.
- Additional planned controls remain backlog items until their public contract and design are approved.

## Verification

`examples/controls-demo` covers TextBox, PasswordBox, ScrollView, Button, selection controls, Dropdown, Slider, and existing TextArea/Button regressions. AppKit uses `tools/macos-ui-driver`; WinUI 3 verification uses Windows UI Automation and real input.
