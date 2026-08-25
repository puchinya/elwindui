# ControlTemplate specification

Tracking: [#187](https://github.com/puchinya/elwindui/issues/187)

## 1. Scope

`ControlTemplate<C>` is a typed, deferred factory for the visual subtree of a
`Control`-derived value. `NativeControl` is excluded because its backend owns
the native visual structure. Selection is mount-time only; a mounted control is
not re-templated when an Environment value changes.

## 2. Typed authoring

```rust
let template: ControlTemplate<MyButton> = template_view! {
    Grid {
        TextBlock { text: templated_parent.label }
    }
};
```

`template_view!` is an expression-producing macro returning
`ControlTemplate<C>`. `templated_parent` is the typed target from
`ControlTemplateContext<C>` and uses the ordinary typed getter/event wiring
system. Component defaults, named `#[control_template]` templates, and
standalone expressions share the same parser, validator, dynamic-region
lowering, ContentPresenter restrictions, and Environment propagation. A
completely unconstrained expression may require a Rust type annotation.

Inside a `#[component]` declaration, the reserved pseudo-field
`template: template_view! { ... }` declares the component type's default
`ControlTemplate<Self>`. It is not a `#[prop]`, instance field, observable
property, or runtime setter.

```rust
#[elwindui::component(inherits ContentControl)]
struct CustomButton {
    #[prop(default = String::new())]
    label: String,

    template: template_view! {
        Border {
            TextBlock { text: templated_parent.label }
        }
    },
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

## 7. Named templates

`#[elwindui::control_template(target = T)]` remains a named reusable-template
convenience API. Its declaration uses the same `template_view!` parser,
validator, factory generation, and `templated_parent` semantics, and exposes
`Name::template() -> ControlTemplate<T>`.

## 8. Removed and forbidden forms

- `#[component_body_presentation]` and `@component_body_presentation`;
- per-control Environment Keys used only for template selection;
- `#[component(template = key)]`;
- instance-level template properties or runtime re-template;
- base-type fallback/covariance;
- TemplatePart, VisualStateManager, triggers, styles, reflection, string
  lookup, `Any` target erasure, and NativeControl templates.

Legacy `#[component(template = key)]` produces a migration diagnostic directing
authors to `template: template_view! { ... }` and
`EnvironmentContext::set_control_template::<Target>(...)`.
