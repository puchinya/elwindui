//! `elwindui::new!(<ClassName>(args))` — sugar for `ClassNameImpl::new(args)`, for hand-written user
//! code (`main.rs` and the like) that wants to construct a `#[class]`-managed type without having to
//! know/write out its `Impl`-suffixed name. The outer parens are the macro invocation's single
//! required delimiter (a bare `new!<ClassName>(args)` isn't valid Rust — a macro invocation's `!` may
//! only be followed by one delimited token tree, so `<ClassName>` can't sit outside it); `<...>` is
//! parsed back out of that one token tree here. Not used by codegen-generated code (`generate_view`
//! already emits `XImpl::new(args)` directly) — see docs/elwindui_spec.md 付録H.2.1a.
//!
//! Only the path's last segment is rewritten (mirroring `class::to_impl_name`/`class::base_impl_type`'s
//! same "already `Impl`-suffixed -> used as-is" heuristic) — the rest of the path (`elwindui::ui::`,
//! a module path, ...) is passed through untouched, so ordinary `use`/path resolution decides whether
//! the name refers to a builtin or a consumer-defined component. No process-global registry is
//! needed here (unlike `class.rs`'s `ancestor_registry`) since this macro never needs to know whether
//! a class is "virtual"/abstract/etc. — it just rewrites a name and forwards a call.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Path, Token, parenthesized};

pub fn expand(input: TokenStream) -> TokenStream {
    match syn::parse2::<NewCallInput>(input) {
        Ok(call) => {
            let mut path = call.path;
            if let Some(seg) = path.segments.last_mut() {
                let name = seg.ident.to_string();
                if !name.ends_with("Impl") {
                    seg.ident = syn::Ident::new(&format!("{name}Impl"), seg.ident.span());
                }
            }
            let args = call.args;
            quote! { #path::new(#args) }
        }
        Err(e) => e.to_compile_error(),
    }
}

struct NewCallInput {
    path: Path,
    args: Punctuated<Expr, Token![,]>,
}

impl Parse for NewCallInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![<]>()?;
        let path = input.parse::<Path>()?;
        input.parse::<Token![>]>()?;
        let content;
        parenthesized!(content in input);
        let args = content.parse_terminated(Expr::parse, Token![,])?;
        Ok(NewCallInput { path, args })
    }
}
