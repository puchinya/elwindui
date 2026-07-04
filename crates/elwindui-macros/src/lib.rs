use proc_macro::TokenStream;

/// `elwindui::component! { component NotepadWindow { ... } view NotepadWindow { ... } }`, the
/// proc-macro alternative to the build.rs codegen path (`elwindui_codegen::compile_dir`) — lets
/// `component`/`viewmodel`/`enum`/`view` items be written inline in a `.rs` file instead of in a
/// separate `.elwind` file, the way `slint::slint! { ... }` embeds Slint markup inline. See
/// docs/elwindui_spec.md 付録B.1.
///
/// A single invocation is parsed as one self-contained `Module` (see
/// `elwindui_codegen::generate_from_source`): cross-references only resolve against `component`/
/// `viewmodel`/`enum` items written in the *same* macro invocation, unlike `compile_dir`, which
/// builds one symbol table spanning every `.elwind` file in a directory.
#[proc_macro]
pub fn component(input: TokenStream) -> TokenStream {
    let src = input.to_string();
    match elwindui_codegen::generate_from_source(&src) {
        Ok(tokens) => tokens.into(),
        Err(e) => {
            let msg = format!("elwindui::component!: {e}");
            quote::quote! { compile_error!(#msg); }.into()
        }
    }
}
