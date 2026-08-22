//! Issue #146: rust-analyzer dual expansion for `#[elwindui::component]`/`#[elwindui::theme]`.
//!
//! `component_frontend.rs`/`theme_frontend.rs`'s same-crate registries (`same_crate_components`,
//! `same_crate_environment_keys`) assume ordinary `cargo build`'s "one process per crate, expanded in
//! source order" contract — see those modules' own doc comments. rust-analyzer instead runs one
//! persistent `proc-macro-srv` per workspace and expands macros on demand, so a registry lookup that
//! is guaranteed to succeed under real `rustc` can fail spuriously there even when the source is
//! correctly ordered, producing the exact "no `#[elwindui::component] struct Name { .. }` was
//! expanded before this `impl` block" / "no `#[elwindui::environment_key]` was declared earlier in
//! this crate" ghost diagnostics Issue #146 tracks.
//!
//! `crates/elwindui-macros/src/class.rs` already solved this once, for `#[elwindui_macros::class]` —
//! this module ports the same *dual expansion* model to Component/Theme:
//!
//! ```text
//! normal rustc/cargo
//!     -> the existing strict registry-backed expansion (component_frontend.rs/theme_frontend.rs,
//!        unchanged)
//!     -> #[cfg(not(rust_analyzer))] real generated items
//!
//! rust-analyzer
//!     -> source-local/self-contained shadow expansion (this module)
//!     -> #[cfg(rust_analyzer)] shadow items
//! ```
//!
//! Two ground rules, both load-bearing (`docs/design/tools/codegen_design.md` §3.2a):
//!
//! - **Normal rustc strict semantics never change.** A genuinely missing same-crate Component
//!   struct/Environment Key stays a real `cargo build`/`cargo check` error — every shadow builder
//!   here is *additive*, gated to only ever appear under `cfg(rust_analyzer)`, and every real
//!   registry-dependent diagnostic this module's callers preserve stays gated to
//!   `cfg(not(rust_analyzer))` rather than being deleted or downgraded.
//! - **A shadow is source-local.** It never scans the filesystem, never detects rust-analyzer from
//!   inside the proc-macro process itself (`cfg(rust_analyzer)` is the *only* signal — applied to
//!   generated Rust items, never read back at macro-execution time), and never guesses at another
//!   same-crate registry entry's shape when that entry can't be found. `build_component_struct_shadow`
//!   in particular only ever consults the one `syn::ItemStruct`/`ast::ComponentDef` it was handed —
//!   see `component_frontend::component_public_shape`'s own doc comment for the exact
//!   source-local-only field classification both it and real (view-less) generation share.
//!
//! A shadow's own body is never a runtime reimplementation — every method here is `unreachable!()`.
//! It exists purely to give rust-analyzer's name/type resolution a self-contained, always-succeeding
//! surface; real behavior is exclusively what `cfg(not(rust_analyzer))` generation already produces.

use crate::ast::{self, ComponentDef};
// `ComponentPublicShape` is not named directly in this file (only `component_public_shape`'s return
// value, used structurally) but is re-exported as part of this module's own documented surface
// (`docs/design/tools/codegen_design.md` §3.2a) alongside the function/enum this file does use.
#[allow(unused_imports)]
pub(crate) use crate::component_frontend::{
    ComponentConstructorReturn, ComponentPublicShape, ShadowVisibility, component_public_shape,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Parses `tokens` as a sequence of top-level Rust items and appends `#[cfg(not(rust_analyzer))]` to
/// every one of them, in place — the real-generation half of this module's dual expansion model
/// (`docs/design/tools/codegen_design.md` §3.2a). Every `#[elwindui::component]`/`#[elwindui::theme]`
/// real generated item is routed through this exactly once, so rust-analyzer never sees both the real
/// output and this module's shadow for the same declaration at once (which would be a duplicate
/// definition, not a diagnostic improvement).
///
/// Deliberately item-level (`syn::Item`'s own `attrs` field), never module-wrapping: moving generated
/// items into a synthetic `mod { .. }` would change their effective path, breaking any other generated
/// code that names them by their current (crate-root-flat) path — see `component_frontend.rs`'s own
/// doc comment on why every `#[elwindui::component]`-generated type lives at a flat crate-root path
/// today. A macro-generated helper item, `impl`, `trait`, or `macro_rules!` item all keep their
/// original module scope; only a `#[cfg(not(rust_analyzer))]` attribute is added to each.
///
/// Fails explicitly (rather than silently returning `tokens` unchanged) if `tokens` doesn't parse as
/// a plain sequence of top-level items — that would mean a caller handed this something other than
/// what `elwindui-codegen`'s own generators produce, an internal invariant violation worth surfacing
/// loudly rather than masking.
pub(crate) fn gate_real_items_for_rustc(tokens: TokenStream) -> Result<TokenStream, String> {
    gate_top_level_items(tokens, quote! { not(rust_analyzer) }).map_err(|error| {
        format!(
            "internal: rust_analyzer_shadow::gate_real_items_for_rustc: failed to parse generated \
             tokens as a sequence of top-level items: {error}"
        )
    })
}

/// The shadow-side counterpart to `gate_real_items_for_rustc`: parses `tokens` as a sequence of
/// top-level items and appends `#[cfg(rust_analyzer)]` to each one **individually**, in place —
/// deliberately *not* a single enclosing `const _: () = { .. };` block. An earlier revision of every
/// shadow builder in this module wrapped its own struct/impl items in exactly such a block for
/// convenience; that broke rust-analyzer's own name resolution outright, because an item declared
/// inside an anonymous `const _: () = { .. };` is scoped to that block and invisible to the rest of
/// the file — under `RUSTFLAGS="--cfg rust_analyzer"` (`docs/design/tools/codegen_design.md` §3.2a's
/// own deterministic stand-in for real rust-analyzer expansion), every `Theme`/Component shadow name
/// (`DefaultTheme`, `ThemeDemoWindow`, ...) came back "not found in this scope" — exactly the
/// regression T10 exists to catch. Every item stays gated but otherwise ordinary and directly
/// name-resolvable at its original module scope, mirroring `gate_real_items_for_rustc`'s own item-level
/// (never module-wrapping) approach for the real side.
fn gate_shadow_items(tokens: TokenStream) -> Result<TokenStream, String> {
    gate_top_level_items(tokens, quote! { rust_analyzer }).map_err(|error| {
        format!(
            "internal: rust_analyzer_shadow::gate_shadow_items: failed to parse shadow tokens as a \
             sequence of top-level items: {error}"
        )
    })
}

fn gate_top_level_items(
    tokens: TokenStream,
    cfg_predicate: TokenStream,
) -> Result<TokenStream, String> {
    let file: syn::File = syn::parse2(tokens).map_err(|error| error.to_string())?;
    let mut out = TokenStream::new();
    for mut item in file.items {
        push_cfg(&mut item, &cfg_predicate);
        out.extend(quote! { #item });
    }
    Ok(out)
}

/// Appends `#[cfg(#cfg_predicate)] #[allow(unexpected_cfgs)]` to whichever top-level item variant
/// `item` is. `#[allow(unexpected_cfgs)]` mirrors every `#[cfg(rust_analyzer)]`/`#[cfg(not(rust_analyzer))]`
/// emission `class.rs` already does (its `deref_shadow`/`build_rust_analyzer_shadow` call sites) —
/// belt-and-suspenders alongside the workspace-level `check-cfg` registration (root `Cargo.toml`) for
/// any generated code that ends up compiled outside this workspace's own lint configuration.
/// `syn::Item` is `#[non_exhaustive]` and a few variants (e.g. `Verbatim`) carry no `attrs` field at
/// all — left untouched; `elwindui-codegen`'s own generators never actually produce one of those at
/// the top level.
fn push_cfg(item: &mut syn::Item, cfg_predicate: &TokenStream) {
    let cfg_attrs: [syn::Attribute; 2] = [
        syn::parse_quote!(#[cfg(#cfg_predicate)]),
        syn::parse_quote!(#[allow(unexpected_cfgs)]),
    ];
    let attrs = match item {
        syn::Item::Const(i) => &mut i.attrs,
        syn::Item::Enum(i) => &mut i.attrs,
        syn::Item::ExternCrate(i) => &mut i.attrs,
        syn::Item::Fn(i) => &mut i.attrs,
        syn::Item::ForeignMod(i) => &mut i.attrs,
        syn::Item::Impl(i) => &mut i.attrs,
        syn::Item::Macro(i) => &mut i.attrs,
        syn::Item::Mod(i) => &mut i.attrs,
        syn::Item::Static(i) => &mut i.attrs,
        syn::Item::Struct(i) => &mut i.attrs,
        syn::Item::Trait(i) => &mut i.attrs,
        syn::Item::TraitAlias(i) => &mut i.attrs,
        syn::Item::Type(i) => &mut i.attrs,
        syn::Item::Union(i) => &mut i.attrs,
        syn::Item::Use(i) => &mut i.attrs,
        _ => return,
    };
    attrs.extend(cfg_attrs);
}

fn parse_type(ty: &str) -> Result<syn::Type, String> {
    syn::parse_str(ty).map_err(|error| {
        format!("internal: rust-analyzer shadow: type `{ty}` failed to parse: {error}")
    })
}

/// The rust-analyzer-only Component struct shadow (Issue #146, `docs/design/tools/codegen_design.md`
/// §3.2a) — built entirely from `item_struct`/`component`'s own source, independent of the same-crate
/// Component registry (`component_frontend::same_crate_components`) real generation depends on, so it
/// resolves identically regardless of rust-analyzer's own macro expansion order.
///
/// `base` is `component.base.as_deref()` — the bare ancestor name (a builtin or, when
/// `component.base_path` is set, a same-crate user component), when this component declares one.
/// Emits a `Deref<Target = <ancestor>>` for it (matching `class.rs`'s own `deref_shadow`) so
/// rust-analyzer's autoderef-based method resolution can still walk to the ancestor's own methods —
/// `codegen::immediate_base_qualified_path` is reused unchanged for a same-crate user ancestor's own
/// fully-qualified path (Refs #25); a bare builtin name resolves through the facade re-export path
/// every other builtin-referencing generated code already uses (`codegen::composed_construct_path`'s
/// own `elwindui::ui::#ident` convention).
///
/// Every method body is `unreachable!()` — this type is never actually constructed; see this module's
/// own doc comment for why. `new`'s own parameter list and every accessor's own name/type/visibility
/// come from [`component_public_shape`], shared with `codegen::generate_component`'s real (view-less)
/// generation and `codegen::generate_view`'s own real `has_view` generation. `view` is `component`'s
/// own `ViewDef` when it has one — `None` for a view-less component — and drives the same
/// referenced-vs-unreferenced own `Option<T>` deferral decision real generation makes (PR #169
/// review, AD-R3/AD-R4).
pub(crate) fn build_component_struct_shadow(
    base: Option<&str>,
    item_struct: &syn::ItemStruct,
    component: &ComponentDef,
    view: Option<&ast::ViewDef>,
) -> Result<TokenStream, String> {
    let vis = &item_struct.vis;
    let ident = &item_struct.ident;
    let shape = component_public_shape(component, view);

    let mut ctor_params = TokenStream::new();
    for (name, ty) in &shape.constructor_params {
        let field_ident = format_ident!("{name}");
        let ty = parse_type(ty)?;
        ctor_params.extend(quote! { #field_ident: #ty, });
    }

    // PR #169 review remediation, round 2 (AD-R2-7): the real constructor return type — bare
    // `Self` for a view-less Component (`codegen::generate_component`), `std::rc::Rc<Self>` for a
    // `has_view` one (`codegen::generate_view`) — is read from `shape.constructor_return`, not
    // decided independently here.
    let mut methods = TokenStream::new();
    methods.extend(match shape.constructor_return {
        ComponentConstructorReturn::SelfValue => quote! {
            pub fn new(#ctor_params) -> Self {
                unreachable!()
            }
        },
        ComponentConstructorReturn::RcSelf => quote! {
            pub fn new(#ctor_params) -> std::rc::Rc<Self> {
                unreachable!()
            }
        },
    });
    for (name, ty, visibility) in &shape.readable_fields {
        let field_ident = format_ident!("{name}");
        let ty = parse_type(ty)?;
        let vis_tokens = shadow_vis_tokens(*visibility);
        methods.extend(quote! {
            #vis_tokens fn #field_ident(&self) -> #ty { unreachable!() }
        });
    }
    for (name, ty, visibility) in &shape.writable_fields {
        let set_ident = format_ident!("set_{name}");
        let ty = parse_type(ty)?;
        let vis_tokens = shadow_vis_tokens(*visibility);
        methods.extend(quote! {
            #vis_tokens fn #set_ident(&self, value: #ty) { unreachable!() }
        });
    }

    let deref_shadow = match base {
        Some(base_name) => {
            let target = base_type_path(component, base_name);
            Some(quote! {
                impl std::ops::Deref for #ident {
                    type Target = #target;
                    fn deref(&self) -> &Self::Target { unreachable!() }
                }
            })
        }
        None => None,
    };

    gate_shadow_items(quote! {
        #vis struct #ident;

        impl #ident {
            #methods
        }

        #deref_shadow
    })
}

fn shadow_vis_tokens(visibility: ShadowVisibility) -> TokenStream {
    match visibility {
        ShadowVisibility::Public => quote! { pub },
        ShadowVisibility::Private => quote! {},
    }
}

/// `base_name`'s own fully-qualified type path for a `Deref` shadow target — reuses
/// `codegen::immediate_base_qualified_path` unchanged for a same-crate user ancestor written as a
/// qualified path (Refs #25); falls back to the facade re-export path (`elwindui::ui::#ident`) every
/// other generated reference to a bare builtin name already uses
/// (`codegen::composed_construct_path`'s own convention) otherwise.
fn base_type_path(component: &ComponentDef, base_name: &str) -> TokenStream {
    if let Some(path) = crate::codegen::immediate_base_qualified_path(component, base_name) {
        return path;
    }
    let ident = format_ident!("{base_name}");
    quote! { elwindui::ui::#ident }
}

/// The rust-analyzer-only Component impl shadow (Issue #146) — a plain inherent `impl Name { .. }`
/// exposing every `#[overridable]`/`#[overrides]` method in `item_impl` as a `pub fn`, regardless of
/// whether this component's paired `struct Name { .. }` has been expanded yet in this rust-analyzer
/// session (`component_frontend::registered_component_parts`'s own same-crate registry dependency,
/// which `lib.rs::generate_component_from_item_impl` never lets gate this shadow's own generation —
/// see that function's own doc comment).
///
/// Self-contained: reparses `item_impl` via `component_frontend::methods_from_item_impl` itself,
/// exactly like `class.rs`'s own `build_rust_analyzer_shadow` reclassifies its `impl`'s own
/// `item.items`. An item-local error here (a malformed method signature, an untagged `fn`, ...) is a
/// genuine mistake real generation would also reject — propagated as `Err`, not swallowed, so it still
/// surfaces as an ordinary (unconditional) diagnostic under rust-analyzer.
///
/// An `#[overrides]` method is included alongside `#[overridable]` ones, unconditionally `pub` here —
/// mirroring `class.rs`'s own `build_rust_analyzer_shadow` doc comment: a class's own override is
/// exactly the method body that wins at this class's own concrete type, matching real dispatch's own
/// "closest override" semantics, and the real build routes both through a `pub` surface (the
/// generated type's own inherent methods) regardless of which tag declared them.
pub(crate) fn build_component_impl_shadow(
    item_impl: &syn::ItemImpl,
) -> Result<TokenStream, String> {
    let (name, methods) = crate::component_frontend::methods_from_item_impl(item_impl)?;
    let ident = format_ident!("{name}");
    let mut out = TokenStream::new();
    for m in &methods {
        let method_ident = format_ident!("{}", m.name);
        let mut params = TokenStream::new();
        for (param_name, ty) in &m.params {
            let param_ident = format_ident!("{param_name}");
            params.extend(quote! { #param_ident: #ty, });
        }
        let ret = match &m.return_ty {
            Some(ty) => quote! { -> #ty },
            None => quote! {},
        };
        out.extend(quote! {
            pub fn #method_ident(&self, #params) #ret { unreachable!() }
        });
    }
    gate_shadow_items(quote! {
        impl #ident {
            #out
        }
    })
}

/// The rust-analyzer-only Theme shadow (Issue #146) — a no-op marker type implementing
/// `elwindui::core::theme::Theme`, entirely independent of the same-crate Environment Key registry
/// real `#[elwindui::theme]` generation resolves each field's `#[theme(value = ..)]` writable key
/// against (`component_frontend::lookup_writable_environment_key`). rust-analyzer's own role here is
/// only to resolve the marker type and confirm it implements `Theme` (for `.apply(..)` completion) —
/// it never needs a per-Environment-Key `set::<K>()` body, so this never reproduces
/// `theme_frontend::generate_theme_from_item_struct`'s own field loop at all.
///
/// `item_struct`'s own field attributes (`#[theme(value = ..)]` syntax, duplicate field names, ...)
/// are never re-validated here — a malformed one is an item-local error `theme_frontend.rs` itself
/// already rejects before this is ever reached (see its own dual-expansion split), so by the time this
/// runs, only the still-registry-dependent step (writable-key resolution) can be outstanding.
pub(crate) fn build_theme_shadow(item_struct: &syn::ItemStruct) -> Result<TokenStream, String> {
    let vis = &item_struct.vis;
    let ident = &item_struct.ident;
    gate_shadow_items(quote! {
        #vis struct #ident;

        impl elwindui::core::theme::Theme for #ident {
            fn apply(&self, _environment: &elwindui::core::environment::EnvironmentContext) {}
        }
    })
}

/// PR #169 review remediation (A3/AD-R6): the rust-analyzer-only shadow for
/// `#[elwindui::control_template(target = ..)]`'s own **public** declaration — `TemplateName` and
/// `TemplateName::template() -> ControlTemplate<Target>`, signature only. Deliberately does not
/// reach for the generic Component shadow's own machinery: real `template()`'s body constructs and
/// mounts a private hidden Component instance via real runtime-only methods
/// (`__new_unmounted`/`mount`/`into_node`) the generic Component shadow never fakes (see
/// `build_component_impl_shadow`'s own doc comment and AD-R6 of the Issue #146/PR #169 contract:
/// "do not add runtime-only APIs to generic Component shadows just to make `#[control_template]`
/// compile") — so this shadow's own `template()` body is `unreachable!()`, exactly like every other
/// shadow method in this module, rather than attempting to replicate that construction.
pub(crate) fn build_control_template_shadow(
    item_struct: &syn::ItemStruct,
    target: &syn::Path,
) -> Result<TokenStream, String> {
    let vis = &item_struct.vis;
    let ident = &item_struct.ident;
    gate_shadow_items(quote! {
        #vis struct #ident;

        impl #ident {
            pub fn template() -> elwindui::core::ui::ControlTemplate<#target> {
                unreachable!()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, FieldDef, FieldKind, Initializer};

    fn parses_as_items(tokens: &TokenStream) {
        syn::parse2::<syn::File>(tokens.clone()).unwrap_or_else(|e| {
            panic!("shadow output is not valid Rust items: {e}\n---\n{tokens}")
        });
    }

    /// T1: every top-level item variant this crate's own generators can produce gets
    /// `#[cfg(not(rust_analyzer))]`, item names/scope stay unchanged, and the output re-parses as a
    /// `syn::File`.
    #[test]
    fn gate_real_items_for_rustc_gates_every_top_level_item_kind() {
        let input = quote! {
            pub struct DemoStruct { pub field: i32 }
            pub enum DemoEnum { A, B }
            pub trait DemoTrait { fn method(&self); }
            impl DemoTrait for DemoStruct { fn method(&self) {} }
            macro_rules! demo_macro { () => {}; }
            pub fn demo_fn() {}
        };
        let gated = gate_real_items_for_rustc(input).expect("should gate successfully");
        parses_as_items(&gated);
        let s = gated.to_string();
        assert!(s.contains("struct DemoStruct"), "{s}");
        assert!(s.contains("enum DemoEnum"), "{s}");
        assert!(s.contains("trait DemoTrait"), "{s}");
        assert!(s.contains("impl DemoTrait for DemoStruct"), "{s}");
        assert!(s.contains("macro_rules ! demo_macro"), "{s}");
        assert!(s.contains("fn demo_fn"), "{s}");
        let gate_count = s.matches("cfg (not (rust_analyzer))").count();
        assert_eq!(
            gate_count, 6,
            "every one of the 6 top-level items should be gated exactly once: {s}"
        );
    }

    #[test]
    fn gate_real_items_for_rustc_rejects_unparseable_input() {
        let input = quote! { this is not valid rust syntax at all &&& };
        let err =
            gate_real_items_for_rustc(input).expect_err("malformed input must fail explicitly");
        assert!(err.contains("internal"), "error: {err}");
    }

    /// A minimal, otherwise-empty `ViewDef` — used only to give `component_public_shape` a
    /// `Some(view)` to consult (real `has_view` components always have one), without needing a
    /// real element tree for tests that don't exercise the referenced-vs-unreferenced own
    /// `Option<T>` distinction.
    fn empty_view(target: &str) -> ast::ViewDef {
        ast::ViewDef {
            target: target.to_string(),
            on_mount: None,
            on_unmount: None,
            on_update: None,
            lets: Vec::new(),
            root: ast::ViewBody {
                attributes: Vec::new(),
                attached: Vec::new(),
                attribute_shortcuts: Vec::new(),
                children: Vec::new(),
            },
            implicit_owner: None,
        }
    }

    fn demo_component(base: Option<&str>) -> ComponentDef {
        ComponentDef {
            name: "ShadowDemo".to_string(),
            base: base.map(str::to_string),
            base_path: None,
            fields: vec![
                FieldDef {
                    name: "vm".to_string(),
                    ty: "std::rc::Rc<ShadowDemoViewModel>".to_string(),
                    kind: FieldKind::Param,
                    attrs: vec![ast::Attr::Bindable, ast::Attr::Inject],
                    initializer: None,
                },
                FieldDef {
                    name: "layout_spacing".to_string(),
                    ty: "f32".to_string(),
                    kind: FieldKind::Environment,
                    attrs: vec![ast::Attr::Environment("layout_spacing".to_string(), None)],
                    initializer: None,
                },
            ],
            methods: Vec::new(),
            embedded: false,
            sealed: false,
            native: false,
            is_abstract: false,
            text_style: false,
            content_field: None,
        }
    }

    /// T2: the struct shadow exposes a `new(vm)` constructor and a `layout_spacing()` getter, with a
    /// `Deref<Target = elwindui::ui::Window>` for the bare builtin ancestor — no real mount/render
    /// implementation.
    #[test]
    fn struct_shadow_exposes_constructor_and_field_accessors() {
        let component = demo_component(Some("Window"));
        let item_struct: syn::ItemStruct = syn::parse_quote! {
            pub struct ShadowDemo {
                #[bindable]
                vm: std::rc::Rc<ShadowDemoViewModel>,
                #[environment(layout_spacing)]
                layout_spacing: f32,
            }
        };
        let view = empty_view("ShadowDemo");
        let shadow =
            build_component_struct_shadow(Some("Window"), &item_struct, &component, Some(&view))
                .expect("struct shadow should build");
        parses_as_items(&shadow);
        let s = shadow.to_string();
        assert!(s.contains("cfg (rust_analyzer)"), "{s}");
        assert!(s.contains("struct ShadowDemo"), "{s}");
        assert!(
            s.contains("fn new (vm : std :: rc :: Rc < ShadowDemoViewModel > ,)"),
            "{s}"
        );
        assert!(s.contains("std :: rc :: Rc < Self >"), "{s}");
        assert!(s.contains("fn layout_spacing (& self) -> f32"), "{s}");
        assert!(s.contains("impl std :: ops :: Deref for ShadowDemo"), "{s}");
        assert!(s.contains("elwindui :: ui :: Window"), "{s}");
        assert!(
            !s.contains("__view_owner"),
            "shadow must not fake runtime state: {s}"
        );
    }

    #[test]
    fn struct_shadow_omits_deref_when_component_has_no_base() {
        let component = demo_component(None);
        let item_struct: syn::ItemStruct = syn::parse_quote! {
            pub struct ShadowDemo {
                #[bindable]
                vm: std::rc::Rc<ShadowDemoViewModel>,
                #[environment(layout_spacing)]
                layout_spacing: f32,
            }
        };
        let shadow = build_component_struct_shadow(None, &item_struct, &component, None)
            .expect("struct shadow should build");
        let s = shadow.to_string();
        assert!(!s.contains("Deref"), "{s}");
    }

    /// PR #169 review remediation, round 2, T-R2-9/T-R2-10 (AD-R2-4/AD-R2-7): the shadow's own
    /// `new(..)` return type must match the real generator that would actually build this
    /// Component — bare `Self` for a view-less Component (`codegen::generate_component`),
    /// `std::rc::Rc<Self>` for a `has_view` one (`codegen::generate_view`) — read from
    /// `ComponentPublicShape::constructor_return`, not decided independently by the shadow itself.
    #[test]
    fn struct_shadow_constructor_return_matches_view_less_vs_has_view_real_generator() {
        let component = demo_component(None);
        let item_struct: syn::ItemStruct = syn::parse_quote! {
            pub struct ShadowDemo {
                #[bindable]
                vm: std::rc::Rc<ShadowDemoViewModel>,
                #[environment(layout_spacing)]
                layout_spacing: f32,
            }
        };

        let view_less_shadow = build_component_struct_shadow(None, &item_struct, &component, None)
            .expect("struct shadow should build")
            .to_string();
        assert!(
            view_less_shadow
                .contains("pub fn new (vm : std :: rc :: Rc < ShadowDemoViewModel > ,) -> Self"),
            "a view-less Component's shadow constructor must return bare Self, matching \
             codegen::generate_component: {view_less_shadow}"
        );
        assert!(
            !view_less_shadow.contains("Rc < Self >"),
            "{view_less_shadow}"
        );

        let view = empty_view("ShadowDemo");
        let has_view_shadow =
            build_component_struct_shadow(None, &item_struct, &component, Some(&view))
                .expect("struct shadow should build")
                .to_string();
        assert!(
            has_view_shadow.contains(
                "pub fn new (vm : std :: rc :: Rc < ShadowDemoViewModel > ,) -> std :: rc :: Rc < Self >"
            ),
            "a has-view Component's shadow constructor must return Rc<Self>, matching \
             codegen::generate_view: {has_view_shadow}"
        );
    }

    /// T2 (deferred `Option<T>` surface): a deferred own field gets a getter returning the full
    /// `Option<T>` and a setter taking the bare inner `T`, and is excluded from the constructor.
    #[test]
    fn struct_shadow_handles_deferred_option_param_and_state_and_prop_fields() {
        let mut component = demo_component(None);
        component.fields.push(FieldDef {
            name: "padding".to_string(),
            ty: "Option<f32>".to_string(),
            kind: FieldKind::Param,
            attrs: Vec::new(),
            initializer: None,
        });
        component.fields.push(FieldDef {
            name: "active".to_string(),
            ty: "bool".to_string(),
            kind: FieldKind::State,
            attrs: Vec::new(),
            initializer: Some(Initializer::Expr(syn::parse_quote!(false))),
        });
        component.fields.push(FieldDef {
            name: "label".to_string(),
            ty: "String".to_string(),
            kind: FieldKind::Prop,
            attrs: Vec::new(),
            initializer: Some(Initializer::Expr(syn::parse_quote!(String::new()))),
        });
        let item_struct: syn::ItemStruct = syn::parse_quote! {
            pub struct ShadowDemo {
                #[bindable]
                vm: std::rc::Rc<ShadowDemoViewModel>,
                #[environment(layout_spacing)]
                layout_spacing: f32,
                padding: Option<f32>,
                #[state]
                active: bool,
                #[prop]
                label: String,
            }
        };
        let shadow = build_component_struct_shadow(None, &item_struct, &component, None)
            .expect("struct shadow should build");
        parses_as_items(&shadow);
        let s = shadow.to_string();
        // Deferred: no ctor param, getter returns Option<f32>, setter takes bare f32.
        assert!(
            !s.contains("padding : Option"),
            "padding must not be a ctor param: {s}"
        );
        assert!(s.contains("fn padding (& self) -> Option < f32 >"), "{s}");
        assert!(s.contains("fn set_padding (& self , value : f32)"), "{s}");
        // State: private getter/setter (no `pub`).
        assert!(s.contains("fn active (& self) -> bool"), "{s}");
        assert!(s.contains("fn set_active (& self , value : bool)"), "{s}");
        assert!(!s.contains("pub fn active"), "{s}");
        assert!(!s.contains("pub fn set_active"), "{s}");
        // Prop: public getter/setter.
        assert!(s.contains("pub fn label (& self) -> String"), "{s}");
        assert!(
            s.contains("pub fn set_label (& self , value : String)"),
            "{s}"
        );
    }

    /// T3: the impl shadow builds from `item_impl` alone, independent of the struct registry.
    #[test]
    fn impl_shadow_exposes_overridable_and_overrides_methods() {
        let item_impl: syn::ItemImpl = syn::parse_quote! {
            impl ShadowDemoUnregistered {
                #[overridable]
                fn label(&self) -> String { "base".to_string() }

                #[overrides]
                fn on_something(&self, index: usize) {}
            }
        };
        let shadow = build_component_impl_shadow(&item_impl).expect("impl shadow should build");
        parses_as_items(&shadow);
        let s = shadow.to_string();
        assert!(s.contains("cfg (rust_analyzer)"), "{s}");
        assert!(s.contains("impl ShadowDemoUnregistered"), "{s}");
        assert!(s.contains("pub fn label (& self ,) -> String"), "{s}");
        assert!(
            s.contains("pub fn on_something (& self , index : usize ,)"),
            "{s}"
        );
    }

    #[test]
    fn impl_shadow_propagates_item_local_errors() {
        let item_impl: syn::ItemImpl = syn::parse_quote! {
            impl ShadowDemoBadTag {
                fn untagged(&self) {}
            }
        };
        let error = build_component_impl_shadow(&item_impl)
            .expect_err("an untagged method is an item-local error, not a shadow-suppressible one");
        assert!(
            error.contains("overridable") || error.contains("overrides"),
            "{error}"
        );
    }

    /// T7 (no-op shadow shape): the Theme shadow implements `Theme::apply` as a no-op, independent of
    /// any registered Environment Key.
    #[test]
    fn theme_shadow_is_a_noop_theme_impl() {
        let item_struct: syn::ItemStruct = syn::parse_quote! {
            struct ShadowThemeDemo {
                #[theme(value = 1.0)]
                totally_unregistered_key: f32,
            }
        };
        let shadow = build_theme_shadow(&item_struct).expect("theme shadow should build");
        parses_as_items(&shadow);
        let s = shadow.to_string();
        assert!(s.contains("cfg (rust_analyzer)"), "{s}");
        assert!(s.contains("struct ShadowThemeDemo"), "{s}");
        assert!(
            s.contains("impl elwindui :: core :: theme :: Theme for ShadowThemeDemo"),
            "{s}"
        );
        assert!(s.contains("fn apply"), "{s}");
    }
}
