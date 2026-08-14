# UI tree and lifecycle design

Related specification: [`../../specs/ui_spec.md`](../../specs/ui_spec.md).

## Responsibilities

`elwindui-core` owns the backend-neutral UI object graph. `UIElement` supplies common state, while generated `*Ext` traits expose polymorphic operations without leaking native handle types into the common API.

The runtime distinguishes:

- logical parent: content ownership and inheritable public values;
- visual parent: layout, rendering, hit testing, Theme context, and backend hosting;
- native owner: mapping from a backend widget to its ElwindUI element.

Helpers inserted by a backend may participate in the visual tree but must not become observable logical parents.

## Ownership

UI nodes are shared handles because a node is referenced by its owner, parent traversal, event subscriptions, and backend host. Parent links are weak or otherwise non-owning so that detaching a subtree releases it without a cycle.

Logical and visual child enumeration remains centralized on the element abstraction. Containers must not maintain a second unsynchronized public child list.

Template-enabled `Control`はlogical childrenとは別に単一のVisual template rootをstrong保持する。
template rootのlogical parentは設定しない。`ContentControl`のlogical contentは常にtargetをlogical parentとし、
`ContentPresenter`が同じcontentを自身のVisual childとして配置する。targetからtemplate rootへのedgeはstrong、
template instanceからtemplated parentへのedgeはWeakでありcycleを作らない。詳細は
[`control_template_design.md`](control_template_design.md)を参照する。

## Lifecycle

Construction establishes the object and local state. Mounting attaches the subtree to a host and enables inherited-context resolution, layout, rendering, input, and native reconciliation. Unmounting removes subscriptions, host resources, focus ownership, and render state before the subtree becomes unreachable.

Component `on_mount`, `on_update`, and `on_unmount` callbacks are invoked by generated component wiring; the tree runtime supplies stable attach/detach boundaries and does not interpret component business state.

How generated code realizes Construction/Mounting/Unmounting — the `new()`/`mount(environment)`/build split, its interaction with `#[class]`'s `construct`/`on_constructed` contract, and Environment's move to mount-time resolution — is specified in [`component_lifecycle_design.md`](component_lifecycle_design.md) (tracking: [#80](https://github.com/puchinya/elwindui/issues/80)).

## Participation

Existence and active participation are separate. A collapsed or inactive hosted subtree may retain UI and native-control state while being excluded from layout, render-tree generation, hit testing, focus order, and shortcut dispatch. Host activation is the boundary used by independently hosted content such as `TabView` pages.

Reactivation starts from layout with the current viewport and reconstructs backend render resources. Participation checks are centralized so the subsystems cannot disagree about whether a subtree is active.
