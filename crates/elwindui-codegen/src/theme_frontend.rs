//! Rust-only frontend for `#[elwindui::theme]`.
//!
//! Mirrors `environment_frontend.rs`'s shape: a Theme declaration does not enter the DSL AST/parser
//! either. This module consumes a `syn::ItemStruct` whose fields are schema only (never stored —
//! the emitted type is a zero-sized marker) and emits a `Theme` impl that batches
//! `EnvironmentContext::set` calls, one per `#[theme(value = ..)]` field, resolving each field's own
//! identifier through the same same-crate registry `#[environment(name)]` resolves against
//! (`component_frontend::lookup_same_crate_environment_key`).
//!
//! See `docs/specs/theme_environment_spec.md` §3/§4 and
//! `docs/design/runtime/theme_environment_design.md` (`## Theme`).

use crate::component_frontend::lookup_environment_key;
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

    let fields = match &item.fields {
        syn::Fields::Named(fields) => &fields.named,
        syn::Fields::Unit => {
            return Err("a theme must declare at least one `#[theme(value = ..)]` field".into());
        }
        _ => return Err("a theme must be a struct with named fields".into()),
    };

    let mut field_names = HashSet::new();
    let mut set_calls = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| "theme field must be named".to_string())?;
        if !field_names.insert(ident.to_string()) {
            return Err(format!("duplicate theme field `{ident}`"));
        }
        let value = field_value_expr(field)?;
        let (key_type_name, _value_type) = lookup_environment_key(&ident.to_string())
            .ok_or_else(|| {
                format!(
                    "`{ident}`: no `#[elwindui::environment_key(name = {ident}, ..)]` was declared \
                     earlier in this crate — a theme field's name must match a declared Environment \
                     Key's `name`"
                )
            })?;
        let key_type: syn::Type = syn::parse_str(&key_type_name).map_err(|_| {
            format!("registered environment key type name `{key_type_name}` must parse")
        })?;
        set_calls.push(quote! {
            env.set::<#key_type>(#value);
        });
    }

    let visibility = &item.vis;
    let name = &item.ident;
    let doc = format!(
        "A Theme Preset generated from the `{name}` declaration — see \
         `docs/specs/theme_environment_spec.md` §3/§4."
    );

    Ok(quote! {
        #[doc = #doc]
        #[derive(Debug, Clone, Copy, Default)]
        #visibility struct #name;

        impl elwindui::core::theme::Theme for #name {
            fn apply(&self, env: &elwindui::core::environment::EnvironmentContext) {
                #(#set_calls)*
            }
        }
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
    }

    #[test]
    fn rejects_field_with_no_registered_environment_key() {
        let item: ItemStruct = syn::parse_quote! {
            struct OceanTheme {
                #[theme(value = 1.0)]
                unregistered_field_for_test_rejects_missing_key: f32,
            }
        };
        let error = generate_theme_from_item_struct(TokenStream::new(), &item).unwrap_err();
        assert!(error.contains("no `#[elwindui::environment_key"));
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
