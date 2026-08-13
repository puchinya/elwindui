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

**Environment resolution is mount-time, not construction-time** (revised 2026-08-13, CI-5 of Issue #80's Component construction/mount/build lifecycle refactor — see `component_lifecycle_design.md` §4/§4a/§4d for the full lifecycle model this section now assumes; this reverses the 2026-08-12 "construction-time, not attach-time" decision recorded below in "Alternatives considered," with the reasoning for the reversal recorded there). `Component::new()` creates the logical instance only; `mount(environment: EnvironmentContext)` (introduced in CI-3, made load-bearing for descendant construction in CI-4) establishes the component's effective `EnvironmentContext` — stored in `self.__mount_environment: OnceCell<EnvironmentContext>` — before `__build_view()` resolves this component's own `#[environment(name)]` fields and, in turn, before any descendant element (including a nested user component's own constructor arguments) is built. This is still not Visual-Tree-attachment-based the way `ThemeContext`'s visual-host lookup or WPF's `ResourceDictionary`/`DynamicResource`/Flutter's `InheritedWidget` are (none of those apply here — ElwindUI has no Visual Tree yet at the point `#[environment(name)]` fields resolve, since descendant construction itself happens from the *same* `__build_view()` call, immediately afterward) — but it is no longer tied to the Rust call stack either.

`#[environment(name)]` fields resolve from `self.__mount_environment.get().expect(..)` (never absent at this point — `mount()` always sets it before `__build_view()` runs) inside `__build_view()`, overwriting each field's already-allocated `Cell`/`RefCell` storage (seeded with `K::default_value()` in `construct()`, since a struct literal cannot leave a field unset and the real value isn't known until mount). The live-update subscription installed afterward reads the same `self.__mount_environment` to `.subscribe::<K>(..)`. There is no longer a separate, ambient-captured `__environment: EnvironmentContext` field distinct from `__mount_environment` — CI-5 deleted it; every view-bearing component already carries `__mount_environment` unconditionally (CI-3's build-idempotency guard), so reusing it for Environment resolution adds no new per-component field, only a new *reader* of an already-present one.

`mount()`'s own `environment` argument is still, for now, supplied via a temporary bridge (`EnvironmentContext::current()`, called from `on_constructed()`/`new()` — see `component_lifecycle_design.md` §4a) rather than genuine explicit tree/parent propagation; Issue #80's CI-6 removes that ambient thread-local mechanism entirely and threads a real, non-ambient context through instead. Until CI-6 lands, the *observable* resolution timing and values are unchanged from before this section's revision (the same ambient context still ends up being what `mount()` resolves against) — what changed is *the mechanism* (mount-time read of an explicit parameter, not a second independent ambient read inside `construct()`) and *the ordering guarantee* (a component's own Environment-dependent fields are now provably resolved before its own descendant elements build, which a purely-ambient, construction-order-dependent read could not guarantee once construction and mount became separable phases).

No DSL-visible API accepts or passes an `EnvironmentContext`; `#[environment(name)]` (`dsl_spec.md` §4, explicitly not a constructor argument) remains the only surface a component author writes. `#[environment(name)]` fields still subscribe using the same generated typed per-component notification path used for `#[prop]` (`{Component}Property`, `docs/design/runtime/state_management_design.md`); no generic runtime `Binding` object is introduced for Environment.

### `EnvironmentScope` (specified, not yet implemented — Issue #100, CI-7 of Issue #80)

`EnvironmentScope` is a codegen-time `view!` construct, not a `UIElement` — it produces no render node. Its intended implementation, updated for mount-time resolution: derive an overridden context (`derive()` off the enclosing component's own `self.__mount_environment`, then `set()` for each declared override) and pass that derived context to each of its children's own `mount(..)` call, instead of the enclosing component's own `__mount_environment` — no `enter()`/ambient-stack manipulation is needed for this once child construction/mount is itself explicit (CI-4), since `EnvironmentScope` becomes purely a matter of *which* `EnvironmentContext` value a `Child.mount(..)` call is given, not a call-stack scope.

Issue #100/CI-7 tracks the actual implementation: `elwindui-codegen`'s child-element planning (`plan_element`) produces a flat `Vec<PlannedNode>`, with construction emitted per node in that flat order (`emit_construction`, now spliced into `__build_view()` per CI-4). `EnvironmentScope` needs a contiguous *range* of its children's construction-and-mount statements to receive the derived context instead of the enclosing component's own — a structural concept the current flat model does not yet support, distinct from `for`/`if`/`match`'s existing dynamic-region machinery, and the specific forward-compatibility concern CI-4's own design review flagged for CI-7 to resolve.

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

**2026-08-12 "construction-time, not attach-time" decision — reversed 2026-08-13 (CI-5 of Issue #80).** This document previously recorded, as a confirmed design-review outcome, that Environment resolution must stay construction-time because ElwindUI's `view!` builds a component's descendant tree synchronously, before any `UIElement`/Visual Tree exists to attach to, and that an *explicit hidden constructor parameter* threaded through every child construction call had already been tried and rejected (Issue #84/PR #101) in favor of the thread-local ambient stack, specifically because constructor-threading would have touched every generated constructor signature in the workspace regardless of whether a component uses Environment.

The repo owner explicitly authorized reversing that decision as part of Issue #80's Component construction/mount/build lifecycle refactor (see `component_lifecycle_design.md`, the tracking document for that initiative). Resolution is now **mount-time**: a component's `#[environment(name)]` fields resolve from the `EnvironmentContext` its own `mount(environment)` call received (§"Resolution and component integration", above), not from a second, independent ambient read performed inside `construct()`.

**Why this does not repeat the 2026 constructor-parameter-threading mistake**: `mount(environment: EnvironmentContext)` is not a parameter added *because of* Environment — it is a lifecycle primitive every view-bearing component already has (Issue #80's CI-3), introduced independently of whether that component consumes Environment, to separate logical construction (`new()`) from initial view build. Reading `self.__mount_environment` inside `__build_view()` therefore adds no new parameter, no new struct field, and no new codegen footprint to a component that declares no `#[environment(..)]` field — `__mount_environment: OnceCell<EnvironmentContext>` is already unconditionally present (it is CI-3's own build-idempotency guard), so an Environment-consuming component's `#[environment(name)]` fields are simply one more *reader* of a field that already exists for an unrelated reason. This is the structural difference from the rejected 2026 approach: that one meant every nested `Type::new(args)` call site gained a new argument whether or not the callee cared; this one means an Environment-consuming component's *own* field-resolution code reads a value that was going to be computed and stored on `self` regardless.

The remaining rationale below is unaffected by this reversal and still holds:

- **A single monotonic revision counter per `EnvironmentContext`**, mirroring `ThemeContext`: rejected. It would force every Environment-consuming component under a context to re-check on any override anywhere in that context rather than only the specific key it reads, contradicting the specification's "必要なControlだけStyle参照" memory policy and giving Environment coarser invalidation than the narrower Theme-token surface it is meant to complement.
- **String-keyed dynamic lookup**: rejected per the specification's explicit prohibition and to keep resolution statically typed and checkable by `elwindui-codegen` at macro-expansion time rather than deferred to runtime failures.
- **Resolving Environment purely through Visual Tree attachment** (`ThemeContext`'s visual-host lookup, WPF's `ResourceDictionary`/`DynamicResource`, Flutter's `InheritedWidget`): still not what ElwindUI does, even after this reversal — there remains no Visual Tree node to attach to at the point `#[environment(name)]` fields resolve (mount-time precedes descendant construction within the same `__build_view()` call, it does not follow a completed Visual Tree the way those systems' attach-time resolution does). Mount-time is a middle ground this section's original two-option framing (construction-time vs. attach-time) did not consider: neither tied to the Rust call stack (like ambient construction-time) nor tied to a completed Visual Tree (like true attach-time).
- **An explicit hidden constructor parameter threaded through every child construction call** (`elwindui-codegen`'s `build_component_args`/`emit_construction`): still rejected, for the same reason as before (Issue #84/PR #101) — it is *not* what `mount(environment)` is; see the reversal rationale above for why `mount()` avoids this specific failure mode while a bare constructor parameter would not have.
- **SwiftUI's `@Environment` as the literal model for the chosen mechanism**: an earlier version of this document described the (now-removed) thread-local stack as working "the way SwiftUI's `@Environment` ... works." That comparison was already corrected once (SwiftUI resolves by tree position in a persistent, re-diffed attribute graph, not call-stack nesting — closer to Flutter's `InheritedWidget`/`BuildContext`; Compose's `CompositionLocal` was the closer analogy for the ambient mechanism). With ambient thread-local propagation itself now superseded by mount-time propagation, ElwindUI's mechanism has moved further *toward* the tree-position-based family (SwiftUI/Flutter) — `mount(environment)` propagates along the explicit mount/tree relationship established in CI-4, not along the Rust call stack — though it remains a from-parent explicit-argument model, not a re-diffed persistent graph.
