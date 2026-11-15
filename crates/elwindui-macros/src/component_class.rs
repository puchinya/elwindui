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
use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;

thread_local! {
    /// Names of the class-backed `#[component]` structs expanded so far in this crate, so the
    /// paired `#[component] impl` takes the same class-backed path.
    static CLASS_BACKED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
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
    CLASS_BACKED.with(|names| names.borrow_mut().insert(item.ident.to_string()));
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
    if CLASS_BACKED.with(|names| names.borrow().contains(&name)) {
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

/// Expands a class-backed `impl` through the `#[class]` layer.
pub(crate) fn expand_impl(item: syn::ItemImpl) -> TokenStream2 {
    if let Err(error) = validate_construct(&item) {
        return error.to_compile_error();
    }
    crate::class::expand(TokenStream2::new(), quote! { #item })
}
