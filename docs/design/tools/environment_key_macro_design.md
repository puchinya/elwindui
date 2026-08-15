# Environment Key cross-crate macro design (Issue #129)

Related specification: [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md) §2, [`../../specs/dsl_spec.md`](../../specs/dsl_spec.md) §4/§5/§13 rules 34/35. Related design: [`class_macro_design.md`](class_macro_design.md) "Shape macro protocol", which this mirrors at a smaller scope.

## Problem

`#[environment(name)]`/`EnvironmentScope { name: value }` resolve `name` against `component_frontend::same_crate_environment_keys()` — a process-local registry keyed by `(compiling_crate_key(), name)`. Each crate's proc-macro expansion is its own isolated compiler process, so this registry cannot see a Key registered by a different crate: `#[elwindui::environment_key]` was resolvable only from the declaring crate.

## Resolution: a per-Key macro export

`environment_frontend::generate_environment_key_from_item_struct` now emits, alongside the existing registry side-effect and `EnvironmentKey` impl, a `#[macro_export] macro_rules! __elwindui_environment_key_{name}` whose sole expansion is `$crate::#ident` (the Key type's own path, using the `$crate` metavariable so it always resolves to the *declaring* crate regardless of where it's invoked from — same technique `class_macro_design.md`'s "Ancestor forwarding" section uses for the same reason).

Unlike `#[class]`'s `__elwindui_props_*!`, this is a single flat macro per Key with **one** expansion arm — no `@verb` dispatch, no ancestor-chain forwarding, and therefore no catch-all arm: an Environment Key has no ancestor chain (`#[class]`'s forwarding exists to walk from a subclass's macro up to whichever ancestor actually declared a given property; a Key is declared exactly once, by exactly one macro).

## DSL syntax and dispatch

`#[environment(name)]`/`EnvironmentScope { name: value }` accept two forms:

- **bare** (`locale`): unchanged — resolved through the same-crate registry, `validate.rs` rule 34/35 rejects an unresolvable name at proc-macro time with a spec-referencing `compile_error!`.
- **qualified** (`some_crate::locale`): resolved through the cross-crate macro. There is no early validation — a proc-macro cannot see whether another crate exports a given macro name before real compilation runs — so an unresolvable qualified name surfaces as `rustc`'s own "cannot find macro" error once the generated code is actually compiled, not a `compile_error!`. This is a deliberate, accepted asymmetry (`dsl_spec.md` §13 rules 34/35): `#[class]`'s own catch-all `compile_error!` only ever validates *within* an already-resolved macro (a wrong property name on an existing class), never "does this macro exist at all" — the same limit applies here.

The two forms are mutually exclusive; there is no bare-then-qualified fallback.

`#[environment(..)]`'s qualified form is parsed as a real `syn::Path` (`attr_frontend::split_environment_key_path`) — proc-macro attribute parsing, so it accepts an arbitrary-depth path (`some_crate::nested::locale`), taking the last segment as the Key's registered `name` and everything before it as the macro-invocation prefix.

`EnvironmentScope { some_crate::locale: value }`'s qualified form instead reuses the hand-written DSL parser's existing `Owner::field: value` attached-property grammar (`parser.rs`) — `EnvironmentScope`'s body has no dedicated grammar of its own (`plan_environment_scope`'s own doc comment), and inventing one just for this would duplicate what `Owner::field` already parses. This means it is restricted to **exactly one** `::` (crate/alias segment + key name), unlike `#[environment(..)]`'s arbitrary-depth form. `validate.rs`'s `check_attached_properties` (which normally checks `Owner::field` against a real `#[attached]`-kind field) skips this list entirely for an `EnvironmentScope` element — `owner` is a crate path here, not a component/builtin type, and (as above) there is nothing it *could* validate early either way.

## Avoiding `macro_expanded_macro_exports_accessed_by_absolute_paths`

`#[class]`'s own `__elwindui_inherit_*!`/`__elwindui_props_*!` call sites splice an absolute-path macro invocation directly into generated code (`elwindui::core::#props_macro!(..)`) and carry `#[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]` for it — a deny-by-default future-incompatible lint that fires when macro-generated code references a `macro_export`-declared `macro_rules!` (itself defined inside macro-generated code) by absolute path. Issue #129 requires the new cross-crate mechanism not depend on that same allow.

An isolated multi-crate repro (built before committing to an approach) confirmed the naive equivalent — `environment_key_type` originally spliced `some_crate::__elwindui_environment_key_locale!()` directly into type position, mirroring `#[class]`'s pattern — does trip the deny-by-default lint for real, hard-erroring `cargo build`. The fix: never reference the macro by absolute path. Instead, `environment_key_type`/`environment_key_type_by_name` return `(preamble, key_type)`:

```rust
use some_crate::__elwindui_environment_key_locale;
type __ElwindEnvKeyAlias_locale = __elwindui_environment_key_locale!();
```

— a `use`-import (ordinary Rust 2018+ path-based macro import, not the legacy absolute-path resolution the lint targets) followed by a **bare** macro invocation, aliased to a local `type` item. The caller splices `preamble` into the same local block as its own (now bare, `__ElwindEnvKeyAlias_locale`-typed) use of `key_type`, before that use. This was also confirmed against the same isolated repro: the `use`-then-bare-call form compiles with no lint at all.

`alias_seed` (the local alias's uniquifying suffix, usually the field name, or `"{owner}_{field}"` for an `EnvironmentScope` override) exists because several call sites accumulate more than one field's preamble into one shared enclosing scope (`generate_component`'s `default_let_stmts`, `emit_environment_scope_construction`'s `sets`) — two qualified references in the same scope must not collide on the same local `type` alias name.

## Cross-crate reachability

No `__elwindui_macros_of_{Name}`-style reexport wrapper module is needed (contrast `class_macro_design.md`'s "Ancestor forwarding", which needs one so a class's props macro is reachable through an arbitrary `pub use module::*` reexport chain matching the type's own nested module path). An Environment Key macro is always invoked as `<crate-root-resolvable-prefix>::__elwindui_environment_key_{name}` — a `#[macro_export]` macro is always placed at the *defining* crate's root regardless of which module the `#[elwindui::environment_key]` struct itself lives in, so the DSL-facing qualified path only ever needs to resolve to that crate's root, never to the struct's own (possibly nested) module.

## Test coverage

- `elwindui-codegen`'s `environment_key_tests` (source-text checks): registration, same-crate resolution/rejection, and `qualified_cross_crate_key_bypasses_the_same_crate_registry` (proves rule 34 does not fire for a qualified name, and that the generated shape is the `use`+alias form, not an absolute-path call).
- `crates/elwindui-environment-key-fixture` — a real second crate declaring one `pub` Key, dev-dependency-cycled back into `crates/elwindui` (a Cargo-supported pattern: dev-dependencies never affect the library build) — backing `crates/elwindui/tests/environment_field_cross_crate.rs` and `environment_scope_cross_crate.rs`, real end-to-end `rustc` coverage for both DSL forms.
