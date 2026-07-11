//! `#[elwindui_macros::class]` — automates the H.2.1a class-hierarchy convention
//! (docs/elwindui_spec.md 付録H.2.1a): `trait Class: SuperClass` + `struct ClassImpl { base:
//! SuperClassImpl, .. }`, with `ClassImpl` implementing `Class` plus every ancestor trait,
//! delegating ancestor methods to `self.base`.
//!
//! Applied to a bare `struct ClassName { .. }` (no `Impl` suffix, no `base` field written by
//! hand) and, separately, a bare `impl ClassName { .. }` (no `for`) — two independent attribute
//! invocations rather than one `mod`-wrapped pair, since Rust resolves item names within a module
//! regardless of declaration order (`struct` must still be written textually before `impl` for
//! readability, but nothing here requires it).
//!
//! Ancestor delegation is **not** implemented as a fully generic cross-crate manifest/token-
//! accumulator (the `ambassador` crate's technique) — it doesn't need to be, because every
//! `inherits = ..` target's `base` field type genuinely implements `UIElement` itself
//! (`UIElementImpl`/`LayoutImpl` included — see their own hand-written one-line `impl UIElement`
//! blocks in `elwindui-core::ui`), so a single blind `self.base.method(..)` forward
//! (`uielement_blind_forward`) is always correct; the macro never needs to know *which* ancestor
//! it is.
//!
//! `inherits` omitted entirely puts a `struct`/`impl` pair into **root class mode** instead of
//! declaring an ordinary subclass — used for the one class with no ancestor of its own
//! (`UIElement`). There, `impl ClassName { .. }` isn't paired with a generated
//! `impl ClassName for ClassNameImpl` at all (that struct is never meant to implement the trait
//! itself — its descendants embed it as `base` and implement the trait *themselves*, the same way
//! every other `#[class]`-managed subclass already does): every method the user writes is instead
//! embedded, body and all, directly into the generated `pub trait ClassName { .. }` as a *default*
//! method, inherited for free by every descendant via Rust's own default-method dispatch. The one
//! exception is `base` itself, whose concrete location differs per implementor and so can't have a
//! shared default — the macro synthesizes its (required, body-less) signature itself, and errors
//! if the user tries to define it by hand. `supertrait = ..` adds any extra bound this root class
//! needs (e.g. `UIElement: AsAny`) — a different concept from `inherits`, which additionally drives
//! `base`-field insertion that a root class (having no ancestor) never wants.
//!
//! Field-driven `Cell`/`RefCell` accessor generation is likewise not implemented — those getters/
//! setters stay hand-written in `impl ClassName { .. }`, exactly as today; they are typically one
//! line each and the real win here is eliminating the `ClassNameImpl`/trait declaration and the
//! ancestor delegation boilerplate.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Field, Fields, FnArg, Ident, ImplItem, ImplItemFn, Item, Path, Token, Type, Visibility,
};

/// Class names that `.elwind`'s `#[sealed]` already marks non-inheritable (docs/elwindui_spec.md
/// `builtins.elwind`) — hardcoded rather than derived from a cross-invocation manifest, matching
/// this module's overall simplification (see its own doc comment).
const SEALED_CLASSES: &[&str] = &["TextArea", "Button"];

/// The fixed method set every `UIElement` implementor may define — used to route a method written
/// inside `impl ClassName { .. }` to the generated `impl elwindui_core::ui::UIElement for
/// ClassNameImpl` block instead of to `ClassName`'s own trait impl.
const UI_ELEMENT_METHODS: &[&str] =
    &["base", "visual_children", "measure_override", "arrange_override", "paint", "as_native_control"];

/// Parsed `#[class(inherits = .., implements = .., supertrait = .., abstract_class, sealed)]`
/// arguments — every key is optional and any subset/order is accepted.
#[derive(Default)]
struct ClassArgs {
    inherits: Option<Type>,
    implements: Option<Path>,
    /// A supertrait bound unrelated to the H.2.1a "class hierarchy" concept `inherits` models
    /// (no `base` field is inserted for it) — e.g. `UIElement: AsAny`. Only meaningful in "root
    /// class mode" (`inherits` omitted): see `expand_impl`'s own doc comment.
    supertrait: Option<Path>,
    abstract_class: bool,
    sealed: bool,
}

impl Parse for ClassArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ClassArgs::default();
        let items = Punctuated::<ClassArg, Token![,]>::parse_terminated(input)?;
        for item in items {
            match item {
                ClassArg::Inherits(ty) => args.inherits = Some(ty),
                ClassArg::Implements(path) => args.implements = Some(path),
                ClassArg::Supertrait(path) => args.supertrait = Some(path),
                ClassArg::AbstractClass => args.abstract_class = true,
                ClassArg::Sealed => args.sealed = true,
            }
        }
        Ok(args)
    }
}

enum ClassArg {
    Inherits(Type),
    Implements(Path),
    Supertrait(Path),
    AbstractClass,
    Sealed,
}

impl Parse for ClassArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match ident.to_string().as_str() {
            "inherits" => {
                input.parse::<Token![=]>()?;
                Ok(ClassArg::Inherits(input.parse()?))
            }
            "implements" => {
                input.parse::<Token![=]>()?;
                Ok(ClassArg::Implements(input.parse()?))
            }
            "supertrait" => {
                input.parse::<Token![=]>()?;
                Ok(ClassArg::Supertrait(input.parse()?))
            }
            "abstract_class" => Ok(ClassArg::AbstractClass),
            "sealed" => Ok(ClassArg::Sealed),
            other => Err(syn::Error::new(ident.span(), format!("#[class]: unknown argument `{other}`"))),
        }
    }
}

/// `NativeControl<AnyView>` -> `NativeControlImpl<AnyView>`; `crate::TextAreaImpl` (already
/// `Impl`-suffixed — the "explicit facade type" form, docs/elwindui_spec.md 付録H.2.1a's
/// discussion of the backend DSL-facing wrapper layer) -> used as-is.
fn base_impl_type(ty: &Type) -> Type {
    let mut ty = ty.clone();
    if let Type::Path(tp) = &mut ty {
        if let Some(seg) = tp.path.segments.last_mut() {
            let name = seg.ident.to_string();
            if !name.ends_with("Impl") {
                seg.ident = Ident::new(&format!("{name}Impl"), seg.ident.span());
            }
        }
    }
    ty
}

/// `VerticalLayout` -> `VerticalLayoutImpl`; `TabViewImpl` (already `Impl`-suffixed — needed when
/// the bare class name is already taken by a hand-written trait declaration in the same module,
/// e.g. appkit's local `TabView` trait, so the `#[class]` struct/impl pair is itself named
/// `TabViewImpl` directly) -> used as-is, mirroring `base_impl_type`'s same heuristic.
fn to_impl_name(class_name: &Ident) -> Ident {
    let name = class_name.to_string();
    if name.ends_with("Impl") { class_name.clone() } else { format_ident!("{name}Impl") }
}

pub fn expand(attr: TokenStream2, item: TokenStream2) -> TokenStream2 {
    let args = match syn::parse2::<ClassArgs>(attr) {
        Ok(args) => args,
        Err(e) => return e.to_compile_error(),
    };
    let parsed_item = match syn::parse2::<Item>(item) {
        Ok(item) => item,
        Err(e) => return e.to_compile_error(),
    };
    match parsed_item {
        Item::Struct(item_struct) => expand_struct(&args, item_struct),
        Item::Impl(item_impl) => expand_impl(&args, item_impl),
        other => {
            let msg = "#[class]: expected a `struct ClassName { .. }` or `impl ClassName { .. }` item";
            let mut ts = syn::Error::new_spanned(&other, msg).to_compile_error();
            ts.extend(quote! { #other });
            ts
        }
    }
}

/// `elwindui_core::ui::X` from every other crate, but `crate::ui::X` when `#[class]` is itself
/// expanding *inside* `elwindui-core` (its own `ui.rs` uses this macro too, e.g. for
/// `NativeControl<H>`) — a crate cannot refer to itself by its external `extern crate` name.
/// `CARGO_PKG_NAME` is set by cargo to whichever crate is currently being compiled, i.e. exactly
/// the crate this macro invocation is expanding within.
fn core_path() -> TokenStream2 {
    match std::env::var("CARGO_PKG_NAME").as_deref() {
        Ok("elwindui-core") => quote! { crate },
        _ => quote! { elwindui_core },
    }
}

fn expand_struct(args: &ClassArgs, item: syn::ItemStruct) -> TokenStream2 {
    let class_name = &item.ident;
    let impl_name = to_impl_name(class_name);
    let vis = &item.vis;
    let attrs = &item.attrs;
    let generics = &item.generics;

    let existing_fields: Vec<Field> = match &item.fields {
        Fields::Named(named) => named.named.iter().cloned().collect(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return syn::Error::new_spanned(&item, "#[class]: struct must use named fields, not a tuple struct")
                .to_compile_error();
        }
    };

    // `pub` (matching H.2.1a's convention): plenty of call sites reach through `.base` across
    // module/crate boundaries (`self.base.handle`, `self.base.base()`, ...).
    let base_field = args.inherits.as_ref().map(|ty| {
        let base_ty = base_impl_type(ty);
        quote! { pub base: #base_ty, }
    });

    quote! {
        #(#attrs)*
        #vis struct #impl_name #generics {
            #base_field
            #(#existing_fields,)*
        }
    }
}

fn expand_impl(args: &ClassArgs, item: syn::ItemImpl) -> TokenStream2 {
    if item.trait_.is_some() {
        return syn::Error::new_spanned(
            &item,
            "#[class]: write `impl ClassName { .. }` (no `for`) — the macro routes each method to the right \
             generated `impl .. for ClassNameImpl` block itself",
        )
        .to_compile_error();
    }
    let class_name = match &*item.self_ty {
        Type::Path(tp) => match tp.path.segments.last() {
            Some(seg) => seg.ident.clone(),
            None => return syn::Error::new_spanned(&item.self_ty, "#[class]: empty type path").to_compile_error(),
        },
        other => return syn::Error::new_spanned(other, "#[class]: unsupported `impl` self type").to_compile_error(),
    };
    let impl_name = to_impl_name(&class_name);
    // `<H>`/`<H: LayoutNode + 'static>`/where-clause, threaded through every generated block below
    // so a generic class (e.g. `NativeControl<H>`) works the same as a non-generic one.
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    if let Some(inh) = &args.inherits {
        if let Type::Path(tp) = inh {
            if let Some(seg) = tp.path.segments.last() {
                if SEALED_CLASSES.contains(&seg.ident.to_string().as_str()) {
                    let msg = format!("class `{}` is #[sealed] and cannot be inherited", seg.ident);
                    return syn::Error::new_spanned(inh, msg).to_compile_error();
                }
            }
        }
    }

    // `bool` = force this entry to `pub` regardless of what the user wrote — true constructors
    // (no `self`) always did in the hand-written original, but an `#[inherent]` `&self` helper
    // keeps whatever visibility the user actually gave it (some, like `rebuild`/
    // `sync_dynamic_entries` in the appkit `TabView` facade, are deliberately private).
    let mut ctor_methods: Vec<(ImplItemFn, bool)> = Vec::new();
    let mut instance_methods: Vec<ImplItemFn> = Vec::new();
    for impl_item in item.items {
        match impl_item {
            ImplItem::Fn(mut f) => {
                // `#[inherent]` opts a `&self` method *out* of trait-impl routing entirely — for
                // helpers that aren't part of any trait (ancestor's or `ClassName`'s own), e.g. the
                // backend facade layer's `into_any_view`/`set_on_text_change`. It lands as a plain
                // `impl ClassNameImpl { .. }` method, alongside constructors.
                let is_inherent = f.attrs.iter().any(|a| a.path().is_ident("inherent"));
                // `#[overridable]`/`#[overrides]` are accepted and stripped but not yet validated
                // against an ancestor's virtual-method list — see this module's own doc comment.
                f.attrs.retain(|a| !(a.path().is_ident("overridable") || a.path().is_ident("overrides") || a.path().is_ident("inherent")));
                if is_inherent {
                    ctor_methods.push((f, false));
                    continue;
                }
                let has_self = matches!(f.sig.inputs.first(), Some(FnArg::Receiver(_)));
                if has_self {
                    // Every instance method lands in a trait impl (either an ancestor's or
                    // `ClassName`'s own) — trait impl items always inherit the trait's own
                    // visibility and reject an explicit qualifier (E0449).
                    f.vis = Visibility::Inherited;
                    instance_methods.push(f);
                } else {
                    ctor_methods.push((f, true));
                }
            }
            other => {
                return syn::Error::new_spanned(other, "#[class]: only `fn` items are supported inside `impl ClassName { .. }`")
                    .to_compile_error();
            }
        }
    }

    // Root-class mode (`inherits` omitted, e.g. `UIElement` itself): there's no ancestor to route
    // anything to, so every method the user wrote is an "own" method — no `UI_ELEMENT_METHODS`
    // partitioning happens at all. See `expand_impl`'s own doc comment for the full shape.
    let is_root_mode = args.inherits.is_none();
    let (ui_element_methods, own_methods): (Vec<ImplItemFn>, Vec<ImplItemFn>) = if is_root_mode {
        (Vec::new(), instance_methods)
    } else {
        instance_methods.into_iter().partition(|f| UI_ELEMENT_METHODS.contains(&f.sig.ident.to_string().as_str()))
    };

    let core = core_path();
    let mut ancestor_impls = Vec::new();
    if let Some(inh) = &args.inherits {
        // `UIElementImpl` and `LayoutImpl` both implement `UIElement` themselves now (see
        // `expand_impl`'s own doc comment), so *every* `inherits = ..` target's `base` field type
        // genuinely implements `UIElement` — blind `self.base.method(..)` forwarding is always
        // correct, with no need to special-case which ancestor this is.
        ancestor_impls.push(uielement_blind_forward(&core, &impl_name, &impl_generics, &ty_generics, &where_clause, &ui_element_methods));
        // Any intermediate marker trait between `UIElement` and this class (`Layout`,
        // `NativeControl<H>`, ...) also needs an (empty, today — every such marker trait declares
        // no methods of its own) `impl` of its own — skipped for `UIElement` itself (already fully
        // covered by the blind forward above) and for the "explicit facade type" form (an
        // already-`Impl`-suffixed concrete struct, not a trait, so there's nothing to `impl`).
        if let Type::Path(tp) = inh {
            if let Some(seg) = tp.path.segments.last() {
                let name = seg.ident.to_string();
                if name != "UIElement" && !name.ends_with("Impl") {
                    ancestor_impls.push(quote! {
                        impl #impl_generics #inh for #impl_name #ty_generics #where_clause {}
                    });
                }
            }
        }
    }

    let (trait_decl, trait_impl) = if let Some(existing) = &args.implements {
        let bodies = own_methods.iter().map(|f| quote! { #f });
        (
            TokenStream2::new(),
            quote! {
                impl #impl_generics #existing for #impl_name #ty_generics #where_clause {
                    #(#bodies)*
                }
            },
        )
    } else if is_root_mode {
        // Root class mode (`inherits` omitted, e.g. `UIElement` itself): every method the user
        // wrote becomes a *default* trait method (body embedded directly in the trait declaration,
        // shared by every future `inherits = ClassName` descendant for free via Rust's own
        // default-method dispatch) rather than a required method paired with a separate `impl
        // ClassName for ClassNameImpl` — that pairing doesn't apply here since `ClassNameImpl`
        // itself is never meant to implement `ClassName` (descendants embed it as `base` and
        // implement the trait themselves, exactly like `UIElementImpl` does today). `base` is the
        // one method every implementor must supply itself (its concrete location differs per
        // type), so the macro synthesizes it as the sole required signature — the user must not
        // define it by hand.
        if let Some(base_fn) = own_methods.iter().find(|f| f.sig.ident == "base") {
            let msg = "#[class]: root class's `base` is auto-generated; do not define it";
            return syn::Error::new_spanned(&base_fn.sig, msg).to_compile_error();
        }
        let bound = args.supertrait.as_ref().map(|t| quote! { : #t });
        let default_methods = own_methods.iter().map(|f| quote! { #f });
        (
            quote! {
                pub trait #class_name #impl_generics #bound #where_clause {
                    fn base(&self) -> &#impl_name;
                    #(#default_methods)*
                }
            },
            TokenStream2::new(),
        )
    } else if class_name.to_string().ends_with("Impl") {
        // The class's own local name is already `Impl`-suffixed (the "explicit facade type" form
        // — see `to_impl_name`'s doc comment) — there's no bare trait name to derive one from, and
        // in practice every method on this kind of facade is either an ancestor delegate or an
        // `#[inherent]` helper, so there's nothing left needing a trait home at all.
        if !own_methods.is_empty() {
            let names: Vec<String> = own_methods.iter().map(|f| f.sig.ident.to_string()).collect();
            let msg = format!(
                "#[class]: `{class_name}` has no bare class name to declare a trait under (its own name is already \
                 `Impl`-suffixed) and no `implements = ..` was given, but these methods aren't ancestor methods \
                 either: {names:?} — mark them `#[inherent]` if they're plain helpers"
            );
            return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
        }
        (TokenStream2::new(), TokenStream2::new())
    } else {
        let bound = args.inherits.as_ref().map(|t| quote! { : #t });
        let sigs = own_methods.iter().map(|f| {
            let sig = &f.sig;
            quote! { #sig; }
        });
        let bodies = own_methods.iter().map(|f| quote! { #f });
        (
            quote! { pub trait #class_name #impl_generics #bound #where_clause { #(#sigs)* } },
            quote! {
                impl #impl_generics #class_name #ty_generics for #impl_name #ty_generics #where_clause {
                    #(#bodies)*
                }
            },
        )
    };

    if args.abstract_class {
        if let Some((new_fn, _)) = ctor_methods.iter().find(|(f, _)| f.sig.ident == "new") {
            let msg = format!("abstract_class `{class_name}` must not define `new`");
            return syn::Error::new_spanned(&new_fn.sig, msg).to_compile_error();
        }
    }

    let mut ctor_block = TokenStream2::new();
    if !ctor_methods.is_empty() {
        let fns: Vec<TokenStream2> = ctor_methods
            .iter()
            .map(|(f, force_pub)| {
                let mut f = f.clone();
                if *force_pub {
                    f.vis = Visibility::Public(Default::default());
                }
                quote! { #f }
            })
            .collect();
        ctor_block = quote! {
            impl #impl_generics #impl_name #ty_generics #where_clause {
                #(#fns)*
            }
        };
    }

    let out = quote! {
        #trait_decl
        #trait_impl
        #(#ancestor_impls)*
        #ctor_block
    };
    if std::env::var("ELWINDUI_CLASS_DEBUG").is_ok() {
        eprintln!("=== #[class] impl {class_name} expansion ===\n{out}\n===");
    }
    out
}

/// Builds `impl elwindui_core::ui::UIElement for #impl_name #ty_generics { .. }` for the
/// [`AncestorShape::BlindDelegate`] case: each of the four methods either comes from
/// `user_methods` (an explicit override) or falls back to a hardcoded `self.base.method(..)`
/// forward.
fn uielement_blind_forward(
    core: &TokenStream2,
    impl_name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: &Option<&syn::WhereClause>,
    user_methods: &[ImplItemFn],
) -> TokenStream2 {
    let find = |name: &str| user_methods.iter().find(|f| f.sig.ident == name).map(|f| quote! { #f });
    let base = find("base").unwrap_or(quote! {
        fn base(&self) -> &#core::ui::UIElementImpl { self.base.base() }
    });
    let measure_override = find("measure_override").unwrap_or(quote! {
        fn measure_override(&self, available: #core::layout::Size, child_sizes: &[#core::layout::Size]) -> #core::layout::Size {
            self.base.measure_override(available, child_sizes)
        }
    });
    let arrange_override = find("arrange_override").unwrap_or(quote! {
        fn arrange_override(&self, final_size: #core::layout::Size, child_sizes: &[#core::layout::Size]) -> Vec<#core::layout::Rect> {
            self.base.arrange_override(final_size, child_sizes)
        }
    });
    let as_native_control = find("as_native_control").unwrap_or(quote! {
        fn as_native_control(&self) -> Option<&dyn std::any::Any> { self.base.as_native_control() }
    });
    let visual_children = find("visual_children");
    let paint = find("paint");
    quote! {
        impl #impl_generics #core::ui::UIElement for #impl_name #ty_generics #where_clause {
            #base
            #measure_override
            #arrange_override
            #as_native_control
            #visual_children
            #paint
        }
    }
}
