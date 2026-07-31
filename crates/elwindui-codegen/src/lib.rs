pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod component_frontend;
pub mod parser;
pub mod theme_frontend;
mod text_style;
pub mod validate;

use std::fs;
use std::io;
use std::path::Path;

/// Test-only stand-in for the old, workspace-wide `builtins.elwind`/`builtin_modules()` (removed —
/// see 05d4861/29ced3d/c916322/d255f31/36292fb, `docs/elwindui_implementation_status.md`, Refs #14):
/// every real builtin's shape now lives as `#[elwindui_macros::class]` DSL attributes on its actual
/// `elwindui-core`/`elwindui-backend-*` declaration, propagated to a consumer crate via the
/// `__elwindui_shape_*!` macro chain (`elwindui-macros::class::build_props_macro`) rather than a
/// parsed text `Module` — so production code (`generate_component_from_item_struct`,
/// `compile_dir_impl`) no longer chains a builtin `Module` in at all, relying entirely on
/// `codegen::emit_external_construction`'s "no local `TypeInfo`, construct via `elwindui::ui::{Name}`
/// and the shape macro's `@set`/`@clear`/`@children` protocol" path (validated end-to-end by
/// temporarily emptying this exact file and confirming every real example still builds and runs).
///
/// What that production path structurally *can't* do is compiler-side validation that needs an
/// actual field list (`is_abstract`/`#[sealed]`/required-attribute completeness/`#[routed]`-ness) —
/// a real Rust `type` has no equivalent a proc-macro can read across a crate boundary, so those
/// checks silently no-op for an external reference (`check_element_value`/`check_shortcut_attrs`/
/// `validate_inherits`'s own doc comments). That's an acceptable trade for *production* code (a
/// wrong reference still fails to compile, just later, via `elwindui::ui::{Name}::new()`/its shape
/// macro directly) — but several of `codegen.rs`/`validate.rs`'s own unit tests exist specifically to
/// exercise those richer checks, and unlike production they call `validate::validate`/
/// `codegen::generate_module` directly (no later rustc pass to fall back on). This file — a private
/// copy of the old real shape source, now allowed to drift out of sync with the real builtins without
/// consequence, since nothing production-facing reads it — gives those tests real `TypeInfo` to
/// check against again, exactly as `builtin_modules()` used to for everyone.
#[cfg(test)]
const TEST_BUILTIN_SHAPE_SOURCE: &str = include_str!("testdata/builtins.elwind");

/// Test-only counterpart to the removed `builtin_modules()` — see `TEST_BUILTIN_SHAPE_SOURCE`'s own
/// doc comment. `pub(crate)` (not `pub`): only `codegen.rs`/`validate.rs`/`component_frontend.rs`'s
/// own `#[cfg(test)] mod tests` blocks call this, never production code.
#[cfg(test)]
pub(crate) fn test_builtin_modules() -> Vec<ast::Module> {
    // `parse_module` always defaults a freshly-parsed module's `path` to `[]` already.
    let mut module = parser::parse_module(TEST_BUILTIN_SHAPE_SOURCE).unwrap_or_else(|e| {
        panic!("failed to parse test builtin shapes: {e}\n---\n{TEST_BUILTIN_SHAPE_SOURCE}")
    });
    // Marks every component parsed from here as eligible for `#[embedded]` — see
    // `ast::Module::is_builtin`'s doc comment and `validate::validate`'s check.
    module.is_builtin = true;
    vec![module]
}

/// The attribute-macro counterpart to `generate_component_from_item_struct`: takes a
/// `#[elwindui::viewmodel] mod foo { struct Foo { ... } impl Foo { ... } }` (already parsed as a `syn::ItemMod` by the
/// `elwindui-macros` proc-macro), builds the same `ViewModelDef` AST `parser.rs` would from
/// equivalent `.elwind` text (see `attr_frontend`), and feeds it through `generate_module` (not
/// `generate_viewmodel` directly — `generate_module` is also what conditionally emits the
/// `__elwindui_block_on_ready` helper an async `#[command]` needs, and there's no reason to
/// duplicate that check here).
pub fn generate_viewmodel_from_item_mod(
    item_mod: &syn::ItemMod,
) -> Result<proc_macro2::TokenStream, String> {
    let def = attr_frontend::viewmodel_def_from_item_mod(item_mod)?;
    let name = def.name.clone();
    // A single macro invocation has no directory of sibling modules to cross-reference (`use`
    // resolution is moot with only one module), so the exact real path doesn't matter here — `[]`
    // (crate root) is as good as any.
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: vec![ast::Item::ViewModel(def)],
        ..Default::default()
    };
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_viewmodel(&name, item_mod);
    Ok(generated)
}

/// `#[elwindui::dsl_enum] enum Name { A, B, C }` — the opt-in that makes a plain Rust `enum`
/// visible to `validate::validate`'s `match`-exhaustiveness checking the same way `.elwind`'s own
/// `enum Name { .. }` syntax always was (§14's user-enum rule; see
/// `component_frontend::same_crate_enums`'s own doc comment for why an opt-in is needed at all —
/// unlike a `#[elwindui::component]` struct or `#[elwindui::viewmodel]` mod, nothing about a bare
/// `enum` item marks it as DSL-relevant to any proc-macro). Transparent passthrough: the enum body
/// is emitted completely unchanged (it's real Rust, matched with real Rust `match`/`if let`) — the
/// only effect is registering it via `component_frontend::register_same_crate_enum` so a later
/// same-crate `#[elwindui::component]`'s `view!` can be checked against it. Same
/// declared-before-use ordering constraint as the component/viewmodel registries.
pub fn generate_dsl_enum_from_item_enum(
    item_enum: &syn::ItemEnum,
) -> Result<proc_macro2::TokenStream, String> {
    let name = item_enum.ident.to_string();
    // Built purely to validate the enum shape up front (bare unit variants only) — the real
    // `EnumDef` a `view!` elsewhere gets checked against is rebuilt fresh, from the registered
    // source text, by `sibling_enum_modules` itself.
    component_frontend::enum_def_from_item_enum(item_enum)?;
    component_frontend::register_same_crate_enum(&name, item_enum);
    Ok(quote::quote! { #item_enum })
}

/// The attribute-macro counterpart for `component`/`view` (the struct+`view!` frontend, successor
/// to the removed `elwindui::component!` bang macro): takes an already-parsed
/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` (`base`
/// from the attribute's own `inherits Base` argument, `item_struct` parsed by the
/// `elwindui-macros` proc-macro) and builds the matching `ComponentDef`/`ViewDef` pair (see
/// `component_frontend`). Unlike `generate_viewmodel_from_item_mod`, this also chains in
/// `component_frontend::sibling_component_modules()`/`sibling_viewmodel_modules()`/
/// `sibling_enum_modules()`, so a `view!` can reference an *earlier* same-crate
/// `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` as a plain element
/// type, `bind!`/field target, or `match`/`if let` subject (each attribute-macro invocation
/// otherwise only ever sees its own single annotated item — see
/// `component_frontend::same_crate_components`'s own doc comment for the full mechanism and its
/// declaration-order requirement). A `view!` body routinely references
/// `Window`/`VerticalLayout`/etc. too, but those resolve with no `Module` chained in for them at all —
/// see `TEST_BUILTIN_SHAPE_SOURCE`'s own doc comment on why, and
/// `codegen::emit_external_construction`.
pub fn generate_component_from_item_struct(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<proc_macro2::TokenStream, String> {
    let (component_def, view_def) =
        component_frontend::component_and_view_from_item_struct(base.clone(), item_struct)?;
    let name = component_def.name.clone();
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: component_frontend::component_module_items(component_def, view_def),
        allows_external_builtins: true,
        ..Default::default()
    };
    let sibling_modules = component_frontend::sibling_component_modules(&name);
    let sibling_viewmodels = component_frontend::sibling_viewmodel_modules();
    let sibling_enums = component_frontend::sibling_enum_modules();
    let all_modules: Vec<_> = std::iter::once(module.clone())
        .chain(sibling_modules)
        .chain(sibling_viewmodels)
        .chain(sibling_enums)
        .collect();
    validate::validate(&all_modules).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(&all_modules);
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_component(&name, base.as_deref(), item_struct);
    Ok(generated)
}

/// Compiles every `.elwind` file under `src` into Rust source under `out_dir`. The generated
/// code's `t!(..)` calls resolve through `elwindui::i18n` (`elwindui-i18n`, §11) — the caller only
/// needs a one-time `elwindui::i18n::declare!();` (typically at the top of `main()`) for that
/// crate's own `strings/<lang>.ftl` to be found, no per-crate generated i18n glue. Intended to be
/// called from a crate's `build.rs`. See docs/elwindui_spec.md 付録B.1.
pub fn compile_dir(src: impl AsRef<Path>, out_dir: impl AsRef<Path>) -> io::Result<()> {
    compile_dir_impl(src, out_dir, Vec::new())
}

/// Like `compile_dir`, but also folds `ViewModelDef`s found in `extra_rs_files` — plain `.rs` files
/// containing top-level `#[elwindui::viewmodel] mod foo { ... }` blocks, read via
/// `attr_frontend::viewmodel_defs_from_rs_file` — into the `SymbolTable` used to validate the
/// `.elwind` files' `component`/`view` definitions. This is how `vm.field` /
/// `vm.command.execute()` / `vm.command.can_execute` references in a `view { ... }` tree get
/// checked against a viewmodel that's actually defined as ordinary Rust elsewhere in the crate
/// (`examples/notepad`'s `NotepadViewModel`/`Document`, for instance) rather than in another
/// `.elwind` file — as long as the referencing `.elwind` file actually `use`s its real path
/// (`crate::<mod name>::<Type>`, using the `mod` name `viewmodel_defs_from_rs_file` returns
/// alongside each def), matching Rust's own name resolution (§12).
///
/// The extra viewmodels are **not** code-generated here — that already happens for real when the
/// crate compiles and `#[elwindui::viewmodel]` actually expands; this only reads their *shape* for
/// validation, the same static, no-macro-expansion-needed trick `viewmodel_defs_from_rs_file` uses
/// (necessary because `build.rs`, which calls this, always runs before the crate's own source is
/// compiled/macro-expanded — there is no "wait for the macro to run first" option).
pub fn compile_dir_with_extra_viewmodels(
    src: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
    extra_rs_files: &[impl AsRef<Path>],
) -> io::Result<()> {
    let mut extra_modules = Vec::new();
    for path in extra_rs_files {
        let defs = attr_frontend::viewmodel_defs_from_rs_file(path.as_ref()).unwrap_or_else(|e| {
            panic!(
                "scanning {} for #[elwindui::viewmodel] mods: {e}",
                path.as_ref().display()
            )
        });
        extra_modules.extend(defs.into_iter().map(|(mod_name, def)| ast::Module {
            path: vec![mod_name],
            uses: Vec::new(),
            items: vec![ast::Item::ViewModel(def)],
            ..Default::default()
        }));
    }
    compile_dir_impl(src, out_dir, extra_modules)
}

fn compile_dir_impl(
    src: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
    extra_modules: Vec<ast::Module>,
) -> io::Result<()> {
    let src = src.as_ref();
    let out_dir = out_dir.as_ref();

    let mut entries: Vec<_> = fs::read_dir(src)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "elwind"))
        .collect();
    entries.sort_by_key(|e| e.path());

    let mut sources = Vec::new();
    for entry in &entries {
        let text = fs::read_to_string(entry.path())?;
        sources.push((entry.path(), text));
    }

    // `allows_external_builtins: true` — same reason `generate_component_from_item_struct` sets it
    // on its own module (see `TEST_BUILTIN_SHAPE_SOURCE`'s doc comment): there is no builtin `Module`
    // to resolve `Window`/`VerticalLayout`/etc. against anymore, so an `.elwind` file needs the same
    // "no local `TypeInfo`, treat as external" allowance the proc-macro frontend already gets.
    let elwind_modules: Vec<_> = sources
        .iter()
        .map(|(path, text)| {
            let mut module = parser::parse_module(text)
                .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));
            module.allows_external_builtins = true;
            module
        })
        .collect();

    // `extra_modules` (Rust-attribute-macro viewmodels, if any) join in for validation/symbol-table
    // visibility only — see `compile_dir_with_extra_viewmodels`'s doc comment for why they must
    // not be code-generated again in the loop below.
    let all_modules: Vec<_> = elwind_modules
        .iter()
        .cloned()
        .chain(extra_modules.iter().cloned())
        .collect();

    if let Err(errors) = validate::validate(&all_modules) {
        panic!("elwind validation failed:\n{}", errors.join("\n"));
    }

    let table = codegen::build_symbol_table(&all_modules);

    for ((path, _), module) in sources.iter().zip(&elwind_modules) {
        let generated = codegen::generate_module(module, &table);
        let file: syn::File = syn::parse2(generated.clone()).unwrap_or_else(|e| {
            panic!(
                "generated code for {} is not valid Rust: {e}\n---\n{}",
                path.display(),
                generated
            )
        });
        let pretty = prettyplease::unparse(&file);

        let out_name = path.file_stem().unwrap().to_string_lossy().to_string();
        fs::write(out_dir.join(format!("{out_name}.rs")), pretty)?;
    }

    // Every composed builtin (`ContentControl`/`Rectangle`/`Ellipse`) is hand-written directly in
    // `elwindui-core::ui` instead of being regenerated into each consumer's own `OUT_DIR` — a bare
    // reference to one (e.g. `Rectangle { fill: .. }`) resolves via `emit_external_construction`, not
    // a builtin `Module`/the symbol table (see `TEST_BUILTIN_SHAPE_SOURCE`'s own doc comment).
    // `i18n_support.rs` is likewise no longer generated — `elwindui-codegen`'s own emitted `t!(..)`
    // calls resolve through `elwindui::i18n` (see `codegen::emit_expr`), which is a real crate
    // (`elwindui-i18n`, re-exported by the `elwindui` facade) rather than per-consumer generated code.

    Ok(())
}

/// Phase 4 (`docs/elwindui_implementation_status.md`): exercises `#[elwindui::dsl_enum]` end to
/// end through the exact same path `generate_component_from_item_struct` uses in production
/// (register the enum via `generate_dsl_enum_from_item_enum`, then chain it in via
/// `component_frontend::sibling_enum_modules()` while building a sibling component) — confirming a
/// `match` in a Rust-macro-path `view!` gets real exhaustiveness checking against a same-crate
/// `#[elwindui::dsl_enum]`, the gap `component_frontend::same_crate_enums`'s own doc comment
/// describes. Names are unique per test (`DslEnumTest*`) — `same_crate_enums`/
/// `same_crate_components` are process-global statics keyed by `compiling_crate_key()`, which is
/// constant across every test in this one crate's test binary, so two tests reusing the same enum/
/// component name would leak state into each other (same constraint `component_frontend.rs`'s own
/// tests already live with).
#[cfg(test)]
mod dsl_enum_tests {
    use super::*;

    fn register_enum(src: &str) {
        let item_enum: syn::ItemEnum = syn::parse_str(src).expect("enum should parse");
        generate_dsl_enum_from_item_enum(&item_enum).expect("dsl_enum generation should succeed");
    }

    #[test]
    fn exhaustive_match_against_sibling_dsl_enum_validates() {
        register_enum("enum DslEnumTestStatusA { Loading, Ready }");
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct DslEnumTestScreenA {
                #[prop]
                status: DslEnumTestStatusA,
                body: view! {
                    VerticalLayout {
                        match status {
                            DslEnumTestStatusA::Loading => TextBlock { text: "loading" },
                            DslEnumTestStatusA::Ready => TextBlock { text: "ready" },
                        }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let result = generate_component_from_item_struct(None, &item_struct);
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[test]
    fn non_exhaustive_match_against_sibling_dsl_enum_is_rejected() {
        register_enum("enum DslEnumTestStatusB { Loading, Ready }");
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct DslEnumTestScreenB {
                #[prop]
                status: DslEnumTestStatusB,
                body: view! {
                    VerticalLayout {
                        match status {
                            DslEnumTestStatusB::Loading => TextBlock { text: "loading" },
                        }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("non-exhaustive match should be rejected");
        assert!(
            err.contains("not exhaustive") && err.contains("Ready"),
            "error should name the missing variant: {err}"
        );
    }
}

/// Phase 4: confirms the same-crate viewmodel registry (`component_frontend::same_crate_viewmodels`,
/// populated by `generate_viewmodel_from_item_mod`) actually catches, on the Rust-macro path, the
/// two mistakes its own doc comment claims it fixes — a typo'd `vm.field` reference and a
/// `#[bindable]` field whose type isn't a `viewmodel` at all — mirroring `validate.rs`'s own
/// `rejects_reference_to_unknown_vm_field`/`rejects_bindable_field_whose_type_is_not_a_viewmodel`
/// tests for the `.elwind`-text frontend. Names are unique per test for the same reason
/// `dsl_enum_tests` uses unique names (shared process-global registries).
#[cfg(test)]
mod viewmodel_registry_tests {
    use super::*;

    fn register_viewmodel(src: &str) {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse");
        generate_viewmodel_from_item_mod(&item_mod).expect("viewmodel generation should succeed");
    }

    #[test]
    fn typo_vm_field_reference_is_rejected_on_macro_path() {
        register_viewmodel(
            r#"
            mod vm_typo_a_mod {
                struct VmTypoA {
                    #[observable(default = String::new())]
                    content: String,
                }
            }
            "#,
        );
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct ScreenTypoA {
                #[param]
                #[inject]
                vm: VmTypoA,
                body: view! {
                    VerticalLayout {
                        TextBlock { text: vm.no_such_field }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("a typo'd vm field reference should be rejected");
        assert!(err.contains("no_such_field"), "error: {err}");
    }

    #[test]
    fn bindable_field_on_non_viewmodel_type_is_rejected_on_macro_path() {
        // A plain sibling component (not a viewmodel), registered exactly like any other
        // #[elwindui::component] would be.
        let not_vm_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct NotAViewModelB {
                #[param]
                label: String,
                body: view! { TextBlock { text: label } },
            }
            "#,
        )
        .unwrap();
        generate_component_from_item_struct(None, &not_vm_struct)
            .expect("sibling component should build");

        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct WindowBindableB {
                #[bindable]
                thing: std::rc::Rc<NotAViewModelB>,
                body: view! {
                    Window { TextBlock { text: "x" } }
                },
            }
            "#,
        )
        .unwrap();
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("#[bindable] on a non-viewmodel type should be rejected");
        assert!(err.contains("isn't a `viewmodel`"), "error: {err}");
    }
}
