# `#[class]` macro implementation design

Related specification: [`../../specs/macro_class_spec.md`](../../specs/macro_class_spec.md). Implementation invariants for agents are in [`../../agents/class-model.md`](../../agents/class-model.md).

## Expansion model

The macro parses the class declaration and paired implementation, validates the public class contract, then emits the concrete storage type, extension trait implementation, ancestor forwarding machinery, constructors, and generated accessors required by that contract.

Ordinary and root classes generate their own extension trait surface. `trait_only` declares an interface without concrete storage. `struct_only` implements an existing trait for concrete storage and therefore cannot add the same per-class forwarding surface as an ordinary class.

## Ancestor forwarding

Generated `__elwindui_inherit_*!` macros carry the ancestor chain across module and crate boundaries. `#[overridable]` emits dynamic accessors used by forwarding; `#[overrides]` routes through the subclass implementation. Sealed members omit the override route.

Generated macros use fully qualified paths. `$crate` is retained where it must refer to the declaring crate; tokens that intentionally refer to the consuming crate are rewritten during expansion.

## Construction

`construct` is the single source for constructor arguments. Expansion generates `new`, initializes the weak self handle after allocation, and invokes `on_constructed` only after the object can be safely upgraded through that handle.

Hand-written `new` is rejected because it would compete with the generated ownership sequence.

## Registry and analysis

Cross-item class information is stored in a compilation-scoped registry keyed so separate consuming crates cannot contaminate one another. Source-order dependencies are minimized, but rust-analyzer may expand incomplete files in an order unavailable to normal rustc.

The rust-analyzer shadow expansion supplies enough generated structure for name resolution without changing rustc output. Spans from user declarations are preserved on generated public items and forwarding calls where possible so diagnostics and breakpoints point back to source.

## Shape macro protocol (`__elwindui_props_*!`)

Every class also emits a `__elwindui_props_{Name}!` declarative macro carrying its DSL-visible property surface across the same crate boundary `__elwindui_inherit_*!` carries the ancestor chain across (`elwindui-codegen` never has a local `TypeInfo` for a real builtin — only a same-crate DSL fixture in tests does — so it constructs against this macro instead of a registry entry). Each query is a two-hop `@x`/`@x_from` pair: the entry seeds `$origin` with this class's own name and hands off to the `_from` form, which either matches a literal per-property arm or forwards unmatched names to the parent's own macro, unchanged, so `$origin` still names the type the use site actually wrote by the time a terminal `compile_error!` or catch-all is reached.

- `@set`/`@set_from` — assigns a plain (non-attached, non-collection-content) property, via `wrap_prop_value`'s per-property conversion (a `String` gets `&`, `Brush`/`Color` gets `.into()` plus a declared-type-driven `Some(..)`, a plain type passes through unchanged).
- `@clear`/`@clear_from` — resets a property to its platform default, the `ThemeValue::PlatformDefault` half of `@set`'s `ThemeValue::Value` half.
- `@routed`/`@routed_from` — registers a `#[routed]` callback, building the bubbling adapter around a bare DSL-supplied closure.
- `@attached_set` — a `#[attached]` property's setter, one hop only (no ancestor forwarding — the DSL's `Owner::field` syntax always names the owning class explicitly).
- `@children`/`@children_into` — attaches bare nested child elements to whichever property `#[content(..)]` names, forwarding up the chain until the declaring class is found.
- `@field_type`/`@field_type_from` — expands, in **type position**, to the real Rust type a declared property was given (`elwindui-codegen`'s `resolve_effective_fields`/`synthesize_external_base_fields`, Refs #90: a consumer component that bare-forwards an inherited attribute value from a genuinely external base — `padding: padding` on `#[elwindui::component(inherits Control)]`, dsl_spec.md §3's `ContentControl` pattern — has no local `TypeInfo` to read `padding`'s declared type from, so the synthesized field's own `FieldDef::ty` is instead the literal text `elwindui::core::__elwindui_props_Control!(@field_type padding)`; `generate_view`'s `syn::parse_str::<syn::Type>` already parses an arbitrary type-position macro invocation as an ordinary `syn::Type::Macro`, so this needs no special casing downstream). Same per-property arm/forwarding/terminal-`compile_error!` shape as `@set`, restricted to the same `takes_set_arm`-eligible property set (a routed/attached/collection-content property has no single settable "value type" a struct field could hold).
- `@content_item_dyn`/`@content_field_get` — cross-crate queries backing dynamic (`for`/`if`/`match`) region reconciliation against a content-collection field with no local `TypeInfo`.
- `@assert_undeclared`/`@assert_declared` — compile-time collision/designation probes, emitted in item position next to the class declaration rather than consulted from a use site.

Only `@set`'s `wrap_prop_value` knows how to shape a value into a declared property's real setter; a bare-forwarded field whose own value already carries that exact shape (rather than the bare/literal shape every ordinary call site supplies) is `elwindui-codegen`'s own responsibility to normalize before calling `@set` — see `emit_resync`/`emit_external_attribute_sets`/`build_component_args`'s own `ty.contains('!')` branches in `codegen.rs`.

## Validation boundary

Publicly observable accepted/rejected forms remain in the specification. Parser representation, token generation, registry layout, shadow expansion, and debugging strategy belong only here.
