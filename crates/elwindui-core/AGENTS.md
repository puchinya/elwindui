# Crate Guidelines: elwindui-core

When modifying this crate:

- Read [`docs/agents/common.md`](../../docs/agents/common.md).
- Read [`docs/agents/class-model.md`](../../docs/agents/class-model.md) when working with `UIElement` class hierarchy or macro-generated classes.
- Use [`docs/design/README.md`](../../docs/design/README.md) to select the relevant runtime design.
- Keep pure logic (geometry, layout math, tree exploration) in this crate. Do not leak backend-specific types here.
