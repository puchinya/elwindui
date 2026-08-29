//! Alternative frontend, sibling to `attr_frontend.rs`'s viewmodel path: builds the same
//! `ComponentDef`/`ViewDef` AST (`ast.rs`, unchanged) that `parser.rs`'s hand-written
//! recursive-descent parser produces from DSL text — but from a real Rust `struct`
//! instead, annotated `#[elwindui::component(inherits Base)]`. Ordinary fields become the
//! component's `#[param]`/`#[prop]`/etc. fields (via `attr_frontend::fields_from_item_struct`,
//! shared with the viewmodel frontend); at most one authored presentation field, typed as a
//! `view!` or `template_view!` macro invocation (`field: view! { .. }` / `template: template_view!
//! { .. }`, parsed by `syn` as `syn::Type::Macro` — legal Rust in type position), supplies the
//! ordinary view or typed ControlTemplate definition.
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

/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` or
/// `template: template_view! { .. }` (already
/// parsed as a `syn::ItemStruct` by the `elwindui-macros` proc-macro, `base` from the attribute's
/// own `inherits Base` argument) — builds the matching `ComponentDef`/`ViewDef` pair. `Name` may
/// omit the `view! { .. }` field entirely — same as a DSL `component X { .. }` with no
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
    let item_template_instance = item_struct
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("template_instance"));
    // `base` arrives as whatever `elwindui_macros::path_to_string` produced from the attribute's own
    // `inherits <path>` argument: a bare name (`ContentControl`) or a full crate-root-qualified path
    // (`crate::ui::LabeledPanel`, Refs #25). Split it here rather than upstream — this is the one
    // place both halves (`ComponentDef::base`, the bare symbol-table name every resolution site
    // already expects, and `ComponentDef::base_path`, the qualified spelling `codegen::generate_view`
    // needs to emit) come together.
    let (base, base_path) = split_base_path(base);
    let (sealed, is_abstract, text_style, content_field) =
        component_item_attrs(&item_struct.attrs)?;

    let syn::Fields::Named(named) = &item_struct.fields else {
        return Err(format!("`{name}` must have named fields"));
    };

    if named.named.iter().any(|field| {
        field
            .ident
            .as_ref()
            .is_some_and(|ident| ident == "template")
            && !is_presentation_macro_field(field)
    }) {
        return Err(format!(
            "`{name}`: `template` is reserved for `template: template_view! {{ ... }}` and cannot be declared as a normal property"
        ));
    }

    let view_fields: Vec<&syn::Field> = named
        .named
        .iter()
        .filter(|f| is_presentation_macro_field(f))
        .collect();
    let view_field = match view_fields.as_slice() {
        [only] => Some(*only),
        [] => None,
        _ => {
            return Err(format!(
                "`{name}`: expected at most one authored presentation field (`body: view! {{ .. }}` or `template: template_view! {{ .. }}`), found {}",
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
            let is_template = view_field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "template");
            let template_instance = item_template_instance
                || view_field
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("template_instance"));
            let macro_is_template = view_macro.mac.path.is_ident("template_view");
            let expected_field = if is_template { "template" } else { "body" };
            if (is_template != macro_is_template)
                || (!is_template && !view_macro.mac.path.is_ident("view"))
            {
                return Err(format!(
                    "`{name}`: `{expected_field}` must use {}",
                    if is_template {
                        "`template_view! { ... }`"
                    } else {
                        "`view! { ... }`"
                    }
                ));
            }
            let view_src = view_macro.mac.tokens.to_string();
            let (on_mount, on_unmount, on_update, lets, root) = parser::parse_view_body(&view_src)
                .map_err(|e| format!("`{name}`: invalid authored presentation body: {e}"))?;
            Ok::<_, String>(ViewDef {
                target: name.clone(),
                is_template,
                template_instance,
                on_mount,
                on_unmount,
                on_update,
                lets,
                root,
                implicit_owner: None,
            })
        })
        .transpose()?;

    let mut non_view_struct = item_struct.clone();
    if let syn::Fields::Named(named) = &mut non_view_struct.fields {
        named.named = named
            .named
            .iter()
            .filter(|f| !is_presentation_macro_field(f))
            .cloned()
            .collect();
    }
    let mut fields =
        attr_frontend::fields_from_item_struct(&non_view_struct, FieldKind::Prop, true)?;
    attr_frontend::normalize_component_fields(&mut fields);

    let component_def = ComponentDef {
        name,
        base,
        base_path,
        fields,
        methods: Vec::new(),
        // `#[embedded]`/`#[native]` are no longer recognized attribute names for this (real,
        // production) frontend — see `component_item_attrs`'s own doc comment. Only `testdata.rs`'s
        // test-only Rust-literal `builtin_component` helper still produces a `ComponentDef` with
        // either field `true`.
        embedded: false,
        sealed,
        native: false,
        is_abstract,
        text_style,
        content_field,
    };

    Ok((component_def, view_def))
}

/// Splits a raw `inherits` argument string (as `elwindui_macros::path_to_string` produced it) into
/// its bare symbol-table name and, only when the DSL author wrote a qualified path, that full path
/// text — see `ComponentDef::base_path`'s own doc comment. `"ContentControl"` splits to
/// `(Some("ContentControl"), None)`; `"crate::ui::LabeledPanel"` splits to
/// `(Some("LabeledPanel"), Some("crate::ui::LabeledPanel"))`.
fn split_base_path(base: Option<String>) -> (Option<String>, Option<String>) {
    let Some(raw) = base else {
        return (None, None);
    };
    match raw.rsplit_once("::") {
        Some((_, bare)) => (Some(bare.to_string()), Some(raw)),
        None => (Some(raw), None),
    }
}

/// `#[sealed]`/`#[abstract_]`/`#[text_style]`/`#[content(field_name)]`, read off
/// `item_struct.attrs` — decided `abstract` should be spelled `abstract_` here instead of the
/// reserved Rust keyword colliding with a raw identifier (`r#abstract`), since this whole attribute
/// vocabulary is otherwise plain identifiers. Any other component-level attribute the user wrote
/// (`#[derive(..)]`, doc comments, ...) is left alone/ignored — `#[elwindui::component]` replaces
/// the whole struct with generated code, so nothing downstream ever re-emits `item_struct.attrs`
/// verbatim.
///
/// Deliberately does **not** recognize `#[embedded]`/`#[native]` — those are for
/// `elwindui-codegen`'s own test-only builtin-shape fixture only (`testdata.rs`'s Rust-literal
/// `ComponentDef` construction); `validate::validate` rejects both on any component whose
/// `Module::is_builtin` isn't set, which no real (non-test) invocation of this frontend ever
/// produces. A consumer writing `#[embedded]`/`#[native]` on their own component now just gets it
/// silently ignored, the same as any other unrecognized attribute name, rather than that
/// internal-sounding rejection message.
fn component_item_attrs(
    attrs: &[syn::Attribute],
) -> Result<(bool, bool, bool, Option<String>), String> {
    let mut sealed = false;
    let mut is_abstract = false;
    let mut text_style = false;
    let mut content_field = None;
    for attr in attrs {
        let Some(attr_name) = attr.path().get_ident().map(|i| i.to_string()) else {
            continue;
        };
        match attr_name.as_str() {
            "sealed" => sealed = true,
            "abstract_" => is_abstract = true,
            "text_style" => text_style = true,
            "content" => {
                let field: syn::Ident = attr
                    .parse_args()
                    .map_err(|e| format!("invalid #[content(field_name)] arguments: {e}"))?;
                content_field = Some(field.to_string());
            }
            _ => continue,
        }
    }
    Ok((sealed, is_abstract, text_style, content_field))
}

fn is_presentation_macro_field(field: &syn::Field) -> bool {
    matches!(&field.ty, syn::Type::Macro(tm) if tm.mac.path.is_ident("view") || tm.mac.path.is_ident("template_view"))
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

/// Issue #162 §3.5: builds the synthetic hidden `ComponentDef`/`ViewDef` pair a single
/// `ViewExpr::DeferredView` (`context_popup: view! { .. }`) lowers to — a pure AST construction
/// helper, not a `syn::ItemStruct` frontend, since the deferred body is already fully parsed
/// (`ast::DeferredViewBody`) by the time this runs (`lib.rs`'s lowering pass, called after
/// validation, before `codegen::build_symbol_table`).
///
/// Mirrors `lib.rs::generate_control_template_from_item_struct`'s existing `ControlTemplate`
/// precedent (a `#[param]` weak-owner field plus the authored body, composed over
/// `ContentControl`), but builds the `ComponentDef`/`ViewDef` values directly rather than
/// round-tripping through a synthesized `syn::ItemStruct` and re-parsing its `view!` tokens — there
/// is no token-level `view!` invocation left to re-parse here, only already-structured AST.
///
/// `hidden_name` must already be the deterministic, ordinal-qualified name the caller assigned
/// (`__ElwinduiViewTemplateInstanceFor<Outer>_<ordinal>`); `owner_type_name` is always the
/// *original source* lexical Component's own bare name — the real, DSL-author-visible Component
/// whose `view! { .. }` body this `DeferredView` was written inside — regardless of how many
/// levels of nested `context_popup: view! { .. }` separate it from that Component (PR #165 review
/// remediation, A3: an earlier revision passed the *hidden* Component's own name here for a
/// nested `DeferredView`, changing source lexical-scoping semantics — see `lib.rs`'s
/// `lower_deferred_views_in_expr` for why every level keeps the same `owner_type_name` and only
/// the generated hidden component's own *name* changes per nesting depth). `implicit_owner` is the
/// same source-Component field-readable/writable schema at every nesting depth too (PR #165 final
/// rereview remediation, A2 — `codegen::implicit_owner_schema`, computed once from `owner_type_name`
/// before any lowering happens, threaded through unchanged by every `lower_deferred_views_in_*`
/// call, never recomputed from the hidden Component's own field list).
pub(crate) fn hidden_view_template_component(
    hidden_name: &str,
    owner_type_name: &str,
    implicit_owner: &ast::ImplicitOwnerDef,
    body: &ast::DeferredViewBody,
) -> (ComponentDef, ViewDef) {
    let component_def = ComponentDef {
        name: hidden_name.to_string(),
        base: Some("ContentControl".to_string()),
        base_path: None,
        fields: vec![ast::FieldDef {
            name: "__view_owner".to_string(),
            ty: format!("std::rc::Weak<{owner_type_name}>"),
            kind: FieldKind::Param,
            attrs: Vec::new(),
            initializer: None,
        }],
        methods: Vec::new(),
        embedded: false,
        sealed: false,
        native: false,
        is_abstract: false,
        text_style: false,
        content_field: None,
    };
    let view_def = ViewDef {
        target: hidden_name.to_string(),
        is_template: false,
        template_instance: false,
        on_mount: body.on_mount.clone(),
        on_unmount: body.on_unmount.clone(),
        on_update: body.on_update.clone(),
        lets: body.lets.clone(),
        root: body.root.clone(),
        implicit_owner: Some(implicit_owner.clone()),
    };
    (component_def, view_def)
}

/// Visibility a [`component_public_shape`] accessor should be emitted with — mirrors the exact
/// getter/setter visibility `codegen::generate_component`'s own field-classification loop already
/// decides per `FieldKind` (that function's own `visibility` local, `#[state]`'s bare exception). Not
/// a general Rust visibility type: only the two shapes any own-field accessor here ever actually
/// gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShadowVisibility {
    Public,
    Private,
}

/// Issue #146 / PR #169 review remediation (A2): the source-local constructor/getter/setter surface
/// one `#[elwindui::component]` struct's own fields alone (no ancestor/effective-field resolution,
/// no cross-item lookup) determine — shared by `codegen::generate_component`'s own view-less real
/// generation, the own-field constructor/deferred portion of `codegen::generate_view`'s real
/// `has_view` generation, and every rust-analyzer Component struct shadow
/// (`rust_analyzer_shadow::build_component_struct_shadow`), so all three can never independently
/// drift. See [`component_public_shape`]'s own doc comment for the exact per-`FieldKind` rule.
/// PR #169 review remediation, round 2 (AD-R2-4): the real `new(..)` return type — a view-less
/// Component's own real generator (`codegen::generate_component`) returns a bare `Self`; a `has_view`
/// Component's own real generator (`codegen::generate_view`) always returns `std::rc::Rc<Self>`. The
/// rust-analyzer Component struct shadow consumes this directly rather than deciding independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentConstructorReturn {
    SelfValue,
    RcSelf,
}

pub(crate) struct ComponentPublicShape {
    /// `(field name, declared type)` pairs, in field order — exactly `new(..)`'s own positional
    /// argument list for required `#[param]`/`#[bindable]` fields.
    pub constructor_params: Vec<(String, String)>,
    /// `(field name, declared type)` pairs for `#[param(default = ...)]` fields.  These are
    /// readable after construction but are not public constructor arguments and have no public
    /// runtime setter; generated code applies an authored override through a hidden initialization
    /// setter before mount.
    pub defaulted_params: Vec<(String, String)>,
    /// Whether real `new(..)` returns a bare `Self` or `std::rc::Rc<Self>` — see
    /// [`ComponentConstructorReturn`]'s own doc comment.
    pub constructor_return: ComponentConstructorReturn,
    /// `(field name, getter return type, visibility)` — every own field with a public or private
    /// getter (every kind except `#[attached]`, a viewmodel/store-only kind, or an event schema
    /// field). Props always expose the full declared type, including `Option<T>`.
    pub readable_fields: Vec<(String, String, ShadowVisibility)>,
    /// `(field name, setter parameter type, visibility)` for ordinary mutable Props and private
    /// State fields.  Required/defaulted Params deliberately do not appear here: Params are fixed
    /// after construction.
    pub writable_fields: Vec<(String, String, ShadowVisibility)>,
}

/// Classifies `component`'s own fields into the constructor/getter/setter surface described by
/// [`ComponentPublicShape`] — the single source of truth real generation (both
/// `codegen::generate_component`'s view-less path and `codegen::generate_view`'s own-field
/// construction decision) and every rust-analyzer Component struct shadow (Issue #146) all consult.
///
/// Deliberately **source-local only**: unlike `codegen::resolve_effective_fields`, this never
/// resolves an inherited ancestor's own fields (a rust-analyzer shadow must never guess at another
/// crate's/another same-crate registry entry's constructor shape) and never consults sibling module
/// data. `view` is retained because the return shape distinguishes view-backed `Rc<Self>`
/// construction from view-less `Self` construction; field membership itself is independent of the
/// view tree.
///
/// Per-`FieldKind` rule (mirrors real generation's own per-field decisions exactly):
/// - `#[attached]`, and the viewmodel/store-only `Action`/`Observable`/`AsyncComputed` kinds (never
///   legal on a real `#[elwindui::component]` field; `validate::validate` rejects them there): no
///   constructor param, no accessor at all.
/// - `#[environment(name)]`: never a constructor param; public getter only, no setter.
/// - `#[computed(expr = ..)]`: never a constructor param; public getter only, no setter.
/// - `#[param]`/`#[bindable]` without an initializer: required constructor param, public getter,
///   and no public setter.
/// - `#[param(default = ..)]`: no constructor param; public getter and hidden initialization setter.
/// - ordinary `#[prop]` (whether explicitly annotated or an unannotated Rust field): never a
///   constructor param; public getter and setter using the full declared type.  The frontend gives
///   an omitted initializer an implicit `Default::default()`.
/// - `#[state(default = ..)]`: never a constructor param; **private** getter and setter (never part
///   of the component's external property surface — `ast::FieldKind::State`'s own doc comment).
///
/// A field combination `validate::validate` would reject anyway (e.g. a `#[param]` field carrying an
/// initializer expression) contributes no constructor param and no accessor rather than panicking —
/// a rust-analyzer shadow builder must stay self-contained and never abort a proc-macro-srv process
/// over a shape mistake real validation would have caught first.
pub(crate) fn component_public_shape(
    component: &ComponentDef,
    view: Option<&ViewDef>,
) -> ComponentPublicShape {
    let mut constructor_params = Vec::new();
    let mut defaulted_params = Vec::new();
    let mut readable_fields = Vec::new();
    let mut writable_fields = Vec::new();

    for f in &component.fields {
        if crate::attr_frontend::is_event_schema_field(f) {
            continue;
        }
        match f.kind {
            FieldKind::Attached
            | FieldKind::Action
            | FieldKind::Observable
            | FieldKind::AsyncComputed => {
                continue;
            }
            FieldKind::Environment => {
                if f.initializer.is_some() {
                    continue;
                }
                readable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Public));
            }
            FieldKind::Computed => {
                if !matches!(f.initializer, Some(ast::Initializer::Expr(_))) {
                    continue;
                }
                readable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Public));
            }
            FieldKind::Param if f.initializer.is_none() => {
                constructor_params.push((f.name.clone(), f.ty.clone()));
                readable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Public));
            }
            FieldKind::Param => {
                if !matches!(f.initializer, Some(ast::Initializer::Expr(_))) {
                    continue;
                }
                defaulted_params.push((f.name.clone(), f.ty.clone()));
                readable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Public));
            }
            FieldKind::Prop => {
                if !matches!(f.initializer, None | Some(ast::Initializer::Expr(_))) {
                    continue;
                }
                readable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Public));
                writable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Public));
            }
            FieldKind::State => {
                if !matches!(f.initializer, Some(ast::Initializer::Expr(_))) {
                    continue;
                }
                readable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Private));
                writable_fields.push((f.name.clone(), f.ty.clone(), ShadowVisibility::Private));
            }
        }
    }

    let constructor_return = if view.is_some() {
        ComponentConstructorReturn::RcSelf
    } else {
        ComponentConstructorReturn::SelfValue
    };

    ComponentPublicShape {
        constructor_params,
        defaulted_params,
        constructor_return,
        readable_fields,
        writable_fields,
    }
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
    /// The companion `#[elwindui::component] impl Name { .. }` block's source, if one was expanded
    /// after this struct. Kept as text for the same reason `struct_src` is. Empty until
    /// `register_same_crate_component_methods` runs, which is what lets a *later* component
    /// resolve this one's `#[overridable]` methods when it writes `#[overrides]` against them.
    impl_src: Option<String>,
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
///
/// Issue #146: this registry is the strict source of truth for real `rustc`/`cargo build` only.
/// rust-analyzer's own `struct`/`impl` name resolution never depends on this registry being complete
/// at lookup time — see `rust_analyzer_shadow::build_component_struct_shadow`/
/// `build_component_impl_shadow`, both of which build a self-contained shadow straight from the one
/// `syn::ItemStruct`/`syn::ItemImpl` they're handed, with no lookup into this map at all — and
/// `lib.rs::generate_component_from_item_impl`, which never lets a lookup miss here suppress that
/// shadow (`docs/design/tools/codegen_design.md` §3.2a).
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
        impl_src: None,
    };
    same_crate_components()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), name.to_string()), stored);
}

/// Rebuilds `name`'s `ComponentDef`/`ViewDef` from its registered struct text — the same
/// reconstruction `sibling_component_modules` does, but for one named component rather than every
/// sibling. Returns `None` if no `#[elwindui::component] struct name { .. }` has been expanded in
/// this crate yet, which is exactly the "impl written before its struct" mistake.
pub fn registered_component_parts(name: &str) -> Option<(ComponentDef, Option<ViewDef>)> {
    let store = same_crate_components().lock().unwrap();
    let stored = store.get(&(compiling_crate_key(), name.to_string()))?;
    let item_struct: syn::ItemStruct = syn::parse_str(&stored.struct_src).ok()?;
    component_and_view_from_item_struct(stored.base.clone(), &item_struct).ok()
}

/// Attaches `name`'s companion `#[elwindui::component] impl Name { .. }` to its already-registered
/// struct, so a *later* same-crate component writing `#[overrides]` against one of these methods can
/// see it (`sibling_component_modules` replays both halves). Must run after the `impl` block's own
/// validation succeeds, for the same reason `register_same_crate_component` runs after codegen does.
pub fn register_same_crate_component_methods(name: &str, item_impl: &syn::ItemImpl) {
    let mut store = same_crate_components().lock().unwrap();
    if let Some(stored) = store.get_mut(&(compiling_crate_key(), name.to_string())) {
        stored.impl_src = Some(quote::quote! { #item_impl }.to_string());
    }
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
/// `#[elwindui::component]`'s field type or reactive owner path actually names — see
/// `register_same_crate_viewmodel`'s own doc comment). Populated by
/// `lib.rs::generate_viewmodel_from_item_mod`; read by `sibling_viewmodel_modules` so a
/// `#[bindable]`-using component elsewhere in the same crate can be checked against the
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
/// usually differ, and it's the struct name real Rust code (field types, reactive owner paths) actually
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

/// A registered `#[elwindui::store] mod foo { .. }` — kept as reparseable source text for the same
/// reason `StoredViewModel` is. A **separate** registry from `same_crate_viewmodels`, not a reuse
/// of it — `Item::Store` and `Item::ViewModel` must remain distinct symbol-table entries (a store
/// is a process-wide singleton referenced by a bare type-qualified `TypeName.field` path, not
/// something a component holds via `#[bindable]`; see docs/design/runtime/state_management_design.md
/// "Stores").
struct StoredStore {
    item_mod_src: String,
}

/// Keyed by `(compiling_crate_key(), store type name)` — mirrors `same_crate_viewmodels`, but for
/// `#[elwindui::store] mod foo { struct Foo { .. } }`. Populated by
/// `lib.rs::generate_store_from_item_mod`; read by `sibling_store_modules` so a component/viewmodel
/// elsewhere in the same crate referencing `Foo.field` can be checked against the store's real
/// fields. Same declaration-order requirement as `same_crate_viewmodels` — a store must be declared
/// before anything that references it.
fn same_crate_stores() -> &'static Mutex<HashMap<(String, String), StoredStore>> {
    static REGISTRY: OnceLock<Mutex<HashMap<(String, String), StoredStore>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `name`'s already-successfully-generated `#[elwindui::store] mod item_mod { .. }` — see
/// `same_crate_stores`'s own doc comment. `name` is the store *struct's* name, not the enclosing
/// `mod`'s name. Only call this after this store's own codegen has actually succeeded.
pub fn register_same_crate_store(name: &str, item_mod: &syn::ItemMod) {
    let stored = StoredStore {
        item_mod_src: quote::quote! { #item_mod }.to_string(),
    };
    same_crate_stores()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), name.to_string()), stored);
}

/// Every same-crate `#[elwindui::store]` registered so far, rebuilt as one `Module` each — see
/// `same_crate_stores`'s own doc comment.
pub fn sibling_store_modules() -> Vec<Module> {
    let key = compiling_crate_key();
    let store = same_crate_stores().lock().unwrap();
    store
        .iter()
        .filter(|((crate_key, _), _)| crate_key == &key)
        .map(|(_, stored)| {
            let item_mod: syn::ItemMod = syn::parse_str(&stored.item_mod_src)
                .expect("internal: failed to reparse a registered sibling store's mod text");
            let def = attr_frontend::store_def_from_item_mod(&item_mod)
                .expect("internal: failed to rebuild a registered sibling store");
            Module {
                path: Vec::new(),
                uses: Vec::new(),
                items: vec![ast::Item::Store(def)],
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

/// A registered `#[elwindui::environment_key]` struct — see `same_crate_environment_keys`'s own
/// doc comment.
struct StoredEnvironmentKey {
    key_type_name: String,
    value_type: String,
}

/// Keyed by `(compiling_crate_key(), environment key name)` — mirrors `same_crate_enums`, but for
/// an `#[elwindui::environment_key(name = .., value = .., default = ..)]` declaration
/// (`docs/specs/theme_environment_spec.md` §2). Populated by
/// `environment_frontend::generate_environment_key_from_item_struct`; read by `validate.rs` to
/// resolve `#[environment(name)]`/`EnvironmentScope { name: .. }` (`docs/specs/dsl_spec.md` §13
/// rules 34/35) and by `codegen.rs` to know which concrete Key type to call
/// `EnvironmentContext::get`/`subscribe` with. Same declaration-order requirement as the other
/// registries in this file — an environment key must be declared before the component(s)/
/// `EnvironmentScope` that reference its name.
///
/// Issue #146: strict source of truth for real `rustc`/`cargo build` only — a `#[elwindui::theme]`
/// field's same-crate key lookup miss here (`lookup_writable_environment_key`) never suppresses that
/// Theme's own rust-analyzer shadow (`rust_analyzer_shadow::build_theme_shadow`, which never consults
/// this registry at all — see `theme_frontend.rs`'s own dual-expansion split,
/// `docs/design/tools/codegen_design.md` §3.2a). Environment Key/ViewModel/Store/DSL enum *defining*
/// expansions themselves stay unchanged — they never depend on a sibling registry to define
/// themselves, so they need no shadow of their own (AD-11 of the Issue #146 implementation contract).
fn same_crate_environment_keys() -> &'static Mutex<HashMap<(String, String), StoredEnvironmentKey>>
{
    static REGISTRY: OnceLock<Mutex<HashMap<(String, String), StoredEnvironmentKey>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `name`'s already-emitted `#[elwindui::environment_key]` Key type and its `Value` type
/// — see `same_crate_environment_keys`'s own doc comment. Returns an error if `name` is already
/// registered by a *different* Key type in this crate (two `#[elwindui::environment_key]`
/// declarations must not claim the same DSL-facing name).
pub fn register_same_crate_environment_key(
    name: &str,
    key_type_name: &str,
    value_type: &str,
) -> Result<(), String> {
    let map_key = (compiling_crate_key(), name.to_string());
    let mut registry = same_crate_environment_keys().lock().unwrap();
    if let Some(existing) = registry.get(&map_key) {
        if existing.key_type_name != key_type_name {
            return Err(format!(
                "environment key name `{name}` is already registered by `{}`",
                existing.key_type_name
            ));
        }
        return Ok(());
    }
    registry.insert(
        map_key,
        StoredEnvironmentKey {
            key_type_name: key_type_name.to_string(),
            value_type: value_type.to_string(),
        },
    );
    Ok(())
}

/// Resolves `name` to its registered Key type name and `Value` type, if any
/// `#[elwindui::environment_key]` in this same-crate compilation has claimed it — see
/// `same_crate_environment_keys`'s own doc comment.
pub fn lookup_same_crate_environment_key(name: &str) -> Option<(String, String)> {
    let map_key = (compiling_crate_key(), name.to_string());
    same_crate_environment_keys()
        .lock()
        .unwrap()
        .get(&map_key)
        .map(|stored| (stored.key_type_name.clone(), stored.value_type.clone()))
}

/// Resolves an unqualified Environment Key name **for a read** (`#[environment(name)]` field
/// syntax), preferring a user declaration, then the framework's fixed Semantic Style keys
/// (`theme_environment_spec.md` §7, Issue #97), then the framework's other fixed built-in
/// *read-only* keys (currently just `popup_dismiss`, `theme_environment_spec.md` §2, Issue #161).
///
/// This is the **read** resolver — do not reuse it for a DSL construct that *writes* an Environment
/// value (`EnvironmentScope`, `#[elwindui::theme]`); those must use
/// [`lookup_writable_environment_key`] instead, which omits `popup_dismiss`. `popup_dismiss` is
/// installed by `ContextMenuService::open_custom_popup` into the popup-scoped Environment it
/// derives — a DSL author consumes it via `#[environment(popup_dismiss)]`, but must not be able to
/// overwrite the framework's own active dismiss action through `EnvironmentScope { popup_dismiss:
/// .. }` or a `#[elwindui::theme]` field.
///
/// The returned strings are emitted as Rust types; this is compile-time fallback only, never a
/// runtime string-keyed Environment lookup.
pub fn lookup_environment_key(name: &str) -> Option<(String, String)> {
    lookup_same_crate_environment_key(name)
        .or_else(|| lookup_builtin_semantic_style_key(name))
        .or_else(|| lookup_builtin_popup_dismiss_key(name))
}

/// Resolves an unqualified Environment Key name **for a write** (`EnvironmentScope { name: value }`,
/// `#[elwindui::theme] struct T { name: value }`), preferring a user declaration, then the
/// framework's fixed Semantic Style keys — deliberately *not* falling through to
/// `lookup_builtin_popup_dismiss_key` (see [`lookup_environment_key`]'s own doc comment for why: a
/// DSL write path must not be able to overwrite the framework-installed active
/// `PopupDismissAction`). A same-crate user key literally named `popup_dismiss` still resolves and
/// is still writable — this only excludes the *framework builtin* fallback, exactly mirroring how a
/// user key of any other builtin name already shadows that builtin for both resolvers.
pub fn lookup_writable_environment_key(name: &str) -> Option<(String, String)> {
    lookup_same_crate_environment_key(name).or_else(|| lookup_builtin_semantic_style_key(name))
}

/// The framework's fixed Semantic Style keys (`theme_environment_spec.md` §7, Issue #97) — every
/// one shares the same `Value = BrushStyle`, unlike `lookup_builtin_popup_dismiss_key`'s single
/// differently-typed key, so this stays its own small match rather than growing one shared arm
/// list with mismatched value types.
fn lookup_builtin_semantic_style_key(name: &str) -> Option<(String, String)> {
    let key = match name {
        "primary" => "PrimaryBrushEnvironment",
        "secondary" => "SecondaryBrushEnvironment",
        "tertiary" => "TertiaryBrushEnvironment",
        "foreground" => "ForegroundBrushEnvironment",
        "background" => "BackgroundBrushEnvironment",
        "window_background" => "WindowBackgroundBrushEnvironment",
        "tint" => "TintBrushEnvironment",
        "selection" => "SelectionBrushEnvironment",
        "separator" => "SeparatorBrushEnvironment",
        "placeholder" => "PlaceholderBrushEnvironment",
        "link" => "LinkBrushEnvironment",
        _ => return None,
    };
    Some((
        format!("elwindui::core::theme::{key}"),
        "elwindui::core::theme::BrushStyle".to_string(),
    ))
}

/// `popup_dismiss` — the framework built-in Environment key carrying the active
/// [`elwindui_core::ui::popup::PopupDismissAction`], `None` outside a popup and `Some(..)` inside
/// the popup-scoped Environment `ContextMenuService::open_custom_popup` derives
/// (`docs/design/runtime/popup_context_menu_design.md` §6). Declaring
/// `#[environment(popup_dismiss)] dismiss: Option<PopupDismissAction>` on any Component resolves
/// through this same built-in-key path a Semantic Style field does — no
/// `#[elwindui::environment_key]` declaration needed, since (like the Brush keys) this is a
/// framework-owned key, not a user- or library-declared one.
fn lookup_builtin_popup_dismiss_key(name: &str) -> Option<(String, String)> {
    if name != "popup_dismiss" {
        return None;
    }
    Some((
        "elwindui::core::ui::popup::PopupDismissActionKey".to_string(),
        "Option<elwindui::core::ui::popup::PopupDismissAction>".to_string(),
    ))
}

/// `#[elwindui::dsl_enum] enum Name { A, B, C }` -> `EnumDef { name: "Name", variants: ["A", "B",
/// "C"] }`. Every variant must be a bare unit variant — same restriction the DSL's own `enum`
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

/// Matches an attribute whose path's *last* segment is `name` — recognizes both `#[elwindui::
/// component]` and a bare `#[component]` (as written after `use elwindui::component;`), the same
/// way real Rust attribute-macro resolution would, without actually resolving anything (this is a
/// read-only, no-macro-expansion caller — `elwindui-languageserver`, which has a whole `.rs` file's
/// text but no compiled crate to resolve paths against).
fn attr_path_ends_with(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().segments.last().is_some_and(|s| s.ident == name))
}

/// Finds the `#[..component(inherits Base)]`/`#[..component]` attribute among `attrs` (see
/// `attr_path_ends_with`) and parses its `inherits Base` argument, if any — the
/// `elwindui-languageserver`-side counterpart to `elwindui_macros::parse_inherits_arg`, which
/// instead receives the attribute's own argument tokens directly from the proc-macro system rather
/// than having to find the attribute itself first.
fn inherits_arg_from_component_attrs(attrs: &[syn::Attribute]) -> Result<Option<String>, String> {
    let Some(attr) = attrs.iter().find(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "component")
    }) else {
        return Ok(None);
    };
    let syn::Meta::List(list) = &attr.meta else {
        return Ok(None);
    };
    if list.tokens.is_empty() {
        return Ok(None);
    }
    use syn::parse::Parser;
    (|input: syn::parse::ParseStream| {
        let kw: syn::Ident = input.parse()?;
        if kw != "inherits" {
            return Err(syn::Error::new(kw.span(), "expected `inherits <Base>`"));
        }
        let base: syn::Ident = input.parse()?;
        Ok(Some(base.to_string()))
    })
    .parse2(list.tokens.clone())
    .map_err(|e| e.to_string())
}

/// Builds the `MethodDef` list for a companion `#[elwindui::component] impl Name { .. }` block —
/// the Rust-macro-path counterpart of `parser.rs`'s `parse_method_def`, which reads the same two
/// tags off the DSL text form's `component` body.
///
/// The two tags are spelled `#[overridable]` (declares a method derived components may override)
/// and `#[overrides]` (overrides the base's same-named `#[overridable]`), matching
/// `#[elwindui_macros::class]`'s vocabulary — see `docs/specs/dsl_spec.md` §3. Every `fn` in the
/// block must carry exactly one of them: an untagged `fn` would silently not participate in the
/// override chain, which is worse than rejecting it.
///
/// Deliberately narrow, exactly as `MethodDef` is: `&self` receiver, plain typed parameters, no
/// generics, no `where` clause, no `async`/`unsafe`. Anything beyond that belongs in an ordinary
/// (non-`#[elwindui::component]`) `impl` block, which this macro never touches.
pub fn methods_from_item_impl(
    item_impl: &syn::ItemImpl,
) -> Result<(String, Vec<ast::MethodDef>), String> {
    if let Some((_, path, _)) = &item_impl.trait_ {
        let name = quote::quote!(#path).to_string();
        return Err(format!(
            "expected an inherent `impl Name {{ .. }}`, found a trait impl for `{name}`"
        ));
    }
    if !item_impl.generics.params.is_empty() || item_impl.generics.where_clause.is_some() {
        return Err("an `impl` block for a component cannot be generic".to_string());
    }
    let syn::Type::Path(type_path) = &*item_impl.self_ty else {
        return Err("expected `impl <ComponentName> { .. }`".to_string());
    };
    let Some(name) = type_path.path.get_ident().map(|i| i.to_string()) else {
        return Err("expected a bare component name, not a qualified path".to_string());
    };

    let mut methods = Vec::new();
    for item in &item_impl.items {
        let syn::ImplItem::Fn(f) = item else {
            return Err(format!(
                "{name}: only `fn` items are allowed in a component `impl` block"
            ));
        };
        let is_overridable = attr_path_ends_with(&f.attrs, "overridable");
        let is_overrides = attr_path_ends_with(&f.attrs, "overrides");
        let fn_name = f.sig.ident.to_string();
        // Issue #162 §3.17: `mount_override`/`unmount_override` are framework-reserved
        // implementation hooks (`Window`'s own `#[overridable]` slots, reached only through the
        // ordinary `#[overridable]`/`#[overrides]` class-bridge chain PR #164 restored) — not a
        // second user-facing lifecycle authoring surface alongside `on_mount`/`on_unmount`.
        if matches!(fn_name.as_str(), "mount_override" | "unmount_override") {
            return Err(format!(
                "{name}::{fn_name}: `{fn_name}` is reserved for framework lifecycle integration; \
                 use `{}` instead",
                if fn_name == "mount_override" {
                    "on_mount"
                } else {
                    "on_unmount"
                }
            ));
        }
        match (is_overridable, is_overrides) {
            (false, false) => {
                return Err(format!(
                    "{name}::{fn_name}: a `fn` in a component `impl` block must be tagged \
                     `#[overridable]` or `#[overrides]`"
                ));
            }
            (true, true) => {
                return Err(format!(
                    "{name}::{fn_name}: `#[overridable]` and `#[overrides]` are mutually exclusive"
                ));
            }
            _ => {}
        }
        if f.sig.asyncness.is_some() || f.sig.unsafety.is_some() {
            return Err(format!(
                "{name}::{fn_name}: `async`/`unsafe` are not supported"
            ));
        }
        if !f.sig.generics.params.is_empty() || f.sig.generics.where_clause.is_some() {
            return Err(format!(
                "{name}::{fn_name}: generic methods are not supported"
            ));
        }

        let mut inputs = f.sig.inputs.iter();
        match inputs.next() {
            Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() => {}
            _ => {
                return Err(format!(
                    "{name}::{fn_name}: must take `&self` as its first parameter"
                ));
            }
        }
        let mut params = Vec::new();
        for arg in inputs {
            let syn::FnArg::Typed(pat_type) = arg else {
                return Err(format!(
                    "{name}::{fn_name}: unexpected receiver after `&self`"
                ));
            };
            let syn::Pat::Ident(pat_ident) = &*pat_type.pat else {
                return Err(format!(
                    "{name}::{fn_name}: parameters must be plain identifiers, not patterns"
                ));
            };
            params.push((pat_ident.ident.to_string(), (*pat_type.ty).clone()));
        }
        let return_ty = match &f.sig.output {
            syn::ReturnType::Default => None,
            syn::ReturnType::Type(_, ty) => Some((**ty).clone()),
        };
        methods.push(ast::MethodDef {
            name: fn_name,
            is_virtual: is_overridable,
            is_override: is_overrides,
            params,
            return_ty,
            body: f.block.clone(),
        });
    }
    Ok((name, methods))
}

/// Builds one `Module` per `#[elwindui::component]` struct / `#[elwindui::viewmodel]` mod /
/// `#[elwindui::dsl_enum]` enum found among `file`'s top-level items, in source order — the
/// `elwindui-languageserver` counterpart to the real macro-expansion path
/// (`generate_component_from_item_struct`/`generate_viewmodel_from_item_mod`/
/// `generate_dsl_enum_from_item_enum`), minus the actual code generation and the same-crate
/// registries (a language server sees one file's text, not a real compiled crate — see
/// `attr_path_ends_with`'s own doc comment). Reuses the exact same conversion functions those
/// entry points call (`component_and_view_from_item_struct`/`attr_frontend::
/// viewmodel_def_from_item_mod`/`enum_def_from_item_enum`), so a file that *would* macro-expand
/// cleanly gets the identical `ComponentDef`/`ViewModelDef`/`EnumDef` shapes here. Every returned
/// `Module` sets `allows_external_builtins: true` for the same reason the real entry points do —
/// there is no builtin `Module` to resolve `Window`/`VerticalLayout`/etc. against (see
/// `crate::testdata`'s own doc comment).
///
/// Items that don't parse as a `ComponentDef`/`ViewModelDef`/`EnumDef` (a malformed `view!` body, a
/// non-unit enum variant, ...) make the whole call fail — matching how a real macro invocation
/// would fail to expand that one item, except surfaced as one error for the whole file rather than
/// pinpointed to the offending item (no span info flows through these conversions — see
/// `docs/design/tools/languageserver_design.md` for why this is an accepted precision limit, same
/// as `validate::validate`'s own error messages).
pub fn modules_from_file(file: &syn::File) -> Result<Vec<Module>, String> {
    let mut modules = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Struct(item_struct)
                if attr_path_ends_with(&item_struct.attrs, "component") =>
            {
                let base = inherits_arg_from_component_attrs(&item_struct.attrs)?;
                let (component_def, view_def) =
                    component_and_view_from_item_struct(base, item_struct)?;
                modules.push(Module {
                    path: Vec::new(),
                    uses: Vec::new(),
                    items: component_module_items(component_def, view_def),
                    allows_external_builtins: true,
                    ..Default::default()
                });
            }
            syn::Item::Mod(item_mod) if attr_path_ends_with(&item_mod.attrs, "viewmodel") => {
                let def = attr_frontend::viewmodel_def_from_item_mod(item_mod)?;
                modules.push(Module {
                    path: Vec::new(),
                    uses: Vec::new(),
                    items: vec![ast::Item::ViewModel(def)],
                    allows_external_builtins: true,
                    ..Default::default()
                });
            }
            syn::Item::Enum(item_enum) if attr_path_ends_with(&item_enum.attrs, "dsl_enum") => {
                let def = enum_def_from_item_enum(item_enum)?;
                modules.push(Module {
                    path: Vec::new(),
                    uses: Vec::new(),
                    items: vec![ast::Item::Enum(def)],
                    allows_external_builtins: true,
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
    Ok(modules)
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
            let (mut component_def, view_def) =
                component_and_view_from_item_struct(stored.base.clone(), &item_struct)
                    .expect("internal: failed to rebuild a registered sibling component");
            // Replay the companion `impl` block too, so this sibling's `#[overridable]` methods are
            // visible to whoever is currently writing `#[overrides]` against them.
            if let Some(impl_src) = &stored.impl_src {
                let item_impl: syn::ItemImpl = syn::parse_str(impl_src).expect(
                    "internal: failed to reparse a registered sibling component's impl text",
                );
                let (_, methods) = methods_from_item_impl(&item_impl)
                    .expect("internal: failed to rebuild a registered sibling component's methods");
                component_def.methods = methods;
            }
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

    /// Issue #162 T16: `mount_override`/`unmount_override` are framework-reserved — a user
    /// `#[elwindui::component] impl` method with either name is rejected with a diagnostic
    /// pointing at the real user-facing lifecycle surface (`on_mount`/`on_unmount`).
    #[test]
    fn rejects_user_authored_mount_override_and_unmount_override_methods() {
        let mount_override_impl: syn::ItemImpl = syn::parse_str(
            r#"
            impl SomeWindow {
                #[overrides]
                fn mount_override(&self, environment: elwindui_core::environment::EnvironmentContext) {}
            }
            "#,
        )
        .expect("impl should parse");
        let err = methods_from_item_impl(&mount_override_impl)
            .expect_err("mount_override must be rejected");
        assert!(err.contains("mount_override"), "error: {err}");
        assert!(err.contains("on_mount"), "error: {err}");

        let unmount_override_impl: syn::ItemImpl = syn::parse_str(
            r#"
            impl SomeWindow {
                #[overrides]
                fn unmount_override(&self) {}
            }
            "#,
        )
        .expect("impl should parse");
        let err = methods_from_item_impl(&unmount_override_impl)
            .expect_err("unmount_override must be rejected");
        assert!(err.contains("unmount_override"), "error: {err}");
        assert!(err.contains("on_unmount"), "error: {err}");
    }

    #[test]
    fn popup_dismiss_resolves_for_read_but_not_for_write() {
        let read = lookup_environment_key("popup_dismiss");
        assert!(
            read.is_some(),
            "popup_dismiss must resolve as a readable framework built-in key"
        );
        let (key_type, value_type) = read.unwrap();
        assert!(key_type.contains("PopupDismissActionKey"));
        assert!(value_type.contains("PopupDismissAction"));

        assert!(
            lookup_writable_environment_key("popup_dismiss").is_none(),
            "popup_dismiss must not resolve through the writable resolver — EnvironmentScope/Theme \
             must not be able to overwrite the framework-installed active PopupDismissAction"
        );
    }

    #[test]
    fn semantic_style_builtin_key_resolves_for_both_read_and_write() {
        // Regression: the writable/read split must not accidentally make every builtin read-only —
        // only `popup_dismiss` is excluded from the writable resolver.
        assert!(lookup_environment_key("primary").is_some());
        assert!(
            lookup_writable_environment_key("primary").is_some(),
            "Semantic Style Brush keys must remain writable via EnvironmentScope/Theme"
        );
    }

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
    fn template_and_body_are_distinct_authored_slots() {
        let both: syn::ItemStruct = syn::parse_str(
            r#"
                struct Both {
                    body: view! { TextBlock { text: "body" } },
                    template: template_view! { TextBlock { text: "template" } },
                }
            "#,
        )
        .expect("slot probe should parse");
        let error = component_and_view_from_item_struct(Some("VerticalLayout".to_string()), &both)
            .expect_err("body and template must not coexist");
        assert!(
            error.contains("at most one authored presentation field"),
            "{error}"
        );

        let wrong_template_macro: syn::ItemStruct = syn::parse_str(
            r#"
                struct WrongTemplateMacro {
                    template: view! { TextBlock { text: "wrong" } },
                }
            "#,
        )
        .expect("wrong template probe should parse");
        let error = component_and_view_from_item_struct(
            Some("VerticalLayout".to_string()),
            &wrong_template_macro,
        )
        .expect_err("template pseudo-field must use template_view!");
        assert!(error.contains("template_view!"), "{error}");

        let ordinary_template_field: syn::ItemStruct = syn::parse_str(
            r#"
                struct OrdinaryTemplateField {
                    #[prop]
                    template: String,
                }
            "#,
        )
        .expect("ordinary template probe should parse");
        let error = component_and_view_from_item_struct(None, &ordinary_template_field)
            .expect_err("template is a reserved component pseudo-field");
        assert!(error.contains("reserved"), "{error}");
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

    /// CI-8 of #80 (docs/design/runtime/component_lifecycle_design.md §4g): a host-composition
    /// (`inherits Window`, matching `Counter` above) component must NOT auto-mount from
    /// `on_constructed`, and must instead gain a plain inherent `show`/`hide`/`close` that
    /// mount-checks (via `__mount_environment`) and reaches the auto-forwarded `WindowExt`
    /// implementation through UFCS, not `#[overrides]` (verified not to propagate correctly across
    /// the `trait_only` -> `struct_only` -> ordinary chain — see this codegen's own comment at the
    /// `window_lifecycle_overrides` definition in `codegen.rs`).
    #[test]
    fn host_composition_gets_override_chain_show_hide_close_and_no_auto_mount_on_constructed() {
        let src = r#"
            struct AppWindow {
                body: view! {
                    title: "app"
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
        // Issue #128: `show`/`hide`/`close` are now ordinary `#[overrides]` methods (private —
        // `#[class]` normalizes an `#[overrides]` method's visibility to inherited, regardless of
        // what's written), not `pub fn` inherent shadows.
        assert!(s.contains("# [overrides] fn show"), "{s}");
        assert!(s.contains("# [overrides] fn hide"), "{s}");
        assert!(s.contains("# [overrides] fn close"), "{s}");
        // `unmount`/`__unmount_local` remain plain, framework-internal inherent methods — untouched
        // by the #128 migration (they were never part of `WindowExt`).
        assert!(s.contains("pub fn unmount"), "{s}");
        assert!(
            s.contains("self . unmount ()"),
            "close() must delegate to self.unmount(): {s}"
        );
        assert!(
            s.contains("unmount_subtree"),
            "unmount() must cascade to descendant subtree: {s}"
        );
        assert!(
            s.contains("__mount_environment . get () . is_none ()"),
            "show() must mount-check before mounting: {s}"
        );
        // Issue #128: reaches the backend's own concrete implementation through the real ancestor-
        // forwarding chain (`self.base.show()`), not the old CI-8 UFCS workaround.
        assert!(
            s.contains("self . base . show ()"),
            "show() must reach the backend's own implementation via the ordinary ancestor chain \
             (self.base.show()), not UFCS: {s}"
        );
        assert!(s.contains("self . base . hide ()"), "{s}");
        assert!(s.contains("self . base . close ()"), "{s}");
        assert!(
            !s.contains("as elwindui :: core :: ui :: WindowExt > :: show"),
            "the old CI-8 UFCS shadow for show() must be gone now that #128 restored the normal \
             override chain: {s}"
        );
        // `on_constructed` must NOT unconditionally auto-mount for this host-composition case —
        // the only `self.mount(` call in the whole generated output must be the one inside `show()`
        // above, not a second, unconditional one inside `on_constructed`.
        let mount_call_count = s.matches("self . mount (").count();
        assert_eq!(
            mount_call_count, 1,
            "expected exactly one self.mount(..) call site (inside show()), found {mount_call_count}: {s}"
        );
    }

    #[test]
    fn composed_component_generates_unmount_and_registers_hook() {
        let src = r#"
            struct CustomCard {
                template: template_view! {
                    on_unmount {
                        // teardown hook
                    }
                    VerticalLayout {
                        TextBlock { text: "card" }
                    }
                }
            }
        "#;
        let generated = generate(Some("ContentControl"), src);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let s = generated.to_string();
        assert!(s.contains("pub fn unmount"), "{s}");
        assert!(s.contains("__lifecycle_state : std :: cell :: Cell < elwindui :: core :: ui :: ComponentLifecycleState >"), "{s}");
        assert!(s.contains("add_begin_unmount_hook"), "{s}");
        assert!(s.contains("add_unmount_hook"), "{s}");
        // Template bodies register their lifecycle block on the generated template root; the
        // ordinary view path's legacy `__run_on_unmount` helper is intentionally not emitted for
        // this shared ControlTemplate backend.
        assert!(s.contains("__unmount_local"), "{s}");
        assert!(s.contains("unmount_subtree"), "{s}");
        assert!(
            s.contains("__property_changed_subscriptions . borrow_mut () . clear ()"),
            "{s}"
        );
    }

    #[test]
    fn layout_component_generates_unmount_and_registers_hook() {
        let src = r#"
            struct PlainCard {
                body: view! {
                    on_unmount {
                        // teardown hook
                    }
                    TextBlock { text: "card" }
                }
            }
        "#;
        let generated = generate(Some("VerticalLayout"), src);
        let s = generated.to_string();
        assert!(s.contains("pub fn unmount"), "{s}");
        assert!(s.contains("__lifecycle_state : std :: cell :: Cell < elwindui :: core :: ui :: ComponentLifecycleState >"), "{s}");
        assert!(s.contains("add_begin_unmount_hook"), "{s}");
        assert!(s.contains("add_unmount_hook"), "{s}");
        assert!(s.contains("__run_on_unmount"), "{s}");
        assert!(s.contains("unmount_subtree"), "{s}");
        assert!(
            s.contains("__property_changed_subscriptions . borrow_mut () . clear ()"),
            "{s}"
        );
    }

    /// Issue #68 bug 4: a component's own `dyn UIElement`-typed field, inserted bare (no `key:`)
    /// in child-element position of its own `view!` — mirrors `docs/specs/dsl_spec.md`'s
    /// `ContentControl` example (§3), but built through this struct/`impl`-based frontend, whose
    /// `attr_frontend::type_to_compact_string` used to strip the mandatory space out of `dyn
    /// UIElement`, so `generate_view`'s `lets_map` seeding never recognized `content` as a valid
    /// bare child reference and codegen panicked with "does not refer to an earlier `let`
    /// binding". The equivalent DSL-text form (`parser.rs` slicing raw source) never hit this,
    /// since it never strips that space to begin with.
    #[test]
    fn bare_self_field_resolves_as_child_via_struct_frontend() {
        let src = r#"
            struct Wrapper {
                #[param]
                content: std::rc::Rc<dyn UIElement>,

                body: view! {
                    padding: padding
                    content
                }
            }
        "#;
        let generated = generate(Some("VerticalLayout"), src);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let rendered = generated.to_string();
        assert!(
            !rendered.contains("component_body_presentation"),
            "template presentation must not depend on hidden body metadata: {rendered}"
        );
    }

    /// Issue #68 bug 5: `format!("{field}!")`'s inline capture (RFC 2795) only sees whatever raw
    /// local happens to be in scope at the exact point the call gets embedded — for a component's
    /// own field, the generated code used to compile only by accident, relying on a local that a
    /// *second* element's own construction had usually already consumed by the time it got there.
    /// One element referencing the field this way always worked; two or more broke.
    #[test]
    fn format_inline_capture_compiles_across_multiple_elements() {
        let src = r#"
            struct VolumeControl {
                #[prop(default = 50.0)]
                volume: f32,

                body: view! {
                    TextBlock { text: format!("{volume}%") }
                    TextBlock { text: format!("Level: {volume}%") }
                }
            }
        "#;
        let generated = generate(Some("VerticalLayout"), src);
        syn::parse2::<syn::File>(generated.clone())
            .unwrap_or_else(|e| panic!("generated code is not valid Rust: {e}\n---\n{generated}"));
        let s = generated.to_string();
        assert!(
            s.matches("volume =").count() >= 2,
            "expected an explicit named `volume = ..` format! argument at each of the two \
             call sites in generated code:\n{s}"
        );
    }

    // Phase 2: a `view!`-less component is legal (the Rust-macro-path counterpart of a DSL
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
        assert!(
            err.contains("at most one"),
            "error should mention the cardinality: {err}"
        );
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
        assert!(
            component_def.sealed,
            "#[sealed] should set ComponentDef::sealed"
        );
        assert!(
            component_def.is_abstract,
            "#[abstract_] should set ComponentDef::is_abstract"
        );
        assert!(
            !component_def.embedded,
            "this frontend never sets ComponentDef::embedded (test-only fixture concept)"
        );
        assert!(
            !component_def.native,
            "this frontend never sets ComponentDef::native (test-only fixture concept)"
        );
        assert!(
            !component_def.text_style,
            "#[text_style] was not written, should stay false"
        );
        assert_eq!(
            component_def.content_field.as_deref(),
            Some("children"),
            "#[content(children)] should set ComponentDef::content_field"
        );
        assert!(
            view_def.is_none(),
            "no `view! {{ .. }}` field, so the view should be None"
        );
    }

    // Phase 2: `#[param(default = ...)]`, mirroring `#[prop(default = ...)]`'s existing
    // token-based routing through `parser::parse_initializer` (so initializer expressions parse the
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
/// resolution". See docs/specs/dsl_spec.md §3's `VolumeControl` example, which this mirrors.
#[cfg(test)]
mod doc_example_own_default_and_computed_fields {
    use crate::codegen::{build_symbol_table, generate_module};

    /// The minimal case: a `#[prop(default = ...)]` field referenced bare in its own view, no
    /// `#[computed]`, no `inherits`, no dynamic (`match`/`if`) child region.
    #[test]
    fn own_default_prop_referenced_bare_in_own_view() {
        let src = r#"
            struct Greeter {
                #[prop(default = "hi".to_string())]
                title: String,

                body: view! {
                    TextBlock { text: title }
                },
            }
        "#;
        let generated = generate_and_check(Some("VerticalLayout"), src);
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
            struct Greeter {
                #[prop(default = 50)]
                volume: i32,

                #[computed(expr = volume.to_string() + "%")]
                label: String,

                body: view! {
                    TextBlock { text: label }
                },
            }
        "#;
        let generated = generate_and_check(Some("VerticalLayout"), src);
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

    /// The exact `docs/specs/dsl_spec.md` §3 example: `VolumeControl`
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
        let item_enum: syn::ItemEnum = syn::parse_str(deps_src).expect("enum should parse");
        let enum_def = crate::component_frontend::enum_def_from_item_enum(&item_enum)
            .expect("enum should build");
        let deps_module = crate::ast::Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: vec![crate::ast::Item::Enum(enum_def)],
            is_builtin: false,
            allows_external_builtins: false,
        };

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
            struct Settings {
                #[prop(default = 50)]
                volume: i32,

                #[computed(expr = volume.to_string() + "%")]
                label: String,
            }
        "#;
        let module = crate::test_module(&[(None, src, None)]).expect("should build");
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

    fn generate_and_check(base: Option<&str>, struct_src: &str) -> String {
        let module = crate::test_module(&[(base, struct_src, None)]).expect("should build");
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

/// `modules_from_file` — the `elwindui-languageserver` counterpart to real macro expansion,
/// finding every `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` item in
/// a whole `.rs` file's worth of `syn::File` without actually expanding any of them.
#[cfg(test)]
mod modules_from_file_tests {
    use super::*;

    #[test]
    fn finds_component_viewmodel_and_dsl_enum_in_one_file() {
        let src = r#"
            #[elwindui::dsl_enum]
            enum StatusC { Loading, Ready }

            #[elwindui::viewmodel]
            mod vm_mod_c {
                struct VmC {
                    #[observable(default = String::new())]
                    content: String,
                }
            }

            #[elwindui::component(inherits Window)]
            struct ScreenC {
                #[param]
                #[inject]
                vm: VmC,
                #[prop]
                status: StatusC,
                body: view! {
                    VerticalLayout {
                        match status {
                            StatusC::Loading => TextBlock { text: "loading" },
                            StatusC::Ready => TextBlock { text: vm.content },
                        }
                    }
                },
            }

            fn main() {}
        "#;
        let file: syn::File = syn::parse_str(src).expect("should parse as a real Rust file");
        let modules = modules_from_file(&file).expect("should build Modules");
        assert_eq!(modules.len(), 3);

        let has_enum = modules.iter().any(|m| {
            m.items
                .iter()
                .any(|i| matches!(i, ast::Item::Enum(e) if e.name == "StatusC"))
        });
        let has_vm = modules.iter().any(|m| {
            m.items
                .iter()
                .any(|i| matches!(i, ast::Item::ViewModel(v) if v.name == "VmC"))
        });
        let has_component = modules.iter().any(|m| {
            m.items
                .iter()
                .any(|i| matches!(i, ast::Item::Component(c) if c.name == "ScreenC"))
        });
        assert!(has_enum && has_vm && has_component, "modules: {modules:?}");

        crate::validate::validate(&modules).expect("should validate cleanly");
    }

    #[test]
    fn plain_rust_items_are_ignored() {
        let src = r#"
            struct NotDsl { x: i32 }
            fn helper() {}
        "#;
        let file: syn::File = syn::parse_str(src).unwrap();
        let modules = modules_from_file(&file).expect("should succeed with zero DSL items");
        assert!(modules.is_empty());
    }

    /// PR #169 review remediation, round 3, T-R3-4 (A2/AD-R3-2): `component_public_shape` must
    /// receive the literal *source* `ComponentDef` a `#[elwindui::component] struct { .. }` actually
    /// declares — never an ancestor-inclusive, `effective_fields`-flattened one — so it never treats
    /// an inherited field as though this Component declared it itself.
    /// `component_and_view_from_item_struct` (the real struct-parsing entry point every real
    /// `#[elwindui::component]` struct half goes through) already only ever returns a component's own
    /// literal fields — it has no ancestor/registry awareness at all — so calling
    /// `component_public_shape` directly on its output is exactly the source-local input `codegen.rs`'s
    /// `generate_component`/`generate_view` must now also pass (this test's own `source_component`
    /// role at that call site).
    #[test]
    fn t_r3_4_component_public_shape_is_source_local_not_effective_fields_flattened() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR34Derived {
                #[param]
                own_value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_and_view_from_item_struct(Some("TR34Base".to_string()), &item_struct)
                .expect("should build");

        // The parser never resolves/flattens the ancestor's own fields into `component_def.fields` —
        // confirming `component_def` here really is source-only, exactly like `source_component`
        // must be at the real `generate_component`/`generate_view` call site.
        assert_eq!(
            component_def
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["own_value"],
            "the struct parser itself must never synthesize an ancestor field"
        );

        let shape = component_public_shape(&component_def, view_def.as_ref());
        assert!(
            shape
                .constructor_params
                .iter()
                .any(|(name, _)| name == "own_value"),
            "shape should contain the component's own field: {:?}",
            shape.constructor_params
        );
        assert!(
            !shape
                .constructor_params
                .iter()
                .any(|(name, _)| name == "base_value")
                && !shape
                    .readable_fields
                    .iter()
                    .any(|(name, _, _)| name == "base_value"),
            "shape must never contain a field the component itself never declared: {:?} / {:?}",
            shape.constructor_params,
            shape.readable_fields
        );
    }
}
