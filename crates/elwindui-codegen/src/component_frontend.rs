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
use crate::{attr_frontend, ast, parser};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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
pub fn register_same_crate_component(name: &str, base: Option<&str>, item_struct: &syn::ItemStruct) {
    let stored = StoredComponent {
        base: base.map(str::to_string),
        struct_src: quote::quote! { #item_struct }.to_string(),
    };
    same_crate_components()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), name.to_string()), stored);
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
                items: vec![ast::Item::Component(component_def), ast::Item::View(view_def)],
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
        assert!(generated.contains("fn title"), "expected a `title` getter:\n{generated}");
        assert!(generated.contains("fn set_title"), "expected a `set_title` setter:\n{generated}");
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
        assert!(generated.contains("fn label"), "expected a `label` getter:\n{generated}");
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
        let set_volume_body = &generated[set_volume_start..(set_volume_start + 400).min(generated.len())];
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
        module.items.push(crate::ast::Item::Component(component_def));
        module.items.push(crate::ast::Item::View(view_def));

        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::builtin_modules())
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
        let set_volume_body = &generated[set_volume_start..(set_volume_start + 400).min(generated.len())];
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
            .chain(crate::builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("should validate");
        let table = build_symbol_table(&all_modules);
        let generated = generate_module(&module, &table);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let generated = generated.to_string();
        assert!(generated.contains("fn label"), "expected a `label` getter:\n{generated}");
        assert!(
            generated.contains("recompute_label"),
            "expected a recompute_label method:\n{generated}"
        );
        let set_volume_start = generated
            .find("fn set_volume")
            .expect("set_volume should be present");
        let set_volume_body = &generated[set_volume_start..(set_volume_start + 400).min(generated.len())];
        assert!(
            set_volume_body.contains("recompute_label"),
            "set_volume should cascade into recompute_label:\n{set_volume_body}"
        );
    }

    fn generate_and_check(src: &str) -> String {
        let module = crate::parser::parse_module(src).expect("dsl should parse");
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::builtin_modules())
            .collect();
        crate::validate::validate(&all_modules).expect("should validate");
        let table = build_symbol_table(&all_modules);
        let generated = generate_module(&module, &table);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        generated.to_string()
    }
}
