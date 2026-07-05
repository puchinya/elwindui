pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod parser;
pub mod validate;

use std::fs;
use std::io;
use std::path::Path;

/// Every builtin's shape-only `.elwind` declaration (`component Name { #[param] ... }`, no
/// matching `view`) — embedded via `include_str!` at `elwindui-codegen`'s own compile time (a
/// plain sibling-file read, not a Cargo dependency edge, so there's no cycle with
/// `elwindui-builtins`, which itself depends on `elwindui-core`/the backend crates). These exist
/// purely so `SymbolTable`/`validate` can resolve and check `Window`/`Column`/`TextArea`/etc.
/// exactly like any other component — `emit_construction`'s only construction mechanism is
/// "resolve via `SymbolTable`, call `Type::new(args)`", so without these, no builtin would resolve
/// at all. The real implementations live in `elwindui-builtins` as ordinary hand-written Rust.
const BUILTIN_SHAPE_SOURCES: &[&str] = &[
    include_str!("../../elwindui-builtins/src/shapes/window.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/column.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/row.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/text_area.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/button.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/text.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/menu_bar.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/menu_bar_item.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/menu.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/menu_item.elwind"),
    include_str!("../../elwindui-builtins/src/shapes/tab_view.elwind"),
];

/// Parses every embedded builtin shape into a `Module`. Registered with the same `path: []`
/// (crate root) every ordinary `.elwind` file compiled by `compile_dir` already uses (付録B.1), so
/// `Window`/`Column`/etc. resolve via `SymbolTable::resolve`'s plain "defined locally in `from`"
/// check — the same way two `.elwind` files in the same directory already see each other without
/// a `use` — rather than needing a separate "implicit visibility" fallback mechanism.
pub fn builtin_modules() -> Vec<ast::Module> {
    BUILTIN_SHAPE_SOURCES
        .iter()
        .map(|src| {
            // `parse_module` always defaults a freshly-parsed module's `path` to `[]` already.
            parser::parse_module(src)
                .unwrap_or_else(|e| panic!("failed to parse embedded builtin shape: {e}\n---\n{src}"))
        })
        .collect()
}

/// Parses, validates and generates Rust code for a single self-contained `.elwind` source string
/// (no filesystem access) — the shared core behind both the build.rs path (`compile_dir`, which
/// additionally builds a symbol table spanning *all* files in a directory for cross-file
/// references) and the proc-macro path (`elwindui-macros`'s `component!`, which only ever sees
/// one macro invocation's worth of source and has no files to cross-reference). See
/// docs/elwindui_spec.md 付録B.1.
pub fn generate_from_source(src: &str) -> Result<proc_macro2::TokenStream, String> {
    let module = parser::parse_module(src)?;
    let all_modules: Vec<_> = std::iter::once(module.clone()).chain(builtin_modules()).collect();
    validate::validate(&all_modules).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(&all_modules);
    Ok(codegen::generate_module(&module, &table))
}

/// The attribute-macro counterpart to `generate_from_source`: takes a `#[elwindui::viewmodel] mod
/// foo { struct Foo { ... } impl Foo { ... } }` (already parsed as a `syn::ItemMod` by the
/// `elwindui-macros` proc-macro), builds the same `ViewModelDef` AST `parser.rs` would from
/// equivalent `.elwind` text (see `attr_frontend`), and feeds it through `generate_module` (not
/// `generate_viewmodel` directly — `generate_module` is also what conditionally emits the
/// `__elwindui_block_on_ready` helper an async `#[command]` needs, and there's no reason to
/// duplicate that check here).
pub fn generate_viewmodel_from_item_mod(item_mod: &syn::ItemMod) -> Result<proc_macro2::TokenStream, String> {
    let def = attr_frontend::viewmodel_def_from_item_mod(item_mod)?;
    // A single macro invocation has no directory of sibling modules to cross-reference (`use`
    // resolution is moot with only one module), so the exact real path doesn't matter here — `[]`
    // (crate root) is as good as any.
    let module = ast::Module { path: Vec::new(), uses: Vec::new(), items: vec![ast::Item::ViewModel(def)] };
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
    Ok(codegen::generate_module(&module, &table))
}

/// Compiles every `.elwind` file under `src` into Rust source under `out_dir`, plus a shared
/// `i18n_support.rs` (fluent-bundle-backed `t()` helper, §11) that the generated files call into.
/// Intended to be called from a crate's `build.rs`. See docs/elwindui_spec.md 付録B.1.
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
        let defs = attr_frontend::viewmodel_defs_from_rs_file(path.as_ref())
            .unwrap_or_else(|e| panic!("scanning {} for #[elwindui::viewmodel] mods: {e}", path.as_ref().display()));
        extra_modules.extend(defs.into_iter().map(|(mod_name, def)| ast::Module {
            path: vec![mod_name],
            uses: Vec::new(),
            items: vec![ast::Item::ViewModel(def)],
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
    let all_modules: Vec<_> =
        elwind_modules.iter().cloned().chain(extra_modules.iter().cloned()).collect();

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

    fs::write(out_dir.join("i18n_support.rs"), I18N_SUPPORT_SRC)?;

    Ok(())
}

const I18N_SUPPORT_SRC: &str = r#"
pub use fluent_bundle::FluentValue;

fn load_bundle() -> fluent_bundle::FluentBundle<fluent_bundle::FluentResource> {
    let ftl_string = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/strings/en.ftl")).to_string();
    let res = fluent_bundle::FluentResource::try_new(ftl_string)
        .unwrap_or_else(|(_, errors)| panic!("invalid .ftl file: {errors:?}"));
    let langid: unic_langid::LanguageIdentifier = "en".parse().expect("valid language id");
    let mut bundle = fluent_bundle::FluentBundle::new(vec![langid]);
    bundle.add_resource(res).expect("adding ftl resource");
    bundle
}

thread_local! {
    static BUNDLE: fluent_bundle::FluentBundle<fluent_bundle::FluentResource> = load_bundle();
}

pub fn t(key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
    BUNDLE.with(|bundle| {
        let mut fluent_args = fluent_bundle::FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, value.clone());
        }
        let msg = bundle.get_message(key).unwrap_or_else(|| panic!("missing fluent message `{key}`"));
        let pattern = msg.value().unwrap_or_else(|| panic!("fluent message `{key}` has no value"));
        let mut errors = Vec::new();
        let result = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
        result.into_owned()
    })
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_rust_from_a_single_inline_source() {
        // Everything the split notepad_viewmodel.elwind/notepad_window.elwind files contain,
        // merged into one source string — the shape an `elwindui::component! { ... }` inline
        // macro invocation would see.
        let src = r#"
enum SaveState { Unsaved, Saving, Saved }

viewmodel NotepadViewModel {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",

    #[observable]
    state: SaveState = SaveState::Unsaved,

    #[computed]
    char_count: i32 = content.chars().count() as i32,

    #[computed]
    window_title: String = t!("notepad-window-title", file_name: file_name),

    #[command(can_execute: state != SaveState::Saving)]
    save: Command = command!(|| {
        state = SaveState::Saving;
        document::save(&content);
        state = SaveState::Saved;
    }),
}

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    content: String = bind!(vm.content, TwoWay),
}

view NotepadWindow {
    Window {
        title: vm.window_title
        Column {
            TextArea { text: content }
        }
    }
}
"#;
        let generated = generate_from_source(src).expect("should generate");
        let file: syn::File = syn::parse2(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let pretty = prettyplease::unparse(&file);
        assert!(pretty.contains("struct NotepadViewModel"));
        assert!(pretty.contains("struct NotepadWindow"));
    }
}
