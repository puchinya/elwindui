//! PR #312 Implementation Contract v3 (D1/D2): the class-backed forms of `#[component]`.
//!
//! One public attribute, `#[component(..)]`, covers both generated (declarative) Components and
//! Components that keep their own hand-written Rust storage. The latter are recognized from the
//! source shape alone — there is no `manual` flag — and are expanded by the existing `#[class]`
//! layer (`crate::class::expand`), unchanged:
//!
//! - `#[component(trait_only, inherits Path)] trait Name { .. }` — a Core interface;
//! - `#[component(struct_only = TraitPath, inherits Path)] struct Name { .. }` — a backend
//!   implementor of such an interface;
//! - a `struct` carrying `#[class]` shape attributes at the struct level (`#[prop(name: Type)]`,
//!   `#[text_style]`, `#[fixed(..)]`, `#[constructor_inputs(..)]`) — hand-written storage;
//! - and the paired `#[component] impl Name { .. }` of any of these, with `fn construct(..) -> Self`
//!   and the usual `#[class]` methods.
//!
//! Every other `struct` stays a declarative Component (`elwindui_codegen`), exactly as before.

use std::cell::RefCell;
use std::collections::HashMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;

/// What the paired `impl` needs from a class-backed struct to publish its constructor ABI.
#[derive(Clone)]
struct ClassBackedStruct {
    abstract_class: bool,
    /// `#[constructor_inputs(..)]`, when declared. Kept as text: tokens must not outlive the
    /// macro invocation that produced them.
    constructor_inputs: Option<Vec<String>>,
    /// `abi_props = <path>` of a `struct_only` implementor, as text.
    abi_props: Option<String>,
}

thread_local! {
    /// The class-backed `#[component]` structs expanded so far in this crate, so the paired
    /// `#[component] impl` takes the same class-backed path.
    static CLASS_BACKED: RefCell<HashMap<String, ClassBackedStruct>> = RefCell::new(HashMap::new());
}

/// `#[component(..)]`'s arguments: `inherits Path` (no `=`), and the class-backed modifiers.
#[derive(Default)]
pub(crate) struct ComponentArgs {
    pub(crate) base: Option<syn::Path>,
    pub(crate) trait_only: bool,
    pub(crate) struct_only: Option<syn::Path>,
    pub(crate) abi_props: Option<syn::Path>,
    pub(crate) abstract_class: bool,
    pub(crate) sealed: bool,
    pub(crate) no_ancestor_forward: bool,
}

impl ComponentArgs {
    /// Whether any argument other than `inherits` was given — they only exist for class-backed
    /// forms.
    fn has_class_modifiers(&self) -> bool {
        self.trait_only
            || self.struct_only.is_some()
            || self.abi_props.is_some()
            || self.abstract_class
            || self.sealed
            || self.no_ancestor_forward
    }

    /// The equivalent `#[class(..)]` argument list.
    fn class_attr(&self) -> TokenStream2 {
        let mut args: Vec<TokenStream2> = Vec::new();
        if let Some(base) = &self.base {
            args.push(quote! { inherits = #base });
        }
        if let Some(path) = &self.struct_only {
            args.push(quote! { struct_only = #path });
        }
        if self.trait_only {
            args.push(quote! { trait_only });
        }
        if self.abstract_class {
            args.push(quote! { abstract_class });
        }
        if self.sealed {
            args.push(quote! { sealed });
        }
        if self.no_ancestor_forward {
            args.push(quote! { no_ancestor_forward });
        }
        quote! { #(#args),* }
    }
}

pub(crate) fn parse_component_args(attr: TokenStream2) -> syn::Result<ComponentArgs> {
    use syn::parse::Parser;
    let mut args = ComponentArgs::default();
    if attr.is_empty() {
        return Ok(args);
    }
    (|input: syn::parse::ParseStream| {
        while !input.is_empty() {
            let kw: syn::Ident = input.parse()?;
            let duplicate = |set: bool| {
                if set {
                    Err(syn::Error::new(kw.span(), format!("duplicate `{kw}` argument")))
                } else {
                    Ok(())
                }
            };
            match kw.to_string().as_str() {
                "inherits" => {
                    duplicate(args.base.is_some())?;
                    args.base = Some(input.parse()?);
                }
                "trait_only" => {
                    duplicate(args.trait_only)?;
                    args.trait_only = true;
                }
                "struct_only" => {
                    duplicate(args.struct_only.is_some())?;
                    input.parse::<syn::Token![=]>()?;
                    args.struct_only = Some(input.parse()?);
                }
                "abi_props" => {
                    duplicate(args.abi_props.is_some())?;
                    input.parse::<syn::Token![=]>()?;
                    args.abi_props = Some(input.parse()?);
                }
                "abstract_class" => {
                    duplicate(args.abstract_class)?;
                    args.abstract_class = true;
                }
                "sealed" => {
                    duplicate(args.sealed)?;
                    args.sealed = true;
                }
                "no_ancestor_forward" => {
                    duplicate(args.no_ancestor_forward)?;
                    args.no_ancestor_forward = true;
                }
                "template" => {
                    return Err(syn::Error::new(
                        kw.span(),
                        "`template = <environment_key>` is no longer supported; declare `template: template_view!(|alias: Self| { ... })` and use EnvironmentContext::set_control_template::<Target>(...) for overrides",
                    ));
                }
                _ => {
                    return Err(syn::Error::new(
                        kw.span(),
                        format!(
                            "unknown argument `{kw}`; expected `inherits <Base>`, `trait_only`, \
                             `struct_only = <Trait>`, `abi_props = <macro path>`, `abstract_class`, \
                             `sealed` or `no_ancestor_forward`"
                        ),
                    ));
                }
            }
            if input.is_empty() {
                break;
            }
            input.parse::<syn::Token![,]>()?;
        }
        Ok(())
    })
    .parse2(attr)?;
    if args.trait_only && args.struct_only.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`trait_only` and `struct_only` are mutually exclusive",
        ));
    }
    if args.abi_props.is_some() && args.struct_only.is_none() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`abi_props` names the interface that declares a `struct_only` implementor's \
             properties; it is only valid together with `struct_only`",
        ));
    }
    Ok(args)
}

/// Struct-level `#[class]` shape attributes. `#[content(..)]` is deliberately not one of them: a
/// declarative Component also names its content field that way.
fn is_class_shape_attr(attr: &syn::Attribute) -> bool {
    ["prop", "text_style", "fixed", "constructor_inputs"]
        .iter()
        .any(|name| attr.path().is_ident(name))
}

/// Field-level attributes of a declarative Component.
const DECLARATIVE_FIELD_ATTRS: &[&str] = &[
    "param",
    "prop",
    "state",
    "observable",
    "bindable",
    "computed",
    "async_computed",
    "environment",
    "inject",
    "two_way",
    "routed",
    "attached",
    "onetime",
    "length",
    "semantic_brush",
    "overrides",
];

fn is_view_field(field: &syn::Field) -> bool {
    matches!(&field.ty, syn::Type::Macro(tm)
        if tm.mac.path.is_ident("view") || tm.mac.path.is_ident("template_view"))
}

/// Whether `item` is a class-backed struct (hand-written storage). A mix of the class-backed and
/// the declarative shape is an error rather than a guess.
pub(crate) fn classify_struct(args: &ComponentArgs, item: &syn::ItemStruct) -> syn::Result<bool> {
    if args.trait_only {
        return Err(syn::Error::new(
            item.ident.span(),
            "`trait_only` declares a Core interface and attaches to a `trait Name { .. }` item, \
             not a `struct`",
        ));
    }
    let class_shape_attr = item.attrs.iter().find(|attr| is_class_shape_attr(attr));
    let class_backed = args.struct_only.is_some() || class_shape_attr.is_some();
    if !class_backed {
        if args.has_class_modifiers() {
            return Err(syn::Error::new(
                item.ident.span(),
                "`abstract_class`, `sealed`, `no_ancestor_forward` and `abi_props` apply to a \
                 Component with hand-written storage (struct-level `#[prop(name: Type)]` or \
                 `struct_only`); a declarative Component does not take them",
            ));
        }
        return Ok(false);
    }
    let reason = if args.struct_only.is_some() {
        "`struct_only`"
    } else {
        "a struct-level class shape attribute"
    };
    for field in &item.fields {
        if is_view_field(field) {
            return Err(syn::Error::new(
                field.span(),
                format!(
                    "`{}` has hand-written storage ({reason}), so it cannot also declare a \
                     `view!`/`template_view!` field",
                    item.ident
                ),
            ));
        }
        if let Some(attr) = field.attrs.iter().find(|attr| {
            DECLARATIVE_FIELD_ATTRS
                .iter()
                .any(|name| attr.path().is_ident(name))
        }) {
            return Err(syn::Error::new(
                attr.span(),
                format!(
                    "`{}` has hand-written storage ({reason}); its fields are plain Rust storage, \
                     so a field-level DSL attribute cannot be mixed in — declare properties at the \
                     struct level (`#[prop(name: Type)]`) instead",
                    item.ident
                ),
            ));
        }
    }
    Ok(true)
}

/// Expands a class-backed `struct` through the `#[class]` layer.
pub(crate) fn expand_struct(args: &ComponentArgs, item: syn::ItemStruct) -> TokenStream2 {
    let mut constructor_inputs = None;
    for attr in &item.attrs {
        if attr.path().is_ident("constructor_inputs") {
            match attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated,
            ) {
                Ok(names) => {
                    constructor_inputs = Some(names.iter().map(|name| name.to_string()).collect())
                }
                Err(error) => return error.to_compile_error(),
            }
        }
    }
    let info = ClassBackedStruct {
        abstract_class: args.abstract_class,
        constructor_inputs,
        abi_props: args
            .abi_props
            .as_ref()
            .map(|path| quote! { #path }.to_string()),
    };
    CLASS_BACKED.with(|names| names.borrow_mut().insert(item.ident.to_string(), info));
    crate::class::expand(args.class_attr(), quote! { #item })
}

/// Expands a `trait` (a Core interface) through the `#[class]` layer.
pub(crate) fn expand_trait(args: &ComponentArgs, item: syn::ItemTrait) -> TokenStream2 {
    if !args.trait_only {
        return syn::Error::new(
            item.ident.span(),
            "a `trait` is a Core interface: write `#[component(trait_only, ..)]`",
        )
        .to_compile_error();
    }
    if args.abi_props.is_some() {
        return syn::Error::new(
            item.ident.span(),
            "`abi_props` is only valid on a `struct_only` implementor",
        )
        .to_compile_error();
    }
    crate::class::expand(args.class_attr(), quote! { #item })
}

fn impl_type_name(item: &syn::ItemImpl) -> Option<String> {
    match item.self_ty.as_ref() {
        syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// Whether the paired `impl` belongs to a class-backed `#[component]` struct. Without the struct
/// registration (an out-of-order rust-analyzer expansion), an impl that only a class-backed
/// Component can have — a `construct`, an untagged method, `#[inherent]` or `on_constructed` — is
/// taken as one.
pub(crate) fn is_class_backed_impl(item: &syn::ItemImpl) -> bool {
    let Some(name) = impl_type_name(item) else {
        return false;
    };
    if CLASS_BACKED.with(|names| names.borrow().contains_key(&name)) {
        return true;
    }
    if elwindui_codegen::component_frontend::registered_component_parts(&name).is_some() {
        return false;
    }
    item.items.iter().any(|impl_item| match impl_item {
        syn::ImplItem::Fn(f) => {
            f.sig.ident == "construct"
                || f.sig.ident == "on_constructed"
                || f.attrs.iter().any(|attr| attr.path().is_ident("inherent"))
                || !f.attrs.iter().any(|attr| {
                    attr.path().is_ident("overridable") || attr.path().is_ident("overrides")
                })
        }
        _ => false,
    })
}

/// D1 case (iii): a struct registered as a declarative Component that has no `view!`/
/// `template_view!` field and no field-level DSL attribute, paired with an `impl` that has a
/// `construct`, keeps its hand-written storage. Only the `impl` can tell, so the struct is
/// re-expanded here, through the `#[class]` layer, from its registered source.
pub(crate) fn late_class_backed_struct(item: &syn::ItemImpl) -> Option<syn::ItemStruct> {
    let name = impl_type_name(item)?;
    let has_construct = item
        .items
        .iter()
        .any(|impl_item| matches!(impl_item, syn::ImplItem::Fn(f) if f.sig.ident == "construct"));
    if !has_construct {
        return None;
    }
    let (_, item_struct) =
        elwindui_codegen::component_frontend::registered_component_struct(&name)?;
    let plain = item_struct.fields.iter().all(|field| {
        !is_view_field(field)
            && !field.attrs.iter().any(|attr| {
                DECLARATIVE_FIELD_ATTRS
                    .iter()
                    .any(|name| attr.path().is_ident(name))
            })
    });
    plain.then_some(item_struct)
}

/// Expands a case (iii) struct and its `impl`. The struct half already emitted the declarative
/// rust-analyzer shadow (rust-analyzer cannot see the `impl`'s `construct` when it expands the
/// struct), so this real expansion is limited to rustc to avoid a second definition there.
pub(crate) fn expand_late(item_struct: syn::ItemStruct, item: syn::ItemImpl) -> TokenStream2 {
    let name = item_struct.ident.to_string();
    let base = elwindui_codegen::component_frontend::registered_component_struct(&name)
        .and_then(|(base, _)| base);
    elwindui_codegen::component_frontend::unregister_same_crate_component(&name);
    let args = ComponentArgs {
        base: match base.as_deref().map(syn::parse_str::<syn::Path>) {
            Some(Ok(path)) => Some(path),
            Some(Err(error)) => return error.to_compile_error(),
            None => None,
        },
        ..ComponentArgs::default()
    };
    let mut tokens = expand_struct(&args, item_struct);
    tokens.extend(expand_impl(item));
    let Ok(file) = syn::parse2::<syn::File>(tokens.clone()) else {
        return tokens;
    };
    let items = file.items.into_iter().map(|item| {
        quote! {
            #[cfg(not(rust_analyzer))]
            #[allow(unexpected_cfgs)]
            #item
        }
    });
    quote! { #(#items)* }
}

/// D2: the item-local rules for a class-backed Component's `construct`.
fn validate_construct(item: &syn::ItemImpl) -> syn::Result<()> {
    let mut seen = false;
    for impl_item in &item.items {
        let syn::ImplItem::Fn(f) = impl_item else {
            continue;
        };
        if f.sig.ident != "construct" {
            continue;
        }
        let fail = |span: proc_macro2::Span, message: &str| {
            Err(syn::Error::new(span, format!("`construct`: {message}")))
        };
        if seen {
            return fail(f.sig.ident.span(), "declared more than once");
        }
        seen = true;
        if let Some(attr) = f
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("overridable") || attr.path().is_ident("overrides"))
        {
            return fail(
                attr.span(),
                "is the constructor, not a virtual method — remove `#[overridable]`/`#[overrides]`",
            );
        }
        if let Some(receiver) = f.sig.receiver() {
            return fail(
                receiver.span(),
                "takes no `self` receiver; it returns the new `Self`",
            );
        }
        if let Some(asyncness) = &f.sig.asyncness {
            return fail(asyncness.span(), "cannot be `async`");
        }
        if let Some(unsafety) = &f.sig.unsafety {
            return fail(unsafety.span(), "cannot be `unsafe`");
        }
        if !f.sig.generics.params.is_empty() || f.sig.generics.where_clause.is_some() {
            return fail(f.sig.generics.span(), "cannot be generic");
        }
        let returns_self = matches!(&f.sig.output, syn::ReturnType::Type(_, ty)
            if matches!(ty.as_ref(), syn::Type::Path(p) if p.qself.is_none() && p.path.is_ident("Self")));
        if !returns_self {
            return fail(f.sig.output.span(), "must return `Self`");
        }
    }
    Ok(())
}

/// Expands a class-backed `impl` through the `#[class]` layer, and publishes the type's
/// constructor ABI (D3).
pub(crate) fn expand_impl(item: syn::ItemImpl) -> TokenStream2 {
    if let Err(error) = validate_construct(&item) {
        return error.to_compile_error();
    }
    let abi = match constructor_abi(&item) {
        Ok(abi) => abi,
        Err(error) => error.to_compile_error(),
    };
    let class = crate::class::expand(TokenStream2::new(), quote! { #item });
    quote! {
        #class
        #abi
    }
}

/// D3: the constructor ABI of a class-backed Component. Its public inputs are exactly the
/// `construct` arguments listed by `#[constructor_inputs(..)]`, in order; a `construct` with
/// arguments and no such declaration is internal-only (a native handle, say), and an abstract
/// class has no public constructor at all.
fn constructor_abi(item: &syn::ItemImpl) -> syn::Result<TokenStream2> {
    let Some(name) = impl_type_name(item) else {
        return Ok(TokenStream2::new());
    };
    // Without the struct registration (an out-of-order rust-analyzer expansion) nothing is known
    // about the type's public shape. Real builds always expand the struct first; rust-analyzer
    // only needs the macro name to exist, so re-exports of it resolve.
    let Some(info) = CLASS_BACKED.with(|names| names.borrow().get(&name).cloned()) else {
        let macro_ident = quote::format_ident!("__elwindui_ctor_{name}");
        return Ok(quote! {
            #[cfg(rust_analyzer)]
            #[allow(unexpected_cfgs)]
            #[doc(hidden)]
            #[macro_export]
            #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
            macro_rules! #macro_ident {
                ($($tokens:tt)*) => {};
            }
        });
    };
    let construct = item.items.iter().find_map(|impl_item| match impl_item {
        syn::ImplItem::Fn(f) if f.sig.ident == "construct" => Some(f),
        _ => None,
    });
    let Some(construct) = construct else {
        return Ok(TokenStream2::new());
    };
    if info.abstract_class {
        return Ok(elwindui_codegen::codegen::unconstructible_class_abi(
            &name,
            "it is an abstract class",
        ));
    }
    let mut args: Vec<(syn::Ident, syn::Type)> = Vec::new();
    for input in &construct.sig.inputs {
        let syn::FnArg::Typed(typed) = input else {
            continue;
        };
        let syn::Pat::Ident(pat) = typed.pat.as_ref() else {
            return Err(syn::Error::new(
                typed.pat.span(),
                "`construct`: a constructor argument must be a plain name",
            ));
        };
        args.push((pat.ident.clone(), (*typed.ty).clone()));
    }
    match &info.constructor_inputs {
        Some(declared) => {
            let names: Vec<String> = args.iter().map(|(n, _)| n.to_string()).collect();
            let declared_names: Vec<String> = declared.clone();
            if names != declared_names {
                return Err(syn::Error::new(
                    construct.sig.ident.span(),
                    format!(
                        "`construct`'s arguments ({}) must be exactly the declared \
                         `#[constructor_inputs({})]`, in the same order",
                        names.join(", "),
                        declared_names.join(", ")
                    ),
                ));
            }
        }
        None if !args.is_empty() => {
            return Ok(elwindui_codegen::codegen::unconstructible_class_abi(
                &name,
                "its `construct` takes internal arguments that are not published with \
                 `#[constructor_inputs(..)]`",
            ));
        }
        None => {}
    }
    let (props_macro, abi_props_reexport) = match &info.abi_props {
        Some(path) => {
            let path: syn::Path = syn::parse_str(path)?;
            let alias = quote::format_ident!("__elwindui_abi_props_{name}");
            (
                quote! { $crate::#alias },
                quote! {
                    #[doc(hidden)]
                    #[allow(unused_imports)]
                    pub use #path as #alias;
                },
            )
        }
        None => {
            let own = quote::format_ident!("__elwindui_props_{name}");
            (quote! { $crate::#own }, TokenStream2::new())
        }
    };
    let abi = elwindui_codegen::codegen::class_constructor_abi(&name, &args, props_macro);
    Ok(quote! {
        #abi_props_reexport
        #abi
    })
}
