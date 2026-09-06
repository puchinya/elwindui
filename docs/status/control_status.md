# Control implementation status

Snapshot: 2026-09-06. Public behavior is defined by [`../specs/ui_spec.md`](../specs/ui_spec.md).

## Current support matrix

| Control | AppKit | WinUI 3 | GTK4 | Current state |
|---|---|---|---|---|
| Window | ✅ | ✅ | ⬜ | Lifecycle, transparent client surface, and normal/topmost ordering are implemented. |
| TextBlock / TextArea / TextBox / PasswordBox | ✅ | ✅ | ⬜ | Native text and secure-entry paths are implemented; PasswordBox preserves the AppKit system font cascade. |
| ScrollView | ✅ | ✅ | ⬜ | Native viewport with ElwindUI content host. |
| Button / CheckBox / RadioButton / ToggleSwitch | ✅ | ✅ | ⬜ | Roles, tri-state/group behavior, TwoWay state, and focus/default paths are implemented. |
| Dropdown / DropdownItem | ✅ | ✅ | ⬜ | Selection and dynamic item rebuilding are implemented. |
| MenuBar / Menu / MenuItem | ✅ | 🚧 | ⬜ | Native/custom menus and icon sources are implemented; WinUI 3 runtime verification remains [#157](https://github.com/puchinya/elwindui/issues/157). |
| PopupSurface / `context_popup` | ✅ | 🚧 | ⬜ | Auto-flip placement, light dismiss, deferred content, environment propagation, and teardown ordering are implemented; Windows runtime remains pending. |
| TabView / TabViewItem | ✅ | ✅ | ⬜ | Hosted page activation, native child reconciliation, and AppKit chrome are implemented. |
| Rectangle / Ellipse / Image | ✅ | ✅ | ⬜ | Backend-neutral self-rendered controls. |
| IconElement / IconSourceElement | ✅ | ✅ | ⬜ | Backend-neutral icon values and rendering paths. |
| CustomTabView / CustomTabViewItem / CustomSplitter | 🚧 | 🚧 | ⬜ | Templated custom controls, retained content, selection, pointer gestures, and splitter semantics are implemented; platform runtime interaction remains incomplete. |
| ControlTemplate / ContentPresenter | ✅ | ✅ | ⬜ | Typed first-application selection and logical/visual separation are implemented; runtime re-template and related advanced features remain out of scope ([#83](https://github.com/puchinya/elwindui/issues/83)). |

## Current gaps

- Runtime re-template, per-instance template properties, `TemplatePart`, and `VisualState` are not implemented.
- `tooltip` is implemented for NativeControl descendants, not backend-neutral self-rendered elements.
- Native control support has no GTK4 implementation.
- Accessibility scaffolds and the NavigationHost/VirtualList/ErrorBoundary surface require an explicit public-contract decision ([#85](https://github.com/puchinya/elwindui/issues/85)).
- Additional controls remain backlog items until their public contract and design are approved.

## Verification state

The controls demo and control-template demo exercise the supported AppKit paths, while Core and codegen tests cover the backend-neutral semantics and RenderTree descendants. WinUI 3 verification uses Windows UI Automation and real input; GTK4 is unavailable. Verification commands remain authoritative in [`../agents/testing.md`](../agents/testing.md).
