//! Rust-only frontend for `#[elwindui::theme]`.
//!
//! Mirrors `environment_frontend.rs`'s shape: a Theme declaration does not enter the DSL AST/parser
//! either. This module consumes a `syn::ItemStruct` whose fields are schema only (never stored —
//! the emitted type is a zero-sized marker) and emits a `Theme` impl that batches
//! `EnvironmentContext::set` calls, one per `#[theme(value = ..)]` field, resolving each field's own
//! identifier through the **writable** Environment Key resolver
//! (`component_frontend::lookup_writable_environment_key`) — a same-crate `#[elwindui::environment_key]`
//! declaration or a framework Semantic Style Brush key, but never the framework's read-only
//! `popup_dismiss` builtin (see that resolver's own doc comment).
//!
//! See `docs/specs/theme_environment_spec.md` §2/§3/§4 and
//! `docs/design/runtime/theme_environment_design.md` (`## Theme`).
//!
//! Issue #146: `generate_theme_from_item_struct` splits into an item-local phase
//! (`item_local_theme_fields` — struct/field shape, always an unconditional error on both
//! rust-analyzer and real `rustc`) and a registry-dependent phase (`resolve_theme_set_calls` — each
//! field's writable Environment Key resolution, real-generation-only, gated to
//! `cfg(not(rust_analyzer))` so a spurious same-crate registry miss under rust-analyzer's own
//! incomplete expansion order never blanks out this Theme's own name/type resolution there). See
//! `rust_analyzer_shadow::build_theme_shadow` and `docs/design/tools/codegen_design.md` §3.2a.

use crate::component_frontend::lookup_writable_environment_key;
use crate::rust_analyzer_shadow::{build_theme_shadow, gate_real_items_for_rustc};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::HashSet;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, Field, ItemStruct, Token};

fn field_value_expr(field: &Field) -> Result<Expr, String> {
    let attributes: Vec<&Attribute> = field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("theme"))
        .collect();
    let ident = field
        .ident
        .as_ref()
        .ok_or_else(|| "theme field must be named".to_string())?;
    if attributes.len() != 1 {
        return Err(format!(
            "`{ident}`: each theme field requires exactly one `#[theme(value = ..)]` attribute"
        ));
    }

    let entries = attributes[0]
        .parse_args_with(Punctuated::<syn::MetaNameValue, Token![,]>::parse_terminated)
        .map_err(|error| format!("`{ident}`: {error}"))?;
    let mut value = None;
    for entry in entries {
        let Some(key) = entry.path.get_ident() else {
            return Err(format!("`{ident}`: theme attribute key must be `value`"));
        };
        if key != "value" {
            return Err(format!("`{ident}`: expected `value = ..`, found `{key}`"));
        }
        if value.replace(entry.value).is_some() {
            return Err(format!("`{ident}`: duplicate `value` argument"));
        }
    }
    value.ok_or_else(|| format!("`{ident}`: missing `value = ..`"))
}

/// Issue #146: item-local structural validation only — attribute syntax, struct shape, duplicate
/// field names, a malformed `#[theme(value = ..)]`. None of this depends on the same-crate
/// Environment Key registry, so it stays an unconditional error under rust-analyzer exactly as under
/// real `rustc` (`docs/design/tools/codegen_design.md` §3.2a's item-local/registry-dependent split).
/// Returns each field's own identifier and parsed `value` expression, in declaration order.
fn item_local_theme_fields(item: &ItemStruct) -> Result<Vec<(syn::Ident, Expr)>, String> {
    let fields = match &item.fields {
        syn::Fields::Named(fields) => &fields.named,
        syn::Fields::Unit => {
            return Err("a theme must declare at least one `#[theme(value = ..)]` field".into());
        }
        _ => return Err("a theme must be a struct with named fields".into()),
    };

    let mut field_names = HashSet::new();
    let mut out = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| "theme field must be named".to_string())?;
        if !field_names.insert(ident.to_string()) {
            return Err(format!("duplicate theme field `{ident}`"));
        }
        let value = field_value_expr(field)?;
        out.push((ident, value));
    }
    Ok(out)
}

/// Resolves every field's writable Environment Key and builds its `env.set::<K>(value)` call — the
/// registry-dependent half of theme generation. Fails on the first field whose key can't be
/// resolved (same-crate registry miss under rust-analyzer's own incomplete expansion order, or a
/// genuine missing/misspelled declaration under real `rustc` — see this module's own dual-expansion
/// split for why both share one error path here and are only told apart by which `cfg` branch a
/// caller routes this error message into).
fn resolve_theme_set_calls(fields: &[(syn::Ident, Expr)]) -> Result<Vec<TokenStream>, String> {
    let mut set_calls = Vec::new();
    for (ident, value) in fields {
        let (key_type_name, _value_type) = lookup_writable_environment_key(&ident.to_string())
            .ok_or_else(|| {
                format!(
                    "`{ident}`: no writable Environment Key named `{ident}` exists; declare a \
                     same-crate `#[elwindui::environment_key(name = {ident}, ..)]` or use a \
                     writable framework Semantic Style key (`theme_environment_spec.md` §7) — \
                     `popup_dismiss` in particular is framework-installed and read-only, not \
                     settable through a theme (`theme_environment_spec.md` §2)"
                )
            })?;
        let key_type: syn::Type = syn::parse_str(&key_type_name).map_err(|_| {
            format!("registered environment key type name `{key_type_name}` must parse")
        })?;
        set_calls.push(quote! {
            env.set::<#key_type>(#value);
        });
    }
    Ok(set_calls)
}

pub fn generate_theme_from_item_struct(
    args: TokenStream,
    item: &ItemStruct,
) -> Result<TokenStream, String> {
    if !args.is_empty() {
        return Err("`#[elwindui::theme]` takes no arguments".into());
    }
    if !item.generics.params.is_empty() {
        return Err("theme structs cannot be generic".into());
    }

    // Item-local (Issue #146): a malformed struct/field shape is a genuine mistake real generation
    // would also reject — propagated immediately, unconditionally, on both rust-analyzer and rustc.
    let fields = item_local_theme_fields(item)?;

    // The shadow is built unconditionally once the struct/field shape is known to be valid, entirely
    // independent of whether the registry-dependent step below succeeds — see `build_theme_shadow`'s
    // own doc comment for why it never needs the resolved `set_calls` at all.
    let shadow = build_theme_shadow(item)?;

    // Registry-dependent (Issue #146): a same-crate Environment Key miss here may be a spurious
    // rust-analyzer ordering artifact even when the source is correctly ordered — real generation
    // stays exactly as strict as before, just gated to `cfg(not(rust_analyzer))` so a real miss is
    // still a real `cargo build`/`cargo check` error while rust-analyzer keeps `shadow`'s own
    // resolution available regardless.
    let set_calls = match resolve_theme_set_calls(&fields) {
        Ok(set_calls) => set_calls,
        Err(error) => {
            let gated_error = quote! {
                #[cfg(not(rust_analyzer))]
                #[allow(unexpected_cfgs)]
                compile_error!(#error);
            };
            return Ok(quote! {
                #shadow
                #gated_error
            });
        }
    };

    let visibility = &item.vis;
    let name = &item.ident;
    let doc = format!(
        "A Theme Preset generated from the `{name}` declaration — see \
         `docs/specs/theme_environment_spec.md` §3/§4."
    );

    let real = gate_real_items_for_rustc(quote! {
        #[doc = #doc]
        #[derive(Debug, Clone, Copy, Default)]
        #visibility struct #name;

        impl elwindui::core::theme::Theme for #name {
            fn apply(&self, env: &elwindui::core::environment::EnvironmentContext) {
                #(#set_calls)*
            }
        }
    })?;

    Ok(quote! {
        #real
        #shadow
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component_frontend::register_same_crate_environment_key;

    #[test]
    fn generates_theme_impl_from_registered_environment_keys() {
        register_same_crate_environment_key(
            "brand_for_test_generates_theme_impl",
            "BrandEnvironmentForTestGeneratesThemeImpl",
            "Brush",
        )
        .unwrap();
        let item: ItemStruct = syn::parse_quote! {
            struct OceanTheme {
                #[theme(value = Brush::Solid(Color::rgb(0, 166, 200)))]
                brand_for_test_generates_theme_impl: Brush,
            }
        };
        let output = generate_theme_from_item_struct(TokenStream::new(), &item)
            .unwrap()
            .to_string();
        assert!(output.contains("struct OceanTheme"));
        assert!(output.contains("impl elwindui :: core :: theme :: Theme for OceanTheme"));
        assert!(output.contains("env . set :: < BrandEnvironmentForTestGeneratesThemeImpl > ("));
        // Issue #146: real items gated to `cfg(not(rust_analyzer))`, plus a no-op RA-only shadow —
        // see `rust_analyzer_shadow::build_theme_shadow`.
        assert!(output.contains("cfg (not (rust_analyzer))"), "{output}");
        assert!(output.contains("cfg (rust_analyzer)"), "{output}");
        assert!(output.contains("fn apply"), "{output}");
    }

    /// Issue #146, T6: a same-crate custom Environment Key miss stays a real (gated
    /// `cfg(not(rust_analyzer))`) `compile_error!` — exactly the existing diagnostic text — while a
    /// no-op Theme shadow is still emitted for rust-analyzer, so a spurious registry-ordering miss
    /// under rust-analyzer never blanks out this Theme's own name/type resolution.
    #[test]
    fn rejects_field_with_no_registered_environment_key() {
        let item: ItemStruct = syn::parse_quote! {
            struct OceanTheme {
                #[theme(value = 1.0)]
                unregistered_field_for_test_rejects_missing_key: f32,
            }
        };
        let output = generate_theme_from_item_struct(TokenStream::new(), &item)
            .expect("a registry-dependent miss must not fail the whole macro expansion")
            .to_string();
        assert!(output.contains("cfg (not (rust_analyzer))"), "{output}");
        assert!(output.contains("compile_error !"), "{output}");
        assert!(
            output.contains("no writable Environment Key named"),
            "{output}"
        );
        assert!(
            output.contains("cfg (rust_analyzer)") && output.contains("struct OceanTheme"),
            "a Theme shadow must still be emitted: {output}"
        );
    }

    #[test]
    fn rejects_theme_field_writing_the_popup_dismiss_builtin_key() {
        // Distinct from `rejects_field_with_no_registered_environment_key`: `popup_dismiss` DOES
        // resolve (it's a real, readable framework built-in key), it just isn't writable — a Theme
        // must not be able to set the framework-installed active `PopupDismissAction`.
        let item: ItemStruct = syn::parse_quote! {
            struct OceanTheme {
                #[theme(value = None)]
                popup_dismiss: Option<i32>,
            }
        };
        let output = generate_theme_from_item_struct(TokenStream::new(), &item)
            .expect("a registry-dependent miss must not fail the whole macro expansion")
            .to_string();
        assert!(
            output.contains("popup_dismiss") && output.contains("read-only"),
            "error should explain popup_dismiss is framework-installed and read-only, not just \
             \"unregistered\": {output}"
        );
        assert!(output.contains("cfg (not (rust_analyzer))"), "{output}");
    }

    #[test]
    fn rejects_field_missing_value_argument() {
        let item: ItemStruct = syn::parse_quote! {
            struct OceanTheme {
                #[theme()]
                brand: Brush,
            }
        };
        let error = generate_theme_from_item_struct(TokenStream::new(), &item).unwrap_err();
        assert!(error.contains("missing `value"));
    }

    #[test]
    fn rejects_generic_theme_struct() {
        let item: ItemStruct = syn::parse_quote! {
            struct OceanTheme<T> {
                #[theme(value = 1.0)]
                brand: T,
            }
        };
        let error = generate_theme_from_item_struct(TokenStream::new(), &item).unwrap_err();
        assert!(error.contains("cannot be generic"));
    }
}
