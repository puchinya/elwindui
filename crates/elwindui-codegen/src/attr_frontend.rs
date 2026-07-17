//! Alternative frontend: builds the same `ViewModelDef`/`FieldDef` AST (`ast.rs`, unchanged) that
//! `parser.rs`'s hand-written recursive-descent parser produces from `.elwind` DSL text — but from
//! real Rust syntax instead (a `syn::ItemMod` containing a `struct` + an `impl` block). This is
//! what lets `viewmodel`s be written as ordinary Rust (matching how WPF-style MVVM keeps the
//! ViewModel in the host language, not markup — see docs/elwindui_spec.md 付録O.2) while
//! `view { ... }` trees still need `.elwind`/`parser.rs` (bare nested child elements aren't valid
//! Rust expression syntax, so that half can't move here).
//!
//! Because `generate_viewmodel` (codegen.rs) only ever consumes the `ViewModelDef`/`FieldDef` AST
//! — never the original DSL text — nothing in codegen.rs needs to change for this frontend to
//! work: it just has to produce the same shape of AST parser.rs already produces.

use crate::ast::{Attr, FieldDef, FieldKind, Initializer, ViewModelDef};
use crate::parser;
use std::path::Path;

/// Finds every `#[elwindui::viewmodel] mod foo { ... }` at the top level of a `.rs` file and builds
/// a `ViewModelDef` for each, paired with the enclosing `mod`'s own name (`"foo"`) — **without**
/// actually expanding the attribute macro. This is `syn` parsing the file's *source text* as data,
/// the same way `viewmodel_def_from_item_mod` reads a macro's input; it never runs
/// `elwindui-macros`, so there's no dependency on Rust's proc-macro expansion order (which matters
/// here: `build.rs` — the caller of `compile_dir_with_extra_viewmodels`, `lib.rs` — always runs
/// *before* the crate's own source, including this file, gets compiled and macro-expanded, so
/// waiting for the real macro to run is not an option).
///
/// The mod name is what lets a caller build this viewmodel's real, crate-relative path (`Module::path`,
/// e.g. `["notepad_view_model"]` for `main.rs`'s `mod notepad_view_model { .. }`), so a `.elwind`
/// file's `use crate::notepad_view_model::NotepadViewModel;` can be resolved against it exactly like
/// Rust's own name resolution (§12, 付録B.1) — the struct name alone isn't enough to know where it
/// actually lives.
pub fn viewmodel_defs_from_rs_file(
    path: impl AsRef<Path>,
) -> Result<Vec<(String, ViewModelDef)>, String> {
    let path = path.as_ref();
    let src =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let file: syn::File =
        syn::parse_file(&src).map_err(|e| format!("parsing {} as Rust: {e}", path.display()))?;

    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(m) if has_viewmodel_attr(m) => Some(m),
            _ => None,
        })
        .map(|m| {
            viewmodel_def_from_item_mod(m)
                .map(|def| (m.ident.to_string(), def))
                .map_err(|e| format!("{} (in {})", e, path.display()))
        })
        .collect()
}

fn has_viewmodel_attr(item_mod: &syn::ItemMod) -> bool {
    item_mod.attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "viewmodel")
    })
}

/// `#[elwindui::viewmodel] mod foo { struct Foo { ... } impl Foo { ... } }` — the `struct` supplies
/// field declarations (`#[observable(default = expr)]` etc.), the `impl`'s `fn`s supply each
/// `#[command]` field's body, matched to its field by name (a field `new_tab: Command` pairs with
/// `fn new_tab(&self) { ... }` in the same mod). A single macro invocation must see both together
/// — Rust attribute macros only ever see one annotated item, so there's no way to correlate a
/// separately-expanded `struct`-only macro with an `impl`-only macro afterwards.
pub fn viewmodel_def_from_item_mod(item_mod: &syn::ItemMod) -> Result<ViewModelDef, String> {
    let (_, items) = item_mod.content.as_ref().ok_or_else(|| {
        "#[elwindui::viewmodel] mod must have a body (`mod foo { ... }`, not `mod foo;`)"
            .to_string()
    })?;

    let item_struct = items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(s) => Some(s),
            _ => None,
        })
        .ok_or_else(|| {
            "expected exactly one `struct` inside the `#[elwindui::viewmodel]` mod".to_string()
        })?;

    let item_impl = items.iter().find_map(|item| match item {
        syn::Item::Impl(i) => Some(i),
        _ => None,
    });

    let name = item_struct.ident.to_string();
    let mut fields = fields_from_item_struct(item_struct, FieldKind::Observable)?;

    if let Some(item_impl) = item_impl {
        attach_command_bodies(&mut fields, item_impl)?;
    }

    if let Some(missing) = fields
        .iter()
        .find(|f| f.kind == FieldKind::Command && f.initializer.is_none())
    {
        return Err(format!(
            "command field `{}` has no matching `fn {}` in the mod's `impl` block",
            missing.name, missing.name
        ));
    }

    Ok(ViewModelDef { name, fields })
}

/// Builds `FieldDef`s from a `syn::ItemStruct`'s named fields, recognizing the same attribute
/// vocabulary `parser.rs`'s DSL field parser (`parse_field_def`) does — `param`/`prop`/
/// `observable`/`computed`/`attached`/`inject`/`two_way`/`routed`/`override`/`onetime`/`command`/
/// `length` — uniformly whether the caller is a `viewmodel` (`default_kind: FieldKind::Observable`,
/// via `viewmodel_def_from_item_mod`) or a `component` (`default_kind: FieldKind::Prop`, via
/// `component_frontend.rs`), exactly mirroring `parse_module`'s two `parse_fields_block` call
/// sites. Whether a particular kind/attribute combination is actually *sensible* (e.g.
/// `#[observable]` on a component field) is left to `validate::validate`, same as hand-written DSL
/// text — no duplicate validation here.
///
/// `#[observable(default = expr)]`/`#[computed(expr = expr)]` parse their value as a plain
/// `syn::Expr` (`parse_name_value_expr`) — fine since neither ever needs `bind!`/`command!` sugar.
/// `#[prop(default = ...)]`/`#[attached(default = ...)]` instead route their raw token text
/// through `parser::parse_initializer` (`parse_name_value_tokens`), so `bind!(vm.content,
/// TwoWay)`/`command!(...)` written there get the same recognition hand-written `.elwind` text
/// gets — unlike a `view!`-typed field's tokens (discarded whole), a field default is emitted
/// verbatim into code that really gets compiled, so it can't be left as an inert
/// `syn::Expr::Macro`.
pub(crate) fn fields_from_item_struct(
    item_struct: &syn::ItemStruct,
    default_kind: FieldKind,
) -> Result<Vec<FieldDef>, String> {
    let syn::Fields::Named(named) = &item_struct.fields else {
        return Err(format!("`{}` must have named fields", item_struct.ident));
    };

    let mut out = Vec::new();
    for field in &named.named {
        let name = field
            .ident
            .as_ref()
            .expect("syn::Fields::Named always has idents")
            .to_string();
        let ty = type_to_compact_string(&field.ty);

        let mut kind = default_kind;
        let mut attrs = Vec::new();
        let mut initializer = None;

        for attr in &field.attrs {
            let Some(attr_name) = attr.path().get_ident().map(|i| i.to_string()) else {
                return Err(format!("field `{name}`: expected a simple attribute name"));
            };
            match attr_name.as_str() {
                "param" => kind = FieldKind::Param,
                "prop" => {
                    kind = FieldKind::Prop;
                    if let Some(tokens) = parse_name_value_tokens(attr, "default")? {
                        initializer = Some(
                            parser::parse_initializer(&tokens.to_string()).map_err(|e| {
                                format!("field `{name}`: invalid #[prop(default = ...)]: {e}")
                            })?,
                        );
                    }
                }
                "observable" => {
                    kind = FieldKind::Observable;
                    let default = parse_name_value_expr(attr, "default")?.ok_or_else(|| {
                        format!("field `{name}`: #[observable(...)] needs `default = expr`")
                    })?;
                    initializer = Some(Initializer::Expr(default));
                }
                "computed" => {
                    kind = FieldKind::Computed;
                    let expr = parse_name_value_expr(attr, "expr")?.ok_or_else(|| {
                        format!("field `{name}`: #[computed(...)] needs `expr = expr`")
                    })?;
                    initializer = Some(Initializer::Expr(expr));
                }
                "attached" => {
                    kind = FieldKind::Attached;
                    if let Some(tokens) = parse_name_value_tokens(attr, "default")? {
                        initializer = Some(
                            parser::parse_initializer(&tokens.to_string()).map_err(|e| {
                                format!("field `{name}`: invalid #[attached(default = ...)]: {e}")
                            })?,
                        );
                    }
                }
                "command" => {
                    kind = FieldKind::Command;
                    let can_execute = parse_name_value_expr(attr, "can_execute")?;
                    // `is_async` is filled in later, once we've seen the matching `fn`'s
                    // signature (`attach_command_bodies`) — a plain field declaration has no
                    // way to say "async" itself.
                    attrs.push(Attr::CommandMeta {
                        is_async: false,
                        can_execute,
                    });
                }
                "inject" => attrs.push(Attr::Inject),
                "bindable" => {
                    kind = FieldKind::Param;
                    attrs.push(Attr::Inject);
                    attrs.push(Attr::Bindable);
                }
                "two_way" => attrs.push(Attr::TwoWay),
                "routed" => attrs.push(Attr::Routed),
                "override" => attrs.push(Attr::Override),
                "onetime" => attrs.push(Attr::Onetime),
                "length" => {
                    let (start, end, inclusive) = parse_length_range(attr)?;
                    attrs.push(Attr::Length {
                        start,
                        end,
                        inclusive,
                    });
                }
                other => return Err(format!("field `{name}`: unknown attribute #[{other}]")),
            }
        }

        // Unlike hand-written `.elwind` text, a plain Rust struct field has no `= expr` syntax of
        // its own — `#[observable(default = ...)]`/`#[computed(expr = ...)]` are the only place
        // either kind's value can be written, so (whether `kind` came from an explicit attribute
        // or fell back to `default_kind`) both must end up with an initializer.
        if matches!(kind, FieldKind::Observable | FieldKind::Computed) && initializer.is_none() {
            return Err(format!(
                "field `{name}`: an Observable/Computed field needs #[observable(default = ...)] \
                 or #[computed(expr = ...)] (plain Rust struct fields have no other way to supply one)"
            ));
        }

        out.push(FieldDef {
            name,
            ty,
            kind,
            attrs,
            initializer,
        });
    }
    Ok(out)
}

/// Matches each `#[command]` field to the `fn` of the same name in the mod's `impl` block, filling
/// in the field's `Initializer::Command` (params + raw body — `rewrite_command_body`, called
/// unconditionally by `generate_viewmodel`, does the sibling-field-reference rewriting, exactly as
/// it already does for DSL-sourced command bodies) and the `CommandMeta::is_async` flag from the
/// `fn`'s own `async` keyword.
fn attach_command_bodies(fields: &mut [FieldDef], item_impl: &syn::ItemImpl) -> Result<(), String> {
    for item in &item_impl.items {
        let syn::ImplItem::Fn(item_fn) = item else {
            continue;
        };
        let fn_name = item_fn.sig.ident.to_string();
        let Some(field) = fields
            .iter_mut()
            .find(|f| f.kind == FieldKind::Command && f.name == fn_name)
        else {
            return Err(format!(
                "fn `{fn_name}` in the impl block doesn't match any #[command] field of the same name"
            ));
        };

        let params = item_fn
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                syn::FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
                    syn::Pat::Ident(pat_ident) => {
                        Some((pat_ident.ident.to_string(), (*pat_type.ty).clone()))
                    }
                    _ => None,
                },
                syn::FnArg::Receiver(_) => None,
            })
            .collect();

        field.initializer = Some(Initializer::Command {
            params,
            body: item_fn.block.clone(),
        });

        let is_async = item_fn.sig.asyncness.is_some();
        for attr in &mut field.attrs {
            if let Attr::CommandMeta { is_async: flag, .. } = attr {
                *flag = is_async;
            }
        }
    }
    Ok(())
}

/// `syn::Type` -> the tight, no-whitespace string form the rest of `codegen.rs` expects (it round-
/// trips field types through plain string matching — `is_copy_type`, `nested_vec_item_type` — since
/// that's the form `parser.rs` produces by slicing raw source text). `quote!`'s `Display` inserts a
/// space around every token (`Vec < Document >`), so it has to be stripped back out here rather
/// than touching codegen.rs's matching logic.
fn type_to_compact_string(ty: &syn::Type) -> String {
    quote::quote! { #ty }.to_string().replace(' ', "")
}

/// Parses `#[attr_name(name = expr)]`'s inner `name = expr` and returns `expr` if present —
/// `Ok(None)` for a bare `#[attr_name]` with no parenthesized arguments at all (e.g. plain
/// `#[command]`).
fn parse_name_value_expr(attr: &syn::Attribute, name: &str) -> Result<Option<syn::Expr>, String> {
    if matches!(attr.meta, syn::Meta::Path(_)) {
        return Ok(None);
    }
    let (ident, expr) = attr
        .parse_args_with(|input: syn::parse::ParseStream| {
            let ident: syn::Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let expr: syn::Expr = input.parse()?;
            Ok((ident, expr))
        })
        .map_err(|e| {
            let attr_name = attr
                .path()
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            format!("invalid #[{attr_name}(...)] arguments: {e}")
        })?;
    if ident == name {
        Ok(Some(expr))
    } else {
        Err(format!("expected `{name} = ...`, found `{ident} = ...`"))
    }
}

/// Like `parse_name_value_expr`, but returns `name = <tokens>`'s raw, unparsed token text instead
/// of eagerly parsing it as a `syn::Expr` — used for `#[prop(default = ...)]`/`#[attached(default
/// = ...)]`, which (unlike `observable`/`computed`) need `bind!`/`command!` sugar recognized via
/// `parser::parse_initializer` rather than left as an inert `syn::Expr::Macro` (see
/// `fields_from_item_struct`'s doc comment).
fn parse_name_value_tokens(
    attr: &syn::Attribute,
    name: &str,
) -> Result<Option<proc_macro2::TokenStream>, String> {
    if matches!(attr.meta, syn::Meta::Path(_)) {
        return Ok(None);
    }
    let (ident, tokens) = attr
        .parse_args_with(|input: syn::parse::ParseStream| {
            let ident: syn::Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let tokens: proc_macro2::TokenStream = input.parse()?;
            Ok((ident, tokens))
        })
        .map_err(|e| {
            let attr_name = attr
                .path()
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            format!("invalid #[{attr_name}(...)] arguments: {e}")
        })?;
    if ident == name {
        Ok(Some(tokens))
    } else {
        Err(format!("expected `{name} = ...`, found `{ident} = ...`"))
    }
}

fn parse_length_range(attr: &syn::Attribute) -> Result<(i64, i64, bool), String> {
    let range: syn::ExprRange = attr
        .parse_args()
        .map_err(|e| format!("invalid #[length(...)] argument: {e}"))?;
    let start = range
        .start
        .as_ref()
        .ok_or_else(|| "#[length(...)] needs a start bound".to_string())?;
    let end = range
        .end
        .as_ref()
        .ok_or_else(|| "#[length(...)] needs an end bound".to_string())?;
    let start = expr_to_i64(start)?;
    let end = expr_to_i64(end)?;
    let inclusive = matches!(range.limits, syn::RangeLimits::Closed(_));
    Ok((start, end, inclusive))
}

fn expr_to_i64(expr: &syn::Expr) -> Result<i64, String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(lit_int),
            ..
        }) => lit_int.base10_parse().map_err(|e| e.to_string()),
        _ => Err("expected an integer literal".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{build_symbol_table, generate_viewmodel};

    fn generate(src: &str) -> proc_macro2::TokenStream {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse as valid Rust");
        let def = viewmodel_def_from_item_mod(&item_mod).expect("should build a ViewModelDef");
        let module = crate::ast::Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: vec![crate::ast::Item::ViewModel(def)],
            ..Default::default()
        };
        let table = build_symbol_table(std::slice::from_ref(&module));
        let crate::ast::Item::ViewModel(def) = &module.items[0] else {
            unreachable!()
        };
        generate_viewmodel(def, &module, &table)
    }

    #[test]
    fn generates_valid_rust_and_matches_expected_shape() {
        let src = r#"
            mod document {
                struct Document {
                    #[observable(default = String::new())]
                    #[length(0..=100000)]
                    content: String,

                    #[observable(default = "untitled.txt")]
                    file_name: String,

                    #[computed(expr = content.chars().count() as i32)]
                    char_count: i32,
                }

                impl Document {}
            }
        "#;
        let generated = generate(src);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let s = generated.to_string();
        assert!(s.contains("struct Document"));
        assert!(s.contains("pub fn content"));
        assert!(s.contains("pub fn set_content"));
        assert!(s.contains("pub fn char_count"));
        assert!(s.contains("fn recompute_char_count"));
    }

    #[test]
    fn command_field_pairs_with_impl_fn_of_the_same_name() {
        let src = r#"
            mod vm {
                struct Counter {
                    #[observable(default = 0i32)]
                    count: i32,

                    #[command(can_execute = count < 10)]
                    increment: Command,
                }

                impl Counter {
                    fn increment(&self) {
                        count = count + 1;
                    }
                }
            }
        "#;
        let generated = generate(src);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let s = generated.to_string();
        assert!(s.contains("fn increment_execute"));
        assert!(s.contains("fn increment_can_execute"));
        // The body's bare `count` reference must have been rewritten to `self.count()`/
        // `self.set_count(...)` by the same `rewrite_command_body` the DSL path uses.
        assert!(s.contains("self . set_count"));
    }

    #[test]
    fn missing_impl_fn_for_command_field_is_an_error() {
        let src = r#"
            mod vm {
                struct Counter {
                    #[command]
                    increment: Command,
                }
                impl Counter {}
            }
        "#;
        let item_mod: syn::ItemMod = syn::parse_str(src).unwrap();
        let err = viewmodel_def_from_item_mod(&item_mod).unwrap_err();
        assert!(
            err.contains("increment"),
            "error should mention the field: {err}"
        );
    }

    /// The attribute-macro frontend must produce *the same* generated code as the equivalent
    /// `.elwind` DSL text through the existing `parser.rs` — proving `generate_viewmodel`
    /// (codegen.rs) really is unchanged/shared, not just superficially similar.
    #[test]
    fn matches_dsl_frontend_output_for_an_equivalent_viewmodel() {
        let attr_src = r#"
            mod vm {
                struct Counter {
                    #[observable(default = 0i32)]
                    count: i32,

                    #[computed(expr = count * 2)]
                    doubled: i32,

                    #[command(can_execute = count < 10)]
                    increment: Command,
                }

                impl Counter {
                    fn increment(&self) {
                        count = count + 1;
                    }
                }
            }
        "#;
        let attr_generated = generate(attr_src).to_string();

        let dsl_src = r#"
viewmodel Counter {
    #[observable]
    count: i32 = 0i32,

    #[computed]
    doubled: i32 = count * 2,

    #[command(can_execute: count < 10)]
    increment: Command = command!(|| {
        count = count + 1;
    }),
}
"#;
        let module = crate::parser::parse_module(dsl_src).expect("dsl should parse");
        let table = build_symbol_table(std::slice::from_ref(&module));
        let crate::ast::Item::ViewModel(def) = &module.items[0] else {
            panic!("expected viewmodel")
        };
        let dsl_generated = generate_viewmodel(def, &module, &table).to_string();

        assert_eq!(attr_generated, dsl_generated);
    }

    #[test]
    fn viewmodel_defs_from_rs_file_finds_top_level_viewmodel_mods() {
        let src = r#"
            use elwindui::platform;

            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            enum Status { Idle, Busy }

            #[elwindui::viewmodel]
            mod counter_vm {
                struct Counter {
                    #[observable(default = 0i32)]
                    count: i32,

                    #[command]
                    increment: Command,
                }

                impl Counter {
                    fn increment(&self) {
                        count = count + 1;
                    }
                }
            }

            fn main() {}
        "#;
        let path = std::env::temp_dir().join(format!(
            "elwindui_attr_frontend_test_{}.rs",
            std::process::id()
        ));
        std::fs::write(&path, src).expect("write temp file");
        let defs = viewmodel_defs_from_rs_file(&path).expect("should find the viewmodel mod");
        std::fs::remove_file(&path).ok();

        assert_eq!(defs.len(), 1);
        let (mod_name, def) = &defs[0];
        assert_eq!(mod_name, "counter_vm");
        assert_eq!(def.name, "Counter");
        assert!(def.fields.iter().any(|f| f.name == "count"));
        assert!(def.fields.iter().any(|f| f.name == "increment"));
    }
}
