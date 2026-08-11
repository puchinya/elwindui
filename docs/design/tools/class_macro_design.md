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

## Validation boundary

Publicly observable accepted/rejected forms remain in the specification. Parser representation, token generation, registry layout, shadow expansion, and debugging strategy belong only here.
