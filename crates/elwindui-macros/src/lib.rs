use proc_macro::TokenStream;

mod class;

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

/// `#[elwindui::viewmodel] mod foo { struct Foo { #[observable(default = ...)] field: Ty, ... }
/// impl Foo { fn some_command(&self) { ... } } }` — lets a `viewmodel` be written as ordinary Rust
/// (a real `struct` + a real `impl` with real attributes and real `fn` bodies) instead of the
/// `.elwind` DSL's `viewmodel Name { ... }` block, matching how WPF-style MVVM frameworks keep the
/// ViewModel in the host language and reserve markup (here, `.elwind`'s `view { ... }`) for the
/// View. See docs/elwindui_spec.md 付録O.2, and `elwindui_codegen::attr_frontend` for why the
/// `struct`+`impl` have to be wrapped in one `mod` (a single attribute-macro invocation only ever
/// sees one annotated item, so both need to arrive together for command fields to be matched up
/// with their `impl` bodies).
///
/// The `mod` wrapper itself doesn't survive expansion — the generated `struct`/`impl` appear
/// unwrapped at the scope where the `mod` was written, the same way `component!`'s output does.
#[proc_macro_attribute]
pub fn viewmodel(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_mod = match syn::parse::<syn::ItemMod>(item) {
        Ok(item_mod) => item_mod,
        Err(e) => {
            let msg = format!(
                "#[elwindui::viewmodel]: expected `mod name {{ struct ... impl ... }}`: {e}"
            );
            return quote::quote! { compile_error!(#msg); }.into();
        }
    };
    match elwindui_codegen::generate_viewmodel_from_item_mod(&item_mod) {
        Ok(tokens) => tokens.into(),
        Err(e) => {
            let msg = format!("#[elwindui::viewmodel]: {e}");
            quote::quote! { compile_error!(#msg); }.into()
        }
    }
}

/// `#[elwindui_macros::class(inherits = SuperClass, struct_only = existing::TraitPath, trait_only, abstract_class, sealed)]`
/// applied to a bare `struct ClassName { .. }` and, separately, a bare `impl ClassName { .. }`
/// (no `for`) — automates the H.2.1a class-hierarchy convention (docs/elwindui_spec.md 付録H.2.1a).
/// See `class::expand`'s own doc comment for the full design and its deliberate simplifications
/// versus a fully generic cross-crate manifest system.
#[proc_macro_attribute]
pub fn class(attr: TokenStream, item: TokenStream) -> TokenStream {
    class::expand(attr.into(), item.into()).into()
}
