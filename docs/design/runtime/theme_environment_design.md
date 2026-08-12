# Theme and Environment implementation design

Related specification: [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md).

Theme and Environment are separate runtime systems (`theme_environment_spec.md` §2, §7, §39 and Issue #84/#96). They share no runtime type; this document keeps their internals in separate sections for that reason. `docs/agents/codegen.md`/`class-model.md` invariants apply to both.

## Theme

### Context

`ThemeContext` is attached at application and Window host boundaries. Elements resolve the nearest context through the visual host relation while logical inheritance remains available to property cascades. A Window override derives from, rather than mutating, the application default.

The context owns the selected definition, variant, appearance preference, resolved appearance, and monotonically increasing revision. Handles allow controls to observe a context without owning the UI tree.

### Resolution

Typed `ThemeToken<T>` lookup first checks the selected concrete token. Missing standard concrete tokens fall back through the declared base-token chain. An explicit `PlatformDefault` terminates lookup.

### Change propagation

Controllers update context state and increment the revision only when an observable value changes. Generated `theme!` bindings record tokens and their `ThemeChangeImpact`, allowing paint, measure, or native-style invalidation to be scheduled narrowly.

Backend appearance observers translate OS changes into `ThemeAppearance` and update only contexts using `System` preference.

### Backend synchronization

Common resolution produces `Value` or `PlatformDefault`. AppKit adapters map them to system fonts/colors/appearance and layer properties. WinUI 3 adapters use dependency-property set/clear operations and `RequestedTheme`. Backend status documents record unsupported mappings; they do not change resolution semantics.

Note (Issue #96): this token/variant model is being replaced by the Preset-over-Environment model described in the specification's Theme sections. This section describes the current implementation and stays authoritative until #96 lands; it is not the target architecture.

## Environment

### Context

`EnvironmentContext` is a small `Clone` (`Rc`-backed) handle holding a set of typed entries, one per `EnvironmentKey`. Each entry is a reactive cell. A context created by `derive()` shares every entry it does not override with its parent by `Rc` identity, and allocates a fresh cell only for the keys the caller overrides — mirroring `ThemeContext`'s "derive, don't mutate" rule for the application/Window relationship, but per-key rather than per-context.

Lookup is by `EnvironmentKey::Value`'s type, resolved through the key's `TypeId`; there is no string-keyed path (`theme_environment_spec.md` §2, `dsl_spec.md` §4/§8).

### Resolution and component integration

Environment resolution does not depend on Visual Tree attachment, unlike `ThemeContext`'s visual-host lookup, and unlike WPF's `ResourceDictionary`/`DynamicResource`, which resolve by walking the logical tree after an element has been constructed and attached. ElwindUI's `view!` evaluates a component's descendant tree synchronously during construction, before any `UIElement` exists to attach — there is no already-built tree available to walk at resolution time. Environment is therefore carried the way SwiftUI's `@Environment` and Jetpack Compose's `CompositionLocal` are: as an implicit parameter threaded through the construction/build call chain by the framework, never written, read, or passed by DSL author code.

Concretely, `elwindui-codegen` gives every generated component a hidden `__environment: EnvironmentContext` field and a matching hidden constructor parameter, populated from the parent's context (or an `EnvironmentScope`-derived one) and forwarded to every child component's construction call the generator emits — the same way a Compose compiler plugin inserts an implicit `Composer` parameter into every composable call, or SwiftUI's render graph injects `EnvironmentValues` into a view's property wrappers before evaluating `body`. No DSL-visible API accepts or passes an `EnvironmentContext`; `#[environment(name)]` (`dsl_spec.md` §4, explicitly not a constructor argument) is the only surface a component author writes. This reuses the construction-ordering guarantee `elwindui-codegen` already provides for `#[bindable]` wiring and `ControlTemplate<Self>` (`codegen_design.md` §3.5); it does not add a new attach-time pass.

`#[environment(name)]` fields resolve once, at construction, via `context.get::<K>()`, and subscribe to that key's cell using the same generated typed per-component notification path used for `#[prop]` (`{Component}Property`, `docs/design/runtime/state_management_design.md`). No generic runtime `Binding` object is introduced for Environment; the field behaves like an ordinary reactive input whose change source is the shared cell instead of an internal setter.

### `EnvironmentScope`

`EnvironmentScope` is a codegen-time construct, not a `UIElement` — it produces no render node. It evaluates to `parent_context.derive()` with its declared key/value overrides applied, and passes the derived `EnvironmentContext` into its children's construction through the same threading path as an ordinary parent-to-child edge.

### Change propagation

Setting a value through `EnvironmentContext::set` notifies only the components that hold a live subscription to that specific key's cell — not every component under a context that shares an ancestor. This is deliberately finer-grained than `ThemeContext`'s single monotonically-increasing revision (see "Alternatives considered"); the two invalidation mechanisms are independent and Environment does not reuse or extend the Theme revision counter.

### Backend interaction

Environment resolution is entirely an `elwindui-core`/`elwindui-codegen` concern. It does not read from or write to backend state and does not depend on backend helper/host nodes in the Visual Tree (`theme_environment_spec.md` §2). Backends observe only the already-resolved values a component reads — the same way they already observe an ordinary `#[prop]` value — never `EnvironmentContext` itself.

### Ownership and lifetime

`EnvironmentContext` and its cells are `Rc`-owned and shared. No per-`Control` allocation happens beyond the cells an `EnvironmentScope` override (or a root default) actually creates. A component that declares no `#[environment(..)]` field subscribes to nothing and pays no cost beyond holding the cloned context handle (`theme_environment_spec.md`/spec §35–36 memory policy).

### Test strategy

Propagation/invalidation coverage (Issue #84 acceptance criteria):

- an unmodified key resolves to the same cell (`Rc` identity) across parent and child contexts;
- an `EnvironmentScope` override allocates a new cell only for the overridden key; sibling keys keep their parent's cell identity;
- a value change on a cell resyncs only the components that read that key, not unrelated siblings sharing the same context;
- `EnvironmentContext` threading through nested child component construction is correct independent of Visual Tree attach order;
- `#[environment(...)]` combined with `#[param]`/`#[prop]`/`#[state]`/`#[bindable]`, and an unresolvable `#[environment(name)]`/`EnvironmentScope` key, are rejected at macro-expansion time (`dsl_spec.md` §13 rules 33–35), not at runtime.

### Alternatives considered

- **A single monotonic revision counter per `EnvironmentContext`**, mirroring `ThemeContext`: rejected. It would force every Environment-consuming component under a context to re-check on any override anywhere in that context rather than only the specific key it reads, contradicting the specification's "必要なControlだけStyle参照" memory policy and giving Environment coarser invalidation than the narrower Theme-token surface it is meant to complement.
- **String-keyed dynamic lookup**: rejected per the specification's explicit prohibition and to keep resolution statically typed and checkable by `elwindui-codegen` at macro-expansion time rather than deferred to runtime failures.
- **Resolving Environment through the Visual Tree/visual-host relation (like `ThemeContext`), or deferring resolution to attach time (WPF's `ResourceDictionary`/`DynamicResource`, Flutter's `InheritedWidget`)**: rejected because Environment must be available during `body`/`view!` evaluation, before any `UIElement` exists to attach to (`theme_environment_spec.md` §2, "Component生成" ordering). WPF and Flutter can defer to attach time because their element/widget constructors do not themselves synchronously build the full descendant tree; ElwindUI's `view!` does. Confirmed in design review (2026-08-12): construction-time resolution is kept, implemented as a codegen-inserted implicit parameter in the same spirit as SwiftUI's `@Environment` injection and Compose's `CompositionLocal`, not as a DSL-visible constructor argument.
