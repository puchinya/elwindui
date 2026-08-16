# ElwindUI implementation status

Snapshot: 2026-08-15. Desired behavior is defined by [`../specs/README.md`](../specs/README.md).

Legend: ✅ implemented/verified, 🚧 partial, ⬜ not implemented.

## Core and language

| Area | State | Current evidence / gap |
|---|---|---|
| Component frontend and `view!` DSL | ✅ | proc-macro frontend, parser, validation, and code generation are in workspace tests |
| ControlTemplate | ✅ | `#[component(template = key)]` mount-time selection, `#[control_template]`, typed/reactive `templated_parent`, backend-neutral template-root ownership, and `ContentPresenter`/ContentControl logical-Visual separation are implemented and tested ([#83](https://github.com/puchinya/elwindui/issues/83)); runtime re-template/per-instance template/TemplatePart/VisualState remain out of scope |
| `param` / `prop` / `state` / `computed` / `bindable` | ✅ | generated storage, notifications, and dependency refresh are used by examples |
| Once / OneWay / TwoWay binding | ✅ | component and ViewModel examples exercise generated paths |
| `if` / `match` dynamic regions | ✅ | conditional region replacement is implemented |
| `for` dynamic regions | ✅ | supported `Rc` items retain stable identity; other supported collections rebuild children, and nested dynamic regions are generated recursively |
| Static diagnostics | 🚧 | main DSL validations exist; cross-file resolution and some planned rules are absent |
| Class hierarchy macro | ✅ | ordinary/root/trait-only/struct-only forms are used throughout core/backends |
| UI tree, layout, routed events, focus | ✅ | core tests and AppKit runtime examples cover the implemented model |
| Window lifecycle and presentation | ✅ | mount/show/hide/close behavior remains implemented; `transparent` and `always_on_top` default false and map to AppKit `NSWindow` plus WinUI 3 root/presenter state ([#80](https://github.com/puchinya/elwindui/issues/80), [#125](https://github.com/puchinya/elwindui/issues/125), [#150](https://github.com/puchinya/elwindui/issues/150)) |
| Component lifecycle hooks | ✅ | `new()`/`mount(environment)`/build are separate generated phases (`docs/design/runtime/component_lifecycle_design.md`); `on_mount` fires after the component's own view builds and its children have mounted; `on_update(field, ...)` dispatches through the existing property-changed subscription machinery, excluding the initial construction-time value-set; `on_unmount` is wired for `Window`-rooted components' own `close()` (top-level subscriptions/native release only — no recursive descendant-Component unmount cascade yet) ([#80](https://github.com/puchinya/elwindui/issues/80), done) |
| ReactiveGraph fallback API | ✅ | the unreachable `SignalId`/`ReactiveGraph` stub (no constructor, no callers anywhere in the workspace) was removed from `crates/elwindui-core/src/reactive.rs`; dependency tracking and change notification for `#[computed]`/`#[async_computed]`/`#[bindable]` fields continue to be handled entirely by `elwindui-codegen`'s static analysis plus `Subscription`/`ObservableExt` in the same file, which are unaffected ([#81](https://github.com/puchinya/elwindui/issues/81), done) |
| Global store / async computed | 🚧 | `#[elwindui::store] mod Name { .. }` (`Item::Store`/`StoreDef`, same field vocabulary as `viewmodel`), the process-wide `EnvironmentContext`-backed singleton (`Name::instance()`), and `#[async_computed(expr = ..)]` on both `viewmodel` and `store` (generation-counter "supersede, not cancel", `AsyncComputed<T>` `Loading`/`Ready`/`Failed` getter) are implemented and tested end-to-end, including a real cross-thread supersede (`crates/elwindui/tests/store_and_async_computed.rs`); `#[elwindui::main]` auto-installs a background `tokio` runtime (`elwindui_core::task::install_background_runtime`/`spawn_background`, verified against a genuinely suspending future by `crates/elwindui-core/tests/spawn_local_cross_thread_wake.rs`); validation rule 20 (`#[async_computed]` viewmodel/store-only) is implemented. **Not yet implemented**: the `view!`-side bare `TypeName.field` store-reference syntax and its auto-subscription codegen (dsl_spec.md §3), and validation rules 12 (store field-reference resolution, which checks exactly that unimplemented syntax) and 13 (`#[param]` isolation from store/viewmodel fields) — a store's fields can be read today from ordinary Rust code (`Name::instance().field()`, e.g. inside an action or `on_click`), just not yet bare-referenced directly inside a `view! { .. }` attribute expression. `#[undoable]`/undo-redo has been removed from the contract entirely (not deferred — ElwindUI does not support declarative undo/redo, matching SwiftUI's `UndoManager`-bridge approach); dsl_spec.md §13 rule 21 is retired (`(欠番)`) ([#82](https://github.com/puchinya/elwindui/issues/82)) |
| Text style cascade | ✅ | AppKit and WinUI 3 adapters exist; GTK4 is absent |
| Context menu and PopupSurface | 🚧 | platform-neutral `ContextRequest`, native NSMenu/MenuFlyout context menu, custom `ContextMenuPresenter`, and generic `PopupSurface` with AutoFlip placement and light dismiss are implemented; AppKit runtime verified, WinUI 3 backend implementation in progress with runtime verification separated into [#157](https://github.com/puchinya/elwindui/issues/157) ([#152](https://github.com/puchinya/elwindui/issues/152)) |
| Theme / Environment | 🚧 | `EnvironmentKey`/`EnvironmentContext`/`#[environment(name)]` are implemented and tested (`crates/elwindui-core/src/environment.rs`, `crates/elwindui/tests/environment_field.rs`); `#[environment(name)]` fields resolve at mount-time (from the `EnvironmentContext` a component's generated `mount()` was called with), not construction-time — the earlier ambient thread-local `EnvironmentContext::current()`/`.enter()` propagation mechanism has been removed from the codebase entirely and superseded by explicit mount-time resolution (`docs/design/runtime/theme_environment_design.md`, [#80](https://github.com/puchinya/elwindui/issues/80), done); Theme is a thin Preset-over-Environment (`crates/elwindui-core/src/theme.rs`'s `Theme` trait, `#[elwindui::theme]`) — application-level, applied via `application_environment()` ([#96](https://github.com/puchinya/elwindui/issues/96), done); the `EnvironmentScope` DSL construct is now implemented (`EnvironmentScope { key: value, ..; <children> }` derives an overridden `EnvironmentContext` and mounts its children against it, producing no `UIElement`/Visual/Render/Layout node of its own; `if`/`match` directly inside a scope are scope-aware, `for` is not yet) ([#100](https://github.com/puchinya/elwindui/issues/100), done) — a DSL author can now express per-Window/subtree Environment (and therefore Theme-value) override by wrapping content in `EnvironmentScope` inside `Window { .. }`; Semantic Style is implemented: `BrushStyle`/`ResolvedValue`, 11組み込みsemantic Brush Key、alias/cycle-safe resolver、および`foreground`/`background`/`fill`/`stroke`のeffective-Environment解決・live再同期・`PlatformDefault` clearを提供し、Theme/EnvironmentScopeを含むend-to-end testは`crates/elwindui/tests/semantic_brush_style.rs`にある ([#97](https://github.com/puchinya/elwindui/issues/97), done); automatic native-control default-appearance stylingは引き続き提供せず、Native Style ([#98](https://github.com/puchinya/elwindui/issues/98)) が残る; `#[environment(name)]`/`EnvironmentScope` can now also resolve an Environment Key declared in a *different* crate via a completely-qualified crate path (`#[environment(some_crate::locale)]`, `EnvironmentScope { some_crate::locale: value }`) — `#[elwindui::environment_key]` additionally exports a `__elwindui_environment_key_{name}!` cross-crate macro (`docs/design/tools/environment_key_macro_design.md`), tested end to end by `crates/elwindui-environment-key-fixture` + `crates/elwindui/tests/environment_field_cross_crate.rs`/`environment_scope_cross_crate.rs` ([#129](https://github.com/puchinya/elwindui/issues/129), done) |
| File dialog | ✅ | current platform service implementation exists |
| Clipboard, notifications, drag and drop | ⬜ | no complete cross-platform implementation |

## Graphics

| Area | State | Current evidence / gap |
|---|---|---|
| Color, brushes, gradients, stroke, path | ✅ | core values and AppKit/WinUI rendering are present |
| Raster image | ✅ | loading/value model and both primary backend paths are present |
| Vector image / SVG | 🚧 | parser, limits, rendering, and many effects are present; both primary backends have documented filter/blend/mask fallbacks |
| Retained rendering and reconciliation | ✅ | active tree groups and native child reconciliation are implemented |
| Snapshot testing | 🚧 | tree dump exists; canvas/image snapshot assertion remains unimplemented |

## Samples

`control-template-demo`, `controls-demo`, `font-demo`, `graphics-demo`, `inheritance-demo`, `mascot-demo`, `notepad`, `theme-demo`, and `viewmodel-attr-demo` exercise the implemented public surface. A sample demonstrates usage but is not itself normative evidence.

## Primary gaps

- GTK4 is a stub and mobile backends have no implementation.
- ControlTemplateのruntime re-template/per-instance template/TemplatePart/VisualStateは未実装である ([#83](https://github.com/puchinya/elwindui/issues/83)); accessibility scaffolds and the NavigationHost/VirtualList/ErrorBoundary contract require a decision ([#85](https://github.com/puchinya/elwindui/issues/85)).
- Clipboard, drag/drop, and notifications are not complete.
- Language-server cross-file resolution, preview tooling, and hot reload are incomplete.
- Whole-workspace rust-analyzer diagnostics are not currently a clean gate because of pre-existing generated-code diagnostics.
