# Codegen & DSL Agent Guidelines

Guidelines for AI agents modifying `elwindui-codegen`, proc-macros (`#[component]`, `#[viewmodel]`, `#[dsl_enum]`), or DSL compiler logic.

## Specification Source of Truth

- **Normative specification**: [`docs/specs/dsl_spec.md`](../specs/dsl_spec.md).
- **Compiler architecture**: [`docs/design/tools/codegen_design.md`](../design/tools/codegen_design.md).
- **Core language & scoping**: `dsl_spec.md` §1–§12.
- **Static verification rules**: `dsl_spec.md` §13 (Rule 1 through Rule 32). Read the specific rule number in `dsl_spec.md` when implementing or modifying compile-time checks.

## Implementation Invariants to Preserve

- **`param` vs `prop`**: `#[param]` fields are fixed at instantiation and may only use static-evaluable expressions (literals, other params, pure builtins, `env::*`, `once!` values) — never reactive prop references or impure calls. Default (`prop`) fields are runtime-mutable and support reactive attribute expressions/`#[computed]`.
- **Enum exhaustiveness**: Enums are the primary value-set mechanism. `match` over an enum must be exhaustive; missing arms are compile errors by design.
- **`store` and `viewmodel` isolation**: `store` and `viewmodel` items must never be read directly from `#[param]`. Access must go through reactive `prop` expressions or explicit `<=>` on a writable target, maintaining MVVM V/VM separation.
- **Builtin scoping & resolution**: `view!` bodies glob-import `elwindui::ui::*`. Standard Rust name resolution applies (local definitions take precedence over glob imports).
- **Proc-macro diagnostic quality**: Ensure `syn::Error` spans accurately point to the user code causing compile issues.
- **IDE compatibility**: Generated proc-macro output must pass both `cargo build` and `rust-analyzer diagnostics .`.
