# Design index

`docs/design` は、規範仕様を実現するための永続的な内部architectureを扱う。

| Area | Design |
|---|---|
| UI tree、ownership、lifecycle | [`runtime/ui_tree_design.md`](runtime/ui_tree_design.md) |
| Measure、Arrange、invalidation、scroll constraint | [`runtime/layout_design.md`](runtime/layout_design.md) |
| RenderTree、reconcile、backend replay、cache | [`runtime/rendering_design.md`](runtime/rendering_design.md) |
| Pointer、routed event、keyboard、focus | [`runtime/input_focus_design.md`](runtime/input_focus_design.md) |
| Native widget host、owner mapping、reconciliation | [`runtime/native_control_design.md`](runtime/native_control_design.md) |
| Text cascade、measurement seam | [`runtime/text_design.md`](runtime/text_design.md) |
| Theme / Environment runtime | [`runtime/theme_environment_design.md`](runtime/theme_environment_design.md) |
| ControlTemplate selection、ownership、ContentPresenter | [`runtime/control_template_design.md`](runtime/control_template_design.md) |
| Component state、ViewModel、async | [`runtime/state_management_design.md`](runtime/state_management_design.md) |
| AppKit backend | [`backends/appkit_backend_design.md`](backends/appkit_backend_design.md) |
| WinUI 3 backend | [`backends/winui3_backend_design.md`](backends/winui3_backend_design.md) |
| DSL parser / codegen | [`tools/codegen_design.md`](tools/codegen_design.md) |
| `#[class]` macro internals | [`tools/class_macro_design.md`](tools/class_macro_design.md) |
| Language server | [`tools/languageserver_design.md`](tools/languageserver_design.md) |
| Preview | [`tools/preview_design.md`](tools/preview_design.md) |
| Hot reload | [`tools/hotreload_design.md`](tools/hotreload_design.md) |

各designは関連specへリンクし、公開contractを再定義しない。実装状態は [`../status/README.md`](../status/README.md) に置く。
