# ElwindUI implementation status

Snapshot: 2026-08-11. Desired behavior is defined by [`../specs/README.md`](../specs/README.md).

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
| Component lifecycle hooks | 🚧 | `on_mount` runs during generated construction; `on_unmount` has no detach trigger and `on_update` is absent ([#80](https://github.com/puchinya/elwindui/issues/80)) |
| ReactiveGraph fallback API | ⬜ | public operations still terminate through `todo!()` ([#81](https://github.com/puchinya/elwindui/issues/81)) |
| Global store / async computed / undo-redo | ⬜ | DSL declarations and generated runtime integration are not implemented ([#82](https://github.com/puchinya/elwindui/issues/82)) |
| Text style cascade | ✅ | AppKit and WinUI 3 adapters exist; GTK4 is absent |
| Theme / Environment | 🚧 | `EnvironmentKey`/`EnvironmentContext`/`#[environment(name)]` are implemented and tested (`crates/elwindui-core/src/environment.rs`, `crates/elwindui/tests/environment_field.rs`); Theme is now a thin Preset-over-Environment (`crates/elwindui-core/src/theme.rs`'s `Theme` trait, `#[elwindui::theme]`) — application-level only, applied via `EnvironmentContext::application_environment()` ([#96](https://github.com/puchinya/elwindui/issues/96), done); the old token/variant/`ThemeHandle` model and per-Window Theme override, and automatic native-control default-appearance styling driven by Theme, were removed rather than migrated (native controls always use the platform default appearance now); the `EnvironmentScope` DSL construct is not yet implemented ([#100](https://github.com/puchinya/elwindui/issues/100)), so per-Window/subtree Theme override is not available; Semantic Style ([#97](https://github.com/puchinya/elwindui/issues/97)) and Native Style ([#98](https://github.com/puchinya/elwindui/issues/98)) are the planned path back to Theme-driven native-control customization |
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
