pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod component_frontend;
pub mod parser;
pub mod validate;

use std::fs;
use std::io;
use std::path::Path;

/// Every builtin's shape-only `.elwind` declaration (`component Name { #[param] ... }`, no
/// matching `view`), all in one file — embedded via `include_str!` from this same crate's own
/// source directory (backend-agnostic compiler metadata, so it lives beside the compiler rather
/// than beside any particular backend). These exist purely so `SymbolTable`/`validate` can resolve
/// and check `Window`/`VerticalLayout`/`TextArea`/etc. exactly like any other component —
/// `emit_construction`'s only construction mechanism is "resolve via `SymbolTable`, call
/// `Type::new(args)`", so without these, no builtin would resolve at all. The real implementations
/// live in each `elwindui-backend-*` crate as ordinary hand-written Rust. Adding a new builtin
/// shape only ever means appending a `component` to that one file — nothing here needs to change
/// to pick it up.
const BUILTIN_SHAPE_SOURCE: &str = include_str!("builtins.elwind");

/// Parses the embedded builtin shape file into a `Module`. Registered with the same `path: []`
/// (crate root) every ordinary `.elwind` file compiled by `compile_dir` already uses (付録B.1), so
/// `Window`/`VerticalLayout`/etc. resolve via `SymbolTable::resolve`'s plain "defined locally in
/// `from`" check — the same way two `.elwind` files in the same directory already see each other
/// without a `use` — rather than needing a separate "implicit visibility" fallback mechanism.
pub fn builtin_modules() -> Vec<ast::Module> {
    // `parse_module` always defaults a freshly-parsed module's `path` to `[]` already.
    let mut module = parser::parse_module(BUILTIN_SHAPE_SOURCE).unwrap_or_else(|e| {
        panic!("failed to parse embedded builtin shapes: {e}\n---\n{BUILTIN_SHAPE_SOURCE}")
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
    Ok(codegen::generate_module(&module, &table))
}

/// The attribute-macro counterpart for `component`/`view` (the struct+`view!` frontend, successor
/// to the removed `elwindui::component!` bang macro): takes an already-parsed
/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` (`base`
/// from the attribute's own `inherits Base` argument, `item_struct` parsed by the
/// `elwindui-macros` proc-macro) and builds the matching `ComponentDef`/`ViewDef` pair (see
/// `component_frontend`). Unlike `generate_viewmodel_from_item_mod`, this chains in
/// `builtin_modules()` — a view body routinely references `Window`/`VerticalLayout`/etc. — as well
/// as `component_frontend::sibling_component_modules()`, so a `view!` can also reference an *earlier*
/// same-crate `#[elwindui::component]` struct as a plain element type (each attribute-macro
/// invocation otherwise only ever sees its own single annotated struct — see
/// `component_frontend::same_crate_components`'s own doc comment for the full mechanism and its
/// declaration-order requirement).
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
        items: vec![ast::Item::Component(component_def), ast::Item::View(view_def)],
        ..Default::default()
    };
    let sibling_modules = component_frontend::sibling_component_modules(&name);
    let all_modules: Vec<_> = std::iter::once(module.clone())
        .chain(sibling_modules)
        .chain(builtin_modules())
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
    mut extra_modules: Vec<ast::Module>,
) -> io::Result<()> {
    extra_modules.extend(builtin_modules());
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

    let elwind_modules: Vec<_> = sources
        .iter()
        .map(|(path, text)| {
            parser::parse_module(text)
                .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
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

    // Every composed builtin (`ContentControl`/`Rectangle`/`Ellipse`, `has_view == true` in
    // `builtins.elwind`) is hand-written directly in `elwindui-core::ui` instead of being
    // regenerated into each consumer's own `OUT_DIR` — their `view` blocks stay in
    // `builtins.elwind` purely for `validate`/the symbol table (so use sites like
    // `Rectangle { fill: .. }` still resolve/type-check), but are never fed to `generate_module`
    // here. `i18n_support.rs` is likewise no longer generated — `elwindui-codegen`'s own emitted
    // `t!(..)` calls resolve through `elwindui::i18n` (see `codegen::emit_expr`), which is a real
    // crate (`elwindui-i18n`, re-exported by the `elwindui` facade) rather than per-consumer
    // generated code.

    Ok(())
}

