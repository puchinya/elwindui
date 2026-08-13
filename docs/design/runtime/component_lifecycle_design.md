# Component construction, mount, and build lifecycle

Related specification: [`../../specs/dsl_spec.md`](../../specs/dsl_spec.md) §3 "Lifecycle hooks", §5 "EnvironmentScope". Related design: [`ui_tree_design.md`](ui_tree_design.md) (Construction/Mounting/Unmounting terminology, defined here in generated-code terms), [`theme_environment_design.md`](theme_environment_design.md) (Environment resolution, migrated to mount-time by a later issue in this initiative), [`../tools/class_macro_design.md`](../tools/class_macro_design.md) (`#[class]`'s `construct`/`on_constructed` contract, reconciled below).

Tracking issue: [#80](https://github.com/puchinya/elwindui/issues/80). This document is the design deliverable of child issue CI-1 (#104).

## 1. Current construction flow (as of this writing)

There is no `Component` trait in this codebase; "component" is a DSL/codegen-level concept realized in two separate proc-macro crates that cooperate:

- **`elwindui-codegen`**'s `generate_view` (`crates/elwindui-codegen/src/codegen.rs`, the function that expands a `#[elwindui::component]` struct with a `view!` body) plans the element tree (`plan_element`, post-order flatten into `Vec<PlannedNode>`) and emits per-node construction (`emit_construction`). For a nested user component, `emit_construction` emits `let #binding = #ChildType::new(#args);` — a direct, synchronous, recursive call into the child's own generated `new()`.
- **`elwindui-macros`**'s `#[elwindui::class]` (`crates/elwindui-macros/src/class.rs`) independently owns generating the *outer* `new()` for any class using the macro (which includes every `view!`-bearing component, since `generate_view` targets are `#[class]`-composed). Its contract (`docs/design/tools/class_macro_design.md` §"Construction", `crates/elwindui-macros/src/class.rs:110-116`):
  - A class must define `fn construct(..) -> Self` (never a hand-written `new` — hand-written `new` is a compile error, "it would compete with the generated ownership sequence").
  - The macro generates `new(..) -> Rc<Self>` as `Rc::new_cyclic(|weak| Self::construct(..))`, then calls `__elwindui_run_on_constructed()` **unconditionally, automatically, exactly once**, immediately after `Rc::new_cyclic` returns.
  - `__elwindui_run_on_constructed()` chains any optional `fn on_constructed(&self)` a class defines, **base-first** across one `inherits` hop (composed multi-level chaining is a known limitation, `docs/status/implementation_status.md`).

`generate_view` supplies the `construct`/`on_constructed` bodies `#[class]` expects:

- `construct(..)` (`codegen.rs:4642-4646`) runs `construct_stmts` — which is where the entire descendant tree is built today: `plan_element`'s flattened nodes are constructed in post-order via `emit_construction`, each nested user-component node making its own recursive `ChildType::new()` call (which itself runs this whole pipeline, including *that* child's own `on_constructed`, before returning). `#[environment(name)]` fields are also resolved here, first, via `EnvironmentContext::current()` (ambient thread-local read).
- `on_constructed(&self)` (`codegen.rs:4674-4700`) runs, in order: `content_attach_stmt`, event `wiring_stmts` (reconstructing a real `Rc<Self>` from the `__self_weak` field `construct` populated), `__refresh_dynamic_regions()`, `resync()`, then (again gated on the reconstructed `Rc`) `component_self_subscription`, `subscribe_stmts`, `own_environment_subscribe_stmts`, and finally `on_mount_stmt` — the DSL-author's `on_mount { .. }` block, spliced verbatim, last.

For a plain (non-`#[class]`, `has_view == true` but no ancestor/shape composition) component, `generate_view` instead emits a single hand-rolled `new()` (`codegen.rs:4725-4739`) that inlines the same sequence directly — construct, `Rc::new`, content attach, wiring, resync, subscribe, `on_mount` — with no `construct`/`on_constructed` split at all today.

`on_unmount` is codegen'd as an inert `__run_on_unmount(&self)` method with zero call sites in either shape (no detach/teardown trigger exists in the runtime today). `on_update` has no codegen path at all.

**Net effect:** today, "construct," "mount," and "build" are the same synchronous call, for both component shapes. The `#[class]`-composed shape already has a two-function seam (`construct`/`on_constructed`) that looks superficially like a construction/mount split, but both halves run back-to-back inside one macro-generated `new()`, automatically, with no host-attach boundary and no way to defer the second half.

## 2. Target lifecycle state model

```
Created
   │
   │ mount(environment)
   ▼
Mounted ──(initial build, performed by mount() itself)──▶ Built
   │
   ├─ layout / render / events / hide / show (existing reactive property updates continue to apply in place)
   │
   │ unmount (cascades to children; Window close is the concrete trigger — CI-8)
   ▼
Unmounted
```

The internal representation does not need a literal `enum LifecycleState` field on every generated component (see §6, Performance) — what must remain true and distinguishable is:

- **Created**: `new()` has returned an `Rc<Self>`. No `view!` has been evaluated. No descendant Components exist. No Environment has been resolved or subscribed to. `__environment` (where present) is absent/unset.
- **Mounted+Built**: `mount(environment)` has run to completion. Per spec §5, "mount" and "initial build" are not separately invoked by generated/application code — `mount()` performs both, in that order, as one step. `__environment` is set. The view subtree exists. `on_mount` has fired.
- **Unmounted**: subscriptions cancelled, Visual subtree released, native backend objects released where applicable. Re-mounting an unmounted Component is not part of this initiative's scope (§80's non-goals; nothing in the Codex Task spec requires resurrection).

This directly reuses `ui_tree_design.md`'s existing "Construction / Mounting / Unmounting" terminology (`ui_tree_design.md:23-27`) rather than inventing new nouns; "Built" is introduced here as the sub-state `ui_tree_design.md` already implies ("Mounting attaches the subtree to a host and enables ... layout, rendering, input") without a separate name. No code in this repository names a `build()` phase independently reachable from `mount()` — per Codex Task spec §5, a separate internal build function ("desirable for code organization and future rebuild behavior") is an implementation-organization choice for CI-2/CI-3, not a third state application or generated code observes.

## 3. Ownership rule (spec §23)

A logical Component may exist before its Visual subtree exists. No code anywhere in the codebase — generated or hand-written — may assume `Component exists ⇒ visual root exists`. Concretely: between `new()` returning and `mount()` completing, a Component's `#[prop]`/`#[state]` accessors must remain callable (they operate on backing fields that exist from `Created` onward), while any accessor that reaches into the Visual subtree (a generated child-element accessor, anything walking `visual_collection`) is illegal before `Mounted+Built` and must be rejected or deferred per CI-3 §24's "legal before mount" catalogue.

## 4. Resolving the `#[class]` `construct`/`on_constructed` seam (spec §5 in #80's unresolved questions)

This is the central structural decision this document must make, because every later child issue's codegen work depends on it.

**Decision: `on_constructed()`'s existing role is subsumed by `mount()` — it does not remain a parallel, separately-invoked concept.** Concretely:

- `construct(..)` keeps its existing meaning — "build the bare struct value" — and becomes, once CI-4/CI-5 remove eager descendant construction and ambient Environment reads from `construct_stmts`, a faithful implementation of the new lifecycle's `new()`/Created phase. No change to `#[class]`'s contract that `construct` is the sole source of constructor arguments and that hand-written `new` is rejected.
- `on_constructed(&self)`'s current body (content attach, wiring, `__refresh_dynamic_regions`, `resync`, subscriptions, `on_mount`) is exactly the work `mount()` needs to perform (Codex Task spec §5, §16). Rather than inventing a second, redundant post-construction hook, the generated `mount()` method becomes (for `#[class]`-composed components) the thing that invokes this existing body — but the body must stop running **automatically and unconditionally** inside `#[class]`-generated `new()`.
- This means `elwindui-macros/src/class.rs`'s generated `new()` — today unconditionally `Rc::new_cyclic(construct); __elwindui_run_on_constructed();` — is **in scope for this initiative**, not just `elwindui-codegen`. CI-2 and CI-3's critical-files lists are amended (see §7) to include `crates/elwindui-macros/src/class.rs`.
- The exact mechanism by which `#[class]`'s generated `new()` stops auto-invoking `__elwindui_run_on_constructed()` for `view!`-bearing (mount-needing) classes, while classes with no `view!` (leaf native controls that don't participate in this lifecycle split, data-only classes) may reasonably keep today's automatic behavior, is left to CI-2/CI-3's own `phase:design` — candidate shapes include a new class-level opt-out attribute, or `generate_view` always supplying its own `mount`-aware `on_constructed` wrapper that the unconditional auto-call still reaches but which no-ops until explicitly re-armed by `mount()`. Either shape must preserve `#[class]`'s existing invariants: `construct` remains the single source of constructor arguments, hand-written `new` remains rejected, `on_constructed` chaining remains base-first across `inherits` hops (`macro_class_spec.md` §13.3's "most-derived object" contract for `__self_weak` is unaffected — `mount()` reconstructing a real `Rc<Self>` from the same weak handle is a direct continuation of today's approach, not a replacement).
- For the plain (non-`#[class]`) `has_view == true` shape, CI-2 introduces an equivalent `construct`/mount-body split so both shapes converge on the same generated method shape post-CI-3; this shape currently has no `#[class]`-imposed constraint to reconcile with, so it is the simpler half of CI-2's work.

## 5. Forward-compatibility notes (no implementation; spec §19–§22)

- **NativeControl** (§19): the native backend widget does not need to exist at `Component::new()` time under this model — nothing in the Created state requires a materialized native handle. Whether a given backend chooses to allocate the native object eagerly anyway (as both AppKit and WinUI3 already do for `Window`'s native shell, per §5's discussion of Window specifically — a different question, deferred to CI-8) versus lazily at mount/build is a backend implementation choice this lifecycle model does not constrain either way, as long as Environment-dependent configuration happens no earlier than `mount()`.
- **Custom Control Style / Native Style** (§20–§21): both need Environment to be known before they resolve a style, which is exactly what `mount(environment)` guarantees is available before the initial build step runs. Nothing in the state model in §2 above blocks a future `mount()` implementation from inserting a style-resolution step between "Environment established" and "build own view."
- **Lazy construction** (§22): a container choosing not to mount/build a child immediately after that child's `new()` is compatible with the Created state existing independently of Mounted+Built — this is precisely why Created and Mounted+Built are modeled as distinguishable states rather than collapsed into one. `ui_tree_design.md`'s existing "Participation" section (collapsed/inactive hosted subtrees retaining state while excluded from layout/render) is the closest existing precedent and is not contradicted by anything above.

None of the above is implemented as part of this initiative (per spec §44's non-goals); each is confirmed here only as "not blocked by the chosen lifecycle model."

## 6. Performance constraints (spec §34, restated as non-functional requirements for every later child issue)

Every child issue (CI-2 through CI-9) implementing part of this lifecycle must not introduce: heap allocation per binding; an Environment map per `UIElement` (the existing `EnvironmentContext`/`EnvironmentCell` `Rc`-sharing-by-identity design already avoids this and must be preserved through the mount-time migration in CI-5); a full-tree rebuild on `show()` (CI-8's idempotent re-show); a full-tree rebuild on Environment value change (CI-5/CI-6's reactive-cell sharing must continue to fan out only to actual subscribers); or duplicate Visual subtree storage. The Created state existing independently of Mounted+Built (§2, §3) must be cheap — no Visual/native allocation happens merely by calling `new()`.

## 7. Amendments to already-created child issues

Discovered during this design pass, not anticipated at requirements time (#80):

- **CI-2** (#105) and **CI-3** (#106): add `crates/elwindui-macros/src/class.rs` to critical files/areas. Their design phases must decide the exact mechanism (§4 above) by which `#[class]`-generated `new()` stops unconditionally auto-invoking the mount-equivalent work.
- **CI-4** (#107): its `plan_element`/`emit_construction` rewrite must account for both component shapes (`#[class]`-composed and plain) converging on the same construct/mount split established by CI-2/CI-3.

(Tracking comments recording these amendments are added to #105 and #106 alongside this document's publication.)

## 8. Required design topics (per `docs/agent-workflow/design.md`)

- **Public API / externally visible behavior**: `mount(environment: EnvironmentContext)` becomes part of the generated Component surface; whether it is public or `#[doc(hidden)]` is CI-3's decision, but per spec §29 ("do not expose framework-internal lifecycle plumbing unnecessarily to DSL authors") and the existing convention of doc-hidden generated methods (`__elwindui_run_on_constructed`, `__refresh_dynamic_regions`), the default posture should be doc-hidden unless a concrete DSL-author use case needs direct visibility. `on_update(field, ...)` becomes a real, functioning DSL construct (CI-4) where today it silently does nothing.
- **Type and module responsibilities**: `elwindui-codegen` (`generate_view`) and `elwindui-macros` (`class.rs`) jointly own generated-`new()`/mount shape, per §4 above — this is a change from today's assumption (implicit in prior planning) that only `elwindui-codegen` needed to change.
- **Ownership and lifetime model**: unchanged from §5's existing summary (Visual tree via `Rc`/`Weak`, `__self_weak` reconstruction pattern) — `mount()` reuses the existing `__self_weak` upgrade pattern, does not introduce a new ownership primitive.
- **Data and event flow**: construction no longer implies subscription; Environment-dependent subscriptions move from construction-time to mount-time (CI-5), consistent with §26.
- **Backend abstraction and backend-specific behavior**: out of scope for CI-1; CI-8 covers Window specifically, CI-9 covers backend verification.
- **Thread and async model**: unchanged — single-thread UI construction, no cross-thread mounting introduced (spec §33).
- **Error representation and recovery**: the specific mechanism (panic/`Result`/no-op) for lifecycle misuse (mount twice, build before mount) is CI-3's decision (#80 unresolved question #1); this document only establishes that such misuse must be a distinguishable, deterministic condition given the state model in §2.
- **Compatibility and migration impact**: `EnvironmentContext::current()`/`.enter()` removal (CI-6) is a breaking API change for any downstream code calling them directly; `#[class]`'s generated `new()` behavior change (§4) is source-compatible (call sites don't change) but changes *when* `on_constructed`-equivalent work runs for affected classes — flagged for CI-2/CI-3's own compatibility note.
- **Performance or caching constraints**: §6 above.
- **Test strategy**: spec §35–§39's test categories (construction, mount, Window, reactivity, cleanup) are distributed across CI-2 through CI-9 per their own acceptance criteria; this document does not duplicate them.
- **Alternatives considered**: (a) introducing an entirely new, separate mount hook parallel to `on_constructed` — rejected as redundant, since `on_constructed`'s existing body already does everything `mount()` needs; the only change needed is *when* it fires, not *what* it does. (b) Solving the `#[class]` seam by leaving `#[class]`'s automatic `on_constructed` invocation untouched and building a second, independent mount concept purely in `elwindui-codegen` for `view!`-bearing components — rejected because it would mean `on_constructed` fires twice in different senses (once automatically per `#[class]`'s existing contract, once again via the new mount call), which is exactly the "silently duplicate" hazard spec §17 warns against.
