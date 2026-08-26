# ControlTemplate runtime and codegen design

Normative contract: [`../../specs/control_template_spec.md`](../../specs/control_template_spec.md)

Tracking: [#187](https://github.com/puchinya/elwindui/issues/187)

## 1. Runtime values and ownership

`ControlTemplate<C>` privately stores a deferred typed factory
`ControlTemplateContext<C> -> Rc<dyn UIElementExt>`. The target is statically
constrained by `C: ControlExt + 'static`; no target strings, `Any`, reflection,
or downcast are used. The context carries the strong target only during factory
execution and the effective `EnvironmentContext`.

`Control` owns one private `template_root`. Installation remains centralized in
`__set_template_root()` and removes the old Visual edge before attaching the
new one. `__prepare_template_presentation()` remains the virtual hook; the
ContentControl override enables template mode and preserves logical content.

## 2. Authoring pipeline

The component frontend recognizes two mutually exclusive pseudo-fields:

```text
body: view! { ... }
    -> ordinary component composition

template: template_view! { ... }
    -> typed ControlTemplate<Self> default factory
```

`template_view!` reuses the existing View AST/parser and enters the single
semantic template backend used by every ControlTemplate source. The backend
owns construction, metadata/property and content lowering, event/lifecycle
wiring, dynamic regions, ContentPresenter handling, ownership, and Environment
propagation. Only the frontend context and output wrapper differ: standalone
expressions acquire a typed target from Rust's expected type, while component
defaults and named templates already have a concrete target; all produce the
same deferred factory semantics. `#[control_template(target = T)]` remains a
thin named-template frontend over that backend.

The standalone expression frontend acquires its `ControlTemplate<C>` target
from Rust's expected type and then enters that same template compilation
context. Its property reads are statically keyed `TemplateProperty` bounds on
`C`; updates use the same subscription/resync contract as component and named
templates. Dynamic `if`/`match`/supported `for` regions, root replacement,
ContentPresenter validation, lifecycle hooks, and nested component mounting are
not separate runtime features of the standalone form.

The generated flow is:

```text
component parser
  template pseudo-field
      -> shared template compiler
      -> ControlTemplate<Self> default factory

mount
  effective Environment
      -> typed ControlTemplate<Self> lookup
      -> selected factory (or default)
      -> __prepare_template_presentation()
      -> __set_template_root(root)
      -> mount/wire subtree
```

The default factory is not executed if an Environment override supplies
`Some`. A cheap factory value may be stored in generated type metadata, but
template nodes, bindings, dynamic regions, and subscriptions are created only
by the selected factory.

## 3. Environment selection

`EnvironmentContext` stores a generic `ControlTemplateEnvironment<C>` entry
whose value is `Option<ControlTemplate<C>>`. The key is distinguished by the
typed `C`/`TypeId`, so exact-type lookup has no base-type fallback or covariance.
`set_control_template::<C>(None)` is a real local entry and shadows an ancestor
`Some`; it selects the component default without removing the local cell.

Selection is mount-time only. The template slot is not subscribed for runtime
replacement. Child contexts reuse the existing Environment `derive()` cell
inheritance and shadowing semantics.

## 4. ContentControl presentation

Raw ContentControl uses direct presentation: logical content is also a Visual
child. Template mode removes that direct edge while preserving the logical
parent. `ContentPresenter` subscribes to the inherited content change surface,
owns the Visual edge, and never changes logical ownership. The existing
validator allows zero or one static presenter and rejects multiple or dynamic
presenters. Content replacement and pre-mount transitions remain handled by the
existing weak/cancelable subscription and template-root paths.

## 5. Validation boundaries

Frontend validation rejects body/template coexistence, template on a non-
Control or NativeControl target, legacy `#[component(template = key)]`, and
invalid ContentPresenter placement. Control-derived `body` declarations get a
migration diagnostic. Cross-crate trait and getter resolution remains a normal
generated-Rust/rustc constraint; no type-name dispatch is introduced.

## 6. Removed protocol

`component_body_presentation` is deleted rather than retained as a dormant
compatibility layer. The class macro emits no presentation query arm, and
generated props metadata contains no body-presentation mode. Any forwarding
macro infrastructure added solely for that protocol is removed or moved to a
separate issue if an independent generic use is proven.

## 7. Lifetime and dynamic content

Template instances use the existing weak-owner dependency and property
resynchronization machinery. `templated_parent.foo` is a typed getter with the
same notification wiring as ordinary view bindings. Dynamic `if`/`match` and
supported `for` subtrees use the established ControlTemplate reconciliation;
ContentPresenter remains forbidden in dynamic regions. There is one active
template root and no runtime re-template operation. Every generated descendant
is mounted with the `ControlTemplateContext.environment` passed to the
selected factory rather than an ambient application context.

The dynamic-child host is resolved from the host's effective `#[content(...)]`
metadata and field shape. Scalar content materializes exactly one active branch
and replaces it through the effective setter. Collection content uses the
effective collection getter with `DynamicChildSlot`; this applies equally to
`Layout` and non-`Layout` collection hosts, including external shapes selected
through exported props metadata. No dynamic template path assumes
`LayoutExt::children` merely because a node is nested in a template.
