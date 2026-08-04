//! Rust-only frontend for `#[elwindui::theme_definition]`.
//!
//! Theme declarations intentionally do not enter the DSL AST/parser. This module consumes a
//! `syn::ItemStruct` and emits the typed runtime adapter used by `theme!(Theme::token)`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{HashMap, HashSet};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, Field, Ident, ItemStruct, Path, Token, Type};

struct ThemeArgs {
    extends: Path,
    variants: Vec<Ident>,
}

impl Parse for ThemeArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut extends = None;
        let mut variants = None;
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            if key == "extends" {
                input.parse::<Token![=]>()?;
                if extends.replace(input.parse()?).is_some() {
                    return Err(syn::Error::new(key.span(), "duplicate `extends` argument"));
                }
            } else if key == "variants" {
                let content;
                syn::parenthesized!(content in input);
                let parsed = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
                    .into_iter()
                    .collect();
                if variants.replace(parsed).is_some() {
                    return Err(syn::Error::new(key.span(), "duplicate `variants` argument"));
                }
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "expected `extends = SystemTheme` or `variants(...)`",
                ));
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Self {
            extends: extends.ok_or_else(|| input.error("missing `extends = SystemTheme`"))?,
            variants: variants.ok_or_else(|| input.error("missing `variants(...)`"))?,
        })
    }
}

#[derive(Default)]
struct FieldValues {
    default: Option<Expr>,
    variants: HashMap<String, Expr>,
}

fn parse_field_values(field: &Field, variants: &HashSet<String>) -> syn::Result<FieldValues> {
    let attributes: Vec<&Attribute> = field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("theme"))
        .collect();
    if attributes.len() != 1 {
        return Err(syn::Error::new_spanned(
            field,
            "each theme field requires exactly one `#[theme(...)]` attribute",
        ));
    }

    let entries = attributes[0]
        .parse_args_with(Punctuated::<syn::MetaNameValue, Token![,]>::parse_terminated)?;
    let mut result = FieldValues::default();
    for entry in entries {
        let Some(ident) = entry.path.get_ident() else {
            return Err(syn::Error::new_spanned(
                entry.path,
                "theme value key must be `default` or a declared variant",
            ));
        };
        let name = ident.to_string();
        if name == "default" {
            if result.default.replace(entry.value).is_some() {
                return Err(syn::Error::new(
                    ident.span(),
                    "duplicate `default` theme value",
                ));
            }
        } else {
            if !variants.contains(&name) {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unknown theme variant `{name}`"),
                ));
            }
            if result.variants.insert(name.clone(), entry.value).is_some() {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("duplicate `{name}` theme value"),
                ));
            }
        }
    }
    Ok(result)
}

fn is_platform_default(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Path(path)
            if path.path.segments.len() == 1
                && path.path.segments[0].ident == "platform_default"
    )
}

fn impact_for(name: &str) -> proc_macro2::Ident {
    if name.contains("font")
        || name.contains("spacing")
        || name.contains("padding")
        || name.contains("size")
        || name.contains("character_spacing")
    {
        format_ident!("Measure")
    } else if name.starts_with("button_")
        || name.starts_with("text_box_")
        || name.starts_with("password_box_")
        || name.starts_with("text_area_")
        || name.starts_with("native_control_")
        || name.starts_with("menu")
        || name.starts_with("tab_view")
        || name.starts_with("scroll_view")
    {
        format_ident!("NativeStyle")
    } else {
        format_ident!("Paint")
    }
}

/// Kept in the Rust frontend rather than the DSL text parser. Names are intentionally based on
/// ElwindUI public/abstract types, never toolkit vocabulary such as Panel or Surface.
pub const STANDARD_TOKEN_NAMES: &[&str] = &[
    "window_background",
    "layout_background",
    "layout_spacing",
    "control_background",
    "control_foreground",
    "control_border",
    "control_padding",
    "control_corner_radius",
    "control_font_family",
    "control_font_size",
    "control_font_weight",
    "control_font_style",
    "control_font_stretch",
    "control_character_spacing",
    "native_control_background",
    "native_control_foreground",
    "native_control_border",
    "native_control_focus_width",
    "native_control_font_family",
    "native_control_font_size",
    "native_control_font_weight",
    "native_control_font_style",
    "native_control_font_stretch",
    "native_control_character_spacing",
    "text_block_foreground",
    "text_block_font_family",
    "text_block_font_size",
    "text_block_font_weight",
    "text_block_font_style",
    "text_block_font_stretch",
    "text_block_character_spacing",
    "shape_fill",
    "shape_stroke",
    "shape_stroke_width",
    "rectangle_corner_radius",
    "button_background",
    "button_foreground",
    "button_border",
    "button_hover_background",
    "button_hover_foreground",
    "button_pressed_background",
    "button_pressed_foreground",
    "button_disabled_background",
    "button_disabled_foreground",
    "text_box_background",
    "text_box_foreground",
    "text_box_border",
    "text_box_placeholder_foreground",
    "text_box_selection_background",
    "text_box_caret",
    "text_box_focus_border",
    "password_box_background",
    "password_box_foreground",
    "password_box_border",
    "password_box_placeholder_foreground",
    "password_box_selection_background",
    "password_box_caret",
    "password_box_focus_border",
    "text_area_background",
    "text_area_foreground",
    "text_area_border",
    "text_area_placeholder_foreground",
    "text_area_selection_background",
    "text_area_caret",
    "text_area_focus_border",
    "scroll_view_background",
    "scroll_view_scrollbar_background",
    "scroll_view_scrollbar_thumb",
    "scroll_view_scrollbar_hover_thumb",
    "menu_bar_background",
    "menu_bar_foreground",
    "menu_background",
    "menu_foreground",
    "menu_item_background",
    "menu_item_foreground",
    "menu_item_selected_background",
    "menu_item_selected_foreground",
    "menu_item_disabled_foreground",
    "tab_view_background",
    "tab_view_foreground",
    "tab_view_item_background",
    "tab_view_item_foreground",
    "tab_view_item_selected_background",
    "tab_view_item_selected_foreground",
    "tab_view_item_hover_background",
    "tab_view_item_disabled_foreground",
    "tab_view_item_close_button_background",
    "tab_view_item_close_button_foreground",
];

fn value_tokens(value: Option<&Expr>, ty: &Type) -> TokenStream {
    match value {
        Some(value) if !is_platform_default(value) => quote! {
            elwindui::core::theme::erase_theme_value::<#ty>(
                elwindui::core::theme::ThemeValue::Value({
                    let __value: #ty = #value;
                    __value
                })
            )
        },
        _ => quote! { elwindui::core::theme::ErasedThemeValue::PlatformDefault },
    }
}

pub fn generate_theme_from_item_struct(
    args: TokenStream,
    item: &ItemStruct,
) -> Result<TokenStream, String> {
    let args = syn::parse2::<ThemeArgs>(args).map_err(|error| error.to_string())?;
    if !item.generics.params.is_empty() {
        return Err("theme structs cannot be generic".into());
    }
    let extends_system_theme = args
        .extends
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "SystemTheme");
    if !extends_system_theme {
        return Err("v1 themes must use `extends = SystemTheme`".into());
    }
    if args.variants.is_empty() {
        return Err("a theme must declare at least one variant".into());
    }

    let mut variant_names = HashSet::new();
    for variant in &args.variants {
        if !variant_names.insert(variant.to_string()) {
            return Err(format!("duplicate theme variant `{variant}`"));
        }
    }

    let fields = match &item.fields {
        syn::Fields::Named(fields) => &fields.named,
        _ => return Err("a theme must be a struct with named fields".into()),
    };
    let mut field_names = HashSet::new();
    let mut parsed_fields = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| "theme field must be named".to_string())?;
        if !field_names.insert(ident.to_string()) {
            return Err(format!("duplicate theme token `{ident}`"));
        }
        let values =
            parse_field_values(field, &variant_names).map_err(|error| error.to_string())?;
        let standard = STANDARD_TOKEN_NAMES.contains(&ident.to_string().as_str());
        if values.default.is_none() && !standard {
            return Err(format!(
                "application token `{ident}` requires a `default` value"
            ));
        }
        parsed_fields.push((ident, field.ty.clone(), values, standard));
    }

    let visibility = &item.vis;
    let name = &item.ident;
    let variant_name = format_ident!("{name}Variant");
    let definition_name = format_ident!("__{name}Definition");
    let variants = &args.variants;
    let declared_impacts: Vec<_> = parsed_fields
        .iter()
        .map(|(field, _, _, _)| impact_for(&field.to_string()).to_string())
        .collect();
    let change_impact = if declared_impacts
        .iter()
        .any(|impact| impact == "NativeStyle")
    {
        format_ident!("NativeStyle")
    } else if declared_impacts.iter().any(|impact| impact == "Measure") {
        format_ident!("Measure")
    } else {
        format_ident!("Paint")
    };

    let token_methods = parsed_fields.iter().map(|(field, ty, _, standard)| {
        let impact = impact_for(&field.to_string());
        let field_doc = format!("The typed `{field}` theme token.");
        quote! {
            #[doc = #field_doc]
            #[allow(non_upper_case_globals)]
            pub const #field: elwindui::core::theme::ThemeToken<#ty> =
                elwindui::core::theme::ThemeToken::new(
                    stringify!(#field),
                    elwindui::core::theme::ThemeChangeImpact::#impact,
                    #standard,
                );
        }
    });

    let resolve_arms = parsed_fields.iter().map(|(field, ty, values, _)| {
        let field_string = field.to_string();
        let variant_arms = variants.iter().map(|variant| {
            let specific = values.variants.get(&variant.to_string());
            let selected = specific.or(values.default.as_ref());
            let value = value_tokens(selected, ty);
            quote! { #variant_name::#variant => #value }
        });
        quote! {
            #field_string => Some(match self.variant {
                #(#variant_arms,)*
            })
        }
    });

    let variant_labels = variants.iter().map(|variant| {
        let label = variant.to_string();
        quote! { #variant_name::#variant => #label }
    });
    let variant_declarations = variants.iter().map(|variant| {
        let doc = format!("Selects the `{variant}` theme variant.");
        quote! {
            #[doc = #doc]
            #variant,
        }
    });
    let theme_doc = format!(
        "A typed theme generated from the `{name}` declaration.\n\n\
         Create a live controller with [`{name}::controller`]."
    );
    let variant_doc = format!("Application variants declared by [`{name}`].");

    Ok(quote! {
        #[doc = #theme_doc]
        #visibility struct #name;

        #[doc = #variant_doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #visibility enum #variant_name {
            #(#variant_declarations)*
        }

        struct #definition_name {
            variant: #variant_name,
        }

        impl elwindui::core::theme::ThemeDefinition for #definition_name {
            fn resolve_erased(
                &self,
                token: &str,
            ) -> Option<elwindui::core::theme::ErasedThemeValue> {
                match token {
                    #(#resolve_arms,)*
                    _ => None,
                }
            }

            fn variant_name(&self) -> &'static str {
                match self.variant {
                    #(#variant_labels,)*
                }
            }
        }

        impl elwindui::core::theme::ThemeFactory for #name {
            type Variant = #variant_name;

            fn create_definition(
                variant: &Self::Variant,
            ) -> std::rc::Rc<dyn elwindui::core::theme::ThemeDefinition> {
                std::rc::Rc::new(#definition_name { variant: *variant })
            }

            fn change_impact(
                _previous: &Self::Variant,
                _next: &Self::Variant,
            ) -> elwindui::core::theme::ThemeChangeImpact {
                elwindui::core::theme::ThemeChangeImpact::#change_impact
            }
        }

        impl #name {
            /// Creates a live controller using `initial_variant`.
            pub fn controller(
                initial_variant: #variant_name,
            ) -> elwindui::core::theme::ThemeController<Self> {
                elwindui::core::theme::ThemeController::new(initial_variant)
            }

            #(#token_methods)*
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn generates_variant_tokens_and_platform_default() {
        let item: ItemStruct = syn::parse_quote! {
            pub struct AppTheme {
                #[theme(default = platform_default, Ocean = Brush::Solid(Color::rgb(1, 2, 3)))]
                layout_background: Brush,
                #[theme(default = 6.0)]
                layout_spacing: f32,
                #[theme(default = Brush::Solid(Color::rgb(4, 5, 6)))]
                brand: Brush,
            }
        };
        let output = generate_theme_from_item_struct(
            quote!(extends = SystemTheme, variants(Default, Ocean)),
            &item,
        )
        .unwrap()
        .to_string();
        assert!(output.contains("enum AppThemeVariant"));
        assert!(output.contains("ErasedThemeValue :: PlatformDefault"));
        assert!(output.contains("pub const brand"));
        assert!(output.contains("ThemeController"));
        assert!(output.contains("ThemeChangeImpact :: Measure"));
    }

    #[test]
    fn rejects_unknown_and_duplicate_variants() {
        let item: ItemStruct = syn::parse_quote! {
            struct AppTheme {
                #[theme(default = 1.0, Missing = 2.0)]
                brand: f32,
            }
        };
        assert!(
            generate_theme_from_item_struct(
                quote!(extends = SystemTheme, variants(Default, Ocean)),
                &item,
            )
            .unwrap_err()
            .contains("unknown theme variant")
        );
        assert!(
            generate_theme_from_item_struct(
                quote!(extends = SystemTheme, variants(Default, Default)),
                &item,
            )
            .unwrap_err()
            .contains("duplicate theme variant")
        );
    }

    #[test]
    fn custom_token_requires_a_default() {
        let item: ItemStruct = syn::parse_quote! {
            struct AppTheme {
                #[theme(Ocean = 1.0)]
                brand: f32,
            }
        };
        assert!(
            generate_theme_from_item_struct(
                quote!(extends = SystemTheme, variants(Default, Ocean)),
                &item,
            )
            .unwrap_err()
            .contains("requires a `default`")
        );
    }

    #[test]
    fn standard_manifest_does_not_use_backend_taxonomy() {
        assert!(STANDARD_TOKEN_NAMES.contains(&"layout_background"));
        assert!(!STANDARD_TOKEN_NAMES.contains(&"panel_background"));
        assert!(!STANDARD_TOKEN_NAMES.contains(&"surface_background"));
        assert!(!STANDARD_TOKEN_NAMES.contains(&"input_background"));
    }

    #[test]
    fn standard_manifest_matches_core_system_theme() {
        let source = include_str!("../../elwindui-core/src/theme.rs");
        let manifest = source
            .split("pub const STANDARD_THEME_TOKEN_NAMES")
            .nth(1)
            .expect("core standard-token manifest")
            .split("];")
            .next()
            .expect("core standard-token manifest terminator");
        let core_names: HashSet<&str> = manifest
            .lines()
            .filter_map(|line| line.trim().strip_prefix('"'))
            .filter_map(|line| line.strip_suffix("\","))
            .collect();
        let frontend_names: HashSet<&str> = STANDARD_TOKEN_NAMES.iter().copied().collect();
        assert_eq!(frontend_names, core_names);
    }
}
