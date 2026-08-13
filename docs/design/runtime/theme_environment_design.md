# Theme and Environment implementation design

Related specification: [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md).

Theme and Environment are separate concerns (`theme_environment_spec.md` §2, §9, §39 and Issue #84/#96), but since #96 Theme is defined *in terms of* Environment — a Theme's only job is to call `EnvironmentContext::set` (see `## Theme`) — rather than owning any runtime state of its own. This document keeps their internals in separate sections because they have different responsibilities, not because they share no type. `docs/agents/codegen.md`/`class-model.md` invariants apply to both.

## Theme

Issue #96 replaced the token/variant model this section used to describe (`ThemeToken`/`ThemeValue`/`ThemeHandle`/`ThemeFactory`/`ThemeController`/`ThemeContext`/`SystemTheme`/`#[elwindui::theme_definition]`, and each backend's pull-based native-control default-appearance resolution) with the Preset-over-Environment model below. There is no migration/compatibility path — the old types no longer exist in `crates/elwindui-core/src/theme.rs`.

### Theme is a Preset, not a runtime lookup system

A Theme is not a second resolution mechanism alongside Environment (`theme_environment_spec.md` §9). It is a batch of `EnvironmentContext::set` calls:

```rust
pub trait Theme {
    fn apply(&self, env: &crate::environment::EnvironmentContext);
}
```

`crates/elwindui-core/src/theme.rs` now contains only this trait. There is no `EnvironmentOverrides` type distinct from `EnvironmentContext` — the specification's illustrative `fn apply(&self, env: &mut EnvironmentOverrides)` is non-normative on this point, the same way `EnvironmentContext::current()`/`enter()` already superseded the specification's constructor-threading illustration for Environment itself (see "Alternatives considered" in the `## Environment` section below). `EnvironmentContext::set` already has exactly the right shape for a Preset to call directly: it takes `&self` (interior-mutable cells), so a Theme's `apply` needs no exclusive borrow of the context, and re-applying a different Theme to the *same* context re-mutates existing cells in place, which is what makes switching Themes at runtime reach every live subscriber for free (see "Change propagation" below).

### `#[elwindui::theme]`

```rust
#[elwindui::theme]
struct OceanTheme {
    #[theme(value = Brush::Solid(Color::rgb(0, 166, 200)))]
    tint: Brush,
}
```

is a Rust-only frontend (`elwindui-codegen/src/theme_frontend.rs`, mirroring `environment_frontend.rs`'s shape — it never enters the DSL/`view!` parser, the same way `#[elwindui::environment_key]` doesn't). For each `#[theme(value = expr)]` field, the frontend resolves the field's own identifier through `component_frontend::lookup_same_crate_environment_key` — the exact same same-crate, declaration-ordered registry `#[environment(name)]` fields already resolve against (`theme_environment_spec.md` §2/§9's field-name convention: a Theme field named `tint` targets the Environment Key declared `#[elwindui::environment_key(name = tint, ..)]`, not a field of that name on some other struct). An unresolvable field name is a macro-expansion-time error, not a runtime one — consistent with `dsl_spec.md` §13 rule 34/35's treatment of `#[environment(name)]` itself.

The macro discards the parsed struct's fields entirely (schema-only, exactly like the old `theme_definition` macro's field list) and emits a zero-sized marker struct plus a `Theme` impl:

```rust
pub struct OceanTheme;

impl elwindui::core::theme::Theme for OceanTheme {
    fn apply(&self, env: &elwindui::core::environment::EnvironmentContext) {
        env.set::<TintEnvironment>(Brush::Solid(Color::rgb(0, 166, 200)));
    }
}
```

`env.set::<KeyType>(expr)` is itself the field-type check — a `#[theme(value = ..)]` expression whose type doesn't match the resolved Key's `Value` is a straightforward rustc type error at that call site, so the frontend does not need its own duplicate type-compatibility validation.

### Application boundary — `application_environment()`

Environment resolution is construction-time and ambient-stack-based (see `## Environment` below): a value only reaches a component if that component was constructed while some entered `EnvironmentContext` was ambient. Before #96, nothing in the workspace ever called `EnvironmentContext::enter()` outside tests — `EnvironmentContext::current()`'s fallback (a fresh, unshared `EnvironmentContext::root()`) was the only context any real application ever observed, so a Theme applied to *some* context would not have been observable by already-constructed (or even not-yet-constructed, on a different accidental root) components.

#96 closes this gap with one new piece of API, in `crates/elwindui-core/src/environment.rs`:

```rust
/// The process's single persistent root `EnvironmentContext`. Lazily created once per thread,
/// then reused — unlike `EnvironmentContext::root()`, which always allocates an unrelated new
/// state. A `Theme::apply` call against this context is what a whole application observes.
pub fn application_environment() -> EnvironmentContext;
```

and each backend's `run()` (`elwindui-backend-appkit`/`elwindui-backend-winui3`'s `app.rs`) now holds `application_environment().enter()` for the run loop's entire lifetime, entered before `startup()` runs and dropped only when `run()` itself returns. An application applies a Theme at any point — typically once before/inside `startup()`, and again later from a click handler to switch — by calling `SomeTheme.apply(&elwindui_core::environment::application_environment())` (or the `elwindui::core::environment::application_environment()` facade path).

### Scope reduction relative to the old model (accepted, 2026-08-13)

Two capabilities the old Theme system had are **not** carried into the Preset model by #96 — both confirmed with the user during design, not silently dropped:

- **No per-Window Theme override.** The old `Window.theme: Option<ThemeHandle>` prop is deleted, not reimplemented. Reproducing it correctly needs a Window-scoped Environment override that takes effect *before* the Window's content subtree is constructed — but `Window.content` is a `#[prop(content: Rc<dyn UIElementExt>)]` (an already-built value), so by the time `set_content`/`set_theme` could run, the content's own `#[environment(name)]` fields have already resolved against whatever was ambient at the *caller's* construction point. Doing this correctly needs the same "wrap a contiguous child range's construction in one `enter()`/drop block" codegen support `EnvironmentScope` (#100) needs — out of scope for #96. A future issue can restore per-Window override once #100 lands, by having a Window with an explicit `theme` override behave as an implicit `EnvironmentScope` around its own content.
- **No automatic native-control default-appearance styling.** The old `SystemTheme` manifest and each backend's pull-based `sync_background`/`sync_text_style` (`native_ui/control.rs`) resolved ~85 standard tokens with a hardcoded fallback chain, so an app could restyle native buttons/text boxes/etc. through a Theme with zero extra declarations. #96 deletes this outright; native controls always render with their platform's own default appearance now. This does not regress the *default* (unthemed) appearance — an unset standard token already resolved to `PlatformDefault` ("do nothing") before #96 — it only removes the *capability* to override it through a Theme, until Semantic Style (#97) and Native Style (#98) reintroduce an Environment-driven equivalent.
- A consequence of dropping native-control styling: the OS light/dark **appearance-changed observation** each backend's `inner/window.rs` used to translate into `ThemeHandle::set_appearance` (feeding the now-deleted `sync_background`/`sync_text_style`) has no remaining consumer and is deleted too. Native controls still follow OS light/dark mode automatically (the platform toolkit does that on its own for an unstyled control) — only Theme-driven re-styling in response to that OS change is gone, consistent with the point above.

### Change propagation

A Theme switch is just `EnvironmentContext::set` calls on `application_environment()`, so it reuses Environment's own per-key subscriber mechanism unchanged (see `## Environment`, "Change propagation" below) — only the components with a live `#[environment(name)]` field on an overridden key re-run. There is no separate Theme-level revision counter or `ThemeChangeImpact` classification; Environment's finer-grained, per-key notification already provides exactly this without a second invalidation system layered on top (`theme_environment_spec.md` §36's memory policy — "必要なControlだけStyle参照").

### Alternatives considered

- **A distinct `EnvironmentOverrides` type for `Theme::apply`**, matching the specification's illustrative `fn apply(&self, env: &mut EnvironmentOverrides)` literally: rejected. `EnvironmentContext::set` already takes `&self`, so introducing a second, `&mut`-taking type would only add a translation layer with no behavioral benefit — the same non-normative-illustration situation the `## Environment` section's own "Alternatives considered" already documents for `EnvironmentContext::current()`/`enter()` versus the specification's constructor-threading sketch.
- **Preserving a variant-enum-per-Theme-type shape** (the old `ThemeFactory::Variant`/`ThemeController::set_variant`): rejected. The specification's Preset model has no variant concept — each named look (`OceanTheme`, `SolarizedTheme`, ...) is its own `#[elwindui::theme]` type/instance; "switching" is applying a different instance to the same `EnvironmentContext`, not selecting a variant within one type. This is strictly simpler and needs no `ThemeFactory`-equivalent trait.
- **Keeping `SystemTheme`'s fallback-chain machinery running internally (not exposed via the deleted DSL) until #97/#98 land**: rejected per explicit user decision (2026-08-13) in favor of full, immediate deletion — see "Scope reduction", above.

## Environment

### Context

`EnvironmentContext` is a small `Clone` (`Rc`-backed) handle holding a set of typed entries, one per `EnvironmentKey`. Each entry is a reactive cell. A context created by `derive()` shares every entry it does not override with its parent by `Rc` identity, and allocates a fresh cell only for the keys the caller overrides — mirroring `ThemeContext`'s "derive, don't mutate" rule for the application/Window relationship, but per-key rather than per-context.

Lookup is by `EnvironmentKey::Value`'s type, resolved through the key's `TypeId`; there is no string-keyed path (`theme_environment_spec.md` §2, `dsl_spec.md` §4/§8).

`EnvironmentContext::application_environment()` (added by Issue #96 — see `## Theme`) is the one process-lifetime exception to "every context is created and owned by whatever derived it": a lazily-initialized, thread-local persistent root, reused across calls rather than re-allocated. Both backends' `run()` hold it entered for the whole run loop so that construction anywhere in the application observes overrides applied to it. Nothing else in this section changes because of it — it is an ordinary `EnvironmentContext`, just one with a well-known, stable identity an application (or a Theme) can target deliberately instead of relying on whatever happens to be ambient.

### Resolution and component integration

Environment resolution does not depend on Visual Tree attachment, unlike `ThemeContext`'s visual-host lookup, and unlike WPF's `ResourceDictionary`/`DynamicResource` or Flutter's `InheritedWidget`, both of which resolve by walking an already-built element tree. ElwindUI's `view!` evaluates a component's descendant tree synchronously during construction, before any `UIElement` exists to attach — there is no already-built tree available to walk at resolution time.

Environment is resolved through a thread-local ambient stack, `EnvironmentContext::current()`/`EnvironmentContext::enter()` (`crates/elwindui-core/src/environment.rs`). Because `view!` bodies build their descendant tree through ordinary, synchronous, single-threaded nested Rust calls — never re-entrant, never interleaved across unrelated subtrees — the ambient context observed at each construction point is exactly the one an explicit hidden constructor parameter threaded through every call would have carried, without changing any constructor's signature. `elwindui-codegen` reads `EnvironmentContext::current()` once, at the top of a generated component's construction, only for a component that actually declares at least one `#[environment(name)]` field; a component with no such field never touches Environment, and no other component's constructor gains a new parameter or struct field.

The resolved context is cached in a hidden `__environment: EnvironmentContext` field (present only on a component that has an `#[environment(..)]` field), so a later subscription callback re-reads from that specific capture rather than from whatever happens to be ambient at an arbitrary later time. No DSL-visible API accepts or passes an `EnvironmentContext`; `#[environment(name)]` (`dsl_spec.md` §4, explicitly not a constructor argument) is the only surface a component author writes.

This design was settled during implementation, superseding an earlier draft of this document that instead described explicit constructor-parameter threading through every child construction call `elwindui-codegen` emits (`build_component_args`/`emit_construction`). That would have touched every component in the workspace — whether or not it uses Environment — purely to carry an internal plumbing value; the thread-local approach keeps the identical observable contract (construction-time resolution, Visual-Tree-independent, never DSL-visible) with a far smaller, opt-in codegen footprint. See "Alternatives considered", below, and the review discussion on Issue #84's Pull Request (#101).

`#[environment(name)]` fields resolve once, at construction, via the captured `EnvironmentContext` (`self.__environment.get::<K>()`, seeded from `EnvironmentContext::current()` at that point), and subscribe to that key's cell using the same generated typed per-component notification path used for `#[prop]` (`{Component}Property`, `docs/design/runtime/state_management_design.md`). No generic runtime `Binding` object is introduced for Environment; the field behaves like an ordinary reactive input whose change source is the shared cell instead of an internal setter.

### `EnvironmentScope` (specified, not yet implemented — Issue #100)

`EnvironmentScope` is a codegen-time `view!` construct, not a `UIElement` — it produces no render node. Its intended implementation derives an overridden context (`EnvironmentContext::current().derive()`, then `set()` for each declared override), `enter()`s it for the duration of its children's construction, and lets the guard drop once they are built — reusing `EnvironmentContext::enter()`, the same primitive an ordinary component's own construction does not need to call at all (only the ambient stack it reads from).

Issue #100 tracks the actual implementation: `elwindui-codegen`'s child-element planning (`plan_element`) produces a flat `Vec<PlannedNode>`, with construction emitted per node in that flat order (`emit_construction`). `EnvironmentScope` needs a contiguous *range* of its children's construction statements wrapped in one `enter()`/drop block — a structural concept the current flat model does not support, distinct from `for`/`if`/`match`'s existing dynamic-region machinery.

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
- **Resolving Environment through the Visual Tree/visual-host relation (like `ThemeContext`), or deferring resolution to attach time (WPF's `ResourceDictionary`/`DynamicResource`, Flutter's `InheritedWidget`)**: rejected because Environment must be available during `body`/`view!` evaluation, before any `UIElement` exists to attach to (`theme_environment_spec.md` §2, "Component生成" ordering). WPF and Flutter can defer to attach time because their element/widget constructors do not themselves synchronously build the full descendant tree; ElwindUI's `view!` does. Confirmed in design review (2026-08-12): construction-time resolution is kept, not attach-time resolution.
- **An explicit hidden constructor parameter threaded through every child construction call** (`elwindui-codegen`'s `build_component_args`/`emit_construction`), the mechanism originally approved for construction-time resolution: superseded during implementation (Issue #84's Pull Request #101) in favor of the thread-local ambient stack described above. Both give the identical observable contract; the constructor-parameter version would have touched every component's generated signature in the workspace, not only components that declare `#[environment(..)]` fields.
- **SwiftUI's `@Environment` as the literal model for the chosen mechanism**: an earlier version of this document described the thread-local stack as working "the way SwiftUI's `@Environment` ... works." That comparison is imprecise and was corrected after review: SwiftUI resolves `EnvironmentValues` by the view's *position in its persistent, re-diffed attribute graph*, not by call-stack nesting — closer in spirit to Flutter's `InheritedWidget`/`BuildContext` (tree-position-based) than to a lexical/call-stack scope, and not fully documented by Apple. Jetpack Compose's `CompositionLocal` — an implicit `Composer` parameter the compiler inserts through every `@Composable` call in the composition chain — is the closer analogy for ElwindUI's thread-local stack, since Compose (like `view!`) treats UI construction as a genuine single-pass, synchronous call chain rather than a persistent graph revisited out of call order.
