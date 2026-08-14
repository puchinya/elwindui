# ElwindUI implementation status

Snapshot: 2026-08-14. Desired behavior is defined by [`../specs/README.md`](../specs/README.md).

Legend: ✅ implemented/verified, 🚧 partial, ⬜ not implemented.

## Core and language

| Area | State | Current evidence / gap |
|---|---|---|
| Component frontend and `view!` DSL | ✅ | proc-macro frontend, parser, validation, and code generation are in workspace tests |
| `param` / `prop` / `state` / `computed` / `bindable` | ✅ | generated storage, notifications, and dependency refresh are used by examples |
| Once / OneWay / TwoWay binding | ✅ | component and ViewModel examples exercise generated paths |
| `if` / `match` dynamic regions | ✅ | conditional region replacement is implemented |
| `for` dynamic regions | ✅ | supported `Rc` items retain stable identity; other supported collections rebuild children, and nested dynamic regions are generated recursively |
| Static diagnostics | 🚧 | main DSL validations exist; cross-file resolution and some planned rules are absent |
| Class hierarchy macro | ✅ | ordinary/root/trait-only/struct-only forms are used throughout core/backends |
| UI tree, layout, routed events, focus | ✅ | core tests and AppKit runtime examples cover the implemented model |
| Window lifecycle (mount/show/hide/close) | ✅ | a `Window`-rooted component's `new()` no longer builds its content; first `show()` implicitly mounts it into `application_environment()` and builds exactly once; repeated `show()` does not rebuild; `hide()` is visibility-only; `close()` cancels the Window's own top-level Environment subscriptions and releases the native window (no recursive descendant-Component unmount cascade yet) — AppKit and WinUI3 are implemented and tested, including WinUI3 native visibility and `Closed`-event retain-list cleanup ([#80](https://github.com/puchinya/elwindui/issues/80), [#125](https://github.com/puchinya/elwindui/issues/125)) |
| Component lifecycle hooks | ✅ | `new()`/`mount(environment)`/build are separate generated phases (`docs/design/runtime/component_lifecycle_design.md`); `on_mount` fires after the component's own view builds and its children have mounted; `on_update(field, ...)` dispatches through the existing property-changed subscription machinery, excluding the initial construction-time value-set; `on_unmount` is wired for `Window`-rooted components' own `close()` (top-level subscriptions/native release only — no recursive descendant-Component unmount cascade yet) ([#80](https://github.com/puchinya/elwindui/issues/80), done) |
| ReactiveGraph fallback API | ✅ | the unreachable `SignalId`/`ReactiveGraph` stub (no constructor, no callers anywhere in the workspace) was removed from `crates/elwindui-core/src/reactive.rs`; dependency tracking and change notification for `#[computed]`/`#[async_computed]`/`#[bindable]` fields continue to be handled entirely by `elwindui-codegen`'s static analysis plus `Subscription`/`ObservableExt` in the same file, which are unaffected ([#81](https://github.com/puchinya/elwindui/issues/81), done) |
| Global store / async computed / undo-redo | ⬜ | DSL declarations and generated runtime integration are not implemented ([#82](https://github.com/puchinya/elwindui/issues/82)) |
| Text style cascade | ✅ | AppKit and WinUI 3 adapters exist; GTK4 is absent |
| Theme / Environment | 🚧 | `EnvironmentKey`/`EnvironmentContext`/`#[environment(name)]` are implemented and tested (`crates/elwindui-core/src/environment.rs`, `crates/elwindui/tests/environment_field.rs`); `#[environment(name)]` fields resolve at mount-time (from the `EnvironmentContext` a component's generated `mount()` was called with), not construction-time — the earlier ambient thread-local `EnvironmentContext::current()`/`.enter()` propagation mechanism has been removed from the codebase entirely and superseded by explicit mount-time resolution (`docs/design/runtime/theme_environment_design.md`, [#80](https://github.com/puchinya/elwindui/issues/80), done); Theme is a thin Preset-over-Environment (`crates/elwindui-core/src/theme.rs`'s `Theme` trait, `#[elwindui::theme]`) — application-level, applied via `application_environment()` ([#96](https://github.com/puchinya/elwindui/issues/96), done); the `EnvironmentScope` DSL construct is now implemented (`EnvironmentScope { key: value, ..; <children> }` derives an overridden `EnvironmentContext` and mounts its children against it, producing no `UIElement`/Visual/Render/Layout node of its own; `if`/`match` directly inside a scope are scope-aware, `for` is not yet) ([#100](https://github.com/puchinya/elwindui/issues/100), done) — a DSL author can now express per-Window/subtree Environment (and therefore Theme-value) override by wrapping content in `EnvironmentScope` inside `Window { .. }`; automatic native-control default-appearance styling driven by Theme/Environment was removed by #96 and not yet restored (native controls always use the platform default appearance) — Semantic Style ([#97](https://github.com/puchinya/elwindui/issues/97)) and Native Style ([#98](https://github.com/puchinya/elwindui/issues/98)) are the planned path back |
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

`controls-demo`, `font-demo`, `graphics-demo`, `inheritance-demo`, `notepad`, `theme-demo`, and `viewmodel-attr-demo` exercise the implemented public surface. A sample demonstrates usage but is not itself normative evidence.

## Primary gaps

- GTK4 is a stub and mobile backends have no implementation.
- General control templates are not complete ([#83](https://github.com/puchinya/elwindui/issues/83)); accessibility scaffolds and the NavigationHost/VirtualList/ErrorBoundary contract require a decision ([#85](https://github.com/puchinya/elwindui/issues/85)).
- Clipboard, drag/drop, and notifications are not complete.
- Language-server cross-file resolution, preview tooling, and hot reload are incomplete.
- Whole-workspace rust-analyzer diagnostics are not currently a clean gate because of pre-existing generated-code diagnostics.
