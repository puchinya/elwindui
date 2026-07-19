use proc_macro::TokenStream;

mod class;

/// `#[elwindui::viewmodel] mod foo { struct Foo { #[observable(default = ...)] field: Ty, ... }
/// impl Foo { fn some_action(&self) { ... } } }` — lets a `viewmodel` be written as ordinary Rust
/// (a real `struct` + a real `impl` with real attributes and real `fn` bodies) instead of the
/// `.elwind` DSL's `viewmodel Name { ... }` block, matching how WPF-style MVVM frameworks keep the
/// ViewModel in the host language and reserve markup (here, `.elwind`'s `view { ... }`) for the
/// View. Every `fn`/`async fn` in the `impl` block is itself an action, auto-detected with no
/// separate struct-side declaration — see `elwindui_codegen::attr_frontend` for why the
/// `struct`+`impl` still have to be wrapped in one `mod` (a single attribute-macro invocation only
/// ever sees one annotated item, so both need to arrive together for action bodies to be picked
/// up at all). `.elwind`-native `viewmodel` text has no equivalent — it only supports
/// `#[observable]`/`#[computed]`; a viewmodel needing actions must use this Rust-native form.
///
/// The `mod` wrapper itself doesn't survive expansion — the generated `struct`/`impl` appear
/// unwrapped at the scope where the `mod` was written.
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

/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` — lets a
/// `component`+`view` pair be written as a single ordinary Rust `struct` instead of the `.elwind`
/// DSL's `component Name inherits Base { .. } view Name { .. }` block pair. Ordinary fields become
/// the component's own `#[param]`/`#[prop]`/etc. fields, exactly as in `.elwind` text; exactly one
/// field, typed as a `view! { .. }` macro invocation, supplies the view tree.
///
/// `view` is never a real macro — it's never invoked, since this attribute macro (which runs
/// before any inner item macro would) replaces the whole annotated `struct` with different code,
/// so `view!`'s tokens never survive into anything Rust itself expands. They're recovered here as
/// plain DSL text instead (`elwindui_codegen::component_frontend`), the same way the (now removed)
/// `elwindui::component!` bang macro treated its whole input as DSL text via `input.to_string()`.
/// See docs/elwindui_spec.md 付録B.1.
///
/// `#[virtual]`/`#[override]` methods aren't supported yet — there's no natural place for a method
/// *body* on a bare `struct` (unlike `#[elwindui::viewmodel]`'s paired `impl` block for action
/// bodies). The natural extension point, if/when needed, is a companion `#[elwindui::component] impl
/// Name { .. }` matched up by struct name.
#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_struct = match syn::parse::<syn::ItemStruct>(item) {
        Ok(item_struct) => item_struct,
        Err(e) => {
            let msg =
                format!("#[elwindui::component]: expected a plain `struct Name {{ .. }}`: {e}");
            return quote::quote! { compile_error!(#msg); }.into();
        }
    };
    let base = match parse_inherits_arg(attr.into()) {
        Ok(base) => base,
        Err(e) => {
            let msg = format!("#[elwindui::component]: {e}");
            return quote::quote! { compile_error!(#msg); }.into();
        }
    };
    match elwindui_codegen::generate_component_from_item_struct(base, &item_struct) {
        Ok(tokens) => tokens.into(),
        Err(e) => {
            let msg = format!("#[elwindui::component]: {e}");
            quote::quote! { compile_error!(#msg); }.into()
        }
    }
}

/// Parses `#[component]`'s own argument list: empty (no base), or exactly `inherits Base` (no
/// `=`, matching the DSL's own `component Name inherits Base` spelling — unlike `#[class]`'s
/// `inherits = ..` convention).
fn parse_inherits_arg(attr: proc_macro2::TokenStream) -> syn::Result<Option<String>> {
    use syn::parse::Parser;
    if attr.is_empty() {
        return Ok(None);
    }
    (|input: syn::parse::ParseStream| {
        let kw: syn::Ident = input.parse()?;
        if kw != "inherits" {
            return Err(syn::Error::new(kw.span(), "expected `inherits <Base>`"));
        }
        let base: syn::Ident = input.parse()?;
        Ok(Some(base.to_string()))
    })
    .parse2(attr)
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
