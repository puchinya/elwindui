# Common Agent Guidelines

These rules apply to all AI agent work modifying Rust code and documentation in this repository. Repository-wide document authority, synchronization order, Issue workflow, and `gh` CLI rules are defined only in the root [`AGENTS.md`](../../AGENTS.md).

## Rules

- **Public APIs require rustdoc**: Every newly added or changed public type, trait, enum variant, field, function, method, macro, and generated public item must have useful `///`/`//!` documentation written in English. Document behavioral contracts and sentinel/reset semantics rather than merely repeating the item name; add a compilable example when the API is not self-explanatory.
- **Document authority**: Follow the document authority rules defined in the root [`AGENTS.md`](../../AGENTS.md). Do not treat source code, status reports, or design documents as overriding normative specifications.
- **Scope discipline**: Keep changes strictly within the approved Issue scope. Do not mix unrelated refactoring, cleanup, formatting changes, or opportunistic API changes.
- **Architecture integrity**: Do not unilaterally invent exceptions to established codebase conventions or rules (e.g. class-hierarchy patterns, module layering, or macro conventions) to work around a problem before fully root-causing it. Verify whether a workaround is truly necessary first. If a real exception is needed, request approval before applying it.
- **Abstraction boundary**: Do not expose backend-specific types or platform primitives in common facade/core APIs (`elwindui`, `elwindui-core`). Keep common traits and types strictly backend-agnostic.
- **Dependency discipline**: Do not introduce new external dependencies or workspace dependencies without explicit justification and approval.
