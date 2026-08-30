# ControlTemplate runtime and codegen design

Normative contract: [`../../specs/control_template_spec.md`](../../specs/control_template_spec.md)

Tracking: [#187](https://github.com/puchinya/elwindui/issues/187)

## 1. Runtime values and ownership

`ControlTemplate<C>` privately stores a deferred typed factory
`ControlTemplateContext<C> -> Rc<dyn UIElementExt>`. The target is statically
constrained by `C: ControlExt + 'static`; no target strings, `Any`, reflection,
or downcast are used. The context carries the strong target only during factory
execution and the effective `EnvironmentContext`.

`Control` owns one private `template_root`, a single-assignment typed provider,
and the private `Unapplied`/`Applying`/`Applied`/`Failed` application state.
Installation remains centralized in `__set_template_root()` and removes the
old Visual edge before attaching the new one. `__prepare_template_presentation()`
remains the virtual hook; the ContentControl override enables template mode and
preserves logical content. The provider captures only a `Weak<C>` and exposes
the Core-internal prepare, build, and post-application operations; it never
owns the target strongly.

## 2. Authoring pipeline

The component frontend recognizes two mutually exclusive pseudo-fields:

```text
body: view! { ... }
    -> ordinary component composition

template: template_view!(|alias: Self| { ... })
    -> typed ControlTemplate<Self> default factory
```

`template_view!` reuses the existing View AST/parser and enters the same
semantic planner/emitter used by ordinary `view!`. The shared lowerer owns
construction, metadata/property and content lowering, event/lifecycle wiring,
dynamic regions, ContentPresenter handling, ownership, and Environment
propagation. There is no recursive `TemplateBackend` compiler. Only the
template adapter differs: it acquires the typed parent, records the
`TemplateProperty`/`WritableTemplateProperty` capability bounds, wraps the
compiled body in the deferred factory, and performs template-root replacement.
Standalone expressions declare their target in the same lambda header:
`template_view!(|alias: ConcreteTarget| { ... })`. Component defaults use
`Self` in that position. A reusable template is an ordinary Rust function
returning `ControlTemplate<ConcreteTarget>`; there is no separate named-template
frontend or marker attribute. All forms produce the same deferred factory
semantics and do not infer the target from Rust's expected type.

The template body uses the declared parent alias. Its property reads are
statically keyed `TemplateProperty` bounds on the explicit target; updates use
the same subscription/resync contract as component defaults and standalone
templates. Write sites add the stronger `WritableTemplateProperty` bound, so a
read-only target fails at compile time. Dynamic `if`/`match`/supported `for`
regions, root replacement, ContentPresenter validation, lifecycle hooks, and
nested component mounting are not separate runtime features of the standalone
form.

`on_update(field, ...)` keeps the existing unqualified selector list: `field` is
the property name such as `label`, not a parent-alias path such as
`alias.label`. Alias-qualified paths remain the syntax for reads, setters, and
bindings in the template body.

The target bound remains `C: ControlExt + 'static` for any valid non-
`NativeControl` target. A property-free template can therefore target a raw
framework/class-managed `Control` or `ContentControl`. A typed
`<alias>.<property>` path additionally requires the matching
`TemplateProperty<KEY>`, while `<alias>.set_<property>(...)` and two-way
bindings require `WritableTemplateProperty<KEY>`. Generated
Control-derived `#[component]` types provide these bridges from effective
property metadata; raw framework/class-managed `ControlExt` types are not
required to provide them in #187. No fake non-reactive bridge, runtime
reflection, or class-wide reactive-property redesign is introduced here.

The generated and runtime flows are:

```text
component parser
  template pseudo-field
      -> shared semantic lowerer/planner/emitter
      -> ControlTemplate<Self> default factory

mount
  effective Environment
      -> install provider specialized to the final concrete component type
      -> target wiring and on_mount
      -> template remains Unapplied

first explicit apply_template() or participating measure()
  Control state machine
      -> provider.prepare()
      -> exact typed Environment lookup at application time
      -> selected factory (or default)
      -> provider.build(environment)
      -> __set_template_root(root)
      -> mount/wire subtree
      -> provider.on_applied() / on_apply_template
```

The default factory is not executed if an Environment override supplies
`Some`. A cheap factory value may be stored in generated type metadata, but
template nodes, bindings, dynamic regions, and subscriptions are created only
by the selected factory. The generated mount path installs the provider; it
does not select, build, or attach the template root.

## 3. Environment selection

`EnvironmentContext` stores a generic `ControlTemplateEnvironment<C>` entry
whose value is `Option<ControlTemplate<C>>`. The key is distinguished by the
typed `C`/`TypeId`, so exact-type lookup has no base-type fallback or covariance.
`set_control_template::<C>(None)` is a real local entry and shadows an ancestor
`Some`; it selects the component default without removing the local cell.

Selection occurs once at the first successful application after mount. The
provider performs the exact `C` lookup in Core generic code, preserving local
`None` shadowing and the existing `Environment::derive()` inheritance. The
template slot is not subscribed for runtime replacement. An Environment change
before first application can affect selection; a change after application
cannot replace the committed root or rerun the factory.

## 4. ContentControl presentation

Raw ContentControl uses direct presentation: logical content is also a Visual
child. Template mode removes that direct edge while preserving the logical
parent. `ContentPresenter` subscribes to the inherited content change surface,
owns the Visual edge, and never changes logical ownership. The existing
validator allows zero or one static presenter and rejects multiple or dynamic
presenters. Content replacement and pre-mount transitions remain handled by the
existing weak/cancelable subscription and template-root paths.

`__prepare_template_presentation()` runs before provider factory build and
wiring so that logical content is no longer directly presented before a
`ContentPresenter` can acquire the Visual edge. Root attachment commits before
`on_apply_template`; descendant mount hooks triggered by attachment therefore
complete before that ordinary overridable hook runs.

When the target is nested in a Window host-composition body, the Window stays
unmounted until its first `show()`, and its generated named child accessors are
not readable during that interval. A demo or application that needs logical
content pre-mount passes it through the initial `content:`/Param construction
path; the target receives it before this template's provider installation and first application. This
preserves the lifecycle boundary instead of adding a special pre-mount
template-accessor path.

For a generated component whose scalar content slot is supplied by an external
base shape, codegen forwards a named `content:` value through that shape's
exported setter protocol before the generated component mounts. The slot is not
duplicated into local component metadata, and the defining shape performs the
single concrete-to-trait conversion.

## 5. Validation boundaries

Frontend validation rejects body/template coexistence, template on a non-
Control or NativeControl target, `Self` in a standalone template, legacy
`#[component(template = key)]`, and invalid ContentPresenter placement.
Control-derived `body` declarations get a migration diagnostic. Cross-crate
trait and getter resolution remains a normal generated-Rust/rustc constraint;
no type-name dispatch is introduced.

## 6. Removed protocol

`component_body_presentation` is deleted rather than retained as a dormant
compatibility layer. The class macro emits no presentation query arm, and
generated props metadata contains no body-presentation mode. Any forwarding
macro infrastructure added solely for that protocol is removed or moved to a
separate issue if an independent generic use is proven.

## 7. Lifetime and dynamic content

Template instances use the existing weak-owner dependency and property
resynchronization machinery. The declared parent alias followed by `.foo` is a
typed getter with the same notification wiring as ordinary view bindings; the corresponding
`WritableTemplateProperty<KEY>` setter capability is emitted only when the
effective property has a real setter. Dynamic `if`/`match` and
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

For a generated component that directly declares a `Vec<Rc<T>>` content field,
the generated `DynamicChildHost<T>` mutates storage only while the slot is
reconciling, then calls the host's single commit hook after all slot borrows are
released. The hook shares the generated content setter's post-mutation helper:
dependent computed fields are recomputed and the content property is published
once. An unchanged concrete sequence does not publish. A derived component
that only inherits such a field has no generated forwarding host in merged PR #192;
that inherited shape is intentionally outside this implementation boundary and
is tracked in [follow-up Issue #194](https://github.com/puchinya/elwindui/issues/194).
It must not be made to work with a fake notification bridge.
