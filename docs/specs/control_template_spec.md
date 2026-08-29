# ControlTemplate specification

Tracking: [#187](https://github.com/puchinya/elwindui/issues/187)

## 1. Scope

`ControlTemplate<C>` is a typed, deferred factory for the visual subtree of a
`Control`-derived value. `NativeControl` is excluded because its backend owns
the native visual structure. Selection is mount-time only; a mounted control is
not re-templated when an Environment value changes.

## 2. Typed authoring

```rust
let template: ControlTemplate<MyButton> = template_view!(|button: MyButton| {
    Grid {
        TextBlock { text: button.label }
    }
});
```

`template_view!` is an expression-producing macro returning
`ControlTemplate<T>`. Its header is the single source of truth for both the
parent alias used in the body and the exact target type `T`; there is no target
inference from an expected result type. The alias uses the ordinary typed
getter/event wiring when the corresponding compile-time capability is present.
Component defaults and standalone expressions enter one shared semantic
lowering path. A reusable template is an ordinary Rust function returning the
appropriate `ControlTemplate<T>`, so it uses that same standalone form.
Ordinary `view!` and both template frontends use the same planner/emitter for
element construction,
metadata/property and content lowering, event and lifecycle wiring,
dynamic-region reconciliation, ContentPresenter restrictions, ownership, and
Environment propagation. Only typed-parent acquisition, template-property
capability bridging, factory wrapping, and template-root replacement are
template-specific.

### 2.1 Read and write capabilities

Template parent property access is split into two compile-time capabilities:

- `TemplateProperty<KEY>` provides a cloned getter and change subscription;
- `WritableTemplateProperty<KEY>` extends it with a typed setter.

Generated code implements the writable capability only for effective `#[prop]`
and `#[state]` fields that have a real setter. Computed, environment,
read-only, derived, and otherwise non-settable fields remain readable but do
not implement `WritableTemplateProperty<KEY>`. A template `<=>` binding or
`button.set_<field>(...)` therefore fails during Rust trait
resolution; it cannot become a runtime panic or silent no-op. Inherited
writable fields delegate through the composed base's existing typed setter,
without duplicating storage.

`KEY` is a compile-time 64-bit FNV-1a-style token derived from the field-name
literal. It is not a runtime registry or string lookup. A collision is an
explicit compile-time duplicate-implementation/associated-type error and is
never resolved silently.

### 2.2 Target capability boundary

`ControlTemplate<C>` accepts any valid non-`NativeControl` target satisfying
`C: ControlExt + 'static`. A property-free `template_view!` therefore works for
raw framework or class-managed targets such as `Control` and `ContentControl`.

Typed template-parent property paths are capability-gated:

- `<alias>.<property>` requires `TemplateProperty<KEY>`;
- `<alias>.set_<property>(...)` and two-way bindings require
  `WritableTemplateProperty<KEY>`.

Generated Control-derived `#[component]` types export these capabilities from
their effective property metadata. Raw framework/class-managed `ControlExt`
types are not required by this contract to export a template-property bridge;
the absence of that capability is a compile-time boundary, not a runtime
reflection or string-lookup fallback. PR #187 does not add class-wide reactive
property notifications or fake no-op subscriptions/setters.

Inside a `#[component]` declaration, the reserved pseudo-field
`template: template_view!(|alias: Self| { ... })` declares the component
type's default `ControlTemplate<Self>`. Component defaults must use `Self` as
their target; the alias is chosen explicitly by the author. It is not a
`#[prop]`, instance field, observable property, or runtime setter.

```rust
#[elwindui::component(inherits ContentControl)]
struct CustomButton {
    #[prop(default = String::new())]
    label: String,

    template: template_view!(|button: Self| {
        Border {
            TextBlock { text: button.label }
        }
    }),
}
```

`body: view!` remains ordinary component composition. A component may declare
only one authored presentation slot: `body` or `template`, never both.
Control-derived authored chrome must use `template`; using `body` on a
Control-derived component is rejected with migration guidance. Layout-derived
ordinary components continue to use `body`.

## 3. Content independence

The template declaration and caller content are independent:

```rust
CustomButton {
    label: "OK"
    TextBlock { text: "logical content" }
}
```

The template subtree is the visual root. The bare caller child is lowered by
the effective `#[content(...)]` metadata and, for `ContentControl`, becomes its
single logical `content`. It never becomes the template root and is never
lowered through the template declaration.

Dynamic template children use the same effective content-shape rule. A scalar
content destination requires one active element and replaces that value through
its content setter; a collection destination is reconciled through the
effective collection surface and `DynamicChildSlot`. `Layout` is only one
collection-content implementation, not a special template host category, and
non-`Layout` controls may provide either shape through their `#[content(...)]`
metadata.

For a generated component that directly declares a `Vec<Rc<T>>` content field,
the generated dynamic-child host batches the raw collection operations and
publishes one completed `children` property change after the slot is coherent.
That notification follows the same computed-property, observer, and template
resynchronization path as the component's normal setter. Inherited generated
`Vec<Rc<T>>` content does not acquire a forwarding host in merged PR #192; dynamic
reconciliation for that shape is a compile-time boundary and does not use a
fake setter or no-op subscription. Framework-class reactive collection support
is deferred to [follow-up Issue #194](https://github.com/puchinya/elwindui/issues/194).

## 4. Core API and Environment lookup

The existing typed values remain:

```rust
pub struct ControlTemplate<C: ControlExt + 'static> { /* private factory */ }

pub struct ControlTemplateContext<C: ControlExt + 'static> {
    pub control: Rc<C>,
    pub environment: EnvironmentContext,
}
```

The framework provides a generic typed Environment slot equivalent to
`ControlTemplateEnvironment<C>` with value
`Option<ControlTemplate<C>>`, keyed by `TypeId` and not by a string.

```rust
impl EnvironmentContext {
    pub fn set_control_template<C: ControlExt + 'static>(
        &self,
        template: Option<ControlTemplate<C>>,
    );

    #[doc(hidden)]
    pub fn __control_template<C: ControlExt + 'static>(
        &self,
    ) -> Option<ControlTemplate<C>>;
}
```

`Some` overrides the component default at that exact context. `None` is an
explicit entry that shadows an ancestor value and selects the component
default; it does not remove the local entry. Lookup is exact target type:
`ControlTemplate<Base>` does not satisfy `ControlTemplate<Derived>`.

Selection order is:

1. effective Environment `ControlTemplateEnvironment<Self>` containing `Some`;
2. otherwise the component-declared default template.

The selected factory receives the effective Environment. If an Environment
override wins, the default factory is not executed.

## 5. Lifecycle and ownership

Mounting establishes the effective Environment before template selection. The
existing lifecycle is retained:

```text
logical construction
 -> effective Environment
 -> __prepare_template_presentation()
 -> select Environment template or component default
 -> build ControlTemplateContext<Self>
 -> __set_template_root(root)
 -> mount/wire template subtree
 -> target wiring and on_mount
```

`Control` retains one private template-root store and
`ContentControl::__prepare_template_presentation()` remains the virtual hook.
No second root store or hidden body-presentation protocol exists.

## 6. ContentControl and ContentPresenter

Raw `ContentControl` direct mode remains compatible: logical content is also a
direct Visual child. Template mode removes that direct Visual edge while
preserving logical ownership. A template may contain zero or one static
`ContentPresenter`:

- zero: logical content remains owned by the target but is not displayed;
- one: the presenter owns the Visual edge while the logical parent remains the
  target.

Multiple presenters and presenters inside `if`/`match`/`for` regions are
compile-time errors. Content replacement detaches the old logical/Visual edge
before attaching the new one, and pre-mount content survives the transition.

## 7. Reusable templates

Named reusable templates are ordinary Rust functions. There is no dedicated
template declaration attribute or marker struct API:

```rust
pub fn compact_button_template() -> ControlTemplate<CustomButton> {
    template_view!(|button: CustomButton| {
        Border {
            TextBlock { text: button.label }
        }
    })
}
```

The function may be called wherever a `ControlTemplate<CustomButton>` value is
needed, including `EnvironmentContext::set_control_template`.

## 8. Removed and forbidden forms

- `#[component_body_presentation]` and `@component_body_presentation`;
- per-control Environment Keys used only for template selection;
- `#[component(template = key)]`;
- instance-level template properties or runtime re-template;
- base-type fallback/covariance;
- TemplatePart, VisualStateManager, triggers, styles, reflection, string
  lookup, `Any` target erasure, and NativeControl templates.

Legacy `#[component(template = key)]` produces a migration diagnostic directing
authors to `template: template_view!(|alias: Self| { ... })` and
`EnvironmentContext::set_control_template::<Target>(...)`. A standalone
template must use a concrete target in the same header; `Self` is invalid
outside a component default.
