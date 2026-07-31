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

use crate::ast::{ComponentDef, FieldKind, Module, ViewDef};
use crate::{ast, attr_frontend, parser};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` (already
/// parsed as a `syn::ItemStruct` by the `elwindui-macros` proc-macro, `base` from the attribute's
/// own `inherits Base` argument) — builds the matching `ComponentDef`/`ViewDef` pair. `Name` may
/// omit the `view! { .. }` field entirely — same as a `.elwind` `component X { .. }` with no
/// paired `view X { .. }` block — in which case the second return value is `None` (the great
/// majority of builtins in `elwindui-core`/backend crates are `view`-less; this frontend's own
/// callers only ever chain a real builtin through `#[elwindui_macros::class]` directly, not this
/// one, but ordinary user components composed purely of `#[param]`/`#[prop]` fields with no view
/// tree of their own — e.g. a pure data-holding leaf meant to be constructed and never rendered
/// standalone — have the same legitimate shape).
pub fn component_and_view_from_item_struct(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<(ComponentDef, Option<ViewDef>), String> {
    let name = item_struct.ident.to_string();
    let (embedded, sealed, native, is_abstract, text_style, content_field) =
        component_item_attrs(&item_struct.attrs)?;

    let syn::Fields::Named(named) = &item_struct.fields else {
        return Err(format!("`{name}` must have named fields"));
    };

    let view_fields: Vec<&syn::Field> = named
        .named
        .iter()
        .filter(|f| is_view_macro_field(f))
        .collect();
    let view_field = match view_fields.as_slice() {
        [only] => Some(*only),
        [] => None,
        _ => {
            return Err(format!(
                "`{name}`: expected at most one field typed `view! {{ .. }}`, found {}",
                view_fields.len()
            ));
        }
    };

    let view_def = view_field
        .map(|view_field| {
            let syn::Type::Macro(view_macro) = &view_field.ty else {
                unreachable!(
                    "is_view_macro_field only returns fields whose type is a macro invocation"
                );
            };
            let view_src = view_macro.mac.tokens.to_string();
            let (on_mount, on_unmount, lets, root) = parser::parse_view_body(&view_src)
                .map_err(|e| format!("`{name}`: invalid `view! {{ .. }}` body: {e}"))?;
            Ok::<_, String>(ViewDef {
                target: name.clone(),
                on_mount,
                on_unmount,
                lets,
                root,
            })
        })
        .transpose()?;

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
        embedded,
        sealed,
        native,
        is_abstract,
        text_style,
        content_field,
    };

    Ok((component_def, view_def))
}

/// `#[embedded]`/`#[sealed]`/`#[native]`/`#[abstract_]`/`#[text_style]`/`#[content(field_name)]`,
/// read off `item_struct.attrs` — the Rust-macro-path counterpart of `parser.rs`'s
/// `parse_item_attrs`, same vocabulary, minus the `.elwind`-text-only wart of `abstract` colliding
/// with the reserved Rust keyword (spelled `abstract_` here instead — decided over introducing a
/// raw-identifier `r#abstract`, since this whole attribute vocabulary is otherwise plain
/// identifiers). Any other component-level attribute the user wrote (`#[derive(..)]`, doc
/// comments, ...) is left alone/ignored — `#[elwindui::component]` replaces the whole struct with
/// generated code, so nothing downstream ever re-emits `item_struct.attrs` verbatim.
fn component_item_attrs(
    attrs: &[syn::Attribute],
) -> Result<(bool, bool, bool, bool, bool, Option<String>), String> {
    let mut embedded = false;
    let mut sealed = false;
    let mut native = false;
    let mut is_abstract = false;
    let mut text_style = false;
    let mut content_field = None;
    for attr in attrs {
        let Some(attr_name) = attr.path().get_ident().map(|i| i.to_string()) else {
            continue;
        };
        match attr_name.as_str() {
            "embedded" => embedded = true,
            "sealed" => sealed = true,
            "native" => native = true,
            "abstract_" => is_abstract = true,
            "text_style" => text_style = true,
            "content" => {
                let field: syn::Ident = attr.parse_args().map_err(|e| {
                    format!("invalid #[content(field_name)] arguments: {e}")
                })?;
                content_field = Some(field.to_string());
            }
            _ => continue,
        }
    }
    Ok((embedded, sealed, native, is_abstract, text_style, content_field))
}

fn is_view_macro_field(field: &syn::Field) -> bool {
    matches!(&field.ty, syn::Type::Macro(tm) if tm.mac.path.is_ident("view"))
}

/// `ast::Item::Component` plus, only when present, `ast::Item::View` — every call site building a
/// `Module` from `component_and_view_from_item_struct`'s output needs this same conditional push
/// now that a `view!`-less component is legal (see that function's own doc comment).
pub(crate) fn component_module_items(
    component_def: ComponentDef,
    view_def: Option<ViewDef>,
) -> Vec<ast::Item> {
    let mut items = vec![ast::Item::Component(component_def)];
    if let Some(view_def) = view_def {
        items.push(ast::Item::View(view_def));
    }
    items
}

/// The identifier of the crate currently being compiled, read fresh from the environment variables
/// cargo (and rust-analyzer's own proc-macro-srv, same protocol/env vars) sets for *this*
/// macro-expansion request. Mirrors `elwindui-macros/src/class.rs`'s own `compiling_crate_key`
/// (duplicated rather than shared — `elwindui-codegen` is the crate `elwindui-macros` depends on,
/// not the other way around) — see that function's doc comment for the full rationale: without this
/// key, `same_crate_components()` (below) would leak one crate's registered components into a
/// completely unrelated crate the moment both get processed within the same rust-analyzer session
/// (rust-analyzer runs one persistent `proc-macro-srv` process workspace-wide, unlike a real `cargo
/// build`'s one-process-per-crate model where a fresh, empty `OnceLock` per compilation would be
/// enough on its own).
fn compiling_crate_key() -> String {
    std::env::var("CARGO_CRATE_NAME")
        .or_else(|_| std::env::var("CARGO_PKG_NAME"))
        .unwrap_or_default()
}

/// A registered `#[elwindui::component]` struct, kept as reparseable source text rather than a raw
/// `ComponentDef`/`ViewDef` (which are full of `syn::Expr`/`syn::Block` etc.) — those wrap the real
/// compiler's own (non-`Send`/`Sync`) proc-macro bridge types when this crate is compiled into an
/// actual proc-macro dylib, which a `static`-held `Mutex<..>` can't store. `quote!{ #item_struct
/// }.to_string()` turns it into a plain, `Send`-safe `String`; `sibling_component_modules` re-parses
/// it and re-runs `component_and_view_from_item_struct` on demand, the same "recover it via the same
/// construction path, don't duplicate the logic" approach `class.rs`'s own `StoredClassArgs` uses
/// for its `inherits`/`struct_only` fields.
struct StoredComponent {
    base: Option<String>,
    struct_src: String,
}

/// Keyed by `(compiling_crate_key(), component name)` — every `#[elwindui::component]` struct
/// successfully generated so far *within this same crate compilation* (see `compiling_crate_key`'s
/// own doc comment for why the crate key is part of the key at all). Populated by
/// `register_same_crate_component` right after a component's own codegen succeeds; read by
/// `sibling_component_modules` so a *later* `#[elwindui::component]` invocation in the same crate can
/// resolve an *earlier* one as a plain element type in its own `view! { .. }` — e.g.
/// `examples/notepad-inline`'s `NotepadWindow` referencing the `CustomCheckBox` declared earlier in
/// the same file. This only ever works in file/declaration order (a component can't see a sibling
/// declared *after* it) — the same order-dependency `class.rs` already relies on and documents for
/// its own struct-before-impl same-crate mechanism, not a new kind of fragility.
fn same_crate_components() -> &'static Mutex<HashMap<(String, String), StoredComponent>> {
    static REGISTRY: OnceLock<Mutex<HashMap<(String, String), StoredComponent>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `name`'s already-successfully-generated `#[elwindui::component(inherits base)] struct
/// item_struct { .. }` so a later same-crate `#[elwindui::component]` invocation can resolve it as a
/// sibling element type — see `same_crate_components`'s own doc comment. Only call this after this
/// component's own codegen has actually succeeded — a component that failed to generate must not
/// become resolvable by anything else.
pub fn register_same_crate_component(
    name: &str,
    base: Option<&str>,
    item_struct: &syn::ItemStruct,
) {
    let stored = StoredComponent {
        base: base.map(str::to_string),
        struct_src: quote::quote! { #item_struct }.to_string(),
    };
    same_crate_components()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), name.to_string()), stored);
}

/// A registered `#[elwindui::viewmodel] mod foo { .. }` — kept as reparseable source text for the
/// same reason `StoredComponent` is (a `ViewModelDef` is full of non-`Send`/`Sync` `syn` types a
/// `static`-held `Mutex` can't store).
struct StoredViewModel {
    item_mod_src: String,
}

/// Keyed by `(compiling_crate_key(), viewmodel type name)` — mirrors `same_crate_components`, but
/// for `#[elwindui::viewmodel] mod foo { struct Foo { .. } }` (`elwindui-macros`'s `viewmodel`
/// attribute macro doesn't keep the `mod` wrapper past expansion, so `Foo` itself is what a sibling
/// `#[elwindui::component]`'s field type or `bind!` target actually names — see
/// `register_same_crate_viewmodel`'s own doc comment). Populated by
/// `lib.rs::generate_viewmodel_from_item_mod`; read by `sibling_viewmodel_modules` so a
/// `#[bindable]`/`bind!`-using component elsewhere in the same crate can be checked against the
/// viewmodel's real fields instead of silently going unchecked (the gap 05d4861-era `validate.rs`
/// comments call out: without this, `vm.typo_field` never gets caught on the proc-macro path). Same
/// declaration-order requirement as `same_crate_components` — a viewmodel must be declared before
/// the component(s) that reference it.
fn same_crate_viewmodels() -> &'static Mutex<HashMap<(String, String), StoredViewModel>> {
    static REGISTRY: OnceLock<Mutex<HashMap<(String, String), StoredViewModel>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `name`'s already-successfully-generated `#[elwindui::viewmodel] mod item_mod { .. }` so
/// a later same-crate `#[elwindui::component]`/`#[elwindui::viewmodel]` invocation can resolve it —
/// see `same_crate_viewmodels`'s own doc comment. `name` is the viewmodel *struct's* name (e.g.
/// `DocumentViewModel`), not the enclosing `mod`'s name (e.g. `document_view_model`) — the two
/// usually differ, and it's the struct name real Rust code (field types, `bind!` targets) actually
/// references. Only call this after this viewmodel's own codegen has actually succeeded.
pub fn register_same_crate_viewmodel(name: &str, item_mod: &syn::ItemMod) {
    let stored = StoredViewModel {
        item_mod_src: quote::quote! { #item_mod }.to_string(),
    };
    same_crate_viewmodels()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), name.to_string()), stored);
}

/// Every same-crate `#[elwindui::viewmodel]` registered so far, rebuilt as one `Module` each — see
/// `same_crate_viewmodels`'s own doc comment. Unlike `sibling_component_modules`, there's no
/// `skip_name` guard: a viewmodel never references itself as a sibling type the way a component's
/// `view!` can reference another component.
pub fn sibling_viewmodel_modules() -> Vec<Module> {
    let key = compiling_crate_key();
    let store = same_crate_viewmodels().lock().unwrap();
    store
        .iter()
        .filter(|((crate_key, _), _)| crate_key == &key)
        .map(|(_, stored)| {
            let item_mod: syn::ItemMod = syn::parse_str(&stored.item_mod_src)
                .expect("internal: failed to reparse a registered sibling viewmodel's mod text");
            let def = attr_frontend::viewmodel_def_from_item_mod(&item_mod)
                .expect("internal: failed to rebuild a registered sibling viewmodel");
            Module {
                path: Vec::new(),
                uses: Vec::new(),
                items: vec![ast::Item::ViewModel(def)],
                allows_external_builtins: true,
                ..Default::default()
            }
        })
        .collect()
}

/// A registered `#[elwindui::dsl_enum] enum Foo { .. }` — kept as reparseable source text for the
/// same reason `StoredComponent`/`StoredViewModel` are.
struct StoredEnum {
    item_enum_src: String,
}

/// Keyed by `(compiling_crate_key(), enum name)` — mirrors `same_crate_viewmodels`, but for a plain
/// Rust `enum` a `view!`'s `match`/`if let` needs exhaustiveness-checked against. Populated by
/// `lib.rs::generate_dsl_enum_from_item_enum`; read by `sibling_enum_modules`. A bare `enum Name {
/// .. }` is otherwise invisible to any proc-macro (unlike a `#[elwindui::component]`/`#[elwindui::
/// viewmodel]` struct, nothing marks it as DSL-relevant) — `#[elwindui::dsl_enum]` is the opt-in:
/// the enum body passes through untouched (it's still a real Rust `enum`, matched with real Rust
/// `match`), this registry is the only side effect. Same declaration-order requirement as the other
/// two registries — an enum must be declared before the component(s) that match over it.
fn same_crate_enums() -> &'static Mutex<HashMap<(String, String), StoredEnum>> {
    static REGISTRY: OnceLock<Mutex<HashMap<(String, String), StoredEnum>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `name`'s already-emitted `#[elwindui::dsl_enum] enum item_enum { .. }` — see
/// `same_crate_enums`'s own doc comment.
pub fn register_same_crate_enum(name: &str, item_enum: &syn::ItemEnum) {
    let stored = StoredEnum {
        item_enum_src: quote::quote! { #item_enum }.to_string(),
    };
    same_crate_enums()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), name.to_string()), stored);
}

/// Every same-crate `#[elwindui::dsl_enum]` registered so far, rebuilt as one `Module` each — see
/// `same_crate_enums`'s own doc comment.
pub fn sibling_enum_modules() -> Vec<Module> {
    let key = compiling_crate_key();
    let store = same_crate_enums().lock().unwrap();
    store
        .iter()
        .filter(|((crate_key, _), _)| crate_key == &key)
        .map(|(_, stored)| {
            let item_enum: syn::ItemEnum = syn::parse_str(&stored.item_enum_src)
                .expect("internal: failed to reparse a registered sibling enum's item text");
            let def = enum_def_from_item_enum(&item_enum)
                .expect("internal: failed to rebuild a registered sibling enum");
            Module {
                path: Vec::new(),
                uses: Vec::new(),
                items: vec![ast::Item::Enum(def)],
                allows_external_builtins: true,
                ..Default::default()
            }
        })
        .collect()
}

/// `#[elwindui::dsl_enum] enum Name { A, B, C }` -> `EnumDef { name: "Name", variants: ["A", "B",
/// "C"] }`. Every variant must be a bare unit variant — same restriction `.elwind`'s own `enum`
/// syntax has (§7 of the DSL spec: "no anonymous unions", enums are plain value sets), and there's
/// no way to `match` a tuple/struct variant's payload from `view!`'s own limited match-arm syntax
/// anyway.
pub fn enum_def_from_item_enum(item_enum: &syn::ItemEnum) -> Result<ast::EnumDef, String> {
    let name = item_enum.ident.to_string();
    let variants = item_enum
        .variants
        .iter()
        .map(|v| {
            if !matches!(v.fields, syn::Fields::Unit) {
                return Err(format!(
                    "enum `{name}`: variant `{}` must be a bare unit variant (no payload)",
                    v.ident
                ));
            }
            Ok(v.ident.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ast::EnumDef { name, variants })
}

/// Every same-crate sibling `#[elwindui::component]` registered so far (via
/// `register_same_crate_component`), other than `skip_name` itself, rebuilt as one `Module` each
/// (`path: []`, matching the flat crate-root visibility every `#[elwindui::component]`-generated type
/// actually has in real Rust — see `builtin_modules`'s own doc comment for why two `Module`s sharing
/// that same empty path already resolve against each other with no `use` needed). `skip_name` guards
/// against a stale self-entry from an earlier rust-analyzer pass over the same struct colliding with
/// the module this invocation is about to build for itself.
pub fn sibling_component_modules(skip_name: &str) -> Vec<Module> {
    let key = compiling_crate_key();
    let store = same_crate_components().lock().unwrap();
    store
        .iter()
        .filter(|((crate_key, name), _)| crate_key == &key && name != skip_name)
        .map(|(_, stored)| {
            let item_struct: syn::ItemStruct = syn::parse_str(&stored.struct_src)
                .expect("internal: failed to reparse a registered sibling component's struct text");
            let (component_def, view_def) =
                component_and_view_from_item_struct(stored.base.clone(), &item_struct)
                    .expect("internal: failed to rebuild a registered sibling component");
            Module {
                path: Vec::new(),
                uses: Vec::new(),
                items: component_module_items(component_def, view_def),
                allows_external_builtins: true,
                ..Default::default()
            }
        })
        .collect()
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
            items: component_module_items(component_def, view_def),
            allows_external_builtins: true,
            ..Default::default()
        };
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
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

    // Phase 2: a `view!`-less component is legal (the Rust-macro-path counterpart of a `.elwind`
    // `component X { .. }` with no paired `view X { .. }` block) — see
    // `component_and_view_from_item_struct`'s own doc comment.
    #[test]
    fn missing_view_field_yields_a_view_less_component() {
        let src = r#"
            struct Counter {
                #[param]
                start: i32,
            }
        "#;
        let item_struct: syn::ItemStruct = syn::parse_str(src).unwrap();
        let (component_def, view_def) =
            component_and_view_from_item_struct(Some("Window".to_string()), &item_struct)
                .expect("a view!-less component should build successfully");
        assert_eq!(component_def.name, "Counter");
        assert!(view_def.is_none());
    }

    #[test]
    fn multiple_view_fields_is_an_error() {
        let src = r#"
            struct Counter {
                #[param]
                start: i32,
                a: view! { TextBlock { text: "a" } },
                b: view! { TextBlock { text: "b" } },
            }
        "#;
        let item_struct: syn::ItemStruct = syn::parse_str(src).unwrap();
        let err = component_and_view_from_item_struct(Some("Window".to_string()), &item_struct)
            .unwrap_err();
        assert!(err.contains("at most one"), "error should mention the cardinality: {err}");
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
            .chain(crate::test_builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("dsl should validate");
        let table = build_symbol_table(&all_modules);
        let dsl_generated = generate_module(&module, &table).to_string();

        assert_eq!(attr_generated, dsl_generated);
    }

    // Phase 2: component-level attributes read straight off `item_struct.attrs`, the Rust-macro
    // counterpart of `parser.rs`'s `#[embedded]`/`#[sealed]`/`#[native]`/`#[content(field)]`
    // vocabulary (`abstract_` here, not `abstract` — a reserved Rust keyword). Checked directly
    // against `component_and_view_from_item_struct`'s own return value, not through `generate()`'s
    // full `validate` pipeline — `#[embedded]` is only valid on a `Module::is_builtin` module,
    // which this test's ad hoc `Module` isn't.
    #[test]
    fn component_level_attrs_are_read_from_struct_attrs() {
        let src = r#"
            #[sealed]
            #[native]
            #[abstract_]
            #[content(children)]
            struct Toolbar {
                #[prop]
                children: i32,
            }
        "#;
        let item_struct: syn::ItemStruct =
            syn::parse_str(src).expect("struct should parse as valid Rust");
        let (component_def, view_def) = component_and_view_from_item_struct(None, &item_struct)
            .expect("should build a ComponentDef");
        assert!(component_def.sealed, "#[sealed] should set ComponentDef::sealed");
        assert!(component_def.native, "#[native] should set ComponentDef::native");
        assert!(component_def.is_abstract, "#[abstract_] should set ComponentDef::is_abstract");
        assert!(!component_def.embedded, "#[embedded] was not written, should stay false");
        assert!(!component_def.text_style, "#[text_style] was not written, should stay false");
        assert_eq!(
            component_def.content_field.as_deref(),
            Some("children"),
            "#[content(children)] should set ComponentDef::content_field"
        );
        assert!(view_def.is_none(), "no `view! {{ .. }}` field, so the view should be None");
    }

    // Phase 2: `#[param(default = ...)]`, mirroring `#[prop(default = ...)]`'s existing
    // token-based routing through `parser::parse_initializer` (so `bind!(..)` sugar parses the
    // same way, even though `validate`'s param-staticness checks are a separate, pre-existing
    // concern this frontend doesn't duplicate — see `attr_frontend::fields_from_item_struct`'s own
    // doc comment).
    #[test]
    fn param_default_attribute_sets_initializer() {
        let src = r#"
            struct Greeting {
                #[param(default = "hi".to_string())]
                label: String,
            }
        "#;
        let item_struct: syn::ItemStruct =
            syn::parse_str(src).expect("struct should parse as valid Rust");
        let (component_def, _view_def) = component_and_view_from_item_struct(None, &item_struct)
            .expect("should build a ComponentDef");
        let field = component_def
            .fields
            .iter()
            .find(|f| f.name == "label")
            .expect("field `label` should exist");
        assert!(matches!(field.kind, FieldKind::Param));
        assert!(
            field.initializer.is_some(),
            "#[param(default = ...)] should set an initializer"
        );
    }
}

/// Exercises a component's own `#[prop(default = ...)]`/`#[computed(expr = ...)]` fields —
/// referenced bare from that *same* component's own `view!` — through the full pipeline
/// (`component_and_view_from_item_struct` -> `validate` -> `generate_module`). This combination
/// (as opposed to a `viewmodel`'s `#[observable]`/`#[computed]`, referenced via `vm.field`) had no
/// codegen support at all before `generate_view`/`generate_component` grew it: `own_fields`, and
/// everything derived from it, used to filter to `f.initializer.is_none()` only, so a bare
/// same-component reference like `text: label` failed with "unsupported path shape after bind
/// resolution". See docs/elwindui_dsl_spec.md's "Rustファイル内での代替記法" subsection, whose
/// `VolumeControl` example this mirrors.
#[cfg(test)]
mod doc_example_own_default_and_computed_fields {
    use crate::codegen::{build_symbol_table, generate_module};

    /// The minimal case: a `#[prop(default = ...)]` field referenced bare in its own view, no
    /// `#[computed]`, no `inherits`, no dynamic (`match`/`if`) child region.
    #[test]
    fn own_default_prop_referenced_bare_in_own_view() {
        let src = r#"
component Greeter {
    #[prop]
    title: String = "hi".to_string(),
}

view Greeter {
    TextBlock { text: title }
}
"#;
        let generated = generate_and_check(src);
        assert!(
            generated.contains("fn title"),
            "expected a `title` getter:\n{generated}"
        );
        assert!(
            generated.contains("fn set_title"),
            "expected a `set_title` setter:\n{generated}"
        );
    }

    /// A `#[computed]` field depending on a `#[prop(default = ...)]` field, both referenced bare in
    /// the owning component's own view — pins the `recompute_<name>`/`on_property_changed` cascade
    /// a defaulted-prop's setter must trigger for any computed field that depends on it.
    #[test]
    fn own_computed_field_depending_on_own_default_prop() {
        let src = r#"
component Greeter {
    #[prop]
    volume: i32 = 50,

    #[computed]
    label: String = volume.to_string() + "%",
}

view Greeter {
    TextBlock { text: label }
}
"#;
        let generated = generate_and_check(src);
        assert!(
            generated.contains("fn label"),
            "expected a `label` getter:\n{generated}"
        );
        assert!(
            generated.contains("recompute_label"),
            "expected a recompute_label method:\n{generated}"
        );
        assert!(
            generated.contains("fn set_volume"),
            "expected a `set_volume` setter:\n{generated}"
        );
        // `set_volume` must cascade into recomputing + notifying `label`, not just itself.
        let set_volume_start = generated
            .find("fn set_volume")
            .expect("set_volume should be present");
        let set_volume_body =
            &generated[set_volume_start..(set_volume_start + 400).min(generated.len())];
        assert!(
            set_volume_body.contains("recompute_label"),
            "set_volume should cascade into recompute_label:\n{set_volume_body}"
        );
    }

    /// The exact `docs/elwindui_dsl_spec.md` "Rustファイル内での代替記法" example: `VolumeControl`
    /// inherits `ContentControl` (a real builtin, already shape-composed over `Control`), and
    /// branches over a `#[param] orientation: Orientation` via `match` inside `view!`, referencing
    /// its own `#[prop(default = 50)] volume`/`#[computed] label` fields bare from inside the match
    /// arms' nested `TextBlock`s.
    #[test]
    fn doc_volume_control_example() {
        let deps_src = r#"
enum Orientation {
    Horizontal,
    Vertical,
}
"#;
        let deps_module = crate::parser::parse_module(deps_src).expect("deps should parse");

        let struct_src = r#"
            struct VolumeControl {
                #[param]
                orientation: Orientation,

                #[prop(default = 50)]
                volume: i32,

                #[computed(expr = volume.to_string() + "%")]
                label: String,

                body: view! {
                    match orientation {
                        Orientation::Horizontal => { HorizontalLayout { TextBlock { text: label } } }
                        Orientation::Vertical => { VerticalLayout { TextBlock { text: label } } }
                    }
                }
            }
        "#;
        let item_struct: syn::ItemStruct =
            syn::parse_str(struct_src).expect("struct should parse as valid Rust");
        let (component_def, view_def) = super::component_and_view_from_item_struct(
            Some("ContentControl".to_string()),
            &item_struct,
        )
        .expect("should build ComponentDef/ViewDef");

        let mut module = deps_module;
        module
            .items
            .extend(crate::component_frontend::component_module_items(
                component_def,
                view_def,
            ));

        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("should validate");
        let table = build_symbol_table(&all_modules);
        let generated = generate_module(&module, &table);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let generated = generated.to_string();
        let set_volume_start = generated
            .find("fn set_volume")
            .expect("set_volume should be present");
        let set_volume_body =
            &generated[set_volume_start..(set_volume_start + 400).min(generated.len())];
        assert!(
            set_volume_body.contains("recompute_label"),
            "set_volume should cascade into recompute_label:\n{set_volume_body}"
        );
    }

    /// `generate_component` (a view-less component — `Item::Component` with no `Item::View`
    /// anywhere in its `inherits` chain, `generate_module`'s `None` branch) needed the exact same
    /// fix as `generate_view` — it used to `panic!("... initializer form not supported yet")` for
    /// any `#[prop(default = ...)]`/`#[computed(...)]` field at all.
    #[test]
    fn view_less_component_own_default_and_computed_fields() {
        let src = r#"
component Settings {
    #[prop]
    volume: i32 = 50,

    #[computed]
    label: String = volume.to_string() + "%",
}
"#;
        let module = crate::parser::parse_module(src).expect("dsl should parse");
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("should validate");
        let table = build_symbol_table(&all_modules);
        let generated = generate_module(&module, &table);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let generated = generated.to_string();
        assert!(
            generated.contains("fn label"),
            "expected a `label` getter:\n{generated}"
        );
        assert!(
            generated.contains("recompute_label"),
            "expected a recompute_label method:\n{generated}"
        );
        let set_volume_start = generated
            .find("fn set_volume")
            .expect("set_volume should be present");
        let set_volume_body =
            &generated[set_volume_start..(set_volume_start + 400).min(generated.len())];
        assert!(
            set_volume_body.contains("recompute_label"),
            "set_volume should cascade into recompute_label:\n{set_volume_body}"
        );
    }

    fn generate_and_check(src: &str) -> String {
        let module = crate::parser::parse_module(src).expect("dsl should parse");
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("should validate");
        let table = build_symbol_table(&all_modules);
        let generated = generate_module(&module, &table);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        generated.to_string()
    }
}
