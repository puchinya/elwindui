//! Alternative frontend: builds the same `ViewModelDef`/`FieldDef` AST (`ast.rs`, unchanged) that
//! `parser.rs`'s hand-written recursive-descent parser produces from DSL text — but from
//! real Rust syntax instead (a `syn::ItemMod` containing a `struct` + an `impl` block). This is
//! what lets `viewmodel`s be written as ordinary Rust (matching how WPF-style MVVM keeps the
//! ViewModel in the host language, not markup — see docs/design/runtime/state_management_design.md) while
//! `view { ... }` trees still need `parser.rs` (bare nested child elements aren't valid
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
/// `elwindui-macros`, so there's no dependency on Rust's proc-macro expansion order — useful for
/// any caller that needs a viewmodel's shape *before* the crate's own macro expansion has run (a
/// `build.rs`, or a language server processing a file in isolation — see
/// `docs/status/implementation_status.md` for whether such a caller currently exists; the
/// text-frontend-era `build.rs` caller this was originally written for, `compile_dir_with_extra_
/// viewmodels`, was removed once DSL text compilation itself was, so this is currently
/// exercised only by its own test below).
///
/// The mod name is what lets a caller build this viewmodel's real, crate-relative path (`Module::path`,
/// e.g. `["notepad_view_model"]` for `main.rs`'s `mod notepad_view_model { .. }`), so a DSL
/// module's `use crate::notepad_view_model::NotepadViewModel;` can be resolved against it exactly like
/// Rust's own name resolution (§12, docs/design/tools/codegen_design.md) — the struct name alone isn't enough to know where it
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
/// `#[observable]`/`#[computed]` field declarations; every `fn`/`async fn` in the `impl` block is
/// itself an action (no separate struct-side declaration needed — see `synthesize_action_fields`).
/// A single macro invocation must see both together — Rust attribute macros only ever see one
/// annotated item, so there's no way to correlate a separately-expanded `struct`-only macro with an
/// `impl`-only macro afterwards.
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
    let mut fields = fields_from_item_struct(item_struct, FieldKind::Observable, false)?;

    if let Some(item_impl) = item_impl {
        fields.extend(synthesize_action_fields(item_impl));
    }

    Ok(ViewModelDef { name, fields })
}

/// Builds `FieldDef`s from a `syn::ItemStruct`'s named fields, recognizing the field-attribute
/// vocabulary `docs/specs/dsl_spec.md` §3/§4 documents — `param`/`prop`/
/// `state`/`observable`/`computed`/`attached`/`environment`/`inject`/`two_way`/`routed`/`overrides`/
/// `onetime`/`length` — uniformly whether the caller is a `viewmodel` (`default_kind:
/// FieldKind::Observable`, via `viewmodel_def_from_item_mod`) or a `component` (`default_kind:
/// FieldKind::Prop`, via `component_frontend.rs`), exactly mirroring `parse_module`'s two
/// `parse_fields_block` call sites. Whether a particular kind/attribute combination is actually
/// *sensible* (e.g. `#[observable]` on a component field) is left to `validate::validate`, same as
/// hand-written DSL text — no duplicate validation here. `FieldKind::Action` never appears here —
/// actions are synthesized separately from the `impl` block, see `synthesize_action_fields`.
///
/// `allow_state` also gates `#[environment]`: both are component-only concepts (a `viewmodel` has
/// no UI context to inherit, docs/specs/dsl_spec.md §4/§13 rule 19's MVVM separation).
///
/// `#[state(default = expr)]`/`#[observable(default = expr)]`/`#[computed(expr = expr)]` parse their value as a plain
/// `syn::Expr` (`parse_name_value_expr`) — fine since neither needs view-attribute syntax.
/// `#[prop(default = ...)]`/`#[attached(default = ...)]` instead route their raw token text
/// through `parser::parse_initializer` (`parse_name_value_tokens`) so every frontend shares the
/// same initializer syntax and diagnostics.
pub(crate) fn fields_from_item_struct(
    item_struct: &syn::ItemStruct,
    default_kind: FieldKind,
    allow_state: bool,
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
        let mut explicit_kind: Option<String> = None;

        for attr in &field.attrs {
            let Some(attr_name) = attr.path().get_ident().map(|i| i.to_string()) else {
                return Err(format!("field `{name}`: expected a simple attribute name"));
            };
            match attr_name.as_str() {
                "param" => {
                    record_explicit_kind(&mut explicit_kind, &name, "param")?;
                    kind = FieldKind::Param;
                    if let Some(tokens) = parse_name_value_tokens(attr, "default")? {
                        initializer =
                            Some(parser::parse_initializer(&tokens.to_string()).map_err(|e| {
                                format!("field `{name}`: invalid #[param(default = ...)]: {e}")
                            })?);
                    }
                }
                "prop" => {
                    record_explicit_kind(&mut explicit_kind, &name, "prop")?;
                    kind = FieldKind::Prop;
                    if let Some(tokens) = parse_name_value_tokens(attr, "default")? {
                        initializer =
                            Some(parser::parse_initializer(&tokens.to_string()).map_err(|e| {
                                format!("field `{name}`: invalid #[prop(default = ...)]: {e}")
                            })?);
                    }
                }
                "state" => {
                    record_explicit_kind(&mut explicit_kind, &name, "state")?;
                    if !allow_state {
                        return Err(format!(
                            "field `{name}`: #[state] is only allowed on a component"
                        ));
                    }
                    kind = FieldKind::State;
                    let default = parse_name_value_expr(attr, "default")?.ok_or_else(|| {
                        format!("field `{name}`: #[state(...)] needs `default = expr`")
                    })?;
                    initializer = Some(Initializer::Expr(default));
                }
                "observable" => {
                    record_explicit_kind(&mut explicit_kind, &name, "observable")?;
                    kind = FieldKind::Observable;
                    let default = parse_name_value_expr(attr, "default")?.ok_or_else(|| {
                        format!("field `{name}`: #[observable(...)] needs `default = expr`")
                    })?;
                    initializer = Some(Initializer::Expr(default));
                }
                "computed" => {
                    record_explicit_kind(&mut explicit_kind, &name, "computed")?;
                    kind = FieldKind::Computed;
                    let expr = parse_name_value_expr(attr, "expr")?.ok_or_else(|| {
                        format!("field `{name}`: #[computed(...)] needs `expr = expr`")
                    })?;
                    initializer = Some(Initializer::Expr(expr));
                }
                "attached" => {
                    record_explicit_kind(&mut explicit_kind, &name, "attached")?;
                    kind = FieldKind::Attached;
                    if let Some(tokens) = parse_name_value_tokens(attr, "default")? {
                        initializer =
                            Some(parser::parse_initializer(&tokens.to_string()).map_err(|e| {
                                format!("field `{name}`: invalid #[attached(default = ...)]: {e}")
                            })?);
                    }
                }
                "environment" => {
                    record_explicit_kind(&mut explicit_kind, &name, "environment")?;
                    if !allow_state {
                        return Err(format!(
                            "field `{name}`: #[environment] is only allowed on a component"
                        ));
                    }
                    kind = FieldKind::Environment;
                    let key_name = attr.parse_args::<syn::Ident>().map_err(|e| {
                        format!("field `{name}`: invalid #[environment(name)]: {e}")
                    })?;
                    attrs.push(Attr::Environment(key_name.to_string()));
                }
                "inject" => attrs.push(Attr::Inject),
                "bindable" => {
                    record_explicit_kind(&mut explicit_kind, &name, "bindable")?;
                    kind = FieldKind::Param;
                    attrs.push(Attr::Inject);
                    attrs.push(Attr::Bindable);
                }
                "two_way" => attrs.push(Attr::TwoWay),
                "routed" => attrs.push(Attr::Routed),
                "overrides" => attrs.push(Attr::Override),
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

        if kind == FieldKind::State && !attrs.is_empty() {
            return Err(format!(
                "field `{name}`: #[state] cannot be combined with inject, bindable, two_way, routed, overrides, onetime, or length"
            ));
        }

        // Unlike hand-written DSL text, a plain Rust struct field has no `= expr` syntax of
        // its own — `#[observable(default = ...)]`/`#[computed(expr = ...)]` are the only place
        // either kind's value can be written, so (whether `kind` came from an explicit attribute
        // or fell back to `default_kind`) both must end up with an initializer.
        if matches!(
            kind,
            FieldKind::State | FieldKind::Observable | FieldKind::Computed
        ) && initializer.is_none()
        {
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

fn record_explicit_kind(
    current: &mut Option<String>,
    field_name: &str,
    new_kind: &str,
) -> Result<(), String> {
    if let Some(previous) = current {
        return Err(format!(
            "field `{field_name}`: #[{previous}] cannot be combined with #[{new_kind}]"
        ));
    }
    *current = Some(new_kind.to_string());
    Ok(())
}

/// Builds one `FieldDef { kind: Action, .. }` per `fn`/`async fn` in the mod's `impl` block — no
/// struct-side declaration is needed or matched against; the `impl` block alone is the action
/// list. `params`/`body` come straight from the `fn`'s own signature/block (`rewrite_action_body`,
/// called unconditionally by `generate_viewmodel`, does the sibling-field-reference rewriting);
/// async-ness is read directly from the `fn`'s own `async` keyword rather than a separate
/// attribute. `ty` is unused by `generate_viewmodel`'s `FieldKind::Action` arm (an action has no
/// declared Rust type of its own — its generated method's signature comes from `params`/the
/// `fn`'s `async`-ness) and left empty.
fn synthesize_action_fields(item_impl: &syn::ItemImpl) -> Vec<FieldDef> {
    item_impl
        .items
        .iter()
        .filter_map(|item| {
            let syn::ImplItem::Fn(item_fn) = item else {
                return None;
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

            Some(FieldDef {
                name: item_fn.sig.ident.to_string(),
                ty: String::new(),
                kind: FieldKind::Action,
                attrs: Vec::new(),
                initializer: Some(Initializer::Action {
                    params,
                    is_async: item_fn.sig.asyncness.is_some(),
                    body: item_fn.block.clone(),
                }),
            })
        })
        .collect()
}

/// `syn::Type` -> the tight string form the rest of `codegen.rs` expects (it round-trips field
/// types through plain string matching — `is_copy_type`, `nested_vec_item_type`, the many
/// `ty.contains("dyn UIElement")` checks — since that's the form `parser.rs` produces by slicing
/// raw source text). `quote!`'s `Display` inserts a space around every token (`Vec < Document >`,
/// `dyn UIElement`), so most of it has to be stripped back out here — but *not* a space that sits
/// between two word characters (e.g. the one in `dyn UIElement`), since that one is a mandatory
/// token separator in valid Rust (`parser.rs`'s raw-slice form always keeps it too, for the same
/// reason) and blindly dropping it collapsed `dyn UIElement` into the never-matching `dynUIElement`
/// (Issue #68 bug 4).
fn type_to_compact_string(ty: &syn::Type) -> String {
    let rendered = quote::quote! { #ty }.to_string();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut compact = String::with_capacity(rendered.len());
    let mut chars = rendered.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ' ' {
            let prev_is_word = compact.chars().next_back().is_some_and(is_word);
            let next_is_word = chars.peek().is_some_and(|&next| is_word(next));
            if prev_is_word && next_is_word {
                compact.push(' ');
            }
        } else {
            compact.push(c);
        }
    }
    compact
}

/// Parses `#[attr_name(name = expr)]`'s inner `name = expr` and returns `expr` if present —
/// `Ok(None)` for a bare `#[attr_name]` with no parenthesized arguments at all.
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
/// = ...)]`, which (unlike `observable`/`computed`) need owned-string coercion recognized via
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
    fn impl_fn_becomes_an_action_method() {
        let src = r#"
            mod vm {
                struct Counter {
                    #[observable(default = 0i32)]
                    count: i32,

                    #[computed(expr = count < 10)]
                    increment_can_execute: bool,
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
        assert!(s.contains("fn increment"));
        assert!(s.contains("fn increment_can_execute"));
        // The body's bare `count` reference must have been rewritten to `self.count()`/
        // `self.set_count(...)` by the same `rewrite_action_body` the DSL path uses.
        assert!(s.contains("self . set_count"));
    }

    /// No struct-side declaration is needed for an action — an `impl` `fn` with no field of the
    /// same name is not an error (unlike the old `#[command]` pairing scheme).
    #[test]
    fn impl_fn_needs_no_matching_struct_field() {
        let src = r#"
            mod vm {
                struct Counter {}
                impl Counter {
                    fn increment(&self) {}
                }
            }
        "#;
        let item_mod: syn::ItemMod = syn::parse_str(src).unwrap();
        let def = viewmodel_def_from_item_mod(&item_mod).expect("should build a ViewModelDef");
        assert!(def.fields.iter().any(|f| f.name == "increment"));
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

    #[test]
    fn component_state_requires_a_default_and_rejects_kind_conflicts() {
        let missing: syn::ItemStruct = syn::parse_quote! {
            struct Search { #[state] query: String }
        };
        let error = fields_from_item_struct(&missing, FieldKind::Prop, true)
            .expect_err("state default is mandatory");
        assert!(error.contains("default = expr"), "{error}");

        let conflicting: syn::ItemStruct = syn::parse_quote! {
            struct Search {
                #[state(default = "")]
                #[prop(default = String::new())]
                query: String,
            }
        };
        let error = fields_from_item_struct(&conflicting, FieldKind::Prop, true)
            .expect_err("state and prop must conflict");
        assert!(error.contains("cannot be combined"), "{error}");
    }

    #[test]
    fn viewmodel_rejects_component_state() {
        let item: syn::ItemStruct = syn::parse_quote! {
            struct Search { #[state(default = "")] query: String }
        };
        let error = fields_from_item_struct(&item, FieldKind::Observable, false)
            .expect_err("state is component-only");
        assert!(error.contains("only allowed on a component"), "{error}");
    }

    /// Issue #68 bug 4: `quote!`'s `Display` renders `dyn UIElement` with a space (mandatory
    /// Rust token separator), and `type_to_compact_string` used to strip *every* space
    /// unconditionally, collapsing it into the never-matching `dynUIElement` — so a `dyn
    /// UIElement`-typed field never got recognized as UIElement-typed by any of `codegen.rs`'s
    /// `ty.contains("dyn UIElement")` checks (e.g. `generate_view`'s `lets_map` seeding, which
    /// makes a bare self-field reference valid in child-element position).
    #[test]
    fn compact_type_string_keeps_the_space_in_dyn_trait_bounds() {
        let item: syn::ItemStruct = syn::parse_quote! {
            struct ContentControl {
                content: std::rc::Rc<dyn UIElement>,
            }
        };
        let fields = fields_from_item_struct(&item, FieldKind::Prop, true)
            .expect("plain struct field should parse");
        let content = fields
            .iter()
            .find(|f| f.name == "content")
            .expect("`content` field");
        assert_eq!(content.ty, "std::rc::Rc<dyn UIElement>");
        assert!(content.ty.contains("dyn UIElement"));
    }
}
