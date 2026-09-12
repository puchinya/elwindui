# ElwindUI implementation status

Snapshot: 2026-09-12. Desired behavior is defined by [`../specs/README.md`](../specs/README.md); durable architecture is defined by [`../design/README.md`](../design/README.md).

Legend: ✅ implemented, 🚧 partial, ⬜ not implemented.

## Current capabilities

| Area | State | Current capability / gap |
|---|---|---|
| Component frontend and `view!` DSL | ✅ | Proc-macro parsing, metadata-driven scalar/collection `#[content]` validation and lowering, named `elwindui::new!` construction, and code generation are implemented and covered by workspace tests. |
| ControlTemplate | 🚧 | Explicit-target and component-default `template_view!`, typed parent/environment capabilities, first-application selection, shared ordinary/template planning, deferred views, bindings, dynamic regions, ownership, cleanup, and `ContentPresenter` logical/visual separation are implemented. Runtime re-template, per-instance templates, `TemplatePart`, and `VisualState` remain out of scope ([#83](https://github.com/puchinya/elwindui/issues/83)). |
| Component properties and bindings | ✅ | `param`, `prop`, `state`, `computed`, `bindable`, `Once`, `OneWay`, and `TwoWay` generated storage, notification, dependency refresh, and lifecycle paths are implemented and exercised by examples/tests. |
| Dynamic regions and lifecycle | ✅ | `if`, `match`, and `for` reconciliation, stable `Rc` item identity where supported, child-first unmount, subscription cleanup, and environment-listener cleanup are implemented. |
| Animation and transitions | 🚧 | Core target/presentation animation, transactions, springs, transform-aware hit testing, dynamic insertion/removal transitions, AppKit `CVDisplayLink`, and the `animation-demo` harness are implemented. AppKit compilation/runtime evidence is available; WinUI 3 compilation and runtime evidence remains platform-dependent, and GTK4 is unverified. |
| Accessibility | 🚧 | Core-owned semantic snapshots, stable `AccessibilityId`, ordinary UIElement properties/hooks, intrinsic built-in mappings, disabled-state synchronization, template-aware Automatic traversal, inactive-host suppression, AppKit synthetic AX projection, and the shared AppKit E2E baseline are implemented. The WinUI XAML AutomationPeer bridge is partial: its semantic tree/basic properties are present, but UIA pattern providers are not implemented and no Windows host verification is available; those items are delegated to [#256](https://github.com/puchinya/elwindui/issues/256). |
| Store and async computed | 🚧 | Store singletons, async-computed loading/ready/failed state, supersede semantics, and the background runtime are implemented. Bare `TypeName.field` store references inside `view!` and the related validation rules remain unimplemented ([#82](https://github.com/puchinya/elwindui/issues/82)). |
| Context menu and popup surface | 🚧 | Native/custom context menus, deferred `ViewFactory` popup content, popup-scoped environment, light dismiss, and teardown ordering are implemented. AppKit is verified; WinUI 3 runtime verification remains in the platform backlog ([#157](https://github.com/puchinya/elwindui/issues/157)). |
| Theme and environment | ✅ | `EnvironmentKey`, mount-time environment resolution, `EnvironmentScope`, cross-crate keys, semantic brushes, live resynchronization, and `PlatformDefault` clearing are implemented. Automatic native-control default appearance remains intentionally outside this surface. |
| Graphics and retained rendering | 🚧 | Colors, brushes, gradients, paths, raster images, retained tree reconciliation, and both primary backend rendering paths are implemented. SVG effects use documented fallbacks, and canvas/image snapshot assertions remain unimplemented. |
| Samples | ✅ | `animation-demo`, `control-template-demo`, `controls-demo`, `font-demo`, `graphics-demo`, `inheritance-demo`, `mascot-demo`, `notepad`, `theme-demo`, and `viewmodel-attr-demo` exercise the implemented public surface. Samples are supplementary evidence, not normative contracts. |

## External generated-component DSL

Qualified external components in `view!` and named `elwindui::new!` construction are implemented across ordinary, template, dynamic, event, two-way, semantic-brush, and resync lowering. The downstream fixture covers external properties/content, inherited scalar content, reactive resync, nested paths, Cargo aliases, constructor defaults, required inputs, and `Option` Props.

The inherited generated `Vec<Rc<T>>` content-forwarding boundary remains follow-up Issue [#194](https://github.com/puchinya/elwindui/issues/194). Generated-component shape identity is currently basename-based within a crate; distinct same-basename components in different modules remain tracked by [#196](https://github.com/puchinya/elwindui/issues/196). Unqualified imported shorthand and a defining-crate `pub mod ui` facade are not promised.

## Current gaps and blockers

- GTK4 is a stub; mobile backends have no implementation.
- Accessibility is adopted as a Core-owned semantic tree through child implementation Issue [#253](https://github.com/puchinya/elwindui/issues/253). AppKit shared E2E passes twice with the checked-in driver, including direct AX TextArea SetText; WinUI has no real-host verification and still lacks UIA pattern providers, which are owned by follow-up Issue [#256](https://github.com/puchinya/elwindui/issues/256). NavigationHost, VirtualList, and ErrorBoundary remain unresolved under decision Issue [#85](https://github.com/puchinya/elwindui/issues/85).
- Clipboard, drag/drop, notifications, preview tooling, hot reload, and language-server cross-file resolution are incomplete.
- Native-control support and the complete backend contract require platform-specific verification as described in [`backend_status.md`](backend_status.md).

## Verification state

The current repository baseline reports passing formatter, workspace check/build/test, and Rust analyzer-policy gates with only intentional conditional-compilation diagnostics. AppKit focused runtime evidence exists for the covered demos and controls; WinUI 3 and GTK4 runtime verification is environment/platform dependent and is not inferred from macOS results. Command authority remains [`../agents/testing.md`](../agents/testing.md).
