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
| `for` dynamic regions | 🚧 | stable identity is implemented for supported `Rc` items; nested/root dynamic regions retain documented limitations |
| Static diagnostics | 🚧 | main DSL validations exist; cross-file resolution and some planned rules are absent |
| Class hierarchy macro | ✅ | ordinary/root/trait-only/struct-only forms are used throughout core/backends |
| UI tree, layout, routed events, focus | ✅ | core tests and AppKit runtime examples cover the implemented model |
| Component lifecycle hooks | 🚧 | `on_mount` runs during generated construction; `on_unmount` is generated without a tree detach trigger, and `on_update` is not implemented |
| Global store / async computed / undo-redo | ⬜ | DSL declarations and generated runtime integration are not implemented |
| Text style cascade | ✅ | AppKit and WinUI 3 adapters exist; GTK4 is absent |
| Theme / Environment | 🚧 | Theme runtime and backend adapters exist; public Environment surface and some native state mappings remain incomplete |
| File dialog | ✅ | current platform service implementation exists |
| Clipboard, notifications, drag and drop | ⬜ | no complete cross-platform implementation |

## Graphics

| Area | State | Current evidence / gap |
|---|---|---|
| Color, brushes, gradients, stroke, path | ✅ | core values and AppKit/WinUI rendering are present |
| Raster image | ✅ | loading/value model and both primary backend paths are present |
| Vector image / SVG | 🚧 | parser, limits, rendering, and many effects are present; some WinUI blend/filter/mask paths are incomplete |
| Retained rendering and reconciliation | ✅ | active tree groups and native child reconciliation are implemented |
| Snapshot testing | 🚧 | tree dump exists; canvas/image snapshot assertion remains unimplemented |

## Samples

`graphics-demo`, `controls-demo`, `font-demo`, `theme-demo`, `notepad`, and `notepad-inline` exercise the implemented public surface. A sample demonstrates usage but is not itself normative evidence.

## Primary gaps

- GTK4 is a stub and mobile backends have no implementation.
- General control templates, navigation host, list virtualization, error boundary, clipboard, drag/drop, and notifications are not complete.
- Language-server cross-file resolution, preview tooling, and hot reload are incomplete.
- Whole-workspace rust-analyzer diagnostics are not currently a clean gate because of pre-existing generated-code diagnostics.
