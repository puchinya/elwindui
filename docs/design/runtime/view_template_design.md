# ViewTemplate: generic deferred View factory

Normative contract: [`../../specs/ui_spec.md`](../../specs/ui_spec.md) (`context_popup`)

Tracking: [#161](https://github.com/puchinya/elwindui/issues/161)

## 1. Relationship to `ControlTemplate<C>`

`ViewTemplate` and `ControlTemplate<C>` (`docs/design/runtime/control_template_design.md`) are
**separate public types** — neither an alias nor a shared public supertype of the other, and this is
deliberate, not an oversight to unify later.

`ControlTemplate<C>` carries Control-specific semantics that `ViewTemplate` does not share:
`templated_parent` (a strongly-typed `Weak<C>`), the `ContentPresenter`/logical-content vs.
template-visual split, and template selection fixed for the owning Control's entire mount lifetime
(selected once, in `mount()`, never re-evaluated). `ViewTemplate` has none of these — it exists for
content whose lifecycle is independent of any single owner's mount lifetime and may be built again on
each new demand (a popup opened a second time gets a second, unrelated `ViewTemplate::build` call, not
a re-selection of the same template instance).

What the two types share is only the shape of their private storage:

```rust
struct DeferredViewFactory<C> {
    factory: Rc<dyn Fn(C) -> Option<Rc<dyn UIElementExt>>>,
}
```

(`crates/elwindui-core/src/ui/view_template.rs`, `pub(crate)`, not exported). `ControlTemplate<C>`
wraps `DeferredViewFactory<ControlTemplateContext<C>>` and unwraps the `Option` in `__build` (its own
contract guarantees a factory always produces a root — building is infallible from a caller's
perspective, since `ControlTemplateContext<C>::control: Rc<C>` is always alive during a build).
`ViewTemplate` wraps `DeferredViewFactory<ViewBuildContext>` and stays `Option`-returning end to end,
since its `ViewBuildContext::owner: Weak<dyn UIElementExt>` may have already been dropped by build
time.

## 2. `ViewTemplate` / `ViewBuildContext`

```rust
pub struct ViewBuildContext {
    pub owner: Weak<dyn UIElementExt>,
    pub environment: EnvironmentContext,
}

pub struct ViewTemplate {
    factory: DeferredViewFactory<ViewBuildContext>,
}

impl ViewTemplate {
    pub fn new(factory: impl Fn(ViewBuildContext) -> Option<Rc<dyn UIElementExt>> + 'static) -> Self;
    pub fn build(&self, context: ViewBuildContext) -> Option<Rc<dyn UIElementExt>>;
}
```

`ViewBuildContext` carries no popup-specific field (no dismiss handle) — a popup-specific value
(`PopupDismissAction`) is threaded through the popup-scoped `EnvironmentContext` instead
(`docs/design/runtime/popup_context_menu_design.md` §6), never through `ViewBuildContext` itself, so
that `ViewTemplate` stays a genuinely general primitive: today `context_popup`
(`crate::ui::element::UIElement::context_popup`), potentially in the future lazy tab content,
dialogs, sheets, or popovers — none of which is implemented against `ViewTemplate` yet, but the type
carries no popup-only shape that would need to be generalized later.

`owner` is always `Weak`: a `ViewTemplate` value is typically stored as a property of the very element
it is a deferred view *for* (e.g. `context_popup` on the element that opens the popup), so a factory
that captured its owner strongly would create an ownership cycle through that property. `build`
upgrading a dead `owner` and returning `None` (rather than panicking or falling back to some default)
is the documented, tested contract for "the owner went away between template capture and build time."

## 3. Declarative `context_popup: view! { .. }` — not yet implemented

The durable architecture for compiling `context_popup: view! { .. }` (the same `view!` grammar as an
ordinary Component body, deferred to popup-open time, reusing `view!`'s existing parser/AST/codegen
construction pipeline rather than a separate DSL) is **not implemented as of this design revision**.

The investigation for Issue #161 identified the central open design question precisely: unqualified
identifiers written inside `context_popup: view! { .. }` (e.g. `item: selected_item`, referencing the
*enclosing* Component's own field) need to resolve exactly the way any other bare name inside an
ordinary `view!` body already does — through the same `self`/`vm` accessor-rewriting `emit_closure_value`
already performs for `on_click`-style event-handler closures and `ClosureBody::Element`-shaped values
(`render_content: |doc| DocumentView { doc: doc }`) — rather than through `ControlTemplate`'s
`templated_parent.foo`-style *explicit*-qualification convention, which requires an explicit prefix and
would not match the declarative examples this Issue's directive specifies. Implementing this correctly
means extending the closure/weak-self-capture machinery already used for event handlers to a
multi-statement, `on_mount`/`lets`/`if`/`match`/`for`-capable body shape (not just a single
`ClosureBody::Element` construction) — substantial, delicate `elwindui-codegen` work spanning
`parser.rs` (recognizing a nested `view! { .. }` token sequence as an attribute value), `ast.rs` (a new
`ViewExpr` variant), and every one of `codegen.rs`'s ~20 exhaustive `ViewExpr`/`ClosureBody` match
sites.

Until that lands, `context_popup` is authored via the low-level `ViewTemplate::new(|ctx| ...)` API
directly (see `crates/elwindui/tests/context_menu_and_popup.rs` for examples), exactly as
`PopupContentTemplate::new(|ctx| ...)` was authored before this revision.

## 4. What this revision changed at the `elwindui-core`/backend layer

Independent of the DSL question above, this revision:

- Replaced `PopupContentTemplate`/`PopupContentContext` with `ViewTemplate`/`ViewBuildContext`
  (breaking rename — no compatibility shim was kept; `context_popup`'s prop type changed from
  `Option<PopupContentTemplate>` to `Option<ViewTemplate>`).
- Made `ContextMenuService::open_custom_popup` take the owner element, derive a popup-scoped
  `EnvironmentContext`, install a `PopupDismissAction`, and return `Option<Rc<dyn PopupSurfaceHandle>>`
  (`None` when the template declines to build).
- Connected AppKit's and WinUI3's popup `close()` to `unmount_subtree` (`crate::ui::unmount_subtree`,
  PR #160's generic recursive Component/UIElement teardown), run synchronously before each backend's
  native detach (teardown-before-detach), preserving AppKit's PR #156 deferred-`clear_tree` reentrancy
  workaround unchanged — see `docs/design/runtime/popup_context_menu_design.md` §6 for the full
  sequence and `crates/elwindui-backend-appkit/src/inner/popup.rs`'s `close()` doc comment for why the
  two can be split (unmount_subtree touches only the `UIElementExt` tree's own state, never
  `TreeHostView`'s `RefCell`s, so it's safe to run synchronously even when `close()` is reentrant from
  a popup-internal event handler — verified by
  `elwindui-core`'s `unmount_subtree_reentrant_from_within_own_event_dispatch_does_not_panic`).
