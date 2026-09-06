# ElwindUI implementation status

Snapshot: 2026-09-06. Desired behavior is defined by [`../specs/README.md`](../specs/README.md); durable architecture is defined by [`../design/README.md`](../design/README.md).

Legend: ✅ implemented, 🚧 partial, ⬜ not implemented.

## Current capabilities

| Area | State | Current capability / gap |
|---|---|---|
| Component frontend and `view!` DSL | ✅ | Proc-macro parsing, metadata-driven scalar/collection `#[content]` validation and lowering, named `elwindui::new!` construction, and code generation are implemented and covered by workspace tests. |
| ControlTemplate | 🚧 | Explicit-target and component-default `template_view!`, typed parent/environment capabilities, first-application selection, shared ordinary/template planning, deferred views, bindings, dynamic regions, ownership, cleanup, and `ContentPresenter` logical/visual separation are implemented. Runtime re-template, per-instance templates, `TemplatePart`, and `VisualState` remain out of scope ([#83](https://github.com/puchinya/elwindui/issues/83)). |
| Component properties and bindings | ✅ | `param`, `prop`, `state`, `computed`, `bindable`, `Once`, `OneWay`, and `TwoWay` generated storage, notification, dependency refresh, and lifecycle paths are implemented and exercised by examples/tests. |
| Dynamic regions and lifecycle | ✅ | `if`, `match`, and `for` reconciliation, stable `Rc` item identity where supported, child-first unmount, subscription cleanup, and environment-listener cleanup are implemented. |
| Store and async computed | 🚧 | Store singletons, async-computed loading/ready/failed state, supersede semantics, and the background runtime are implemented. Bare `TypeName.field` store references inside `view!` and the related validation rules remain unimplemented ([#82](https://github.com/puchinya/elwindui/issues/82)). |
| Context menu and popup surface | 🚧 | Native/custom context menus, deferred `ViewFactory` popup content, popup-scoped environment, light dismiss, and teardown ordering are implemented. AppKit is verified; WinUI 3 runtime verification remains in the platform backlog ([#157](https://github.com/puchinya/elwindui/issues/157)). |
| Theme and environment | ✅ | `EnvironmentKey`, mount-time environment resolution, `EnvironmentScope`, cross-crate keys, semantic brushes, live resynchronization, and `PlatformDefault` clearing are implemented. Automatic native-control default appearance remains intentionally outside this surface. |
| Graphics and retained rendering | 🚧 | Colors, brushes, gradients, paths, raster images, retained tree reconciliation, and both primary backend rendering paths are implemented. SVG effects use documented fallbacks, and canvas/image snapshot assertions remain unimplemented. |
| Samples | ✅ | `control-template-demo`, `controls-demo`, `font-demo`, `graphics-demo`, `inheritance-demo`, `mascot-demo`, `notepad`, `theme-demo`, and `viewmodel-attr-demo` exercise the implemented public surface. Samples are supplementary evidence, not normative contracts. |

## External generated-component DSL

Qualified external components in `view!` and named `elwindui::new!` construction are implemented across ordinary, template, dynamic, event, two-way, semantic-brush, and resync lowering. The downstream fixture covers external properties/content, inherited scalar content, reactive resync, nested paths, Cargo aliases, constructor defaults, required inputs, and `Option` Props.

The inherited generated `Vec<Rc<T>>` content-forwarding boundary remains follow-up Issue [#194](https://github.com/puchinya/elwindui/issues/194). Generated-component shape identity is currently basename-based within a crate; distinct same-basename components in different modules remain tracked by [#196](https://github.com/puchinya/elwindui/issues/196). Unqualified imported shorthand and a defining-crate `pub mod ui` facade are not promised.

## Current gaps and blockers

- GTK4 is a stub; mobile backends have no implementation.
- Accessibility scaffolds and the NavigationHost/VirtualList/ErrorBoundary public contract require a decision ([#85](https://github.com/puchinya/elwindui/issues/85)).
- Clipboard, drag/drop, notifications, preview tooling, hot reload, and language-server cross-file resolution are incomplete.
- Native-control support and the complete backend contract require platform-specific verification as described in [`backend_status.md`](backend_status.md).

## Verification state

The current repository baseline reports passing formatter, workspace check/build/test, and Rust analyzer-policy gates with only intentional conditional-compilation diagnostics. AppKit focused runtime evidence exists for the covered demos and controls; WinUI 3 and GTK4 runtime verification is environment/platform dependent and is not inferred from macOS results. Command authority remains [`../agents/testing.md`](../agents/testing.md).
