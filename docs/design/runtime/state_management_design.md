# State management design

Related specification: [`../../specs/dsl_spec.md`](../../specs/dsl_spec.md).

## Component state

The component macro separates construction parameters, mutable properties, private state, computed values, and bindable ViewModel references. Generated dependency tracking refreshes only expressions and dynamic regions that depend on a changed field.

State mutation is confined to the UI thread unless a public type explicitly provides synchronization. Generated notifications schedule UI synchronization instead of calling backend code from arbitrary threads.

## ViewModel binding

`#[bindable]` fields hold ViewModel identity and subscribe to property change notifications. OneWay and TwoWay bindings share generated read paths; TwoWay additionally installs the target-to-source event path defined by the DSL specification.

Replacing a ViewModel detaches old subscriptions before attaching the new one. Dynamic `if` / `match` / `for` regions reconcile from the latest source snapshot and preserve stable item identity where the DSL contract supplies it.

## Async work

`spawn_local` is the runtime seam for UI-affine futures. Backend application loops install the task executor and wake it on the UI thread. Completion updates component/ViewModel state through the ordinary notification path.

Long-running or blocking work must execute outside the UI thread and return a result to this seam. Cancellation belongs to the owner lifecycle; unmounted components must not retain callbacks solely through an outstanding task.

## Stores and history

Global stores and undo/redo remain optional layers above the same notification model. They must not bypass property change dispatch or make backend-specific state observable through common APIs.
