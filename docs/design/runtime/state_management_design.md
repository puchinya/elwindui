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

`#[async_computed]` fields re-run their expression through `spawn_local` whenever a dependency changes. A per-field generation counter, bumped synchronously before each spawn, supersedes a stale in-flight recompute: a completion that no longer matches the current generation is discarded without notifying observers. This is not true cancellation — the previous future still runs to completion — and it does not yet extend to component/ViewModel teardown, since general unmount-triggered cleanup is not wired into the runtime; a live `Rc` keeps a recompute pinned for its duration regardless.

`#[elwindui::main]` installs a background async runtime automatically: its generated `fn main()` calls `elwindui::core::task::install_background_runtime()` immediately after `elwindui::init()` and before `elwindui::application::run(...)`, holding the returned `tokio::runtime::Runtime` alive in a local binding for the remainder of `main` (which does not return until `application::run` does, at process exit). `elwindui::core::task::spawn_background` hands a `'static + Send` future to that runtime's thread pool and returns a `tokio::task::JoinHandle`; an `#[async_computed]` expression `.await`s that handle to cross onto real off-thread I/O, then resumes on `spawn_local`'s UI-affine executor exactly as any other awaited future does. `spawn_local`/`LocalExecutor` themselves are unchanged and never drive I/O directly — only the UI-affine resumption. `crates/elwindui-core/tests/spawn_local_cross_thread_wake.rs` verifies this cross-thread wake path against a genuinely suspending future (not one that resolves on its first poll, as every prior async-action example did).

Running a multi-thread `tokio` runtime unconditionally means every ElwindUI application pays its worker-thread startup cost, even one with no `#[async_computed]` fields at all. This is an accepted 0.1.0 trade-off in favor of `#[async_computed]` working without any manual runtime setup; a conditional/configurable runtime is left for a future revision.

## Stores

A `store` shares the same field vocabulary as a `viewmodel` (`#[observable]`, `#[computed]`, `#[async_computed]`, and `impl`-detected actions) and rides the same notification model — it must not bypass property change dispatch or make backend-specific state observable through common APIs.

Unlike a `viewmodel`, a `store` is not held by any single component: it is a process-wide singleton, lazily constructed on first access and installed through the same `EnvironmentContext` mechanism `#[elwindui::theme]` already uses. `TypeName::instance()` returns the shared `Rc<TypeName>` from ordinary Rust code — including from another store's own field expressions, with no additional DSL wiring. A `view!`'s bare, type-qualified `TypeName.field` read path (documented in `docs/specs/dsl_spec.md` §3) and its auto-subscription codegen are not implemented yet — see `docs/status/implementation_status.md`.
