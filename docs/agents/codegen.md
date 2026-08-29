# Codegen & DSL Agent Guidelines

Guidelines for AI agents modifying `elwindui-codegen`, proc-macros (`#[component]`, `#[viewmodel]`, `#[dsl_enum]`), or DSL compiler logic.

## Specification Source of Truth

- **Normative specification**: [`docs/specs/dsl_spec.md`](../specs/dsl_spec.md).
- **Compiler architecture**: [`docs/design/tools/codegen_design.md`](../design/tools/codegen_design.md).
- **Core language & scoping**: `dsl_spec.md` §1–§12.
- **Static verification rules**: `dsl_spec.md` §13 (Rule 1 through Rule 38). Read the specific rule number in `dsl_spec.md` when implementing or modifying compile-time checks.

## Implementation Invariants to Preserve

- **`param` vs `prop`**: `#[param]` without an initializer is a required named construction input; `#[param(default = expr)]` is optional but remains fixed after construction. Both use only static-evaluable expressions (literals, other params, pure builtins, `env::*`, `once!` values) — never reactive prop references or impure calls. Ordinary `#[prop]` fields are always runtime-mutable and are never promoted to constructor inputs; an omitted initializer is normalized to `Default::default()`.
- **Option shape**: A Prop declared as `Option<T>` keeps `Option<T>` in storage, getter, setter, and `new!` initial assignment. Do not infer an inner-`T` setter or promote the field into a separate constructor category because it is optional.
- **Named construction**: `elwindui::new!(Type(field: expr, ...))` accepts only named arguments and ordinary Rust expressions. Preserve free argument order, reject positional/`=`/children/dynamic/binding syntax, and diagnose duplicates before evaluating their expressions. Route local generated components through the same-crate registry, builtins through builtin construction, and qualified external generated components through the defining crate-root `__elwindui_ctor_<Type>!` ABI; never infer an unqualified external crate.
- **Construction shape and lifecycle**: Keep `ComponentPublicShape`/`TypeInfo` as the source of truth: required Params/Bindables, defaulted Params, readable fields, and writable Props/content. Apply required construction, unmounted allocation, defaulted Params, initial Props/content, mount, then runtime bindings/resync. Hidden initial setters share normal storage but do not notify, resync, or invoke user callbacks; fixed Param/Bindable fields are never runtime-resynced.
- **Content boundary**: `#[content(field)]` selects the regular `view!` child/content protocol; it is not a constructor category. `new!` has no bare-child or constructor-content form. A named content value is accepted only through the same writable-Prop/content ABI and must preserve the declared field type.
- **Class/codegen boundary**: `class.rs` must not infer Param/Prop/Bindable/defaulted-Param/default semantics. It consumes the shape and emits the storage/accessor/ABI mechanics selected by `elwindui-codegen`.
- **Enum exhaustiveness**: Enums are the primary value-set mechanism. `match` over an enum must be exhaustive; missing arms are compile errors by design.
- **`store` and `viewmodel` isolation**: `store` and `viewmodel` items must never be read directly from `#[param]`. Access must go through reactive `prop` expressions or explicit `<=>` on a writable target, maintaining MVVM V/VM separation.
- **Builtin scoping & resolution**: `view!` bodies glob-import `elwindui::ui::*`. Standard Rust name resolution applies (local definitions take precedence over glob imports).
- **Proc-macro diagnostic quality**: Ensure `syn::Error` spans accurately point to the user code causing compile issues.
- **IDE compatibility**: Generated proc-macro output must pass both `cargo build` and `rust-analyzer diagnostics .`.
