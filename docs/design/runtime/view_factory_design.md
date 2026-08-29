# ViewFactory: generic deferred View factory

Normative contract: [`../../specs/ui_spec.md`](../../specs/ui_spec.md) (`context_popup`)

Tracking: [#161](https://github.com/puchinya/elwindui/issues/161) (this document's own scope — the
`ViewFactory` runtime/backend foundation); declarative `context_popup: view! { .. }` DSL codegen
sugar (§3) was split out to [#162](https://github.com/puchinya/elwindui/issues/162) and is now
implemented — see `docs/design/runtime/popup_context_menu_design.md`'s "Declarative `context_popup:
view! { .. }` DSL" subsection for the lowering mechanism in full; §3/§4 below are kept as a shorter
summary from this document's own, narrower perspective.

## 1. Relationship to `ControlTemplate<C>`

`ViewFactory` and `ControlTemplate<C>` (`docs/design/runtime/control_template_design.md`) are
**separate public types** — neither an alias nor a shared public supertype of the other, and this is
deliberate, not an oversight to unify later.

`ControlTemplate<C>` carries Control-specific semantics that `ViewFactory` does not share:
`templated_parent` (a strongly-typed `Weak<C>`), the `ContentPresenter`/logical-content vs.
template-visual split, and template selection fixed for the owning Control's entire mount lifetime
(selected once, in `mount()`, never re-evaluated). `ViewFactory` has none of these — it exists for
content whose lifecycle is independent of any single owner's mount lifetime and may be built again on
each new demand (a popup opened a second time gets a second, unrelated `ViewFactory::build` call, not
a re-selection of the same template instance).

What the two types share is only the shape of their private storage:

```rust
struct DeferredViewFactory<C> {
    factory: Rc<dyn Fn(C) -> Option<Rc<dyn UIElementExt>>>,
}
```

(`crates/elwindui-core/src/ui/view_factory.rs`, `pub(crate)`, not exported). `ControlTemplate<C>`
wraps `DeferredViewFactory<ControlTemplateContext<C>>` and unwraps the `Option` in `__build` (its own
contract guarantees a factory always produces a root — building is infallible from a caller's
perspective, since `ControlTemplateContext<C>::control: Rc<C>` is always alive during a build).
`ViewFactory` wraps `DeferredViewFactory<ViewBuildContext>` and stays `Option`-returning end to end,
since its `ViewBuildContext::owner: Weak<dyn UIElementExt>` may have already been dropped by build
time.

## 2. `ViewFactory` / `ViewBuildContext`

```rust
pub struct ViewBuildContext {
    pub owner: Weak<dyn UIElementExt>,
    pub environment: EnvironmentContext,
}

pub struct ViewFactory {
    factory: DeferredViewFactory<ViewBuildContext>,
}

impl ViewFactory {
    pub fn new(factory: impl Fn(ViewBuildContext) -> Option<Rc<dyn UIElementExt>> + 'static) -> Self;
    pub fn build(&self, context: ViewBuildContext) -> Option<Rc<dyn UIElementExt>>;
}
```

`ViewBuildContext` carries no popup-specific field (no dismiss handle) — a popup-specific value
(`PopupDismissAction`) is threaded through the popup-scoped `EnvironmentContext` instead
(`docs/design/runtime/popup_context_menu_design.md` §6), never through `ViewBuildContext` itself, so
that `ViewFactory` stays a genuinely general primitive: today `context_popup`
(`crate::ui::element::UIElement::context_popup`), potentially in the future lazy tab content,
dialogs, sheets, or popovers — none of which is implemented against `ViewFactory` yet, but the type
carries no popup-only shape that would need to be generalized later.

`owner` is always `Weak`: a `ViewFactory` value is typically stored as a property of the very element
it is a deferred view *for* (e.g. `context_popup` on the element that opens the popup), so a factory
that captured its owner strongly would create an ownership cycle through that property. `build`
upgrading a dead `owner` and returning `None` (rather than panicking or falling back to some default)
is the documented, tested contract for "the owner went away between template capture and build time."

## 3. Declarative `context_popup: view! { .. }` — implemented (Issue #162)

The durable architecture for compiling `context_popup: view! { .. }` (the same `view!` grammar as an
ordinary Component body, deferred to popup-open time, reusing `view!`'s existing parser/AST/codegen
construction pipeline rather than a separate DSL) is implemented, tracked by Issue
[#162](https://github.com/puchinya/elwindui/issues/162).

The design question the original investigation (recorded on #162) identified was precise: unqualified
identifiers written inside `context_popup: view! { .. }` (e.g. `item: selected_item`, referencing the
*enclosing* Component's own field) need to resolve exactly the way any other bare name inside an
ordinary `view!` body already does — through the same `self`/`vm` accessor-rewriting mechanism
ordinary nested elements already use — rather than through `ControlTemplate`'s
`templated_parent.foo`-style *explicit*-qualification convention. The shipped solution lowers the
whole `view! { .. }` block, at macro-expansion time, into its own hidden `ComponentDef`/`ViewDef` — a
real, ordinary Component whose single synthetic field (`__view_owner: Weak<Owner>`) is treated exactly
like `ControlTemplate`'s own `templated_parent` for weak-owner and Environment-propagation purposes.
Because the lowered body is a genuinely ordinary Component, every existing *DSL-attribute-value*
bare-name-resolution code path in `codegen.rs` (`emit_expr`'s own `ViewExpr::Path` handling —
`on_mount`/`lets`/`if`/`match`/`for` as *structural* `view!` constructs, element/value codegen) already
handles the body's interior correctly with no new `ViewExpr`/`ClosureBody` match arms needed there.
The implicit-owner fallback that handling applies is schema-gated (PR #165 final rereview
remediation, A2 — `ImplicitOwnerDef::readable_fields`, computed once from the source Component's own
effective fields): only a bare name that is actually a real, readable field of the source Component
falls back to `__view_owner.<name>()` — an ordinary Rust name with no relation to the source Component
is never rewritten.

This does **not** extend to the *raw Rust* inside `on_mount { .. }`/`on_unmount { .. }`/`on_update { ..
}` blocks and `on_*` event-handler closure bodies — an arbitrary, unconstrained Rust statement
sequence, not DSL grammar, walked by a separate `syn::visit_mut::VisitMut` pass
(`ViewClosureRewriter`/`rewrite_view_closure_block`/`rewrite_view_closure_expr`) that already existed
for event handlers before this Issue. That pass genuinely did need generalizing (PR #165 review
remediation, A2, further tightened by PR #165 final rereview remediation, A2): it gained an
implicit-owner fallback (`ViewClosureRewriter::resolved_implicit_owner_field`, reusing the same
2-segment `owner.field` machinery `resolved_owner` already uses) so a bare name inside one of these
raw blocks that is a known-readable field of the source Component (the exact same
`ImplicitOwnerDef::readable_fields` schema the DSL-attribute-value path above consults, so both paths
agree on membership) falls back to `__view_owner` the same way a DSL attribute value's bare name
already did. An unshadowed bare name that is *not* in that schema — a module constant, `None`, a free
function call, anything unrelated to the source Component — is left as ordinary Rust, never rewritten;
an earlier revision fell back to the owner for *any* unshadowed bare name regardless of whether it was
actually a source-Component field at all, which silently miscompiled such names into bogus
`__view_owner` getter calls. Assignment to a bare name that is a known-*writable* source-Component
field (`Prop`/`State` only — `ImplicitOwnerDef::writable_fields`) is likewise routed through that
owner's own generated `set_<name>` setter (`resolved_implicit_owner_setter`), so `selected = true;`
inside a popup event closure actually mutates the enclosing Component's own state, not a no-op. Since
raw Rust (unlike `view!`'s own attribute-value grammar) can contain arbitrary nested scopes, a real
lexical scope stack (not a single flat per-block set — an earlier revision's own bug, changing source
semantics for a block combining an outer-field read with a same-named local shadow) tracks `let`/`if
let`/`while let`/`match`/`for`/nested-closure bindings, so a local binding shadows the implicit owner
— for both reads and writes — only exactly where real Rust scoping would consider it in scope. This is
still "no second popup binding engine" in the sense the original design intended — it is the *same*
rewriter every `on_click` handler already went through, generalized (twice) rather than replaced by a
parallel mechanism built specifically for `context_popup`.

The same schema also covers *source-qualified* 2-segment paths (`vm.label`, `vm.save`) and direct
bare source-field reactivity, not just the 1-segment fallback above (PR #165 post-final rereview
remediation, A8/A9): `ImplicitOwnerDef::bindable_fields` lets `emit_path_get`/`emit_setter`/
`ViewClosureRewriter` bridge a `#[bindable]` owner through `__view_owner` instead of the nonexistent
`self.vm` an earlier revision emitted, and `ImplicitOwnerDef::reactive_fields` lets the dependency
scanners (`collect_view_expr_owner_properties`/`view_expr_has_reactive_dependency`/`view_expr_
depends_on`) recognize a direct bare source field as a real dependency of the hidden Component's
existing `__view_owner` subscription — before this, such a field could read correctly at popup-open
time but never live-update while the popup stayed open. A `#[bindable]` owner referenced this way
gets its own real `ObservableExt` subscription too (bridged through `__view_owner`, since the hidden
Component has no physical field to subscribe through directly), reusing the exact same resync-method
shape a physical bind owner already gets. See `docs/design/tools/codegen_design.md` §3.35 for the
full derivation and emission detail.

Only the small amount of new surface area needed to recognize a `view! { .. }` token sequence in
`context_popup` position, extract it into the hidden pair, and emit a `ViewFactory::new(..)` factory
that constructs a fresh instance of it per popup open is genuinely new. See
`docs/design/runtime/popup_context_menu_design.md`'s "Declarative `context_popup: view! { .. }` DSL"
subsection for the full three-part mechanism (lowering / weak-owner codegen / factory emission), and
`docs/design/tools/codegen_design.md` §3.35 for the lowering pass and the raw-block rewriter's own
lexical-scope-stack mechanism.

`context_popup` may still be authored via the low-level `ViewFactory::new(|ctx| ...)` API directly
when full manual control is wanted (see `crates/elwindui/tests/context_menu_and_popup.rs` for
examples) — the declarative sugar above compiles down to exactly that API.

## 4. Two distinct contracts — do not conflate them

This document's own scope (§§1–2) and #162's scope (§3) are both delivered, but they make different
guarantees, and the difference matters enough to state explicitly rather than leave implicit:

**The `ViewFactory` runtime contract (delivered, this document)** guarantees only:

- the factory closure is invoked at deferred build/open time, not at any earlier declaration time;
- `ViewBuildContext::owner` is supplied as `Weak<dyn UIElementExt>`;
- a popup-scoped, `derive()`-d `EnvironmentContext` is supplied;
- the owner may already be gone by build time, in which case `build` returns `None` — mechanically
  enforced by `ViewFactory::build` itself (`context.owner.upgrade()?` before the factory ever runs),
  not merely documented intent a factory could still bypass by never checking `ctx.owner`.

It does **not** guarantee that a hand-written `ViewFactory::new(|ctx| ...)` closure automatically
reads the owner's *current* field/state value — `ViewFactory::new` takes arbitrary Rust code, and a
caller can just as easily capture a stale value by mistake (e.g. `move |_ctx| { /* uses `selected_item`
captured by value before this closure was even stored */ }`) as read it correctly through `ctx.owner`.
The runtime type cannot enforce an authoring discipline it has no visibility into.

**The declarative `context_popup: view! { .. }` DSL contract (§3, #162)** additionally guarantees:

- bare identifiers referencing the *enclosing* Component's own fields/state are read fresh, at
  popup-open time, not snapshotted at any earlier point;
- the generated capture of that enclosing Component is `Weak`, matching this document's own
  no-strong-cycle rule;
- bindings/subscriptions the generated popup content creates belong to that popup instance's own
  lifetime, not the enclosing Component's.

`docs/specs/ui_spec.md`'s "owner の現在値を評価する" wording describes the second (declarative)
contract's behavior specifically — not a guarantee the first (low-level `ViewFactory`) contract
enforces mechanically, since a hand-written closure can always capture a stale value by mistake as
§4's first contract already notes.

## 5. `popup_dismiss` — a framework built-in Environment key

`PopupDismissAction`/`PopupDismissActionKey` (`crates/elwindui-core/src/ui/popup/mod.rs`) are
resolvable through the ordinary `#[environment(name)]` DSL field syntax under the fixed name
`popup_dismiss`, via `component_frontend::lookup_builtin_popup_dismiss_key` — the same
"framework-owned built-in key, no `#[elwindui::environment_key]` declaration needed" resolution path
the Semantic Style Brush keys (`primary`/`secondary`/.../`link`, `theme_environment_spec.md` §7) already
use, just with a different (non-`BrushStyle`) `Value` type:

```rust
#[environment(popup_dismiss)]
dismiss: Option<elwindui::core::ui::popup::PopupDismissAction>,
```

The framework key defaults to `None` (`PopupDismissActionKey::default_value()`).
`ContextMenuService::open_custom_popup` installs `Some(..)` into the derived popup-scoped
Environment (`docs/design/runtime/popup_context_menu_design.md` §6). The framework's DSL-managed
path does not install an active dismiss action outside popup scope; low-level typed Rust
(`EnvironmentContext::set::<PopupDismissActionKey>(..)`) remains capable of explicitly overriding
the public key, per `theme_environment_spec.md` §2 — this is not an absolute "always `None` outside
popup" runtime guarantee, only what the DSL-managed popup machinery itself does. This works today,
independent of #162 — see
`crates/elwindui/tests/context_menu_and_popup.rs`'s `popup_dismiss_environment_field_*` tests for a
Component using it end to end via the low-level `ViewFactory` API (§4's first contract).

## 6. What this revision changed at the `elwindui-core`/backend layer

Independent of the DSL question above, this revision:

- Replaced `PopupContentTemplate`/`PopupContentContext` with `ViewFactory`/`ViewBuildContext`
  (breaking rename — no compatibility shim was kept; `context_popup`'s prop type changed from
  `Option<PopupContentTemplate>` to `Option<ViewFactory>`).
- Made `ContextMenuService::open_custom_popup` take the owner element, derive a popup-scoped
  `EnvironmentContext`, install a `PopupDismissAction` (also resolvable declaratively, §5), and return
  `Option<Rc<dyn PopupSurfaceHandle>>` (`None` when the template declines to build).
- Connected AppKit's and WinUI3's popup close paths to `unmount_subtree` (`crate::ui::unmount_subtree`,
  PR #160's generic recursive Component/UIElement teardown). The portable, cross-backend guarantee is
  unmount-before-ElwindUI-host-tree-detach (`TreeHost::clear_tree()`), on every dismissal path. Each
  backend's *framework-initiated* close (AppKit's/WinUI3's `close()`, e.g. `PopupDismissAction`, item
  selection, explicit `PopupSurfaceHandle::close()`) additionally runs unmount before the native
  visibility/detach call it itself issues (`removeChildWindow`/`orderOut` on AppKit, `SetIsOpen(false)`
  on WinUI3) — preserving AppKit's PR #156 deferred-`clear_tree` reentrancy workaround unchanged.
  WinUI3 has one documented exception: its native `Popup.Closed` event (used for light-dismiss) fires
  only *after* WinUI has already changed `Popup.IsOpen`, so that specific native-originated path
  (`on_native_closed`, distinct from `close()`) cannot offer the stronger native-visibility ordering —
  only the portable host-tree-detach one (Issue #161 review finding; corrected after this document's
  own initial revision overstated the cross-backend guarantee). See `docs/design/runtime/
  popup_context_menu_design.md` §6/§7 for the full branch-by-branch sequence and
  `crates/elwindui-backend-appkit/src/inner/popup.rs`'s/`crates/elwindui-backend-winui3/src/inner/
  popup.rs`'s doc comments for why unmount-before-detach is safe even when close is reentrant from a
  popup-internal event handler — verified by `elwindui-core`'s
  `unmount_subtree_reentrant_from_within_own_event_dispatch_does_not_panic` and
  `unmount_hook_observes_intact_environment_before_backend_would_detach`.
- Made both backends' `InnerPopupSurface` release their own strong reference to the popup content
  root once `close()` completes (`content: RefCell<Option<Rc<dyn UIElementExt>>>`, taken — not a bare
  `Rc<dyn UIElementExt>` field), so a closed-but-still-reachable `PopupSurfaceHandle` (e.g. via a
  host's `active_popup` field before it's replaced/dropped) no longer keeps the entire
  already-unmounted popup subtree alive. Verified by `elwindui-core`'s
  `popup_surface_handle_releases_content_after_close_not_just_unmounted`.
- Gave `PopupDismissAction` a private `PopupDismissState` (`Building` / `Open(Weak<..>)` /
  `Dismissed`) inside `open_custom_popup`, so a dismiss request arriving during `ViewFactory::build`
  (before any native surface exists — including a generated Component's own `on_mount`, both the
  hand-authored `ViewFactory` case and, since #162, a lowered hidden Component's own `on_mount`)
  aborts the show entirely (`unmount_subtree`'d, never displayed) instead of being silently lost.
  Verified by `elwindui-core`'s `open_custom_popup_dismiss_during_build_prevents_the_popup_from_
  showing`; end to end with a real `#[elwindui::component]`, `elwindui`'s
  `popup_dismiss_during_on_mount_prevents_popup_from_showing` (low-level `ViewFactory` API) and
  `declarative_context_popup_dismiss_during_on_mount_prevents_popup_from_showing` (declarative
  `context_popup: view! { .. }`, #162).
- Made `PopupHost::show_popup` fallible (`-> Option<Rc<dyn PopupSurfaceHandle>>`, previously
  infallible) — WinUI3's `InnerPopupSurface::show` could already fail (coordinate conversion, `Popup`
  construction), but `WinUI3PopupHost::show_popup` previously papered over it with a handle wrapping a
  `None` inner surface, so Core believed the popup opened when nothing was shown. Backend show
  failure now unmounts the already-built content and returns `None` from
  `open_custom_popup`/`open_custom_menu`, rather than leaking a mounted-but-never-shown subtree.
  Verified by `elwindui-core`'s `open_custom_popup_unmounts_and_returns_none_when_backend_show_fails`
  and `open_custom_menu_unmounts_and_returns_none_when_backend_show_fails`.
- Made `ViewFactory::build` itself enforce owner liveness (`context.owner.upgrade()?` before
  invoking the factory) rather than merely documenting that a factory *should* check it — closing the
  gap between the documented runtime contract and what the type actually did. Verified by
  `elwindui-core`'s `build_returns_none_when_owner_dropped_and_never_invokes_the_factory`, which
  proves the factory itself is never called (stronger than a factory that voluntarily declines).
- Fixed `open_custom_popup`'s post-`show_popup` `PopupDismissState` transition: it previously
  assigned `Open(Weak::downgrade(handle))` unconditionally after `host.show_popup` returned, which
  silently overwrote a `Dismissed` state reached *during* that call (a backend's native "show" can
  itself dispatch synchronously/reentrantly). The transition out of `Building` is now atomic (one
  mutable borrow): `Building` → `Open`, or `Dismissed` stays `Dismissed` and the just-created handle
  is closed immediately rather than published. Verified by `elwindui-core`'s
  `open_custom_popup_dismiss_during_show_popup_is_not_lost_or_reopened`.
- Rewrote WinUI3's `InnerPopupSurface::show` to defer attaching `request.content`
  (`TreeHostPanel::set_tree`) until every fallible structural native setup step — coordinate
  conversion, `Popup::new()`, casts, every `FrameworkElement`/`Popup` property setter, `Closed`
  handler registration, and `SetIsOpen(true)` itself — has already succeeded, propagating every one
  of those failures as `None` instead of silently discarding them with `.ok()`. Previously
  `SetIsOpen(true)`'s own failure (and several earlier steps') was discarded, so a popup could fail to
  actually open while `show()` still returned `Some(surface)`. Also checks `is_open` immediately after
  `SetIsOpen(true)` and again after content attachment, to detect a synchronous native `Closed` event
  racing the open/attach sequence. Code-reviewed only — `elwindui-backend-winui3` is
  `#![cfg(target_os = "windows")]`-gated and cannot be compiled on this project's macOS development
  sandbox; see Issue #157 for the pending hardware verification.
- Split `component_frontend::lookup_environment_key` (read-only resolution — `#[environment(name)]`)
  from a new `lookup_writable_environment_key` (`EnvironmentScope`/`#[elwindui::theme]`), so the
  framework-installed `popup_dismiss` key is readable via `#[environment(popup_dismiss)]` but cannot
  be overwritten through either DSL write path — only `ContextMenuService::open_custom_popup` may
  set it. Semantic Style Brush keys remain writable through both resolvers; a same-crate user key
  named `popup_dismiss` still shadows the builtin and is fully writable, matching how any other
  builtin name is already shadowed. See `docs/specs/dsl_spec.md` §4/§13 (rules 34–36) and
  `docs/specs/theme_environment_spec.md` §2 for the normative contract. Verified by
  `elwindui-codegen`'s `popup_dismiss_resolves_for_read_but_not_for_write`,
  `semantic_style_builtin_key_resolves_for_both_read_and_write`,
  `rejects_theme_field_writing_the_popup_dismiss_builtin_key`, and
  `environment_scope_rejects_writing_the_popup_dismiss_builtin_key`.
