# Crate Guidelines: elwindui-core

When modifying this crate:

- Read [`docs/agents/common.md`](../../docs/agents/common.md).
- Read [`docs/agents/class-model.md`](../../docs/agents/class-model.md) when working with `UIElement` class hierarchy or macro-generated classes.
- Read [`docs/design/gui_framework_design.md`](../../docs/design/gui_framework_design.md) §5 (Core Runtime) when runtime architecture details are needed.
- Keep pure logic (geometry, layout math, tree exploration) in this crate. Do not leak backend-specific types here.
