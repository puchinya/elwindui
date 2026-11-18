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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;

/// What the paired `impl` needs from a class-backed struct to publish its constructor ABI.
#[derive(Clone)]
struct ClassBackedStruct {
    abstract_class: bool,
    /// Whether the class `inherits` a base (whose embedded generated levels it must drive).
    has_base: bool,
    /// Whether that base is a generated Component (see `base_is_generated`): then the `#[class]`
    /// expansion is rustc-only and rust-analyzer gets `rust_analyzer_shadow` instead.
    base_generated: bool,
    /// The base path and the struct source, for that shadow (as text).
    base_path: Option<String>,
    struct_src: String,
    /// `#[constructor_inputs(..)]`, when declared. Kept as text: tokens must not outlive the
    /// macro invocation that produced them.
    constructor_inputs: Option<Vec<String>>,
    /// `abi_props = <path>` of a `struct_only` implementor, as text.
    abi_props: Option<String>,
}

/// The crate being compiled, so the process-wide registries below (rust-analyzer's proc-macro
/// server expands on several threads, for several crates) keep crates apart.
fn crate_key() -> String {
    std::env::var("CARGO_CRATE_NAME")
        .or_else(|_| std::env::var("CARGO_PKG_NAME"))
        .unwrap_or_default()
}

/// D1 case (iii): the declarative output (the rust-analyzer shadow) of a struct whose kind only
/// its `impl` decides, held back until that `impl` is expanded. Kept as text: tokens must not
/// outlive the macro invocation that produced them.
fn deferred_shadows() -> &'static Mutex<HashMap<(String, String), String>> {
    static SHADOWS: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    SHADOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// D1 case (iii): whether `item` might still turn out to have hand-written storage — no
/// `view!`/`template_view!` field and no field-level DSL attribute — so only its `impl` decides.
pub(crate) fn is_undecided_struct(item: &syn::ItemStruct) -> bool {
    item.fields.iter().all(|field| {
        !is_view_field(field)
            && !field.attrs.iter().any(|attr| {
                DECLARATIVE_FIELD_ATTRS
                    .iter()
                    .any(|name| attr.path().is_ident(name))
            })
    })
}

/// Holds back an undecided struct's declarative output until its `impl` is expanded.
pub(crate) fn defer_struct_shadow(name: &str, tokens: &TokenStream2) {
    deferred_shadows()
        .lock()
        .unwrap()
        .insert((crate_key(), name.to_string()), tokens.to_string());
}

/// The held-back declarative output of `name`, for its declarative `impl`.
pub(crate) fn take_deferred_shadow(item: &syn::ItemImpl) -> TokenStream2 {
    let Some(name) = impl_type_name(item) else {
        return TokenStream2::new();
    };
    deferred_shadows()
        .lock()
        .unwrap()
        .remove(&(crate_key(), name))
        .and_then(|text| text.parse().ok())
        .unwrap_or_default()
}

/// The class-backed `#[component]` structs expanded so far, by crate and name, so the paired
/// `#[component] impl` takes the same class-backed path.
fn class_backed() -> &'static Mutex<HashMap<(String, String), ClassBackedStruct>> {
    static CLASS_BACKED: OnceLock<Mutex<HashMap<(String, String), ClassBackedStruct>>> =
        OnceLock::new();
    CLASS_BACKED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn class_backed_info(name: &str) -> Option<ClassBackedStruct> {
    class_backed()
        .lock()
        .unwrap()
        .get(&(crate_key(), name.to_string()))
        .cloned()
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
    let base_generated = args.base.as_ref().is_some_and(base_is_generated);
    let info = ClassBackedStruct {
        abstract_class: args.abstract_class,
        has_base: args.base.is_some(),
        base_generated,
        base_path: args.base.as_ref().map(|path| quote! { #path }.to_string()),
        struct_src: quote! { #item }.to_string(),
        constructor_inputs,
        abi_props: args
            .abi_props
            .as_ref()
            .map(|path| quote! { #path }.to_string()),
    };
    class_backed()
        .lock()
        .unwrap()
        .insert((crate_key(), item.ident.to_string()), info);
    let expanded = crate::class::expand(args.class_attr(), quote! { #item });
    if base_generated {
        let expanded = rustc_only(expanded);
        let shape_stub = rust_analyzer_shape_stub(
            &item.ident,
            &item.attrs,
            args.base.as_ref().expect("base_generated implies a base"),
        );
        quote! {
            #expanded
            #shape_stub
        }
    } else {
        expanded
    }
}

/// rust-analyzer's stand-in for the property-shape macro of a class-backed Component over a
/// generated base (its real `#[class]` items are rustc-only): a further-derived generated
/// Component asks it for the required constructor inputs (`#[constructor_inputs]`); any other
/// query is answered loosely, which is enough for analysis.
fn rust_analyzer_shape_stub(
    ident: &syn::Ident,
    attrs: &[syn::Attribute],
    base: &syn::Path,
) -> TokenStream2 {
    let inputs: Vec<syn::Ident> = attrs
        .iter()
        .find(|attr| attr.path().is_ident("constructor_inputs"))
        .and_then(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated,
            )
            .ok()
        })
        .map(|names| names.into_iter().collect())
        .unwrap_or_default();
    let macro_ident = quote::format_ident!("__elwindui_props_{ident}");
    // The class hierarchy surface a further-derived Component's own shadow expects, as the
    // generated shadow provides it: `{Name}Ext`, the inherit macro and its module, and the template
    // capability macro, each forwarding to the base's.
    let base_name = base
        .segments
        .last()
        .expect("non-empty base path")
        .ident
        .clone();
    let ext = quote::format_ident!("{ident}Ext");
    let inherit = quote::format_ident!("__elwindui_inherit_{ident}");
    let macros_of = quote::format_ident!("__elwindui_macros_of_{ident}");
    let capability = quote::format_ident!("__elwindui_template_capabilities_{ident}");
    let with_last = |last: syn::Ident| {
        let mut path = base.clone();
        path.segments.last_mut().expect("non-empty base path").ident = last;
        path
    };
    let base_ext = with_last(quote::format_ident!("{base_name}Ext"));
    let base_macros_of = {
        let mut path = base.clone();
        path.segments.pop();
        path.segments
            .push(syn::PathSegment::from(quote::format_ident!(
                "__elwindui_macros_of_{base_name}"
            )));
        path
    };
    let base_root_macro = |name: syn::Ident| {
        let first = base
            .segments
            .first()
            .expect("non-empty base path")
            .ident
            .clone();
        if base.segments.len() == 1 {
            quote! { #name }
        } else {
            quote! { #first::#name }
        }
    };
    let base_capability = base_root_macro(quote::format_ident!(
        "__elwindui_template_capabilities_{base_name}"
    ));
    // Inside an exported `macro_rules!`, a crate-relative path names the defining crate.
    let exported = |tokens: TokenStream2| {
        let text = tokens.to_string();
        match text.strip_prefix("crate ") {
            Some(rest) => format!("$crate {rest}").parse().unwrap_or(tokens),
            None => tokens,
        }
    };
    let inherit_base = quote::format_ident!("__elwindui_inherit_{base_name}");
    let base_inherit = exported(quote! { #base_macros_of::#inherit_base });
    let base_alias = exported(quote! { #base_macros_of::__ElwindUIOwnExt });
    let base_concrete = exported(quote! { #base });
    let base_trait = exported(quote! { #base_ext });
    let base_capability_exported = exported(base_capability.clone());
    quote! {
        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #macro_ident {
            (@required_params $cb:path, { $($item:tt)* }) => {
                $cb! { { $($item)* } [#(#inputs),*] }
            };
            (@field_type $name:ident) => { () };
            ($($tokens:tt)*) => {};
        }

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        pub trait #ext: #base_ext {}

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        impl #ext for #ident {}

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        #base_macros_of::#inherit_base!(#ident, #base_macros_of::__ElwindUIOwnExt, #base, #base, #base_ext;);

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #inherit {
            ($SubType:ty, $OwnTrait:path, $OwnConcrete:path, $RootConcrete:path, $BridgePath:path; $($overrides:tt)*) => {
                impl $OwnTrait for $SubType {}
                #base_inherit!($SubType, #base_alias, #base_concrete, #base_concrete, #base_trait;);
            };
        }

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        #[doc(hidden)]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        pub mod #macros_of {
            pub use crate::#inherit;
            pub use super::#ext as __ElwindUIOwnExt;
        }

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #capability {
            (@impl $target:ty) => {
                #base_capability_exported!(@impl $target);
            };
            (@if_control { $($tokens:tt)* }) => {
                #base_capability_exported!(@if_control { $($tokens)* });
            };
        }
    }
}

/// Whether a class-backed Component's base is a generated Component: a same-crate one is known
/// from the Component registry; a framework path (`elwindui`, `elwindui_core`) is a builtin; any
/// other crate's type is taken as generated. A generated Component's own `#[class]` items are
/// rustc-only (rust-analyzer sees its shadow), so a class over it cannot expand under
/// rust-analyzer either.
fn base_is_generated(base: &syn::Path) -> bool {
    let first = base.segments.first().map(|s| s.ident.to_string());
    let last = base.segments.last().map(|s| s.ident.to_string());
    let registered = || {
        last.as_deref().is_some_and(|name| {
            elwindui_codegen::component_frontend::registered_component_parts(name).is_some()
        })
    };
    match first.as_deref() {
        _ if base.segments.len() == 1 => registered(),
        Some("crate" | "self" | "super") => registered(),
        Some("elwindui" | "elwindui_core") => false,
        _ => true,
    }
}

/// Limits every item of `tokens` to rustc.
fn rustc_only(tokens: TokenStream2) -> TokenStream2 {
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

/// rust-analyzer's view of a class-backed Component over a generated base: the struct with its
/// `base`, `new`, the hand-written methods as inherent methods, and `Deref` to the base.
fn rust_analyzer_shadow(info: &ClassBackedStruct, item: &syn::ItemImpl) -> TokenStream2 {
    let (Ok(mut item_struct), Some(Ok(base))) = (
        syn::parse_str::<syn::ItemStruct>(&info.struct_src),
        info.base_path.as_deref().map(syn::parse_str::<syn::Path>),
    ) else {
        return TokenStream2::new();
    };
    item_struct
        .attrs
        .retain(|attr| !is_class_shape_attr(attr) && !attr.path().is_ident("content"));
    let ident = &item_struct.ident;
    let vis = &item_struct.vis;
    let attrs = &item_struct.attrs;
    let fields = match &item_struct.fields {
        syn::Fields::Named(named) => named.named.iter().collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let mut new_inputs = TokenStream2::new();
    let mut methods = Vec::new();
    for impl_item in &item.items {
        let syn::ImplItem::Fn(f) = impl_item else {
            continue;
        };
        if f.sig.ident == "construct" {
            let inputs = &f.sig.inputs;
            new_inputs = quote! { #inputs };
            continue;
        }
        if f.sig.ident == "on_constructed" {
            continue;
        }
        let mut f = f.clone();
        f.attrs.retain(|attr| {
            !["overrides", "overridable", "inherent"]
                .iter()
                .any(|name| attr.path().is_ident(name))
        });
        f.vis = syn::parse_quote! { pub };
        methods.push(f);
    }
    quote! {
        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs, dead_code)]
        #(#attrs)*
        #vis struct #ident {
            pub base: #base,
            #(#fields,)*
        }

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs, dead_code, unused_variables)]
        impl #ident {
            pub fn new(#new_inputs) -> std::rc::Rc<Self> {
                unimplemented!()
            }
            #(#methods)*
        }

        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        impl std::ops::Deref for #ident {
            type Target = #base;
            fn deref(&self) -> &Self::Target {
                &self.base
            }
        }
    }
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
    if class_backed_info(&name).is_some() {
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

/// Expands a case (iii) struct and its `impl`. The struct half deferred its declarative
/// rust-analyzer shadow (`defer_struct_shadow`), which is dropped here, so the class-backed
/// expansion is what both rustc and rust-analyzer see.
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
    deferred_shadows()
        .lock()
        .unwrap()
        .remove(&(crate_key(), name.clone()));
    let mut tokens = expand_struct(&args, item_struct);
    tokens.extend(expand_impl(item));
    tokens
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
pub(crate) fn expand_impl(mut item: syn::ItemImpl) -> TokenStream2 {
    if let Err(error) = validate_construct(&item) {
        return error.to_compile_error();
    }
    let abi = match constructor_abi(&item) {
        Ok(abi) => {
            if !abi.is_empty()
                && let Some(name) = impl_type_name(&item)
            {
                elwindui_codegen::component_frontend::register_class_backed_constructor(&name);
            }
            abi
        }
        Err(error) => error.to_compile_error(),
    };
    let info = impl_type_name(&item).and_then(|name| class_backed_info(&name));
    let shadow = info
        .as_ref()
        .filter(|info| info.base_generated)
        .map(|info| rust_analyzer_shadow(info, &item));
    let embedded = embedded_base_support(&mut item);
    let class = crate::class::expand(TokenStream2::new(), quote! { #item });
    let expanded = quote! {
        #class
        #abi
        #embedded
    };
    match shadow {
        Some(shadow) => {
            let expanded = rustc_only(expanded);
            quote! {
                #expanded
                #shadow
            }
        }
        None => expanded,
    }
}

/// T8: a class-backed Component between generated levels. A generated Component embedded as a
/// `base` is mounted, wired and subscribed by the level that embeds it (`EmbeddedViewSync`), one
/// level at a time. A class-backed level forwards those calls to its own `base`; and when it is
/// itself the outermost object, it starts them once it is attached (its first visual parent), as a
/// generated outermost Component does from its own `mount`.
fn embedded_base_support(item: &mut syn::ItemImpl) -> TokenStream2 {
    let Some(name) = impl_type_name(item) else {
        return TokenStream2::new();
    };
    let has_base = class_backed_info(&name).is_some_and(|info| info.has_base);
    if !has_base {
        return TokenStream2::new();
    }
    let core = crate::class::core_path();
    let ty = &item.self_ty;
    let start_when_outermost: syn::Stmt = syn::parse_quote! {
        {
            if let Some(__elwindui_owner) =
                #core::ui::UIElementExt::as_ui_element(self).visual_collection.owner_rc()
            {
                let __elwindui_owner = std::rc::Rc::downgrade(&__elwindui_owner);
                #core::ui::UIElementExt::add_mount_hook(
                    self,
                    Box::new(move || {
                        let Some(__elwindui_owner) = __elwindui_owner.upgrade() else {
                            return;
                        };
                        let __elwindui_owner: std::rc::Rc<dyn std::any::Any> = __elwindui_owner;
                        // Embedded in a further-derived level, that level drives this one.
                        let Ok(__elwindui_this) = __elwindui_owner.downcast::<#ty>() else {
                            return;
                        };
                        let __elwindui_handle =
                            #core::reactive::EmbeddedHandle::new(std::rc::Rc::clone(&__elwindui_this));
                        let __elwindui_environment =
                            #core::ui::UIElementExt::effective_environment(&*__elwindui_this);
                        #core::reactive::EmbeddedViewSync::__elwindui_mount_as_base(
                            &*__elwindui_this,
                            Some(#core::reactive::WiringOwner::downgrade_wiring(&__elwindui_handle)),
                            __elwindui_environment,
                        );
                        #core::reactive::EmbeddedViewSync::__elwindui_wire_embedded(
                            &*__elwindui_this,
                            __elwindui_handle.clone(),
                        );
                        #core::reactive::EmbeddedViewSync::__elwindui_mount_embedded(
                            &*__elwindui_this,
                            __elwindui_handle,
                        );
                    }),
                );
            }
        }
    };
    let existing = item.items.iter_mut().find_map(|impl_item| match impl_item {
        syn::ImplItem::Fn(f) if f.sig.ident == "on_constructed" => Some(f),
        _ => None,
    });
    match existing {
        Some(on_constructed) => on_constructed.block.stmts.insert(0, start_when_outermost),
        None => item.items.push(syn::parse_quote! {
            fn on_constructed(&self) {
                #start_when_outermost
            }
        }),
    }
    let forward_use = quote! {
        #[allow(unused_imports)]
        use #core::reactive::{EmbeddedNotifyForward as _, EmbeddedNotifyIgnore as _};
    };
    quote! {
        impl #core::reactive::EmbeddedViewSync for #ty {
            fn __elwindui_embedded_property_changed(&self, property: &'static str) {
                #forward_use
                (&#core::reactive::EmbeddedNotify(&self.base)).notify_embedded(property);
            }

            fn __elwindui_mount_as_base(
                &self,
                handle: Option<#core::reactive::WeakEmbeddedHandle<Self>>,
                environment: #core::environment::EnvironmentContext,
            ) {
                #forward_use
                let base_handle = handle
                    .and_then(|handle| #core::reactive::WiringWeak::upgrade_wiring(&handle))
                    .map(|handle| {
                        #core::reactive::WiringOwner::downgrade_wiring(
                            &handle.map(|level| &level.base),
                        )
                    });
                (&#core::reactive::EmbeddedNotify(&self.base)).mount_as_base(base_handle, environment);
            }

            fn __elwindui_wire_embedded(&self, this: #core::reactive::EmbeddedHandle<Self>) {
                #forward_use
                (&#core::reactive::EmbeddedNotify(&self.base))
                    .wire_embedded(this.map(|level| &level.base));
            }

            fn __elwindui_mount_embedded(&self, this: #core::reactive::EmbeddedHandle<Self>) {
                #forward_use
                (&#core::reactive::EmbeddedNotify(&self.base))
                    .mount_embedded(this.map(|level| &level.base));
            }

            fn __elwindui_root_content_span(&self) -> usize {
                #forward_use
                (&#core::reactive::EmbeddedNotify(&self.base)).embedded_span()
            }
        }
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
    // Without the struct registration (an out-of-order rust-analyzer expansion; real builds always
    // expand the struct first) the struct-side facts are unknown: assume a concrete class without
    // `#[constructor_inputs]` or `abi_props`, which is enough for analysis.
    let info = class_backed_info(&name).unwrap_or(ClassBackedStruct {
        abstract_class: false,
        has_base: false,
        base_generated: false,
        base_path: None,
        struct_src: String::new(),
        constructor_inputs: None,
        abi_props: None,
    });
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
    // `@environment` calls this on the type: its path to Core resolves here, in the defining
    // crate (`crate` inside Core itself).
    let application_environment = elwindui_codegen::codegen::application_environment_fn(
        &quote::format_ident!("{name}"),
        &crate::class::core_path(),
    );
    let abi = elwindui_codegen::codegen::class_constructor_abi(&name, &args, props_macro);
    Ok(quote! {
        #abi_props_reexport
        #application_environment
        #abi
    })
}
