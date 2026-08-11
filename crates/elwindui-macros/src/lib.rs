use proc_macro::TokenStream;
use syn::spanned::Spanned;

mod class;

/// `#[elwindui::viewmodel] mod foo { struct Foo { #[observable(default = ...)] field: Ty, ... }
/// impl Foo { fn some_action(&self) { ... } } }` — lets a `viewmodel` be written as ordinary Rust
/// (a real `struct` + a real `impl` with real attributes and real `fn` bodies) instead of the
/// the DSL text form's `viewmodel Name { ... }` block, matching how WPF-style MVVM frameworks keep the
/// ViewModel in the host language and reserve markup (here, the DSL's `view { ... }`) for the
/// View. Every `fn`/`async fn` in the `impl` block is itself an action, auto-detected with no
/// separate struct-side declaration — see `elwindui_codegen::attr_frontend` for why the
/// `struct`+`impl` still have to be wrapped in one `mod` (a single attribute-macro invocation only
/// ever sees one annotated item, so both need to arrive together for action bodies to be picked
/// up at all). The DSL text form's `viewmodel` has no equivalent — it only supports
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
/// `component`+`view` pair be written as a single ordinary Rust `struct` instead of the DSL text
/// form's `component Name inherits Base { .. } view Name { .. }` block pair. Ordinary fields become
/// the component's own `#[param]`/`#[prop]`/etc. fields, exactly as in DSL text; exactly one
/// field, typed as a `view! { .. }` macro invocation, supplies the view tree.
///
/// `Base` is a bare name (`inherits ContentControl`) when inheriting a builtin, or a full
/// crate-root-qualified path (`inherits crate::ui::LabeledPanel`) when inheriting another
/// `#[elwindui::component]` — mirroring `#[elwindui_macros::class]`'s own `inherits = ..`
/// requirement (`docs/specs/macro_class_spec.md` §7) and for the same reason: this base's own name
/// ends up embedded in generated code that's textually placed wherever the `use`-less qualified
/// form makes it resolvable, and in the `#[class(inherits = ..)]` argument this type ultimately
/// becomes, whose own `__elwindui_inherit_*!` macro chain may expand from a different module
/// entirely. A bare name naming a user-defined base is rejected (`validate::validate_inherits`,
/// Refs #25).
///
/// `view` is never a real macro — it's never invoked, since this attribute macro (which runs
/// before any inner item macro would) replaces the whole annotated `struct` with different code,
/// so `view!`'s tokens never survive into anything Rust itself expands. They're recovered here as
/// plain DSL text instead (`elwindui_codegen::component_frontend`), the same way the (now removed)
/// `elwindui::component!` bang macro treated its whole input as DSL text via `input.to_string()`.
/// See docs/design/tools/codegen_design.md
///
/// Also accepts a companion `#[elwindui::component] impl Name { .. }` (no arguments), holding
/// `#[overridable]`/`#[overrides]` methods — a bare `struct` has nowhere to put a method *body*, so
/// method inheritance uses the same paired-item shape `#[elwindui_macros::class]` does. The `struct`
/// must be declared first; the `impl` expansion emits a second inherent `impl` block with those
/// methods plus the `__base_<name>` shadows a `base::<name>(..)` call rewrites into. See
/// docs/specs/dsl_spec.md §3 and `elwindui_codegen::generate_component_methods_from_item_impl`.
#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    // The companion form: `#[elwindui::component] impl Name { .. }`, carrying the
    // `#[overridable]`/`#[overrides]` methods for an already-declared `struct Name`. Tried first
    // because an `impl` never parses as an `ItemStruct`, so there's no ambiguity to resolve.
    if let Ok(item_impl) = syn::parse::<syn::ItemImpl>(item.clone()) {
        if !attr.is_empty() {
            let msg = "#[elwindui::component]: the `impl` form takes no arguments — `inherits Base` \
                       belongs on the `struct`";
            return quote::quote! { compile_error!(#msg); }.into();
        }
        return match elwindui_codegen::generate_component_from_item_impl(&item_impl) {
            Ok(tokens) => tokens.into(),
            Err(e) => {
                let msg = format!("#[elwindui::component]: {e}");
                quote::quote! { compile_error!(#msg); }.into()
            }
        };
    }
    let item_struct = match syn::parse::<syn::ItemStruct>(item) {
        Ok(item_struct) => item_struct,
        Err(e) => {
            let msg = format!(
                "#[elwindui::component]: expected `struct Name {{ .. }}` or `impl Name {{ .. }}`: {e}"
            );
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

/// `#[elwindui::dsl_enum] enum Name { A, B, C }` — opts a plain Rust `enum` into `view!`'s
/// `match`/`if let` exhaustiveness checking, the same way the DSL text form's own `enum Name { .. }`
/// syntax always got it. Nothing about a bare `enum` item is otherwise visible to any proc-macro
/// (unlike a `#[elwindui::component]`/`#[elwindui::viewmodel]` item), so an opt-in attribute is the
/// only way to register it into the same-crate symbol table a sibling `#[elwindui::component]`'s
/// `view!` is checked against. Every variant must be a bare unit variant (no payload) — the enum
/// body itself passes through unchanged, since it's real Rust, matched with real Rust `match`.
#[proc_macro_attribute]
pub fn dsl_enum(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_enum = match syn::parse::<syn::ItemEnum>(item) {
        Ok(item_enum) => item_enum,
        Err(e) => {
            let msg = format!("#[elwindui::dsl_enum]: expected a plain `enum Name {{ .. }}`: {e}");
            return quote::quote! { compile_error!(#msg); }.into();
        }
    };
    match elwindui_codegen::generate_dsl_enum_from_item_enum(&item_enum) {
        Ok(tokens) => tokens.into(),
        Err(e) => {
            let msg = format!("#[elwindui::dsl_enum]: {e}");
            quote::quote! { compile_error!(#msg); }.into()
        }
    }
}

/// Declares a typed Rust theme.
///
/// The annotated struct is a declaration surface; its fields become typed tokens and the
/// expansion emits a variant enum plus a live `ThemeController`. The name is intentionally
/// `theme_definition` because Rust uses one macro namespace for attribute and function-like
/// macros, while token references reserve the shorter `theme!(...)` spelling.
///
/// # Example
///
/// ```ignore
/// #[elwindui::theme_definition(
///     extends = SystemTheme,
///     variants(Default, Ocean)
/// )]
/// struct AppTheme {
///     #[theme(default = platform_default, Ocean = Brush::Solid(Color::rgb(0, 80, 120)))]
///     layout_background: Brush,
///
///     #[theme(default = Brush::Solid(Color::rgb(39, 103, 216)))]
///     brand: Brush,
/// }
/// ```
#[proc_macro_attribute]
pub fn theme_definition(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_struct = match syn::parse::<syn::ItemStruct>(item) {
        Ok(item) => item,
        Err(error) => return error.to_compile_error().into(),
    };
    match elwindui_codegen::theme_frontend::generate_theme_from_item_struct(
        attr.into(),
        &item_struct,
    ) {
        Ok(output) => output.into(),
        Err(error) => syn::Error::new_spanned(item_struct, error)
            .to_compile_error()
            .into(),
    }
}


/// Parses `#[component]`'s own argument list: empty (no base), or exactly `inherits Base` (no
/// `=`, matching the DSL's own `component Name inherits Base` spelling — unlike `#[class]`'s
/// `inherits = ..` convention). `Base` may be a bare name (`ContentControl`) — only ever valid for
/// a builtin, since a bare name has no anchor `#[class]`'s own generated `__elwindui_inherit_*!`
/// chain could resolve from elsewhere (see `docs/specs/macro_class_spec.md` §7) — or a full
/// crate-root-qualified path (`crate::ui::LabeledPanel`), required for a user-defined base for the
/// exact same reason. Either form is accepted here; `elwindui_codegen::component_frontend` is what
/// splits the result back into a bare symbol-table name plus an optional qualifying path (Refs #25).
fn parse_inherits_arg(attr: proc_macro2::TokenStream) -> syn::Result<Option<String>> {
    use syn::parse::Parser;
    if attr.is_empty() {
        return Ok(None);
    }
    (|input: syn::parse::ParseStream| {
        let kw: syn::Ident = input.parse()?;
        if kw != "inherits" {
            return Err(syn::Error::new(
                kw.span(),
                "expected `inherits <Base>` or `inherits <crate::path::To::Base>`",
            ));
        }
        let path: syn::Path = input.parse()?;
        Ok(Some(path_to_string(&path)))
    })
    .parse2(attr)
}

/// Joins `path`'s segments with `::`, ignoring any generic arguments (`inherits` targets are
/// always plain type paths, never generic) — a compact, whitespace-free string
/// (`crate::ui::LabeledPanel`) rather than `quote!`'s spaced-out token rendering
/// (`crate :: ui :: LabeledPanel`), which is both easier to re-split (`component_frontend`'s
/// `rsplit_once("::")`) and friendlier to read back in diagnostics.
fn path_to_string(path: &syn::Path) -> String {
    let mut s = String::new();
    if path.leading_colon.is_some() {
        s.push_str("::");
    }
    for (i, seg) in path.segments.iter().enumerate() {
        if i > 0 {
            s.push_str("::");
        }
        s.push_str(&seg.ident.to_string());
    }
    s
}

/// `#[elwindui_macros::class(inherits = SuperClass, struct_only = existing::TraitPath, trait_only, abstract_class, sealed)]`
/// applied to a bare `struct ClassName { .. }` and, separately, a bare `impl ClassName { .. }`
/// (no `for`) — automates the H.2.1a class-hierarchy convention (docs/design/runtime/ui_tree_design.md).
/// See `class::expand`'s own doc comment for the full design and its deliberate simplifications
/// versus a fully generic cross-crate manifest system.
#[proc_macro_attribute]
pub fn class(attr: TokenStream, item: TokenStream) -> TokenStream {
    class::expand(attr.into(), item.into()).into()
}

/// Defines the process entry point for an elwindui application.
///
/// The application runtime needs to be initialized before a UI thread is entered, but the
/// platform-specific event loop must own construction of the first native controls.  Moving the
/// user's body into `application::run` makes that ordering explicit without exposing it in every
/// application.
#[proc_macro_attribute]
pub fn main(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new_spanned(
            proc_macro2::TokenStream::from(attr),
            "#[elwindui::main] does not accept arguments",
        )
        .into_compile_error()
        .into();
    }

    let function = match syn::parse::<syn::ItemFn>(item) {
        Ok(function) => function,
        Err(error) => return error.into_compile_error().into(),
    };

    let mut errors: Option<syn::Error> = None;
    let mut reject = |span: proc_macro2::Span, message: &str| {
        let error = syn::Error::new(span, message);
        if let Some(existing) = &mut errors {
            existing.combine(error);
        } else {
            errors = Some(error);
        }
    };
    if function.sig.ident != "main" {
        reject(
            function.sig.ident.span(),
            "#[elwindui::main] can only be applied to `fn main()`",
        );
    }
    if !function.sig.inputs.is_empty() {
        reject(
            function.sig.inputs.span(),
            "#[elwindui::main] requires a `main` function without arguments",
        );
    }
    if function.sig.asyncness.is_some() {
        reject(
            function.sig.asyncness.span(),
            "#[elwindui::main] does not support `async fn`",
        );
    }
    if !matches!(function.sig.output, syn::ReturnType::Default) {
        reject(
            function.sig.output.span(),
            "#[elwindui::main] requires `fn main()` to return `()`",
        );
    }
    if !function.sig.generics.params.is_empty() || function.sig.generics.where_clause.is_some() {
        reject(
            function.sig.generics.span(),
            "#[elwindui::main] does not support generics or a where clause",
        );
    }
    if function.sig.abi.is_some() {
        reject(
            function.sig.abi.span(),
            "#[elwindui::main] does not support an extern ABI",
        );
    }
    if function.sig.variadic.is_some() {
        reject(
            function.sig.variadic.span(),
            "#[elwindui::main] does not support variadic functions",
        );
    }
    if function.sig.constness.is_some() {
        reject(
            function.sig.constness.span(),
            "#[elwindui::main] does not support const functions",
        );
    }
    if let Some(errors) = errors {
        return errors.into_compile_error().into();
    }

    let attrs = function.attrs;
    let block = function.block;
    quote::quote! {
        #(#attrs)*
        fn main() {
            if let Err(error) = ::elwindui::init() {
                panic!("initialize elwindui: {error:?}");
            }
            ::elwindui::application::run(move || #block);
        }
    }
    .into()
}
