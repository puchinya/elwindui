pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod parser;
pub mod validate;

use std::fs;
use std::io;
use std::path::Path;

/// Parses, validates and generates Rust code for a single self-contained `.elwind` source string
/// (no filesystem access) — the shared core behind both the build.rs path (`compile_dir`, which
/// additionally builds a symbol table spanning *all* files in a directory for cross-file
/// references) and the proc-macro path (`elwindui-macros`'s `component!`, which only ever sees
/// one macro invocation's worth of source and has no files to cross-reference). See
/// docs/elwindui_spec.md 付録B.1.
pub fn generate_from_source(src: &str) -> Result<proc_macro2::TokenStream, String> {
    let module = parser::parse_module(src)?;
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
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
    let module = ast::Module { uses: Vec::new(), items: vec![ast::Item::ViewModel(def)] };
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
    Ok(codegen::generate_module(&module, &table))
}

/// Compiles every `.elwind` file under `src` into Rust source under `out_dir`, plus a shared
/// `i18n_support.rs` (fluent-bundle-backed `t()` helper, §11) that the generated files call into.
/// Intended to be called from a crate's `build.rs`. See docs/elwindui_spec.md 付録B.1.
pub fn compile_dir(src: impl AsRef<Path>, out_dir: impl AsRef<Path>) -> io::Result<()> {
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

    let modules: Vec<_> = sources
        .iter()
        .map(|(path, text)| {
            parser::parse_module(text)
                .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
        })
        .collect();

    if let Err(errors) = validate::validate(&modules) {
        panic!("elwind validation failed:\n{}", errors.join("\n"));
    }

    let table = codegen::build_symbol_table(&modules);

    for ((path, _), module) in sources.iter().zip(&modules) {
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
