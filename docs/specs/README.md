# Specification index

`docs/specs` は、実装方法を変更しても守る必要がある規範的な公開contractの正本である。

| Question | Specification |
|---|---|
| DSL syntax、component、binding、control flow、diagnostics | [`dsl_spec.md`](dsl_spec.md) |
| UIElement、layout、controls、events、focus | [`ui_spec.md`](ui_spec.md) |
| Font、text style、inheritance、reset、fallback | [`text_style_spec.md`](text_style_spec.md) |
| Color、Brush、Path、Image、VectorImage | [`graphics_spec.md`](graphics_spec.md) |
| Theme、Environment、token、appearance | [`theme_environment_spec.md`](theme_environment_spec.md) |
| ControlTemplate selection、authoring、content presentation | [`control_template_spec.md`](control_template_spec.md) |
| File dialog等のOS service | [`platform_spec.md`](platform_spec.md) |
| `#[class]` の公開contract | [`macro_class_spec.md`](macro_class_spec.md) |

実装状況は [`../status/README.md`](../status/README.md)、内部方式は [`../design/README.md`](../design/README.md) を参照する。
