//! Alternative frontend, sibling to `attr_frontend.rs`'s viewmodel path: builds the same
//! `ComponentDef`/`ViewDef` AST (`ast.rs`, unchanged) that `parser.rs`'s hand-written
//! recursive-descent parser produces from `.elwind` DSL text — but from a real Rust `struct`
//! instead, annotated `#[elwindui::component(inherits Base)]`. Ordinary fields become the
//! component's `#[param]`/`#[prop]`/etc. fields (via `attr_frontend::fields_from_item_struct`,
//! shared with the viewmodel frontend); exactly one field, typed as a `view!` macro invocation
//! (`field: view! { .. }`, parsed by `syn` as `syn::Type::Macro` — legal Rust in type position),
//! supplies the view tree.
//!
//! `view!` itself is never a real macro and never gets expanded: `#[elwindui::component]` (a
//! `proc_macro_attribute`) replaces the entire annotated struct with different code, and Rust only
//! expands an attribute macro's *own* inner item macros if they survive into that replacement —
//! they don't here, so `view` doesn't need to be defined anywhere. Its tokens are recovered here as
//! plain text (`syn::Macro::tokens.to_string()`) and re-parsed via `parser::parse_view_body`, the
//! same "grab the raw tokens as DSL text" trick `elwindui-macros` used for the (now removed)
//! `component!` bang macro, just relocated to one struct field's type position.
//!
//! Because `generate_module` (codegen.rs) only ever consumes the `ComponentDef`/`ViewDef` AST —
//! never the original source — nothing in codegen.rs needs to change for this frontend to work.

use crate::ast::{ComponentDef, FieldKind, ViewDef};
use crate::{attr_frontend, parser};

/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` (already
/// parsed as a `syn::ItemStruct` by the `elwindui-macros` proc-macro, `base` from the attribute's
/// own `inherits Base` argument) — builds the matching `ComponentDef`/`ViewDef` pair.
pub fn component_and_view_from_item_struct(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<(ComponentDef, ViewDef), String> {
    let name = item_struct.ident.to_string();

    let syn::Fields::Named(named) = &item_struct.fields else {
        return Err(format!("`{name}` must have named fields"));
    };

    let view_fields: Vec<&syn::Field> = named
        .named
        .iter()
        .filter(|f| is_view_macro_field(f))
        .collect();
    let view_field = match view_fields.as_slice() {
        [only] => *only,
        [] => {
            return Err(format!(
                "`{name}`: expected exactly one field typed `view! {{ .. }}` to supply the view body, found none"
            ));
        }
        _ => {
            return Err(format!(
                "`{name}`: expected exactly one field typed `view! {{ .. }}`, found {}",
                view_fields.len()
            ));
        }
    };

    let syn::Type::Macro(view_macro) = &view_field.ty else {
        unreachable!("is_view_macro_field only returns fields whose type is a macro invocation");
    };
    let view_src = view_macro.mac.tokens.to_string();
    let (on_mount, on_unmount, lets, root) = parser::parse_view_body(&view_src)
        .map_err(|e| format!("`{name}`: invalid `view! {{ .. }}` body: {e}"))?;

    let view_def = ViewDef {
        target: name.clone(),
        on_mount,
        on_unmount,
        lets,
        root,
    };

    let mut non_view_struct = item_struct.clone();
    if let syn::Fields::Named(named) = &mut non_view_struct.fields {
        named.named = named
            .named
            .iter()
            .filter(|f| !is_view_macro_field(f))
            .cloned()
            .collect();
    }
    let fields = attr_frontend::fields_from_item_struct(&non_view_struct, FieldKind::Prop)?;

    let component_def = ComponentDef {
        name,
        base,
        fields,
        methods: Vec::new(),
        embedded: false,
        sealed: false,
        native: false,
        is_abstract: false,
        content_field: None,
    };

    Ok((component_def, view_def))
}

fn is_view_macro_field(field: &syn::Field) -> bool {
    matches!(&field.ty, syn::Type::Macro(tm) if tm.mac.path.is_ident("view"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{build_symbol_table, generate_module};

    fn generate(base: Option<&str>, src: &str) -> proc_macro2::TokenStream {
        let item_struct: syn::ItemStruct =
            syn::parse_str(src).expect("struct should parse as valid Rust");
        let (component_def, view_def) =
            component_and_view_from_item_struct(base.map(str::to_string), &item_struct)
                .expect("should build a ComponentDef/ViewDef");
        let module = crate::ast::Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: vec![
                crate::ast::Item::Component(component_def),
                crate::ast::Item::View(view_def),
            ],
            ..Default::default()
        };
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("should validate");
        let table = build_symbol_table(&all_modules);
        generate_module(&module, &table)
    }

    #[test]
    fn generates_valid_rust_and_matches_expected_shape() {
        let src = r#"
            struct Counter {
                #[param]
                #[inject]
                start: i32,

                body: view! {
                    title: "counter"
                    content: VerticalLayout {
                        TextBlock { text: "hi" }
                    }
                }
            }
        "#;
        let generated = generate(Some("Window"), src);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let s = generated.to_string();
        assert!(s.contains("struct Counter"));
        assert!(s.contains("impl"));
    }

    #[test]
    fn missing_view_field_is_an_error() {
        let src = r#"
            struct Counter {
                #[param]
                start: i32,
            }
        "#;
        let item_struct: syn::ItemStruct = syn::parse_str(src).unwrap();
        let err =
            component_and_view_from_item_struct(Some("Window".to_string()), &item_struct)
                .unwrap_err();
        assert!(err.contains("view!"), "error should mention view!: {err}");
    }

    /// The attribute-macro frontend must produce *the same* generated code as the equivalent
    /// `.elwind` DSL text through the existing `parser.rs` — proving `codegen.rs` really is
    /// unchanged/shared, not just superficially similar.
    #[test]
    fn matches_dsl_frontend_output_for_an_equivalent_component() {
        let attr_src = r#"
            struct Counter {
                #[param]
                #[inject]
                start: i32,

                body: view! {
                    title: "counter"
                    content: VerticalLayout {
                        TextBlock { text: "hi" }
                    }
                }
            }
        "#;
        let attr_generated = generate(Some("Window"), attr_src).to_string();

        let dsl_src = r#"
component Counter inherits Window {
    #[param]
    #[inject]
    start: i32,
}

view Counter {
    title: "counter"
    content: VerticalLayout {
        TextBlock { text: "hi" }
    }
}
"#;
        let module = crate::parser::parse_module(dsl_src).expect("dsl should parse");
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("dsl should validate");
        let table = build_symbol_table(&all_modules);
        let dsl_generated = generate_module(&module, &table).to_string();

        assert_eq!(attr_generated, dsl_generated);
    }
}
