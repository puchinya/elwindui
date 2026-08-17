# Class Model & `#[class]` Macro Guidelines

Guidelines for AI agents working with `elwindui-macros` (`#[elwindui_macros::class]`) or hand-written runtime class hierarchies in `elwindui-core` and backend crates.

## Specification Source of Truth

- **Authoritative spec**: [`docs/specs/macro_class_spec.md`](../specs/macro_class_spec.md).
- **Internal design**: [`docs/design/tools/class_macro_design.md`](../design/tools/class_macro_design.md).
- If source or design conflicts with `macro_class_spec.md`, do not change the spec without an approved contract change.

## Class Hierarchy Convention

For a class `Class` with parent `SuperClass`:

1. **Struct definition**:
   ```rust
   pub struct Class {
       base: SuperClass,
       // own fields...
   }
   ```
   Root classes (with no parent) omit the `base` field. Use bare struct names with no suffix.
2. **Trait definition**:
   ```rust
   pub trait ClassExt: SuperClassExt {
       // own methods...
   }
   ```
3. **Ancestor trait delegation**:
   `Class` must implement `ClassExt` and every ancestor trait in the hierarchy. Ancestor method implementations delegate to `self.base.method(...)`.
4. **Factory construction**:
   Always construct instances via a `create_class(...)` factory function. Never construct bare struct literals directly in external code.

## Macro Generated Code & IDE Support

- `#[elwindui_macros::class]` automates struct/trait generation, inheritance macro expansion (`__elwindui_inherit_*!`), and `rust-analyzer` shadow generation.
- Always verify changes to `#[class]` by running `rust-analyzer diagnostics .` in addition to `cargo check`.

## `struct_only` override transparency (Issue #128)

A `struct_only` implementor of any class-managed interface (`trait_only`, ordinary, or root class) is a transparent bridge for that interface's `#[overridable]`/`#[overrides]` override contract — at arbitrary ordinary-descendant depth, across real crate boundaries. A descendant reached through a `struct_only` implementor uses ordinary `#[overrides]` and `self.base.method()` exactly as it would through any all-`ordinary` chain; it does not need to know that an ancestor happens to be `struct_only`. See `docs/design/tools/class_macro_design.md`'s "`struct_only` transparency" section and `docs/specs/macro_class_spec.md` §5 for the mechanism and public contract.

Do not introduce a new, type-specific override workaround (inherent-method shadowing, UFCS, a second override system) for a case that fits this pattern — use the normal `#[overridable]`/`#[overrides]` mechanism instead. `Window::show`/`hide`/`close` (`crates/elwindui-core/src/ui/controls/window.rs`, migrated by #128) is the reference example.
