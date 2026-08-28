//! AST(検証済み) → Rustソース。Pipeline designは`docs/design/tools/codegen_design.md`を参照。Cargo
//! featureでの静的分岐に落とし込み、`elwindui-core`のトレイト境界に対して書かれたコードを生成する
//! (今回はelwindui-backend-appkitのAPIを直接呼ぶ)。
//! 依存関係グラフに基づくCell/RefCellベースの更新関数生成は`docs/design/runtime/state_management_design.md`に対応する。

use crate::ast::{
    AssignmentKind, Attr, ChildEntry, ClosureBody, ComponentDef, DeferredViewExpr, ElementNode,
    EnumDef, FieldDef, FieldKind, Initializer, Item, MethodDef, Module, ShortcutScope, StoreDef,
    ViewAttribute, ViewBody, ViewDef, ViewExpr, ViewModelDef,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

/// What every `component`/`viewmodel` in the whole compilation unit looks like, so that
/// cross-file references (e.g. `notepad_window.rs`'s `vm.window_title` referring to a
/// `#[computed]` field defined in `notepad_view_model`) can be resolved.
///
/// Keyed by `(module real path, item name)` — the same address Rust's own name resolution uses
/// (see `ast::Module::path`) — rather than a bare item name, so two same-named types defined in
/// different modules never collide, and a lookup must go through `resolve` (i.e. through a `use`,
/// or be in the same module) instead of being visible from anywhere in the compilation unit. See
/// docs/specs/dsl_spec.md §11, docs/design/tools/codegen_design.md
#[derive(Clone)]
pub struct SymbolTable {
    types: HashMap<(Vec<String>, String), TypeInfo>,
}

#[derive(Clone)]
pub struct TypeInfo {
    pub fields: HashMap<String, FieldKind>,
    /// Every no-initializer field, `#[param]` or plain `prop` alike (kind-agnostic — see
    /// `build_symbol_table`'s own comment on why), in declaration order — the positional argument
    /// list `Target::new(...)` expects. Used to construct a nested user-defined component from an
    /// `ElementNode` (e.g. a `render_content` closure's `DocumentView { doc: doc }` body). Despite
    /// the name, a member can still get a real `set_<name>` setter and stay externally updatable —
    /// see `is_settable_field`, consulted by `emit_resync`'s own param-skip guard.
    pub param_fields: Vec<(String, String)>,
    /// Names of `#[param] #[two_way]` fields — a builtin shape's opt-in to automatic two-way
    /// wiring (see `emit_wiring`'s generic two-way rule). Empty for ordinary user components.
    pub two_way_fields: HashSet<String>,
    /// Names of `#[routed]` fields (docs/specs/dsl_spec.md §12) — a callback's opt-in to WinUI3-
    /// style bubbling via `elwindui::core::ui::dispatch_routed` instead of being called directly.
    /// Non-empty exactly when this type needs `into_node_if_needed` to share its own
    /// `routed_handlers()` into the `NativeControl`/virtual-builtin `UIElementBase` wrapping it,
    /// rather than starting that wrapper with a fresh, empty one.
    pub routed_fields: HashSet<String>,
    /// Fields implemented through the orthogonal `TextStyleOwner` capability. Derived from the
    /// injected `Attr::TextStyle` marker, never from a field-name list in codegen.
    pub text_style_fields: HashSet<String>,
    /// Brush properties that accept `BrushStyle`, derived from declaration metadata.
    pub semantic_brush_fields: HashSet<String>,
    /// Names of `#[bindable]` fields (`ast::Attr::Bindable`'s own doc comment,
    /// `docs/design/runtime/state_management_design.md`) — a component field injecting a viewmodel by
    /// syntax marker rather than type resolution. `collection_uses_rc_identity` consults this on a
    /// `for`-loop body's child element types to decide `replace_rc_items` vs `replace_items`
    /// without ever needing to resolve the loop's *element* type (only the child component type,
    /// e.g. `DocumentView`, which is always in scope — unlike the viewmodel type it injects, which
    /// may not be, exactly the same visibility gap `#[bindable]` itself exists to route around).
    /// Empty for a `viewmodel` (never itself has `#[bindable]` fields — only components inject).
    pub bindable_fields: HashSet<String>,
    /// `field_name -> name of the component that *directly* declares it` (the component whose own
    /// `ComponentDef::fields` literally lists it, not merely inherits it) — `resolve_effective_fields`
    /// flattens the whole `inherits` chain into one list and loses this, so it's tracked separately
    /// here (`resolve_field_declaring_types`, mirroring that same recursion). Consulted by
    /// `emit_field_setter_call` to decide whether a setter call needs UFCS disambiguation (see its
    /// own doc comment) — a field this type declares itself is never ambiguous, only one it inherited
    /// from some ancestor.
    pub declaring_types: HashMap<String, String>,
    /// Names of `#[onetime]` fields (`ast::Attr::Onetime`'s own doc comment) — applied once at
    /// construction, never re-pushed by `emit_resync`'s per-attribute loop. Empty for ordinary user
    /// components (only `Window` declares any today: `left`/`top`/`width`/
    /// `height`).
    pub onetime_fields: HashSet<String>,
    /// Whether this type is one of the hand-written-in-`elwindui_core::ui` "virtual" builtins with
    /// no `Type::new(args)` constructor and no `view` of its own (`VerticalLayout`/
    /// `HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape` today) — computed structurally
    /// (`is_builtin && !has_view && !is_native_control_leaf && !` this component's own `#[native]`
    /// flag, at `TypeInfo` construction time) rather than an enumerated name list, so adding a
    /// future virtual builtin needs no matching change here. See
    /// `build_virtual_value`'s own doc comment for the construction convention this drives.
    pub is_virtual_builtin: bool,
    /// Every field with no initializer, `#[param]` or not, mapped to its declared type — used
    /// purely for type-hint lookups (an `on_*` callback's arity, a resync setter's by-value-vs-
    /// by-reference calling convention), independent of whether the field is a constructor
    /// argument. A callback shape field (e.g. `TabView`'s `on_select: Box<dyn Fn(usize)>`) is
    /// deliberately *not* `#[param]` — it's wired post-construction via `emit_wiring`'s generic
    /// `on_*` rule, not passed to `Target::new(...)` — so it never appears in `param_fields`, but
    /// still needs its declared type visible here for the arity check.
    pub field_types: HashMap<String, String>,
    /// Declared types for every stored value field, including observable fields with an
    /// initializer. Dynamic `for` uses this metadata to identify `Vec<Rc<T>>` sources.
    pub value_field_types: HashMap<String, String>,
    /// `#[attached]` fields declared by this type (docs/specs/dsl_spec.md §3の添付プロパティ), mapped
    /// to their declared type — e.g. `Grid`'s own `{"row": "i32", "column": "i32"}`. Kept separate
    /// from `field_types` (rather than folded in) because that map filters out every field *with* an
    /// initializer, and `#[attached]` fields always have one (their required default value) —
    /// `validate.rs`'s `rejects_attached_field_without_default_value`. Consulted only by
    /// `emit_attached_setters`, which needs an owner's field's exact declared type to pick the right
    /// turbofish for `UIElementImpl::set_attached::<T>` — see that function's own doc comment for why
    /// guessing the type from the value expression alone isn't safe.
    pub attached_field_types: HashMap<String, String>,
    /// Whether this type is a `viewmodel` (`generate_viewmodel`'s output, which carries a
    /// `subscribe(impl Fn())` method) as opposed to a `component` (`generate_component`/
    /// `generate_view`'s output, which doesn't). Dependency subscription code checks this before
    /// emitting a `.subscribe(...)` call because plain component owners expose typed property
    /// subscriptions instead.
    pub is_viewmodel: bool,
    /// Whether this type is a `store` (`generate_store`'s output — a `ViewModelDef`-shaped body
    /// wrapped in a singleton `EnvironmentKey`/`instance()` accessor). Always paired with
    /// `is_viewmodel: true` (a store carries the same `subscribe`/`PropertyChanged` surface a
    /// viewmodel does, so every existing `is_viewmodel`-gated code path keeps working for a
    /// store-typed reference unmodified) — this flag additionally distinguishes a store for the
    /// `TypeName.field` bare-reference resolution `validate.rs`'s store-reference checks perform,
    /// which a plain `#[bindable]`-injected viewmodel reference does not use. See
    /// docs/design/runtime/state_management_design.md "Stores".
    pub is_store: bool,
    /// Whether this type is a genuine native-backed leaf (`Button`/`TextArea`/`Text`/`MenuBar`/
    /// `MenuBarItem`/`Menu`/`MenuItem`/`TabView` — the "NativeControl" family; or `Window`, whose
    /// own `#[native]` attribute marks it native despite having no meaningful `inherits` base at all
    /// — see `ComponentDef::native`'s doc comment) as opposed to a purely elwindui-side virtual node
    /// (`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`, or a user-defined `component`+
    /// `view` pair whose `view` root is itself virtual, e.g. `examples/notepad`'s `DocumentView`).
    /// This is a *structural* property computed recursively from the `view`'s root element type —
    /// see `build_symbol_table`'s `resolve_is_native` — not merely whether `inherits NativeControl`/
    /// `#[native]` was written (either is checked for *consistency* against this in `validate.rs`,
    /// but a plain `component X { .. } view X { VerticalLayout { .. } }` with no `inherits` at all is
    /// still correctly inferred as virtual). See docs/design/runtime/layout_design.md.
    pub is_native: bool,
    /// Whether this component's own declaration literally reads `inherits NativeControl`
    /// (`Button`/`TextArea`/`TabView` — as opposed to `#[native]` directly, e.g. `Window` or
    /// `MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabViewItem`, which never enter the visual tree).
    /// Unlike `is_native` (a recursively-inferred structural property), this is purely a shape-only
    /// declaration flag — only ever `true` for a hand-written builtin whose backend `XxxImpl` struct
    /// owns a real `base` (a backend-owned `NativeControlImpl`) and implements
    /// `NativeControl`/`UIElement` by delegating to it (docs/design/runtime/ui_tree_design.md).
    /// `emit_construction` uses this to pass a use-site `base: UIElementImpl` as this type's
    /// `Type::new(..)`'s leading argument (mirroring `emit_virtual_construction`'s own `base` — see
    /// `build_ui_element_base`), and `into_node_if_needed` uses it to skip the external
    /// `NativeControlImpl`, since the value already implements
    /// `UIElement` on its own.
    pub is_native_control_leaf: bool,
    /// Whether this type has a paired `view` (i.e. is `generate_view`'s output) as opposed to a
    /// hand-written `elwindui-backend-*` widget declared shape-only for the symbol table (every
    /// native leaf, and every virtual builtin like `Rectangle`). Every hand-written builtin's real
    /// `new(..)` takes `&str` for a `String`-shaped param by convention (see `emit_construction`'s
    /// `&(..)`-wrapping) — but a `view`-having component's *generated* `new(..)` takes the field's
    /// literal declared type verbatim (`generate_view`'s `param_types`), which for a plain
    /// `#[param] label: String` is an owned `String`, not `&str`. This flag is what lets
    /// `emit_construction` tell the two conventions apart at a call site. `true` whenever
    /// `effective_view` is `Some` — including a component with no `view` text of its own that
    /// inherits one from its base (see `resolve_view_for`), since that's still generated via
    /// `generate_view`, not `generate_component`.
    pub has_view: bool,
    /// This component's fully-flattened field list (`inherits`'s base fields, recursively, minus
    /// any legitimately `#[override]`n `#[computed]` field, followed by this component's own new
    /// fields) — see `resolve_effective_fields`. Empty for a `viewmodel` (which never inherits).
    /// What `generate_module` actually feeds to `generate_component`/`generate_view` instead of a
    /// component's raw, un-flattened `ComponentDef::fields`.
    pub effective_fields: Vec<FieldDef>,
    /// This component's fully-flattened method list — see `resolve_effective_methods`. Empty for a
    /// `viewmodel`.
    pub effective_methods: Vec<MethodDef>,
    /// This component's effective ordinary `view` — its own, if it wrote one, otherwise its base's
    /// ordinary view (recursively), retargeted to this component's name — see `resolve_view_for`.
    /// A base's typed `template: template_view!` is not copied into a derived target. `None` for a
    /// component with no ordinary view anywhere in its `inherits` chain (a plain data component,
    /// or one inheriting a primitive shape family with no `view` of its own, e.g. `Control`/
    /// `Rectangle`).
    pub effective_view: Option<ViewDef>,
    /// This component's own literal `view`'s `on_mount`/`on_unmount` blocks (not inherited/cloned —
    /// see `find_view`), used by `generate_view` to emit `__base_on_mount`/`__base_on_unmount`
    /// shadow methods for a *directly* derived component's `base::on_mount()`/`base::on_unmount()`
    /// calls. Deliberately only one level deep (not `effective_view`'s recursively-resolved hook) —
    /// see `generate_view`'s doc comment on the scope limit this implies for `base::` chains longer
    /// than one `inherits` hop.
    pub own_on_mount: Option<syn::Block>,
    pub own_on_unmount: Option<syn::Block>,
    /// The DSL name of the virtual-builtin shape (`Control`/`Shape`/`TextBlock`/`Grid`/
    /// `VerticalLayout`/`HorizontalLayout`) this component's generated struct ultimately composes
    /// over via a real `base: <Impl>` field (docs/design/runtime/ui_tree_design.md), if any — see
    /// `resolve_composed_shape`. `Some` in three cases, all "direct" ones collapsing into the same
    /// generated shape (`generate_view`'s `is_shape_composition` doesn't distinguish them):
    /// - Directly against a hand-written `elwindui::core::ui` primitive: this component's own
    ///   `view` root literally constructs that shape (`ContentControl inherits Control`).
    /// - Directly against another *already-composed* DSL component: same as above, one delegation
    ///   hop further out (`RoundedPanel inherits ContentControl`, own `view` root literally
    ///   `ContentControl`).
    /// - Transitively (`is_inherited_view_composition`): this component has no `view` of its own and
    ///   inherits an already-composed component (`LabeledPanel inherits ContentControl`).
    ///
    /// `None` for a plain component, one inheriting `NativeControl`, or one inheriting another
    /// component's *code* (a `#[virtual]`/`#[override]` method-hook base like `Derived inherits
    /// Base`) rather than its composed structure.
    pub composed_shape: Option<String>,
    /// The DSL name of a hand-written native host with no `UIElement` implementation of its own
    /// (only `Window` today — `is_native && !has_view && !is_native_control_leaf`) this component
    /// composes over via a real `base: <Impl>` field, "host composition" (docs/design/README.md
    /// §5.1) — the same `base`-field shape as `composed_shape`, but for a base that isn't a
    /// `UIElement` at all (so no `impl UIElement` is generated), and kept as a separate resolution
    /// pass from `composed_shape` since the two bases are structurally distinct categories that
    /// never overlap. `Some` iff this component's own `view` root literally constructs the base
    /// (mirroring `resolve_composed_shape`'s own root-match requirement) — see
    /// `resolve_host_composition_base`.
    pub host_composition_base: Option<String>,
    /// Whether this component is `#[sealed]` (docs/specs/dsl_spec.md 付録A) — `validate.rs`'s
    /// `validate_inherits` rejects `component X inherits Name` when this is `true`. `false` for a
    /// `viewmodel` (never a valid `inherits` target at all).
    pub sealed: bool,
    /// Whether this component is `#[abstract]` (docs/specs/dsl_spec.md 付録A) — a pure category tag
    /// (`UIElement`/`NativeControl`/`Layout`/`Shape`) that cannot be
    /// instantiated directly. `validate::check_element_value` rejects any `Type { .. }`/bare-child
    /// use site naming one; `generate_module` skips generating a `create_<snake case>(..)`/`new(..)`
    /// for it entirely. `false` for a `viewmodel`.
    pub is_abstract: bool,
    /// This component's own `#[content(field_name)]` (docs/specs/dsl_spec.md 付録A, WinUI3's
    /// `ContentPropertyAttribute` equivalent), copied verbatim from `ComponentDef::content_field` —
    /// no recursive resolution needed (unlike `is_native`/`composed_shape`), since a bare nested
    /// child element only ever binds to *this* component's own declared field, never inherited from
    /// a base. `build_component_args` reads this to know which field (if any) a bare nested child in
    /// a `view` construction of this component binds to, independent of field declaration order.
    /// ("first still-unclaimed non-`Option` field") fallback. `None` for a `viewmodel` and for any
    /// component that doesn't declare `#[content(..)]`.
    pub content_field: Option<String>,
    /// Whether this type is marked `#[embedded]` as a builtin shape,
    /// rather than being a consumer's own `#[elwindui::component]` declaration. `Module::is_builtin`
    /// only authorizes that attribute inside the embedded shape source; `ComponentDef::embedded`
    /// is the actual per-type builtin boundary.
    /// `concrete_type_ident`/`composed_create_fn_ident`/the `host_composition_base` trait-bound
    /// site use this to decide whether a reference to this type can be fully qualified as
    /// `elwindui::ui::..` (a builtin always lives there) or must stay a bare identifier (a
    /// consumer-defined component could be generated into any scope — codegen has no fixed path
    /// for it, only the flat crate-root `include!`/proc-macro convention that makes it visible
    /// unqualified).
    pub is_builtin: bool,
}

/// The source-level origin of a DSL element type.  Code generation cannot ask Rust's resolver
/// where an arbitrary imported, unqualified name was defined, so qualified external paths are the
/// explicit cross-crate boundary (`some_crate::Widget`).  All lowering paths use this same
/// classification when choosing a construction type, a class-shape macro, or an extension-trait
/// path; no individual emitter guesses from a control name or a crate-specific table.
#[derive(Clone)]
enum DslTypeOrigin {
    Builtin,
    Local,
    ExternalQualified { crate_prefix: syn::Ident },
    UnresolvedUnqualified,
}

fn dsl_type_origin(type_path: &str, info: Option<&TypeInfo>) -> DslTypeOrigin {
    if info.is_some_and(|info| info.is_builtin) {
        return DslTypeOrigin::Builtin;
    }

    // A qualified source path is the explicit cross-crate/local boundary.  Do this before
    // consulting a non-builtin same-crate table entry so a future table entry for a qualified
    // spelling cannot accidentally turn an authored external path back into a flat local name.
    if let Ok(path) = syn::parse_str::<syn::Path>(type_path)
        && path.segments.len() >= 2
        && let Some(first) = path.segments.first()
    {
        return match first.ident.to_string().as_str() {
            "elwindui" => DslTypeOrigin::Builtin,
            "crate" | "self" | "super" => DslTypeOrigin::Local,
            _ => DslTypeOrigin::ExternalQualified {
                crate_prefix: first.ident.clone(),
            },
        };
    }

    match info {
        Some(info) if info.is_builtin => DslTypeOrigin::Builtin,
        Some(_) => DslTypeOrigin::Local,
        None => DslTypeOrigin::UnresolvedUnqualified,
    }
}

fn dsl_type_ident(type_path: &str) -> syn::Ident {
    let name = type_path.rsplit("::").next().unwrap_or(type_path);
    format_ident!("{name}")
}

fn dsl_authored_path(type_path: &str) -> syn::Path {
    syn::parse_str(type_path).unwrap_or_else(|error| {
        panic!("DSL element type path `{type_path}` should be valid Rust syntax: {error}")
    })
}

/// Resolves the defining crate's exported `__elwindui_props_<Type>!` macro.  `#[macro_export]`
/// places that macro at the defining crate root, so only the first segment of a qualified Rust
/// path is retained (`crate_alias::widgets::Thing` -> `crate_alias::__elwindui_props_Thing`).
fn dsl_props_macro_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    let ident = format_ident!("__elwindui_props_{}", dsl_type_ident(type_path));
    match dsl_type_origin(type_path, info) {
        DslTypeOrigin::Builtin | DslTypeOrigin::UnresolvedUnqualified => {
            quote! { elwindui::core::#ident }
        }
        DslTypeOrigin::Local => quote! { crate::#ident },
        DslTypeOrigin::ExternalQualified { crate_prefix } => {
            quote! { #crate_prefix::#ident }
        }
    }
}

/// Resolves the extension trait associated with a DSL element.  Unlike a props macro, a generated
/// extension trait lives alongside the authored type, so a qualified external path keeps all of
/// its intermediate Rust modules (`crate_alias::widgets::ThingExt`).
fn dsl_ext_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    let mut authored = dsl_authored_path(type_path);
    let ext_ident = format_ident!("{}Ext", dsl_type_ident(type_path));
    match dsl_type_origin(type_path, info) {
        DslTypeOrigin::Builtin | DslTypeOrigin::UnresolvedUnqualified => {
            quote! { elwindui::core::ui::#ext_ident }
        }
        DslTypeOrigin::Local => {
            if type_path.contains("::") {
                authored
                    .segments
                    .last_mut()
                    .expect("qualified DSL type path has a final segment")
                    .ident = ext_ident;
                quote! { #authored }
            } else {
                quote! { #ext_ident }
            }
        }
        DslTypeOrigin::ExternalQualified { .. } => {
            authored
                .segments
                .last_mut()
                .expect("qualified DSL type path has a final segment")
                .ident = ext_ident;
            quote! { #authored }
        }
    }
}

/// Resolves the concrete Rust type constructed for a DSL element.  Builtins retain the facade
/// normalization used by existing `view!` code; local generated components retain their flat
/// crate-local name; a qualified external component is constructed through exactly the path the
/// author wrote.
fn dsl_concrete_type_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    let ident = dsl_type_ident(type_path);
    let authored = dsl_authored_path(type_path);
    match dsl_type_origin(type_path, info) {
        DslTypeOrigin::ExternalQualified { .. } => {
            let authored = dsl_authored_path(type_path);
            quote! { #authored }
        }
        DslTypeOrigin::Builtin | DslTypeOrigin::UnresolvedUnqualified => {
            quote! { elwindui::ui::#ident }
        }
        DslTypeOrigin::Local => {
            if type_path.contains("::") {
                quote! { #authored }
            } else {
                quote! { #ident }
            }
        }
    }
}

impl SymbolTable {
    /// Resolves `name` as seen from `from` to its symbol-table key: a type defined locally in
    /// `from` (same real path), or brought into scope by one of `from`'s `use` declarations,
    /// matched by real path exactly like Rust's own name resolution (`use`'s last path segment is
    /// the item name; the segments before it — with a leading `crate` keyword stripped, since
    /// `Module::path` never includes it — must equal some module's real path). `resolve` (below)
    /// is the public, common-case wrapper; `resolve_is_native` needs the key itself so it can
    /// recurse into *that* type's own `is_native` computation rather than reading a
    /// not-yet-finalized `TypeInfo`.
    fn resolve_key(&self, from: &Module, name: &str) -> Option<(Vec<String>, String)> {
        let direct = (from.path.clone(), name.to_string());
        if self.types.contains_key(&direct) {
            return Some(direct);
        }
        from.uses.iter().find_map(|u| {
            let [prefix @ .., last] = u.path.as_slice() else {
                return None;
            };
            if last != name {
                return None;
            }
            let real_prefix = match prefix {
                [first, rest @ ..] if first == "crate" => rest,
                other => other,
            };
            let key = (real_prefix.to_vec(), name.to_string());
            self.types.contains_key(&key).then_some(key)
        })
    }

    /// Resolves `name` as seen from `from`. Returns `None` if `name` isn't visible from `from` at
    /// all — an unresolved reference (e.g. a missing `use`), which callers turn into a validation
    /// error.
    pub fn resolve(&self, from: &Module, name: &str) -> Option<&TypeInfo> {
        self.resolve_key(from, name).map(|key| &self.types[&key])
    }

    /// Resolves an unqualified user-defined type when the frontend has no lexical module
    /// context (the expression-form `template_view!` frontend is the one such caller).  The
    /// normal `resolve` path remains authoritative whenever a module/import context exists.  In
    /// the context-free case a name is accepted only when exactly one non-builtin type with that
    /// spelling is present; ambiguous names stay unresolved instead of inventing a type-name
    /// dispatch rule or silently selecting one component from another module.
    pub fn resolve_unqualified(&self, name: &str) -> Option<&TypeInfo> {
        let mut matches = self
            .types
            .iter()
            .filter(|((_, candidate), info)| candidate == name && !info.is_builtin)
            .map(|(_, info)| info);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
}

/// PR #165 final rereview remediation, A2: computes the exact field-readable/writable schema a
/// `ViewExpr::DeferredView` lowered out of `component_name`'s own `view! { .. }` body is allowed to
/// implicitly fall back to (`crate::ast::ImplicitOwnerDef`). Called once per outer `#[elwindui::
/// component] impl` invocation (`lib.rs`'s `generate_component_from_item_impl`), against a
/// symbol table built from the *unlowered* `all_modules` (before any hidden Component/View pair
/// exists), then threaded unchanged through every nesting depth of `lower_deferred_views_in_*` —
/// never recomputed from a synthetic hidden Component's own (effectively empty) field list.
///
/// `readable_fields`/`writable_fields`/`reactive_fields`/`bindable_fields` are derived from
/// `component_name`'s own *effective* fields (`TypeInfo::effective_fields` — `resolve_effective_
/// fields`, inherited fields included, not just `ComponentDef::fields`'s literal declarations),
/// matching exactly what that Component's own ordinary generated `view!` code can already read/
/// write/subscribe-to through a bare name:
///
/// - readable: `Prop`, `State`, `Param`, `Computed`, `Environment` — every field kind with an
///   ordinary generated instance getter.
/// - writable: `Prop`, `State` — every field kind with an ordinary generated `set_<name>` setter.
/// - reactive (PR #165 post-final rereview remediation, A9): `Prop`, `State`, `Computed`,
///   `Environment` — every field kind that participates in `generate_view`'s own
///   `component_property_variants` (the source of the generated `<Component>Property` enum and its
///   `on_property_changed` dispatch). Deliberately excludes `Param`: a `#[param]` field (including a
///   plain, non-`#[bindable]` one) is fixed at construction and never reassigned, so it has no
///   `PropertyChanged` variant of its own in the generated code — repository reality confirmed via
///   `component_property_variants`'s own construction (`mutable_required_names`/`own_default_names`/
///   `own_computed_names`/`own_environment_names`, never `param_names`), not assumed.
/// - bindable (PR #165 post-final rereview remediation, A9): reuses `TypeInfo::bindable_fields`
///   directly (same `Attr::Bindable`-tagged effective-field derivation `build_symbol_table` already
///   performs for every ordinary Component) rather than re-deriving it — a `#[bindable]` field is
///   always `FieldKind::Param` (`attr_frontend.rs`'s own `"bindable"` arm), so it is already present
///   in `readable_fields` and absent from `writable_fields`/`reactive_fields`; this set exists so a
///   2-segment `vm.field` path can be recognized as a *logical* bindable owner reachable through the
///   source lexical owner, distinct from a plain 1-segment readable field.
/// - excluded entirely: `Attached` (schema-only, not real instance data of the declaring
///   component — `FieldKind::Attached`'s own doc comment) and `Action` (never appears in a
///   Component's own `effective_fields`; `#[observable]`/`#[async_computed]` are viewmodel/store-
///   only by construction and validation rule 20, so they never appear here either).
///
/// Panics if `component_name` doesn't resolve in `table` as seen from `from` — an internal codegen
/// invariant failure, since this is only ever called after `validate::validate` already confirmed
/// `component_name` is a real, well-formed Component in this exact compilation unit.
pub(crate) fn implicit_owner_schema(
    table: &SymbolTable,
    from: &Module,
    component_name: &str,
) -> crate::ast::ImplicitOwnerDef {
    let info = table.resolve(from, component_name).unwrap_or_else(|| {
        panic!(
            "internal codegen invariant violated: source Component `{component_name}` must \
             resolve in the pre-lowering symbol table once validation has already succeeded"
        )
    });
    let mut readable_fields = HashSet::new();
    let mut writable_fields = HashSet::new();
    let mut reactive_fields = HashSet::new();
    for f in &info.effective_fields {
        match f.kind {
            FieldKind::Prop | FieldKind::State => {
                readable_fields.insert(f.name.clone());
                writable_fields.insert(f.name.clone());
                reactive_fields.insert(f.name.clone());
            }
            FieldKind::Computed | FieldKind::Environment => {
                readable_fields.insert(f.name.clone());
                reactive_fields.insert(f.name.clone());
            }
            FieldKind::Param => {
                readable_fields.insert(f.name.clone());
            }
            FieldKind::Attached
            | FieldKind::Action
            | FieldKind::Observable
            | FieldKind::AsyncComputed => {}
        }
    }
    crate::ast::ImplicitOwnerDef {
        field_name: "__view_owner".to_string(),
        readable_fields,
        writable_fields,
        reactive_fields,
        bindable_fields: info.bindable_fields.clone(),
    }
}

/// Strips a single `Rc<...>`/`std::rc::Rc<...>` wrapper so a `#[param] #[inject]` field declared
/// as `doc: std::rc::Rc<DocumentViewModel>` still resolves against the bare `DocumentViewModel`
/// entry in the symbol table — fields are commonly `Rc`-wrapped since `#[inject]`'s whole purpose
/// is sharing one instance across owners (docs/design/runtime/state_management_design.md). Leaves any other type string unchanged.
pub(crate) fn strip_rc_wrapper(ty: &str) -> &str {
    let ty = ty.trim();
    for prefix in ["std::rc::Rc<", "rc::Rc<", "Rc<"] {
        if let Some(inner) = ty.strip_prefix(prefix).and_then(|s| s.strip_suffix('>')) {
            return inner.trim();
        }
    }
    ty
}

fn is_weak_type(ty: &str) -> bool {
    let ty = ty.trim();
    ["std::rc::Weak<", "rc::Weak<", "Weak<"]
        .iter()
        .any(|prefix| ty.starts_with(prefix) && ty.ends_with('>'))
}

pub(crate) fn strip_weak_wrapper(ty: &str) -> &str {
    let ty = ty.trim();
    for prefix in ["std::rc::Weak<", "rc::Weak<", "Weak<"] {
        if let Some(inner) = ty.strip_prefix(prefix).and_then(|s| s.strip_suffix('>')) {
            return inner.trim();
        }
    }
    ty
}

pub fn build_symbol_table(modules: &[Module]) -> SymbolTable {
    let mut types = HashMap::new();
    // `(module index, #[[inherits]] base name, effective view's root element type, #[native])` per
    // `component` key — the raw material `resolve_is_native` (below) needs; not every component has
    // an effective `view` (native leaf builtins and virtual builtins like `VerticalLayout`/`Rectangle`
    // are declared shape-only, see `BUILTIN_SHAPE_SOURCE`) or a `base` (only `inherits`-using
    // components do — `#[native]` components, e.g. `Window`, deliberately have neither). The root is
    // the *effective* one (`resolve_view_for` — own view, or inherited from `base`), not just a
    // literal same-module `Item::View`, so a component with no `view` of its own that inherits a
    // logical base's template is still inferred native/virtual correctly.
    let mut component_meta: HashMap<
        (Vec<String>, String),
        (usize, Option<String>, Option<String>, bool),
    > = HashMap::new();

    for (module_index, module) in modules.iter().enumerate() {
        for item in &module.items {
            let Item::Component(c) = item else { continue };
            let view_root = resolve_effective_root_type(module, c, modules);
            component_meta.insert(
                (module.path.clone(), c.name.clone()),
                (module_index, c.base.clone(), view_root, c.native),
            );
        }

        for item in &module.items {
            match item {
                Item::Component(c) => {
                    let effective_fields = resolve_effective_fields(module, c, modules);
                    let effective_methods = resolve_effective_methods(module, c, modules);
                    let effective_view = resolve_view_for(module, c, modules);
                    let own_view = find_view(module, &c.name);
                    let field_kinds = effective_fields
                        .iter()
                        .filter(|field| field.kind != FieldKind::State)
                        .map(|f| (f.name.clone(), f.kind))
                        .collect();
                    // Kind-agnostic (not `f.kind == FieldKind::Param`): now that the builtin shape source is gone,
                    // own fields are plain (unattributed) `prop`s rather than `#[param]` (their
                    // backing Rust types are all zero-arg-constructed with post-construction
                    // `set_<field>` setters regardless — docs/design/runtime/ui_tree_design.md — so
                    // `#[param]` fields remain fixed at instantiation, so this
                    // must select construction-time fields the same way `generate_view`'s own
                    // `param_names` already does (`f.initializer.is_none()`, kind-independent) for
                    // caller/callee agreement (`base_param_count`, `build_component_args`/
                    // `build_component_setters`/`build_component_optional_setters`, validate.rs's
                    // `check_element_value`). `on_*`-named fields are excluded explicitly — they're
                    // event callbacks routed entirely through `emit_wiring`/`emit_resync` (which
                    // already key off this exact same `on_` name prefix, not `FieldKind`), never
                    // construction-time values, and never had a matching `set_on_<x>` on
                    // hand-written natives (only `register_routed_handler` for `#[routed]` ones).
                    // `#[environment(name)]` fields are excluded too, for a different reason than
                    // `on_*`: they also have no initializer, but are resolved from the ambient
                    // `EnvironmentContext` at construction (`docs/design/runtime/
                    // theme_environment_design.md`'s "Environment" section), never supplied by a
                    // caller — `f.kind`-based here since (unlike `on_*`) there is no name convention
                    // to key off instead.
                    let param_fields = effective_fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && !f.name.starts_with("on_")
                                && f.kind != FieldKind::Environment
                        })
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    let two_way_fields = effective_fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::TwoWay))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let routed_fields = effective_fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::Routed))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let text_style_fields = effective_fields
                        .iter()
                        .filter(|f| f.attrs.iter().any(|a| matches!(a, Attr::TextStyle)))
                        .map(|f| f.name.clone())
                        .collect();
                    let semantic_brush_fields = effective_fields
                        .iter()
                        .filter(|f| f.attrs.iter().any(|a| matches!(a, Attr::SemanticBrush)))
                        .map(|f| f.name.clone())
                        .collect();
                    let bindable_fields = effective_fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::Bindable))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let onetime_fields = effective_fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::Onetime))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let field_types = effective_fields
                        .iter()
                        .filter(|f| f.initializer.is_none() && f.kind != FieldKind::Environment)
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    let attached_field_types = effective_fields
                        .iter()
                        .filter(|f| f.kind == FieldKind::Attached)
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    let has_view = effective_view.is_some();
                    // `is_native` is finalized in the second pass below, once every type is present
                    // in `table` to recurse through (a component's `view` root may be defined later
                    // in iteration order, or in another module entirely).
                    types.insert(
                        (module.path.clone(), c.name.clone()),
                        TypeInfo {
                            fields: field_kinds,
                            param_fields,
                            two_way_fields,
                            routed_fields,
                            text_style_fields,
                            semantic_brush_fields,
                            bindable_fields,
                            onetime_fields,
                            // A "virtual builtin" is exactly: an `#[embedded]` shape declaration
                            // from the builtin shape set, with no `view` of its own, that isn't native
                            // (neither `inherits NativeControl` nor `#[native]` directly). `Module::
                            // is_builtin` only establishes that `#[embedded]` is legal in this source
                            // file; the component-level attribute is the actual builtin/user boundary.
                            // This is computable in this first pass from `c.embedded`/`has_view`/
                            // `c.base`/`c.native`, unlike `is_native` itself (which needs the second,
                            // cross-module-recursive pass below).
                            is_virtual_builtin: c.embedded
                                && !has_view
                                && c.base.as_deref() != Some("NativeControl")
                                && !c.native,
                            field_types,
                            value_field_types: c
                                .fields
                                .iter()
                                .map(|f| (f.name.clone(), f.ty.clone()))
                                .collect(),
                            attached_field_types,
                            is_viewmodel: false,
                            is_store: false,
                            is_native: false,
                            is_native_control_leaf: c.base.as_deref() == Some("NativeControl"),
                            has_view,
                            effective_fields,
                            effective_methods,
                            effective_view,
                            own_on_mount: own_view.and_then(|v| v.on_mount.clone()),
                            own_on_unmount: own_view.and_then(|v| v.on_unmount.clone()),
                            // Finalized in the same later pass as `is_native`, for the same reason.
                            composed_shape: None,
                            host_composition_base: None,
                            sealed: c.sealed,
                            is_abstract: c.is_abstract,
                            content_field: resolve_content_field(module, c, modules),
                            is_builtin: c.embedded,
                            declaring_types: resolve_field_declaring_types(module, c, modules),
                        },
                    );
                }
                Item::ViewModel(v) => {
                    let field_kinds = v.fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
                    // Kind-agnostic — see the matching `Item::Component` arm's `param_fields`
                    // above for why.
                    let param_fields = v
                        .fields
                        .iter()
                        .filter(|f| f.initializer.is_none() && !f.name.starts_with("on_"))
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    let two_way_fields = v
                        .fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::TwoWay))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let routed_fields = v
                        .fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::Routed))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let field_types = v
                        .fields
                        .iter()
                        .filter(|f| f.initializer.is_none())
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    types.insert(
                        (module.path.clone(), v.name.clone()),
                        TypeInfo {
                            fields: field_kinds,
                            param_fields,
                            two_way_fields,
                            routed_fields,
                            text_style_fields: HashSet::new(),
                            semantic_brush_fields: HashSet::new(),
                            bindable_fields: HashSet::new(),
                            declaring_types: HashMap::new(),
                            onetime_fields: HashSet::new(),
                            is_virtual_builtin: false,
                            field_types,
                            value_field_types: v
                                .fields
                                .iter()
                                .map(|f| (f.name.clone(), f.ty.clone()))
                                .collect(),
                            attached_field_types: HashMap::new(),
                            is_viewmodel: true,
                            is_store: false,
                            is_native: false,
                            is_native_control_leaf: false,
                            has_view: false,
                            effective_fields: Vec::new(),
                            effective_methods: Vec::new(),
                            effective_view: None,
                            own_on_mount: None,
                            own_on_unmount: None,
                            composed_shape: None,
                            host_composition_base: None,
                            sealed: false,
                            is_abstract: false,
                            content_field: None,
                            is_builtin: module.is_builtin,
                        },
                    );
                }
                Item::Store(s) => {
                    let field_kinds = s.fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
                    let param_fields = s
                        .fields
                        .iter()
                        .filter(|f| f.initializer.is_none() && !f.name.starts_with("on_"))
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    let two_way_fields = s
                        .fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::TwoWay))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let routed_fields = s
                        .fields
                        .iter()
                        .filter(|f| {
                            f.initializer.is_none()
                                && f.attrs.iter().any(|a| matches!(a, Attr::Routed))
                        })
                        .map(|f| f.name.clone())
                        .collect();
                    let field_types = s
                        .fields
                        .iter()
                        .filter(|f| f.initializer.is_none())
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect();
                    types.insert(
                        (module.path.clone(), s.name.clone()),
                        TypeInfo {
                            fields: field_kinds,
                            param_fields,
                            two_way_fields,
                            routed_fields,
                            text_style_fields: HashSet::new(),
                            semantic_brush_fields: HashSet::new(),
                            bindable_fields: HashSet::new(),
                            declaring_types: HashMap::new(),
                            onetime_fields: HashSet::new(),
                            is_virtual_builtin: false,
                            field_types,
                            value_field_types: s
                                .fields
                                .iter()
                                .map(|f| (f.name.clone(), f.ty.clone()))
                                .collect(),
                            attached_field_types: HashMap::new(),
                            // A store carries the same `subscribe`/`PropertyChanged` surface a
                            // viewmodel does (`generate_store` delegates field codegen to
                            // `generate_viewmodel`), so every existing `is_viewmodel`-gated code
                            // path (dependency subscription codegen, etc.) keeps working for a
                            // store-typed reference without auditing every call site.
                            is_viewmodel: true,
                            is_store: true,
                            is_native: false,
                            is_native_control_leaf: false,
                            has_view: false,
                            effective_fields: Vec::new(),
                            effective_methods: Vec::new(),
                            effective_view: None,
                            own_on_mount: None,
                            own_on_unmount: None,
                            composed_shape: None,
                            host_composition_base: None,
                            sealed: false,
                            is_abstract: false,
                            content_field: None,
                            is_builtin: module.is_builtin,
                        },
                    );
                }
                Item::Enum(_) | Item::View(_) => {}
            }
        }
    }

    let table = SymbolTable { types };
    let mut memo: HashMap<(Vec<String>, String), bool> = HashMap::new();
    let keys: Vec<(Vec<String>, String)> = table.types.keys().cloned().collect();
    for key in &keys {
        resolve_is_native(key, &component_meta, modules, &table, &mut memo);
    }

    let mut composed_shape_memo: HashMap<(Vec<String>, String), Option<String>> = HashMap::new();
    for key in &keys {
        resolve_composed_shape(
            key,
            &component_meta,
            modules,
            &table,
            &mut composed_shape_memo,
        );
    }

    let host_composition_memo: HashMap<
        (Vec<String>, String),
        Option<(String, (Vec<String>, String))>,
    > = keys
        .iter()
        .map(|key| {
            (
                key.clone(),
                resolve_host_composition_base(key, &component_meta, modules, &table, &memo),
            )
        })
        .collect();
    let mut types = table.types;
    for (key, info) in types.iter_mut() {
        info.is_native = memo.get(key).copied().unwrap_or(false);
        info.composed_shape = composed_shape_memo.get(key).cloned().flatten();
        info.host_composition_base = host_composition_memo
            .get(key)
            .cloned()
            .flatten()
            .map(|(name, _)| name);
    }
    SymbolTable { types }
}

/// Resolves `name` as seen from `from` directly against `modules`' raw AST (no `SymbolTable`
/// needed — this is what `build_symbol_table` itself uses to resolve an `inherits` base while
/// still building the table), mirroring `SymbolTable::resolve_key`'s own name-resolution rule:
/// defined locally — in *any* module sharing `from`'s real path, not just `from` itself, since
/// every builtin shape lives in the same same-path (`[]`), `use`-less builtin shape set
/// (`builtin_modules`'s own doc comment) — or brought into scope by one of `from`'s `use`
/// declarations.
fn find_component_and_module<'m>(
    from: &'m Module,
    name: &str,
    modules: &'m [Module],
) -> Option<(&'m Module, &'m ComponentDef)> {
    if let Some(found) = modules
        .iter()
        .filter(|m| m.path == from.path)
        .find_map(|m| {
            m.items.iter().find_map(|i| match i {
                Item::Component(c) if c.name == name => Some((m, c)),
                _ => None,
            })
        })
    {
        return Some(found);
    }
    for u in &from.uses {
        let [prefix @ .., last] = u.path.as_slice() else {
            continue;
        };
        if last != name {
            continue;
        }
        let real_prefix: &[String] = match prefix {
            [first, rest @ ..] if first == "crate" => rest,
            other => other,
        };
        if let Some(m) = modules.iter().find(|m| m.path == real_prefix) {
            if let Some(c) = m.items.iter().find_map(|i| match i {
                Item::Component(c) if c.name == name => Some(c),
                _ => None,
            }) {
                return Some((m, c));
            }
        }
    }
    None
}

/// A component's own literal `view` (not inherited/cloned from a base) — `None` for a shape-only
/// declaration (`Control`/`Rectangle`/every native leaf).
fn find_view<'m>(module: &'m Module, target: &str) -> Option<&'m ViewDef> {
    module.items.iter().find_map(|i| match i {
        Item::View(v) if v.target == target => Some(v),
        _ => None,
    })
}

/// PR #169 review remediation, round 3 (A1/AD-R3-1): whether `#[content(name)]`'s target field's
/// presence-or-absence on `c` is decidable from `c`/`c`'s own `view` alone, or genuinely needs
/// `modules` (same-crate sibling registry data that may not yet be fully populated under
/// rust-analyzer's own incremental expansion order even when the source is correctly ordered).
pub(crate) enum ContentFieldKnowledge {
    /// `name` is one of `c`'s own literal fields, or — when `c.base` has no local `ComponentDef` in
    /// `modules` — one of `synthesize_external_base_fields`'s own view-bare-reference-derived
    /// fields. Both are fully decidable from `c`/`c`'s own view alone.
    KnownPresent,
    /// `name` is absent from `c`'s own fields, and either `c` has no `base` at all, or `c.base` has
    /// no local `ComponentDef` in `modules` (so `synthesize_external_base_fields`'s own
    /// view-bare-reference-only synthesis is the complete answer) — either way, decidable from `c`
    /// alone with no sibling-registry dependency.
    KnownMissingItemLocal,
    /// `name` is absent from `c`'s own fields, and `c.base` names a `ComponentDef` found locally in
    /// `modules` (a same-crate sibling) — whether `name` is actually present depends on that
    /// sibling's own (possibly further-inherited) effective fields, which may be spuriously
    /// unresolvable under rust-analyzer's own incomplete same-crate registry expansion order even
    /// when the source is correctly ordered.
    NeedsSameCrateRegistry,
}

/// Classifies whether `#[content(name)]` naming a field absent from `c`'s own literal fields is
/// decidable from `c` alone (see [`ContentFieldKnowledge`]) — used by `validate::validate_classified`
/// to route the corresponding diagnostic (PR #169 review, round 3, A1/AD-R3-1: a local
/// `#[content(name)]` typo on a base-less component, or one whose base isn't a locally-visible
/// `ComponentDef` at all, was previously over-classified `RegistryDependent` merely because the
/// general-purpose `resolve_effective_fields` this check calls *can*, in other cases, consult
/// `modules` — not because *this* call's own branch actually needed to).
pub(crate) fn content_field_knowledge(
    from: &Module,
    c: &ComponentDef,
    name: &str,
    modules: &[Module],
) -> ContentFieldKnowledge {
    if c.fields.iter().any(|f| f.name == *name) {
        return ContentFieldKnowledge::KnownPresent;
    }
    let Some(base) = c.base.as_deref() else {
        return ContentFieldKnowledge::KnownMissingItemLocal;
    };
    if find_component_and_module(from, base, modules).is_none() {
        let synthesized = synthesize_external_base_fields(
            c,
            base,
            c.base_path.as_deref(),
            find_view(from, &c.name),
        );
        return if synthesized.iter().any(|f| f.name == *name) {
            ContentFieldKnowledge::KnownPresent
        } else {
            ContentFieldKnowledge::KnownMissingItemLocal
        };
    }
    ContentFieldKnowledge::NeedsSameCrateRegistry
}

/// Recursively flattens `c`'s effective field list: its (non-`NativeControl`) base's own effective
/// fields, minus any this component legitimately redeclares (an `#[override]`n `#[computed]` field
/// — validated by `validate::validate_field_overrides`; codegen trusts that here rather than
/// re-checking), followed by `c`'s own newly-declared fields. See `ComponentDef`'s doc comment.
///
/// A component with its own `view` only inherits the specific base fields that view actually
/// forwards by a bare same-name reference (e.g. `ContentControl`'s `Control { padding: padding }`)
/// — one it hardcodes with a literal/expression instead (`RoundedPanel`'s `Rectangle { fill:
/// "#3a3a3c" }`) or never mentions at all (`RoundedPanel` never sets `Rectangle`'s `stroke`) stays
/// invisible and keeps its own default, exactly like the pre-inheritance hand-written model — a
/// hardcoded/unset base field becoming a new required constructor parameter on the derived
/// component (with no way for its own view to ever use the caller's value) would be a silent API
/// break, not real inheritance. A component with *no* own view (pure template inheritance, see
/// `resolve_view_for`) gets every one of base's fields unconditionally, since the entire inherited
/// view already references them all the normal way.
pub(crate) fn resolve_effective_fields<'m>(
    from: &'m Module,
    c: &ComponentDef,
    modules: &'m [Module],
) -> Vec<FieldDef> {
    let Some(base) = c.base.as_deref() else {
        return c.fields.clone();
    };
    let Some((base_module, base_c)) = find_component_and_module(from, base, modules) else {
        return synthesize_external_base_fields(
            c,
            base,
            c.base_path.as_deref(),
            find_view(from, &c.name),
        );
    };
    let base_fields: Vec<FieldDef> = resolve_effective_fields(base_module, base_c, modules)
        .into_iter()
        .filter(|field| field.kind != FieldKind::State)
        .collect();
    let base_fields: Vec<FieldDef> = match find_view(from, &c.name) {
        // `#[routed]` fields (docs/design/runtime/ui_tree_design.md, e.g. `UIElement`'s own
        // `on_tapped`/`on_pointer_pressed`/...), and every field declared directly on the root
        // `UIElement` component itself (`margin`/`width`/`height`/... — the builtin's own `#[class]` doc
        // comment on that declaration: "every component — builtin or user-defined — picks them up
        // for free ... with no per-attribute-name hardcoding in the compiler"), are exempt from the
        // bare-reference requirement below: both apply directly to whatever concrete node this
        // component constructs (`emit_wiring`'s `is_routed` branch for the former,
        // `build_component_args`/`build_component_setters`/`build_component_optional_setters`'s
        // generic per-`param_fields` setter emission for the latter) regardless of whether the view
        // body happens to mention them by name — unlike an ordinary value field, there is nothing
        // for the view to "forward" in the first place, so requiring a bare reference would just
        // silently drop them for any component with its own view (in practice nearly every real
        // one). The `UIElement`-membership check is a plain name lookup against its own (not
        // recursively flattened) `ComponentDef::fields` — resolved the same way any other
        // `inherits` target already is in this function, so no field name is ever hardcoded here.
        Some(view) => {
            let common_fields: HashSet<&str> =
                find_component_and_module(from, "UIElement", modules)
                    .map(|(_, ui)| ui.fields.iter().map(|f| f.name.as_str()).collect())
                    .unwrap_or_default();
            let template_parent_fields = if view.is_template {
                collect_template_parent_field_names(view)
            } else {
                HashSet::new()
            };
            base_fields
                .into_iter()
                .filter(|f| {
                    f.attrs
                        .iter()
                        .any(|a| matches!(a, Attr::Routed | Attr::TextStyle))
                        || common_fields.contains(f.name.as_str())
                        || view_references_bare_name(view, &f.name)
                        || template_parent_fields.contains(&f.name)
                })
                .collect()
        }
        None => base_fields,
    };
    let own_names: HashSet<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
    let mut result: Vec<FieldDef> = base_fields
        .into_iter()
        .filter(|f| !own_names.contains(f.name.as_str()))
        .collect();
    result.extend(c.fields.iter().cloned());
    result
}

/// `resolve_effective_fields`'s fallback when `base` has no local `ComponentDef` visible to this
/// macro invocation at all — the normal case for every real builtin in production
/// (`elwindui_codegen::testdata`'s own doc comment: the old workspace-wide `builtin_modules()` was
/// removed, so a real `Control`/`ContentControl`/... is never parsed DSL text here, only a compiled
/// Rust type reachable through `__elwindui_props_{Name}!`, Refs #90).
/// Without a local field list there's nothing to filter a *known* base field list against the way
/// the `find_component_and_module`-succeeds branch does — so this takes the DSL author's own bare
/// same-name attribute-value reference (`padding: padding`, dsl_spec.md §3's `ContentControl`
/// example) as the *only* available evidence that `base` declares such a field at all, exactly
/// mirroring how a bare `ChildEntry::Ref` (`content`) is already accepted structurally with no
/// `TypeInfo` lookup (`generate_view`'s `PASSTHROUGH_NODE` seeding). Each recovered name is given
/// the defining base's `__elwindui_props_{Name}!(@field_type {name})` as its literal `FieldDef::ty` —
/// a type-position macro invocation `generate_view`'s existing `syn::parse_str::<syn::Type>` calls
/// already handle as an ordinary `syn::Type::Macro` — deferring the actual type (and, for a
/// nonexistent name, a real `compile_error!`) to `base`'s own shape-macro chain
/// (`elwindui_macros::class::build_props_macro`'s `@field_type` arm) at the *consumer's* expansion
/// time, the same "defer to rustc" trade-off `emit_external_construction`'s own doc comment already
/// accepts for every other external-base construction path. `view` is `None` for a component with
/// no own view at all (pure template/shape composition) — there is no bare reference to read in
/// that case, so this can only return `c`'s own literal fields, same as it always could.
fn synthesize_external_base_fields(
    c: &ComponentDef,
    base: &str,
    base_path: Option<&str>,
    view: Option<&ViewDef>,
) -> Vec<FieldDef> {
    let Some(view) = view else {
        return c.fields.clone();
    };
    // Issue #162: a hidden Component lowered from a `ViewExpr::DeferredView`
    // (`ViewDef::implicit_owner`) already resolves every otherwise-unresolved bare name through its
    // own `__view_owner` weak-owner fallback (`emit_expr`'s own `ViewExpr::Path` handling) — a bare
    // name used as a nested Component's own attribute value there (`DeferredPopupProbe { vm: vm,
    // log: log }`) is a forwarded reference to the *lexically enclosing* Component's field, not an
    // unresolved reference to `base`'s (`ContentControl`'s) own field needing synthesis here. Only
    // relevant in a real (non-`elwindui-codegen`-internal-test) compilation, where every real
    // `#[elwindui::component(inherits ContentControl)]` composes over an external, `TypeInfo`-less
    // base (`resolve_composed_shape`'s own doc comment) and so always reaches this function.
    // A named `#[control_template]` instance resolves `templated_parent.*` through its
    // explicit weak owner field.  Those paths are not bare references to the external
    // ContentControl base and must never be synthesized as ad-hoc base properties (for example,
    // `templated_parent.label` must not create a `label: ()` constructor slot on the hidden
    // ContentControl instance).  The generated template instance carries the same explicit
    // owner contract as a deferred view, but without `implicit_owner` because its owner is typed
    // and named in the synthesized struct itself.
    if view.implicit_owner.is_some() || view.template_instance {
        return c.fields.clone();
    }
    let own_names: HashSet<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
    let bound_names = collect_locally_bound_names(view);
    let mut bare_names: Vec<String> = collect_bare_attribute_value_names(view)
        .into_iter()
        .filter(|name| !own_names.contains(name.as_str()) && !bound_names.contains(name.as_str()))
        .collect();
    // Deterministic output order — `collect_bare_attribute_value_names` returns a `HashSet`, whose
    // iteration order must not leak into generated constructor-parameter order.
    bare_names.sort();
    // `base` is only the final symbol-table segment.  Preserve `base_path` when the author wrote
    // an external/local qualified inherits path so the deferred type query reaches that defining
    // crate's shape macro root instead of falling back to the builtin crate.
    let props_macro = dsl_props_macro_path(base_path.unwrap_or(base), None).to_string();
    let mut result = c.fields.clone();
    result.extend(bare_names.into_iter().map(|name| FieldDef {
        ty: format!("{props_macro}!(@field_type {name})"),
        name,
        kind: FieldKind::Param,
        attrs: Vec::new(),
        initializer: None,
    }));
    result
}

/// Every name a `view` binds locally — `let`-binding names, `for`-loop item bindings, and every
/// identifier-shaped token appearing in a `match` arm's pattern text (`MatchArm::pattern` is plain
/// source text, not a parsed `syn::Pat`, so this can't distinguish an actual binding from an enum
/// path segment inside it — treating every token as bound is the safe direction). Flat and scope-
/// independent (a `for`-loop's own `binding` is really only shadowed within its own `body`, not the
/// whole view) rather than lexically precise, because over-excluding here is harmless —
/// `synthesize_external_base_fields` just leaves that name unsynthesized, falling back to
/// `resolve_effective_fields`'s pre-existing "not found" behavior, no worse than before this
/// function existed — while under-excluding is a real regression: a loop/match-bound identifier
/// misread as a forwarded external-base field reference (Refs #90's own regression this guards
/// against: `for doc in vm.documents { DocumentView { doc: doc } }` bare-referencing the loop's own
/// `doc` binding, not any inherited field of `Window`).
fn collect_locally_bound_names(view: &ViewDef) -> HashSet<String> {
    let mut bound = HashSet::new();
    for l in &view.lets {
        bound.insert(l.name.clone());
        collect_locally_bound_names_in_element(&l.element, &mut bound);
    }
    for attribute in &view.root.attributes {
        collect_locally_bound_names_in_view_expr(&attribute.value, &mut bound);
    }
    for child in &view.root.children {
        collect_locally_bound_names_in_child(child, &mut bound);
    }
    bound
}

fn collect_locally_bound_names_in_element(node: &ElementNode, bound: &mut HashSet<String>) {
    for attribute in &node.attributes {
        collect_locally_bound_names_in_view_expr(&attribute.value, bound);
    }
    for child in &node.children {
        collect_locally_bound_names_in_child(child, bound);
    }
}

// Mirrors `collect_view_expr_bare_names`'s exact traversal shape (same reachable `ElementNode`/
// `ChildEntry` set) — a bound name hiding inside an element-valued attribute (`content: Grid { for
// doc in .. { .. } }`) or a closure body (`render_content: |item| Card { title: item }`) must be
// found here too, or `synthesize_external_base_fields`'s exclusion filter misses it exactly the way
// it missed `NotepadWindow`'s `content: Grid { .. TabView { for doc in vm.documents { .. } .. } }`
// during development of this function (`doc` lives inside an attribute value, not `view.root`'s own
// `children`, so an earlier version that only walked `children` never saw it).
fn collect_locally_bound_names_in_view_expr(expr: &ViewExpr, bound: &mut HashSet<String>) {
    match expr {
        ViewExpr::Element(elem) => collect_locally_bound_names_in_element(elem, bound),
        ViewExpr::Closure { params, body } => {
            bound.extend(params.iter().cloned());
            match body {
                ClosureBody::Element(elem) => collect_locally_bound_names_in_element(elem, bound),
                ClosureBody::Expr(_) | ClosureBody::Block(_) => {}
            }
        }
        ViewExpr::TFluent(_, args) => {
            for (_, v) in args {
                collect_locally_bound_names_in_view_expr(v, bound);
            }
        }
        // A deferred view is lowered to its own synthetic hidden Component with its own,
        // independent `lets`/closure-param scope (`lib.rs`'s lowering pass) — it introduces no
        // locally-bound name into the *enclosing* view's own scope, so there is nothing to walk
        // here (Issue #162 §3.9's dependency-boundary rule applies to local-name collection too).
        ViewExpr::Path(_) | ViewExpr::Expr(_) | ViewExpr::DeferredView(_) => {}
    }
}

fn collect_locally_bound_names_in_child(child: &ChildEntry, bound: &mut HashSet<String>) {
    match child {
        ChildEntry::Literal(element) => collect_locally_bound_names_in_element(element, bound),
        ChildEntry::Ref(_) => {}
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_locally_bound_names_in_view_expr(condition, bound);
            for c in then_branch {
                collect_locally_bound_names_in_child(c, bound);
            }
            for c in else_branch {
                collect_locally_bound_names_in_child(c, bound);
            }
        }
        ChildEntry::Match { value, arms } => {
            collect_locally_bound_names_in_view_expr(value, bound);
            for arm in arms {
                for token in arm
                    .pattern
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                {
                    if !token.is_empty() {
                        bound.insert(token.to_string());
                    }
                }
                for c in &arm.body {
                    collect_locally_bound_names_in_child(c, bound);
                }
            }
        }
        ChildEntry::For {
            binding,
            collection,
            body,
        } => {
            collect_locally_bound_names_in_view_expr(collection, bound);
            bound.insert(binding.clone());
            for c in body {
                collect_locally_bound_names_in_child(c, bound);
            }
        }
    }
}

/// Collects every 1-segment bare `ViewExpr::Path` name referenced as an *attribute value* anywhere
/// in `view`'s element tree — same traversal `view_references_bare_name` checks membership against,
/// gathering candidates instead. Deliberately does not collect a bare `ChildEntry::Ref` (`Control {
/// content }`): that is a *child*-position forward, already resolved independently of `TypeInfo` by
/// `generate_view`'s `PASSTHROUGH_NODE`/`lets_map` seeding, not an attribute value this function's
/// only caller (`synthesize_external_base_fields`) needs to recover a type for. Does not itself
/// exclude a locally bound name (`collect_locally_bound_names`) — callers combine the two.
fn collect_bare_attribute_value_names(view: &ViewDef) -> HashSet<String> {
    let mut names = HashSet::new();
    for l in &view.lets {
        collect_element_bare_names(&l.element, &mut names);
    }
    for attribute in &view.root.attributes {
        collect_view_expr_bare_names(&attribute.value, &mut names);
    }
    for child in &view.root.children {
        collect_child_bare_names(child, &mut names);
    }
    names
}

fn collect_element_bare_names(node: &ElementNode, names: &mut HashSet<String>) {
    for attribute in &node.attributes {
        collect_view_expr_bare_names(&attribute.value, names);
    }
    for child in &node.children {
        collect_child_bare_names(child, names);
    }
}

fn collect_child_bare_names(child: &ChildEntry, names: &mut HashSet<String>) {
    match child {
        ChildEntry::Literal(element) => collect_element_bare_names(element, names),
        ChildEntry::Ref(_) => {}
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_view_expr_bare_names(condition, names);
            for c in then_branch {
                collect_child_bare_names(c, names);
            }
            for c in else_branch {
                collect_child_bare_names(c, names);
            }
        }
        ChildEntry::Match { value, arms } => {
            collect_view_expr_bare_names(value, names);
            for arm in arms {
                for c in &arm.body {
                    collect_child_bare_names(c, names);
                }
            }
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            collect_view_expr_bare_names(collection, names);
            for c in body {
                collect_child_bare_names(c, names);
            }
        }
    }
}

fn collect_view_expr_bare_names(expr: &ViewExpr, names: &mut HashSet<String>) {
    match expr {
        ViewExpr::Path(path) => {
            if let [only] = path.as_slice() {
                names.insert(only.clone());
            }
        }
        ViewExpr::Element(elem) => collect_element_bare_names(elem, names),
        ViewExpr::Closure {
            body: ClosureBody::Element(elem),
            ..
        } => collect_element_bare_names(elem, names),
        ViewExpr::TFluent(_, args) => {
            for (_, v) in args {
                collect_view_expr_bare_names(v, names);
            }
        }
        // A deferred view's own inner bare names are not this outer view's dependencies — Issue
        // #162 §3.9: outer dependency scans must not recurse into `ViewExpr::DeferredView`.
        ViewExpr::Expr(_)
        | ViewExpr::DeferredView(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_) | ClosureBody::Block(_),
            ..
        } => {}
    }
}

/// `field_name -> declaring component name`, for every field `resolve_effective_fields(from, c,
/// modules)` would return — same recursion (same `inherits`-chain walk, same `#[routed]`/common-
/// field/bare-reference exemption filter for a `has_view` component's own base), but tracking
/// *which* component's own `ComponentDef::fields` literally declares each name rather than the
/// `FieldDef` itself. See `TypeInfo::declaring_types`'s own doc comment for why this needs to be
/// tracked separately (the flattened field list alone can't answer "who declared this").
fn resolve_field_declaring_types(
    from: &Module,
    c: &ComponentDef,
    modules: &[Module],
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    if let Some(base) = c.base.as_deref() {
        if let Some((base_module, base_c)) = find_component_and_module(from, base, modules) {
            let base_declaring = resolve_field_declaring_types(base_module, base_c, modules);
            // Mirrors `resolve_effective_fields`'s own exemption filter exactly: a `has_view`
            // component only forwards its base's `#[routed]`/`UIElement`-common/bare-referenced
            // fields, everything else is dropped (never reachable on this component at all, so it
            // shouldn't appear in `declaring_types` either).
            match find_view(from, &c.name) {
                Some(view) => {
                    let common_fields: HashSet<&str> =
                        find_component_and_module(from, "UIElement", modules)
                            .map(|(_, ui)| ui.fields.iter().map(|f| f.name.as_str()).collect())
                            .unwrap_or_default();
                    let base_fields: Vec<FieldDef> =
                        resolve_effective_fields(base_module, base_c, modules)
                            .into_iter()
                            .filter(|field| field.kind != FieldKind::State)
                            .collect();
                    let template_parent_fields = if view.is_template {
                        collect_template_parent_field_names(view)
                    } else {
                        HashSet::new()
                    };
                    let kept_names: HashSet<&str> = base_fields
                        .iter()
                        .filter(|f| {
                            f.attrs
                                .iter()
                                .any(|a| matches!(a, Attr::Routed | Attr::TextStyle))
                                || common_fields.contains(f.name.as_str())
                                || view_references_bare_name(view, &f.name)
                                || template_parent_fields.contains(&f.name)
                        })
                        .map(|f| f.name.as_str())
                        .collect();
                    result.extend(
                        base_declaring
                            .into_iter()
                            .filter(|(name, _)| kept_names.contains(name.as_str())),
                    );
                }
                None => result.extend(base_declaring),
            }
        } else if let Some(view) = find_view(from, &c.name) {
            // `synthesize_external_base_fields`'s own counterpart: a name recovered only from the
            // view's own bare attribute-value reference (no local `ComponentDef` for `base` at all)
            // has no findable ancestor to attribute it to, so it counts as declared by `c` itself —
            // correctly so, not merely a fallback default: `emit_field_setter_call`'s UFCS
            // disambiguation only matters when some ancestor's own `#[class]`-generated `..Ext`
            // trait is also in scope providing the same setter, and no such local ancestor chain
            // exists here to collide with in the first place.
            let own_names: HashSet<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
            let bound_names = collect_locally_bound_names(view);
            result.extend(
                collect_bare_attribute_value_names(view)
                    .into_iter()
                    .filter(|name| {
                        !own_names.contains(name.as_str()) && !bound_names.contains(name.as_str())
                    })
                    .map(|name| (name, c.name.clone())),
            );
        }
    }
    for f in &c.fields {
        result.insert(f.name.clone(), c.name.clone());
    }
    result
}

/// Whether `view`'s element tree references `name` as a *bare* value anywhere — a 1-segment
/// `ViewExpr::Path` (`padding: padding`) or a bare `ChildEntry::Ref` (`Control { content }`) — as
/// opposed to a literal/computed value (`fill: "#3a3a3c"`) or no mention at all. See
/// `resolve_effective_fields`'s doc comment.
fn view_references_bare_name(view: &ViewDef, name: &str) -> bool {
    view.lets
        .iter()
        .any(|l| element_references_bare_name(&l.element, name))
        || view
            .root
            .attributes
            .iter()
            .any(|attribute| view_expr_references_bare_name(&attribute.value, name))
        || view
            .root
            .children
            .iter()
            .any(|child| child_references_bare_name(child, name))
}

fn element_references_bare_name(node: &ElementNode, name: &str) -> bool {
    if node
        .attributes
        .iter()
        .any(|attribute| view_expr_references_bare_name(&attribute.value, name))
    {
        return true;
    }
    node.children
        .iter()
        .any(|child| child_references_bare_name(child, name))
}

fn child_references_bare_name(child: &ChildEntry, name: &str) -> bool {
    match child {
        ChildEntry::Literal(element) => element_references_bare_name(element, name),
        ChildEntry::Ref(binding) => binding == name,
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            view_expr_references_bare_name(condition, name)
                || then_branch
                    .iter()
                    .any(|child| child_references_bare_name(child, name))
                || else_branch
                    .iter()
                    .any(|child| child_references_bare_name(child, name))
        }
        ChildEntry::Match { value, arms } => {
            view_expr_references_bare_name(value, name)
                || arms.iter().any(|arm| {
                    arm.body
                        .iter()
                        .any(|child| child_references_bare_name(child, name))
                })
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            view_expr_references_bare_name(collection, name)
                || body
                    .iter()
                    .any(|child| child_references_bare_name(child, name))
        }
    }
}

fn view_expr_references_bare_name(expr: &ViewExpr, name: &str) -> bool {
    match expr {
        ViewExpr::Path(path) => path.len() == 1 && path[0] == name,
        ViewExpr::Element(elem) => element_references_bare_name(elem, name),
        ViewExpr::Closure {
            body: ClosureBody::Element(elem),
            ..
        } => element_references_bare_name(elem, name),
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, v)| view_expr_references_bare_name(v, name)),
        // Issue #162 §3.9: a deferred view is a dependency boundary, not a bare forward of `name`.
        ViewExpr::Expr(_)
        | ViewExpr::DeferredView(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_) | ClosureBody::Block(_),
            ..
        } => false,
    }
}

/// Collects property names explicitly read or written through a template's typed parent.  These
/// references are different from ordinary bare forwards (`padding: padding`): they address the
/// inherited component surface through `templated_parent`, so an inherited field must remain in
/// the effective metadata even when a component's own `template_view!` never forwards it into the
/// composed base constructor.  The same traversal covers DSL paths and raw Rust expressions so
/// the generated `TemplateProperty` bridge has one complete set of inherited fields.
fn collect_template_parent_field_names(view: &ViewDef) -> HashSet<String> {
    let mut names = HashSet::new();
    for binding in &view.lets {
        collect_template_parent_field_names_in_element(&binding.element, &mut names);
    }
    for attribute in &view.root.attributes {
        collect_template_parent_field_names_in_expr(&attribute.value, &mut names);
    }
    for (_, _, value) in &view.root.attached {
        collect_template_parent_field_names_in_expr(value, &mut names);
    }
    for child in &view.root.children {
        collect_template_parent_field_names_in_child(child, &mut names);
    }
    names
}

fn collect_template_parent_field_names_in_element(
    element: &ElementNode,
    names: &mut HashSet<String>,
) {
    for attribute in &element.attributes {
        collect_template_parent_field_names_in_expr(&attribute.value, names);
    }
    for (_, _, value) in &element.attached {
        collect_template_parent_field_names_in_expr(value, names);
    }
    for child in &element.children {
        collect_template_parent_field_names_in_child(child, names);
    }
}

fn collect_template_parent_field_names_in_child(child: &ChildEntry, names: &mut HashSet<String>) {
    match child {
        ChildEntry::Literal(element) => {
            collect_template_parent_field_names_in_element(element, names)
        }
        ChildEntry::Ref(_) => {}
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_template_parent_field_names_in_expr(condition, names);
            for child in then_branch.iter().chain(else_branch) {
                collect_template_parent_field_names_in_child(child, names);
            }
        }
        ChildEntry::Match { value, arms } => {
            collect_template_parent_field_names_in_expr(value, names);
            for arm in arms {
                for child in &arm.body {
                    collect_template_parent_field_names_in_child(child, names);
                }
            }
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            collect_template_parent_field_names_in_expr(collection, names);
            for child in body {
                collect_template_parent_field_names_in_child(child, names);
            }
        }
    }
}

fn collect_template_parent_field_names_in_expr(expr: &ViewExpr, names: &mut HashSet<String>) {
    match expr {
        ViewExpr::Path(path) if path.len() >= 2 && path[0] == "templated_parent" => {
            names.insert(path[1].clone());
        }
        ViewExpr::Path(_) => {}
        ViewExpr::TFluent(_, args) => {
            for (_, value) in args {
                collect_template_parent_field_names_in_expr(value, names);
            }
        }
        ViewExpr::Expr(expression) => {
            collect_template_parent_field_names_in_rust_expr(expression, names);
        }
        ViewExpr::Closure { body, .. } => match body {
            ClosureBody::Expr(expr) => collect_template_parent_field_names_in_expr(expr, names),
            ClosureBody::Element(element) => {
                collect_template_parent_field_names_in_element(element, names)
            }
            ClosureBody::Block(block) => {
                collect_template_parent_field_names_in_rust_block(block, names)
            }
        },
        ViewExpr::Element(element) => {
            collect_template_parent_field_names_in_element(element, names)
        }
        // A deferred view is lowered with its own lexical owner and property bridge.  Its
        // `templated_parent` (if any) is not this template's parent and must not be attributed to
        // the enclosing component's effective field list.
        ViewExpr::DeferredView(_) => {}
    }
}

fn collect_template_parent_field_names_in_rust_expr(expr: &syn::Expr, names: &mut HashSet<String>) {
    struct Collector<'a> {
        names: &'a mut HashSet<String>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_expr_field(&mut self, node: &'ast syn::ExprField) {
            if let syn::Expr::Path(base) = node.base.as_ref()
                && base.path.segments.len() == 1
                && base.path.segments[0].ident == "templated_parent"
                && let syn::Member::Named(field) = &node.member
            {
                self.names.insert(field.to_string());
            }
            syn::visit::visit_expr_field(self, node);
        }

        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if let syn::Expr::Path(receiver) = node.receiver.as_ref()
                && receiver.path.segments.len() == 1
                && receiver.path.segments[0].ident == "templated_parent"
            {
                let method = node.method.to_string();
                let field = method.strip_prefix("set_").unwrap_or(&method);
                self.names.insert(field.to_string());
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    Collector { names }.visit_expr(expr);
}

fn collect_template_parent_field_names_in_rust_block(
    block: &syn::Block,
    names: &mut HashSet<String>,
) {
    struct Collector<'a> {
        names: &'a mut HashSet<String>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_expr_field(&mut self, node: &'ast syn::ExprField) {
            if let syn::Expr::Path(base) = node.base.as_ref()
                && base.path.segments.len() == 1
                && base.path.segments[0].ident == "templated_parent"
                && let syn::Member::Named(field) = &node.member
            {
                self.names.insert(field.to_string());
            }
            syn::visit::visit_expr_field(self, node);
        }

        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if let syn::Expr::Path(receiver) = node.receiver.as_ref()
                && receiver.path.segments.len() == 1
                && receiver.path.segments[0].ident == "templated_parent"
            {
                let method = node.method.to_string();
                let field = method.strip_prefix("set_").unwrap_or(&method);
                self.names.insert(field.to_string());
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    Collector { names }.visit_block(block);
}

/// Whether `view`'s element tree references `name` *anywhere at all* — broader than
/// `view_references_bare_name`'s own notion (a *literal* same-name forward, `padding: padding`):
/// this also counts `name` appearing as a sub-expression identifier within a larger computed value
/// (e.g. `Rectangle`'s own `kind: ShapeKind::RoundedRect { corner_radius: corner_radius.unwrap_or
/// (0.0) }` — `corner_radius` is not a *bare* forward there, but its value is still read eagerly,
/// before `Self` exists). Used exclusively to decide whether a field's value is needed at
/// construction time (docs/design/runtime/ui_tree_design.md's post-construction setter convention, Phase
/// 2's `is_deferred_field`/`generate_view`'s `is_deferred_own_field`) — deliberately *not* used by
/// `resolve_effective_fields`'s own inherited-field-forwarding decision, which specifically wants
/// the narrower "literal forward" notion (a field only *contributing* to some other computed value
/// isn't being forwarded unchanged, so shouldn't be silently treated as inherited).
pub(crate) fn view_references_name_anywhere(view: &ViewDef, name: &str) -> bool {
    view.lets
        .iter()
        .any(|l| element_references_name_anywhere(&l.element, name))
        || view
            .root
            .attributes
            .iter()
            .any(|attribute| view_expr_references_name_anywhere(&attribute.value, name))
        || view
            .root
            .attached
            .iter()
            .any(|(_, _, expr)| view_expr_references_name_anywhere(expr, name))
        || view
            .root
            .children
            .iter()
            .any(|child| child_references_name_anywhere(child, name))
}

fn element_references_name_anywhere(node: &ElementNode, name: &str) -> bool {
    if node
        .attributes
        .iter()
        .any(|attribute| view_expr_references_name_anywhere(&attribute.value, name))
    {
        return true;
    }
    if node
        .attached
        .iter()
        .any(|(_, _, expr)| view_expr_references_name_anywhere(expr, name))
    {
        return true;
    }
    node.children
        .iter()
        .any(|child| child_references_name_anywhere(child, name))
}

fn child_references_name_anywhere(child: &ChildEntry, name: &str) -> bool {
    match child {
        ChildEntry::Literal(element) => element_references_name_anywhere(element, name),
        ChildEntry::Ref(binding) => binding == name,
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            view_expr_references_name_anywhere(condition, name)
                || then_branch
                    .iter()
                    .any(|child| child_references_name_anywhere(child, name))
                || else_branch
                    .iter()
                    .any(|child| child_references_name_anywhere(child, name))
        }
        ChildEntry::Match { value, arms } => {
            view_expr_references_name_anywhere(value, name)
                || arms.iter().any(|arm| {
                    arm.body
                        .iter()
                        .any(|child| child_references_name_anywhere(child, name))
                })
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            view_expr_references_name_anywhere(collection, name)
                || body
                    .iter()
                    .any(|child| child_references_name_anywhere(child, name))
        }
    }
}

fn view_expr_references_name_anywhere(expr: &ViewExpr, name: &str) -> bool {
    match expr {
        ViewExpr::Path(path) => path.iter().any(|seg| seg == name),
        ViewExpr::Element(elem) => element_references_name_anywhere(elem, name),
        ViewExpr::Closure {
            body: ClosureBody::Element(elem),
            ..
        } => element_references_name_anywhere(elem, name),
        ViewExpr::Closure {
            body: ClosureBody::Expr(e),
            ..
        } => view_expr_references_name_anywhere(e, name),
        ViewExpr::Closure {
            body: ClosureBody::Block(block),
            ..
        } => block_references_ident(block, name),
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, v)| view_expr_references_name_anywhere(v, name)),
        ViewExpr::Expr(e) => expr_references_ident(e, name),
        // A bare outer name read inside a deferred view is not read eagerly, at the enclosing
        // component's own construction time — it is read later, through the generated hidden
        // Component's `__view_owner` weak upgrade, at popup-open (build) time. So it must not force
        // `is_deferred_field`'s own "referenced anywhere -> not deferrable" decision the way an
        // ordinary eager reference would (Issue #162 §3.9's dependency-boundary rule, applied here
        // to construction-time-need rather than resync).
        ViewExpr::DeferredView(_) => false,
    }
}

/// Whether the raw Rust expression `expr` references a bare identifier `name` anywhere within it
/// (e.g. `corner_radius` inside `corner_radius.unwrap_or(0.0)`) — a `syn::visit::Visit` walk over
/// every `syn::Expr::Path` node, since `ViewExpr::Expr` wraps an arbitrary parsed Rust expression
/// with no DSL-level structure of its own left to pattern-match on.
fn expr_references_ident(expr: &syn::Expr, name: &str) -> bool {
    struct Finder<'a> {
        name: &'a str,
        found: bool,
    }
    impl<'a> syn::visit::Visit<'a> for Finder<'a> {
        fn visit_expr_path(&mut self, node: &'a syn::ExprPath) {
            if node.path.segments.len() == 1 && node.path.segments[0].ident == self.name {
                self.found = true;
            }
            syn::visit::visit_expr_path(self, node);
        }
    }
    let mut finder = Finder { name, found: false };
    syn::visit::Visit::visit_expr(&mut finder, expr);
    finder.found
}

/// [`expr_references_ident`]'s counterpart for a `ClosureBody::Block` (a multi-statement `on_*`
/// handler body) — same bare-identifier walk, over every statement instead of a single expression.
fn block_references_ident(block: &syn::Block, name: &str) -> bool {
    struct Finder<'a> {
        name: &'a str,
        found: bool,
    }
    impl<'a> syn::visit::Visit<'a> for Finder<'a> {
        fn visit_expr_path(&mut self, node: &'a syn::ExprPath) {
            if node.path.segments.len() == 1 && node.path.segments[0].ident == self.name {
                self.found = true;
            }
            syn::visit::visit_expr_path(self, node);
        }
    }
    let mut finder = Finder { name, found: false };
    syn::visit::Visit::visit_block(&mut finder, block);
    finder.found
}

/// Recursively flattens `c`'s effective method list: its base's own effective methods (an
/// `#[override]`n one is kept alongside under a mangled `__base_<name>` so the override's body can
/// still reach it via `base::name(...)`, rewritten by `rewrite_base_calls`), followed by `c`'s own
/// methods (an override's body rewritten the same way). See `ComponentDef`'s doc comment. Only one
/// `inherits` hop's worth of `base::` chaining is guaranteed correct — see `generate_view`'s doc
/// comment on `own_on_mount`/`own_on_unmount` for the same limitation applied to lifecycle hooks.
/// Resolves the `#[content(field_name)]` slot a component's bare nested children go into: its own
/// declaration if it has one, otherwise its base's (recursively).
///
/// `#[content(..)]` is not *inherited as an attribute* — each builtin declares its own, on purpose
/// (see `docs/specs/ui_spec.md`). But a user component that composes over a base which has
/// one still needs the slot resolvable, because the composition itself puts the base's own view root
/// there: `Derived inherits Base` whose view root literally constructs `Base` plans `Base`'s view
/// root as a bare child of the `Base` node. Without walking the chain, `build_component_args`
/// rejects that child as having nowhere to go, even though the base does declare a destination —
/// which made `inherits <user component>` unusable whenever the base composed over something with a
/// content slot (e.g. `ContentControl`).
fn resolve_content_field(module: &Module, c: &ComponentDef, modules: &[Module]) -> Option<String> {
    if let Some(own) = &c.content_field {
        return Some(own.clone());
    }
    let base = c.base.as_deref()?;
    if base == "NativeControl" {
        return None;
    }
    let (base_module, base_c) = find_component_and_module(module, base, modules)?;
    resolve_content_field(base_module, base_c, modules)
}

pub(crate) fn resolve_effective_methods<'m>(
    from: &'m Module,
    c: &ComponentDef,
    modules: &'m [Module],
) -> Vec<MethodDef> {
    let mut result = Vec::new();
    if let Some(base) = c.base.as_deref() {
        if base != "NativeControl" {
            if let Some((base_module, base_c)) = find_component_and_module(from, base, modules) {
                let base_methods = resolve_effective_methods(base_module, base_c, modules);
                let overridden: HashSet<&str> = c
                    .methods
                    .iter()
                    .filter(|m| m.is_override)
                    .map(|m| m.name.as_str())
                    .collect();
                for bm in base_methods {
                    if overridden.contains(bm.name.as_str()) {
                        // Keep the base body reachable as a private `__base_<name>` shadow (what a
                        // `base::<name>(..)` call is rewritten onto), but do *not* also keep the
                        // original under its own name: `c`'s own override is appended below under
                        // that exact name, and two inherent methods with one name don't compile.
                        let mut shadow = bm.clone();
                        shadow.name = format!("__base_{}", bm.name);
                        shadow.is_virtual = false;
                        shadow.is_override = false;
                        result.push(shadow);
                        continue;
                    }
                    result.push(bm);
                }
            }
        }
    }
    for m in &c.methods {
        let mut m = m.clone();
        if m.is_override {
            m.body = rewrite_base_calls(m.body, &format_ident!("self"));
        }
        result.push(m);
    }
    result
}

/// Resolves `c`'s effective ordinary component view: its own literal `view` if it wrote one,
/// otherwise its base's effective `view` (recursively), retargeted to `c.name`. A base component's
/// typed `template: template_view!` declaration is deliberately not copied into a derived target:
/// doing so would synthesize an unsound `ControlTemplate<Derived>` and violate exact-type lookup.
/// Returns `None` when there's no ordinary view anywhere in the chain — a plain data component, or
/// one inheriting a primitive shape family with no `view` of its own (`Control`/`Rectangle`; those
/// still require an explicit hand-written `view` — see `validate::validate_inherits`).
pub(crate) fn resolve_view_for<'m>(
    from: &'m Module,
    c: &ComponentDef,
    modules: &'m [Module],
) -> Option<ViewDef> {
    if let Some(own) = find_view(from, &c.name) {
        return Some(own.clone());
    }
    let base = c.base.as_deref()?;
    if base == "NativeControl" {
        return None;
    }
    let (base_module, base_c) = find_component_and_module(from, base, modules)?;
    let base_view = resolve_view_for(base_module, base_c, modules)?;
    if base_view.is_template {
        return None;
    }
    Some(ViewDef {
        target: c.name.clone(),
        is_template: false,
        template_instance: false,
        ..base_view
    })
}

/// Resolves the concrete `ElementNode` a `view`'s body (`ast::ViewBody`) actually constructs.
/// `is_composed` is whatever the caller already knows from `TypeInfo` (`composed_shape.is_some() ||
/// host_composition_base.is_some()` — see `generate_view`'s own `is_composed` and its call site,
/// and `validate.rs`'s main loop) — deliberately *not* re-derived here from `base`'s name alone,
/// since composability depends on `base`'s own recursively-resolved shape (`resolve_composed_shape`/
/// `resolve_host_composition_base`), not just whether it's one of the three base-less category tags.
///
/// `is_composed`: the body *is* `base`'s own attributes/children directly — Phase 0's
/// implicit-composition sugar, no wrapper element written (docs/design/runtime/ui_tree_design.md).
/// `!is_composed`: an ordinary (non-composing) component's `view`, which may only contain exactly
/// one literal child — that child is the root.
pub(crate) fn resolve_view_root_element(
    body: &ViewBody,
    base: Option<&str>,
    is_composed: bool,
) -> Option<ElementNode> {
    if is_composed {
        return Some(ElementNode {
            type_path: base.expect("is_composed implies a base").to_string(),
            attributes: body.attributes.clone(),
            attached: body.attached.clone(),
            attribute_shortcuts: body.attribute_shortcuts.clone(),
            children: body.children.clone(),
        });
    }
    match body.children.as_slice() {
        [ChildEntry::Literal(elem)]
            if body.attributes.is_empty()
                && body.attached.is_empty()
                && body.attribute_shortcuts.is_empty() =>
        {
            Some(elem.clone())
        }
        _ => None,
    }
}

/// `component_meta`-building-time (i.e. before any `TypeInfo` exists) approximation of "is `base`
/// composable" — mirrors `resolve_composed_shape`/`resolve_host_composition_base`'s own conditions
/// but computed purely from each component's own locally-declared `ComponentDef` flags (`embedded`/
/// `native`/`base`) plus whether it `find_view`s, recursing the same way `resolve_composed_shape`
/// does, since no cross-module `SymbolTable` is available yet at this point in `build_symbol_table`.
fn base_is_composable_early(from: &Module, base: &str, modules: &[Module]) -> bool {
    if base == "NativeControl" {
        return false;
    }
    let Some((base_module, base_c)) = find_component_and_module(from, base, modules) else {
        return false;
    };
    let base_has_view = find_view(base_module, &base_c.name).is_some();
    let base_is_virtual_builtin = base_c.embedded
        && !base_has_view
        && base_c.base.as_deref() != Some("NativeControl")
        && !base_c.native;
    if base_is_virtual_builtin {
        return true;
    }
    // Hand-written native host with no `view` of its own (`Window`-like, "host composition") —
    // `#[native]` components are validated to declare no `base`, so this never overlaps with the
    // `NativeControl`-leaf case above.
    if base_c.native && !base_has_view {
        return true;
    }
    if base_has_view {
        return match base_c.base.as_deref() {
            Some(grandparent) => base_is_composable_early(base_module, grandparent, modules),
            None => false,
        };
    }
    false
}

/// Lenient, `component_meta`-building-time counterpart of `resolve_view_root_element`: resolves
/// just the effective root's *type name* (not a full `ElementNode`) so `resolve_is_native` can
/// recurse into that type's own nativeness. Returns `None` for a malformed body (no `view` anywhere
/// in the chain, or a non-composing body that doesn't reduce to exactly one literal child) —
/// `validate::validate` reports that case with a real error message; this function only needs *a*
/// reasonable answer for native/virtual inference, not a diagnostic.
fn resolve_effective_root_type(
    from: &Module,
    c: &ComponentDef,
    modules: &[Module],
) -> Option<String> {
    if let Some(base) = c.base.as_deref() {
        if base_is_composable_early(from, base, modules) {
            // The wrapper is always the *composing* component's own immediate base, regardless of
            // whether that component wrote its own `view` or inherited one as a template — see
            // `resolve_view_root_element`'s doc comment.
            return Some(base.to_string());
        }
    }
    if let Some(own) = find_view(from, &c.name) {
        return match own.root.children.as_slice() {
            [ChildEntry::Literal(elem)]
                if own.root.attributes.is_empty() && own.root.attached.is_empty() =>
            {
                Some(elem.type_path.clone())
            }
            _ => None,
        };
    }
    let base = c.base.as_deref()?;
    if base == "NativeControl" {
        return None;
    }
    let (base_module, base_c) = find_component_and_module(from, base, modules)?;
    resolve_effective_root_type(base_module, base_c, modules)
}

/// Rewrites `base::name(args)` — a method/`#[computed]`-initializer/`on_mount`/`on_unmount` body's
/// call into its immediate base's implementation of the same name (§3) — to `#receiver.__base_name
/// (args)`, the shadow copy `resolve_effective_methods`/`generate_view` emit alongside an
/// `#[override]`. Structurally identical to `rewrite_field_refs`'s own `VisitMut` idiom.
fn rewrite_base_calls(mut block: syn::Block, receiver: &syn::Ident) -> syn::Block {
    struct Rewriter<'a> {
        receiver: &'a syn::Ident,
    }
    impl VisitMut for Rewriter<'_> {
        fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = &*call.func {
                    let segs: Vec<String> = p
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect();
                    if let [base_seg, name] = segs.as_slice() {
                        if base_seg == "base" {
                            let receiver = self.receiver;
                            let method = format_ident!("__base_{}", name);
                            let args = &call.args;
                            *node = syn::parse_quote! { #receiver.#method(#args) };
                            return;
                        }
                    }
                }
            }
            syn::visit_mut::visit_expr_mut(self, node);
        }

        /// A macro's arguments are an opaque `TokenStream` to `syn` — `visit_expr_mut` never
        /// reaches inside one, so `format!("{}!", base::label())` (the very shape
        /// `docs/specs/dsl_spec.md` §3 uses) would otherwise keep an unresolvable `base::label()`.
        /// Rewrite at the token level instead: `base :: name ( args )` -> `receiver . __base_name (
        /// args )`, recursing into every nested group so it works at any depth.
        fn visit_macro_mut(&mut self, node: &mut syn::Macro) {
            node.tokens = rewrite_base_calls_in_tokens(node.tokens.clone(), self.receiver);
            syn::visit_mut::visit_macro_mut(self, node);
        }
    }
    let mut rewriter = Rewriter { receiver };
    rewriter.visit_block_mut(&mut block);
    block
}

/// Token-level counterpart of `rewrite_base_calls`' `syn` visitor, for macro argument streams —
/// see `Rewriter::visit_macro_mut`. Matches the four-token prefix `base`, `::`, `<name>`,
/// `(<args>)` and rewrites it to `<receiver> . __base_<name> (<args>)`, recursing into the
/// argument group and into any other group it passes over.
fn rewrite_base_calls_in_tokens(
    tokens: proc_macro2::TokenStream,
    receiver: &syn::Ident,
) -> proc_macro2::TokenStream {
    use proc_macro2::{Delimiter, Group, TokenTree};
    let flat: Vec<TokenTree> = tokens.into_iter().collect();
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < flat.len() {
        // `base` `::` `name` `( .. )`
        if let (
            Some(TokenTree::Ident(base_ident)),
            Some(TokenTree::Punct(colon1)),
            Some(TokenTree::Punct(colon2)),
            Some(TokenTree::Ident(name)),
            Some(TokenTree::Group(args)),
        ) = (
            flat.get(i),
            flat.get(i + 1),
            flat.get(i + 2),
            flat.get(i + 3),
            flat.get(i + 4),
        ) {
            if base_ident == "base"
                && colon1.as_char() == ':'
                && colon2.as_char() == ':'
                && args.delimiter() == Delimiter::Parenthesis
            {
                let method = format_ident!("__base_{}", name.to_string());
                let inner = rewrite_base_calls_in_tokens(args.stream(), receiver);
                out.extend(quote! { #receiver.#method(#inner) });
                i += 5;
                continue;
            }
        }
        match &flat[i] {
            TokenTree::Group(g) => {
                let inner = rewrite_base_calls_in_tokens(g.stream(), receiver);
                let mut replaced = Group::new(g.delimiter(), inner);
                replaced.set_span(g.span());
                out.extend(std::iter::once(TokenTree::Group(replaced)));
            }
            other => out.extend(std::iter::once(other.clone())),
        }
        i += 1;
    }
    out
}

/// Recursively resolves whether the component at `key` is native (see `TypeInfo::is_native`'s doc
/// comment). A component with its own `view` is *always* inferred from that view's root element's
/// own (recursively resolved) nativeness — `inherits` never overrides this for a view-having
/// component, it's only checked for consistency against it (`validate::validate_inherits`).
/// A component with **no** `view` of its own (a hand-written builtin, declared shape-only — see
/// `NativeControl`/`BUILTIN_SHAPE_SOURCE`) has no root to recurse through, so it falls
/// back to either its explicit `inherits NativeControl` declaration (`Button`/...) or its own
/// `#[native]` attribute (`Window` — a native leaf with no meaningful `inherits` base at all, see
/// `ComponentDef::native`'s doc comment): either present → native; both absent → virtual
/// (`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`).
fn resolve_is_native(
    key: &(Vec<String>, String),
    component_meta: &HashMap<(Vec<String>, String), (usize, Option<String>, Option<String>, bool)>,
    modules: &[Module],
    table: &SymbolTable,
    memo: &mut HashMap<(Vec<String>, String), bool>,
) -> bool {
    if let Some(&cached) = memo.get(key) {
        return cached;
    }
    // Guards against a cyclic `view` root reference (shouldn't occur in valid programs) recursing
    // forever — provisionally `false` while this key is being resolved.
    memo.insert(key.clone(), false);

    let is_native = match component_meta.get(key) {
        None => false,
        Some((module_index, base, view_root, native)) => {
            if let Some(root_name) = view_root {
                let from = &modules[*module_index];
                match table.resolve_key(from, root_name) {
                    Some(root_key) => {
                        resolve_is_native(&root_key, component_meta, modules, table, memo)
                    }
                    None => false,
                }
            } else {
                base.as_deref() == Some("NativeControl") || *native
            }
        }
    };

    memo.insert(key.clone(), is_native);
    is_native
}

/// Recursively resolves the virtual-builtin shape (if any) the component at `key` composes over via
/// a real `base: <Impl>` field — see `TypeInfo::composed_shape`'s doc comment and
/// `codegen::generate_view`'s `composed_shape`-driven branch.
fn resolve_composed_shape(
    key: &(Vec<String>, String),
    component_meta: &HashMap<(Vec<String>, String), (usize, Option<String>, Option<String>, bool)>,
    modules: &[Module],
    table: &SymbolTable,
    memo: &mut HashMap<(Vec<String>, String), Option<String>>,
) -> Option<String> {
    if let Some(cached) = memo.get(key) {
        return cached.clone();
    }
    // Guards against a cyclic `inherits` chain (shouldn't occur in valid programs) recursing
    // forever — provisionally `None` while this key is being resolved.
    memo.insert(key.clone(), None);

    let result = (|| {
        let (module_index, base, _view_root, _native) = component_meta.get(key)?;
        let base = base.as_deref()?;
        if base == "NativeControl" {
            return None;
        }
        let from = &modules[*module_index];

        if table
            .resolve(from, base)
            .is_some_and(|i| i.is_virtual_builtin)
        {
            // Direct shape composition against a hand-written `elwindui::core::ui` primitive
            // (`ContentControl inherits Control`): Phase 0's implicit-composition sugar means
            // there's no separate "own effective root literally constructs `base`" requirement to
            // check anymore (docs/design/runtime/ui_tree_design.md) — a composable `base` always
            // composes, and `generate_view`'s `resolve_view_root_element` supplies the missing
            // `Type { .. }` wrapper the view body no longer writes.
            return Some(base.to_string());
        }

        match table.resolve_key(from, base) {
            Some(base_key) => {
                // Direct composition against an *already-composed DSL component*, one delegation
                // hop further out (`RoundedPanel inherits ContentControl`) — the same shape as the
                // virtual-builtin case above, just one level up the chain. `generate_view`'s
                // `is_shape_composition`/`is_inherited_view_composition` don't otherwise care whether
                // `base` is a hand-written primitive or another composed DSL component, since both
                // always delegate through `self.base` regardless of that type's own nature — see
                // this function's own `has_own_view` split there, not here.
                resolve_composed_shape(&base_key, component_meta, modules, table, memo)
            }
            // External (no local `TypeInfo`, e.g. a builtin declared entirely in `elwindui-core`)
            // and not the one host-composition-eligible builtin (`resolve_host_composition_base`
            // handles `Window` instead): assume shape composition, the same treatment a virtual
            // builtin gets just above. Every real `#[elwindui::component(inherits ..)]` in this
            // codebase composes over either `Window` or `ContentControl` — there is no actual
            // "ordinary, single-root-element" usage to preserve a narrower default for — and a base
            // that turns out not to implement `UIElement` fails to compile on `#[class]`'s own
            // generated delegation (`self.base.as_ui_element()` etc.) instead of being caught here.
            None if base != "Window" => Some(base.to_string()),
            None => None,
        }
    })();

    memo.insert(key.clone(), result.clone());
    result
}

/// Resolves whether the component at `key` inherits a hand-written native host with no `UIElement`
/// implementation of its own ("host composition" — only `Window` qualifies today, see
/// `TypeInfo::host_composition_base`'s doc comment): `base` must resolve to a type that's
/// structurally native (`is_native_memo`, already fully resolved by the time this runs — see
/// `build_symbol_table`), has no `view`, and isn't itself a `NativeControl`-leaf (that combination
/// is unique to a hand-written host like `Window`; `Button`/`TextArea`/`TabView` all have
/// `is_native_control_leaf == true` and so are excluded, and `NativeControl`/virtual-builtin
/// category tags are excluded up front since they're `resolve_composed_shape`'s territory, not
/// this one's). Returns the base's DSL name alongside its resolved key (the pair `is_
/// host_composition_base` needs to mark the *base* side too — see `build_symbol_table`).
fn resolve_host_composition_base(
    key: &(Vec<String>, String),
    component_meta: &HashMap<(Vec<String>, String), (usize, Option<String>, Option<String>, bool)>,
    modules: &[Module],
    table: &SymbolTable,
    is_native_memo: &HashMap<(Vec<String>, String), bool>,
) -> Option<(String, (Vec<String>, String))> {
    let (module_index, base, _view_root, _native) = component_meta.get(key)?;
    let base = base.as_deref()?;
    let from = &modules[*module_index];
    if base == "NativeControl"
        || table
            .resolve(from, base)
            .is_some_and(|i| i.is_virtual_builtin)
    {
        return None;
    }
    match table.resolve_key(from, base) {
        Some(base_key) => {
            let base_info = table.types.get(&base_key)?;
            let base_is_native = is_native_memo.get(&base_key).copied().unwrap_or(false);
            if base_is_native && !base_info.has_view && !base_info.is_native_control_leaf {
                Some((base.to_string(), base_key))
            } else {
                None
            }
        }
        // External (no local `TypeInfo`): `Window` is the one builtin this ever actually
        // applies to (see this function's own doc comment — a hand-written native host with no
        // `UIElement` of its own), matching `generate_view`'s own pre-existing
        // `resolved_root.type_path == "Window"` fallback. The returned key is never a real
        // lookup target — nothing in this crate dereferences the base-key half of this pairing
        // (the now-removed `is_host_composition_base` field was the only reader, dead code even
        // before this).
        None if base == "Window" => Some((base.to_string(), (Vec::new(), "Window".to_string()))),
        None => None,
    }
}

pub fn generate_module(module: &Module, table: &SymbolTable) -> TokenStream {
    // A component with an effective `view` (its own, or inherited from its `inherits` base — see
    // `resolve_view_for`) is generated as a single struct+impl by `generate_view`, which also owns
    // the widget fields; one with no `view` anywhere in its chain falls back to
    // `generate_component`'s plain struct+accessors. Both are fed a *synthetic* `ComponentDef`
    // carrying `TypeInfo`'s already-flattened `effective_fields`/`effective_methods`, not the
    // literal (un-flattened) `ComponentDef` parsed from source — see `ComponentDef`'s doc comment.
    let mut out = TokenStream::new();
    for item in &module.items {
        out.extend(match item {
            Item::Enum(e) => generate_enum(e),
            Item::ViewModel(v) => generate_viewmodel(v, module, table),
            Item::Store(s) => generate_store(s, module, table),
            Item::Component(c) => {
                let info = table.resolve(module, &c.name).unwrap_or_else(|| {
                    panic!("component `{}` missing from its own symbol table", c.name)
                });
                // `#[abstract]` (docs/specs/dsl_spec.md 付録A): a pure category tag
                // (`UIElement`/`NativeControl`/`Layout`/`Shape`) never gets a `create_<snake
                // case>(..)`/`new(..)` of its own — `validate::check_element_value` already rejects
                // any DSL use site that would need one, so this is a second, codegen-level guarantee
                // that holds even if this function is ever called on unvalidated input.
                if info.is_abstract {
                    continue;
                }
                let synthetic = ComponentDef {
                    name: c.name.clone(),
                    base: c.base.clone(),
                    // Carried through unchanged: `generate_view`'s qualified-base-path helpers read
                    // this off the *synthetic* def they're actually passed, not the original `c`.
                    base_path: c.base_path.clone(),
                    fields: info.effective_fields.clone(),
                    methods: info.effective_methods.clone(),
                    // Irrelevant downstream: `generate_component`/`generate_view` never consult
                    // `embedded`/`sealed`/`native`/`is_abstract`/`text_style`/`content_field` (only
                    // `validate::validate`/`TypeInfo::sealed`/`TypeInfo::is_native`/
                    // `TypeInfo::is_abstract`/`TypeInfo::content_field`, all already checked/computed
                    // against the *original* `c`, do).
                    embedded: false,
                    sealed: false,
                    native: false,
                    is_abstract: false,
                    text_style: false,
                    content_field: None,
                };
                match &info.effective_view {
                    Some(view) => generate_view(view, c, &synthetic, module, table),
                    None => generate_component(c, &synthetic, table),
                }
            }
            // Always handled above, via the paired `Item::Component`'s effective view (own or
            // inherited) — see `resolve_view_for`.
            Item::View(_) => TokenStream::new(),
        });
    }

    out
}

fn generate_enum(e: &EnumDef) -> TokenStream {
    let name = format_ident!("{}", e.name);
    let variants: Vec<_> = e.variants.iter().map(|v| format_ident!("{}", v)).collect();
    quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #name {
            #(#variants),*
        }
    }
}

/// `file_name: String = "untitled.txt"`: a bare string literal initializer for a `String`-typed
/// field is `&str`, not `String` — append `.to_string()` so it type-checks.
fn coerce_to_owned_string(ty: &str, expr: syn::Expr) -> syn::Expr {
    if ty == "String" {
        if let syn::Expr::Lit(lit) = &expr {
            if matches!(lit.lit, syn::Lit::Str(_)) {
                return syn::parse_quote! { (#expr).to_string() };
            }
        }
    }
    expr
}

/// Copy-able field types get `Cell<T>`, everything else gets `RefCell<T>` (docs/design/runtime/state_management_design.md).
fn is_copy_type(ty: &str) -> bool {
    matches!(
        ty,
        "i32" | "i64" | "f32" | "f64" | "bool" | "u32" | "u64" | "usize"
    ) || {
        // A bare, capitalized single-*word* type (no generic `<..>`/`::` of its own — `Vec<T>`/
        // `Box<dyn Fn()>`/`Rc<T>` are never Copy no matter what's inside the brackets) that isn't a
        // known non-Copy std type is assumed to be one of this file's own enums (all generated with
        // `derive(Copy)`, see `generate_enum`).
        ty.chars().next().is_some_and(|c| c.is_uppercase())
            // These backend-independent graphics value types are intentionally Clone but not
            // Copy. Attribute-macro viewmodels commonly import them under these bare names, where
            // full type resolution is unavailable to this frontend.
            && !matches!(ty, "String" | "FontFamily" | "Brush" | "BrushStyle")
            && !ty.contains('<')
            && !ty.contains("::")
    }
}

/// `Vec<Document>` where `Document` is itself a known `component`/`viewmodel` in this compilation
/// unit: such a field needs `Rc`-wrapped elements (`Vec<Rc<Document>>`) rather than the generic
/// `is_copy_type`-driven wrapping, because cloning a plain `Vec<Document>` on every getter call
/// (as every other `#[observable]` field does) would clone each `Document`'s `Cell`/`RefCell`
/// fields into independent copies — mutating one through the getter's clone would silently not
/// persist. `Rc` cloning is cheap (a refcount bump) and every clone still refers to the same
/// shared `Document`, so e.g. a `TabView`'s per-tab `TextArea` edits reach the real stored
/// document. This is what lets a `viewmodel` hold a dynamic list of independently-reactive
/// sub-viewmodels (needed for notepad's real multi-document tabs) without a general nested-list
/// compiler feature; see docs/specs/ui_spec.md#tabs.
fn nested_vec_item_type(ty: &str, from: &Module, table: &SymbolTable) -> Option<String> {
    let inner = ty.strip_prefix("Vec<")?.strip_suffix(">")?.trim();
    // `resolve` only finds `inner` if it's locally defined in `from` or reachable through one of
    // `from`'s `use` declarations. The attribute-macro frontend (`attr_frontend.rs`) expands each
    // `#[elwindui::viewmodel] mod { ... }` in isolation — it has no way to see a *different* mod's
    // struct, so it always calls this with an empty table and relies entirely on the heuristic
    // below, same idea as `is_copy_type`'s "capitalized and not a known scalar" guess.
    let known = table.resolve(from, inner).is_some();
    let looks_nested = inner.chars().next().is_some_and(|c| c.is_uppercase()) && inner != "String";
    (known || looks_nested).then(|| inner.to_string())
}

/// Builds the token sequence a dependency's setter emits for one of its dependent fields, branching
/// on the dependent's own `FieldKind`: a `Computed` dependent recomputes synchronously inline and
/// notifies immediately (existing behavior); an `AsyncComputed` dependent only spawns a new
/// recompute (`__spawn_recompute_<dep>`) — its own `on_property_changed` fires later, inside the
/// spawned future, only if not superseded by a newer trigger before it resolves. Shared by both
/// `dependents_of` consumption sites (the scalar `Observable` setter and the `Vec<Rc<T>>` arm) so
/// they can't drift apart.
fn dependent_recompute_call(
    dep: &str,
    field_kind_by_name: &HashMap<&str, FieldKind>,
    property_enum: &syn::Ident,
) -> TokenStream {
    let property = format_ident!("{}", dep);
    match field_kind_by_name.get(dep) {
        Some(FieldKind::AsyncComputed) => {
            let spawn = format_ident!("__spawn_recompute_{}", dep);
            quote! { self.#spawn(); }
        }
        _ => {
            let recompute = format_ident!("recompute_{}", dep);
            quote! {
                self.#recompute();
                self.on_property_changed(#property_enum::#property);
            }
        }
    }
}

pub fn generate_viewmodel(v: &ViewModelDef, from: &Module, table: &SymbolTable) -> TokenStream {
    let struct_name = format_ident!("{}", v.name);
    let property_enum = format_ident!("{}Property", v.name);
    let field_names: HashSet<&str> = v.fields.iter().map(|f| f.name.as_str()).collect();
    // PropertyChanged is intentionally typed per viewmodel.  A generated view can only subscribe
    // to properties that its DSL expression actually references, so a stringly-typed global event
    // would merely hide mistakes from the compiler.
    //
    // The `ObservableExt` impl below (`#[bindable]`'s target, `elwindui_core::reactive`) is the one
    // deliberate exception: a component injecting this viewmodel across a *separate* macro
    // invocation (`#[elwindui::component]` + `body: view! { .. }`, or any DSL `view`
    // referencing a viewmodel it can't resolve in its own symbol table) has no name for
    // `#property_enum` to write a match arm against at all, enum-typed or otherwise — the choice
    // there isn't "enum vs. string", it's "string vs. nothing works". It doesn't reopen the typo
    // risk this comment warns about, either: the owning component's generated `&'static str` match
    // arms are derived mechanically from the same parsed `view!`/`view` body that also generates
    // its `self.vm.<field>()` read calls, never hand-typed independently, so the two can't drift
    // apart the way a genuinely stringly-typed API could.
    let property_names: Vec<syn::Ident> = v
        .fields
        .iter()
        .filter_map(|f| match f.kind {
            FieldKind::Observable | FieldKind::Computed | FieldKind::AsyncComputed => {
                Some(format_ident!("{}", f.name))
            }
            _ => None,
        })
        .collect();
    let property_name_strs: Vec<String> = property_names
        .iter()
        .map(|ident| ident.to_string())
        .collect();
    // Viewmodels retain a weak self-reference so async actions can upgrade it to `Rc<Self>` and
    // create the `'static` future required by `elwindui::core::task::spawn_local`.

    // `#[computed]` fields need a dependency list so that each observable's setter can call
    // exactly the recompute functions that depend on it (no dynamic subscriber list). An action's
    // own gating condition (what used to be `#[command(can_execute: ...)]`) is now just an
    // ordinary `#[computed]` field the caller writes by hand, so it's already covered here.
    let mut dependents_of: HashMap<String, Vec<String>> = HashMap::new();
    for f in &v.fields {
        if matches!(f.kind, FieldKind::Computed | FieldKind::AsyncComputed) {
            if let Some(Initializer::Expr(expr)) = &f.initializer {
                for dep in referenced_fields(expr, &field_names) {
                    dependents_of.entry(dep).or_default().push(f.name.clone());
                }
            }
        }
    }
    // Looked up when a dependency's setter decides how to notify each of its dependents (below):
    // a `Computed` dependent recomputes synchronously inline, an `AsyncComputed` one only spawns
    // (its own `on_property_changed` fires later, inside the spawned future, only if not
    // superseded by a newer trigger in the meantime).
    let field_kind_by_name: HashMap<&str, FieldKind> =
        v.fields.iter().map(|f| (f.name.as_str(), f.kind)).collect();

    let mut struct_fields = TokenStream::new();
    let mut ctor_fields = TokenStream::new();
    let mut accessors = TokenStream::new();
    let mut recompute_calls_after_new = TokenStream::new();
    // Unlike `recompute_calls_after_new` (run synchronously inside `Rc::new_cyclic`, before
    // `__self_weak` is valid), an async-computed field's first spawn must happen after `new()`'s
    // `Rc::new_cyclic` call has returned — `spawn_local` polls its future once immediately, and
    // that poll upgrades `__self_weak`, which is `None` until the strong `Rc` is fully installed.
    let mut async_spawn_calls_after_new = TokenStream::new();

    for f in &v.fields {
        match f.kind {
            FieldKind::Observable if nested_vec_item_type(&f.ty, from, table).is_some() => {
                let field_ident = format_ident!("{}", f.name);
                let item_ty: syn::Type =
                    syn::parse_str(&nested_vec_item_type(&f.ty, from, table).unwrap())
                        .expect("nested viewmodel type name must parse");

                struct_fields.extend(quote! {
                    #field_ident: std::cell::RefCell<Vec<std::rc::Rc<#item_ty>>>,
                });
                ctor_fields.extend(quote! { #field_ident: std::cell::RefCell::new(Vec::new()), });

                let getter = format_ident!("{}", f.name);
                let pusher = format_ident!("{}_push", f.name);
                let remover = format_ident!("{}_remove", f.name);
                let recompute_calls: Vec<_> = dependents_of
                    .get(&f.name)
                    .into_iter()
                    .flatten()
                    .map(|dep| dependent_recompute_call(dep, &field_kind_by_name, &property_enum))
                    .collect();

                accessors.extend(quote! {
                    pub fn #getter(&self) -> Vec<std::rc::Rc<#item_ty>> {
                        self.#field_ident.borrow().clone()
                    }
                    pub fn #pusher(&self, item: std::rc::Rc<#item_ty>) {
                        self.#field_ident.borrow_mut().push(item);
                        #(#recompute_calls)*
                        self.on_property_changed(#property_enum::#field_ident);
                    }
                    pub fn #remover(&self, index: usize) {
                        self.#field_ident.borrow_mut().remove(index);
                        #(#recompute_calls)*
                        self.on_property_changed(#property_enum::#field_ident);
                    }
                });
            }
            FieldKind::Observable => {
                let field_ident = format_ident!("{}", f.name);
                let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
                let init_expr = match &f.initializer {
                    Some(Initializer::Expr(e)) => rewrite_field_refs(
                        coerce_to_owned_string(&f.ty, e.clone()),
                        &field_names,
                        &format_ident!("self"),
                    ),
                    _ => panic!(
                        "observable field `{}` needs a plain initializer expr",
                        f.name
                    ),
                };

                let (cell_ty, get_body, set_body): (TokenStream, TokenStream, TokenStream) =
                    if is_copy_type(&f.ty) {
                        (
                            quote! { std::cell::Cell<#ty> },
                            quote! { self.#field_ident.get() },
                            quote! { self.#field_ident.set(value); },
                        )
                    } else {
                        (
                            quote! { std::cell::RefCell<#ty> },
                            quote! { self.#field_ident.borrow().clone() },
                            quote! { *self.#field_ident.borrow_mut() = value; },
                        )
                    };

                struct_fields.extend(quote! { #field_ident: #cell_ty, });
                let cell_ctor = if is_copy_type(&f.ty) {
                    quote! { std::cell::Cell::new(#init_expr) }
                } else {
                    quote! { std::cell::RefCell::new(#init_expr) }
                };
                ctor_fields.extend(quote! { #field_ident: #cell_ctor, });

                let getter = format_ident!("{}", f.name);
                let setter = format_ident!("set_{}", f.name);
                let recompute_calls: Vec<_> = dependents_of
                    .get(&f.name)
                    .into_iter()
                    .flatten()
                    .map(|dep| dependent_recompute_call(dep, &field_kind_by_name, &property_enum))
                    .collect();

                accessors.extend(quote! {
                    pub fn #getter(&self) -> #ty { #get_body }
                    pub fn #setter(&self, value: #ty) {
                        #set_body
                        #(#recompute_calls)*
                        self.on_property_changed(#property_enum::#field_ident);
                    }
                });
            }
            FieldKind::Computed => {
                let field_ident = format_ident!("{}", f.name);
                let cache_ident = format_ident!("{}_cache", f.name);
                let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
                let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
                    panic!("#[computed] field `{}` needs an initializer expr", f.name);
                };
                let compute_expr = rewrite_t_macro(
                    rewrite_field_refs(raw_expr.clone(), &field_names, &format_ident!("self")),
                    &field_names,
                    &format_ident!("self"),
                );

                let (cell_ty, get_body, set_cache): (TokenStream, TokenStream, TokenStream) =
                    if is_copy_type(&f.ty) {
                        (
                            quote! { std::cell::Cell<#ty> },
                            quote! { self.#cache_ident.get() },
                            quote! { self.#cache_ident.set(value); },
                        )
                    } else {
                        (
                            quote! { std::cell::RefCell<#ty> },
                            quote! { self.#cache_ident.borrow().clone() },
                            quote! { *self.#cache_ident.borrow_mut() = value; },
                        )
                    };
                let default_ctor = if is_copy_type(&f.ty) {
                    quote! { std::cell::Cell::new(Default::default()) }
                } else {
                    quote! { std::cell::RefCell::new(Default::default()) }
                };

                struct_fields.extend(quote! { #cache_ident: #cell_ty, });
                ctor_fields.extend(quote! { #cache_ident: #default_ctor, });

                let recompute = format_ident!("recompute_{}", f.name);
                accessors.extend(quote! {
                    pub fn #field_ident(&self) -> #ty { #get_body }
                    fn #recompute(&self) {
                        let value: #ty = #compute_expr;
                        #set_cache
                    }
                });
                recompute_calls_after_new.extend(quote! { instance.#recompute(); });
            }
            FieldKind::AsyncComputed => {
                let field_ident = format_ident!("{}", f.name);
                let cache_ident = format_ident!("{}_cache", f.name);
                let generation_ident = format_ident!("{}_generation", f.name);
                let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
                let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
                    panic!(
                        "#[async_computed] field `{}` needs an initializer expr",
                        f.name
                    );
                };
                // Rewrites sibling-field references to `__self.<field>()`, not `self.<field>()`:
                // this expression is evaluated inside `async move { .. }` below, which only ever
                // captures the owned `__self: Rc<Self>` (required for `spawn_local`'s `'static`
                // bound) — never the enclosing method's borrowed `&self`, which would otherwise be
                // implicitly captured by reference and make the whole block non-`'static` (mirrors
                // `FieldKind::Action`'s `is_async` arm, which rewrites to `__self` for the same
                // reason).
                let compute_expr = rewrite_t_macro(
                    rewrite_field_refs(raw_expr.clone(), &field_names, &format_ident!("__self")),
                    &field_names,
                    &format_ident!("__self"),
                );

                struct_fields.extend(quote! {
                    #cache_ident: std::cell::RefCell<elwindui::core::reactive::AsyncComputed<#ty>>,
                    #generation_ident: std::cell::Cell<u64>,
                });
                ctor_fields.extend(quote! {
                    #cache_ident: std::cell::RefCell::new(elwindui::core::reactive::AsyncComputed::Loading),
                    #generation_ident: std::cell::Cell::new(0),
                });

                let spawn_recompute = format_ident!("__spawn_recompute_{}", f.name);
                let property = format_ident!("{}", f.name);
                accessors.extend(quote! {
                    pub fn #field_ident(&self) -> elwindui::core::reactive::AsyncComputed<#ty> {
                        self.#cache_ident.borrow().clone()
                    }
                    // Bumps this field's generation counter synchronously (before `spawn_local`
                    // ever yields), then spawns the recompute. A completion whose captured
                    // generation no longer matches — because a newer trigger fired and re-bumped
                    // in the meantime — is discarded without notifying observers: "supersede, not
                    // cancel" (see docs/design/runtime/state_management_design.md "Async work").
                    fn #spawn_recompute(&self) {
                        let __gen = self.#generation_ident.get().wrapping_add(1);
                        self.#generation_ident.set(__gen);
                        let __self = self.__self_weak.upgrade().expect(
                            "elwindui: viewmodel/store was dropped while an #[async_computed] recompute was still pending"
                        );
                        elwindui::core::task::spawn_local(async move {
                            let __result: Result<#ty, _> = (#compute_expr).await;
                            if __self.#generation_ident.get() == __gen {
                                let __value = match __result {
                                    Ok(v) => elwindui::core::reactive::AsyncComputed::Ready(v),
                                    Err(e) => elwindui::core::reactive::AsyncComputed::Failed(e.to_string()),
                                };
                                *__self.#cache_ident.borrow_mut() = __value;
                                __self.on_property_changed(#property_enum::#property);
                            }
                        });
                    }
                });
                async_spawn_calls_after_new.extend(quote! { instance.#spawn_recompute(); });
            }
            FieldKind::Action => {
                let Some(Initializer::Action {
                    params,
                    is_async,
                    body: block,
                }) = &f.initializer
                else {
                    panic!(
                        "action field `{}` needs a body (an `impl` fn of the same name)",
                        f.name
                    );
                };
                let action_ident = format_ident!("{}", f.name);
                let param_decls = params.iter().map(|(name, ty)| {
                    let ident = format_ident!("{}", name);
                    quote! { #ident: #ty }
                });
                if *is_async {
                    // Async actions use an owned `Rc<Self>` because `spawn_local` requires a
                    // `'static` future. `async move` also captures the action's arguments by
                    // value.
                    let self_ident = format_ident!("__self");
                    let rewritten_block =
                        rewrite_action_body(block.clone(), &field_names, &self_ident);
                    accessors.extend(quote! {
                        pub fn #action_ident(&self, #(#param_decls),*) {
                            let __self = self.__self_weak.upgrade().expect(
                                "elwindui: viewmodel was dropped while an async action was still pending"
                            );
                            elwindui::core::task::spawn_local(async move #rewritten_block);
                        }
                    });
                } else {
                    let self_ident = format_ident!("self");
                    let rewritten_block =
                        rewrite_action_body(block.clone(), &field_names, &self_ident);
                    accessors.extend(quote! {
                        pub fn #action_ident(&self, #(#param_decls),*) #rewritten_block
                    });
                }
            }
            FieldKind::Prop
            | FieldKind::Param
            | FieldKind::Attached
            | FieldKind::State
            | FieldKind::Environment => {
                panic!(
                    "viewmodel field `{}` must be #[observable]/#[computed]",
                    f.name
                );
            }
        }
    }

    quote! {
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #property_enum {
            #(#property_names),*
        }

        pub struct #struct_name {
            #struct_fields
            // `active` is separate from the callback borrow. `on_property_changed` snapshots this
            // list before invocation, so a callback may cancel itself or another callback without
            // conflicting with a RefCell borrow held by the notifier. Each handler itself is a bare
            // `Rc<dyn Fn(..)>`, not `Rc<RefCell<Box<dyn Fn(..)>>>` — it's write-once (only ever
            // constructed inside `subscribe_property_changed`, never replaced in place), so wrapping
            // it in a `RefCell` bought nothing and was actively unsafe: `on_property_changed` calling
            // through a `.borrow()` of it (`(handler.borrow())(property)`) holds that `Ref` for the
            // whole statement — including the callback's own execution — so any handler that
            // (directly or via a nested dispatch) re-entered this same subscription would panic with
            // `BorrowMutError`.
            __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<dyn Fn(#property_enum)>)>>>,
            // Lets an async action body upgrade to an owned `Rc<Self>` before spawning (see the
            // `FieldKind::Action` `is_async` arm) instead of capturing a borrowed `&self` that
            // can't outlive this call. Unused (and so `#[allow(dead_code)]`) on a viewmodel with
            // no async action.
            #[allow(dead_code)]
            __self_weak: std::rc::Weak<Self>,
        }

        impl #struct_name {
            /// Every viewmodel is always `Rc`-allocated from construction on (`Rc::new_cyclic`,
            /// not a plain `Self` a caller wraps later) — both so `#[command(async)]` bodies always
            /// have `__self_weak` to upgrade, and so a `Vec<NestedViewModel>` field's
            /// `documents_push(item: Rc<NestedViewModel>)` never needs a redundant caller-side
            /// `Rc::new(..)` around `NestedViewModel::new()`'s result.
            pub fn new() -> std::rc::Rc<Self> {
                let instance = std::rc::Rc::new_cyclic(|__self_weak| {
                    let instance = Self {
                        #ctor_fields
                        __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                        __self_weak: __self_weak.clone(),
                    };
                    #recompute_calls_after_new
                    instance
                });
                // Must run after `Rc::new_cyclic` returns, not inside its closure: `spawn_local`
                // polls its future once immediately, and that first poll upgrades `__self_weak` —
                // which is only valid once the strong `Rc` above is fully installed.
                #async_spawn_calls_after_new
                instance
            }

            /// Registers a typed PropertyChanged handler. Dropping the returned handle unregisters
            /// it, which is essential for dynamic view regions and item templates.
            pub fn subscribe_property_changed(
                &self,
                f: impl Fn(#property_enum) + 'static,
            ) -> elwindui::core::reactive::Subscription {
                let active = std::rc::Rc::new(std::cell::Cell::new(true));
                let handler: std::rc::Rc<dyn Fn(#property_enum)> = std::rc::Rc::new(f);
                self.__property_changed_handlers.borrow_mut().push((active.clone(), handler));
                let handlers = std::rc::Rc::downgrade(&self.__property_changed_handlers);
                elwindui::core::reactive::Subscription::new(move || {
                    active.set(false);
                    if let Some(handlers) = handlers.upgrade() {
                        handlers
                            .borrow_mut()
                            .retain(|(registered, _)| !std::rc::Rc::ptr_eq(registered, &active));
                    }
                })
            }

            fn on_property_changed(&self, property: #property_enum) {
                let handlers = self.__property_changed_handlers.borrow().clone();
                for (active, handler) in handlers {
                    if active.get() {
                        handler(property);
                    }
                }
            }

            #accessors
        }

        impl #property_enum {
            fn name(&self) -> &'static str {
                match self {
                    #(Self::#property_names => #property_name_strs,)*
                }
            }
        }

        // `#[bindable]`'s target (`ast::Attr::Bindable`'s own doc comment) — lets a component that
        // can't name `#property_enum` (a separate macro invocation from this one) still wire a
        // fine-grained, per-property `PropertyChanged` subscription, identifying properties by
        // name instead. Delegates to the inherent `subscribe_property_changed` above (inherent
        // methods resolve before trait methods, so this isn't self-recursive) purely to convert
        // `#property_enum` to its name — every other behavior (handler storage, cancellation) is
        // shared, unchanged.
        impl elwindui::core::reactive::ObservableExt for #struct_name {
            fn subscribe_property_changed(
                &self,
                f: impl Fn(&'static str) + 'static,
            ) -> elwindui::core::reactive::Subscription {
                self.subscribe_property_changed(move |property| f(property.name()))
            }
        }
    }
}

/// `store Name { fields }` — converts `s` into a throwaway `ViewModelDef` (same name, same fields)
/// and delegates all field codegen to `generate_viewmodel` unchanged (a store's `#[observable]`/
/// `#[computed]`/`#[async_computed]`/action fields behave identically to a viewmodel's own), then
/// appends the singleton access surface: a `#[doc(hidden)]` `EnvironmentKey` whose `Value` is
/// `Rc<Name>` and whose `default_value()` constructs one via `Name::new()` — the same
/// `EnvironmentContext`/`application_environment()` mechanism `#[elwindui::theme]` already uses —
/// plus `Name::instance() -> Rc<Name>`, which any Rust code (including another store's own field
/// expressions, or a `view!`'s generated `TypeName.field` reference codegen) calls to reach the
/// lazily-constructed shared instance. See docs/design/runtime/state_management_design.md "Stores".
pub fn generate_store(s: &StoreDef, from: &Module, table: &SymbolTable) -> TokenStream {
    let as_viewmodel = ViewModelDef {
        name: s.name.clone(),
        fields: s.fields.clone(),
    };
    let body = generate_viewmodel(&as_viewmodel, from, table);
    let struct_name = format_ident!("{}", s.name);
    let key_name = format_ident!("__{}StoreKey", s.name);

    quote! {
        #body

        #[doc(hidden)]
        pub struct #key_name;

        impl elwindui::core::environment::EnvironmentKey for #key_name {
            type Value = std::rc::Rc<#struct_name>;

            fn default_value() -> Self::Value {
                #struct_name::new()
            }
        }

        impl #struct_name {
            /// Returns the process-wide shared instance, lazily constructing it on first access
            /// (`EnvironmentContext::get`'s own "materialize the default at the root, once" — see
            /// `elwindui_core::environment`).
            pub fn instance() -> std::rc::Rc<Self> {
                elwindui::core::environment::application_environment().get::<#key_name>()
            }
        }
    }
}

/// Collects identifiers in `expr` that name one of `field_names` (a bare, single-segment path —
/// `SaveState::Saving` and similar multi-segment paths are never a field reference).
///
/// `#[computed]` initializers routinely wrap their real expression in `t!("key", name: expr, ...)`
/// (e.g. `window_title: String = t!("notepad-window-title", file_name: file_name)`) — since
/// `syn::visit` never descends into a macro's raw token stream, a field referenced only inside a
/// `t!(...)` argument would otherwise be invisible here, silently dropping it from
/// `dependents_of` and leaving the owning setter without the recompute call it needs.
fn referenced_fields(expr: &syn::Expr, field_names: &HashSet<&str>) -> Vec<String> {
    struct Collector<'a> {
        field_names: &'a HashSet<&'a str>,
        found: Vec<String>,
    }
    impl<'a> Visit<'a> for Collector<'a> {
        fn visit_expr_path(&mut self, node: &'a syn::ExprPath) {
            if let Some(ident) = node.path.get_ident() {
                let name = ident.to_string();
                if self.field_names.contains(name.as_str()) {
                    self.found.push(name);
                }
            }
            syn::visit::visit_expr_path(self, node);
        }
        fn visit_expr_macro(&mut self, node: &'a syn::ExprMacro) {
            if node.mac.path.is_ident("t") {
                if let Ok((_, args)) = parse_t_macro_tokens(&node.mac.tokens) {
                    // `args`' values are owned locally (parsed fresh from the macro's token
                    // stream), so they can't be visited via `self.visit_expr` — that requires a
                    // reference living as long as the outer AST's `'a`. Recurse into the
                    // free function instead, which is happy to build its own short-lived
                    // `Collector` over these owned exprs.
                    for (_, value) in &args {
                        self.found
                            .extend(referenced_fields(value, self.field_names));
                    }
                }
            }
            syn::visit::visit_expr_macro(self, node);
        }
    }
    let mut collector = Collector {
        field_names,
        found: Vec::new(),
    };
    collector.visit_expr(expr);
    collector.found.sort();
    collector.found.dedup();
    collector.found
}

/// Rewrites bare identifier reads that name a sibling field (`content` inside a `#[computed]`
/// initializer) into accessor calls (`self.content()`). Does not touch assignment targets —
/// action bodies use [`rewrite_action_body`] for that.
fn rewrite_field_refs(
    mut expr: syn::Expr,
    field_names: &HashSet<&str>,
    receiver: &syn::Ident,
) -> TokenStream {
    struct Rewriter<'a> {
        field_names: &'a HashSet<&'a str>,
        receiver: &'a syn::Ident,
    }
    impl<'a> VisitMut for Rewriter<'a> {
        fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
            if let syn::Expr::Path(p) = node {
                if let Some(ident) = p.path.get_ident() {
                    if self.field_names.contains(ident.to_string().as_str()) {
                        let receiver = self.receiver;
                        let call: syn::Expr = syn::parse_quote! { #receiver.#ident() };
                        *node = call;
                        return;
                    }
                }
            }
            syn::visit_mut::visit_expr_mut(self, node);
        }
    }
    let mut rewriter = Rewriter {
        field_names,
        receiver,
    };
    rewriter.visit_expr_mut(&mut expr);
    quote! { #expr }
}

/// Recognizes `t!("key", name: expr, ...)` (parsed as an opaque `syn::Expr::Macro` by the DSL
/// parser, since `name: expr` argument lists aren't valid standalone Rust) and rewrites it into a
/// call to the generated `t()` i18n helper (see `i18n_prelude`). See docs/specs/dsl_spec.md §10.
///
/// `syn::visit_mut` never descends into a macro's token stream (it has no structure to visit), so
/// [`rewrite_field_refs`] alone can't see field references nested inside `t!(...)`'s arguments —
/// each argument value is re-rewritten here once it's been pulled out as a real `syn::Expr`.
fn rewrite_t_macro(
    expr: TokenStream,
    field_names: &HashSet<&str>,
    receiver: &syn::Ident,
) -> TokenStream {
    let expr: syn::Expr = syn::parse2(expr).expect("rewrite_field_refs always yields valid Expr");
    if let syn::Expr::Macro(m) = &expr {
        if m.mac.path.is_ident("t") {
            return rewrite_t_call(&m.mac.tokens, field_names, receiver);
        }
    }
    quote! { #expr }
}

/// [`rewrite_t_macro`]'s counterpart for an expression emitted where sibling field references are
/// already-correct bare local identifiers rather than needing a `self.<field>()` rewrite — a
/// component's own defaulted-prop/computed field's *initial* value, computed once via a plain `let`
/// before `self` exists (`generate_view`'s own-field construction-time `let` bindings, above). Only
/// `t!(...)`'s own macro-call shape needs expanding here (it isn't real Rust `syn::visit` can walk
/// into); its argument values are left exactly as parsed, unlike [`rewrite_t_call`]'s `receiver`-
/// prefixed ones.
fn rewrite_t_macro_bare(expr: syn::Expr) -> TokenStream {
    if let syn::Expr::Macro(m) = &expr {
        if m.mac.path.is_ident("t") {
            let (key, args) = parse_t_macro_tokens(&m.mac.tokens)
                .expect("t!(...) arguments must be `\"key\", name: expr, ...`");
            let arg_pairs = args.iter().map(|(name, value)| {
                let name_str = name.to_string();
                quote! { (#name_str, elwindui::i18n::FluentValue::from(#value)) }
            });
            return quote! { elwindui::i18n::t(#key, &[ #(#arg_pairs),* ]) };
        }
    }
    quote! { #expr }
}

/// Parses a `t!(...)` macro's raw tokens (`"key", name1: expr1, name2: expr2`) into the key and
/// its named argument expressions. Shared by [`rewrite_t_call`] (codegen) and [`referenced_fields`]
/// (dependency-graph analysis) — both need to look inside the macro's opaque token stream, since
/// `syn::visit`/`syn::visit_mut` never descend into a macro's tokens on their own.
fn parse_t_macro_tokens(
    tokens: &TokenStream,
) -> syn::Result<(syn::LitStr, Vec<(syn::Ident, syn::Expr)>)> {
    let parser = |input: syn::parse::ParseStream| -> syn::Result<(syn::LitStr, Vec<(syn::Ident, syn::Expr)>)> {
        let key: syn::LitStr = input.parse()?;
        let mut args = Vec::new();
        while input.parse::<syn::Token![,]>().is_ok() {
            if input.is_empty() {
                break;
            }
            let name: syn::Ident = input.parse()?;
            input.parse::<syn::Token![:]>()?;
            let value: syn::Expr = input.parse()?;
            args.push((name, value));
        }
        Ok((key, args))
    };
    syn::parse::Parser::parse2(parser, tokens.clone())
}

fn rewrite_t_call(
    tokens: &TokenStream,
    field_names: &HashSet<&str>,
    receiver: &syn::Ident,
) -> TokenStream {
    // Tokens look like: "key", name1: expr1, name2: expr2
    let (key, args) =
        parse_t_macro_tokens(tokens).expect("t!(...) arguments must be `\"key\", name: expr, ...`");
    let arg_pairs = args.iter().map(|(name, value)| {
        let name_str = name.to_string();
        let value = rewrite_field_refs(value.clone(), field_names, receiver);
        quote! { (#name_str, elwindui::i18n::FluentValue::from(#value)) }
    });
    quote! { elwindui::i18n::t(#key, &[ #(#arg_pairs),* ]) }
}

/// Rewrites a viewmodel action's `impl` fn body: assignments to a sibling field (`state = expr`)
/// become setter calls, bare reads of a sibling field become getter calls, and the whole thing
/// becomes a method body (`fn f(&self) { ... }`). `receiver` is `self` for a plain (synchronous)
/// action, or an owned local (`__self: Rc<Self>`) for an async one — see the `FieldKind::Action`
/// `is_async` arm for why a borrowed `self` won't do there.
fn rewrite_action_body(
    mut block: syn::Block,
    field_names: &HashSet<&str>,
    receiver: &syn::Ident,
) -> TokenStream {
    struct Rewriter<'a> {
        field_names: &'a HashSet<&'a str>,
        receiver: &'a syn::Ident,
    }
    impl<'a> VisitMut for Rewriter<'a> {
        fn visit_stmt_mut(&mut self, stmt: &mut syn::Stmt) {
            syn::visit_mut::visit_stmt_mut(self, stmt);
        }

        fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
            let receiver = self.receiver;
            if let syn::Expr::Assign(assign) = node {
                if let syn::Expr::Path(p) = assign.left.as_ref() {
                    if let Some(ident) = p.path.get_ident() {
                        if self.field_names.contains(ident.to_string().as_str()) {
                            let setter = format_ident!("set_{}", ident);
                            let mut value = (*assign.right).clone();
                            self.visit_expr_mut(&mut value);
                            // `#value` is bound to a `let` in its own statement rather than
                            // embedded directly as the setter's argument: if `#value` itself reads
                            // the same field (e.g. `field = format!("{}x", self.field.borrow())`),
                            // the `Ref` temporary that produces would otherwise live until the end
                            // of the *whole* `#receiver.#setter(#value)` statement (Rust drops
                            // temporaries at statement end, not at the end of the sub-expression
                            // that created them) — so it would still be alive when the setter's own
                            // `borrow_mut()` runs, panicking with `BorrowMutError`. Ending the `let`
                            // statement first drops that temporary before the setter call.
                            *node = syn::parse_quote! {{
                                let __elwindui_value = #value;
                                #receiver.#setter(__elwindui_value)
                            }};
                            return;
                        }
                    }
                }
            }
            // `documents.push(doc)` / `documents.remove(index)` on a sibling `Vec<NestedVM>`
            // field: must be special-cased *before* the generic `Expr::Path` fallback below would
            // otherwise rewrite just the receiver to `self.documents()` (a *cloned* Vec — pushing
            // to or removing from that clone wouldn't persist). Rewrites the whole call to the
            // dedicated `documents_push`/`documents_remove` methods `generate_viewmodel` generates
            // for `Vec<NestedViewModel>` fields (see `nested_vec_item_type`).
            if let syn::Expr::MethodCall(call) = node {
                if let syn::Expr::Path(p) = call.receiver.as_ref() {
                    if let Some(ident) = p.path.get_ident() {
                        let method = call.method.to_string();
                        if self.field_names.contains(ident.to_string().as_str())
                            && (method == "push" || method == "remove")
                        {
                            let helper = format_ident!("{}_{}", ident, method);
                            let mut args = call.args.clone();
                            for arg in args.iter_mut() {
                                self.visit_expr_mut(arg);
                            }
                            // Same "bind to a `let` before calling the mutating helper" reasoning
                            // as the `Expr::Assign` arm above: an arg expression that itself reads
                            // the same `field` (e.g. `documents.push(make_doc(self.documents.borrow().len()))`)
                            // would otherwise keep a `Ref` temporary alive across the whole
                            // `#receiver.#helper(#args)` statement, clashing with the helper's own
                            // internal `borrow_mut()`.
                            let arg_idents: Vec<syn::Ident> = (0..args.len())
                                .map(|i| format_ident!("__elwindui_arg{}", i))
                                .collect();
                            let arg_values: Vec<&syn::Expr> = args.iter().collect();
                            *node = syn::parse_quote! {{
                                #( let #arg_idents = #arg_values; )*
                                #receiver.#helper(#(#arg_idents),*)
                            }};
                            return;
                        }
                    }
                }
            }
            // `t!(...)` inside an action body: `syn::visit_mut` never descends into a macro's
            // token stream, so this has to be special-cased the same way as
            // `rewrite_t_macro`/`rewrite_t_call` (used for `#[computed]` initializers).
            if let syn::Expr::Macro(m) = node {
                if m.mac.path.is_ident("t") {
                    let rewritten = rewrite_t_call(&m.mac.tokens, self.field_names, self.receiver);
                    *node =
                        syn::parse2(rewritten).expect("rewrite_t_call always yields a valid Expr");
                    return;
                }
            }
            if let syn::Expr::Path(p) = node {
                if let Some(ident) = p.path.get_ident() {
                    if self.field_names.contains(ident.to_string().as_str()) {
                        *node = syn::parse_quote! { #receiver.#ident() };
                        return;
                    }
                }
            }
            syn::visit_mut::visit_expr_mut(self, node);
        }
    }
    let mut rewriter = Rewriter {
        field_names,
        receiver,
    };
    rewriter.visit_block_mut(&mut block);
    quote! { #block }
}

/// Extracts `#[environment(name)]`'s referenced Environment Key name from a
/// `FieldKind::Environment` field's `attrs` — see `Attr::Environment`'s own doc comment.
fn environment_key_name(f: &FieldDef) -> &str {
    f.attrs
        .iter()
        .find_map(|a| match a {
            Attr::Environment(name, _prefix) => Some(name.as_str()),
            _ => None,
        })
        .expect(
            "internal: FieldKind::Environment field must carry Attr::Environment(name, prefix) \
             (attr_frontend.rs invariant)",
        )
}

/// The crate-qualifying prefix of a cross-crate `#[environment(some_crate::name)]` (Issue #129,
/// `attr_frontend::split_environment_key_path`) — `None` for the same-crate bare form
/// (`#[environment(name)]`).
fn environment_key_prefix(f: &FieldDef) -> Option<&str> {
    f.attrs
        .iter()
        .find_map(|a| match a {
            Attr::Environment(_name, prefix) => Some(prefix.as_deref()),
            _ => None,
        })
        .expect(
            "internal: FieldKind::Environment field must carry Attr::Environment(name, prefix) \
             (attr_frontend.rs invariant)",
        )
}

/// Resolves `#[environment(name)]`'s referenced Key type. Returns `(preamble, key_type)` — the
/// caller must splice `preamble` into the *same* local block as its own use of `key_type`, before
/// that use (an empty `TokenStream` for the bare form, so existing call sites are unaffected).
///
/// Bare form (`environment_key_prefix(f)` is `None`): resolved from the same-crate registry
/// (`component_frontend::lookup_same_crate_environment_key`) — `validate::validate` (rule 34,
/// `docs/specs/dsl_spec.md` §13) already rejected an unresolvable name before codegen runs.
/// `preamble` is empty and `key_type` is the registered Key type path directly, exactly as before
/// this function returned a plain `syn::Type`.
///
/// Qualified form (`some_crate::name`, Issue #129): naively splicing a type-position invocation of
/// the declaring crate's exported `__elwindui_environment_key_{name}!` macro *by absolute path*
/// (`some_crate::__elwindui_environment_key_name!()`, the same technique `elwindui-codegen`'s
/// `#[class]`-facing code uses for `__elwindui_props_*!(@field_type ..)`) was tried first and
/// rejected: unlike the `#[class]` case, this one is a `macro_export`-declared `macro_rules!`
/// referenced via an absolute path from *other* macro-generated code, which trips rustc's
/// deny-by-default `macro_expanded_macro_exports_accessed_by_absolute_paths` future-incompatible
/// lint (confirmed by an isolated multi-crate repro before this function was written — this is
/// exactly the lint Issue #129's own constraints section rules out depending on, unlike `#[class]`'s
/// own `__elwindui_inherit_*!`/`__elwindui_props_*!`, which use `#[allow(..)]` for it). Instead this
/// emits `preamble = "use #prefix::#macro; type #alias = #macro!();"` (a local item pair — a
/// `use`-import followed by a bare, unqualified macro call, not an absolute-path one) and returns
/// `key_type = #alias`, a plain local type identifier the caller splices in place of the type it
/// used to receive directly. `#alias` is derived from `alias_seed` (a per-call-site-unique name,
/// e.g. the field name) specifically so that a caller accumulating several fields' preambles into
/// one shared enclosing scope (`generate_component`'s own `default_let_stmts`, for one) never
/// emits two colliding `type` aliases with the same name in that scope.
///
/// There is no same-crate-style early validation for the qualified form: a proc-macro genuinely
/// cannot see whether another crate exports a given macro name before real compilation runs, so an
/// unresolvable qualified name surfaces later as `rustc`'s own "cannot find macro" error, not a
/// `compile_error!` — deliberately accepted asymmetry with the bare form, documented in
/// `docs/specs/dsl_spec.md` §13 rules 34/35.
fn environment_key_type(f: &FieldDef) -> (TokenStream, syn::Type) {
    environment_key_type_by_name(environment_key_name(f), environment_key_prefix(f), &f.name)
}

/// Core resolution shared by `environment_key_type` (`#[environment(name)]` fields) and
/// ordinary `#[environment(name)]` fields' same-crate lookup. ControlTemplate selection uses the
/// generic typed Environment slot and does not call this resolver.
fn environment_key_type_by_name(
    name: &str,
    prefix: Option<&str>,
    alias_seed: &str,
) -> (TokenStream, syn::Type) {
    match prefix {
        Some(prefix) => {
            let macro_ident = format_ident!("__elwindui_environment_key_{name}");
            let prefix_path: syn::Path = syn::parse_str(prefix)
                .expect("qualified environment key crate prefix must parse as a path");
            let alias_ident = format_ident!("__ElwindEnvKeyAlias_{alias_seed}");
            let preamble = quote! {
                use #prefix_path::#macro_ident;
                type #alias_ident = #macro_ident!();
            };
            (preamble, syn::parse_quote!(#alias_ident))
        }
        None => {
            let (key_type_name, _value_type) =
                crate::component_frontend::lookup_environment_key(name).unwrap_or_else(|| {
                    panic!(
                        "internal: `#[environment({name})]` referenced an unregistered \
                             Environment Key — validate::validate should have rejected this \
                             before codegen"
                    )
                });
            (
                TokenStream::new(),
                syn::parse_str(&key_type_name)
                    .expect("registered environment key type name must parse"),
            )
        }
    }
}

// PR #169 review remediation, round 3 (A2/AD-R3-2/AD-R3-4): `source_component` is the literal,
// un-flattened `ComponentDef` this Component's own source actually declares (never
// `info.effective_fields`-flattened) — the only input `component_public_shape` may ever receive
// (AD-R3-2: the helper is source-local by design and must never be handed an ancestor-inclusive
// field list, which would make it treat an inherited field as though this Component declared it
// itself). `c` (unchanged parameter name/position, still the effective/possibly-flattened
// `ComponentDef` the caller already built) remains the input to every other, non-shape decision in
// this function — inherited-field forwarding/storage stays real generation's own job, not the
// shape's (AD-R3-3).
fn generate_component(
    source_component: &ComponentDef,
    c: &ComponentDef,
    table: &SymbolTable,
) -> TokenStream {
    let struct_name = format_ident!("{}", c.name);
    let mut struct_fields = TokenStream::new();
    let mut ctor_params = TokenStream::new();
    let mut ctor_field_inits = TokenStream::new();
    let mut accessors = TokenStream::new();

    // A defaulted `#[prop(default = ...)]`/`#[computed(expr = ...)]` field (`generate_view`'s own
    // sibling handling above has the full design-rationale doc comment) — this view-less component
    // has no widget tree to construct and no `resync()` to hook into, so this is simpler than
    // `generate_view`'s version: just Cell/RefCell storage, seeded by a `let <name> = <expr>;`
    // chain (bare sibling references — plain local identifiers, exactly like `generate_view`'s own,
    // since `self` doesn't exist yet inside `new(..)`'s still-being-built struct literal either),
    // a getter, a `#[prop]`-default field's setter (cascading into any `#[computed]` field that
    // depends on it, mirroring `generate_viewmodel`'s Observable-setter cascade), and a
    // `recompute_<name>` for each `#[computed]` field.
    let field_names: HashSet<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
    let own_computed_fields: Vec<&FieldDef> = c
        .fields
        .iter()
        .filter(|f| {
            f.kind == FieldKind::Computed && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
        .collect();
    let mut dependents_of: HashMap<String, Vec<String>> = HashMap::new();
    for f in &own_computed_fields {
        if let Some(Initializer::Expr(expr)) = &f.initializer {
            for dep in referenced_fields(expr, &field_names) {
                dependents_of.entry(dep).or_default().push(f.name.clone());
            }
        }
    }
    let mut default_let_stmts = TokenStream::new();
    for f in c.fields.iter().filter(|f| {
        matches!(f.initializer, Some(Initializer::Expr(_)))
            && matches!(
                f.kind,
                FieldKind::Prop | FieldKind::State | FieldKind::Computed
            )
    }) {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
        let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
            unreachable!("filtered to Some(Initializer::Expr(_)) above");
        };
        let init_expr = rewrite_t_macro_bare(coerce_to_owned_string(&f.ty, raw_expr.clone()));
        default_let_stmts.extend(quote! { let #field_ident: #ty = #init_expr; });
    }
    // `#[environment(name)]` fields resolve from `application_environment()` at construction
    // (CI-6 of #80, docs/design/runtime/component_lifecycle_design.md §4e — this view-less
    // component has no `Rc<Self>`/`mount()`/property-changed dispatch to subscribe a live update
    // through, unlike `generate_view`'s composed/plain paths, so there is no later point to defer
    // resolution to; it is resolved once, here, and never updates afterward, same as before this
    // change — only *what* it reads from changed, not *when*) rather than from a declared
    // expression — seeded as a bare `let` the same way, so the view-body bare-identifier reference
    // this field's own name still resolves during construction (`own_fields`, `emit_expr`).
    for f in c.fields.iter().filter(|f| f.kind == FieldKind::Environment) {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
        let (key_type_preamble, key_type) = environment_key_type(f);
        default_let_stmts.extend(quote! {
            #key_type_preamble
            let #field_ident: #ty = elwindui::core::environment::application_environment().get::<#key_type>();
        });
    }
    let component_property_enum = format_ident!("{}Property", c.name);
    let property_variants: Vec<syn::Ident> = c
        .fields
        .iter()
        .filter(|f| {
            matches!(f.initializer, Some(Initializer::Expr(_)))
                && matches!(
                    f.kind,
                    FieldKind::Prop | FieldKind::State | FieldKind::Computed
                )
        })
        .map(|f| format_ident!("{}", f.name))
        .collect();

    // PR #169 review remediation, round 2 (AD-R2-6), input corrected round 3 (A2/AD-R3-2): a
    // no-initializer *own* field's (one `source_component.fields` itself declares — never an
    // inherited one, see this function's own leading doc comment) deferred-vs-required membership
    // is decided once, here, by `component_public_shape(source_component, None)` — the same
    // source-local classification `rust_analyzer_shadow::build_component_struct_shadow` and
    // `generate_view`'s own own-field constructor decision (below, in this file) both consult —
    // rather than this loop independently re-running `strip_option`/deferral logic (PR #169 review
    // finding A2's own forbidden pattern). `view: None` matches this function's own view-less
    // generation exactly. A field present in `c.fields` (the effective set) but absent from
    // `source_component.fields` is inherited, not this Component's own declaration — never looked
    // up here, so it falls through to the pre-#146 direct `strip_option` computation below
    // unchanged, exactly mirroring `generate_view`'s own `is_deferred_own_field` fallback boundary.
    let own_field_shape = crate::component_frontend::component_public_shape(source_component, None);
    let declared_own_field_names: HashSet<&str> = source_component
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    // PR #169 review remediation, round 4 (AD-R4-1/AD-R4-2): every own-field API-membership
    // decision below (constructor param, deferred-option storage, getter, setter, and each
    // accessor's visibility) is driven exclusively from these four shape-derived indices — never
    // independently re-derived from `f.kind`/`f.initializer`/`strip_option` for a name in
    // `declared_own_field_names`. `FieldDef` (`f` itself) is still consulted, but only to select
    // *how* an already-shape-decided accessor is implemented (Cell vs RefCell, getter/setter body,
    // recompute cascade) — never *whether* it exists. An inherited field (absent from
    // `declared_own_field_names`) is untouched by any of these indices and keeps the pre-#146
    // direct computation unchanged (AD-R3-3/AD-R4-8: inherited-field forwarding is real
    // generation's own job, never routed through the source-local shape).
    let own_constructor_params: HashMap<&str, &str> = own_field_shape
        .constructor_params
        .iter()
        .map(|(name, ty)| (name.as_str(), ty.as_str()))
        .collect();
    let own_deferred_fields: HashMap<&str, (&str, &str)> = own_field_shape
        .deferred_option_fields
        .iter()
        .map(|(name, declared_ty, inner_ty)| {
            (name.as_str(), (declared_ty.as_str(), inner_ty.as_str()))
        })
        .collect();
    let own_readable_fields: HashMap<&str, (&str, crate::component_frontend::ShadowVisibility)> =
        own_field_shape
            .readable_fields
            .iter()
            .map(|(name, ty, visibility)| (name.as_str(), (ty.as_str(), *visibility)))
            .collect();
    let own_writable_fields: HashMap<&str, (&str, crate::component_frontend::ShadowVisibility)> =
        own_field_shape
            .writable_fields
            .iter()
            .map(|(name, ty, visibility)| (name.as_str(), (ty.as_str(), *visibility)))
            .collect();

    for f in &c.fields {
        let is_own = declared_own_field_names.contains(f.name.as_str());
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");

        match &f.initializer {
            None if f.kind == FieldKind::Environment => {
                // Seeded by the `let #field_ident = ..;` emitted into `default_let_stmts` above —
                // never a constructor argument (`param_fields`/`param_names` exclude
                // `FieldKind::Environment` explicitly), never mutated after construction on this
                // view-less path (see that loop's own doc comment).
                struct_fields.extend(quote! { #field_ident: #ty, });
                ctor_field_inits.extend(quote! { #field_ident, });
                if is_own {
                    if let Some((_, visibility)) = own_readable_fields.get(f.name.as_str()) {
                        let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                        accessors.extend(quote! {
                            #vis fn #field_ident(&self) -> #ty { self.#field_ident.clone() }
                        });
                    }
                } else {
                    accessors.extend(quote! {
                        pub fn #field_ident(&self) -> #ty { self.#field_ident.clone() }
                    });
                }
            }
            None => {
                // `#[param] #[inject]` field: supplied by the caller. `Option<T>`-typed fields
                // (docs/design/runtime/ui_tree_design.md's post-construction setter convention,
                // extended from builtins to plain `component`s) are deferred instead — dropped from
                // `new(..)`'s own argument list, stored `Cell`/`RefCell`-wrapped (`is_copy_type`)
                // defaulting to `None`, and given a `set_<name>(&self, value: T)` setter — `None`
                // is `Option<T>`'s own natural "not yet set" value, so (unlike a required field of
                // arbitrary, possibly non-`Default` type) there's always a sound value to start
                // from. A required (non-`Option`) field stays exactly as before: a `new(..)`
                // argument, plain storage, no setter.
                //
                // PR #169 review remediation, round 4 (AD-R4-3/AD-R4-4/AD-R4-5/AD-R4-6): for an
                // *own* field, `is_deferred` (storage shape), constructor membership, and
                // getter/setter membership+visibility all come from the shape's four indices —
                // `f.kind`/`f.initializer`/`strip_option` below only ever select the deferred vs.
                // required *implementation* (Cell/RefCell type, get/set body), never whether an
                // accessor exists. An inherited field keeps the exact pre-#146 direct computation
                // (unconditional pub getter/setter either way), unchanged (AD-R4-8).
                let is_deferred = if is_own {
                    own_deferred_fields.contains_key(f.name.as_str())
                } else {
                    strip_option(&f.ty).1
                };
                if is_deferred {
                    let inner_ty_str = if is_own {
                        own_deferred_fields
                            .get(f.name.as_str())
                            .map(|(_, inner_ty)| *inner_ty)
                            .expect("is_deferred true for an own field implies a deferred_option_fields entry")
                    } else {
                        strip_option(&f.ty).0
                    };
                    let inner_ty: syn::Type =
                        syn::parse_str(inner_ty_str).expect("field inner type must parse");
                    let cell_ty = if is_copy_type(inner_ty_str) {
                        quote! { std::cell::Cell }
                    } else {
                        quote! { std::cell::RefCell }
                    };
                    struct_fields.extend(quote! { #field_ident: #cell_ty<Option<#inner_ty>>, });
                    ctor_field_inits.extend(quote! { #field_ident: #cell_ty::new(None), });
                    let set_name = format_ident!("set_{}", f.name);
                    let get_body = if is_copy_type(inner_ty_str) {
                        quote! { self.#field_ident.get() }
                    } else {
                        quote! { self.#field_ident.borrow().clone() }
                    };
                    let set_body = if is_copy_type(inner_ty_str) {
                        quote! { self.#field_ident.set(Some(value)); }
                    } else {
                        quote! { *self.#field_ident.borrow_mut() = Some(value); }
                    };
                    if is_own {
                        if let Some((_, visibility)) = own_readable_fields.get(f.name.as_str()) {
                            let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                            accessors.extend(quote! {
                                #vis fn #field_ident(&self) -> #ty { #get_body }
                            });
                        }
                        // PR #169 review remediation, round 5 (AD-R5-1/AD-R5-3/AD-R5-4): the
                        // setter parameter *type* comes from `own_writable_fields`'s own type
                        // entry, not `inner_ty` (round 4 still discarded this with `_`) — `#ty`/
                        // `strip_option`/`own_deferred_fields` remain the sole authority for
                        // *storage* (`Cell`/`RefCell<Option<T>>`, the `Some(value)` wrapping in
                        // `#set_body` above), never for the setter's own public signature.
                        if let Some((setter_ty_str, visibility)) =
                            own_writable_fields.get(f.name.as_str())
                        {
                            let setter_ty: syn::Type = syn::parse_str(setter_ty_str)
                                .expect("shape setter type must parse");
                            let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                            accessors.extend(quote! {
                                #vis fn #set_name(&self, value: #setter_ty) { #set_body }
                            });
                        }
                    } else {
                        accessors.extend(quote! {
                            pub fn #field_ident(&self) -> #ty { #get_body }
                            pub fn #set_name(&self, value: #inner_ty) { #set_body }
                        });
                    }
                } else {
                    struct_fields.extend(quote! { #field_ident: #ty, });
                    ctor_field_inits.extend(quote! { #field_ident, });
                    if is_own {
                        if own_constructor_params.contains_key(f.name.as_str()) {
                            ctor_params.extend(quote! { #field_ident: #ty, });
                        }
                        if let Some((_, visibility)) = own_readable_fields.get(f.name.as_str()) {
                            let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                            accessors.extend(quote! {
                                #vis fn #field_ident(&self) -> #ty { self.#field_ident.clone() }
                            });
                        }
                        // A required (non-deferred) own field never appears in `writable_fields`
                        // for `component_public_shape(.., None)` — the view-less path
                        // (`component_public_shape`'s own doc comment: a required `Prop` field's
                        // setter is `has_view`-only) — so there is no sound setter body to emit
                        // here (this field's storage is plain `#ty`, not `Cell`/`RefCell`-wrapped).
                        debug_assert!(
                            !own_writable_fields.contains_key(f.name.as_str()),
                            "view-less component_public_shape must never mark a required own field writable: {}",
                            f.name
                        );
                    } else {
                        ctor_params.extend(quote! { #field_ident: #ty, });
                        accessors.extend(quote! {
                            pub fn #field_ident(&self) -> #ty { self.#field_ident.clone() }
                        });
                    }
                }
            }
            Some(Initializer::Expr(raw_expr))
                if matches!(f.kind, FieldKind::Prop | FieldKind::State) =>
            {
                let cell_ty = if is_copy_type(&f.ty) {
                    quote! { std::cell::Cell }
                } else {
                    quote! { std::cell::RefCell }
                };
                struct_fields.extend(quote! { #field_ident: #cell_ty<#ty>, });
                ctor_field_inits.extend(quote! { #field_ident: <#cell_ty<_>>::new(#field_ident), });
                let get_body = if is_copy_type(&f.ty) {
                    quote! { self.#field_ident.get() }
                } else {
                    quote! { self.#field_ident.borrow().clone() }
                };
                let set_name = format_ident!("set_{}", f.name);
                let set_body = if is_copy_type(&f.ty) {
                    quote! { self.#field_ident.set(value); }
                } else {
                    quote! { *self.#field_ident.borrow_mut() = value; }
                };
                let recompute_calls: Vec<TokenStream> = dependents_of
                    .get(&f.name)
                    .into_iter()
                    .flatten()
                    .map(|dep| {
                        let recompute = format_ident!("recompute_{}", dep);
                        let property = format_ident!("{}", dep);
                        quote! {
                            self.#recompute();
                            self.on_property_changed(#component_property_enum::#property);
                        }
                    })
                    .collect();
                // PR #169 review remediation, round 4 (AD-R4-4/AD-R4-5): for an own field, getter
                // and setter membership+visibility both come from the shape, not an independent
                // `f.kind == FieldKind::State` check — `component_public_shape` already applies
                // exactly this rule (`ShadowVisibility::Private` for `State`, `Public` for `Prop`)
                // when it builds `readable_fields`/`writable_fields`, so this is the same decision
                // read from one place instead of two. An inherited field keeps the original direct
                // `FieldKind::State` check unchanged (AD-R4-8).
                if is_own {
                    if let Some((_, visibility)) = own_readable_fields.get(f.name.as_str()) {
                        let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                        accessors.extend(quote! {
                            #vis fn #field_ident(&self) -> #ty { #get_body }
                        });
                    }
                    // PR #169 review remediation, round 5 (AD-R5-1/AD-R5-3/AD-R5-5): the setter
                    // parameter type comes from `own_writable_fields`'s own type entry, not `#ty`
                    // (round 4 still discarded this with `_`) — `#ty`/`is_copy_type` remain the
                    // sole authority for `#set_body`'s own storage mechanics above.
                    if let Some((setter_ty_str, visibility)) =
                        own_writable_fields.get(f.name.as_str())
                    {
                        let setter_ty: syn::Type =
                            syn::parse_str(setter_ty_str).expect("shape setter type must parse");
                        let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                        accessors.extend(quote! {
                            #vis fn #set_name(&self, value: #setter_ty) {
                                #set_body
                                #(#recompute_calls)*
                                self.on_property_changed(#component_property_enum::#field_ident);
                            }
                        });
                    }
                } else {
                    let visibility = if f.kind == FieldKind::State {
                        quote! {}
                    } else {
                        quote! { pub }
                    };
                    accessors.extend(quote! {
                        #visibility fn #field_ident(&self) -> #ty { #get_body }
                        #visibility fn #set_name(&self, value: #ty) {
                            #set_body
                            #(#recompute_calls)*
                            self.on_property_changed(#component_property_enum::#field_ident);
                        }
                    });
                }
                let _ = raw_expr; // consumed by `default_let_stmts`, above
            }
            Some(Initializer::Expr(raw_expr)) if f.kind == FieldKind::Computed => {
                let cell_ty = if is_copy_type(&f.ty) {
                    quote! { std::cell::Cell }
                } else {
                    quote! { std::cell::RefCell }
                };
                struct_fields.extend(quote! { #field_ident: #cell_ty<#ty>, });
                ctor_field_inits.extend(quote! { #field_ident: <#cell_ty<_>>::new(#field_ident), });
                let get_body = if is_copy_type(&f.ty) {
                    quote! { self.#field_ident.get() }
                } else {
                    quote! { self.#field_ident.borrow().clone() }
                };
                let set_cache = if is_copy_type(&f.ty) {
                    quote! { self.#field_ident.set(value); }
                } else {
                    quote! { *self.#field_ident.borrow_mut() = value; }
                };
                let compute_expr = rewrite_t_macro(
                    rewrite_field_refs(raw_expr.clone(), &field_names, &format_ident!("self")),
                    &field_names,
                    &format_ident!("self"),
                );
                let recompute = format_ident!("recompute_{}", f.name);
                // PR #169 review remediation, round 4 (AD-R4-4): the getter's own membership comes
                // from the shape for an own field (`component_public_shape` always places a
                // `Computed` field in `readable_fields`, `ShadowVisibility::Public`, so this never
                // actually diverges from the previous unconditional `pub fn` — but it is now the
                // shape making that decision, not this match arm independently). `recompute_<name>`
                // is a private implementation helper with no `ComponentPublicShape` concept at
                // all — always emitted, exactly as before.
                if is_own {
                    if let Some((_, visibility)) = own_readable_fields.get(f.name.as_str()) {
                        let vis = crate::rust_analyzer_shadow::shadow_vis_tokens(*visibility);
                        accessors.extend(quote! {
                            #vis fn #field_ident(&self) -> #ty { #get_body }
                        });
                    }
                } else {
                    accessors.extend(quote! {
                        pub fn #field_ident(&self) -> #ty { #get_body }
                    });
                }
                accessors.extend(quote! {
                    fn #recompute(&self) {
                        let value: #ty = #compute_expr;
                        #set_cache
                    }
                });
            }
            Some(Initializer::Expr(_)) => unreachable!(
                "field `{}`: a plain initializer expr is only valid on #[prop]/#[computed] (validate.rs already rejects other kinds)",
                f.name
            ),
            Some(Initializer::Action { .. }) => {
                panic!(
                    "component field `{}`: an action is a viewmodel-only construct, synthesized \
                     from an `impl` block's `fn`s — not supported on a plain component",
                    f.name
                );
            }
        }
    }

    let _ = table; // reserved for future cross-component validation
    let methods = emit_methods(&c.methods);
    quote! {
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #component_property_enum {
            #(#property_variants),*
        }

        pub struct #struct_name {
            #struct_fields
            __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<dyn Fn(#component_property_enum)>)>>>,
        }

        impl #struct_name {
            pub fn new(#ctor_params) -> Self {
                #default_let_stmts
                Self {
                    #ctor_field_inits
                    __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                }
            }

            pub fn subscribe_property_changed(
                &self,
                f: impl Fn(#component_property_enum) + 'static,
            ) -> elwindui::core::reactive::Subscription {
                let active = std::rc::Rc::new(std::cell::Cell::new(true));
                let handler: std::rc::Rc<dyn Fn(#component_property_enum)> = std::rc::Rc::new(f);
                self.__property_changed_handlers
                    .borrow_mut()
                    .push((active.clone(), handler));
                let handlers = std::rc::Rc::downgrade(&self.__property_changed_handlers);
                elwindui::core::reactive::Subscription::new(move || {
                    active.set(false);
                    if let Some(handlers) = handlers.upgrade() {
                        handlers
                            .borrow_mut()
                            .retain(|(registered, _)| !std::rc::Rc::ptr_eq(registered, &active));
                    }
                })
            }

            #[allow(dead_code)]
            fn on_property_changed(&self, property: #component_property_enum) {
                let handlers = self.__property_changed_handlers.borrow().clone();
                for (active, handler) in handlers {
                    if active.get() {
                        handler(property);
                    }
                }
            }

            #accessors
            #methods
        }
    }
}

/// Emits every `MethodDef` (§3's `#[virtual]`/`#[override]` hooks, plus their `__base_<name>`
/// shadow copies — see `resolve_effective_methods`) as an ordinary inherent method. A shadow copy
/// (its mangled name starting with `__base_`) is kept private — it exists only to be called via a
/// `base::name(...)`-rewritten `self.__base_name(...)`, never part of the type's public surface.

fn emit_methods(methods: &[MethodDef]) -> TokenStream {
    let mut out = TokenStream::new();
    for m in methods {
        let name = format_ident!("{}", m.name);
        let vis = if m.name.starts_with("__base_") {
            quote! {}
        } else {
            quote! { pub }
        };
        let params = m.params.iter().map(|(n, ty)| {
            let ident = format_ident!("{}", n);
            quote! { #ident: #ty }
        });
        let ret = match &m.return_ty {
            Some(ty) => quote! { -> #ty },
            None => quote! {},
        };
        let body = &m.body;
        out.extend(quote! {
            #vis fn #name(&self, #(#params),*) #ret #body
        });
    }
    out
}

/// Emits component companion methods inside the generated `#[class]` impl. The effective method
/// list contains inherited methods and private `__base_` shadows as well as this component's own
/// declarations, so only the latter may retain the source method classification. This keeps the
/// component-to-class bridge generic: the `#[class]` macro remains the sole owner of virtual method
/// routing and receives the same metadata it would receive from a hand-written class impl. The
/// matching `#[inherent]` copy for an own virtual/override method preserves the pre-bridge concrete
/// component API; it executes the authored body directly and is not part of host trait dispatch.
fn emit_class_methods(methods: &[MethodDef], own_methods: &[MethodDef]) -> TokenStream {
    let own_names: HashSet<&str> = own_methods
        .iter()
        .map(|method| method.name.as_str())
        .collect();
    let mut out = TokenStream::new();
    for method in methods {
        let name = format_ident!("{}", method.name);
        let (attr, vis) = if method.name.starts_with("__base_") {
            (quote! { #[inherent] }, quote! {})
        } else if own_names.contains(method.name.as_str()) && method.is_virtual {
            (quote! { #[overridable] }, quote! {})
        } else if own_names.contains(method.name.as_str()) && method.is_override {
            (quote! { #[overrides] }, quote! {})
        } else {
            (quote! { #[inherent] }, quote! { pub })
        };
        let params: Vec<_> = method
            .params
            .iter()
            .map(|(param_name, ty)| {
                let ident = format_ident!("{}", param_name);
                quote! { #ident: #ty }
            })
            .collect();
        let ret = match &method.return_ty {
            Some(ty) => quote! { -> #ty },
            None => quote! {},
        };
        let body = &method.body;
        out.extend(quote! {
            #attr #vis fn #name(&self, #(#params),*) #ret #body
        });
        if own_names.contains(method.name.as_str()) && (method.is_virtual || method.is_override) {
            out.extend(quote! {
                #[inherent]
                pub fn #name(&self, #(#params),*) #ret {
                    #body
                }
            });
        }
    }

    // `resolve_effective_methods` can provide a private `__base_<name>` shadow when the
    // immediate ancestor is another generated component. A hand-written class ancestor does not
    // participate in that component-only flattening, but an override body is rewritten to the
    // same `self.__base_<name>(..)` surface regardless of where the ancestor method originated.
    // Fill only the missing shadows by forwarding through this component's concrete `base` field;
    // the class macro remains the sole owner of the virtual dispatch chain, and no type/method
    // names are consulted here.
    let existing_shadows: HashSet<&str> = methods
        .iter()
        .filter(|method| method.name.starts_with("__base_"))
        .map(|method| method.name.as_str())
        .collect();
    for method in own_methods
        .iter()
        .filter(|method| method.is_override)
        .filter(|method| !existing_shadows.contains(format!("__base_{}", method.name).as_str()))
    {
        let name = format_ident!("{}", method.name);
        let shadow_name = format_ident!("__base_{}", method.name);
        let params: Vec<_> = method
            .params
            .iter()
            .map(|(param_name, ty)| {
                let ident = format_ident!("{}", param_name);
                quote! { #ident: #ty }
            })
            .collect();
        let args: Vec<_> = method
            .params
            .iter()
            .map(|(param_name, _)| format_ident!("{}", param_name))
            .collect();
        let ret = match &method.return_ty {
            Some(ty) => quote! { -> #ty },
            None => quote! {},
        };
        out.extend(quote! {
            #[inherent]
            fn #shadow_name(&self, #(#params),*) #ret {
                self.base.#name(#(#args),*)
            }
        });
    }
    out
}

/// Where a path/method-call expression is being emitted: during initial widget construction
/// (before `Rc<Self>` exists — the injected param, e.g. `vm`, is only reachable as a bare local
/// variable) or afterwards, inside a stored closure or `resync()`, where it hangs off a
/// `Rc<Self>` token (`self`/`this`).
#[derive(Clone)]
enum EmitMode {
    Construction,
    WithSelf(TokenStream),
}

impl EmitMode {
    fn owner_tokens(&self, owner: &str) -> TokenStream {
        let owner_ident = format_ident!("{}", owner);
        match self {
            EmitMode::Construction => quote! { #owner_ident },
            EmitMode::WithSelf(self_tok) => quote! { #self_tok.#owner_ident },
        }
    }
}

// PR #169 review remediation, round 3 (A2/AD-R3-2/AD-R3-4): `source_component` is the literal,
// un-flattened `ComponentDef` this Component's own source actually declares — the only input
// `component_public_shape` may ever receive (AD-R3-2). `component` (unchanged parameter
// name/position) remains the effective/possibly-`effective_fields`-flattened `ComponentDef` every
// other, non-shape decision in this function already uses — inherited-field
// forwarding/composition/storage stays real generation's own job (AD-R3-3), never routed through
// the shape.
fn generate_view(
    view: &ViewDef,
    source_component: &ComponentDef,
    component: &ComponentDef,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let target_name = view.target.clone();
    let target = format_ident!("{}", target_name);
    let target_ext = format_ident!("{}Ext", target_name);
    let has_own_view = find_view(from, &target_name).is_some();
    // A component-level `template: template_view!` is an explicit typed template declaration.
    // Environment selection is performed through the generic ControlTemplate<C> slot; no
    // per-control Environment key or component-name lookup participates in this decision.
    let is_control_template_enabled = view.is_template;

    // `component X inherits Y` where `Y` is a virtual-builtin shape primitive (`Control`/
    // `Rectangle`/`Ellipse`/`TextBlock`/`Grid`/`VerticalLayout`/`HorizontalLayout` —
    // `is_virtual_builtin`) and `X`'s own view root is literally a construction of `Y`
    // (`validate::validate_inherits` already enforces this) — the real, load-bearing case of
    // docs/design/runtime/ui_tree_design.md's `struct XImpl { base: YImpl, .. }` composition: `X`'s
    // generated struct embeds `Y`'s real `elwindui::core::ui` `YImpl` as its own `base` field and
    // implements `UIElement` (and `Y`'s own trait) by delegating to it, instead of the ordinary
    // "wrapper owns a separately-`Rc`-erased root" shape every other `view`-having component uses
    // (see this function's tail `quote!`).
    let composed_shape = table
        .resolve(from, &target_name)
        .and_then(|i| i.composed_shape.clone());
    let is_shape_composition = has_own_view && composed_shape.is_some();
    // A component without its own view reuses the composed base value directly. Components with an
    // own view inherit behavior but retain their independently constructed root.
    let is_inherited_view_composition = !has_own_view && composed_shape.is_some();
    // `component X inherits Y` where `Y` is a hand-written native host with no `UIElement`
    // implementation of its own (only `Window` today) and `X`'s own view root literally constructs
    // `Y` — "host composition" (docs/design/runtime/ui_tree_design.md, `TypeInfo::host_composition_base`).
    // Follows the same `base`-field/`XImpl`-rename/synthesized-trait shape as shape composition
    // below, just without an `impl UIElement` (`Y` doesn't implement it either) — see this
    // function's dedicated branch further down.
    let host_composition_base = table
        .resolve(from, &target_name)
        .and_then(|i| i.host_composition_base.clone());
    let is_host_composition = host_composition_base.is_some();
    let is_composed = composed_shape.is_some() || is_host_composition;
    // `#[class]` derives an `XExt` trait from the component struct `X`.
    let struct_ident = target.clone();

    // The component's own `#[param]`-shaped fields (no initializer) become `new`'s positional
    // arguments and private struct fields — e.g. `NotepadWindow`'s `#[param] #[inject] vm:
    // NotepadViewModel`, or `DocumentView`'s `#[param] #[inject] doc: Rc<DocumentViewModel>`.
    // Maps to each field's own declared type string (not just its name) so a virtual builtin's
    // `get_attr`/`get_attr_string` (`emit_virtual_construction`) can tell "an already-`Option<T>`
    // own field forwarded as-is" (e.g. `ContentControl`'s `padding: padding` forwarded into
    // `Control { padding: padding }`) apart from "a plain value that itself needs `Some(..)`
    // wrapping" (e.g. a literal `padding: 8.0`) — forwarding the former through the latter's
    // wrapping convention would double-wrap into `Option<Option<T>>`.
    let mut own_fields: std::collections::HashMap<String, String> = component
        .fields
        .iter()
        .filter(|f| f.initializer.is_none())
        .map(|f| (f.name.clone(), f.ty.clone()))
        .collect();

    // A component's own `#[prop(default = expr)]`/`#[computed(expr = expr)]` fields — unlike every
    // category above, these carry a real initializer expression, but (unlike `viewmodel`'s
    // `#[observable]`/`#[computed]`, `generate_viewmodel` above) they weren't recognized as "one of
    // this component's own fields" by anything downstream at all until this block: not stored on
    // the struct, no accessor, and (critically) invisible to `emit_expr`'s bare-identifier
    // resolution (`ctx.own_fields`), which used to make a same-component reference like `text:
    // label` fail with "unsupported path shape after bind resolution". Mirrors two things already
    // proven out elsewhere in this file rather than inventing a third mechanism: `mutable_required_names`
    // below (Cell/RefCell storage + a generated `{Component}Property` enum + per-property `resync`,
    // for a component's own *required* mutable `prop` fields) and `generate_viewmodel`'s own
    // `dependents_of`-driven Computed-field cascade (this function's sibling above).
    let field_names: HashSet<&str> = component.fields.iter().map(|f| f.name.as_str()).collect();
    let source_field_names: HashSet<&str> = source_component
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    let own_default_fields: Vec<&FieldDef> = component
        .fields
        .iter()
        .filter(|f| {
            (!is_shape_composition || source_field_names.contains(f.name.as_str()))
                && f.kind == FieldKind::Prop
                && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
        .collect();
    let own_state_fields: Vec<&FieldDef> = component
        .fields
        .iter()
        .filter(|f| {
            (!is_shape_composition || source_field_names.contains(f.name.as_str()))
                && f.kind == FieldKind::State
                && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
        .collect();
    let own_stored_fields: Vec<&FieldDef> = own_default_fields
        .iter()
        .chain(own_state_fields.iter())
        .copied()
        .collect();
    let own_computed_fields: Vec<&FieldDef> = component
        .fields
        .iter()
        .filter(|f| {
            (!is_shape_composition || source_field_names.contains(f.name.as_str()))
                && f.kind == FieldKind::Computed
                && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
        .collect();
    // `#[environment(name)]` fields — see `own_computed_fields`'s own construction-time/storage
    // shape just above; unlike it, the initial (and every later) value comes from the ambient
    // `EnvironmentContext` (`own_environment_construct_stmts`, below), not a declared expression.
    let own_environment_fields: Vec<&FieldDef> = component
        .fields
        .iter()
        .filter(|f| {
            (!is_shape_composition || source_field_names.contains(f.name.as_str()))
                && f.kind == FieldKind::Environment
        })
        .collect();
    own_fields.extend(
        own_stored_fields
            .iter()
            .chain(own_computed_fields.iter())
            .chain(own_environment_fields.iter())
            .map(|f| (f.name.clone(), f.ty.clone())),
    );
    // Dependency graph (mirrors `generate_viewmodel`'s own `dependents_of`, `codegen.rs` above) so
    // that setting an own defaulted-prop field cascades into recomputing + notifying every own
    // computed field that depends on it — scoped to this component's own fields only (a computed
    // field's expression may also reference a `#[param]` field, which never changes after
    // construction and so needs no cascade entry).
    let mut own_dependents_of: HashMap<String, Vec<String>> = HashMap::new();
    for f in &own_computed_fields {
        if let Some(Initializer::Expr(expr)) = &f.initializer {
            for dep in referenced_fields(expr, &field_names) {
                own_dependents_of
                    .entry(dep)
                    .or_default()
                    .push(f.name.clone());
            }
        }
    }
    // Own defaulted-prop/computed fields are read as plain bare identifiers (`label`, not
    // `vm.label`) inside this component's own view — including while the view's root element tree
    // is still being *constructed* (`EmitMode::Construction`, before `self`/`Rc<Self>` exists,
    // exactly like a `#[param]` field's own ctor argument). Since a plain Rust struct field has no
    // way to carry a default value expression as a real `new(..)` computation step, each one is
    // instead seeded by a `let <name> = <expr>;` statement emitted up front, before any element gets
    // constructed — `emit_expr`'s existing own-field bare-path branch already resolves a
    // `Construction`-mode reference to a plain local identifier, so no changes are needed there
    // beyond making `ctx.own_fields` aware of these names (done above). Unlike the `recompute_<name>`
    // methods generated below (which run later, as `&self` methods, and so rewrite sibling
    // references to `self.<field>()`), this initial computation runs before `self` exists, so
    // sibling references here are deliberately left as bare identifiers — already valid Rust once
    // each is its own preceding `let`. Field order in the source is trusted to already put
    // dependencies before dependents (the same assumption DSL authors already have to satisfy
    // for a `#[computed]` field to read sensibly top-to-bottom); this doesn't topologically sort.
    let mut own_default_construct_stmts = TokenStream::new();
    for f in component.fields.iter().filter(|f| {
        (!is_shape_composition || source_field_names.contains(f.name.as_str()))
            && matches!(f.initializer, Some(Initializer::Expr(_)))
            && matches!(
                f.kind,
                FieldKind::Prop | FieldKind::State | FieldKind::Computed
            )
    }) {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
        let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
            unreachable!("filtered to Some(Initializer::Expr(_)) above");
        };
        let init_expr = rewrite_t_macro_bare(coerce_to_owned_string(&f.ty, raw_expr.clone()));
        own_default_construct_stmts.extend(quote! { let #field_ident: #ty = #init_expr; });
    }
    // `#[environment(name)]` fields no longer resolve here (CI-5 of #80,
    // docs/design/runtime/component_lifecycle_design.md §4d) — real resolution happens in
    // `__build_view()`, from the `EnvironmentContext` `mount()` was actually called with
    // (`self.__mount_environment`), not the ambient thread-local. `construct()`/`Self { .. }` still
    // needs *some* value to seed each field's `Cell`/`RefCell` with (a struct literal can't leave a
    // field unset), so this seeds the same fallback `EnvironmentContext::get::<K>()` itself would
    // return for an unmounted/un-overridden key — `K::default_value()` — never a real resolution.
    for f in &own_environment_fields {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
        let (key_type_preamble, key_type) = environment_key_type(f);
        own_default_construct_stmts.extend(quote! {
            #key_type_preamble
            let #field_ident: #ty =
                <#key_type as elwindui::core::environment::EnvironmentKey>::default_value();
        });
    }

    // `mutable_own_fields` is populated below, once `mutable_required_names` is known (it needs
    // `required_own_names`/`deferred_own_names`, computed further down using `ctx.own_fields`
    // itself) — every `emit_expr`/`plan_element`/`emit_construction`/`emit_resync` call that could
    // actually observe it happens later still, so setting it after the fact here is sound.
    let template_base_fields = if view.is_template {
        table
            .resolve(from, &target_name)
            .map(|info| {
                info.declaring_types
                    .iter()
                    .filter(|(_, owner)| owner.as_str() != target_name.as_str())
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    let mut ctx = ViewCtx {
        closure_param: None,
        own_fields,
        mutable_own_fields: HashSet::new(),
        bindable_owners: HashSet::new(),
        weak_bindable_owners: HashSet::new(),
        // A component-declared `template: template_view!` is compiled as the default
        // `ControlTemplate<Self>` factory.  Its `templated_parent` is therefore the component
        // instance itself at runtime, while named/external template factories keep an explicit
        // weak `templated_parent` field.  Keep this distinction in the generic expression
        // resolver rather than rewriting the AST or introducing a template-specific shortcut.
        default_template_parent: view.is_template && !view.template_instance,
        template_base_fields,
        implicit_owner: view.implicit_owner.as_ref().map(ImplicitOwnerCtx::from),
        target: target.clone(),
        template_parent: None,
        template_property_bounds: None,
        template_target: None,
        template_bare_parent_fields: HashSet::new(),
        storage: ViewStorage::Component,
    };

    // All explicit ControlTemplate bodies (component defaults and named template instances) are
    // compiled through the same semantic body backend used by `template_view!`.  The surrounding
    // component generation below still owns storage/class wiring; only the template's visual
    // factory is supplied by this shared layer.
    let shared_template_body = view.is_template.then(|| {
        crate::compile_template_body(
            &view.root,
            &view.lets,
            view.on_mount.as_ref(),
            view.on_unmount.as_ref(),
            view.on_update.as_ref(),
            from.clone(),
            table.clone(),
            quote! { #target },
            component
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "{}: shared ControlTemplate body compilation failed: {error}",
                view.target
            )
        })
    });

    // PR #169 review remediation, round 2 (AD-R2-6), input corrected round 3 (A2/AD-R3-2/AD-R3-3):
    // `component_public_shape` is the single source-local classification of which no-initializer
    // *own* fields are constructor-eligible at all — computed once, here, before
    // `param_names`/`param_types`, from `source_component` (the literal, un-flattened `ComponentDef`
    // this Component's own source declares — never `component`, the *synthetic*,
    // already-`effective_fields`-flattened one this function's other, non-shape logic still uses
    // throughout). `component_public_shape` is source-local by design (AD-R3-2): it must never be
    // handed an ancestor-inclusive field list, which would make it treat an inherited field as
    // though this Component declared it itself.
    let own_field_shape =
        crate::component_frontend::component_public_shape(source_component, Some(view));
    let declared_own_field_names: HashSet<&str> = source_component
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    // A shape-composed component delegates inherited fields to its concrete `base` value. They
    // remain in the effective field map above so authored expressions can resolve them, but must
    // not be re-stored or re-exposed as a second set of accessors on the derived type.
    let inherited_shape_field_names: HashSet<String> = if is_shape_composition {
        component
            .fields
            .iter()
            .filter(|f| !declared_own_field_names.contains(f.name.as_str()))
            .map(|f| f.name.clone())
            .collect()
    } else {
        HashSet::new()
    };
    // `on_*`-named fields are excluded because a `#[routed]` field (`UIElement`'s own
    // `on_tapped`/`on_pointer_pressed`/... — inherited by every component through
    // `resolve_effective_fields`, not just ones that declare it directly, e.g. `Button.on_click`) is
    // wired through the `on_x: ..` DSL attribute + `register_routed_handler` (`emit_wiring`'s own
    // `is_routed` branch), never as a positional constructor argument — before this exclusion
    // existed, every `has_view` composed component's `new(..)` silently gained 9 required
    // `fn(PointerEventArgs)`-typed parameters nothing ever supplied, breaking every existing call
    // site the moment these fields became inheritable (`RoundedPanel`/`DocumentView`, e.g.).
    // `#[environment(name)]` fields never appear in `constructor_params`/`deferred_option_fields`
    // either — see `build_symbol_table`'s matching `param_fields` filter (`codegen.rs`,
    // `Item::Component` arm) for the parallel reasoning on the `TypeInfo` side.
    let shape_param_eligible_names: HashSet<&str> = own_field_shape
        .constructor_params
        .iter()
        .map(|(name, _)| name.as_str())
        .chain(
            own_field_shape
                .deferred_option_fields
                .iter()
                .map(|(name, _, _)| name.as_str()),
        )
        .collect();
    // Field-level membership: an *own* field (in `source_component.fields`) is decided by the shape
    // above; an *inherited* one (present in `component.fields`, the effective set, but not
    // `source_component.fields` — a base's own field this Component never redeclares) is decided by
    // the original, pre-shape direct predicate unchanged (AD-R3-3: inherited-field forwarding stays
    // real generation's own job, never routed through the source-local shape).
    let is_param_eligible = |f: &FieldDef| -> bool {
        if declared_own_field_names.contains(f.name.as_str()) {
            shape_param_eligible_names.contains(f.name.as_str())
        } else {
            f.initializer.is_none()
                && !f.name.starts_with("on_")
                && f.kind != FieldKind::Environment
        }
    };
    let param_names: Vec<syn::Ident> = component
        .fields
        .iter()
        .filter(|f| is_param_eligible(f))
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let param_types: Vec<syn::Type> = component
        .fields
        .iter()
        .filter(|f| is_param_eligible(f))
        .map(|f| syn::parse_str(&f.ty).expect("field type must parse"))
        .collect();

    // Only meaningful when `is_inherited_view_composition`: `resolve_effective_fields` gives this
    // component *every* field of its (already-composed) base unconditionally when it writes no
    // `view` of its own, in the base's own declaration order, followed by any genuinely new field
    // this component adds on top — so the base's own params are always exactly `param_names`'s
    // leading slice. Forwarded verbatim to the base's own `create_<snake case>(..)` factory to build
    // this component's `base` field; the (usually empty) remainder are this component's own extra
    // struct fields, unrelated to the base.
    let base_param_count = component
        .base
        .as_deref()
        .and_then(|base| table.resolve(from, base))
        .map(|info| info.param_fields.len())
        .unwrap_or(0);
    let forward_param_names = &param_names[..base_param_count.min(param_names.len())];
    // For `is_inherited_view_composition`: the forwarded params above are fully consumed building `base`
    // (`field_inits`'s `base: create_<base>(..)`) — storing them *again* as this component's own
    // top-level struct fields (the ordinary, non-composed shape every other component uses) would
    // both duplicate the data pointlessly and, since they're passed by value (not `.clone()`d) into
    // the base factory, be a use-after-move compile error. Only the genuinely-new fields this
    // component adds beyond its base (rare — empty for `LabeledPanel`) become its own struct fields;
    // reads of a forwarded name instead delegate to `self.base.<name>()` (`named_accessors`, below).
    let mut own_struct_param_names: Vec<syn::Ident> = if is_inherited_view_composition {
        param_names[base_param_count.min(param_names.len())..].to_vec()
    } else {
        param_names.clone()
    };
    // Assigned below once `shape_forwarded_names` is known (`is_shape_composition` narrows this
    // further still), from `own_struct_param_names`'s own final value — see there.
    let own_struct_param_types: Vec<syn::Type>;

    // Every `#[bindable]` field (`ast::Attr::Bindable`'s own doc comment) gets one auto-refreshing
    // `PropertyChanged` subscription, dispatching by property *name* through
    // `elwindui::core::reactive::ObservableExt` rather than a per-viewmodel-typed enum — deliberately
    // a syntactic marker rather than inferred from whether the field's type happens to resolve as a
    // `viewmodel` in *this* compilation's symbol table: `#[elwindui::component]`'s own macro
    // invocation never has symbol-table visibility into a `viewmodel` declared by a separate
    // `#[elwindui::viewmodel]` invocation (each proc-macro expansion only ever sees its own tokens),
    // so relying on resolved-type inference would silently produce no subscription at all in
    // exactly that case. This covers both a field referenced only through `owner.field`
    // from another field's initializer, and one referenced directly in the view body (e.g.
    // `vm.active_tab`) — either way, "does this field need a subscription" now depends solely on
    // whether *it itself* is `#[bindable]`, not on how other fields/expressions reference it.
    // `templated_parent` (`ControlTemplate`, always explicit-qualification-only) and
    // `__view_owner` (Issue #162's deferred-view lowering, additionally an implicit bare-name
    // fallback — `ctx.implicit_owner`, `emit_expr`'s own `ViewExpr::Path` handling) are both
    // reserved weak-owner field names, generalizing the same "owner is `Weak<T>`, reads upgrade
    // it, its own properties are still ordinary reactive dependencies" mechanism.
    let is_reserved_weak_owner = |f: &&FieldDef| {
        matches!(f.name.as_str(), "templated_parent" | "__view_owner") && is_weak_type(&f.ty)
    };
    let bind_owners: Vec<syn::Ident> = component
        .fields
        .iter()
        .filter(|f| {
            f.attrs.iter().any(|a| matches!(a, Attr::Bindable)) || is_reserved_weak_owner(f)
        })
        .map(|f| format_ident!("{}", f.name))
        .collect();
    ctx.bindable_owners = bind_owners.iter().map(ToString::to_string).collect();
    ctx.weak_bindable_owners = component
        .fields
        .iter()
        .filter(is_reserved_weak_owner)
        .map(|f| f.name.clone())
        .collect();
    // PR #165 post-final rereview remediation, A9 (§10): a source-Component `#[bindable]` field
    // (e.g. `vm`) referenced directly inside a lowered `DeferredView` (`vm.label`) is never a
    // physical field of the hidden Component — it can never become a real `bind_owners` entry,
    // whose `subscribe_stmts`/`property_resync_methods_for` machinery assumes `self.#owner_ident`
    // is a genuine struct field to subscribe/upgrade. It still needs the exact same subscription/
    // resync machinery, bridged through the source lexical owner (`__view_owner.upgrade().vm()`)
    // instead of a physical field — see `implicit_bindable_subscribe_stmts`, below, and the
    // `property_resync_methods_for(&implicit_bind_owners, ..)` call alongside the ordinary one.
    // Excludes any name already resolvable as a real own field/bind owner of *this* Component (an
    // actual own field of the same name always wins — mirrors `path_owner_value_tokens`'s own
    // `ctx.own_fields` check).
    let implicit_bind_owners: Vec<syn::Ident> = ctx
        .implicit_owner
        .as_ref()
        .map(|implicit| {
            implicit
                .bindable_fields
                .iter()
                .filter(|name| !ctx.own_fields.contains_key(name.as_str()))
                .map(|name| format_ident!("{}", name))
                .collect()
        })
        .unwrap_or_default();
    // `templated_parent` (`ControlTemplate`) triggers this for its own, narrower reason (a
    // "selected once by an already-mounted target, before `Self` exists" lifecycle). A hidden
    // Component lowered from a `ViewExpr::DeferredView` (`view.implicit_owner`, Issue #162) needs
    // this same machinery for a different reason: it's `mount()`-ed with the popup-scoped derived
    // `ctx.environment`, not `application_environment()` — and an *ordinary* nested `has_view`
    // child (`node.environment_scope.is_none()`, `emit_construction`'s `None` arm) constructs via
    // plain `Type::new()`, which unconditionally self-mounts against `application_environment()`,
    // never its own parent's mount-time environment. Without opting into the same
    // `node.environment_scope` propagation `ControlTemplate`'s own replaceable body already uses,
    // a nested Component inside a popup's declarative content (e.g. one reading
    // `#[environment(popup_dismiss)]`) would silently observe the *global* Environment instead of
    // the popup-scoped one — confirmed by `declarative_context_popup_dismiss_during_on_mount_
    // prevents_popup_from_showing` failing without this. The `ContentPresenter`-binding branch
    // below (§ "content_capture_stmt/content_attach_stmt") stays independently gated on
    // `templated_parent` specifically, so enabling this for `__view_owner` too does not affect it.
    let is_template_or_deferred_scope = is_control_template_enabled
        || ctx.weak_bindable_owners.contains("templated_parent")
        || view.implicit_owner.is_some();

    // Every node that has a callback or a value that can change after construction gets a
    // generated field name and is stored on the component so `resync`/closures can reach it later.
    let mut plan = Vec::new();

    // `let`-bindings (§13): planned, in source order, *before* `root` so a later `let`'s own
    // element (or `root` itself) can reference an earlier one via a bare `ChildEntry::Ref`.
    // `is_root: let_binding.id.is_some()` reuses `plan_element`'s existing "force `stored`" flag —
    // an `#[id(...)]`-tagged binding must survive past construction the same way a literal root
    // element already does (`emit_named_accessors` reads `self.<binding>` later), even though it
    // isn't the view's actual root.
    let mut lets_map: HashMap<String, (syn::Ident, String)> = HashMap::new();
    // A `dyn UIElement`-typed `#[param]` field (e.g. `ContentControl`'s `content`) is already a
    // fully-constructed `Rc<dyn UIElement>` value by the time it reaches this view's body, with no
    // component type name of its own left to resolve — unlike a literal nested element or a `let`,
    // it can't be re-planned via `plan_element`. Seeding `lets_map` with it here lets a bare
    // reference to it in `{}` (e.g. `ContentControl`'s `Control { content }`) resolve via the
    // ordinary `ChildEntry::Ref` path, tagged with `PASSTHROUGH_NODE` so `into_node_if_needed` uses
    // it as-is instead of trying to resolve it via `SymbolTable`.
    for field in &component.fields {
        if field.initializer.is_none() && is_ui_element_type(&field.ty) {
            lets_map.insert(
                field.name.clone(),
                (
                    format_ident!("{}", field.name),
                    PASSTHROUGH_NODE.to_string(),
                ),
            );
        }
    }
    for let_binding in &view.lets {
        let resolved = plan_element(
            &let_binding.element,
            &ctx,
            from,
            table,
            &mut plan,
            let_binding.id.is_some(),
            &lets_map,
        );
        if let_binding.id.is_some() {
            plan.last_mut()
                .expect("plan_element always pushes its own node")
                .id = let_binding.id.clone();
        }
        lets_map.insert(let_binding.name.clone(), resolved);
    }

    // Phase 0 (docs/design/runtime/ui_tree_design.md's "inherits" section): a composable `base` (virtual
    // builtin / already-composed DSL component / hand-written native host) has no wrapper element
    // written in `view`'s body anymore — the body's own attributes/children directly *are* `base`'s
    // — so the concrete root `ElementNode` `plan_element` (and everything below) still expects is
    // synthesized here, once, from `view.root: ast::ViewBody`. An ordinary (non-composing)
    // component's body must instead reduce to exactly one literal child; `validate::validate`
    // reports that case as a real diagnostic; this is a second, codegen-level guarantee that holds
    // even if this function is ever called on unvalidated input (mirrors `is_abstract`'s own
    // `continue` in `generate_module` just above).
    let resolved_root = resolve_view_root_element(
        &view.root,
        component.base.as_deref(),
        is_composed,
    )
    .unwrap_or_else(|| {
        panic!(
            "{}: view root must be exactly one element unless it inherits a composable base",
            component.name
        )
    });

    // Property/content lowering is selected by the effective root/base props macro. Local-vs-
    // external resolution only determines where that macro is exported; template presentation is
    // selected solely by the explicit `template: template_view!` authoring slot above.
    let root_props_macro_path = dsl_props_macro_path(
        &resolved_root.type_path,
        resolve_context_info(&ctx, from, table, &resolved_root.type_path),
    );

    plan_element(
        &resolved_root,
        &ctx,
        from,
        table,
        &mut plan,
        true,
        &lets_map,
    );

    let template_environment_ident = format_ident!("__control_template_environment");
    if is_template_or_deferred_scope {
        for node in &mut plan {
            if node.environment_scope.is_none()
                && table
                    .resolve(from, &node.type_path)
                    .is_some_and(|info| info.has_view)
            {
                node.environment_scope = Some(template_environment_ident.clone());
            }
        }
    }

    // Host composition (`is_host_composition`'s doc comment): the root's stored field must be
    // named `base` (the same trait+Impl+base convention `is_shape_composition` follows), not the
    // generic auto-numbered binding every other stored node gets — renamed here, before anything
    // below reads `node.binding`, so the ordinary "stored field" path (`struct_fields`/
    // `field_inits`), `emit_wiring`, and `emit_resync` all naturally reference `self.base` with no
    // further special-casing (unlike shape composition, the root here is still built by ordinary
    // `emit_construction`, so there's no separate construction path to intercept — only storage).
    if is_host_composition {
        plan.last_mut()
            .expect("plan_element always pushes a node for the root")
            .binding = format_ident!("base");
    }

    // A shape-composition root's authored children are attached after the outer `Rc` exists,
    // through the root's effective `#[content(...)]` property. The child therefore has to survive
    // the root's plain-value construction whenever it is a real (non-dynamic) child. This is
    // intentionally derived from content metadata rather than from the root type's name: a scalar
    // slot, a `Vec`, and a live collection all use the same ownership boundary here.
    if is_shape_composition {
        let bindings: Vec<_> = plan
            .last()
            .map(|root| {
                root.child_bindings
                    .iter()
                    .filter(|(_, ty)| *ty != DYNAMIC_CHILD_SLOT_MARKER)
                    .map(|(binding, _)| binding.clone())
                    .collect()
            })
            .unwrap_or_default();
        for binding in bindings {
            if let Some(node) = plan.iter_mut().find(|n| n.binding == binding) {
                node.stored = true;
            }
        }
    }

    // `is_shape_composition`'s own analog of `is_inherited_view_composition`'s `forward_param_names`:
    // which of this component's own params are bare-forwarded (`fill: fill`) straight into the
    // shape-composition root's construction (`build_virtual_value`/`build_component_value`) —
    // consumed there by move (`EmitMode::Construction`'s bare-identifier emission, see `emit_expr`'s
    // `ctx.own_fields`-bare-path branch), unlike `is_inherited_view_composition`'s always-Copy `padding`
    // case. Rectangle's `fill`/`stroke`/`stroke_width` (`Option<String>`/`Option<f32>`, forwarded
    // verbatim into `Shape { fill: fill, .. }`) are the motivating case: storing them *again* as
    // `RectangleImpl`'s own top-level fields (the ordinary shorthand every other param gets) would be
    // a use-after-move compile error, exactly like `is_inherited_view_composition`'s forwarded fields.
    // Detected structurally (a 1-segment `ViewExpr::Path` attribute on the root element exactly
    // equal to the param's own name), but only for non-`Copy` fields (`Option<String>`'s `fill`/
    // `stroke`, say) — a `Copy` field forwarded the same way (`stroke_width: Option<f32>`,
    // `padding: Option<f32>`) is harmless to also keep as its own struct field (no move to avoid),
    // and *must* be kept: the underlying `elwindui::core::ui` base field it forwards into is often
    // a narrower stored shape (`ShapeImpl::stroke_width`/`ControlImpl::padding` are plain `f32`, not
    // `Option<f32>` — `build_virtual_value`'s `get_attr` unwraps via `.unwrap_or(0.0)` before
    // storing), so delegating its accessor to `self.base.<name>` would return the wrong type.
    let shape_forwarded_names: HashSet<String> = if is_shape_composition {
        let root_node = plan
            .last()
            .expect("plan_element always pushes a node for the root");
        param_names
            .iter()
            .map(|n| n.to_string())
            .filter(|name| {
                let is_bare_forward =
                    matches!(find_attr(root_node, name), Some(ViewExpr::Path(p)) if p.as_slice() == [name.clone()]);
                let ty = ctx.own_fields.get(name);
                // A `synthesize_external_base_fields`-synthesized field's `ty` is a type-position
                // macro invocation (`{Base}!(@field_type {name})`, always containing a literal `!`
                // no ordinary Rust type spelling in this codebase ever does), opaque to
                // `is_copy_type`'s string matching — it always answers "not Copy" for a string it
                // doesn't recognize, which would otherwise wrongly move such a field out of
                // `own_struct_param_names` here and into the `self.base.#name.borrow().clone()`
                // accessor branch below (assuming `RefCell`-backed storage that may not exist —
                // `Control::padding` is `Cell<f32>`, Refs #90). Never necessary to forward one of
                // these regardless of the field's real Copy-ness: unlike `build_virtual_value`'s
                // local-`TypeInfo` construction (which *moves* a bare-forwarded value, motivating
                // this whole mechanism), a synthesized field's root is always genuinely external —
                // `emit_construction` always routes it through `emit_external_construction`, whose
                // `emit_external_attribute_sets` already unconditionally `.clone()`s any bare-
                // forwarded own field (`bare_own_field_type`'s own branch there) — so keeping it as
                // its own struct field too is always safe, exactly like a real Copy field.
                let is_synthesized_external_ty = ty.is_some_and(|ty| ty.contains('!'));
                let is_copy = is_synthesized_external_ty
                    || ty.is_some_and(|ty| is_copy_type(strip_option(ty).0));
                is_bare_forward && !is_copy
            })
            .collect()
    } else {
        HashSet::new()
    };
    own_struct_param_names.retain(|n| {
        !shape_forwarded_names.contains(&n.to_string())
            && !inherited_shape_field_names.contains(&n.to_string())
    });
    let own_struct_param_names_set: HashSet<String> = own_struct_param_names
        .iter()
        .map(|n| n.to_string())
        .collect();
    own_struct_param_types = param_names
        .iter()
        .zip(param_types.iter())
        .filter(|(n, _)| own_struct_param_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();

    // Unreferenced own `Option<T>` fields are initialized as `None` and exposed through
    // `set_<name>`. Fields needed while constructing the view remain constructor arguments.
    //
    // PR #169 review remediation (AD-R4); source corrected round 3 (A2/AD-R3-2): for a field
    // declared directly on `source_component` (not a `synthesize_external_base_fields`-synthesized
    // one, Refs #90, and not merely present in `component`'s own `effective_fields`-flattened list
    // — `component_public_shape` is source-local only and has no notion of either), this reads
    // `component_frontend::component_public_shape`'s own deferred-field classification instead of
    // re-deriving the same `strip_option(..).1 && !view_references_name_anywhere(..)` rule
    // independently — the exact rust-analyzer Component struct shadow
    // (`rust_analyzer_shadow::build_component_struct_shadow`) consults, so the two can never
    // silently drift. A synthesized or inherited field (absent from `source_component.fields`) falls
    // back to the original direct computation unchanged. `own_field_shape`/`declared_own_field_names`
    // are computed once, above (before `param_names`), and reused here (AD-R2-6) rather than
    // recomputed.
    let shape_deferred_names: HashSet<String> = own_field_shape
        .deferred_option_fields
        .iter()
        .map(|(name, _, _)| name.clone())
        .collect();
    let is_deferred_own_field = |name: &syn::Ident| -> bool {
        let name_str = name.to_string();
        if declared_own_field_names.contains(name_str.as_str()) {
            return shape_deferred_names.contains(&name_str);
        }
        let ty_str = ctx
            .own_fields
            .get(&name_str)
            .expect("own_struct_param_names names one of ctx.own_fields' own keys");
        strip_option(ty_str).1 && !view_references_name_anywhere(view, &name_str)
    };
    let deferred_own_names: Vec<syn::Ident> = own_struct_param_names
        .iter()
        .filter(|n| is_deferred_own_field(n))
        .cloned()
        .collect();
    let deferred_own_inner_types: Vec<syn::Type> = deferred_own_names
        .iter()
        .map(|n| {
            let ty_str = ctx
                .own_fields
                .get(&n.to_string())
                .expect("own_struct_param_names names one of ctx.own_fields' own keys");
            syn::parse_str(strip_option(ty_str).0).expect("field inner type must parse")
        })
        .collect();
    let deferred_own_cell_types: Vec<TokenStream> = deferred_own_names
        .iter()
        .zip(deferred_own_inner_types.iter())
        .map(|(n, inner_ty)| {
            let ty_str = ctx.own_fields.get(&n.to_string()).unwrap();
            let cell_ty = if is_copy_type(strip_option(ty_str).0) {
                quote! { std::cell::Cell }
            } else {
                quote! { std::cell::RefCell }
            };
            quote! { #cell_ty<Option<#inner_ty>> }
        })
        .collect();
    let deferred_own_names_set: HashSet<String> =
        deferred_own_names.iter().map(|n| n.to_string()).collect();
    // The `Self { .. }`/`#struct_ident { .. }` construction shorthand (`#(#name,)*`) only works for
    // a field with a live local variable of the same name — still true for a required own field
    // (still a `new(..)` argument), but not a deferred one (no argument, no local variable at all),
    // which instead needs an explicit `#name: #cell_ty::new(None)` initializer built here once and
    // reused by both `new(..)`'s own inline construction and `create_<snake case>(..)` below.
    let required_own_names: Vec<syn::Ident> = own_struct_param_names
        .iter()
        .filter(|n| !deferred_own_names_set.contains(&n.to_string()))
        .cloned()
        .collect();
    let required_own_types: Vec<syn::Type> = own_struct_param_names
        .iter()
        .zip(own_struct_param_types.iter())
        .filter(|(n, _)| !deferred_own_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();
    let deferred_own_field_decls: TokenStream = deferred_own_names
        .iter()
        .zip(deferred_own_cell_types.iter())
        .map(|(name, cell_ty)| quote! { #name: #cell_ty, })
        .collect();
    let deferred_field_inits: TokenStream = deferred_own_names
        .iter()
        .zip(deferred_own_cell_types.iter())
        // `<#cell_ty>::new(..)`, not the bare `#cell_ty::new(..)` — a generic type's own associated
        // function called in *expression* position needs the qualified-path `<Type>::method()` form
        // (`Vec<i32>::new()` alone is ambiguous with a chained `<`/`>` comparison at this position;
        // only a type *annotation* context, e.g. `let x: Vec<i32> = ..`, allows the bare form).
        .map(|(name, cell_ty)| quote! { #name: <#cell_ty>::new(None), })
        .collect();
    // `new(..)`/`create_<snake case>(..)`'s own argument list — `param_names`/`param_types` (which
    // also includes any `forward_param_names` prefix, never deferred — see above) minus the
    // deferred subset.
    let ctor_param_names: Vec<syn::Ident> = param_names
        .iter()
        .filter(|n| {
            !deferred_own_names_set.contains(&n.to_string())
                && !inherited_shape_field_names.contains(&n.to_string())
        })
        .cloned()
        .collect();
    let ctor_param_types: Vec<syn::Type> = param_names
        .iter()
        .zip(param_types.iter())
        .filter(|(n, _)| {
            !deferred_own_names_set.contains(&n.to_string())
                && !inherited_shape_field_names.contains(&n.to_string())
        })
        .map(|(_, t)| t.clone())
        .collect();

    // A required own field (can't be deferred — `is_deferred_own_field` above already excluded it
    // because it's referenced somewhere in this component's own view) that's declared a plain
    // `prop` (not `#[param]`, docs/specs/dsl_spec.md §4) still needs to stay externally updatable
    // after construction — a `prop` field is runtime-mutable *by definition*, and "referenced at
    // construction time" doesn't change that (e.g. `RoundedPanel`'s `label`, used immediately to
    // build its own internal `TextBlock` but also meant to change on every `resync()` of whichever
    // *other* component instantiated it — `document_view.rs`'s `RoundedPanel { label:
    // t!("notepad-status-chars", count: doc.char_count) }`). Cell/RefCell-wrapped
    // (`is_copy_type`) like a deferred field's storage, but — unlike a deferred field — stays a
    // `new(..)` positional argument (its value is needed immediately, before `Self` exists) and its
    // setter also re-runs `self.resync()` (its own view, being required, is guaranteed to actually
    // reference it, so the change needs to reach the widgets built from it right away — see the
    // setter loop below).
    //
    // PR #169 review remediation, round 3 (AD-R3-5): for an *own* field (declared directly on
    // `source_component`), membership comes from `own_shape.writable_fields` — `required_own_names`
    // (the outer iteration) already establishes "required" (in `own_shape.constructor_params`, via
    // `param_names`'s own shape-driven filter above), so intersecting with `writable_fields` here is
    // exactly the "required + writable" the shape already models (`component_public_shape`'s own
    // doc comment: a required own `Prop` field's setter is `has_view`-only, i.e. exactly this
    // function). The forbidden pattern this replaces re-derived `FieldKind::Prop` membership
    // directly from `component.fields` (the effective/flattened set) instead of consuming the
    // shape. An inherited field (absent from `source_component.fields`) falls back to the original
    // direct `FieldKind::Prop` check unchanged (AD-R3-3: inherited-field forwarding stays real
    // generation's own job, never routed through the source-local shape).
    let shape_writable_names: HashSet<&str> = own_field_shape
        .writable_fields
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect();
    let mutable_required_names: Vec<syn::Ident> = required_own_names
        .iter()
        .filter(|n| {
            let name_str = n.to_string();
            if declared_own_field_names.contains(name_str.as_str()) {
                shape_writable_names.contains(name_str.as_str())
            } else {
                component
                    .fields
                    .iter()
                    .any(|f| f.name == name_str && f.kind == FieldKind::Prop)
            }
        })
        .cloned()
        .collect();
    let mutable_required_names_set: HashSet<String> = mutable_required_names
        .iter()
        .map(|n| n.to_string())
        .collect();
    // A component's runtime-mutable props use the same typed notification surface as a
    // viewmodel. Only required props participate here: deferred props are not referenced by this
    // component's view (otherwise they would not be deferred) and therefore have no local visual
    // update to dispatch.
    let component_property_enum = format_ident!("{}Property", component.name);

    // Own defaulted-prop/computed fields (collected above, before `ctx.own_fields`'s own map was
    // finalized) each get exactly the same Cell/RefCell storage shape as `mutable_required_names`
    // just above — the only difference is what seeds the initial value: a `mutable_required_names`
    // field is seeded from a `new(..)` ctor argument, these are seeded from the `let <name> = ..;`
    // statements already sitting at the front of `construct_stmts` (`own_default_construct_stmts`,
    // above) — `#name: <#cell_ty<_>>::new(#name)` is agnostic to which kind of in-scope local
    // `#name` actually is.
    let own_default_names: Vec<syn::Ident> = own_stored_fields
        .iter()
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let own_default_types: Vec<syn::Type> = own_stored_fields
        .iter()
        .map(|f| syn::parse_str(&f.ty).expect("field type must parse"))
        .collect();
    let own_default_cell_types: Vec<TokenStream> = own_stored_fields
        .iter()
        .map(|f| {
            if is_copy_type(&f.ty) {
                quote! { std::cell::Cell }
            } else {
                quote! { std::cell::RefCell }
            }
        })
        .collect();
    let own_default_field_decls: TokenStream = own_default_names
        .iter()
        .zip(own_default_types.iter())
        .zip(own_default_cell_types.iter())
        .map(|((name, ty), cell_ty)| quote! { #name: #cell_ty<#ty>, })
        .collect();
    let own_default_field_inits: TokenStream = own_default_names
        .iter()
        .zip(own_default_cell_types.iter())
        .map(|(name, cell_ty)| quote! { #name: <#cell_ty<_>>::new(#name), })
        .collect();

    let own_computed_names: Vec<syn::Ident> = own_computed_fields
        .iter()
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let own_computed_types: Vec<syn::Type> = own_computed_fields
        .iter()
        .map(|f| syn::parse_str(&f.ty).expect("field type must parse"))
        .collect();
    let own_computed_cell_types: Vec<TokenStream> = own_computed_fields
        .iter()
        .map(|f| {
            if is_copy_type(&f.ty) {
                quote! { std::cell::Cell }
            } else {
                quote! { std::cell::RefCell }
            }
        })
        .collect();
    let own_computed_field_decls: TokenStream = own_computed_names
        .iter()
        .zip(own_computed_types.iter())
        .zip(own_computed_cell_types.iter())
        .map(|((name, ty), cell_ty)| quote! { #name: #cell_ty<#ty>, })
        .collect();
    let own_computed_field_inits: TokenStream = own_computed_names
        .iter()
        .zip(own_computed_cell_types.iter())
        .map(|(name, cell_ty)| quote! { #name: <#cell_ty<_>>::new(#name), })
        .collect();

    // `#[environment(name)]` fields — same Cell/RefCell storage shape as `own_computed_*` just
    // above (read through the same generic own-field bare-path branch, `ctx.mutable_own_fields`),
    // seeded from `own_default_construct_stmts`'s `let`s instead of a declared expression.
    let own_environment_names: Vec<syn::Ident> = own_environment_fields
        .iter()
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let own_environment_types: Vec<syn::Type> = own_environment_fields
        .iter()
        .map(|f| syn::parse_str(&f.ty).expect("field type must parse"))
        .collect();
    let own_environment_cell_types: Vec<TokenStream> = own_environment_fields
        .iter()
        .map(|f| {
            if is_copy_type(&f.ty) {
                quote! { std::cell::Cell }
            } else {
                quote! { std::cell::RefCell }
            }
        })
        .collect();
    let own_environment_field_decls: TokenStream = own_environment_names
        .iter()
        .zip(own_environment_types.iter())
        .zip(own_environment_cell_types.iter())
        .map(|((name, ty), cell_ty)| quote! { #name: #cell_ty<#ty>, })
        .collect();
    let own_environment_field_inits: TokenStream = own_environment_names
        .iter()
        .zip(own_environment_cell_types.iter())
        .map(|(name, cell_ty)| quote! { #name: <#cell_ty<_>>::new(#name), })
        .collect();
    // The legacy, ambient-captured `__environment: EnvironmentContext` field (and its
    // `has_own_environment_fields`-gated memory policy) is gone (CI-5 of #80,
    // docs/design/runtime/component_lifecycle_design.md §4d) — every view-bearing component already
    // carries `__mount_environment: OnceCell<EnvironmentContext>` unconditionally (CI-3), populated
    // by `mount()` with the *real* context (not an ambient re-read), which is exactly what this
    // field used to approximate. `own_environment_recompute_methods`/`own_environment_subscribe_stmts`
    // below read `self.__mount_environment.get().expect(..)` directly instead.

    let mut component_property_variants = mutable_required_names.clone();
    component_property_variants.extend(own_default_names.iter().cloned());
    component_property_variants.extend(own_computed_names.iter().cloned());
    component_property_variants.extend(own_environment_names.iter().cloned());
    ctx.mutable_own_fields = mutable_required_names_set.clone();
    ctx.mutable_own_fields
        .extend(own_default_names.iter().map(|n| n.to_string()));
    ctx.mutable_own_fields
        .extend(own_computed_names.iter().map(|n| n.to_string()));
    ctx.mutable_own_fields
        .extend(own_environment_names.iter().map(|n| n.to_string()));

    // A standalone `template_view!` factory is generic over the expected
    // `ControlTemplate<C>` target.  Its `templated_parent.foo` expressions therefore cannot call
    // an inherent getter on an as-yet-uninferred `C`; expose the existing generated getter and
    // property-notification surface through a compile-time, hashed property bridge instead.  The
    // bridge is emitted for explicit template declarations and structurally-resolved Control-family
    // components, so a named or standalone template may target a Control-derived component whose
    // default is supplied elsewhere.  It remains entirely static: no strings, maps, or erased
    // target values participate at runtime.
    let template_property_impls: TokenStream = if is_control_template_enabled
        || table
            .resolve(from, &target_name)
            // `composed_shape` is the symbol-table's structural class-family result.  Only the
            // Control-family components can be targets of a typed ControlTemplate; host/window
            // and ordinary layout components must not receive a public associated-type bridge
            // for their private fields.  This is a capability check, not a template/codegen
            // dispatch path.
            .is_some_and(|info| info.composed_shape.as_deref() == Some("Control"))
    {
        // The bridge's writable capability follows effective field metadata, not the target's
        // own property-notification enum.  Inherited properties are intentionally absent from
        // the derived component's local storage/enum because the value lives in `base`, but their
        // generated base setter is still a real writable template surface.
        let writable_property_names: HashSet<String> = component
            .fields
            .iter()
            .filter(|field| matches!(field.kind, FieldKind::Prop | FieldKind::State))
            .map(|field| field.name.clone())
            .collect();
        let notified_property_names: HashSet<String> = component_property_variants
            .iter()
            .map(ToString::to_string)
            .collect();
        // The bridge receiver is the concrete `base` field of a composed target.  Use that
        // immediate base's trait for inherited accessors rather than a bare method call: a
        // consumer-defined base may live in another module, and a multi-hop base still exposes
        // its ancestor accessors through the immediate base trait's supertrait chain.  This keeps
        // the bridge path-qualified without adding a runtime dispatch table.
        let template_base_ext_path = if let Some(base_name) = if is_shape_composition {
            Some(resolved_root.type_path.as_str())
        } else {
            component.base.as_deref()
        } {
            if let Some(qualified) = immediate_base_qualified_ext_path(component, base_name) {
                Some(qualified)
            } else {
                Some(dsl_ext_path(base_name, table.resolve(from, base_name)))
            }
        } else {
            None
        };
        let mut readable: HashMap<String, syn::Type> = HashMap::new();
        for field in &component.fields {
            if field.name.starts_with("on_") {
                continue;
            }
            if matches!(
                field.kind,
                FieldKind::Attached
                    | FieldKind::Action
                    | FieldKind::Observable
                    | FieldKind::AsyncComputed
            ) {
                continue;
            }
            let ty = syn::parse_str::<syn::Type>(&field.ty)
                .expect("template property field type must parse");
            readable.entry(field.name.clone()).or_insert(ty);
        }
        readable
            .into_iter()
            .map(|(name, ty)| {
                let ident = format_ident!("{name}");
                let key = crate::template_property_key(&name);
                let getter = &ident;
                let inherited_owner = table
                    .resolve(from, &target_name)
                    .and_then(|info| info.declaring_types.get(&name))
                    .filter(|owner| *owner != &target_name)
                    .cloned();
                let inherited = inherited_owner.is_some();
                let getter_body = if inherited {
                    if let Some(base_ext_path) = &template_base_ext_path {
                        quote! { #base_ext_path::#getter(&self.base) }
                    } else {
                        quote! { self.base.#getter() }
                    }
                } else if is_composed
                    && source_component.content_field.as_deref() == Some(name.as_str())
                {
                    // A component may expose both its typed generated content getter and an
                    // erased collection convenience method under the same name. Keep the
                    // template-property bridge on the generated trait so its associated value
                    // type remains the authored content type.
                    quote! { <Self as #target_ext>::#getter(self) }
                } else {
                    quote! { self.#getter() }
                };
                let setter_available = writable_property_names.contains(&name);
                let setter_body = if setter_available {
                    let setter = format_ident!("set_{name}");
                    if inherited_owner.is_some() {
                        if let Some(base_ext_path) = &template_base_ext_path {
                            quote! { #base_ext_path::#setter(&self.base, value); }
                        } else {
                            quote! { self.base.#setter(value); }
                        }
                    } else if is_composed
                        && source_component.content_field.as_deref() == Some(name.as_str())
                    {
                        quote! { <Self as #target_ext>::#setter(self, value); }
                    } else {
                        quote! { self.#setter(value); }
                    }
                } else {
                    TokenStream::new()
                };
                let bindable = component
                    .fields
                    .iter()
                    .find(|field| field.name == name)
                    .is_some_and(|field| {
                        field
                            .attrs
                            .iter()
                            .any(|attr| matches!(attr, Attr::Bindable))
                    });
                let subscription = if bindable {
                    // A bindable field is an owned observable object (normally `Rc<ViewModel>`).
                    // Subscribe to that nested stream so a template expression such as
                    // `templated_parent.vm.show_child` refreshes when the view-model property
                    // changes; this is the same owner-level dependency used by ordinary `view!`
                    // generation and remains metadata-driven.
                    quote! {
                        {
                            let owner = self.#getter();
                            elwindui::core::reactive::ObservableExt::subscribe_property_changed(
                                &*owner,
                                move |_| listener(),
                            )
                        }
                    }
                } else if inherited_owner
                    .as_deref()
                    .and_then(|owner| table.resolve(from, owner))
                    .is_some_and(|info| !info.is_builtin)
                {
                    // A component-derived template reads inherited values through its composed
                    // base.  The base's typed stream is sufficient here; the property bridge is
                    // intentionally static and does not introduce a second runtime lookup table.
                    quote! { self.base.subscribe_property_changed(move |_| listener()) }
                } else if notified_property_names.contains(&name) {
                    quote! {
                        self.subscribe_property_changed(move |property| {
                            if matches!(property, #component_property_enum::#ident) {
                                listener();
                            }
                        })
                    }
                } else {
                    quote! { elwindui::core::reactive::Subscription::new(|| {}) }
                };
                let readable_impl = quote! {
                    impl elwindui::core::ui::TemplateProperty<#key> for #target {
                        type Value = #ty;

                        fn __template_get(&self) -> Self::Value {
                            #getter_body
                        }

                        fn __template_subscribe(
                            &self,
                            listener: impl Fn() + 'static,
                        ) -> elwindui::core::reactive::Subscription {
                            #subscription
                        }
                    }
                };
                let writable_impl = setter_available.then(|| {
                    quote! {
                        impl elwindui::core::ui::WritableTemplateProperty<#key> for #target {
                            fn __template_set(&self, value: Self::Value) {
                                #setter_body
                            }
                        }
                    }
                });
                quote! {
                    #readable_impl
                    #writable_impl
                }
            })
            .collect()
    } else {
        TokenStream::new()
    };
    let mutable_required_types: Vec<syn::Type> = required_own_names
        .iter()
        .zip(required_own_types.iter())
        .filter(|(n, _)| mutable_required_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();
    let mutable_required_cell_types: Vec<TokenStream> = mutable_required_names
        .iter()
        .map(|n| {
            let ty_str = ctx.own_fields.get(&n.to_string()).unwrap();
            if is_copy_type(ty_str) {
                quote! { std::cell::Cell }
            } else {
                quote! { std::cell::RefCell }
            }
        })
        .collect();
    let mutable_required_field_decls: TokenStream = mutable_required_names
        .iter()
        .zip(mutable_required_types.iter())
        .zip(mutable_required_cell_types.iter())
        .map(|((name, ty), cell_ty)| quote! { #name: #cell_ty<#ty>, })
        .collect();
    let mutable_required_field_inits: TokenStream = mutable_required_names
        .iter()
        .zip(mutable_required_cell_types.iter())
        .map(|(name, cell_ty)| quote! { #name: <#cell_ty<_>>::new(#name), })
        .collect();
    // The plain (bare-storage, `Self { #name, .. }`-shorthand-eligible) subset of `required_own_names`
    // — everything not promoted to Cell/RefCell storage above.
    let plain_required_names: Vec<syn::Ident> = required_own_names
        .iter()
        .filter(|n| !mutable_required_names_set.contains(&n.to_string()))
        .cloned()
        .collect();
    let plain_required_types: Vec<syn::Type> = required_own_names
        .iter()
        .zip(required_own_types.iter())
        .filter(|(n, _)| !mutable_required_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();

    let mut struct_fields = TokenStream::new();
    let mut construct_stmts = own_default_construct_stmts;
    // CI-4 of #80 (docs/design/runtime/component_lifecycle_design.md §4b): plan-driven descendant
    // construction — every ordinary (non-root-composition) `PlannedNode` — is emitted here instead
    // of into `construct_stmts`, so it runs from `__build_view()` (post-`Rc::new_cyclic`, once this
    // component's own `Rc<Self>`/mount context exists) rather than from `construct()` (pre-`Rc`).
    // The shape/host-composition root's own `base` field is a distinct, always-required mechanism
    // (never a `stored` `PlannedNode`) and stays in `construct_stmts`, unmoved.
    let mut child_construct_stmts = TokenStream::new();
    if is_template_or_deferred_scope {
        child_construct_stmts.extend(quote! {
            let #template_environment_ident = self
                .__mount_environment
                .get()
                .expect("ControlTemplate body: component is not yet mounted")
                .clone();
        });
    }
    // `emit_expr`'s `EmitMode::Construction` (used throughout `emit_construction` and its helpers,
    // unchanged by the `child_construct_stmts` move above) emits a bare own-field reference (e.g.
    // `tint` in `TextBlock { text: format!("{tint}") }`) as a plain local identifier — correct when
    // this code ran inside `construct()`, where `own_default_construct_stmts`'s `let #field = ..;`
    // declarations put those names directly in scope. Now that this code runs inside `__build_view`
    // instead, those `let`s are out of scope (they still exist, but only inside `construct()`/
    // `__class_construct`, a different function) — so re-declare the same bare names here, sourced
    // from each field's own already-generated `self.#name()` accessor (every `ctx.own_fields` entry
    // has one, per the `param_names`/own-default/own-computed/own-environment accessor-generation
    // loops elsewhere in this function, *except* an `on_*`-named `#[routed]` field, which is wired
    // through event handling, not read as a value — excluded here, matching `param_names`' own
    // exclusion of it). `#[allow(unused_variables)]` since not every field is necessarily referenced
    // by name inside the view tree. Order doesn't matter (each reads independently from `self`,
    // unlike `own_default_construct_stmts`'s own sibling-dependent expressions).
    for own_field_name in ctx.own_fields.keys() {
        if own_field_name.starts_with("on_") {
            continue;
        }
        let own_field_ident = format_ident!("{}", own_field_name);
        let own_field_get = if is_composed
            && source_component.content_field.as_deref() == Some(own_field_name.as_str())
        {
            // Keep a generated component's typed content getter distinct from a same-named
            // erased collection convenience method when initializing its authored view.
            quote! { <Self as #target_ext>::#own_field_ident(self) }
        } else {
            quote! { self.#own_field_ident() }
        };
        child_construct_stmts.extend(quote! {
            #[allow(unused_variables)]
            let #own_field_ident = #own_field_get;
        });
    }
    let mut field_inits = TokenStream::new();
    let mut wiring_stmts = TokenStream::new();
    let mut resync_stmts = TokenStream::new();
    // `#[id("...")]` bindings (§13) — a monomorphized `pub fn <id>(&self) -> Rc<ConcreteType>`
    // per binding, not a runtime string-keyed lookup (every `#[id(...)]` name is fixed at compile
    // time, so a plain accessor is strictly sufficient — see docs/specs/dsl_spec.md §12 and
    // docs/design/runtime/state_management_design.md's avoid-type-erasure convention).
    let mut named_accessors = TokenStream::new();
    // Populated instead of `named_accessors` for a composed target's own `#[param]`
    // getters/deferred setters (below) — `#[id(...)]`-tagged child accessors never move here (they
    // return a concrete `Rc<ConcreteType>` specific to this component's own view structure, not
    // part of the base class's shared interface), so `named_accessors` alone still covers those
    // regardless of `is_composed`. Each entry here is a full `fn name(&self, ..) { .. }` (signature
    // *and* body) — under `#[class]` (this function's tail `quote!`) these become untagged methods
    // in the merged `impl #target { .. }` block, and the macro derives both the generated `pub
    // trait #target: <base> { .. }`'s signatures and `impl #target for #targetImpl { .. }`'s bodies
    // from them automatically, so there's no separate signature-only list to maintain here anymore.
    let mut own_class_methods = TokenStream::new();

    let component_property_api = mark_inherent(quote! {
        pub fn subscribe_property_changed(
            &self,
            f: impl Fn(#component_property_enum) + 'static,
        ) -> elwindui::core::reactive::Subscription {
            let active = std::rc::Rc::new(std::cell::Cell::new(true));
            let handler: std::rc::Rc<dyn Fn(#component_property_enum)> = std::rc::Rc::new(f);
            self.__property_changed_handlers
                .borrow_mut()
                .push((active.clone(), handler));
            let handlers = std::rc::Rc::downgrade(&self.__property_changed_handlers);
            elwindui::core::reactive::Subscription::new(move || {
                active.set(false);
                if let Some(handlers) = handlers.upgrade() {
                    handlers
                        .borrow_mut()
                        .retain(|(registered, _)| !std::rc::Rc::ptr_eq(registered, &active));
                }
            })
        }

        #[allow(dead_code)]
        fn on_property_changed(&self, property: #component_property_enum) {
            let handlers = self.__property_changed_handlers.borrow().clone();
            for (active, handler) in handlers {
                if active.get() {
                    handler(property);
                }
            }
        }
    });

    // Every `#[param]` field gets a public `pub fn <name>(&self) -> <Type>` accessor, not just
    // `#[id(...)]`-tagged lets above — code outside the generated view (and DSL-composed wrappers
    // like `ContentControl`, whose `content`/`padding` need to be readable the same way any other
    // component's properties are) needs to reach a component's own properties, not just its named
    // child elements. Each field is already stored verbatim on `Self` via `new`'s `Self {
    // #(#param_names,)* .. }` shorthand below, so this only adds the accessor, not new storage —
    // except a forwarded name (`own_struct_param_names` doesn't include it, see that binding's doc
    // comment and `shape_forwarded_names`'s), which has no field of its own to read and instead
    // delegates to the base: a `is_inherited_view_composition` forward reads the base's own already-
    // generated accessor method of the same name (`self.base.<name>()`), while a
    // `shape_forwarded_names` one reads the field straight off the base's `elwindui::core::ui`
    // struct instead — those structs' non-`Copy` fields are `RefCell`-wrapped (docs/design/README.md
    // §5.1's post-construction setter convention), so this reads `self.base.<name>.borrow()
    // .clone()`, not a plain `.clone()` (unlike a DSL-composed base's own accessor method).
    for (name, ty) in param_names.iter().zip(param_types.iter()) {
        if inherited_shape_field_names.contains(&name.to_string()) {
            continue;
        }
        let is_forwarded = !own_struct_param_names.contains(name);
        // A deferred field and a mutable-required one (`mutable_required_names`) are both
        // Cell/RefCell-backed storage read the same way — `strip_option` is a harmless no-op for
        // the latter (never `Option<T>`-typed itself), so one branch covers both.
        let is_cell_backed = deferred_own_names_set.contains(&name.to_string())
            || mutable_required_names_set.contains(&name.to_string());
        let body = if is_inherited_view_composition && is_forwarded {
            quote! { self.base.#name() }
        } else if is_forwarded {
            quote! { self.base.#name.borrow().clone() }
        } else if is_cell_backed {
            let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
            if is_copy_type(strip_option(ty_str).0) {
                quote! { self.#name.get() }
            } else {
                quote! { self.#name.borrow().clone() }
            }
        } else {
            quote! { self.#name.clone() }
        };
        // A composed target's own class trait (docs/design/runtime/ui_tree_design.md) gets this getter
        // as a real (untagged) `#[class]` method — reachable generically through `dyn #target`/any
        // bound on it — not just non-composed (plain) components stay purely inherent.
        if is_composed {
            own_class_methods.extend(quote! {
                fn #name(&self) -> #ty {
                    #body
                }
            });
        } else {
            named_accessors.extend(quote! {
                pub fn #name(&self) -> #ty {
                    #body
                }
            });
        }
    }
    // `set_<name>(&self, value: T)` for every deferred own field — the post-construction setter
    // half of the convention (`deferred_own_names`'s own doc comment). `T` is the field's *inner*
    // (unwrapped) type, bare — not `Option<T>` — matching builtin setter signatures.
    // exactly (`build_component_setters`): an absent value simply never calls this at all, leaving
    // the field's own `None` default in place, so the setter itself never needs to accept `None`.
    for (name, inner_ty) in deferred_own_names
        .iter()
        .zip(deferred_own_inner_types.iter())
    {
        let set_name = format_ident!("set_{}", name);
        let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
        let set_body = if is_copy_type(strip_option(ty_str).0) {
            quote! { self.#name.set(Some(value)); }
        } else {
            quote! { *self.#name.borrow_mut() = Some(value); }
        };
        if is_composed {
            own_class_methods.extend(quote! {
                fn #set_name(&self, value: #inner_ty) {
                    #set_body
                }
            });
        } else {
            named_accessors.extend(quote! {
                pub fn #set_name(&self, value: #inner_ty) {
                    #set_body
                }
            });
        }
    }
    // `set_<name>(&self, value: T)` for every mutable-required own field (`mutable_required_names`'s
    // own doc comment) — unlike a deferred field's setter above, no `Some(..)` wrap (this storage
    // is never `Option`-shaped: the field always holds a real value from construction on) and it
    // re-runs `self.resync()` afterward, since this field — being required — is guaranteed to
    // actually feed into this component's own view.
    for (name, ty) in mutable_required_names
        .iter()
        .zip(mutable_required_types.iter())
    {
        let set_name = format_ident!("set_{}", name);
        let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
        let set_body = if is_copy_type(ty_str) {
            quote! { self.#name.set(value); }
        } else {
            quote! { *self.#name.borrow_mut() = value; }
        };
        if is_composed {
            own_class_methods.extend(quote! {
                fn #set_name(&self, value: #ty) {
                    #set_body
                    self.on_property_changed(#component_property_enum::#name);
                }
            });
        } else {
            named_accessors.extend(quote! {
                pub fn #set_name(&self, value: #ty) {
                    #set_body
                    self.on_property_changed(#component_property_enum::#name);
                }
            });
        }
    }

    // Getter + setter for a component's own defaulted-prop field (`own_default_names`, collected
    // near the top of this function alongside `own_computed_names`) — same Cell/RefCell read as
    // `mutable_required_names`' own getter, except it has no entry in the `param_names` getter loop
    // above at all (these fields are never `new(..)` arguments), and the same `on_property_changed`-
    // driven setter as `mutable_required_names`' own, additionally cascading into any own
    // `#[computed]` field that depends on it (`own_dependents_of`, collected near the top) —
    // mirroring `generate_viewmodel`'s own Observable-field setter cascade (`recompute_calls`,
    // this function's sibling above).
    for ((name, ty), field) in own_default_names
        .iter()
        .zip(own_default_types.iter())
        .zip(own_stored_fields.iter())
    {
        let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
        let get_body = if is_copy_type(ty_str) {
            quote! { self.#name.get() }
        } else {
            quote! { self.#name.borrow().clone() }
        };
        let set_name = format_ident!("set_{}", name);
        let set_body = if is_copy_type(ty_str) {
            quote! { self.#name.set(value); }
        } else {
            quote! { *self.#name.borrow_mut() = value; }
        };
        let recompute_calls: Vec<TokenStream> = own_dependents_of
            .get(&name.to_string())
            .into_iter()
            .flatten()
            .map(|dep| {
                let recompute = format_ident!("recompute_{}", dep);
                let property = format_ident!("{}", dep);
                quote! {
                    self.#recompute();
                    self.on_property_changed(#component_property_enum::#property);
                }
            })
            .collect();
        if field.kind == FieldKind::State {
            named_accessors.extend(quote! {
                fn #name(&self) -> #ty { #get_body }
                fn #set_name(&self, value: #ty) {
                    #set_body
                    #(#recompute_calls)*
                    self.on_property_changed(#component_property_enum::#name);
                }
            });
        } else if is_composed {
            own_class_methods.extend(quote! {
                fn #name(&self) -> #ty { #get_body }
                fn #set_name(&self, value: #ty) {
                    #set_body
                    #(#recompute_calls)*
                    self.on_property_changed(#component_property_enum::#name);
                }
            });
        } else {
            named_accessors.extend(quote! {
                pub fn #name(&self) -> #ty { #get_body }
                pub fn #set_name(&self, value: #ty) {
                    #set_body
                    #(#recompute_calls)*
                    self.on_property_changed(#component_property_enum::#name);
                }
            });
        }
    }

    // Getter for a component's own `#[computed]` field (`own_computed_names`) — read-only (external
    // assignment to a `#[computed]` field is already a static error, docs/specs/dsl_spec.md §13
    // ルール3), Cell/RefCell-backed under the *same* field name as the accessor (not a `_cache`-
    // suffixed one like `generate_viewmodel`'s own Computed arm uses): this generic own-field
    // bare-path branch (`emit_expr`) reads `self.#ident.get()`/`.borrow().clone()` directly off
    // `ctx.mutable_own_fields`'s matching field name, and Rust allows a struct field and a
    // same-named inherent method to coexist (disambiguated by call syntax) — so keeping them at the
    // same name lets that existing machinery apply unmodified instead of needing a second lookup
    // table just for a suffix. The matching private `recompute_<name>` method (which actually
    // (re)computes this cache) is generated separately, alongside `component_property_resync_methods`
    // below — same reasoning as that method: internal-only, must not appear on `#[class]`'s generated
    // public trait.
    for (name, ty) in own_computed_names.iter().zip(own_computed_types.iter()) {
        let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
        let get_body = if is_copy_type(ty_str) {
            quote! { self.#name.get() }
        } else {
            quote! { self.#name.borrow().clone() }
        };
        if is_composed {
            own_class_methods.extend(quote! {
                fn #name(&self) -> #ty { #get_body }
            });
        } else {
            named_accessors.extend(quote! {
                pub fn #name(&self) -> #ty { #get_body }
            });
        }
    }
    // `recompute_<name>` for every own `#[computed]` field — mirrors `generate_viewmodel`'s own
    // Computed arm's `recompute_<name>` exactly (recomputes from the current values of whatever it
    // references, via `self.<field>()` calls, and overwrites the cache). Computed unconditionally
    // here (not `is_composed`-branched) and used both as-is (non-composed — already private, no
    // `pub` needed) and `mark_inherent`-wrapped (composed) below, exactly like
    // `component_property_resync_methods`.
    let own_computed_recompute_methods: TokenStream = own_computed_fields
        .iter()
        .map(|f| {
            let name = format_ident!("{}", f.name);
            let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
            let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
                unreachable!("own_computed_fields filtered to Some(Initializer::Expr(_))");
            };
            let compute_expr = rewrite_t_macro(
                rewrite_field_refs(raw_expr.clone(), &field_names, &format_ident!("self")),
                &field_names,
                &format_ident!("self"),
            );
            let set_cache = if is_copy_type(&f.ty) {
                quote! { self.#name.set(value); }
            } else {
                quote! { *self.#name.borrow_mut() = value; }
            };
            let recompute = format_ident!("recompute_{}", name);
            quote! {
                fn #recompute(&self) {
                    let value: #ty = #compute_expr;
                    #set_cache
                }
            }
        })
        .collect();

    // Getter for a component's own `#[environment(name)]` field (`own_environment_names`) — same
    // read-only, same-name field/method shape as the `#[computed]` loop just above.
    for (name, ty) in own_environment_names
        .iter()
        .zip(own_environment_types.iter())
    {
        let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
        let get_body = if is_copy_type(ty_str) {
            quote! { self.#name.get() }
        } else {
            quote! { self.#name.borrow().clone() }
        };
        if is_composed {
            own_class_methods.extend(quote! {
                fn #name(&self) -> #ty { #get_body }
            });
        } else {
            named_accessors.extend(quote! {
                pub fn #name(&self) -> #ty { #get_body }
            });
        }
    }
    // `recompute_<name>` for every own `#[environment(name)]` field — unlike `#[computed]`'s (which
    // recomputes from sibling fields), this re-reads `self.__mount_environment` (CI-5 of #80,
    // docs/design/runtime/component_lifecycle_design.md §4d: the real `EnvironmentContext` `mount()`
    // was called with, not an ambient re-read) and is called from that field's live subscription
    // callback (`own_environment_subscribe_stmts`, below), never from a sibling setter's cascade —
    // always after `mount()` has run, so the `.expect(..)` below can never actually fire.
    let own_environment_recompute_methods: TokenStream = own_environment_fields
        .iter()
        .map(|f| {
            let name = format_ident!("{}", f.name);
            let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
            let (key_type_preamble, key_type) = environment_key_type(f);
            let set_cache = if is_copy_type(&f.ty) {
                quote! { self.#name.set(value); }
            } else {
                quote! { *self.#name.borrow_mut() = value; }
            };
            let recompute = format_ident!("recompute_{}", name);
            quote! {
                fn #recompute(&self) {
                    #key_type_preamble
                    let value: #ty = self
                        .__mount_environment
                        .get()
                        .expect("recompute: component is not yet mounted")
                        .get::<#key_type>();
                    #set_cache
                }
            }
        })
        .collect();
    // Live subscription for every own `#[environment(name)]` field — mirrors `subscribe_stmts`'
    // bind-owner shape (weak `this`, pushed into `__property_changed_subscriptions`) but against
    // this field's own Environment cell instead of a viewmodel's `PropertyChanged`; unconditionally
    // also calls `__refresh_dynamic_regions()`, the same as `component_property_dispatch` already
    // does for every other own-field change (`docs/design/runtime/theme_environment_design.md`,
    // "Change propagation"). Subscribes against `self.__mount_environment` (CI-5), installed from
    // `__build_view()` — i.e. always after `mount()` set it.
    let own_environment_subscribe_stmts: TokenStream = own_environment_fields
        .iter()
        .map(|f| {
            let name = format_ident!("{}", f.name);
            let (key_type_preamble, key_type) = environment_key_type(f);
            let recompute = format_ident!("recompute_{}", name);
            quote! {
                {
                    #key_type_preamble
                    let weak = std::rc::Rc::downgrade(&this);
                    let subscription = this
                        .__mount_environment
                        .get()
                        .expect("subscribe: component is not yet mounted")
                        .subscribe::<#key_type>(move || {
                            if let Some(this) = weak.upgrade() {
                                this.#recompute();
                                this.on_property_changed(#component_property_enum::#name);
                                #target_ext::__refresh_dynamic_regions(&*this);
                            }
                        });
                    this.__property_changed_subscriptions.borrow_mut().push(subscription);
                }
            }
        })
        .collect();
    let semantic_brush_query = |node: &PlannedNode, name: &str| {
        if let Some(info) = resolve_context_info(&ctx, from, table, &node.type_path) {
            let value = is_semantic_brush_property(info, name);
            quote! { #value }
        } else {
            let props_macro = dsl_props_macro_path(&node.type_path, None);
            let name = format_ident!("{name}");
            quote! { #props_macro!(@is_semantic_brush #name) }
        }
    };
    let semantic_brush_lazy_leaves = collect_lazy_leaves(&plan);
    let semantic_brush_queries: Vec<TokenStream> = plan
        .iter()
        .filter(|node| node.type_path != ENVIRONMENT_SCOPE_MARKER)
        .chain(semantic_brush_lazy_leaves.iter().map(|(_, node)| *node))
        .flat_map(|node| {
            node.attributes
                .iter()
                .map(|attribute| semantic_brush_query(node, &attribute.name))
        })
        .collect();
    let semantic_brush_subscribe_stmts = quote! {
        if false #(|| #semantic_brush_queries)* {
            {
                let weak = std::rc::Rc::downgrade(&this);
                let listener: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
                    if let Some(this) = weak.upgrade() {
                        this.resync();
                        #target_ext::__refresh_dynamic_regions(&*this);
                    }
                });
                let subscriptions = elwindui::core::theme::subscribe_semantic_brushes(
                    this.__mount_environment
                        .get()
                        .expect("semantic brush subscribe: component is not yet mounted"),
                    listener,
                );
                this.__property_changed_subscriptions
                    .borrow_mut()
                    .extend(subscriptions);
            }
        }
    };
    // Real `#[environment(name)]` resolution (CI-5 of #80,
    // docs/design/runtime/component_lifecycle_design.md §4d) — spliced into `__build_view()`, before
    // `#child_construct_stmts`, so this component's own environment-dependent field values (and
    // anything descendant construction reads through their `self.#name()` accessors, per
    // `child_construct_stmts`' own bare-name-redeclaration preamble) are correct *before* any
    // descendant element gets built. Overwrites each field's already-`Cell`/`RefCell`-backed storage
    // (seeded with `K::default_value()` in `construct()`, above) with the real value read from
    // `self.__mount_environment` — never a second, independent ambient read.
    let own_environment_resolve_stmts: TokenStream = if own_environment_fields.is_empty() {
        TokenStream::new()
    } else {
        let mut stmts = quote! {
            let __elwindui_environment = self
                .__mount_environment
                .get()
                .expect("__build_view: component is not yet mounted")
                .clone();
        };
        for f in &own_environment_fields {
            let field_ident = format_ident!("{}", f.name);
            let (key_type_preamble, key_type) = environment_key_type(f);
            let set_stmt = if is_copy_type(&f.ty) {
                quote! { self.#field_ident.set(__elwindui_environment.get::<#key_type>()); }
            } else {
                quote! { *self.#field_ident.borrow_mut() = __elwindui_environment.get::<#key_type>(); }
            };
            stmts.extend(key_type_preamble);
            stmts.extend(set_stmt);
        }
        stmts
    };

    // `is_inherited_view_composition`'s `plan`/`view` are the *base's* own (cloned, `resolve_view_for`)
    // tree, not this component's — its only real construction step is calling the base's own
    // `create_<snake case>(..)` factory (below), so none of `plan`'s nodes are constructed or wired
    // here at all.
    let root_index = plan.len() - 1;
    for node in &plan {
        if node.dynamic.is_none() {
            continue;
        }
        // The real (non-dynamic) ancestor element — walking through any number of enclosing
        // dynamic regions for a nested one (Phase 1) — whose own content-collection item type
        // every dynamic node sharing that ancestor stores its `DynamicChildSlot` against. A
        // *scalar* content field (Phase 2) needs no such slot at all — refreshing it is just a
        // stateless `set_<field>(..)` swap, so it gets no struct field here (see
        // `dynamic_region_refresh_method`'s own scalar/list split).
        let parent = find_dynamic_region_anchor(&plan, &node.binding);
        // Local scalar content needs no slot at all. For an external class the field declaration
        // remains unconditional because macro_rules cannot expand to a struct-field list; its
        // defining shape macro selects `()` for scalar content and `DynamicChildSlot<_>` for
        // collection content in type position.
        let parent_info = table.resolve(from, &parent.type_path);
        let parent_shape = parent_info.map(effective_content_shape);
        if parent_shape == Some(EffectiveContentShape::Scalar) {
            continue;
        }
        let slot = dynamic_slot_ident(&node.binding);
        let item_ext = dynamic_collection_item_trait_for_type_with_props_macro(
            &parent.type_path,
            from,
            table,
            dynamic_content_props_macro_path(&parent.type_path, parent_info),
        );
        let slot_type = match parent_shape {
            Some(EffectiveContentShape::Collection) => {
                quote! { elwindui::core::ui::DynamicChildSlot<#item_ext> }
            }
            Some(EffectiveContentShape::External) | None => {
                let props_macro_path =
                    dynamic_content_props_macro_path(&parent.type_path, parent_info);
                quote! {
                    #props_macro_path!(@content_slot_type #item_ext)
                }
            }
            Some(EffectiveContentShape::Scalar) => unreachable!(),
        };
        struct_fields.extend(quote! {
            #slot: #slot_type,
        });
        field_inits.extend(quote! {
            #slot: ::std::default::Default::default(),
        });
    }
    // `RefCell<Option<Rc<..>>>` per lazily-materialized `if`/`match` branch leaf
    // (`lazy_branch_plan`'s own eligibility rule; see `emit_lazy_leaf_value`/
    // `emit_lazy_branch_resync`, the field's only other readers) — declared unconditionally here
    // (not gated by `is_list`/`is_inherited_view_composition` above, since a lazy leaf's own field is
    // never part of `plan` itself and so isn't touched by anything else in this function).
    for (cache_field, leaf) in collect_lazy_leaves(&plan) {
        let type_ident = concrete_type_ident(&leaf.type_path, table.resolve(from, &leaf.type_path));
        struct_fields.extend(quote! {
            #cache_field: std::cell::RefCell<Option<std::rc::Rc<#type_ident>>>,
        });
        field_inits.extend(quote! {
            #cache_field: std::cell::RefCell::new(None),
        });
    }
    if !is_inherited_view_composition {
        for (i, node) in plan.iter().enumerate() {
            if node.dynamic.is_some() {
                continue;
            }
            // The shape-composition root (see `is_shape_composition`'s doc comment) is built as a
            // plain, unwrapped `elwindui::core::ui::create_xxx(...)` value under its own
            // `node.binding` name — retained at its concrete type rather than erased into `Rc<dyn UIElement>` like
            // every other node — so it can be moved into `Self`'s own `base` field as-is (see the
            // `struct_fields`/`field_inits` branch below and this function's tail `quote!`).
            if is_shape_composition && i == root_index {
                let binding = &node.binding;
                // The base may be a hand-written `elwindui::core::ui` primitive (`Control`/`Shape`/
                // ...) or itself a resolved DSL component (`ContentControl`, for `RoundedPanel
                // inherits ContentControl`) — either way the result is a plain, unwrapped value
                // moved into `Self`'s own `base` field as-is (see the `field_inits` branch below and
                // this function's tail `quote!`), never wrapped/erased into `Rc<dyn
                // UIElement>` like every other node.
                if table
                    .resolve(from, &resolved_root.type_path)
                    .is_some_and(|i| i.is_virtual_builtin)
                {
                    let value = build_virtual_value(node, &ctx, from, table, true);
                    let base_impl_ty = shape_composition_base_type(&resolved_root.type_path);
                    construct_stmts.extend(quote! { let #binding: #base_impl_ty = #value; });
                } else {
                    let value = build_component_value(node, &ctx, from, table, &plan, component);
                    let base_impl_ty =
                        immediate_base_qualified_path(component, &resolved_root.type_path)
                            .unwrap_or_else(|| {
                                concrete_type_ident(
                                    &resolved_root.type_path,
                                    table.resolve(from, &resolved_root.type_path),
                                )
                            });
                    construct_stmts.extend(quote! { let #binding: #base_impl_ty = #value; });
                }
                continue;
            }
            // Host composition's root (the `Window` element itself) is likewise built as a plain,
            // unwrapped value via its own `construct()` — mirroring the shape-composition root just
            // above — so it can be moved into `Self`'s own `base` field as-is (see the `field_inits`
            // branch below): `Window` doesn't implement `UIElement` at all, so there's no
            // node erasure to skip here the way shape composition's comment
            // describes, but the "build unwrapped, embed directly" shape is identical. Mirrors
            // `emit_construction`'s `is_hand_written_native` branch exactly (`Type::new()` +
            // `build_component_setters`) except calling `construct()` — not `new()` — so the result
            // is the bare value rather than `Rc<Self>`; the node's own attributes (`title`/
            // `menu_bar`/`content`/`left`/`top`/`width`/`height`) still need applying right here,
            // since this root is never `stored` and so never reaches `emit_construction`'s normal
            // per-node loop.
            if is_host_composition && i == root_index {
                let binding = &node.binding;
                match table.resolve(from, &node.type_path) {
                    Some(info) => {
                        let type_ident = concrete_type_ident(&node.type_path, Some(info));
                        // CI-4 of #80 (docs/design/runtime/component_lifecycle_design.md §4b): the
                        // root's own attribute setters (e.g. `set_content(..)`) may reference an
                        // ordinary stored child's binding (e.g. the content tree's root), which is
                        // now built later, in `child_construct_stmts`/`__build_view()` — so the
                        // setters themselves must run there too, not here. Only the bare skeleton
                        // (needed by `field_inits`'s `base: #root_binding` below) stays in
                        // `construct_stmts`. `#binding` is re-bound to `&self.base` (the skeleton,
                        // already moved into `Self` by the time `__build_view()` runs) so the
                        // existing `#(#setters)*` tokens — which call methods on `#binding` — keep
                        // working unchanged.
                        let setters = build_component_setters(node, &ctx, from, table, info, &plan);
                        let trait_use = builtin_trait_use(&node.type_path, Some(info));
                        construct_stmts.extend(quote! {
                            #trait_use
                            let #binding: #type_ident = #type_ident::construct();
                        });
                        child_construct_stmts.extend(quote! {
                            #trait_use
                            let #binding = &self.base;
                            #(#setters)*
                        });
                    }
                    // External (no local `TypeInfo`) — same construction shape
                    // `emit_external_construction` uses for an ordinary node, just `construct()`
                    // (bare value, for `Self`'s own `base` field) instead of `new()` (`Rc<Self>`).
                    None => {
                        let type_path = dsl_concrete_type_path(&node.type_path, None);
                        let sets = emit_external_attribute_sets(node, &ctx, from, table);
                        construct_stmts.extend(quote! {
                            #[allow(unused_imports)]
                            use elwindui::ui::*;
                            let #binding = #type_path::construct();
                        });
                        child_construct_stmts.extend(quote! {
                            #[allow(unused_imports)]
                            use elwindui::ui::*;
                            let #binding = &self.base;
                            #sets
                        });
                    }
                }
                continue;
            }
            emit_construction(node, &ctx, from, table, &mut child_construct_stmts, &plan);
            if node.type_path == ENVIRONMENT_SCOPE_MARKER {
                let binding = &node.binding;
                struct_fields.extend(quote! {
                    #binding: std::cell::OnceCell<
                        elwindui::core::environment::EnvironmentContext
                    >,
                });
                field_inits.extend(quote! { #binding: std::cell::OnceCell::new(), });
                child_construct_stmts.extend(quote! {
                    self.#binding.set(#binding.clone()).ok();
                });
                continue;
            }
            if node.stored {
                let binding = &node.binding;
                // Every resolved type (a `component`/`view` pair or a hand-written builtin in
                // an `elwindui-backend-*` crate) is constructed as `Rc<Self>` uniformly (see `emit_construction`
                // and this same convention below in `root_embed_method`), so a stored handle is always
                // just `Rc<Type>` — no backend-crate-qualified path, no per-type bookkeeping fields.
                let type_ident =
                    concrete_type_ident(&node.type_path, table.resolve(from, &node.type_path));
                // `OnceCell`, not a plain field: this node is now constructed in `__build_view()`
                // (CI-4 of #80, docs/design/runtime/component_lifecycle_design.md §4b), which runs
                // after `Self` already exists, so it can no longer be set directly inside a `Self {
                // .. }` literal. `OnceCell::set` below (in `child_construct_stmts`) is a plain,
                // first-and-only write — no double-set possible, since each node is constructed
                // exactly once per `__build_view()` call, which is itself guarded by `mount()`'s own
                // idempotency check (docs/design/runtime/component_lifecycle_design.md §4a).
                struct_fields
                    .extend(quote! { #binding: std::cell::OnceCell<std::rc::Rc<#type_ident>>, });
                field_inits.extend(quote! { #binding: std::cell::OnceCell::new(), });
                child_construct_stmts.extend(quote! {
                    self.#binding.set(#binding.clone()).ok();
                });
                if let Some(id) = &node.id {
                    let accessor = format_ident!("{}", id);
                    let not_mounted_msg = format!("{id}: component is not yet mounted");
                    named_accessors.extend(quote! {
                        pub fn #accessor(&self) -> std::rc::Rc<#type_ident> {
                            self.#binding
                                .get()
                                .expect(#not_mounted_msg)
                                .clone()
                        }
                    });
                }
            }
        }
        for node in &plan {
            if node.dynamic.is_some() {
                continue;
            }
            // See `emit_wiring`'s/`emit_resync`'s own `self_is_node` doc comment: only the plan's
            // own root can be a shape/host-composition root with no separate `self.#binding` field.
            let self_is_node = (is_shape_composition || is_host_composition)
                && node.binding == plan[root_index].binding;
            emit_wiring(node, &ctx, from, table, &mut wiring_stmts, self_is_node);
            emit_resync(
                node,
                &ctx,
                from,
                table,
                ResyncFilter::All,
                &mut resync_stmts,
                self_is_node,
            );
        }

        if is_template_or_deferred_scope {
            emit_content_presenter_wiring(&plan, &ctx, &mut wiring_stmts);
        }
    }

    // `plan_element` pushes children before their parent (post-order), so the root is always last.
    // Irrelevant (the base's own root, not this component's) when `is_inherited_view_composition`.
    let root_binding = &plan.last().expect("view must have a root element").binding;

    // A plain virtual-builtin-rooted view (`VerticalLayout`, say — `DocumentView`'s actual root, if
    // it weren't wrapped in `ContentControl`) needs no special-casing here anymore: `plan_element`
    // now stores every root node — virtual builtin or not — under the same rule as any other node
    // (`is_root || !attributes.is_empty()`), so the generic per-node loop above already gave it a
    // real `Rc<XxxImpl>` struct field; `root_embed_method` below reaches it via the same
    // `into_node_if_needed` path any other non-native root uses.
    //
    // The shape-composition case (`is_shape_composition`) stashes it differently: as a real `base`
    // field of the shape's own `elwindui::core::ui` `YImpl` type (built unwrapped, above), not a
    // type-erased `Rc<dyn UIElement>` — `#[class(inherits = ..)]` (this function's tail `quote!`)
    // adds the field's *declaration* automatically; only the field's *value*, for the struct literal
    // inside `construct()`, needs assembling here. Template composition (`is_inherited_view_composition`)
    // is the same idea one level up: `base`'s type is the immediate DSL base's own struct (not an
    // `elwindui::core::ui` type), built by calling that base's own `construct(..)` directly rather
    // than constructing anything itself. Host composition (`is_host_composition`) reuses the exact
    // same "value only, no declaration" shape — its root was already built unwrapped, above.
    if is_inherited_view_composition {
        let base_name = component
            .base
            .as_deref()
            .expect("is_inherited_view_composition implies a base");
        // `base_name` (bare) is itself a composed component, so it's a real *trait* now, not a
        // struct (see `struct_ident`'s doc comment) — the field's concrete type must be its `Impl`
        // struct, exactly like `concrete_type_ident` resolves for any other reference to it.
        let base_info = table.resolve(from, base_name);
        let base_construct = qualified_construct_path(component, base_name)
            .unwrap_or_else(|| dsl_construct_path(base_name, base_info));
        if base_name == "ContentControl" && base_info.is_some_and(|info| info.is_builtin) {
            field_inits.extend(quote! { base: #base_construct(), });
        } else {
            field_inits.extend(quote! { base: #base_construct(#(#forward_param_names),*), });
        }
    } else if is_shape_composition || is_host_composition {
        field_inits.extend(quote! { base: #root_binding, });
    }

    // Whether this component itself ends up "native" or "virtual" (from the *caller's*
    // perspective — see `into_node_if_needed`) is inherited from its own view root, computed the
    // same way `build_symbol_table`'s `resolve_is_native` does. A native root (including `Window`,
    // handled separately above) keeps its local `into_any_view` inherent method
    // (not a `From`/`Into` impl: `impl From<Rc<#target>> for AnyView` would be rejected by Rust's
    // orphan rules, since `Rc` isn't "fundamental" and so `#target` nested inside it counts as
    // covered by a foreign generic — E0117). A virtual root gets `into_node` instead, returning
    // `Rc<dyn elwindui::core::ui::UIElementExt>`, via `into_node_if_needed` on its own stored root
    // field (the same path any other non-native embedding site uses) — whether that root is a
    // hardcoded virtual builtin or a user-defined component whose own root is itself virtual
    // (chained `inherits`), `into_node_if_needed` dispatches on the root's resolved type either way.
    let root_is_native = !is_inherited_view_composition
        && table
            .resolve(from, &resolved_root.type_path)
            .is_some_and(|info| info.is_native);
    let root_embed_method = if is_inherited_view_composition || is_shape_composition {
        // `#target` implements `UIElement` itself now (see this function's tail `quote!`), so
        // `self` — not a separately-stored root field — already *is* the tree node; `Rc<Self>`
        // unsizes to `Rc<dyn UIElement>` directly.
        quote! {
            pub fn into_node(self: std::rc::Rc<Self>) -> std::rc::Rc<dyn elwindui::core::ui::UIElementExt> {
                self
            }
        }
    } else if is_host_composition {
        // `#[class(inherits = Window)]` generates the `WindowExt` forwarding, including `show`.
        TokenStream::new()
    } else if resolved_root.type_path == "Window" {
        // A top-level window must use `inherits Window` to receive the `WindowExt` API.
        TokenStream::new()
    } else if root_is_native {
        let root_expr = into_any_view_if_needed(
            quote! { self.#root_binding.get().expect("into_any_view: component is not yet mounted") },
            "AnyView",
        );
        quote! {
            pub fn into_any_view(self: std::rc::Rc<Self>) -> elwindui::backend::AnyView {
                #root_expr
            }
        }
    } else {
        let root_expr = into_node_if_needed(
            quote! { self.#root_binding.get().expect("into_node: component is not yet mounted") },
            &resolved_root.type_path,
            from,
            table,
        );
        quote! {
            pub fn into_node(self: std::rc::Rc<Self>) -> std::rc::Rc<dyn elwindui::core::ui::UIElementExt> {
                #root_expr
            }
        }
    };

    // The generated update method covers every attribute owned by this component.
    // It is triggered by a PropertyChanged event (dispatched through `ObservableExt`, keyed by
    // property name — see `bind_owners`'s own doc comment above for why this isn't a per-viewmodel
    // enum), and the subscription's lifetime is owned by the view. Nested viewmodels do not bubble
    // their changes through a collection owner, preventing edits to a document from resyncking the
    // parent TabView. Called through the trait path (`ObservableExt::subscribe_property_changed`,
    // not `this.#owner_ident.subscribe_property_changed`) since this component's own codegen has no
    // name for `#owner_ident`'s concrete type to resolve an inherent method against — only that it
    // implements `ObservableExt`, satisfied generically for any type that does.
    let subscribe_stmts: TokenStream = bind_owners
        .iter()
        .map(|owner_ident| {
            let method = format_ident!("__resync_{}", owner_ident);
            let owner_name = owner_ident.to_string();
            let owner = if ctx.weak_bindable_owners.contains(&owner_name) {
                let upgrade_panic_message =
                    format!("weak owner `{owner_name}` was dropped before its template instance");
                quote! {
                    let owner = this.#owner_ident.upgrade().expect(#upgrade_panic_message);
                }
            } else {
                quote! { let owner = std::rc::Rc::clone(&this.#owner_ident); }
            };
            quote! {
                {
                    let weak = std::rc::Rc::downgrade(&this);
                    #owner
                    let subscription = elwindui::core::reactive::ObservableExt::subscribe_property_changed(&*owner, move |property: &'static str| {
                        if let Some(this) = weak.upgrade() { this.#method(property); }
                    });
                    this.__property_changed_subscriptions.borrow_mut().push(subscription);
                }
            }
        })
        .collect();
    // PR #165 post-final rereview remediation, A9 (§10.3): the implicit-bind-owner counterpart to
    // `subscribe_stmts` above — subscribes to the *resolved* `vm` value's own `ObservableExt`
    // stream (a genuinely separate notification stream from the source lexical owner's own
    // `PropertyChanged`, since `vm` is a distinct object), bridged through `__view_owner` since the
    // hidden Component has no physical `vm` field of its own to read `this.vm` from directly.
    // Dispatches to the exact same `__resync_<name>` method shape `property_resync_methods_for`
    // already generates for a physical bind owner (see the `implicit_property_resync_methods` call
    // alongside `property_resync_methods`, below) — reused unmodified, not a second resync engine.
    let implicit_bindable_subscribe_stmts: TokenStream = implicit_bind_owners
        .iter()
        .map(|owner_ident| {
            let implicit_field_name = ctx
                .implicit_owner
                .as_ref()
                .expect("implicit_bind_owners is only ever non-empty when ctx.implicit_owner is Some")
                .field_name
                .clone();
            let implicit_field = format_ident!("{}", implicit_field_name);
            let method = format_ident!("__resync_{}", owner_ident);
            let upgrade_panic_message = format!(
                "source lexical owner `{implicit_field_name}` was dropped before its template instance"
            );
            quote! {
                {
                    let weak = std::rc::Rc::downgrade(&this);
                    let __source_owner = this.#implicit_field.upgrade().expect(#upgrade_panic_message);
                    let owner = __source_owner.#owner_ident();
                    let subscription = elwindui::core::reactive::ObservableExt::subscribe_property_changed(&*owner, move |property: &'static str| {
                        if let Some(this) = weak.upgrade() { this.#method(property); }
                    });
                    this.__property_changed_subscriptions.borrow_mut().push(subscription);
                }
            }
        })
        .collect();
    let subscribe_stmts: TokenStream =
        quote! { #subscribe_stmts #implicit_bindable_subscribe_stmts };
    // Only real-anchored (top-level) dynamic nodes get their own top-level statement here — a
    // nested one (Phase 1) has no entry in any real element's own `child_bindings`, so the `find`
    // below returns `None` for it and `?` skips it; it's reached instead through
    // `emit_dynamic_node_refresh`'s own recursion into its real-anchored ancestor's branches.
    let dynamic_region_refresh_method: TokenStream = plan
        .iter()
        .filter_map(|node| {
            node.dynamic.as_ref()?;
            let parent = plan.iter().find(|candidate| {
                candidate
                    .child_bindings
                    .iter()
                    .any(|(child, _)| child == &node.binding)
            })?;
            let parent_binding = &parent.binding;
            let parent_info = table.resolve(from, &parent.type_path);
            let parent_ext_path = if let Some(qualified) =
                immediate_base_qualified_ext_path(component, &parent.type_path)
            {
                quote! { #qualified }
            } else {
                dsl_ext_path(&parent.type_path, parent_info)
            };
            let scalar_item_ext = ItemTraitTokens::KnownIdent(format_ident!("UIElementExt"));
            let parent_is_self = (is_shape_composition || is_host_composition)
                && parent.binding == plan[root_index].binding;
            let parent_receiver = if parent_is_self {
                quote! { self }
            } else {
                quote! {
                    self.#parent_binding
                        .get()
                        .expect("__refresh_dynamic_regions: component is not yet mounted")
                }
            };
            let scalar_body = {
                // Phase 2: a scalar `#[content(...)]` field needs no `DynamicChildSlot` at all —
                // every branch resolves to exactly one element (`validate::validate`'s
                // `dynamic_children_reduce_to_one_element` already guarantees this), so refreshing
                // is just picking the active branch's already-constructed value and swapping it in
                // via the field's own setter.
                let setter = parent_info
                    .and_then(|i| i.content_field.as_deref())
                    .map(|field| format_ident!("set_{field}"));
                let template_presentation = is_control_template_enabled && parent_is_self;
                emit_scalar_dynamic_node_refresh(
                    &plan,
                    node,
                    parent_binding,
                    setter.as_ref(),
                    parent_is_self,
                    &parent.type_path,
                    &scalar_item_ext,
                    &ctx,
                    from,
                    table,
                    template_presentation,
                )
            };
            let body = match parent_info.map(effective_content_shape) {
                Some(EffectiveContentShape::Collection) => {
                    let info = parent_info.expect("content shape came from parent metadata");
                    let item_ext = dynamic_collection_item_trait_for_type_with_props_macro(
                        &parent.type_path,
                        from,
                        table,
                        dynamic_content_props_macro_path(&parent.type_path, parent_info),
                    );
                    // The getter `#[content(..)]` names, not always literally `children` (`Dropdown`'s
                    // is `items`, `Menu`'s is `items`) — a local `TypeInfo` names it directly.
                    let field = info
                        .content_field
                        .as_deref()
                        .expect("collection content must name a field");
                    let field_ident = format_ident!("{field}");
                    let host = quote! { #parent_receiver.#field_ident() };
                    emit_dynamic_node_refresh(&plan, node, &host, &item_ext, &ctx, from, table)
                }
                Some(EffectiveContentShape::Scalar) => scalar_body,
                Some(EffectiveContentShape::External) | None => {
                    // External classes do not contribute a local `TypeInfo`. Dispatch both lowering
                    // blocks through the defining class's shape macro; the macro chooses the scalar
                    // or collection branch from effective `#[content]` metadata, so external Control
                    // remains scalar without a codegen type-name special case.
                    let props_macro_path =
                        dynamic_content_props_macro_path(&parent.type_path, parent_info);
                    let item_ext = dynamic_collection_item_trait_for_type_with_props_macro(
                        &parent.type_path,
                        from,
                        table,
                        props_macro_path.clone(),
                    );
                    let host = quote! {
                        #props_macro_path!(@content_field_get #parent_receiver)
                    };
                    let collection_body =
                        emit_dynamic_node_refresh(&plan, node, &host, &item_ext, &ctx, from, table);
                    quote! {
                        #props_macro_path!(@content_shape { #scalar_body }, { #collection_body });
                    }
                }
            };
            // `.children()` (called inside `#body`, when the parent is a `Layout` family type —
            // `VerticalLayout`/`HorizontalLayout`/`Grid`, always a virtual builtin) is `LayoutExt`'s
            // own default method, inherited (not redeclared) by each of those — `#parent_ext` alone
            // isn't enough to bring a default *ancestor* trait method into scope, the same reason
            // `emit_wiring`'s routed-handler registration needs its own explicit `UIElementExt`
            // import. Not needed for `TabView` (the only other collection-content type), whose
            // own `children()` is declared directly on `TabViewExt` — gated instead of unconditional
            // to avoid an always-unused import there.
            // External (no local `TypeInfo`): can't tell a `Layout`-family parent (needs this) from
            // a `TabView`-family one (doesn't) without a shape table, so this always includes it —
            // `#[allow(unused_imports)]` absorbs the case where it turns out not needed, the same
            // harmless-when-unused convention `emit_external_construction`'s own glob import uses.
            let layout_children_use = if parent_info.is_some_and(|i| i.is_virtual_builtin) {
                quote! { use elwindui::core::ui::LayoutExt as _; }
            } else if parent_info.is_none() {
                quote! { #[allow(unused_imports)] use elwindui::core::ui::LayoutExt as _; }
            } else {
                TokenStream::new()
            };
            Some(quote! {
                {
                    use #parent_ext_path as _;
                    #layout_children_use
                    #body
                }
            })
        })
        .collect();
    let dynamic_region_refresh_method = if dynamic_region_refresh_method.is_empty() {
        quote! { fn __refresh_dynamic_regions(&self) {} }
    } else {
        quote! {
            fn __refresh_dynamic_regions(&self) {
                #dynamic_region_refresh_method
            }
        }
    };

    // §3/docs/design/runtime/ui_tree_design.md's lifecycle hooks. `on_mount` is spliced directly into `new()` (against the local
    // `this: Rc<Self>`, the same receiver `base::on_mount()` rewrites to — see below); `on_unmount`
    // is codegen'd as a real (if presently uncalled) `__run_on_unmount` method — `elwindui::core::ui`
    // has no detach/teardown hook yet to wire it to, see docs/design/runtime/ui_tree_design.md.
    //
    // A `base::on_mount()`/`base::on_unmount()` call is only meaningful when *this* component wrote
    // its own `view` (an override of an inherited template) — a component with no `view` of its own
    // just inherited `view` wholesale (already containing its base's `on_mount`/`on_unmount`
    // verbatim, spliced in below with nothing further to rewrite). Only one `inherits` hop's worth
    // of `base::` chaining is guaranteed correct here — a base whose own `on_mount` itself calls
    // *its* base's `on_mount` would need multi-level shadow-name mangling this doesn't attempt.
    let (base_on_mount_block, base_on_unmount_block) = if has_own_view {
        component
            .base
            .as_deref()
            .filter(|b| *b != "NativeControl")
            .and_then(|base| table.resolve(from, base))
            .map(|info| (info.own_on_mount.clone(), info.own_on_unmount.clone()))
            .unwrap_or((None, None))
    } else {
        (None, None)
    };

    // PR #165 review remediation, A2: `on_mount`/`on_unmount` fire synchronously, inline within
    // `__build_view`/`__run_on_unmount`, always behind a plain `self`-shaped receiver in every
    // generated shape (`&self` for the `#[class]`-composed shapes, `self: &Rc<Self>` for the plain
    // shape) — `self` is therefore the one receiver token valid everywhere these two hooks are
    // spliced, unlike `this` (only bound in some of those shapes). `rewrite_view_closure_block`
    // (the same `ViewClosureRewriter` machinery `on_*` event handlers already go through) resolves
    // bare references to this component's own fields *and*, inside a lowered deferred view
    // (`ctx.implicit_owner`), the enclosing source Component's own fields — generalizing the exact
    // mechanism, not duplicating it for lifecycle hooks specifically.
    let self_ident = format_ident!("self");
    let hook_mode = EmitMode::WithSelf(quote! { #self_ident });
    let on_mount_stmt = view.on_mount.as_ref().map(|block| {
        let rewritten = rewrite_base_calls(block.clone(), &self_ident);
        rewrite_view_closure_block(rewritten, &[], &ctx, &hook_mode)
    });

    let mut shadow_hooks = TokenStream::new();
    if let Some(block) = &base_on_mount_block {
        shadow_hooks.extend(quote! { #[allow(dead_code)] fn __base_on_mount(&self) #block });
    }
    if let Some(block) = &base_on_unmount_block {
        shadow_hooks.extend(quote! { #[allow(dead_code)] fn __base_on_unmount(&self) #block });
    }
    let on_unmount_method = (!is_control_template_enabled)
        .then(|| view.on_unmount.as_ref())
        .flatten()
        .map(|block| {
            let rewritten = rewrite_base_calls(block.clone(), &self_ident);
            let rewritten = rewrite_view_closure_block(rewritten, &[], &ctx, &hook_mode);
            quote! { #[allow(dead_code)] fn __run_on_unmount(&self) #rewritten }
        });

    // CI-8 of #80 (docs/design/runtime/component_lifecycle_design.md §4g): a host-composition
    // (`inherits Window`) component's own generated `new()` no longer auto-mounts (see this
    // function's `on_constructed` splice below) — `show()` must do it instead, on first call only.
    //
    // Issue #128 migrated `Window::show`/`hide`/`close` from the CI-8 inherent-method/UFCS
    // workaround (`elwindui_core::ui::controls::window.rs`'s three methods were plain,
    // non-`#[overridable]`, because `#[overridable]`/`#[overrides]` did not propagate across the
    // `trait_only` (Window) -> `struct_only` (each backend's concrete Window) -> ordinary (this
    // generated component) chain) to normal `#[overridable]`/`#[overrides]`, once #128 fixed that
    // propagation generically in `#[class]` itself. `window_show_hide_close_overrides` below now
    // emits ordinary `#[overrides]` methods routed through the real ancestor-forwarding chain
    // (`self.base.show()`, not UFCS) — see that binding's own comment.
    let on_constructed_mount_call = (!is_host_composition).then(|| {
        quote! {
            <Self as #target_ext>::mount(
                self,
                elwindui::core::environment::application_environment(),
            );
        }
    });
    let mount_set_env = (!is_host_composition).then(|| {
        quote! {
            <Self as elwindui::core::ui::UIElementExt>::set_environment_context(
                self,
                environment.clone(),
            );
        }
    });
    // Issue #162 §3.14-§3.15: `Window`'s own framework-reserved `mount_override` hook, inserted
    // into the fixed generated `mount()` at the exact point the contract specifies — after the
    // lifecycle state is already `Mounted`, before the generated content is built (`__build_view`)
    // and before user `on_mount` (spliced later, inside `__build_view` itself). Only for a
    // host-composition (`inherits Window`) component — `mount_override` exists on `Window`
    // specifically, not on every composed component. UFCS (`<Self as WindowExt>::..`), matching
    // this same generated block's existing `content_element`/`show`/`hide`/`close` precedent.
    let mount_override_call = is_host_composition.then(|| {
        quote! {
            <Self as elwindui::core::ui::WindowExt>::mount_override(self, environment.clone());
        }
    });
    // Issue #162 §3.16: the `unmount()` counterpart, inserted into `window_lifecycle_overrides`'s
    // own generated `unmount()` below (not here — that method, not this `mount`/`__build_view`
    // pair, is where the fixed unmount algorithm lives for a host-composition component).
    let unmount_override_call = is_host_composition.then(|| {
        quote! {
            <Self as elwindui::core::ui::WindowExt>::unmount_override(self);
        }
    });
    let call_on_unmount = on_unmount_method
        .is_some()
        .then(|| quote! { self.__run_on_unmount(); });
    let window_lifecycle_overrides = is_host_composition.then(|| {
        quote! {
            #[doc(hidden)]
            pub fn __unmount_local(&self) {
                let prev = self.__lifecycle_state.get();
                if prev == elwindui::core::ui::ComponentLifecycleState::Unmounted
                    || prev == elwindui::core::ui::ComponentLifecycleState::Created
                {
                    self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
                    return;
                }
                self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                #call_on_unmount
                self.__property_changed_handlers.borrow_mut().clear();
                self.__property_changed_subscriptions.borrow_mut().clear();
                self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
            }

            #[doc(hidden)]
            pub fn unmount(&self) {
                match self.__lifecycle_state.get() {
                    elwindui::core::ui::ComponentLifecycleState::Created => {
                        self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
                        return;
                    }
                    elwindui::core::ui::ComponentLifecycleState::Mounted => {
                        self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                    }
                    elwindui::core::ui::ComponentLifecycleState::Unmounting
                    | elwindui::core::ui::ComponentLifecycleState::Unmounted => {
                        return;
                    }
                }
                #unmount_override_call
                if let Some(content) = <Self as elwindui::core::ui::WindowExt>::content_element(self) {
                    elwindui::core::ui::unmount_subtree(&content);
                }
                self.__unmount_local();
            }
        }
    });
    // Issue #128: normal `#[overrides]` methods (not `mark_inherent`-wrapped — these must reach
    // `#[class]`'s own `override_methods` collection, unlike `__unmount_local`/`unmount` above,
    // which are framework-internal and never part of `WindowExt`) — `self.base.show()` reaches the
    // backend's own concrete `impl WindowExt for BackendWindow` directly, exactly like any other
    // ordinary ancestor call at this layer (see `docs/agents/class-model.md`); no UFCS needed now
    // that #128 restored the normal override chain across the `trait_only -> struct_only ->
    // ordinary` boundary.
    let window_show_hide_close_overrides = is_host_composition.then(|| {
        quote! {
            #[overrides]
            fn show(&self) {
                if self.__closed.get() {
                    return;
                }
                if self.__mount_environment.get().is_none() {
                    self.mount(elwindui::core::environment::application_environment());
                }
                self.base.show();
            }

            #[overrides]
            fn hide(&self) {
                if self.__closed.get() {
                    return;
                }
                self.base.hide();
            }

            // Cancels this component's own property-changed/on_update/Environment subscriptions,
            // recursively cascades unmount to all descendant Components (Issue #126), and releases
            // the native window exactly once.
            #[overrides]
            fn close(&self) {
                if self.__closed.replace(true) {
                    return;
                }
                self.unmount();
                self.base.close();
            }

            // Issue #162 §3.19-§3.20: chains into the backend's own `mount_override` (installing
            // the native close-request handler, e.g. AppKit's `windowShouldClose:`/WinUI3's
            // `AppWindow.Closing`) first, then registers the common close callback every backend's
            // native close affordance must route through — a weak, downcast-recovered `Rc<Self>`
            // invoking this generated Window's own most-derived `WindowExt::close`, never the
            // backend base directly (the former inherent/UFCS workaround #128 removed). Ownership
            // stays acyclic: the closure captures only the type-erased `Weak<dyn Any>` this
            // component's own `__self_weak` field already holds, never a strong `Rc`.
            #[overrides]
            fn mount_override(&self, environment: elwindui::core::environment::EnvironmentContext) {
                self.base.mount_override(environment);
                let __weak_self_erased = self.__self_weak.borrow().clone();
                elwindui::core::ui::WindowLifecycleHost::set_close_request_handler(
                    &self.base,
                    Some(std::rc::Rc::new(move || {
                        let Some(__this) = __weak_self_erased
                            .upgrade()
                            .and_then(|__rc| __rc.downcast::<#target>().ok())
                        else {
                            return false;
                        };
                        <#target as elwindui::core::ui::WindowExt>::close(&__this);
                        true
                    })),
                );
            }

            // Clears the close-request handler *before* delegating to the backend's own
            // `unmount_override` (Issue #162 §3.23) — the backend's native close-request storage
            // must never keep pointing at a closure this component is about to tear down.
            #[overrides]
            fn unmount_override(&self) {
                elwindui::core::ui::WindowLifecycleHost::set_close_request_handler(&self.base, None);
                self.base.unmount_override();
            }
        }
    });
    let composed_unmount_method = (!is_host_composition).then(|| {
        quote! {
            #[doc(hidden)]
            pub fn __unmount_local(&self) {
                let prev = self.__lifecycle_state.get();
                if prev == elwindui::core::ui::ComponentLifecycleState::Unmounted
                    || prev == elwindui::core::ui::ComponentLifecycleState::Created
                {
                    self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
                    return;
                }
                self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                #call_on_unmount
                self.__property_changed_handlers.borrow_mut().clear();
                self.__property_changed_subscriptions.borrow_mut().clear();
                self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
            }

            #[doc(hidden)]
            pub fn unmount(&self) {
                match self.__lifecycle_state.get() {
                    elwindui::core::ui::ComponentLifecycleState::Created => {
                        self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
                        return;
                    }
                    elwindui::core::ui::ComponentLifecycleState::Mounted => {
                        self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                    }
                    elwindui::core::ui::ComponentLifecycleState::Unmounting
                    | elwindui::core::ui::ComponentLifecycleState::Unmounted => {
                        return;
                    }
                }
                let children = <Self as elwindui::core::ui::UIElementExt>::visual_children(self);
                for child in &children {
                    elwindui::core::ui::unmount_subtree(child);
                }
                self.__unmount_local();
                <Self as elwindui::core::ui::UIElementExt>::unmount(self);
                <Self as elwindui::core::ui::UIElementExt>::as_ui_element(self).visual_collection.clear();
            }
        }
    });
    let plain_unmount_root = if !root_is_native {
        let root_expr = into_node_if_needed(
            quote! { root.clone() },
            &resolved_root.type_path,
            from,
            table,
        );
        quote! {
            if let Some(root) = self.#root_binding.get() {
                let __root_node = #root_expr;
                elwindui::core::ui::unmount_subtree(&__root_node);
            }
        }
    } else {
        TokenStream::new()
    };
    let plain_unmount_method = quote! {
        #[doc(hidden)]
        pub fn __unmount_local(&self) {
            let prev = self.__lifecycle_state.get();
            if prev == elwindui::core::ui::ComponentLifecycleState::Unmounted
                || prev == elwindui::core::ui::ComponentLifecycleState::Created
            {
                self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
                return;
            }
            self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
            #call_on_unmount
            self.__property_changed_handlers.borrow_mut().clear();
            self.__property_changed_subscriptions.borrow_mut().clear();
            self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
        }

        #[doc(hidden)]
        pub fn unmount(&self) {
            match self.__lifecycle_state.get() {
                elwindui::core::ui::ComponentLifecycleState::Created => {
                    self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounted);
                    return;
                }
                elwindui::core::ui::ComponentLifecycleState::Mounted => {
                    self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                }
                elwindui::core::ui::ComponentLifecycleState::Unmounting
                | elwindui::core::ui::ComponentLifecycleState::Unmounted => {
                    return;
                }
            }
            #plain_unmount_root
            self.__unmount_local();
        }
    };

    let methods = emit_methods(&component.methods);
    // A composed component's companion impl is lowered into an existing `#[class]` impl. Keep
    // the method metadata on that boundary so the class macro can route virtual/override methods
    // through the same ancestor dispatch chain as hand-written classes. Inherited effective
    // methods remain generic compatibility helpers; only methods declared by this component carry
    // their source `#[overridable]`/`#[overrides]` classification.
    // A shape-composition root receives authored children only after the outer `Rc` exists. The
    // destination and its scalar/collection lowering are derived from the effective content
    // metadata. External builtins use their exported shape macro, which applies the same rule
    // without requiring a local type table.
    let (body_prepare_stmt, content_capture_stmt, content_attach_stmt) = if is_shape_composition {
        let root = plan.last();
        let has_root_children = root.is_some_and(|root| !root.child_bindings.is_empty());
        let children: Vec<(syn::Ident, String)> = root
            .map(|root| {
                root.child_bindings
                    .iter()
                    .filter(|(_, child_ty)| *child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                    .map(|(binding, child_ty)| (binding.clone(), child_ty.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let child_value = |binding: &syn::Ident, child_ty: &str| {
            if child_ty == PASSTHROUGH_NODE {
                quote! { self.#binding() }
            } else {
                quote! {
                    self.#binding
                        .get()
                        .expect("content_attach: component is not yet mounted")
                        .clone()
                }
            }
        };
        let attach = if children.is_empty() {
            TokenStream::new()
        } else {
            let values = children
                .iter()
                .map(|(binding, child_ty)| child_value(binding, child_ty));
            let erased = children
                .iter()
                .any(|(_, child_ty)| child_ty == PASSTHROUGH_NODE);
            let content_attach = if erased {
                quote! {
                    #root_props_macro_path!(@children_erased self, [#(#values),*]);
                }
            } else {
                quote! {
                    #root_props_macro_path!(@children self, [#(#values),*]);
                }
            };
            let template_value = children
                .first()
                .map(|(binding, child_ty)| {
                    into_node_if_needed(child_value(binding, child_ty), child_ty, from, table)
                })
                .unwrap_or_else(|| quote! { compile_error!("missing scalar template root") });
            let template_attach = quote! {
                {
                    use elwindui::core::ui::ControlExt as _;
                    self.__set_template_root(#template_value);
                }
            };
            if is_control_template_enabled {
                template_attach
            } else {
                content_attach
            }
        };
        if !is_control_template_enabled && has_root_children {
            let prepare = TokenStream::new();
            (prepare, TokenStream::new(), attach)
        } else {
            (TokenStream::new(), TokenStream::new(), attach)
        }
    } else if is_inherited_view_composition && component.base.as_deref() == Some("ContentControl") {
        // Unlike the shape-composition case above, `content`/`padding` are `construct`'s own
        // parameters, not stored fields — `on_constructed` has no parameters of its own to read
        // them back from, so `construct` stashes them in this hidden field for `on_constructed`
        // to drain exactly once.
        struct_fields.extend(quote! {
                __deferred_content_attach: std::cell::RefCell<Option<(Option<f32>, std::rc::Rc<dyn elwindui::core::ui::UIElementExt>)>>,
            });
        field_inits.extend(quote! {
            __deferred_content_attach: std::cell::RefCell::new(Some((padding, content.clone()))),
        });
        (
            TokenStream::new(),
            TokenStream::new(),
            quote! {
                if let Some((padding, content)) = self.__deferred_content_attach.borrow_mut().take() {
                    use elwindui::core::ui::ContentControlExt as _;
                    self.set_padding(padding.unwrap_or_default());
                    self.set_content(content);
                }
            },
        )
    } else {
        (TokenStream::new(), TokenStream::new(), TokenStream::new())
    };

    // `#target`'s own class-hierarchy declaration (docs/design/runtime/ui_tree_design.md). A composed
    // component (`is_shape_composition`/`is_inherited_view_composition`/`is_host_composition`) is declared
    // as `#[elwindui::class(inherits = <immediate base's own trait path>)] pub struct #target
    // { .. }` + a paired bare `#[elwindui::class] impl #target { .. }` (`elwindui::class` — not
    // `elwindui_macros::class` directly — since a consumer crate only ever has `elwindui` itself,
    // the facade, as a direct dependency; see `elwindui_macros::class::core_path`'s own doc comment
    // for the matching path-resolution rule this relies on) — the macro derives
    // `#targetImpl`'s own `base: <BaseImpl>` field, the bare-named `pub trait #target: <base>`
    // (reaching `UIElement`/deeper ancestors transitively through the base's own supertrait chain —
    // see `inherits_path`'s own doc comment), `impl #target for #targetImpl { .. }`, the `UIElement`
    // blind-forward (skipped via `no_ui_element` for host composition, whose base — `Window` —
    // doesn't implement `UIElement` at all), and `pub fn new(..) -> Rc<Self>` — all automatically,
    // once this component's own `construct`/`own_class_methods` below are in place — ancestor-trait
    // forwarding itself is entirely `#[class]`'s own job now (its hop-0/transitive handling in
    // `elwindui-macros`), not something this function generates. A non-composed component declares
    // neither attribute (plain struct, no
    // class-hierarchy participation).
    //
    // The immediate base's own trait path — bare `X` for a consumer-defined base, `elwindui::ui::X`
    // for a builtin (`concrete_type_ident`'s own "is_builtin" rule, applied to the trait name rather
    // than the `Impl`-suffixed struct name). Deliberately the *immediate* base
    // (`resolved_root.type_path`/`component.base`/`"Window"`), not the transitively-resolved
    // `composed_shape` rather than the immediate base, e.g. `Control`, for
    // a template-composed `LabeledPanel inherits ContentControl`): `#target: ContentControl` alone
    // already reaches `Control`/`UIElement` transitively through `ContentControl`'s own supertrait
    // chain, exactly like `elwindui_core::ui::TextArea: NativeControl` does — no need to skip ahead
    // to every ancestor through the supertrait chain.
    let base_trait_path = |name: &str| -> TokenStream {
        if let Some(qualified) = immediate_base_qualified_path(component, name) {
            return qualified;
        }
        // `info.is_none()` (external, no local `TypeInfo`) treated the same as a known builtin —
        // same rule `concrete_type_ident` already applies, for the same reason (see its own doc
        // comment): every name unresolved here is one, by construction.
        let info = table.resolve(from, name);
        dsl_concrete_type_path(name, info)
    };
    // The literal name (DSL-level, e.g. `"ContentControl"`/`"Rectangle"`/`"Window"`) this
    // component's own generated trait bound (`inherits_path`) is keyed off — the *immediate* base
    // actually embedded as this component's own `base: <BaseImpl>` field (`resolved_root.type_path` for
    // shape composition,
    // `component.base` for template composition, `"Window"` for host composition), deliberately
    // *not* the transitively-resolved `composed_shape`.
    let immediate_base_name: Option<String> = if is_shape_composition {
        Some(resolved_root.type_path.clone())
    } else if is_inherited_view_composition {
        component.base.clone()
    } else {
        host_composition_base.clone()
    };
    // `#[class]`'s own `inherits = ..` argument always names the base's *struct* (bare `X` for a
    // consumer-defined base, `elwindui::ui::X` for a builtin — `concrete_type_ident`'s own
    // "is_builtin" rule — or `shape_composition_base_type`'s `elwindui::core::ui::X`
    // struct path for a raw virtual-builtin shape); the macro derives the matching `XExt` supertrait
    // bound on `#target`'s own generated trait internally (docs/design/runtime/ui_tree_design.md) — never
    // something this function needs to spell out itself. `#target: <immediate base>` already reaches
    // every deeper ancestor (down to `UIElement`) through the base's own supertrait chain — exactly
    // like `elwindui_core::ui::TextAreaExt: NativeControlExt` does — so there's no need to skip
    // every transitive ancestor through the base trait's supertrait chain.
    let inherits_path: TokenStream = match &immediate_base_name {
        Some(name)
            if table
                .resolve(from, name)
                .is_some_and(|i| i.is_virtual_builtin) =>
        {
            shape_composition_base_type(name)
        }
        Some(name) => base_trait_path(name),
        None => TokenStream::new(),
    };
    let class_methods = emit_class_methods(&component.methods, &source_component.methods);
    // A composed Rust `#[component]` is also a class declaration. Preserve its own public
    // property/content shape on the generated `#[class]` struct so a downstream crate can use the
    // exported `__elwindui_props_<Type>!` macro. This is deliberately limited to shape/template
    // composition: host composition wraps a backend `Window` façade and does not expose that
    // façade as a generated component-property surface. Computed/state/environment fields remain
    // component internals rather than writable external class properties.
    let generated_class_prop_decls: Vec<TokenStream> =
        if is_shape_composition || is_inherited_view_composition {
            source_component
                .fields
                .iter()
                .filter(|field| {
                    field.kind == FieldKind::Prop
                        && !(field.initializer.is_none() && field.name.starts_with("on_"))
                })
                .map(|field| {
                    let name = format_ident!("{}", field.name);
                    let ty: syn::Type = syn::parse_str(&field.ty)
                        .expect("component property type must parse for class declaration");
                    let mut flags: Vec<TokenStream> = field
                        .attrs
                        .iter()
                        .filter_map(|attr| match attr {
                            Attr::Routed => Some(quote! { routed }),
                            Attr::Onetime => Some(quote! { onetime }),
                            Attr::TwoWay => Some(quote! { two_way }),
                            Attr::SemanticBrush => Some(quote! { semantic_brush }),
                            _ => None,
                        })
                        .collect();
                    flags.insert(0, quote! { owned });
                    quote! { #[prop(#(#flags,)* #name: #ty)] }
                })
                .collect()
        } else {
            Vec::new()
        };
    let generated_class_content_decl = if is_shape_composition || is_inherited_view_composition {
        source_component.content_field.as_ref().map(|field| {
            let field = format_ident!("{}", field);
            quote! { #[content(#field)] }
        })
    } else {
        None
    };
    // PR #165 post-final rereview remediation, A9 (§10.2): the implicit-bind-owner counterpart to
    // `property_resync_methods` — `property_resync_methods_for` itself needs no changes at all,
    // since `collect_view_expr_owner_properties`/`view_expr_depends_on`/`emit_resync` only ever
    // compare `owner_name` by plain string equality against a `ViewExpr::Path`'s own first
    // segment, never checking whether that name is a *physical* field — reusing it with
    // `implicit_bind_owners` (`["vm"]`) already produces a correct `__resync_vm` method whose
    // per-property read expressions go through `emit_expr`/`path_owner_value_tokens` (already
    // fixed to bridge `vm.field` through `__view_owner`, A8) unmodified.
    let property_resync_methods: TokenStream = mark_inherent(property_resync_methods_for(
        &bind_owners,
        &plan,
        &ctx,
        from,
        table,
        true,
        is_shape_composition || is_host_composition,
    ));
    let implicit_property_resync_methods: TokenStream = mark_inherent(property_resync_methods_for(
        &implicit_bind_owners,
        &plan,
        &ctx,
        from,
        table,
        true,
        is_shape_composition || is_host_composition,
    ));
    let property_resync_methods: TokenStream =
        quote! { #property_resync_methods #implicit_property_resync_methods };
    let lazy_leaves_for_own_resync = collect_lazy_leaves(&plan);
    let component_property_resync_methods: TokenStream = component_property_variants
        .iter()
        .map(|property| {
            let method = format_ident!("__resync_{}", property);
            let property_name = property.to_string();
            let mut statements = TokenStream::new();
            for node in &plan {
                let self_is_node = (is_shape_composition || is_host_composition)
                    && node.binding == plan[root_index].binding;
                emit_resync(
                    node,
                    &ctx,
                    from,
                    table,
                    ResyncFilter::Property("", &property_name),
                    &mut statements,
                    self_is_node,
                );
            }
            for (cache_field, leaf) in &lazy_leaves_for_own_resync {
                emit_lazy_branch_resync(
                    cache_field,
                    leaf,
                    &ctx,
                    from,
                    table,
                    ResyncFilter::Property("", &property_name),
                    &mut statements,
                );
            }
            quote! {
                fn #method(&self) {
                    #statements
                }
            }
        })
        .collect();
    let component_property_dispatch: TokenStream = component_property_variants
        .iter()
        .map(|property| {
            let method = format_ident!("__resync_{}", property);
            quote! {
                #component_property_enum::#property => {
                    this.#method();
                    #target_ext::__refresh_dynamic_regions(&*this);
                },
            }
        })
        .collect();
    let component_self_subscription = if component_property_variants.is_empty() {
        TokenStream::new()
    } else {
        quote! {
            {
                let weak = std::rc::Rc::downgrade(&this);
                let subscription = this.subscribe_property_changed(move |property| {
                    if let Some(this) = weak.upgrade() {
                        match property { #component_property_dispatch }
                    }
                });
                this.__property_changed_subscriptions.borrow_mut().push(subscription);
            }
        }
    };

    // `on_update(field, ...)`/bare `on_update { .. }` (docs/specs/dsl_spec.md §3, CI-4 of #80,
    // docs/design/runtime/component_lifecycle_design.md §4c). Reuses the exact same
    // `subscribe_property_changed` mechanism `component_self_subscription` above already installs
    // (own-field-changed -> `__resync_<field>` dispatch) — a second, independent subscription rather
    // than folding into that one, since this one runs the DSL author's own block instead of an
    // internal resync method, and is optional (most components have none). Installed from the same
    // post-construction point as `on_mount_stmt` (never from `construct`), so it can never observe
    // the initial, construction-time field value-set: that happens via the plain `Self { field:
    // value, .. }` struct literal, which never calls `self.on_property_changed(..)` — only a
    // generated setter does, and only a setter call after this subscription exists can be observed
    // by it.
    // PR #165 review remediation, A2: unlike `on_mount`/`on_unmount` (synchronous, inline), this
    // block runs later, inside a *stored* `subscribe_property_changed` closure — the closure
    // captures only `weak` and re-derives an owned `this: Rc<Self>` from it on each invocation
    // (below), so `this` (not `self`, which cannot be captured into a `'static` closure by
    // reference) is the correct `EmitMode::WithSelf` receiver here.
    let on_update_hook_mode = EmitMode::WithSelf(quote! { this });
    let own_on_update_subscription = view.on_update.as_ref().map(|hook| {
        let block = rewrite_view_closure_block(hook.block.clone(), &[], &ctx, &on_update_hook_mode);
        let match_arms = match &hook.fields {
            None => quote! { _ => #block },
            Some(names) => {
                let mut variant_idents = Vec::new();
                let mut unknown_errors = TokenStream::new();
                for name in names {
                    if component_property_variants.iter().any(|v| v == name) {
                        variant_idents.push(format_ident!("{}", name));
                    } else {
                        let msg = format!(
                            "on_update({name}, ..): `{name}` is not a #[prop]/#[computed]/#[state]/#[environment(..)] field of this component"
                        );
                        unknown_errors.extend(quote! { compile_error!(#msg); });
                    }
                }
                if !unknown_errors.is_empty() {
                    unknown_errors
                } else {
                    quote! {
                        #(#component_property_enum::#variant_idents)|* => #block
                        _ => {}
                    }
                }
            }
        };
        quote! {
            {
                let weak = std::rc::Rc::downgrade(&this);
                let subscription = this.subscribe_property_changed(move |property| {
                    if let Some(this) = weak.upgrade() {
                        match property { #match_arms }
                    }
                });
                this.__property_changed_subscriptions.borrow_mut().push(subscription);
            }
        }
    });

    let shared_template_factory = shared_template_body
        .as_ref()
        .map(|body| crate::emit_compiled_template_factory(body, quote! { #target }));
    // Template-enabled components return from the specialized template branch before the
    // ordinary authored-view path can install its lifecycle hooks.  Keep the component lifecycle
    // state guard on the component itself so subtree traversal marks it Unmounting before visiting
    // template children; the template backend owns the user on_unmount body, so this hook only
    // performs the generated component teardown and never invokes that body a second time.
    let template_unmount_hook_attach = (!is_host_composition).then(|| {
        quote! {
            let weak_begin = std::rc::Rc::downgrade(&this);
            <Self as elwindui::core::ui::UIElementExt>::add_begin_unmount_hook(
                self,
                Box::new(move || {
                    if let Some(this) = weak_begin.upgrade() {
                        match this.__lifecycle_state.get() {
                            elwindui::core::ui::ComponentLifecycleState::Mounted => {
                                this.__lifecycle_state
                                    .set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                                true
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                }),
            );
            let weak = std::rc::Rc::downgrade(&this);
            <Self as elwindui::core::ui::UIElementExt>::add_unmount_hook(
                self,
                Box::new(move || {
                    if let Some(this) = weak.upgrade() {
                        this.__unmount_local();
                    }
                }),
            );
        }
    });
    let custom_template_branch = view.is_template.then(|| {
        let default_template = shared_template_factory
            .as_ref()
            .expect("template view always has a shared default factory");
        quote! {
            {
                use elwindui::core::ui::ControlExt as _;
                // A component with an authored template may also be embedded as the composed
                // `base` value of a more-derived component.  In that case `__self_weak` points at
                // the outer most-derived Rc, so this component must not install its own template
                // root (the outer component's template is authoritative).  The old generated
                // path naturally skipped this branch when no Environment override existed; keep
                // the same ownership rule while using the shared default factory.
                let __template_target = self
                    .__self_weak
                    .borrow()
                    .upgrade()
                    .and_then(|value| value.downcast::<#target>().ok());
                if let Some(this) = __template_target {
                    self.__prepare_template_presentation();
                    let __environment_template = self
                        .__mount_environment
                        .get()
                        .expect("template selection: component is not yet mounted")
                        .__control_template::<#target>();
                    let __selected_template = __environment_template
                        .clone()
                        .unwrap_or_else(|| #default_template);
                    let __template_root = __selected_template.__build(
                        elwindui::core::ui::ControlTemplateContext {
                            control: this.clone(),
                            environment: self
                                .__mount_environment
                                .get()
                                .expect("template selection: component is not yet mounted")
                                .clone(),
                        },
                    );
                    self.__set_template_root(__template_root);
                    #template_unmount_hook_attach
                    #own_environment_subscribe_stmts
                    if __environment_template.is_some() {
                        #on_mount_stmt
                    }
                }
                return;
            }
        }
    });

    let component_property_names: Vec<String> = component_property_variants
        .iter()
        .map(ToString::to_string)
        .collect();
    let component_observable_impl = quote! {
        impl elwindui::core::reactive::ObservableExt for #target {
            #[allow(unreachable_code)]
            fn subscribe_property_changed(
                &self,
                f: impl Fn(&'static str) + 'static,
            ) -> elwindui::core::reactive::Subscription {
                #target::subscribe_property_changed(self, move |property| {
                    let property_name = match property {
                        #(
                            #component_property_enum::#component_property_variants =>
                                #component_property_names,
                        )*
                    };
                    f(property_name);
                })
            }
        }
    };

    if is_composed {
        let unmount_hook_attach = (!is_host_composition).then(|| {
            quote! {
                let weak_begin = std::rc::Rc::downgrade(&this);
                <Self as elwindui::core::ui::UIElementExt>::add_begin_unmount_hook(
                    self,
                    Box::new(move || {
                        if let Some(this) = weak_begin.upgrade() {
                            match this.__lifecycle_state.get() {
                                elwindui::core::ui::ComponentLifecycleState::Mounted => {
                                    this.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                                    true
                                }
                                _ => false,
                            }
                        } else {
                            false
                        }
                    }),
                );
                let weak = std::rc::Rc::downgrade(&this);
                <Self as elwindui::core::ui::UIElementExt>::add_unmount_hook(
                    self,
                    Box::new(move || {
                        if let Some(this) = weak.upgrade() {
                            this.__unmount_local();
                        }
                    }),
                );
            }
        });
        // Every one of these is purely inherent (`resync`/`#[id(..)]` child accessors/user methods/
        // lifecycle shadow hooks) — none is part of `#target`'s own generated trait — so `mark_inherent`
        // tags each with `#[inherent]` and they all land in the single `#[elwindui::class] impl
        // #target { .. }` block below instead of needing a second, separate plain `impl` purely to
        // hold them.
        //
        // `property_resync_methods` (bind-owner `__resync_<owner>` methods) is reused as-is from the
        // outer scope above, already built with `include_refresh: true` — a composed component's
        // `on_constructed` only calls `self.__refresh_dynamic_regions()` once, at construction, same
        // as the non-composed path (see that call's own site); nothing else calls it again on a bind
        // owner's later `PropertyChanged`, so composed and non-composed both need every bind-owner
        // resync arm to call it itself, exactly like `component_property_dispatch` (own-field changes,
        // a few lines above) already does unconditionally regardless of `is_composed`. A second
        // `include_refresh: false` copy used to be built here on the mistaken premise that composed
        // components get this call from elsewhere — they don't, so a bind-owner property referenced
        // only by a dynamic region's own condition/value/collection expression never switched the
        // active branch in a composed component (issue #58).
        let component_property_resync_methods = mark_inherent(component_property_resync_methods);
        let own_computed_recompute_methods = mark_inherent(own_computed_recompute_methods);
        let own_environment_recompute_methods = mark_inherent(own_environment_recompute_methods);

        let resync_method = mark_inherent(quote! {
            fn resync(&self) {
                #resync_stmts
            }
        });
        let root_embed_method = mark_inherent(root_embed_method);
        let window_lifecycle_overrides = window_lifecycle_overrides.map(mark_inherent);
        let named_accessors = mark_inherent(named_accessors);
        let shadow_hooks = mark_inherent(shadow_hooks);
        let on_unmount_method = on_unmount_method.map(mark_inherent);
        let composed_unmount_method = composed_unmount_method.map(mark_inherent);
        let mount_helper = mark_inherent(quote! {
            #[doc(hidden)]
            pub fn __mount(
                &self,
                environment: elwindui::core::environment::EnvironmentContext,
            ) {
                <Self as #target_ext>::mount(self, environment);
            }
        });
        quote! {
            #[allow(non_camel_case_types)]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum #component_property_enum {
                #(#component_property_variants),*
            }

            #[elwindui::class(inherits = #inherits_path)]
            #(#generated_class_prop_decls)*
            #generated_class_content_decl
            pub struct #target {
                #(#plain_required_names: #plain_required_types,)*
                #mutable_required_field_decls
                #own_default_field_decls
                #own_computed_field_decls
                #own_environment_field_decls
                #deferred_own_field_decls
                #struct_fields
                __property_changed_subscriptions: std::cell::RefCell<Vec<elwindui::core::reactive::Subscription>>,
                __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<dyn Fn(#component_property_enum)>)>>>,
                // Erased to `dyn Any` (not e.g. `dyn elwindui::core::ui::UIElementExt`) so this same
                // field shape works uniformly whether `#target`'s own chain reaches `UIElementExt`
                // (shape/template composition) or not (host composition, `inherits Window` — `Window`
                // never implements `UIElementExt` at all). `#[class]`'s own `{ClassName}Ext: ..`
                // supertrait chain always transitively reaches `AsAny: Any`, so `__self_weak` (see
                // `construct`, below) always coerces into this regardless of which chain it's in.
                __self_weak: std::cell::RefCell<std::rc::Weak<dyn std::any::Any>>,
                // Set exactly once, by `mount()` (docs/design/runtime/component_lifecycle_design.md
                // §4a, CI-3 of #80). `OnceCell::set` failing on a second call *is* this component's
                // build-idempotency guard — no separate boolean flag needed. Present unconditionally
                // because every view-bearing component needs this guard, whether or not it consumes
                // Environment itself — and, as of CI-5 (§4d), it's also every `#[environment(name)]`
                // field's own resolution source, replacing the legacy, ambient-captured
                // `__environment` field this struct used to declare separately.
                __mount_environment: std::cell::OnceCell<elwindui::core::environment::EnvironmentContext>,
                __lifecycle_state: std::cell::Cell<elwindui::core::ui::ComponentLifecycleState>,
                __closed: std::cell::Cell<bool>,
            }

            #[elwindui::class]
            impl #target {
                fn construct(#(#ctor_param_names: #ctor_param_types),*) -> Self {
                    let __self_weak_erased: std::rc::Weak<dyn std::any::Any> = __self_weak.clone();
                    #construct_stmts
                    Self { #(#plain_required_names,)* #mutable_required_field_inits #own_default_field_inits #own_computed_field_inits #own_environment_field_inits #deferred_field_inits #field_inits __property_changed_subscriptions: std::cell::RefCell::new(Vec::new()), __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())), __self_weak: std::cell::RefCell::new(__self_weak_erased), __mount_environment: std::cell::OnceCell::new(), __lifecycle_state: std::cell::Cell::new(elwindui::core::ui::ComponentLifecycleState::Created), __closed: std::cell::Cell::new(false) }
                }

                // Runs automatically, exactly once, right after `#[class]`'s auto-generated `new()`
                // completes its `Rc::new_cyclic` (parent-pointer wiring, event wiring, the initial
                // `resync()`, lifecycle hooks — see `ContentControlImpl`'s own `on_constructed` doc
                // comment in `elwindui-core` for the same shape). `new()` itself is never hand-written
                // here — `#[class]` derives it from `construct` above.
                //
                // `content_attach_stmt`/`__refresh_dynamic_regions`/`resync` only ever need `&self` —
                // safe to call unconditionally, on `self` directly, regardless of how this object was
                // constructed.
                //
                // `wiring_stmts`/`component_self_subscription`/`subscribe_stmts`/
                // `on_mount_stmt`, in contrast, `move` a cloned `this`/`Rc::downgrade(&this)` into a
                // callback that outlives this call, which needs a genuine `Rc<#target>` — reconstructed
                // from the `__self_weak` field `construct` populated. `__self_weak` always upgrades
                // successfully (`on_constructed` only ever runs once the enclosing `Rc` exists) but is
                // contractually a weak reference to the *most-derived* object under construction
                // (`docs/specs/macro_class_spec.md` §13.3), not necessarily `#target` itself — when this
                // component is instead embedded as *another* generated component's own composed `base:`
                // field (Refs #25 — a user-defined `inherits` base), the most-derived object is that
                // outer component, and `downcast::<#target>()` correctly fails. Skip these `Rc`-needing
                // steps rather than panic in that case.
                //
                // KNOWN LIMITATION (docs/status/implementation_status.md): a composed base that
                // itself declares `on_*` wiring, bindable fields, or `on_mount` loses that wiring when
                // embedded this way — reaching it needs a typed weak reference alongside `__self_weak`
                // (or deferring these closures' downcast to call time), out of scope here.
                fn on_constructed(&self) {
                    // `application_environment()` (CI-6 of #80,
                    // docs/design/runtime/component_lifecycle_design.md §4e) — ambient thread-local
                    // propagation (`EnvironmentContext::current()`/`.enter()`) is removed entirely;
                    // this is now a plain, deterministic, non-stack function call. Real per-subtree
                    // derivation (something other than the single process-wide
                    // `application_environment()`) is CI-7 (`EnvironmentScope`)'s work. A host-
                    // composition (`inherits Window`) component omits this call entirely — its own
                    // `show()` override (CI-8 of #80,
                    // docs/design/runtime/component_lifecycle_design.md §4g) mounts on first call
                    // instead, so `new()` alone never builds a Window-rooted component's content.
                    #on_constructed_mount_call
                }

                // Establishes this component's effective Environment and performs its initial view
                // build, exactly once (docs/design/runtime/component_lifecycle_design.md §4a, CI-3 of
                // #80). `on_constructed` invokes this immediately today, so timing/behavior is
                // unchanged from before this method existed — a later issue in that tracking issue
                // moves the call site so child components are mounted explicitly by their parent
                // instead of automatically by `#[class]`.
                #[doc(hidden)]
                pub fn mount(&self, environment: elwindui::core::environment::EnvironmentContext) {
                    if self.__lifecycle_state.get() != elwindui::core::ui::ComponentLifecycleState::Created {
                        panic!("mount: component is already mounted or unmounted");
                    }
                    self.__mount_environment
                        .set(environment.clone())
                        .expect("mount: component is already mounted");
                    self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Mounted);
                    #mount_set_env
                    #mount_override_call
                    <Self as #target_ext>::__build_view(self);
                }

                // View-construction statements, split out of `on_constructed` so this component's
                // `new()`/`construct()`/`on_constructed()` do not themselves textually contain them
                // (docs/design/runtime/component_lifecycle_design.md §4, CI-2 of #80). Called from
                // `mount()`, once, per the build-idempotency guard there.
                #[doc(hidden)]
                fn __build_view(&self) {
                    #own_environment_resolve_stmts
                    #custom_template_branch
                    #body_prepare_stmt
                    #child_construct_stmts
                    #content_attach_stmt
                    let __most_derived: Option<std::rc::Rc<#target>> = self
                        .__self_weak
                        .borrow()
                        .upgrade()
                        .expect("__build_view: object must already be Rc-constructed")
                        .downcast::<#target>()
                        .ok();
                    if let Some(this) = __most_derived.clone() {
                        #wiring_stmts
                        #unmount_hook_attach
                    }
                    <Self as #target_ext>::__refresh_dynamic_regions(self);
                    // Most widgets already read live model state at construction time, so this is a
                    // no-op for them. A widget whose own state only ever appears in `resync()` (e.g.
                    // a dynamic list, like `TabView`'s tabs) needs this call so state populated
                    // before construction (as `main.rs` does, calling `new_tab_execute()` first)
                    // appears immediately rather than waiting for the first unrelated user
                    // interaction.
                    self.resync();
                    if let Some(this) = __most_derived {
                        #component_self_subscription
                        #subscribe_stmts
                        #own_environment_subscribe_stmts
                        #semantic_brush_subscribe_stmts
                        #own_on_update_subscription
                        #on_mount_stmt
                    }
                }

                #own_class_methods
                #component_property_api
                #resync_method
                #property_resync_methods
                #component_property_resync_methods
                #own_computed_recompute_methods
                #own_environment_recompute_methods
                #dynamic_region_refresh_method
                #root_embed_method
                #window_lifecycle_overrides
                #window_show_hide_close_overrides
                #composed_unmount_method
                #named_accessors
                #class_methods
                #shadow_hooks
                #on_unmount_method
                #mount_helper
            }

            #component_observable_impl
            #template_property_impls
        }
    } else {
        let mount_helper = quote! {
            #[doc(hidden)]
            pub fn __mount(
                self: &std::rc::Rc<Self>,
                environment: elwindui::core::environment::EnvironmentContext,
            ) {
                <Self as #target_ext>::mount(self, environment);
            }
        };
        quote! {
            #[allow(non_camel_case_types)]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum #component_property_enum {
                #(#component_property_variants),*
            }

            impl #struct_ident {
                #mount_helper
                pub fn new(#(#ctor_param_names: #ctor_param_types),*) -> std::rc::Rc<Self> {
                    let this = Self::__new_unmounted(#(#ctor_param_names),*);
                    // See the composed shape's own `on_constructed` doc comment above — same
                    // `application_environment()` call, same reasoning (CI-6 of #80).
                    this.mount(elwindui::core::environment::application_environment());
                    this
                }

                // CI-7 of #80 (docs/design/runtime/component_lifecycle_design.md §4f): the same
                // construction step as `new()` above, but without the trailing `.mount(..)` call —
                // the caller (only ever `EnvironmentScope`'s own generated code today) is
                // responsible for calling `.mount(environment)` on the returned `Rc<Self>` itself,
                // explicitly, afterward. `pub` (unlike this shape's other `#[doc(hidden)]` internals)
                // because `EnvironmentScope`'s generated code calling it lives in a *different*
                // component's own generated module.
                #[doc(hidden)]
                pub fn __new_unmounted(#(#ctor_param_names: #ctor_param_types),*) -> std::rc::Rc<Self> {
                    #content_capture_stmt
                    #construct_stmts
                    std::rc::Rc::new(Self { #(#plain_required_names,)* #mutable_required_field_inits #own_default_field_inits #own_computed_field_inits #own_environment_field_inits #deferred_field_inits #field_inits __property_changed_subscriptions: std::cell::RefCell::new(Vec::new()), __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())), __mount_environment: std::cell::OnceCell::new(), __lifecycle_state: std::cell::Cell::new(elwindui::core::ui::ComponentLifecycleState::Created) })
                }

                // Establishes this component's effective Environment and performs its initial view
                // build, exactly once (docs/design/runtime/component_lifecycle_design.md §4a, CI-3 of
                // #80). `new()` invokes this immediately today, so timing/behavior is unchanged from
                // before this method existed. `pub` (CI-7 of #80): `EnvironmentScope`'s generated code
                // calls this explicitly on a node it constructed via `__new_unmounted` above.
                #[doc(hidden)]
                pub fn mount(self: &std::rc::Rc<Self>, environment: elwindui::core::environment::EnvironmentContext) {
                    if self.__lifecycle_state.get() != elwindui::core::ui::ComponentLifecycleState::Created {
                        panic!("mount: component is already mounted or unmounted");
                    }
                    self.__mount_environment
                        .set(environment.clone())
                        .expect("mount: component is already mounted");
                    self.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Mounted);
                    #mount_set_env
                    #mount_override_call
                    self.__build_view();
                }

                // View-construction statements, split out of `mount()` so `new()`/`mount()` do not
                // themselves textually contain them (docs/design/runtime/component_lifecycle_design.md
                // §4, CI-2 of #80). Called from `mount()`, once, per the build-idempotency guard there.
                #[doc(hidden)]
                fn __build_view(self: &std::rc::Rc<Self>) {
                    #own_environment_resolve_stmts
                    #body_prepare_stmt
                    #child_construct_stmts
                    #content_attach_stmt
                    let weak_begin = std::rc::Rc::downgrade(self);
                    let weak = std::rc::Rc::downgrade(self);
                    if let Some(root) = self.#root_binding.get() {
                        elwindui::core::ui::UIElementExt::add_begin_unmount_hook(
                            &**root,
                            Box::new(move || {
                                if let Some(this) = weak_begin.upgrade() {
                                    match this.__lifecycle_state.get() {
                                        elwindui::core::ui::ComponentLifecycleState::Mounted => {
                                            this.__lifecycle_state.set(elwindui::core::ui::ComponentLifecycleState::Unmounting);
                                            true
                                        }
                                        _ => false,
                                    }
                                } else {
                                    false
                                }
                            }),
                        );
                        elwindui::core::ui::UIElementExt::add_unmount_hook(
                            &**root,
                            Box::new(move || {
                                if let Some(this) = weak.upgrade() {
                                    this.__unmount_local();
                                }
                            }),
                        );
                    }
                    #wiring_stmts
                    self.resync();
                    self.__refresh_dynamic_regions();
                    #component_self_subscription
                    #subscribe_stmts
                    #own_environment_subscribe_stmts
                    #semantic_brush_subscribe_stmts
                    #own_on_update_subscription
                    #on_mount_stmt
                }

                fn resync(&self) {
                    #resync_stmts
                }

                #plain_unmount_method
                #property_resync_methods
                #component_property_resync_methods
                #own_computed_recompute_methods
                #own_environment_recompute_methods
                #dynamic_region_refresh_method
                #component_property_api

                #root_embed_method

                #named_accessors
                #methods
                #shadow_hooks
                #on_unmount_method
            }

            pub struct #struct_ident {
                #(#plain_required_names: #plain_required_types,)*
                #mutable_required_field_decls
                #own_default_field_decls
                #own_computed_field_decls
                #own_environment_field_decls
                #deferred_own_field_decls
                #struct_fields
                __property_changed_subscriptions: std::cell::RefCell<Vec<elwindui::core::reactive::Subscription>>,
                __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<dyn Fn(#component_property_enum)>)>>>,
                // See the composed-shape struct's own `__mount_environment` doc comment above
                // (docs/design/runtime/component_lifecycle_design.md §4a, CI-3 of #80) — same guard,
                // same reasoning, for this non-`#[class]` shape.
                __mount_environment: std::cell::OnceCell<elwindui::core::environment::EnvironmentContext>,
                __lifecycle_state: std::cell::Cell<elwindui::core::ui::ComponentLifecycleState>,
            }

            #component_observable_impl
            #template_property_impls
        }
    }
}

/// Codegen-side counterpart to `crate::ast::ImplicitOwnerDef`, with every field name set already
/// converted to `HashSet` for `O(1)` membership checks during closure-body rewriting (`ast`'s own
/// version stays a plain `HashSet<String>` too — this type exists mainly so `ViewCtx`/
/// `ViewClosureRewriter` don't need to reach into `crate::ast` directly, and so a future divergence
/// between the AST-level schema and the codegen-side consumption shape has somewhere to live). PR
/// #165 final rereview remediation, A2: `readable_fields`/`writable_fields` are what makes the
/// implicit-owner fallback schema-driven instead of "any unshadowed bare name falls back to the
/// owner" — see `ViewClosureRewriter::resolved_implicit_owner_field`/`resolved_implicit_owner_setter`.
/// PR #165 post-final rereview remediation, A8/A9: `reactive_fields`/`bindable_fields` extend this
/// to source-qualified 2-segment paths (`vm.field`) and direct bare source-field dependency tracking
/// — see `crate::ast::ImplicitOwnerDef`'s own doc comment for each set's exact derivation rule.
#[derive(Clone)]
struct ImplicitOwnerCtx {
    field_name: String,
    readable_fields: HashSet<String>,
    writable_fields: HashSet<String>,
    reactive_fields: HashSet<String>,
    bindable_fields: HashSet<String>,
}

impl From<&crate::ast::ImplicitOwnerDef> for ImplicitOwnerCtx {
    fn from(def: &crate::ast::ImplicitOwnerDef) -> Self {
        ImplicitOwnerCtx {
            field_name: def.field_name.clone(),
            readable_fields: def.readable_fields.clone(),
            writable_fields: def.writable_fields.clone(),
            reactive_fields: def.reactive_fields.clone(),
            bindable_fields: def.bindable_fields.clone(),
        }
    }
}

#[derive(Clone)]
enum ViewStorage {
    /// The normal component lowerer stores nodes, dynamic slots, and environment scopes on `self`.
    Component,
    /// A template lowerer stores the same semantic values in the factory's lexical scope.  The
    /// semantic planner/emitter is deliberately shared; only this storage/receiver adapter differs
    /// between an ordinary `view!` and a `template_view!` factory.
    Template {
        environment: syn::Ident,
        refresh_cell: syn::Ident,
    },
}

struct ViewCtx {
    /// Set while evaluating a `ViewExpr::Closure` body (`key`/`render_label`/`render_content`) to
    /// the closure's own declared parameter name (e.g. `"doc"`), so a bare reference to it emits
    /// the plain local variable that name is aliased to, rather than treating it as a component
    /// field owner. `None` everywhere else.
    closure_param: Option<String>,
    /// This component's own `#[param]`-shaped fields (no initializer — the same set `generate_view`
    /// turns into `new`'s positional arguments / raw struct fields, see `param_names`), mapped to
    /// each field's own declared type string. A bare 1-segment reference to one of these (e.g.
    /// `RoundedPanel`'s own `label` used as `TextBlock { text: label }`, not `vm.something`) is the
    /// field/constructor-parameter itself, not an owner to call a getter on. The type string also lets
    /// `emit_virtual_construction`'s `get_attr`/`get_attr_string` recognize an already-`Option<T>`
    /// own field forwarded as-is, so it isn't double-wrapped in another `Some(..)`.
    own_fields: std::collections::HashMap<String, String>,
    /// The subset of `own_fields` that's Cell/RefCell-backed (`generate_view`'s
    /// `mutable_required_names` — a required, non-`#[param]` own field, still needing to be read
    /// through its Cell/RefCell in `WithSelf` mode instead of the bare `self.<name>` every other
    /// own field uses). Empty at `Construction` time's own use (`emit_expr`'s `EmitMode::
    /// Construction` reads the raw constructor-argument local instead, always bare regardless).
    mutable_own_fields: HashSet<String>,
    /// Component fields explicitly marked `#[bindable]`; only their direct properties are
    /// reactive owner dependencies in ordinary view expressions.
    bindable_owners: HashSet<String>,
    /// Reserved `ControlTemplate` owner fields are stored as `Weak<Target>` so the template
    /// instance cannot keep its templated parent alive. Expression and subscription emission
    /// upgrades these owners only for the duration of each read/resync.
    weak_bindable_owners: HashSet<String>,
    /// The component's own default `template: template_view!` factory is compiled in the
    /// component's `&self` context.  In that one context `templated_parent` resolves to `self`;
    /// standalone/external/named templates retain their typed weak parent field instead.
    default_template_parent: bool,
    /// In an explicitly declared default template, inherited fields are accessed through the
    /// composed `base` value rather than relying on a trait import for the most-derived type.
    template_base_fields: HashSet<String>,
    /// Issue #162 §3.10-§3.11: the generated field (`ViewDef::implicit_owner`, always
    /// `"__view_owner"` when set) an otherwise-unresolved bare name falls back to, for a hidden
    /// Component lowered from a `ViewExpr::DeferredView` — together with the exact schema of
    /// source-Component field names that fallback is allowed to reach (PR #165 final rereview
    /// remediation, A2 — see `ImplicitOwnerCtx`'s own doc comment for why membership is checked at
    /// all, not just shadowing). `None` for every ordinary component (including a `ControlTemplate`
    /// — `templated_parent` stays explicit-qualification-only, never an implicit bare-name
    /// fallback; see `emit_expr`'s own `ViewExpr::Path` handling).
    implicit_owner: Option<ImplicitOwnerCtx>,
    /// The concrete type being generated (`generate_view`'s own `target`) — needed by
    /// `emit_for_item_wiring` to downcast `__self_weak` the same way `on_constructed`'s own
    /// `#wiring_stmts` does (see that field's own doc comment), since a `for`-loop item's renderer
    /// closure has no already-upgraded `this: Rc<Self>` handed to it the way `on_constructed`'s
    /// body does — it only ever runs with `self: &Self` in scope (inside `__refresh_dynamic_
    /// regions`), so wiring an `on_*` attribute on an item template element has to perform that
    /// upgrade itself.
    target: syn::Ident,
    /// Optional typed parent used by the expression frontend of `template_view!`.  Ordinary
    /// generated views leave this unset; template expressions provide the local binding that
    /// represents `ControlTemplateContext<C>::control`.  Keeping this in the shared context makes
    /// path rewriting and dependency discovery identical for component, named, and standalone
    /// template sources.
    template_parent: Option<syn::Ident>,
    /// Shared compile-time property bounds collected while lowering a template expression.  The
    /// standalone frontend owns the storage, while the semantic expression backend only borrows
    /// it through this common context.
    template_property_bounds: Option<Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>>,
    /// Type used by the compile-time TemplateProperty bridge.  Standalone factories use the
    /// generic `C`; generated component/named-template factories provide their concrete target so
    /// all template frontends can share the same expression lowering.
    template_target: Option<TokenStream>,
    /// Bare property names accepted by a concrete component-default template.  They are
    /// normalized to the same typed `templated_parent.<name>` bridge used by explicit template
    /// expressions.  Standalone and named templates leave this empty.
    template_bare_parent_fields: HashSet<String>,
    /// Storage/receiver policy for the shared semantic lowerer.  Ordinary component views use
    /// generated fields and methods; ControlTemplate bodies use factory-local bindings and a
    /// refresh cell while retaining the exact same planning and value/event emitters.
    storage: ViewStorage,
}

impl ViewCtx {
    fn with_closure_param(&self, param: &str) -> ViewCtx {
        ViewCtx {
            closure_param: Some(param.to_string()),
            own_fields: self.own_fields.clone(),
            mutable_own_fields: self.mutable_own_fields.clone(),
            bindable_owners: self.bindable_owners.clone(),
            weak_bindable_owners: self.weak_bindable_owners.clone(),
            default_template_parent: self.default_template_parent,
            template_base_fields: self.template_base_fields.clone(),
            implicit_owner: self.implicit_owner.clone(),
            target: self.target.clone(),
            template_parent: self.template_parent.clone(),
            template_property_bounds: self.template_property_bounds.clone(),
            template_target: self.template_target.clone(),
            template_bare_parent_fields: self.template_bare_parent_fields.clone(),
            storage: self.storage.clone(),
        }
    }

    fn is_template_storage(&self) -> bool {
        matches!(self.storage, ViewStorage::Template { .. })
    }

    fn semantic_receiver(&self) -> TokenStream {
        if self.is_template_storage() {
            quote! { this }
        } else {
            quote! { self }
        }
    }

    fn template_environment(&self) -> Option<syn::Ident> {
        match &self.storage {
            ViewStorage::Template { environment, .. } => Some(environment.clone()),
            ViewStorage::Component => None,
        }
    }

    fn template_refresh_cell(&self) -> Option<syn::Ident> {
        match &self.storage {
            ViewStorage::Template { refresh_cell, .. } => Some(refresh_cell.clone()),
            ViewStorage::Component => None,
        }
    }

    /// Returns an owned node handle in the scope where a shared emitter is running.  In an
    /// ordinary component the handle is an `OnceCell` field; in a template it is the local `Rc`
    /// binding emitted by the same construction pass.
    fn node_receiver(
        &self,
        binding: &syn::Ident,
        self_is_node: bool,
        receiver_override: Option<TokenStream>,
    ) -> TokenStream {
        if let Some(receiver) = receiver_override {
            return receiver;
        }
        if self_is_node {
            return self.semantic_receiver();
        }
        if self.is_template_storage() {
            quote! { #binding.clone() }
        } else {
            quote! {
                self.#binding
                    .get()
                    .expect("shared view lowerer: component is not yet mounted")
                    .clone()
            }
        }
    }

    fn dynamic_slot(&self, binding: &syn::Ident) -> TokenStream {
        let slot = dynamic_slot_ident(binding);
        if self.is_template_storage() {
            quote! { #slot }
        } else {
            quote! { self.#slot }
        }
    }

    fn refresh_statement(&self) -> TokenStream {
        if let Some(cell) = self.template_refresh_cell() {
            quote! {
                if let Some(__elwindui_template_refresh_callback) =
                    #cell.borrow().as_ref().cloned()
                {
                    __elwindui_template_refresh_callback();
                }
            }
        } else {
            quote! { this.__refresh_dynamic_regions(); }
        }
    }

    /// A `move` callback that uses the template refresh cell must own a clone, otherwise the first
    /// emitted event would move the factory's single `Rc` out of scope.  Each shared callback
    /// emitter places this shadowing statement immediately before its closure.
    fn refresh_capture(&self) -> TokenStream {
        let Some(cell) = self.template_refresh_cell() else {
            return TokenStream::new();
        };
        quote! { let #cell = std::rc::Rc::clone(&#cell); }
    }
}

/// Resolves a type for a shared emitter.  Ordinary `view!` lowering must respect the source
/// module's lexical visibility, while an expression-form `template_view!` is expanded at a call
/// site that has no `Module::path`/`use` context.  The latter may still refer to one unique
/// same-crate component registered in the symbol table, so the template adapter allows the
/// explicit context-free fallback without changing ordinary name resolution.
fn resolve_context_info<'a>(
    ctx: &ViewCtx,
    from: &Module,
    table: &'a SymbolTable,
    type_path: &str,
) -> Option<&'a TypeInfo> {
    table.resolve(from, type_path).or_else(|| {
        (ctx.is_template_storage() && !type_path.contains("::"))
            .then(|| table.resolve_unqualified(type_path))
            .flatten()
    })
}

/// The context-free counterpart used by template-only planning helpers that do not otherwise
/// carry a `ViewCtx` (dynamic content shape and attribute type constraints).
fn resolve_template_info<'a>(
    from: &Module,
    table: &'a SymbolTable,
    type_path: &str,
) -> Option<&'a TypeInfo> {
    table.resolve(from, type_path).or_else(|| {
        (!type_path.contains("::"))
            .then(|| table.resolve_unqualified(type_path))
            .flatten()
    })
}

/// The semantic result shared by every `template_view!` frontend.  The factory shells in
/// `lib.rs` only add the typed-parent/lifecycle wrapper around this value; all construction,
/// binding, dynamic-region, and environment statements come from the same planner/emitter used by
/// an ordinary `view!`.
pub(crate) struct LoweredTemplateBody {
    pub(crate) root: TokenStream,
    pub(crate) let_statements: TokenStream,
    pub(crate) refresh: TokenStream,
    pub(crate) property_bounds: Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
    pub(crate) writable_properties: BTreeSet<u64>,
    pub(crate) iterable_properties: BTreeSet<u64>,
    pub(crate) has_deferred_views: bool,
    /// Some shared emitters need the typed parent receiver even when no property subscription was
    /// collected (for example an event closure or a root dynamic replacement).  The standalone
    /// shell must therefore select its generic `C` form for those bodies as well.
    pub(crate) requires_parent: bool,
}

/// Lowers a parsed template body through the ordinary view planner and emitters.  Template-specific
/// behavior is limited to `ViewCtx`'s storage/receiver adapter and the returned refresh closure;
/// there is intentionally no recursive template compiler here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_template_body(
    body: &ViewBody,
    lets: &[crate::ast::LetBinding],
    on_mount: Option<&syn::Block>,
    on_unmount: Option<&syn::Block>,
    on_update: Option<&crate::ast::OnUpdateHook>,
    from: &Module,
    table: &SymbolTable,
    target_type: TokenStream,
    bare_parent_fields: HashSet<String>,
) -> Result<LoweredTemplateBody, String> {
    let property_bounds = Rc::new(RefCell::new(BTreeMap::new()));
    let parent_ident = format_ident!("__elwindui_template_parent");
    let environment_ident = format_ident!("__environment");
    let refresh_cell_ident = format_ident!("__elwindui_template_refresh_cell");
    let ctx = ViewCtx {
        closure_param: None,
        own_fields: HashMap::new(),
        mutable_own_fields: HashSet::new(),
        bindable_owners: HashSet::new(),
        weak_bindable_owners: HashSet::new(),
        default_template_parent: false,
        template_base_fields: HashSet::new(),
        implicit_owner: None,
        target: format_ident!("__ElwinduiTemplateTarget"),
        template_parent: Some(parent_ident.clone()),
        template_property_bounds: Some(property_bounds.clone()),
        template_target: Some(target_type.clone()),
        template_bare_parent_fields: bare_parent_fields.clone(),
        storage: ViewStorage::Template {
            environment: environment_ident.clone(),
            refresh_cell: refresh_cell_ident.clone(),
        },
    };

    let mut plan = Vec::new();
    let mut lets_map: HashMap<String, (syn::Ident, String)> = HashMap::new();
    let mut prelude = TokenStream::new();
    let mut statements = TokenStream::new();

    // Keep `let` construction in source order.  The aliases are lexical values used by later
    // `ChildEntry::Ref` nodes, while the planned nodes themselves remain in the shared flat plan
    // so their attributes, dynamic children, and environment scopes use the ordinary emitters.
    for binding in lets {
        let start = plan.len();
        let resolved = plan_element(
            &binding.element,
            &ctx,
            from,
            table,
            &mut plan,
            true,
            &lets_map,
        );
        for planned in &plan[start..] {
            if planned.dynamic.is_none() {
                emit_construction(planned, &ctx, from, table, &mut prelude, &plan);
            }
        }
        let alias = format_ident!("__elwindui_let_{}", binding.name);
        let root_binding = &resolved.0;
        prelude.extend(quote! {
            let #alias = #root_binding.clone();
        });
        lets_map.insert(binding.name.clone(), (alias, resolved.1));
    }

    let root_start = plan.len();
    let root = match body.children.as_slice() {
        [ChildEntry::Literal(element)] if element.type_path == "EnvironmentScope" => {
            let scope = plan_environment_scope(element, &mut plan, None);
            let roots = plan_children_in_scope(
                &element.children,
                &element.type_path,
                &ctx,
                from,
                table,
                &mut plan,
                &lets_map,
                Some(&scope),
            );
            if roots.len() != 1 {
                return Err(
                    "an EnvironmentScope used as a template root must contain exactly one child"
                        .into(),
                );
            }
            roots[0].clone()
        }
        [ChildEntry::Literal(element)] => {
            // Body-level attributes are the same shorthand accepted by ordinary `view!`; fold
            // them into the sole root before invoking the shared planner.
            let mut root = element.clone();
            root.attributes.extend(body.attributes.iter().cloned());
            root.attached.extend(body.attached.iter().cloned());
            root.attribute_shortcuts
                .extend(body.attribute_shortcuts.iter().cloned());
            plan_element(&root, &ctx, from, table, &mut plan, true, &lets_map)
        }
        [ChildEntry::If { .. } | ChildEntry::Match { .. }] => {
            if !body.attributes.is_empty()
                || !body.attached.is_empty()
                || !body.attribute_shortcuts.is_empty()
            {
                return Err(
                    "root properties require a static element; dynamic roots cannot receive body attributes"
                        .into(),
                );
            }
            validate_template_dynamic_root_shape(&body.children[0])?;
            plan_dynamic_entry(
                &body.children[0],
                "Control",
                &ctx,
                from,
                table,
                &mut plan,
                &lets_map,
                None,
            )
        }
        [ChildEntry::For { .. }] => {
            if !body.attributes.is_empty()
                || !body.attached.is_empty()
                || !body.attribute_shortcuts.is_empty()
            {
                return Err(
                    "root properties require a static element; a `for` region cannot be the sole ControlTemplate root"
                        .into(),
                );
            }
            return Err("a `for` region cannot be the sole ControlTemplate root".into());
        }
        [ChildEntry::Ref(name)] => {
            if !body.attributes.is_empty()
                || !body.attached.is_empty()
                || !body.attribute_shortcuts.is_empty()
            {
                return Err(
                    "root properties require a static element; a reference cannot be the sole ControlTemplate root"
                        .into(),
                );
            }
            lets_map
                .get(name)
                .cloned()
                .ok_or_else(|| format!("template root reference `{name}` is not defined"))?
        }
        _ => return Err("`template_view!` requires exactly one effective root".into()),
    };

    // Template-local branch caches must be declared before any construction that can reference a
    // lazy branch.  Keep the let-binding construction in a separate prelude until the complete
    // plan is known, then emit caches, lets, and the ordinary post-order construction in that
    // source order.
    for (cache_field, leaf) in collect_lazy_leaves(&plan) {
        let type_ident = concrete_type_ident(
            &leaf.type_path,
            resolve_template_info(from, table, &leaf.type_path),
        );
        statements.extend(quote! {
            let #cache_field: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<#type_ident>>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
        });
    }
    statements.extend(prelude);

    // Construction is post-order in the plan, so every child/attribute element exists before its
    // parent constructor consumes it.  Dynamic markers are transparent planning nodes and never
    // receive a constructor of their own.
    for planned in &plan[root_start..] {
        if planned.dynamic.is_none() {
            emit_construction(planned, &ctx, from, table, &mut statements, &plan);
        }
    }

    // Template-local dynamic slots replace the ordinary component fields.  Scalar content has no
    // slot; its branch value is swapped through the content setter instead.
    for node in &plan {
        if node.dynamic.is_none() || !has_real_dynamic_anchor(&plan, &node.binding) {
            continue;
        }
        let parent = find_dynamic_region_anchor(&plan, &node.binding);
        let parent_info = resolve_template_info(from, table, &parent.type_path);
        let Some(shape) = parent_info.map(effective_content_shape) else {
            // An unresolved/external host still has a collection/scalar decision in its exported
            // props macro.  Its slot type is selected by that macro, exactly as ordinary views do.
            let props_macro = template_dynamic_props_macro_path(&parent.type_path, parent_info);
            let item_ext = dynamic_collection_item_trait_for_type_with_props_macro_template(
                &parent.type_path,
                from,
                table,
                props_macro.clone(),
            );
            let slot = dynamic_slot_ident(&node.binding);
            statements.extend(quote! {
                let #slot: #props_macro!(@content_slot_type #item_ext) =
                    ::std::default::Default::default();
            });
            continue;
        };
        if shape == EffectiveContentShape::Scalar {
            continue;
        }
        let props_macro = template_dynamic_props_macro_path(&parent.type_path, parent_info);
        let item_ext = dynamic_collection_item_trait_for_type_with_props_macro_template(
            &parent.type_path,
            from,
            table,
            props_macro.clone(),
        );
        let slot = dynamic_slot_ident(&node.binding);
        let slot_type = if shape == EffectiveContentShape::Collection {
            quote! { elwindui::core::ui::DynamicChildSlot<#item_ext> }
        } else {
            quote! { #props_macro!(@content_slot_type #item_ext) }
        };
        statements.extend(quote! {
            let #slot: #slot_type = ::std::default::Default::default();
        });
    }

    // Property writes are capability-checked by the generated setter calls; all read dependencies
    // are collected by `emit_path_get`.  The expected value type is filled from the receiving
    // element's declaration where available, preserving the generic factory's useful diagnostics.
    constrain_template_dynamic_selectors(&plan, &property_bounds);
    constrain_template_attribute_values(&plan, from, table, &property_bounds);

    let mut wiring = TokenStream::new();
    let mut resync = TokenStream::new();
    for node in &plan {
        if node.dynamic.is_some() {
            continue;
        }
        emit_wiring(node, &ctx, from, table, &mut wiring, false);
        emit_resync(
            node,
            &ctx,
            from,
            table,
            ResyncFilter::All,
            &mut resync,
            false,
        );
    }
    for (cache_field, leaf) in collect_lazy_leaves(&plan) {
        emit_lazy_branch_resync(
            &cache_field,
            leaf,
            &ctx,
            from,
            table,
            ResyncFilter::All,
            &mut resync,
        );
    }
    emit_content_presenter_wiring(&plan, &ctx, &mut wiring);

    let dynamic_regions = emit_template_dynamic_regions(&plan, &ctx, from, table);
    statements.extend(wiring.clone());
    statements.extend(resync.clone());
    statements.extend(dynamic_regions.clone());

    let (root_binding, root_type) = root;
    let root_is_dynamic = root_type == DYNAMIC_CHILD_SLOT_MARKER;
    let root_value = if root_is_dynamic {
        dynamic_content_value(
            &plan,
            &root_binding,
            "dyn UIElementExt",
            &ctx,
            from,
            table,
            &EmitMode::Construction,
        )
    } else {
        into_node_if_needed(quote! { #root_binding }, &root_type, from, table)
    };

    let root_refresh = if root_is_dynamic {
        let value = dynamic_content_value(
            &plan,
            &root_binding,
            "dyn UIElementExt",
            &ctx,
            from,
            table,
            &EmitMode::WithSelf(quote! { this }),
        );
        quote! {
            this.__set_template_root(#value);
        }
    } else {
        TokenStream::new()
    };
    let refresh = quote! {
        #resync
        #dynamic_regions
        #root_refresh
    };

    let mut writable_properties = BTreeSet::new();
    collect_template_writable_property_keys(
        body,
        lets,
        on_mount,
        on_unmount,
        on_update,
        &bare_parent_fields,
        &mut writable_properties,
    );
    let has_deferred_views = template_body_has_deferred_views(body, lets);
    let requires_parent = !wiring.is_empty() || root_is_dynamic;
    Ok(LoweredTemplateBody {
        root: root_value,
        let_statements: statements,
        refresh,
        property_bounds,
        writable_properties,
        iterable_properties: collect_template_iterable_properties(&plan),
        has_deferred_views,
        requires_parent,
    })
}

fn validate_template_dynamic_root_shape(entry: &ChildEntry) -> Result<(), String> {
    let valid = match entry {
        ChildEntry::If {
            then_branch,
            else_branch,
            ..
        } => then_branch.len() == 1 && else_branch.len() == 1,
        ChildEntry::Match { arms, .. } => arms.iter().all(|arm| arm.body.len() == 1),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("every dynamic ControlTemplate root branch must contain exactly one element".into())
    }
}

fn has_real_dynamic_anchor(plan: &[PlannedNode], target: &syn::Ident) -> bool {
    if plan.iter().any(|candidate| {
        candidate
            .child_bindings
            .iter()
            .any(|(binding, _)| binding == target)
    }) {
        return true;
    }
    plan.iter().any(|candidate| {
        candidate
            .dynamic
            .as_ref()
            .is_some_and(|dynamic| dynamic_plan_contains_binding(dynamic, target))
            && has_real_dynamic_anchor(plan, &candidate.binding)
    })
}

fn template_dynamic_props_macro_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    dsl_props_macro_path(type_path, info)
}

fn template_dynamic_ext_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    dsl_ext_path(type_path, info)
}

fn emit_template_dynamic_regions(
    plan: &[PlannedNode],
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    plan.iter()
        .filter_map(|node| {
            node.dynamic.as_ref()?;
            let parent = plan.iter().find(|candidate| {
                candidate
                    .child_bindings
                    .iter()
                    .any(|(child, _)| child == &node.binding)
            })?;
            let parent_binding = &parent.binding;
            let parent_info = resolve_template_info(from, table, &parent.type_path);
            let parent_receiver = ctx.node_receiver(parent_binding, false, None);
            let parent_ext_path = template_dynamic_ext_path(&parent.type_path, parent_info);
            let scalar_item_ext = ItemTraitTokens::KnownIdent(format_ident!("UIElementExt"));
            let scalar_body = {
                let setter = parent_info
                    .and_then(|info| info.content_field.as_deref())
                    .map(|field| format_ident!("set_{field}"));
                emit_scalar_dynamic_node_refresh(
                    plan,
                    node,
                    parent_binding,
                    setter.as_ref(),
                    false,
                    &parent.type_path,
                    &scalar_item_ext,
                    ctx,
                    from,
                    table,
                    false,
                )
            };
            let body = match parent_info.map(effective_content_shape) {
                Some(EffectiveContentShape::Collection) => {
                    let info = parent_info.expect("collection shape must have type info");
                    let props = template_dynamic_props_macro_path(&parent.type_path, parent_info);
                    let item_ext = dynamic_collection_item_trait_for_type_with_props_macro_template(
                        &parent.type_path,
                        from,
                        table,
                        props,
                    );
                    let field = info
                        .content_field
                        .as_deref()
                        .expect("collection content must name a field");
                    let field_ident = format_ident!("{field}");
                    let host = quote! { #parent_receiver.#field_ident() };
                    emit_dynamic_node_refresh(plan, node, &host, &item_ext, ctx, from, table)
                }
                Some(EffectiveContentShape::Scalar) => scalar_body,
                Some(EffectiveContentShape::External) | None => {
                    let props = template_dynamic_props_macro_path(&parent.type_path, parent_info);
                    let item_ext = dynamic_collection_item_trait_for_type_with_props_macro_template(
                        &parent.type_path,
                        from,
                        table,
                        props.clone(),
                    );
                    let host = quote! { #props!(@content_field_get #parent_receiver) };
                    let collection =
                        emit_dynamic_node_refresh(plan, node, &host, &item_ext, ctx, from, table);
                    quote! { #props!(@content_shape { #scalar_body }, { #collection }); }
                }
            };
            let layout_children_use = if parent_info.is_some_and(|i| i.is_virtual_builtin) {
                quote! { use elwindui::core::ui::LayoutExt as _; }
            } else if parent_info.is_none() {
                quote! { #[allow(unused_imports)] use elwindui::core::ui::LayoutExt as _; }
            } else {
                TokenStream::new()
            };
            Some(quote! {
                {
                    use #parent_ext_path as _;
                    #layout_children_use
                    #body
                }
            })
        })
        .collect()
}

fn collect_template_iterable_properties(plan: &[PlannedNode]) -> BTreeSet<u64> {
    let mut keys = BTreeSet::new();
    for node in plan {
        if let Some(DynamicPlan::For { collection, .. }) = &node.dynamic {
            collect_template_property_keys(collection, &mut keys);
        }
    }
    keys
}

fn constrain_template_dynamic_selectors(
    plan: &[PlannedNode],
    bounds: &Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
) {
    for node in plan {
        let Some(dynamic) = &node.dynamic else {
            continue;
        };
        let (selector, boolean_match) = match dynamic {
            DynamicPlan::If { condition, .. } => (Some(condition), true),
            DynamicPlan::Match { value, arms } => (
                Some(value),
                arms.iter().all(|(pattern, _, _)| {
                    matches!(
                        pattern.to_token_stream().to_string().as_str(),
                        "true" | "false"
                    )
                }),
            ),
            DynamicPlan::For { .. } => (None, false),
        };
        if boolean_match {
            if let Some(selector) = selector {
                let mut keys = BTreeSet::new();
                collect_template_property_keys(selector, &mut keys);
                for key in keys {
                    bounds
                        .borrow_mut()
                        .entry(key)
                        .and_modify(|current| {
                            if current.is_none() {
                                *current = Some(quote! { bool });
                            }
                        })
                        .or_insert(Some(quote! { bool }));
                }
            }
        }
    }
}

fn constrain_template_attribute_values(
    plan: &[PlannedNode],
    from: &Module,
    table: &SymbolTable,
    bounds: &Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
) {
    for node in plan {
        let info = resolve_template_info(from, table, &node.type_path);
        for attribute in &node.attributes {
            let mut keys = BTreeSet::new();
            collect_template_property_keys(&attribute.value, &mut keys);
            if keys.is_empty() {
                continue;
            }
            let expected = info
                .and_then(|info| {
                    info.field_types
                        .get(&attribute.name)
                        .or_else(|| info.value_field_types.get(&attribute.name))
                })
                .map(|ty| {
                    syn::parse_str::<syn::Type>(ty)
                        .map(|ty| quote! { #ty })
                        .unwrap_or_else(|_| quote! { #ty })
                })
                .or_else(|| {
                    (!info.is_some_and(|info| info.is_builtin)).then(|| {
                        let props = template_dynamic_props_macro_path(&node.type_path, info);
                        let name = format_ident!("{}", attribute.name);
                        quote! { #props!(@field_type #name) }
                    })
                });
            for key in keys {
                let expected = expected.clone();
                bounds
                    .borrow_mut()
                    .entry(key)
                    .and_modify(|current| {
                        if current.is_none() {
                            *current = expected.clone();
                        }
                    })
                    .or_insert(expected);
            }
        }
    }
}

fn template_body_has_deferred_views(body: &ViewBody, lets: &[crate::ast::LetBinding]) -> bool {
    lets.iter()
        .any(|binding| template_element_has_deferred_views(&binding.element))
        || body
            .attributes
            .iter()
            .any(|attribute| template_expr_has_deferred_views(&attribute.value))
        || body.children.iter().any(template_child_has_deferred_views)
}

fn template_element_has_deferred_views(element: &ElementNode) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| template_expr_has_deferred_views(&attribute.value))
        || element
            .children
            .iter()
            .any(template_child_has_deferred_views)
}

fn template_child_has_deferred_views(child: &ChildEntry) -> bool {
    match child {
        ChildEntry::Literal(element) => template_element_has_deferred_views(element),
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            template_expr_has_deferred_views(condition)
                || then_branch.iter().any(template_child_has_deferred_views)
                || else_branch.iter().any(template_child_has_deferred_views)
        }
        ChildEntry::Match { value, arms } => {
            template_expr_has_deferred_views(value)
                || arms
                    .iter()
                    .any(|arm| arm.body.iter().any(template_child_has_deferred_views))
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            template_expr_has_deferred_views(collection)
                || body.iter().any(template_child_has_deferred_views)
        }
        ChildEntry::Ref(_) => false,
    }
}

fn template_expr_has_deferred_views(expr: &ViewExpr) -> bool {
    match expr {
        ViewExpr::DeferredView(_) => true,
        ViewExpr::Element(element) => template_element_has_deferred_views(element),
        ViewExpr::Closure { body, .. } => match body {
            ClosureBody::Element(element) => template_element_has_deferred_views(element),
            ClosureBody::Expr(_) | ClosureBody::Block(_) => false,
        },
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, value)| template_expr_has_deferred_views(value)),
        ViewExpr::Path(_) | ViewExpr::Expr(_) => false,
    }
}

/// One element flattened out of the tree, in construction order (children before parents).
struct PlannedNode {
    binding: syn::Ident,
    type_path: String,
    attributes: Vec<ViewAttribute>,
    /// Bindings of the element's *bare* nested children (`Type { ... }` written directly inside
    /// `{}`, not as `name: value`). Used to fill a resolved shape's `children`-named `#[param]`
    /// (an implicit list) or, absent one, the single field named by the component's own
    /// `#[content(field_name)]` (docs/specs/dsl_spec.md 付録A — e.g. `MenuBarItem`'s one nested
    /// `Menu`, bound to its `#[content(submenu)]` field; see `build_component_args`).
    /// Paired with each binding's own `type_path`, needed to decide (at the point it's used as
    /// someone else's argument) whether it's already an `elwindui::core::ui::Node<AnyView>` value
    /// (a virtual builtin/component) or a real native handle needing `Node::Native(..)`/
    /// `.into_any_view()` wrapping — see `into_node_if_needed`/`into_any_view_if_needed`.
    child_bindings: Vec<(syn::Ident, String)>,
    /// `Owner::field: value` attached-property setters written directly on this element (§3) —
    /// copied verbatim from `ElementNode::attached`. Consulted only when constructing this node's
    /// own `UIElementBase` (see `grid_cell_expr`); a node with none gets `GridCell::default()`.
    attached: Vec<(String, String, ViewExpr)>,
    /// `#[shortcut(...)]`-annotated attributes written directly on this element (§8.1) — copied
    /// verbatim (name-keyed, for `emit_wiring`'s lookup) from `ElementNode::attribute_shortcuts`.
    /// See that field's own doc comment for why this lives per-usage-site rather than on
    /// `TypeInfo` the way `routed_fields`/`two_way_fields` do.
    attribute_shortcuts: HashMap<String, (Vec<(Option<String>, String)>, ShortcutScope)>,
    /// Bindings of `ViewExpr::Element`-valued *attributes* (a "named single-child slot", e.g.
    /// `menu_bar: MenuBar { .. }`), keyed by attribute name — planned/constructed the same way
    /// `child_bindings` are, just addressed by name instead of position.
    element_attr_bindings: HashMap<String, (syn::Ident, String)>,
    /// Has an attribute at all (so it might need wiring/resync later), so it needs a struct field
    /// (rather than being a construction-time-only local). No per-type list to check against
    /// anymore — every resolved type is handled identically.
    stored: bool,
    /// This node's owning `LetBinding`'s `#[id("...")]`, if any — set by `generate_view` on
    /// `plan.last_mut()` right after the top-level `plan_element` call for that `let` returns
    /// (`plan_element` itself has no notion of `id`, only the `LetBinding` wrapping it does), never
    /// by `plan_element`. Drives `emit_named_accessors`.
    id: Option<String>,
    dynamic: Option<DynamicPlan>,
    /// CI-7 of #80 (docs/design/runtime/component_lifecycle_design.md §4f): dual-purpose, per
    /// `type_path`.
    ///
    /// - On an ordinary node (any `type_path` other than [`ENVIRONMENT_SCOPE_MARKER`]): the local
    ///   variable name of the `EnvironmentScope` this node was declared inside, if any. `None`
    ///   means "not inside an `EnvironmentScope`" — construct via the ordinary `Type::new(..)` path
    ///   (self-mounts via its own `application_environment()` bridge, unchanged since CI-6).
    ///   `Some(var)` means: construct via `Type::__new_unmounted(..)` instead, then call
    ///   `.mount(#var.clone())` explicitly — `#var` names a local `EnvironmentContext` some
    ///   earlier [`ENVIRONMENT_SCOPE_MARKER`] node in this same flat `plan` (always earlier: plan
    ///   order is the emission order, and a scope's setup is always planned/pushed before its own
    ///   children) already bound via `let`.
    /// - On an [`ENVIRONMENT_SCOPE_MARKER`] node itself: the *outer* scope this scope derives from,
    ///   if this `EnvironmentScope` is nested inside another one. `None` means it derives directly
    ///   from `self.__mount_environment` (the enclosing component's own effective Environment).
    environment_scope: Option<syn::Ident>,
}

/// Internal planning marker for an `EnvironmentScope { key: value, ..; <children> }` block (CI-7 of
/// #80, closes #100) — mirrors [`DYNAMIC_CHILD_SLOT_MARKER`]'s own "never a real resolved type"
/// convention. A node with this `type_path` never reaches `table.resolve` — `emit_construction`
/// checks for it before any type-table lookup — and produces no `UIElement`/Visual/Render/Layout
/// node of its own: it only ever emits a `let #binding = <outer>.derive(); #binding.set::<Key>(v);
/// ...;` local-variable statement (`binding` is the fresh per-instance `EnvironmentContext` local
/// variable name every child inside this scope references via its own `environment_scope: Some(_)`
/// field). Its own `attributes` (reused verbatim from the parsed `EnvironmentScope { key: value }`
/// element — same `ViewAttribute` shape any ordinary element's attributes use) are this scope's
/// declared key overrides, not properties of a real widget.
const ENVIRONMENT_SCOPE_MARKER: &str = "__environment_scope";

/// Internal planning marker for a transparent dynamic child range. It never names a generated
/// Rust type or a runtime element: the generated component owns a `DynamicChildSlot` field and
/// writes that range straight into its parent's declared `#[content]` collection.
const DYNAMIC_CHILD_SLOT_MARKER: &str = "__dynamic_child_slot";

#[allow(dead_code)]
type DynamicMatchArm = (
    syn::Pat,
    Vec<(syn::Ident, String)>,
    Option<Vec<PlannedNode>>,
);

enum DynamicPlan {
    If {
        condition: ViewExpr,
        then_bindings: Vec<(syn::Ident, String)>,
        else_bindings: Vec<(syn::Ident, String)>,
        /// `Some(leaves)` when this branch qualifies for lazy-once materialization (see
        /// `lazy_branch_plan`'s own doc comment for the eligibility rule) — `leaves` are the
        /// branch's own top-level root `PlannedNode`s, planned into a plan of their own rather
        /// than the shared `plan` `generate_view` iterates for eager construction/wiring/resync
        /// (so they get no `Type::new()` call, no struct field, no resync statement — matching
        /// this issue's "not constructed until first reached" requirement by simply never being
        /// in the list those passes walk). `None` means this branch is still eager, exactly as
        /// before this field existed — `then_bindings` are real `plan` entries either way, so
        /// every position/span computation (`slot_span`/`preceding_span`/`dynamic_region_start`)
        /// stays correct without caring which case applies.
        then_lazy: Option<Vec<PlannedNode>>,
        else_lazy: Option<Vec<PlannedNode>>,
    },
    Match {
        value: ViewExpr,
        arms: Vec<DynamicMatchArm>,
    },
    For {
        collection: ViewExpr,
        renderer: TokenStream,
        rc_identity: bool,
    },
}

/// Eligibility + planning for one `if`/`match` branch's lazy-once materialization. A branch
/// qualifies only when *every* one of its top-level entries is a childless literal element
/// (`ChildEntry::Literal` whose own `children` is empty) — deliberately excludes:
/// - a nested `if`/`match`/`for` region (needs its own persistent `self.#slot` field, reachable
///   from `emit_dynamic_node_refresh`'s existing recursion — moving it into this branch's own
///   lazily-constructed, non-`plan`-resident leaves would leave it with no field to be declared
///   on at all);
/// - an element with its own nested children (so this function never has to decide whether a
///   *descendant*, several `child_bindings` hops down, also needs lazy treatment — resync only
///   ever walks a lazy leaf's own direct attributes, see `emit_lazy_branch_resync`);
/// - a `ChildEntry::Ref` (an `#[id(...)]`-bound `let`, always constructed once at the top level
///   regardless of which branch currently uses it — nothing of this branch's own to defer);
/// - (CI-7 follow-up) any entry inside an active `EnvironmentScope` (`environment_scope: Some(_)`)
///   — a lazily-materialized leaf constructs later, from `__refresh_dynamic_regions`, a *different*
///   generated method than `__build_view()`'s own one-time statement sequence, where the scope's
///   derived `EnvironmentContext` was bound as a local variable — that variable is simply out of
///   scope there, so forcing eager construction instead (still correctly mounting against the
///   scope, since eager construction stays inside `__build_view()`) is the only way to keep this
///   correct without a deeper, field-backed storage mechanism for the derived context.
/// Any of these falls back to eager construction, unchanged from before lazy-once existed.
///
/// `unique_prefix` disambiguates this branch's own leaf bindings from every other branch's: each
/// call plans into its own fresh, branch-local `Vec` (starting its own binding-name counter back
/// at 0 — unlike the shared `plan` every *other* planning path threads through), so a `then`
/// branch's first leaf and an `else` branch's first leaf would otherwise both be named
/// `__rectangle_0` — a real collision once both become distinct struct fields (`lazy_branch_
/// cache_ident` derives each field's name from its own leaf's binding). The caller passes
/// something derived from the enclosing marker's own about-to-be-assigned `plan` position (already
/// unique per `if`/`match` region) plus which branch this is.
#[allow(clippy::too_many_arguments)]
fn lazy_branch_plan(
    branch: &[ChildEntry],
    parent_type_path: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    lets: &HashMap<String, (syn::Ident, String)>,
    unique_prefix: &str,
    environment_scope: Option<&syn::Ident>,
) -> Option<(Vec<(syn::Ident, String)>, Vec<PlannedNode>)> {
    // Ordinary generated views store lazy branch caches as component fields; template factories
    // store the same caches as lexical `Rc<RefCell<Option<Rc<_>>>>` bindings.  The branch planner
    // is therefore shared by both storage adapters instead of making template bodies eager.
    if branch.is_empty() {
        return None;
    }
    if environment_scope.is_some() {
        return None;
    }
    let eligible = branch
        .iter()
        .all(|entry| matches!(entry, ChildEntry::Literal(element) if element.children.is_empty()));
    if !eligible {
        return None;
    }
    let mut leaves = Vec::new();
    for entry in branch {
        plan_child_entry(
            entry,
            parent_type_path,
            ctx,
            from,
            table,
            &mut leaves,
            lets,
            None,
        );
    }
    // Rename every leaf's binding to be unique across every branch of every `if`/`match` region in
    // this view — see this function's own doc comment on why the fresh, branch-local `leaves`
    // above can't be trusted to already be unique. Leaves have no `child_bindings`/`element_attr_
    // bindings` of their own to keep in sync with the rename (the eligibility check above already
    // guarantees each is childless), so renaming just the node's own `binding` field is sufficient.
    for (index, leaf) in leaves.iter_mut().enumerate() {
        leaf.binding = format_ident!("{unique_prefix}_{index}");
    }
    let bindings = leaves
        .iter()
        .map(|leaf| (leaf.binding.clone(), leaf.type_path.clone()))
        .collect();
    Some((bindings, leaves))
}

/// The `RefCell<Option<Rc<..>>>` struct field name backing one lazily-materialized branch leaf —
/// shared by struct-field emission, `emit_lazy_leaf_value`, and `emit_lazy_branch_resync` so the
/// naming convention only lives in one place (mirrors `dynamic_slot_ident`'s own role for
/// `DynamicChildSlot` fields).
fn lazy_branch_cache_ident(binding: &syn::Ident) -> syn::Ident {
    format_ident!(
        "__lazy_branch_{}",
        binding.to_string().trim_start_matches('_')
    )
}

/// The "construct once, then reuse" value expression for one lazily-materialized `if`/`match`
/// branch leaf. `lazy_branch_plan`'s own eligibility rule guarantees `leaf` has no children of its
/// own, so `emit_construction` needs nothing from a wider `plan` beyond `leaf` itself — passed a
/// single-element slice of just `leaf` accordingly. Read together with `emit_lazy_branch_resync`
/// (keeps a *materialized* leaf's cached instance in sync with whatever bindable/observable
/// properties it depends on): ordinary components address a generated cache field, while template
/// factories address the corresponding lexical cache binding.
fn emit_lazy_leaf_value(
    leaf: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let cache_field = lazy_branch_cache_ident(&leaf.binding);
    let binding = &leaf.binding;
    let cache = if ctx.is_template_storage() {
        quote! { #cache_field }
    } else {
        quote! { self.#cache_field }
    };
    let mut construct = TokenStream::new();
    emit_construction(
        leaf,
        ctx,
        from,
        table,
        &mut construct,
        std::slice::from_ref(leaf),
    );
    quote! {
        {
            let mut __elwindui_lazy_cache = #cache.borrow_mut();
            if __elwindui_lazy_cache.is_none() {
                #construct
                *__elwindui_lazy_cache = Some(#binding);
            }
            __elwindui_lazy_cache
                .as_ref()
                .expect("just constructed above if it was None")
                .clone()
        }
    }
}

/// Keeps a lazily-materialized branch leaf's cached instance in sync with whatever bindable/
/// observable property it depends on — the lazy-branch counterpart of a direct `emit_resync` call.
/// A leaf never has its own `self.#binding` field (that's the entire point of laziness: the plan
/// slot it would have occupied doesn't exist), so this always resyncs through `cache_field`'s own
/// `RefCell<Option<Rc<..>>>` instead — and, since that cache is only ever populated, never cleared,
/// once the branch first materializes (`emit_lazy_leaf_value`), a `None` here just means "this
/// branch/arm has never been active yet, nothing to resync" and is silently skipped rather than
/// treated as an error. Emits nothing at all (not even an empty `if let`) when the leaf has no
/// attribute depending on `filter`, matching `emit_resync`'s own no-op-when-nothing-matches shape.
fn emit_lazy_branch_resync(
    cache_field: &syn::Ident,
    leaf: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    filter: ResyncFilter<'_>,
    out: &mut TokenStream,
) {
    let mut inner = TokenStream::new();
    emit_resync_with_receiver(
        leaf,
        ctx,
        from,
        table,
        filter,
        &mut inner,
        false,
        Some(quote! { __elwindui_lazy_receiver }),
    );
    if inner.is_empty() {
        return;
    }
    let cache = if ctx.is_template_storage() {
        quote! { #cache_field }
    } else {
        quote! { self.#cache_field }
    };
    out.extend(quote! {
        if let Some(__elwindui_lazy_receiver) = #cache.borrow().as_ref() {
            let __elwindui_lazy_receiver = std::rc::Rc::clone(__elwindui_lazy_receiver);
            #inner
        }
    });
}

/// Every lazily-materialized branch leaf reachable anywhere in `plan` (walking every `If`/`Match`
/// dynamic node's own `then_lazy`/`else_lazy`/per-arm lazy leaves — a nested dynamic region, Phase
/// 1, is always still eager per `lazy_branch_plan`'s own eligibility rule, so it never contributes
/// here, and `For` has no lazy leaves of its own kind at all), paired with the cache field backing
/// each one. Struct-field emission and resync-method generation both need "every lazy leaf in this
/// component, regardless of which region/branch/arm it belongs to" — walking `plan` once here
/// keeps that one flat list the single source of truth for all storage adapters.
fn collect_lazy_leaves(plan: &[PlannedNode]) -> Vec<(syn::Ident, &PlannedNode)> {
    let mut out = Vec::new();
    for node in plan {
        let Some(dynamic) = &node.dynamic else {
            continue;
        };
        match dynamic {
            DynamicPlan::If {
                then_lazy,
                else_lazy,
                ..
            } => {
                for leaves in [then_lazy, else_lazy].into_iter().flatten() {
                    for leaf in leaves {
                        out.push((lazy_branch_cache_ident(&leaf.binding), leaf));
                    }
                }
            }
            DynamicPlan::Match { arms, .. } => {
                for (_, _, lazy) in arms {
                    for leaves in lazy.iter() {
                        for leaf in leaves {
                            out.push((lazy_branch_cache_ident(&leaf.binding), leaf));
                        }
                    }
                }
            }
            DynamicPlan::For { .. } => {}
        }
    }
    out
}

/// Looks up `child` in `lazy` (a branch's own `then_lazy`/`else_lazy`/per-arm lazy leaves, if this
/// branch qualified) and emits either `emit_lazy_leaf_value`'s cache-or-construct expression (found
/// — this leaf is lazily materialized) or the unchanged `self.#child.clone()` field read (not
/// found — either this whole branch stayed eager, or `child` is a `ChildEntry::Ref` naming some
/// other, always-eager `#[id(...)]`-bound `let` that this branch merely borrows).
fn lazy_leaf_or_field_value(
    lazy: Option<&[PlannedNode]>,
    child: &syn::Ident,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    match lazy.and_then(|leaves| leaves.iter().find(|n| &n.binding == child)) {
        Some(leaf) => emit_lazy_leaf_value(leaf, ctx, from, table),
        None if ctx.is_template_storage() => quote! { #child.clone() },
        None => quote! {
            self.#child
                .get()
                .expect("__refresh_dynamic_regions: component is not yet mounted")
                .clone()
        },
    }
}

/// Plans an `EnvironmentScope { key: value, ..; <children> }` element (CI-7 of #80, closes #100)
/// into an [`ENVIRONMENT_SCOPE_MARKER`] `PlannedNode`, pushed *before* any of its own children are
/// planned — deliberately breaking the post-order (children-before-parent) convention every other
/// `plan_element_in_scope` push follows, since this node's own emitted statement (its children's
/// derived `EnvironmentContext` local variable) must exist *before* any of those children's own
/// construction statements run, not after. Returns the fresh local variable's identifier so the
/// caller can pass it down as `environment_scope` to each of the scope's own children.
///
/// `EnvironmentScope { .. }`'s own `{ key: value, .. }` body parses as an entirely ordinary
/// `ElementNode` — its override assignments are indistinguishable, syntactically, from any other
/// element's `key: value` attributes (`parser.rs`'s generic `parse_element_body` already handles
/// this; no dedicated grammar was needed for `EnvironmentScope`, unlike `if`/`match`/`for`).
///
/// A qualified cross-crate override (`EnvironmentScope { some_crate::name: value }`, Issue #129)
/// reuses the *same* parser's `Owner::field: value` attached-property grammar for the same reason
/// — `some_crate::name: value` parses into `elem.attached` exactly like a real attached-property
/// setter would, with `owner` = `some_crate`, `field` = `name`. `EnvironmentScope` never has real
/// attached properties of its own, so `elem.attached` is unambiguous here and is carried through
/// to the `PlannedNode` (unlike an ordinary element's `attached`, which `check_attached_properties`
/// validates as `Owner::field` — that check is skipped for `EnvironmentScope` specifically, see
/// `validate.rs`). `emit_environment_scope_construction` resolves `elem.attributes`'s bare entries
/// via the same-crate registry and `elem.attached`'s qualified entries via the cross-crate macro
/// (`environment_key_type_by_name`), never a fallback between the two. This restricts a qualified
/// key path to exactly one `::` (crate/alias + name) — unlike `#[environment(..)]`'s own qualified
/// form, which accepts an arbitrary-depth `syn::Path` (`attr_frontend::split_environment_key_path`)
/// since it's parsed by `syn`, not this hand-written grammar.
fn plan_environment_scope(
    elem: &ElementNode,
    out: &mut Vec<PlannedNode>,
    outer_scope: Option<&syn::Ident>,
) -> syn::Ident {
    let binding = format_ident!("__elwindui_scope_environment_{}", out.len());
    out.push(PlannedNode {
        binding: binding.clone(),
        type_path: ENVIRONMENT_SCOPE_MARKER.to_string(),
        attributes: elem.attributes.clone(),
        attached: elem.attached.clone(),
        attribute_shortcuts: HashMap::new(),
        child_bindings: Vec::new(),
        element_attr_bindings: HashMap::new(),
        stored: true,
        id: None,
        dynamic: None,
        environment_scope: outer_scope.cloned(),
    });
    binding
}

fn plan_element(
    node: &ElementNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    is_root: bool,
    lets: &HashMap<String, (syn::Ident, String)>,
) -> (syn::Ident, String) {
    plan_element_in_scope(node, ctx, from, table, out, is_root, lets, None)
}

/// Plans one `{ .. }` body's worth of bare child entries (CI-7 of #80,
/// docs/design/runtime/component_lifecycle_design.md §4f) — shared between an ordinary element's
/// own `node.children` (`plan_element_in_scope`, below) and an `EnvironmentScope`'s own `elem.
/// children` (`plan_environment_scope`'s caller), which is exactly why this exists as its own
/// function rather than being inlined into `plan_element_in_scope`: an `EnvironmentScope` nested
/// directly inside another `EnvironmentScope` must be detected by *this same* `elem.type_path ==
/// "EnvironmentScope"` check regardless of which of the two call sites is currently walking its
/// enclosing children list — inlining this into `plan_element_in_scope` only (an earlier revision
/// of this code did exactly that) left the scope's own children loop without the check, silently
/// treating a nested `EnvironmentScope` as an ordinary, unresolvable component type instead.
#[allow(clippy::too_many_arguments)]
fn plan_children_in_scope(
    children: &[ChildEntry],
    parent_type_path: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    lets: &HashMap<String, (syn::Ident, String)>,
    environment_scope: Option<&syn::Ident>,
) -> Vec<(syn::Ident, String)> {
    let mut child_bindings = Vec::new();
    for child in children {
        match child {
            ChildEntry::Literal(elem) if elem.type_path == "EnvironmentScope" => {
                // This position produces no `child_bindings` entry of its own — every one of the
                // scope's own bare children is planned as if it were a direct child of whatever
                // `children` belongs to instead (spec §9: "`EnvironmentScope` itself creates no
                // UIElement... purely a construction/mount context boundary"). Derives from the
                // *currently* active `environment_scope` (this call's own parameter — the outer
                // scope, for a nested `EnvironmentScope`), not unconditionally from
                // `self.__mount_environment` — that's what makes derive-chaining correct for
                // nesting.
                let scope_var = plan_environment_scope(elem, out, environment_scope);
                child_bindings.extend(plan_children_in_scope(
                    &elem.children,
                    &elem.type_path,
                    ctx,
                    from,
                    table,
                    out,
                    lets,
                    Some(&scope_var),
                ));
            }
            ChildEntry::Literal(elem) => child_bindings.push(plan_element_in_scope(
                elem,
                ctx,
                from,
                table,
                out,
                false,
                lets,
                environment_scope,
            )),
            ChildEntry::Ref(name) => {
                let resolved = lets.get(name).unwrap_or_else(|| {
                    panic!("`{name}` does not refer to an earlier `let` binding in this view")
                });
                child_bindings.push(resolved.clone());
            }
            ChildEntry::If { .. } | ChildEntry::Match { .. } => {
                // `if`/`match` directly inside an `EnvironmentScope` (CI-7 follow-up,
                // docs/design/runtime/component_lifecycle_design.md §4f): threaded through so each
                // branch's own literal elements mount against the active scope, same as a bare
                // literal child would. A branch that would otherwise qualify for lazy-once
                // materialization (`lazy_branch_plan`) is instead forced eager while inside a scope
                // — see that function's own eligibility-rule doc comment for why (the scope's
                // derived `EnvironmentContext` lives in a `__build_view()`-local variable that a
                // *lazily* materialized leaf, constructed later from `__refresh_dynamic_regions`,
                // cannot reach).
                child_bindings.push(plan_dynamic_entry(
                    child,
                    parent_type_path,
                    ctx,
                    from,
                    table,
                    out,
                    lets,
                    environment_scope,
                ));
            }
            ChildEntry::For { .. } => {
                // Not yet supported inside an `EnvironmentScope` (CI-7 follow-up) — a `for` loop's
                // items are constructed on demand by a persistent renderer closure that outlives
                // `__build_view()` entirely (unlike an eager `if`/`match` branch, which is part of
                // that same one-time statement sequence), so its items self-mount via the ordinary,
                // non-scoped `application_environment()` bridge, same as if no `EnvironmentScope`
                // were present — `environment_scope` is deliberately not threaded to
                // `plan_dynamic_entry` for this arm. `docs/specs/dsl_spec.md` §5 documents this
                // narrower remaining gap explicitly.
                child_bindings.push(plan_dynamic_entry(
                    child,
                    parent_type_path,
                    ctx,
                    from,
                    table,
                    out,
                    lets,
                    None,
                ));
            }
        }
    }
    child_bindings
}

/// `plan_element`'s real implementation, with an added `environment_scope` parameter (CI-7 of #80,
/// docs/design/runtime/component_lifecycle_design.md §4f): the local variable name of the
/// currently-active `EnvironmentScope` (if any) this node's own recursive planning is nested
/// inside. `plan_element` itself is kept as a `None`-forwarding wrapper so its many existing call
/// sites (top-level `let`-bindings/root, `for`-loop item templates, `render_content` closure
/// bodies, dynamic-region children) don't all need updating merely to pass `None` explicitly — none
/// of them currently need to propagate an enclosing scope in.
#[allow(clippy::too_many_arguments)]
fn plan_element_in_scope(
    node: &ElementNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    is_root: bool,
    lets: &HashMap<String, (syn::Ident, String)>,
    environment_scope: Option<&syn::Ident>,
) -> (syn::Ident, String) {
    let child_bindings = plan_children_in_scope(
        &node.children,
        &node.type_path,
        ctx,
        from,
        table,
        out,
        lets,
        environment_scope,
    );

    let mut element_attr_bindings = HashMap::new();
    for attribute in &node.attributes {
        if let ViewExpr::Element(elem) = &attribute.value {
            element_attr_bindings.insert(
                attribute.name.clone(),
                plan_element(elem, ctx, from, table, out, false, lets),
            );
        }
    }

    let attributes = node.attributes.clone();
    // Bindings are implementation details and must always be valid Rust identifiers. Qualified
    // external paths contain `::`, so derive the readable prefix from the final type segment while
    // retaining the monotonically increasing index for uniqueness.
    let binding = format_ident!(
        "__{}_{}",
        dsl_type_ident(&node.type_path).to_string().to_lowercase(),
        out.len()
    );
    // A virtual builtin (`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape`)
    // has a real `elwindui_core::ui` struct with real `set_*` setters (`TextBlockImpl::set_text`
    // etc.) just like any hand-written native or composed builtin — it's stored under the exact
    // same rule as everything else, so its attributes get resynced too (`emit_wiring`/
    // `emit_resync` already handle any `stored` node uniformly via their `if !node.stored {
    // return; }` guard — no changes needed there).
    // Scoped nodes are retained even when they have no attributes of their own. Their mounted
    // component may subscribe to the derived Environment (including semantic brush roles), so
    // dropping the only concrete `Rc` after `__build_view()` would also drop those subscriptions
    // and make later scope-override updates disappear.
    let stored = is_root || environment_scope.is_some() || !attributes.is_empty();

    out.push(PlannedNode {
        binding: binding.clone(),
        type_path: node.type_path.clone(),
        attributes,
        attached: node.attached.clone(),
        attribute_shortcuts: node
            .attribute_shortcuts
            .iter()
            .map(|(name, chords, scope)| (name.clone(), (chords.clone(), *scope)))
            .collect(),
        child_bindings,
        element_attr_bindings,
        stored,
        id: None,
        dynamic: None,
        environment_scope: environment_scope.cloned(),
    });
    (binding, node.type_path.clone())
}

fn emit_for_renderer(
    binding: &str,
    body: &[ChildEntry],
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    item_trait: &ItemTraitTokens,
    subscribe_to_item_changes: bool,
) -> TokenStream {
    let param_ident = format_ident!("{}", binding);
    let closure_ctx = ctx.with_closure_param(binding);
    let mut plan = Vec::new();
    let mut roots = Vec::new();
    for entry in body {
        let ChildEntry::Literal(element) = entry else {
            unreachable!()
        };
        roots.push(plan_element(
            element,
            &closure_ctx,
            from,
            table,
            &mut plan,
            true,
            &HashMap::new(),
        ));
    }
    let mut construct = TokenStream::new();
    for planned in &plan {
        emit_construction(planned, &closure_ctx, from, table, &mut construct, &plan);
    }
    let wiring = emit_for_item_wiring(&plan, &closure_ctx, from, table);
    let subscriptions = subscribe_to_item_changes
        .then(|| emit_for_item_subscriptions(&plan, binding, &closure_ctx, from, table))
        .unwrap_or_default();
    let children = roots.iter().map(|(binding, ty)| {
        dynamic_child_binding(quote! { #binding }, ty, item_trait, from, table)
    });
    quote! {
        |#param_ident: &_| {
            #construct
            #wiring
            let mut __dynamic_item_subscriptions = Vec::new();
            #subscriptions
            elwindui::core::ui::DynamicChild::with_children(
                vec![#(#children),*],
                __dynamic_item_subscriptions,
            )
        }
    }
}

/// Wires every `on_*` event attribute declared on an element inside a `for` loop's own item
/// template — the item-template counterpart to `emit_wiring`, which only ever walks the *shared*
/// top-level `plan` (`generate_view`'s own loop, driving `on_constructed`'s `#wiring_stmts`) and
/// therefore never reaches an element declared inside a `for` body at all. Before this existed,
/// an `on_*` attribute written on a `for`-loop item element (e.g. `TabViewItem { on_close: vm.
/// close_active_tab }` inside `for doc in vm.documents { .. }`) silently compiled to nothing —
/// `emit_construction`'s own `build_component_setters` skips `on_*`-named fields outright (they're
/// excluded from `param_fields` for exactly this reason: normally `emit_wiring` is the one thing
/// that handles them), and nothing else ever picked up the slack for a `for`-loop item.
///
/// Unlike `emit_wiring`, there is no persistent `self`/`this` field for the wired widget to live
/// on — it's a `DynamicChild`-owned temporary, (re)built fresh whenever its own `for`-loop source
/// item's `Rc` identity changes (see `DynamicChildSlot::replace_rc_items`) — so the widget is read
/// as a local binding (`#binding.clone()`) rather than `this.#binding.clone()`. There is also no
/// already-upgraded `this: Rc<Self>` sitting in scope the way `on_constructed`'s own body has one
/// (`emit_for_renderer`'s returned closure only ever runs with `self: &Self` in scope, from inside
/// `__refresh_dynamic_regions`) — so any node here that actually needs wiring performs the same
/// `__self_weak` upgrade `on_constructed` does, once, up front, shared by every wired attribute in
/// this template. Unlike `emit_for_item_subscriptions` (which deliberately never reaches back into
/// the enclosing view's dynamic-range refresh cycle — it only updates the already-created child's
/// own properties in response to *that item's own* observable changes), firing an item's `on_*`
/// callback *does* call `this.__refresh_dynamic_regions()` afterward, exactly like `emit_wiring`'s
/// own top-level callbacks do: the callback body is arbitrary user code (e.g. `vm.close_active_tab`
/// removing this very item from the `Vec` the enclosing `for` loop iterates), and nothing else
/// would otherwise re-run that loop's own diff — the mutated collection's own `#[observable]`
/// setter publishes a property-changed notification, but nothing subscribes to it on this specific
/// `for` region the way `emit_for_item_subscriptions`' per-item subscriptions do for item-local
/// properties.
///
/// `#[two_way]` fields use the same item-local lifetime: their typed callback clones the current
/// item `Rc` and calls its generated setter, while `emit_for_item_subscriptions` below owns the
/// reverse item-to-widget observer. A pure TwoWay template never upgrades or captures the
/// enclosing component, so replacing the item cannot create a component/widget/item cycle.
fn emit_for_item_wiring(
    plan: &[PlannedNode],
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let has_event_wiring = plan.iter().any(|node| {
        node.stored
            && node
                .attributes
                .iter()
                .any(|attribute| attribute.name.starts_with("on_"))
    });
    let has_two_way_wiring = plan.iter().any(|node| {
        node.stored
            && node.attributes.iter().any(|attribute| {
                attribute.kind == AssignmentKind::TwoWay
                    && table
                        .resolve(from, &node.type_path)
                        .is_some_and(|info| info.two_way_fields.contains(&attribute.name))
            })
    });
    if !has_event_wiring && !has_two_way_wiring {
        return TokenStream::new();
    }
    let self_mode = EmitMode::WithSelf(quote! { __elwindui_for_item_this });
    let mut out = TokenStream::new();
    if has_event_wiring {
        let target = &ctx.target;
        out.extend(quote! {
            let __elwindui_for_item_this: std::rc::Rc<#target> = self
                .__self_weak
                .borrow()
                .upgrade()
                .expect("for-loop item wiring: object must already be Rc-constructed")
                .downcast::<#target>()
                .expect("for-loop item wiring: most-derived object must be this component");
        });
    }
    for node in plan {
        if !node.stored {
            continue;
        }
        let binding = &node.binding;
        let info = resolve_context_info(ctx, from, table, &node.type_path);
        let widget_binding = quote! { #binding.clone() };
        for attribute in &node.attributes {
            let name = &attribute.name;
            let expr = &attribute.value;
            if attribute.kind == AssignmentKind::TwoWay {
                let Some(path) = (match expr {
                    ViewExpr::Path(path) => Some(path),
                    _ => None,
                }) else {
                    continue;
                };
                let [owner, field] = path.as_slice() else {
                    continue;
                };
                if ctx.closure_param.as_deref() != Some(owner.as_str()) {
                    continue;
                }
                let Some(target_info) = info else {
                    continue;
                };
                if !target_info.two_way_fields.contains(name) {
                    continue;
                }
                let source = format_ident!("{owner}");
                let source_setter = format_ident!("set_{field}");
                let change_setter = format_ident!("set_on_{name}_change");
                out.extend(builtin_trait_use(&node.type_path, Some(target_info)));
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        let source = std::rc::Rc::clone(#source);
                        widget.#change_setter(Box::new(move |new_value| {
                            source.#source_setter(new_value);
                        }));
                    }
                });
                continue;
            }
            if name.strip_prefix("on_").is_none() {
                continue;
            }
            if info.is_none() {
                let name_ident = format_ident!("{name}");
                let props_macro = dsl_props_macro_path(&node.type_path, None);
                let call = match expr {
                    ViewExpr::Closure { params, body } => {
                        emit_on_event_closure_body(body, params, ctx, &self_mode)
                    }
                    other => emit_expr(other, ctx, &self_mode),
                };
                let closure_params = match expr {
                    ViewExpr::Closure { params, .. } => params
                        .iter()
                        .map(|p| format_ident!("{p}"))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                let annotated_params: Vec<TokenStream> = closure_params
                    .iter()
                    .map(|p| match common_routed_payload_type(name) {
                        Some(ty) if closure_params.len() == 1 => quote! { #p: #ty },
                        _ => quote! { #p },
                    })
                    .collect();
                out.extend(quote! {
                    {
                        #[allow(unused_imports)]
                        use elwindui::ui::*;
                        let widget = #widget_binding;
                        let __elwindui_for_item_this = std::rc::Rc::clone(&__elwindui_for_item_this);
                        #props_macro!(@set widget, #name_ident, move |#(#annotated_params),*| {
                            #call;
                            __elwindui_for_item_this.__refresh_dynamic_regions();
                        });
                    }
                });
                continue;
            }
            let info = info.expect("checked above");
            out.extend(builtin_trait_use(&node.type_path, Some(info)));
            let setter = format_ident!("set_{name}");
            let is_routed = info.routed_fields.contains(name);
            if is_routed {
                let param_types = info
                    .field_types
                    .get(name)
                    .map(|ty| callback_param_types(ty))
                    .unwrap_or_default();
                let registration = emit_routed_registration(
                    name,
                    expr,
                    &param_types,
                    ctx,
                    &self_mode,
                    &quote! { widget.as_ui_element() },
                );
                let shortcut_registration = node
                    .attribute_shortcuts
                    .get(name)
                    .map(|(chords, scope)| {
                        emit_shortcut_registration(
                            name,
                            chords,
                            *scope,
                            &quote! { widget.as_ui_element() },
                        )
                    })
                    .unwrap_or_default();
                out.extend(quote! {
                    {
                        use elwindui::core::ui::UIElementExt as _;
                        let widget = #widget_binding;
                        #registration
                        #shortcut_registration
                    }
                });
                continue;
            }
            let param_types = info
                .field_types
                .get(name)
                .map(|ty| callback_param_types(ty))
                .unwrap_or_default();
            if param_types.is_empty() {
                let call = match expr {
                    ViewExpr::Closure { params, body } if params.is_empty() => {
                        emit_on_event_closure_body(body, params, ctx, &self_mode)
                    }
                    ViewExpr::Closure { params, .. } => panic!(
                        "`{name}` takes no parameters, but a closure with {} parameter(s) was given",
                        params.len()
                    ),
                    other => emit_expr(other, ctx, &self_mode),
                };
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        let __elwindui_for_item_this = std::rc::Rc::clone(&__elwindui_for_item_this);
                        widget.#setter(Box::new(move || {
                            #call;
                            __elwindui_for_item_this.__refresh_dynamic_regions();
                        }));
                    }
                });
            } else {
                let ViewExpr::Closure { params, body } = expr else {
                    panic!(
                        "`{name}` needs {} parameter(s); write an explicit closure, e.g. `{name}: |x| ...`",
                        param_types.len()
                    );
                };
                if params.len() != param_types.len() {
                    panic!(
                        "`{name}`'s closure takes {} parameter(s) but the callback field declares {}",
                        params.len(),
                        param_types.len()
                    );
                }
                let param_decls = params.iter().zip(&param_types).map(|(name, ty)| {
                    let ident = format_ident!("{}", name);
                    quote! { #ident: #ty }
                });
                let call = emit_on_event_closure_body(body, params, ctx, &self_mode);
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        let __elwindui_for_item_this = std::rc::Rc::clone(&__elwindui_for_item_this);
                        widget.#setter(Box::new(move |#(#param_decls),*| {
                            #call;
                            __elwindui_for_item_this.__refresh_dynamic_regions();
                        }));
                    }
                });
            }
        }
    }
    out
}

/// Emits observers owned by one `for` item. They update the already-created child directly;
/// importantly, they never call the enclosing view's dynamic-range refresh method. `DynamicChild`
/// retains the handles, so removing the item drops every observer before its UI is discarded.
fn emit_for_item_subscriptions(
    plan: &[PlannedNode],
    parameter: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let parameter = format_ident!("{parameter}");
    let mut out = TokenStream::new();
    for node in plan {
        let Some(info) = table.resolve(from, &node.type_path) else {
            continue;
        };
        let binding = &node.binding;
        let node_uses_owned_setters = info.is_virtual_builtin || info.has_view;
        for attribute in &node.attributes {
            let name = &attribute.name;
            let expr = &attribute.value;
            if name.starts_with("on_")
                || !info.field_types.contains_key(name)
                || matches!(
                    expr,
                    ViewExpr::Element(_) | ViewExpr::Closure { .. } | ViewExpr::DeferredView(_)
                )
                || !view_expr_references_closure_parameter(expr, parameter.to_string().as_str())
                || (info.has_view
                    && info.param_fields.iter().any(|(field, _)| field == name)
                    && !is_settable_field(
                        info,
                        &node.type_path,
                        name,
                        info.field_types.get(name).map(String::as_str).unwrap_or(""),
                    ))
            {
                continue;
            }
            let field_ty = info.field_types.get(name).map(String::as_str).unwrap_or("");
            let setter = format_ident!("set_{name}");
            let value = emit_expr(expr, ctx, &EmitMode::Construction);
            let is_copy = is_copy_type(strip_option(field_ty).0);
            let setter_call = if is_copy {
                quote! { item.#setter(#value); }
            } else if strip_option(field_ty).0.starts_with("Vec<") {
                quote! { item.#setter((#value).to_vec()); }
            } else if node_uses_owned_setters {
                let value = virtual_builtin_resync_value(field_ty, value);
                quote! { item.#setter(#value); }
            } else {
                quote! { item.#setter(&(#value)); }
            };
            let trait_use = builtin_trait_use(&node.type_path, Some(info));
            out.extend(quote! {
                {
                    #trait_use
                    let source = std::rc::Rc::clone(#parameter);
                    let subscription_source = std::rc::Rc::clone(&source);
                    let weak_item = std::rc::Rc::downgrade(&#binding);
                    __dynamic_item_subscriptions.push(source.subscribe_property_changed(move |_| {
                        if let Some(item) = weak_item.upgrade() {
                            let #parameter = &subscription_source;
                            #setter_call
                        }
                    }));
                }
            });
        }
    }
    out
}

fn view_expr_references_closure_parameter(expr: &ViewExpr, parameter: &str) -> bool {
    match expr {
        ViewExpr::Path(path) => path.first().is_some_and(|segment| segment == parameter),
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, value)| view_expr_references_closure_parameter(value, parameter)),
        ViewExpr::Expr(expr) => {
            struct Collector<'a> {
                parameter: &'a str,
                found: bool,
            }
            impl<'ast> Visit<'ast> for Collector<'_> {
                fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                    if node
                        .path
                        .segments
                        .first()
                        .is_some_and(|segment| segment.ident == self.parameter)
                    {
                        self.found = true;
                    }
                    syn::visit::visit_expr_path(self, node);
                }
            }
            let mut collector = Collector {
                parameter,
                found: false,
            };
            collector.visit_expr(expr);
            collector.found
        }
        ViewExpr::Element(_) | ViewExpr::Closure { .. } | ViewExpr::DeferredView(_) => false,
    }
}

/// The effective content destination's shape.  This small metadata-driven boundary is shared by
/// ordinary `view!` lowering and the template backend; neither compiler is allowed to infer a
/// dynamic child host from a concrete type name or from the presence of `LayoutExt`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EffectiveContentShape {
    Scalar,
    Collection,
    /// The type is known, but its effective content declaration is inherited from metadata that is
    /// not present in the current symbol table (for example a user component inheriting a builtin).
    /// Generated props macros remain authoritative for this cross-boundary case.
    External,
}

/// Resolve the effective `#[content(...)]` field's scalar/collection shape from declaration
/// metadata.  The field type checks intentionally mirror validation's content-shape predicate.
pub(crate) fn effective_content_shape(info: &TypeInfo) -> EffectiveContentShape {
    let Some(field) = info.content_field.as_deref() else {
        return EffectiveContentShape::External;
    };
    // `field_types` intentionally omits initialized fields because they are not constructor
    // parameters.  A content property may still have a default initializer, however, and its
    // declared type remains the authoritative shape for dynamic lowering.  Fall back to the
    // complete value map before treating the shape as external.
    let Some(ty) = info
        .field_types
        .get(field)
        .or_else(|| info.value_field_types.get(field))
    else {
        return EffectiveContentShape::External;
    };
    if ty.contains("UIElementCollection")
        || ty.trim_start().starts_with("Vec<")
        || ty.contains("ListExt<")
    {
        EffectiveContentShape::Collection
    } else {
        EffectiveContentShape::Scalar
    }
}

/// The trait-object element type of a parent's declared content collection, in whichever form
/// `dynamic_collection_item_trait_ty` (this function's own dispatcher, and every real caller's
/// entry point) can actually produce for it — a bare `Ident` when a shape table resolves the
/// parent (`table.resolve`'s `Some`, DSL-text-path-only since production dropped
/// the builtin shape set — Refs #14), or opaque already-`dyn`-wrapped tokens from the `@content_item_dyn`
/// shape-macro query (`content_item_dyn_type`, `elwindui-macros`) when it isn't. `dynamic_child_binding`
/// is the only reader that needs to tell the two apart (the `KnownIdent(UIElementExt)` case has its
/// own `into_node_if_needed`-based shortcut) — every other caller treats both uniformly as "already
/// the trait-object element type this dynamic region's `DynamicChildSlot<..>` is generic over".
pub(crate) enum ItemTraitTokens {
    KnownIdent(syn::Ident),
    External(TokenStream),
}

impl quote::ToTokens for ItemTraitTokens {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            ItemTraitTokens::KnownIdent(ident) => {
                tokens.extend(quote! { dyn elwindui::core::ui::#ident })
            }
            ItemTraitTokens::External(dyn_ty) => tokens.extend(quote! { #dyn_ty }),
        }
    }
}

/// Dispatches to `dynamic_collection_item_trait` (a shape table resolves `parent`) or the
/// `@content_item_dyn` shape-macro query (it doesn't) — see `ItemTraitTokens`'s own doc comment.
fn dynamic_collection_item_trait_ty(
    parent: &PlannedNode,
    from: &Module,
    table: &SymbolTable,
) -> ItemTraitTokens {
    dynamic_collection_item_trait_for_type(&parent.type_path, from, table)
}

/// Resolve the collection item trait for a host type using the same effective-content metadata
/// boundary as ordinary and template dynamic lowering.  External hosts keep the query in their
/// exported props macro; local hosts derive the trait from the declared collection item type.
pub(crate) fn dynamic_collection_item_trait_for_type(
    type_path: &str,
    from: &Module,
    table: &SymbolTable,
) -> ItemTraitTokens {
    let props_macro = dsl_props_macro_path(type_path, table.resolve(from, type_path));
    dynamic_collection_item_trait_for_type_with_props_macro(type_path, from, table, props_macro)
}

/// Variant of [`dynamic_collection_item_trait_for_type`] used by the ordinary generated-view
/// path, where an unresolved immediate base may still be a consumer-local component.  The caller
/// supplies the already-resolved props-macro path so the fallback remains valid without guessing
/// from a concrete control name.
pub(crate) fn dynamic_collection_item_trait_for_type_with_props_macro(
    type_path: &str,
    from: &Module,
    table: &SymbolTable,
    external_props_macro: TokenStream,
) -> ItemTraitTokens {
    let Some(info) = table.resolve(from, type_path) else {
        return ItemTraitTokens::External(quote! { #external_props_macro!(@content_item_dyn) });
    };
    if effective_content_shape(info) == EffectiveContentShape::Collection {
        ItemTraitTokens::KnownIdent(dynamic_collection_item_trait_for_info(info))
    } else {
        ItemTraitTokens::External(quote! { #external_props_macro!(@content_item_dyn) })
    }
}

/// Template-only variant of [`dynamic_collection_item_trait_for_type_with_props_macro`].  The
/// expression-form frontend has no lexical module, so it uses the same unique unqualified
/// component fallback as the rest of the shared template lowerer before deciding whether the
/// content item trait is known locally.
fn dynamic_collection_item_trait_for_type_with_props_macro_template(
    type_path: &str,
    from: &Module,
    table: &SymbolTable,
    external_props_macro: TokenStream,
) -> ItemTraitTokens {
    let Some(info) = resolve_template_info(from, table, type_path) else {
        return ItemTraitTokens::External(quote! { #external_props_macro!(@content_item_dyn) });
    };
    if effective_content_shape(info) == EffectiveContentShape::Collection {
        ItemTraitTokens::KnownIdent(dynamic_collection_item_trait_for_info(info))
    } else {
        ItemTraitTokens::External(quote! { #external_props_macro!(@content_item_dyn) })
    }
}

/// Resolve the props macro used for an unresolved dynamic-content host.  Ordinary generated views
/// normally encounter unresolved framework types (which live under `elwindui::core`), but an
/// immediate qualified component base can be a consumer-local type whose `#[macro_export]` props
/// macro is re-exported at the consumer crate root.  This helper keeps that distinction driven by
/// path/metadata, never by a control or property name.
fn dynamic_content_props_macro_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    dsl_props_macro_path(type_path, info)
}

fn dynamic_collection_item_trait_for_info(info: &TypeInfo) -> syn::Ident {
    let field = info.content_field.as_deref().unwrap_or("children");
    let ty = info
        .field_types
        .get(field)
        .or_else(|| info.value_field_types.get(field))
        .unwrap_or_else(|| panic!("content host has no declared collection field `{field}`"));
    if is_ui_element_type(ty) || ty.contains("UIElementCollection") {
        return format_ident!("UIElementExt");
    }
    let Some(inner) = ty
        .trim()
        .strip_prefix("Vec<")
        .or_else(|| ty.trim().split_once("ListExt<").map(|(_, value)| value))
        .and_then(|value| value.strip_suffix('>'))
    else {
        // Validation rejects control-flow beneath scalar content fields. Keep this fallback here
        // so generation can still produce a useful diagnostic for incomplete source instead of
        // panicking before the validator has a chance to report it.
        return format_ident!("{}Ext", ty.rsplit("::").next().unwrap_or(ty));
    };
    let inner = inner.trim().trim_start_matches("dyn ");
    let name = inner
        .rsplit("::")
        .next()
        .unwrap_or(inner)
        .trim_matches(|c| c == '<' || c == '>');
    format_ident!("{}Ext", name)
}

/// Only `Vec<Rc<T>>` can preserve per-item UI and subscriptions by pointer identity. Other
/// iterable values are still valid dynamic sources, but refresh by rebuilding just their slot.
/// Keeping this conservative is intentional: an unresolved expression must never be treated as
/// identity-stable merely because it happens to yield `Rc` values at runtime.
///
/// Three independent ways to prove that: the collection's own declared type textually says
/// `Vec<Rc<T>>` (checked here directly), a known viewmodel's `#[observable] Vec<T>` field (which
/// `generate_viewmodel` stores as `Vec<Rc<T>>`), or the loop body hands the item to some child
/// element's `#[bindable]` or `#[two_way]` field (`for_body_binds_item_to_a_bindable_field`, below) — the latter
/// deliberately never resolves the *item*'s own type (e.g. `DocumentViewModel`) at all, only the
/// *receiving component*'s (e.g. `DocumentView`), for the same reason `#[bindable]` itself exists: a
/// `#[elwindui::viewmodel]` type is commonly declared in a plain `.rs` file (or a sibling
/// `#[elwindui::component]` proc-macro invocation) that the `for` loop's own file/module never has
/// a `use` for and was never going to need one, since it only ever references the item through the
/// loop variable — so a resolve-by-name check against *that* type is fragile in a way a check
/// against the always-in-scope receiving component type isn't (see
/// `docs/design/backends/winui3_backend_design.md`'s "Root cause of 'text not reflected after Open'" for the
/// concrete bug this replaced).
fn collection_uses_rc_identity(
    collection: &ViewExpr,
    body: &[ChildEntry],
    binding: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> bool {
    if collection_type_is_vec_rc(collection, ctx, from, table) {
        return true;
    }
    for_body_binds_item_to_a_bindable_field(body, binding, from, table)
}

fn collection_type_is_vec_rc(
    collection: &ViewExpr,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> bool {
    let ViewExpr::Path(path) = collection else {
        return false;
    };
    let (collection_type, is_viewmodel_observable) = match path.as_slice() {
        [field] => match ctx.own_fields.get(field) {
            Some(collection_type) => (collection_type.as_str(), false),
            None => return false,
        },
        [owner, field] => {
            let Some(owner_type) = ctx.own_fields.get(owner) else {
                return false;
            };
            let Some(owner_info) = table.resolve(from, strip_rc_wrapper(owner_type)) else {
                return false;
            };
            let Some(collection_type) = owner_info.value_field_types.get(field) else {
                return false;
            };
            (
                collection_type.as_str(),
                owner_info.is_viewmodel
                    && matches!(owner_info.fields.get(field), Some(FieldKind::Observable))
                    && nested_vec_item_type(collection_type, from, table).is_some(),
            )
        }
        _ => return false,
    };
    let compact = collection_type
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    is_viewmodel_observable
        || compact.starts_with("Vec<Rc<")
        || compact.starts_with("Vec<std::rc::Rc<")
        || compact.starts_with("Vec<rc::Rc<")
}

/// Whether `binding` (the `for`-loop's own bare loop variable) is passed, anywhere in `body`
/// (recursing into nested elements and `if`/`match` branches, but not into a nested `for`'s own
/// body — a shadowing or unrelated inner loop variable can't carry the outer one), as the exact
/// value of some attribute whose receiving element's resolved type declares that field
/// `#[bindable]`/`#[two_way]`. See `collection_uses_rc_identity`'s doc comment for why this is
/// checked against the *receiving* component's type rather than the item's own.
pub(crate) fn for_body_binds_item_to_a_bindable_field(
    body: &[ChildEntry],
    binding: &str,
    from: &Module,
    table: &SymbolTable,
) -> bool {
    body.iter().any(|entry| match entry {
        ChildEntry::Literal(element) => {
            let bound_here = table.resolve(from, &element.type_path).is_some_and(|info| {
                element.attributes.iter().any(|attribute| {
                    matches!(&attribute.value, ViewExpr::Path(path) if path.len() == 1 && path[0] == binding)
                        && (info.bindable_fields.contains(&attribute.name)
                            || info.two_way_fields.contains(&attribute.name))
                })
            });
            bound_here
                || for_body_binds_item_to_a_bindable_field(&element.children, binding, from, table)
        }
        ChildEntry::If {
            then_branch,
            else_branch,
            ..
        } => {
            for_body_binds_item_to_a_bindable_field(then_branch, binding, from, table)
                || for_body_binds_item_to_a_bindable_field(else_branch, binding, from, table)
        }
        ChildEntry::Match { arms, .. } => arms
            .iter()
            .any(|arm| for_body_binds_item_to_a_bindable_field(&arm.body, binding, from, table)),
        ChildEntry::For { .. } | ChildEntry::Ref(_) => false,
    })
}

pub(crate) fn dynamic_child_binding(
    binding: TokenStream,
    child_type: &str,
    item_trait: &ItemTraitTokens,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    if matches!(item_trait, ItemTraitTokens::KnownIdent(ident) if ident == "UIElementExt") {
        return into_node_if_needed(binding, child_type, from, table);
    }
    quote! {
        {
            let __child: std::rc::Rc<#item_trait> = #binding;
            __child
        }
    }
}

/// Phase 2: the construction-time value for a scalar `#[content(...)]` field whose sole bare child
/// is a dynamic (`if`/`match`) region — `marker_binding` names that region's own
/// `DYNAMIC_CHILD_SLOT_MARKER` `PlannedNode`, found in `plan`. Evaluates the region's own
/// condition/value exactly once, as a genuine Rust `if`/`match` *expression* (mirroring
/// `emit_scalar_dynamic_node_refresh`'s structure, which does the same evaluation for every later
/// resync), and constructs only the branch actually selected. This isn't just a nicety since Issue
/// #52's lazy-once branches: a lazy leaf's own binding is never a real local variable at
/// construction time the way an eager leaf's always is (`emit_construction` only ever runs its
/// unconditional `let #binding = ..;` for nodes that stayed in the shared `plan`), so
/// unconditionally reaching for *some* branch's binding — as this used to do, picking `then`/the
/// first arm regardless of which branch construction-time state actually selects — would reference
/// an undefined identifier whenever that guessed branch happened to be the lazy one. Evaluating for
/// real up front avoids ever touching a branch that didn't just get selected, lazy or not. `for`
/// can't reach here (Phase 2's validation rejects it under a scalar field — see `validate.rs`'s
/// `check_dynamic_child_hosts`), so only `If`/`Match` are handled.
fn initial_dynamic_content_value(
    plan: &[PlannedNode],
    marker_binding: &syn::Ident,
    inner_ty: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    dynamic_content_value(
        plan,
        marker_binding,
        inner_ty,
        ctx,
        from,
        table,
        &EmitMode::Construction,
    )
}

/// Lowers a scalar dynamic region to a value expression for either construction or a later
/// template refresh.  The branch plan itself is shared with ordinary `view!`; only the receiver
/// mode changes when a template factory evaluates its parent-dependent selector.
fn dynamic_content_value(
    plan: &[PlannedNode],
    marker_binding: &syn::Ident,
    inner_ty: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    mode: &EmitMode,
) -> TokenStream {
    let node = plan
        .iter()
        .find(|n| &n.binding == marker_binding)
        .expect("dynamic marker must be in plan");
    match node
        .dynamic
        .as_ref()
        .expect("marker binding must be a dynamic node")
    {
        DynamicPlan::If {
            condition,
            then_bindings,
            else_bindings,
            then_lazy,
            else_lazy,
        } => {
            let condition = emit_expr(condition, ctx, mode);
            let Some(then_entry) = then_bindings.first() else {
                return quote! {
                    compile_error!("a dynamic branch cannot be empty for scalar content");
                    unreachable!()
                };
            };
            let Some(else_entry) = else_bindings.first() else {
                return quote! {
                    compile_error!("a dynamic branch cannot be empty for scalar content");
                    unreachable!()
                };
            };
            let then_value = dynamic_branch_value(
                plan,
                then_entry,
                then_lazy.as_deref(),
                inner_ty,
                ctx,
                from,
                table,
                mode,
            );
            let else_value = dynamic_branch_value(
                plan,
                else_entry,
                else_lazy.as_deref(),
                inner_ty,
                ctx,
                from,
                table,
                mode,
            );
            quote! { if #condition { #then_value } else { #else_value } }
        }
        DynamicPlan::Match { value, arms } => {
            let value = emit_expr(value, ctx, mode);
            let arm_stmts = arms.iter().map(|(pattern, children, lazy)| {
                let arm_value = children.first().map_or_else(
                    || {
                        quote! {
                            compile_error!("a dynamic branch cannot be empty for scalar content");
                            unreachable!()
                        }
                    },
                    |entry| {
                        dynamic_branch_value(
                            plan,
                            entry,
                            lazy.as_deref(),
                            inner_ty,
                            ctx,
                            from,
                            table,
                            mode,
                        )
                    },
                );
                quote! { #pattern => #arm_value }
            });
            quote! { match #value { #(#arm_stmts)* } }
        }
        DynamicPlan::For { .. } => {
            quote! {
                compile_error!("a `for` region cannot be the sole content of a scalar content field");
                unreachable!()
            }
        }
    }
}

fn dynamic_branch_value(
    plan: &[PlannedNode],
    entry: &(syn::Ident, String),
    lazy: Option<&[PlannedNode]>,
    inner_ty: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    mode: &EmitMode,
) -> TokenStream {
    let (binding, ty) = entry;
    if ty == DYNAMIC_CHILD_SLOT_MARKER {
        return dynamic_content_value(plan, binding, inner_ty, ctx, from, table, mode);
    }
    let value = lazy_leaf_or_field_value(lazy, binding, ctx, from, table);
    // Synthesized/qualified trait types stringify with the path between `dyn` and the
    // trait name (for example `dyn ::elwindui_core::ui::UIElementExt`), so the older
    // `"dyn UIElement"` substring check misses exactly the Rc<dyn UIElementExt>
    // content shape used by composed controls.  Treat every UIElementExt trait spelling
    // as an erased visual child here; the source type still controls the concrete
    // conversion performed by `into_node_if_needed`.
    if is_ui_element_type(inner_ty) {
        into_node_if_needed(value, ty, from, table)
    } else {
        into_any_view_if_needed(value, inner_ty)
    }
}

/// The `DynamicChildSlot` struct field name for a dynamic `PlannedNode`'s own binding — shared by
/// every place that needs to name it (struct-field emission, refresh-code generation, span/start
/// computation) so the naming convention only lives in one place.
fn dynamic_slot_ident(binding: &syn::Ident) -> syn::Ident {
    format_ident!(
        "__dynamic_slot_{}",
        binding.to_string().trim_start_matches('_')
    )
}

/// Bindings of the dynamic markers (`DYNAMIC_CHILD_SLOT_MARKER`-typed entries — nested `if`/`match`/
/// `for` regions) appearing directly in `plan`'s own branches — not recursively; a nested marker's
/// own further-nested markers are reached by recursing into it separately (`emit_clear_dynamic_node`/
/// `slot_span` both do). `For` has no branches of its own (its body is literal-only, §Phase 1's
/// documented scope boundary), so it never contains one.
fn direct_nested_marker_bindings(plan: &DynamicPlan) -> Vec<&syn::Ident> {
    match plan {
        DynamicPlan::If {
            then_bindings,
            else_bindings,
            ..
        } => then_bindings
            .iter()
            .chain(else_bindings.iter())
            .filter(|(_, ty)| ty == DYNAMIC_CHILD_SLOT_MARKER)
            .map(|(b, _)| b)
            .collect(),
        DynamicPlan::Match { arms, .. } => arms
            .iter()
            .flat_map(|(_, children, _)| children.iter())
            .filter(|(_, ty)| ty == DYNAMIC_CHILD_SLOT_MARKER)
            .map(|(b, _)| b)
            .collect(),
        DynamicPlan::For { .. } => Vec::new(),
    }
}

/// Whether `plan`'s own branches (not recursively) contain `target` — used to find which dynamic
/// node, if any, directly encloses a nested dynamic region.
fn dynamic_plan_contains_binding(plan: &DynamicPlan, target: &syn::Ident) -> bool {
    direct_nested_marker_bindings(plan)
        .into_iter()
        .any(|b| b == target)
}

/// Returns the specific branch list (then/else/one match arm's children) of `plan` that directly
/// contains `target` — the list `preceding_span` needs to compute `target`'s local offset within
/// it. Panics if `target` isn't directly in any of `plan`'s own branches (only ever called after
/// `dynamic_plan_contains_binding` has confirmed it is).
fn dynamic_plan_branch_containing<'a>(
    plan: &'a DynamicPlan,
    target: &syn::Ident,
) -> &'a [(syn::Ident, String)] {
    match plan {
        DynamicPlan::If {
            then_bindings,
            else_bindings,
            ..
        } => {
            if then_bindings.iter().any(|(b, _)| b == target) {
                then_bindings
            } else {
                else_bindings
            }
        }
        DynamicPlan::Match { arms, .. } => arms
            .iter()
            .map(|(_, children, _)| children.as_slice())
            .find(|children| children.iter().any(|(b, _)| b == target))
            .expect("target must be in one of this Match's arms"),
        DynamicPlan::For { .. } => panic!("`For` has no branches to search"),
    }
}

/// Total number of real host children `node` (including any nested dynamic regions within it)
/// currently occupies — `node`'s own slot length plus every nested marker's own `slot_span`,
/// recursively, summed across *all* of `node`'s branches unconditionally. This is sound because
/// every branch not currently selected is kept cleared to an empty `DynamicChildSlot` (see
/// `emit_clear_dynamic_node`), so its nested markers' own `slot_span` is always 0 when inactive —
/// no need to know which branch is active just to compute a later sibling's start position.
fn slot_span(plan: &[PlannedNode], node_binding: &syn::Ident, ctx: &ViewCtx) -> TokenStream {
    let node = plan
        .iter()
        .find(|n| &n.binding == node_binding)
        .expect("dynamic node must be in plan");
    let slot_value = ctx.dynamic_slot(node_binding);
    let own = quote! { #slot_value.len() };
    let nested: Vec<TokenStream> = node
        .dynamic
        .as_ref()
        .map(|d| {
            direct_nested_marker_bindings(d)
                .into_iter()
                .map(|b| slot_span(plan, b, ctx))
                .collect()
        })
        .unwrap_or_default();
    quote! { #own #(+ #nested)* }
}

/// Sum of the spans (`slot_span` for a dynamic marker, `1usize` for a static literal child) of
/// every entry in `siblings` preceding `target` — the shared "how far into this list does `target`
/// start" computation used both for a real element's own `child_bindings` and for a dynamic node's
/// individual branch lists (`then_bindings`/`else_bindings`/a `Match` arm's children).
fn preceding_span(
    plan: &[PlannedNode],
    siblings: &[(syn::Ident, String)],
    target: &syn::Ident,
    ctx: &ViewCtx,
) -> TokenStream {
    let preceding = siblings
        .iter()
        .take_while(|(binding, _)| binding != target)
        .map(|(binding, ty)| {
            if ty == DYNAMIC_CHILD_SLOT_MARKER {
                slot_span(plan, binding, ctx)
            } else {
                quote! { 1usize }
            }
        });
    quote! { 0usize #( + (#preceding) )* }
}

/// Finds the nearest real (non-dynamic) ancestor *element* of a dynamic `PlannedNode`, walking
/// through any number of enclosing dynamic regions (nested `if`/`match`/`for`, Phase 1). A dynamic
/// node's binding appears either directly in a real element's own `child_bindings` (a top-level
/// dynamic region) or inside exactly one other dynamic node's own branch lists (a nested region) —
/// never both, and never neither in a well-formed plan.
fn find_dynamic_region_anchor<'a>(plan: &'a [PlannedNode], target: &syn::Ident) -> &'a PlannedNode {
    if let Some(parent) = plan.iter().find(|candidate| {
        candidate
            .child_bindings
            .iter()
            .any(|(child, _)| child == target)
    }) {
        return parent;
    }
    let enclosing = plan
        .iter()
        .find(|candidate| {
            candidate
                .dynamic
                .as_ref()
                .is_some_and(|d| dynamic_plan_contains_binding(d, target))
        })
        .expect("dynamic child must have a real ancestor or an enclosing dynamic region");
    find_dynamic_region_anchor(plan, &enclosing.binding)
}

/// The absolute insertion point of a dynamic node's slot within its real ancestor's host
/// collection — generalizes the old `dynamic_child_start` to walk through any number of enclosing
/// dynamic regions. For a top-level region (directly under a real element), this is exactly
/// `preceding_span` over that element's own `child_bindings`. For a nested region, it's the
/// enclosing dynamic node's own absolute start (recursively) plus `target`'s local offset within
/// whichever specific branch of the enclosing node it lives in.
fn dynamic_region_start(plan: &[PlannedNode], target: &syn::Ident, ctx: &ViewCtx) -> TokenStream {
    if let Some(parent) = plan.iter().find(|candidate| {
        candidate
            .child_bindings
            .iter()
            .any(|(child, _)| child == target)
    }) {
        return preceding_span(plan, &parent.child_bindings, target, ctx);
    }
    let enclosing = plan
        .iter()
        .find(|candidate| {
            candidate
                .dynamic
                .as_ref()
                .is_some_and(|d| dynamic_plan_contains_binding(d, target))
        })
        .expect("dynamic child must have a real ancestor or an enclosing dynamic region");
    let branch = dynamic_plan_branch_containing(
        enclosing.dynamic.as_ref().expect("just matched Some above"),
        target,
    );
    let local = preceding_span(plan, branch, target, ctx);
    let outer_start = dynamic_region_start(plan, &enclosing.binding, ctx);
    quote! { (#outer_start) + (#local) }
}

/// Partitions a dynamic node's branch bindings into its own direct static leaf children (passed to
/// `dynamic_child_binding` and placed straight into the branch's `vec![]`) and its nested dynamic
/// markers (refreshed/cleared independently — see `emit_dynamic_node_refresh`/
/// `emit_clear_dynamic_node` — since a marker has no `self.#binding` field of its own to read).
fn partition_branch_bindings(
    bindings: &[(syn::Ident, String)],
) -> (Vec<&(syn::Ident, String)>, Vec<&syn::Ident>) {
    let mut leaves = Vec::new();
    let mut nested = Vec::new();
    for entry @ (binding, ty) in bindings {
        if ty == DYNAMIC_CHILD_SLOT_MARKER {
            nested.push(binding);
        } else {
            leaves.push(entry);
        }
    }
    (leaves, nested)
}

/// Forces a dynamic node's slot (and, recursively, every nested dynamic marker within *all* of its
/// own branches) empty — removing whatever real children it currently holds from `host` and
/// resetting its tracked state to 0-length. Used when an enclosing `if`/`match` branch switches
/// away from a branch containing this node, so the node's own contribution to `slot_span` reads 0
/// again the next time a sibling's start position is computed (see `slot_span`'s own doc comment).
fn emit_clear_dynamic_node(
    plan: &[PlannedNode],
    node: &PlannedNode,
    host: &TokenStream,
    ctx: &ViewCtx,
) -> TokenStream {
    let slot = ctx.dynamic_slot(&node.binding);
    let start = dynamic_region_start(plan, &node.binding, ctx);
    // Content getters commonly return an owning `Rc<ListExt>` (user-defined hosts), while some
    // builtins expose a borrowed collection directly.  `DynamicChildSlot` intentionally accepts
    // the erased borrowed shape in both cases; reborrow the dereferenced getter result at this
    // shared boundary so ordinary and template lowering agree on the same call contract.
    let host_ref = quote! { &*#host };
    let mut out = quote! {
        #slot.replace_children(#host_ref, #start, Vec::new());
    };
    if let Some(dynamic) = &node.dynamic {
        for nested_binding in direct_nested_marker_bindings(dynamic) {
            let nested_node = plan
                .iter()
                .find(|n| &n.binding == nested_binding)
                .expect("nested marker must be in plan");
            out.extend(emit_clear_dynamic_node(plan, nested_node, host, ctx));
        }
    }
    out
}

/// Recursively emits the refresh statement for one dynamic node, targeting the real host collection
/// `host` shared by it and every nested region within it (`host_ext`/`item_ext` are likewise the
/// real ancestor's own — computed once, at the top-level `dynamic_region_refresh_method` call site,
/// and threaded down unchanged). A top-level call is made only for a real-anchored node
/// (`dynamic_region_refresh_method`'s own `plan.iter().find(..)` guard); nested markers are reached
/// purely through this function's own recursion into `then_bindings`/`else_bindings`/`Match` arms,
/// never as a separate top-level entry.
fn emit_dynamic_node_refresh(
    plan: &[PlannedNode],
    node: &PlannedNode,
    host: &TokenStream,
    item_ext: &ItemTraitTokens,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let slot = ctx.dynamic_slot(&node.binding);
    let start = dynamic_region_start(plan, &node.binding, ctx);
    let host_ref = quote! { &*#host };
    match node
        .dynamic
        .as_ref()
        .expect("only called for a dynamic node")
    {
        DynamicPlan::For {
            collection,
            renderer,
            rc_identity,
            ..
        } => {
            let receiver = ctx.semantic_receiver();
            let collection = emit_expr(collection, ctx, &EmitMode::WithSelf(receiver.clone()));
            if *rc_identity {
                quote! {
                    #slot.replace_rc_items(#host_ref, #start, &(#collection), #renderer);
                }
            } else {
                quote! {
                    #slot.replace_items(#host_ref, #start, #collection, #renderer);
                }
            }
        }
        DynamicPlan::If {
            condition,
            then_bindings,
            else_bindings,
            then_lazy,
            else_lazy,
        } => {
            let receiver = ctx.semantic_receiver();
            let condition = emit_expr(condition, ctx, &EmitMode::WithSelf(receiver));
            let (then_leaves, then_nested) = partition_branch_bindings(then_bindings);
            let (else_leaves, else_nested) = partition_branch_bindings(else_bindings);
            let then_children = then_leaves.iter().map(|(child, ty)| {
                let value = lazy_leaf_or_field_value(then_lazy.as_deref(), child, ctx, from, table);
                dynamic_child_binding(value, ty, item_ext, from, table)
            });
            let else_children = else_leaves.iter().map(|(child, ty)| {
                let value = lazy_leaf_or_field_value(else_lazy.as_deref(), child, ctx, from, table);
                dynamic_child_binding(value, ty, item_ext, from, table)
            });
            let refresh_nested = |bindings: &[&syn::Ident]| -> TokenStream {
                bindings
                    .iter()
                    .map(|b| {
                        let n = plan.iter().find(|n| &n.binding == *b).expect("in plan");
                        emit_dynamic_node_refresh(plan, n, host, item_ext, ctx, from, table)
                    })
                    .collect()
            };
            let clear_nested = |bindings: &[&syn::Ident]| -> TokenStream {
                bindings
                    .iter()
                    .map(|b| {
                        let n = plan.iter().find(|n| &n.binding == *b).expect("in plan");
                        emit_clear_dynamic_node(plan, n, host, ctx)
                    })
                    .collect()
            };
            let clear_else = clear_nested(&else_nested);
            let clear_then = clear_nested(&then_nested);
            let refresh_then = refresh_nested(&then_nested);
            let refresh_else = refresh_nested(&else_nested);
            quote! {
                if #condition {
                    #clear_else
                    #slot.replace_children(#host_ref, #start, vec![#(#then_children),*]);
                    #refresh_then
                } else {
                    #clear_then
                    #slot.replace_children(#host_ref, #start, vec![#(#else_children),*]);
                    #refresh_else
                }
            }
        }
        DynamicPlan::Match { value, arms } => {
            let receiver = ctx.semantic_receiver();
            let value = emit_expr(value, ctx, &EmitMode::WithSelf(receiver));
            // Each arm clears every *other* arm's own nested markers before repopulating its own —
            // never its own (unlike `If`'s fixed two-way "clear the other side" split, a `match`
            // has no single "other" side, so which markers count as "other" depends on which arm
            // ends up selected, hence computed per arm below). Clearing only the other arms (never
            // the one actually selected) is what lets a nested `for` inside the currently-active
            // arm keep reusing its previously-constructed items by `Rc` identity across refreshes —
            // clearing it too would reset that identity cache for no reason every single time.
            let arm_stmts = arms
                .iter()
                .enumerate()
                .map(|(i, (pattern, children, lazy))| {
                    let (leaves, nested) = partition_branch_bindings(children);
                    let leaf_children = leaves.iter().map(|(child, ty)| {
                        let value =
                            lazy_leaf_or_field_value(lazy.as_deref(), child, ctx, from, table);
                        dynamic_child_binding(value, ty, item_ext, from, table)
                    });
                    let clear_other_arms: TokenStream = arms
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .flat_map(|(_, (_, other_children, _))| {
                            partition_branch_bindings(other_children).1
                        })
                        .map(|b| {
                            let n = plan.iter().find(|n| &n.binding == b).expect("in plan");
                            emit_clear_dynamic_node(plan, n, host, ctx)
                        })
                        .collect();
                    let refresh_nested: TokenStream = nested
                        .iter()
                        .map(|b| {
                            let n = plan.iter().find(|n| &n.binding == *b).expect("in plan");
                            emit_dynamic_node_refresh(plan, n, host, item_ext, ctx, from, table)
                        })
                        .collect();
                    quote! {
                        #pattern => {
                            #clear_other_arms
                            #slot.replace_children(#host_ref, #start, vec![#(#leaf_children),*]);
                            #refresh_nested
                        }
                    }
                });
            quote! {
                match #value { #(#arm_stmts)* }
            }
        }
    }
}

/// Phase 2's scalar counterpart of `emit_dynamic_node_refresh`: no `DynamicChildSlot`/`start`
/// involved at all, since `validate::validate`'s `dynamic_children_reduce_to_one_element` already
/// guarantees every branch (recursively) resolves to exactly one element. Refreshing is just
/// picking the currently-selected branch's value and calling the content field's own
/// `set_<field>(..)` — emitted directly inside whichever leaf branch turns out to be selected (a
/// nested `if`/`match`, Phase 1, just narrows which leaf that is; the call to `#setter` itself only
/// ever appears once the recursion bottoms out at `emit_scalar_branch_value`'s non-marker case).
fn emit_scalar_dynamic_node_refresh(
    plan: &[PlannedNode],
    node: &PlannedNode,
    owner_binding: &syn::Ident,
    setter: Option<&syn::Ident>,
    owner_is_self: bool,
    owner_type: &str,
    item_ext: &ItemTraitTokens,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    template_presentation: bool,
) -> TokenStream {
    match node
        .dynamic
        .as_ref()
        .expect("only called for a dynamic node")
    {
        DynamicPlan::For { .. } => {
            quote! {
                compile_error!("a `for` region cannot be the sole content of a scalar content field");
            }
        }
        DynamicPlan::If {
            condition,
            then_bindings,
            else_bindings,
            then_lazy,
            else_lazy,
        } => {
            let receiver = ctx.semantic_receiver();
            let condition = emit_expr(condition, ctx, &EmitMode::WithSelf(receiver));
            let Some(then_entry) = then_bindings.first() else {
                return quote! {
                    compile_error!("a dynamic branch cannot be empty for scalar content");
                };
            };
            let Some(else_entry) = else_bindings.first() else {
                return quote! {
                    compile_error!("a dynamic branch cannot be empty for scalar content");
                };
            };
            let then_value = emit_scalar_branch_value(
                plan,
                then_entry,
                then_lazy.as_deref(),
                owner_binding,
                setter,
                owner_is_self,
                owner_type,
                item_ext,
                ctx,
                from,
                table,
                template_presentation,
            );
            let else_value = emit_scalar_branch_value(
                plan,
                else_entry,
                else_lazy.as_deref(),
                owner_binding,
                setter,
                owner_is_self,
                owner_type,
                item_ext,
                ctx,
                from,
                table,
                template_presentation,
            );
            quote! {
                if #condition { #then_value } else { #else_value }
            }
        }
        DynamicPlan::Match { value, arms } => {
            let receiver = ctx.semantic_receiver();
            let value = emit_expr(value, ctx, &EmitMode::WithSelf(receiver));
            let arm_stmts =
                arms.iter().map(|(pattern, children, lazy)| {
                    let arm_value = children.first().map_or_else(
                    || quote! {
                        compile_error!("a dynamic branch cannot be empty for scalar content");
                    },
                    |entry| {
                        emit_scalar_branch_value(
                            plan,
                            entry,
                            lazy.as_deref(),
                            owner_binding,
                            setter,
                            owner_is_self,
                            owner_type,
                            item_ext,
                            ctx,
                            from,
                            table,
                            template_presentation,
                        )
                    },
                );
                    quote! { #pattern => { #arm_value } }
                });
            quote! {
                match #value { #(#arm_stmts)* }
            }
        }
    }
}

/// A single branch's contribution to `emit_scalar_dynamic_node_refresh` — either the branch's own
/// leaf child (emits the actual `self.#owner_binding.#setter(..)` call, lazily-materialized via
/// `lazy_leaf_or_field_value` when `lazy` says this branch qualifies) or, when the branch is
/// itself a nested dynamic marker (Phase 1 — always still eager, `lazy_branch_plan`'s own
/// eligibility rule excludes it), a further recursive dispatch that bottoms out at exactly one such
/// call regardless of nesting depth.
fn emit_scalar_branch_value(
    plan: &[PlannedNode],
    entry: &(syn::Ident, String),
    lazy: Option<&[PlannedNode]>,
    owner_binding: &syn::Ident,
    setter: Option<&syn::Ident>,
    owner_is_self: bool,
    owner_type: &str,
    item_ext: &ItemTraitTokens,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    template_presentation: bool,
) -> TokenStream {
    let (binding, ty) = entry;
    if ty == DYNAMIC_CHILD_SLOT_MARKER {
        let nested = plan
            .iter()
            .find(|n| &n.binding == binding)
            .expect("nested marker must be in plan");
        return emit_scalar_dynamic_node_refresh(
            plan,
            nested,
            owner_binding,
            setter,
            owner_is_self,
            owner_type,
            item_ext,
            ctx,
            from,
            table,
            template_presentation,
        );
    }
    let value = lazy_leaf_or_field_value(lazy, binding, ctx, from, table);
    let value = dynamic_child_binding(value, ty, item_ext, from, table);
    let receiver = ctx.node_receiver(owner_binding, owner_is_self, None);
    if let Some(setter) = setter {
        quote! { #receiver.#setter(#value); }
    } else {
        let props_macro = if ctx.is_template_storage() {
            template_dynamic_props_macro_path(
                owner_type,
                resolve_template_info(from, table, owner_type),
            )
        } else {
            dsl_props_macro_path(owner_type, table.resolve(from, owner_type))
        };
        let content = quote! {
            #props_macro!(@children_erased #receiver, [#value]);
        };
        if template_presentation {
            quote! {
                {
                    use elwindui::core::ui::ControlExt as _;
                    #receiver.__set_template_root(#value);
                }
            }
        } else {
            content
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_child_entry(
    entry: &ChildEntry,
    parent_type_path: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    lets: &HashMap<String, (syn::Ident, String)>,
    environment_scope: Option<&syn::Ident>,
) -> (syn::Ident, String) {
    match entry {
        ChildEntry::Literal(element) => {
            let resolved = plan_element_in_scope(
                element,
                ctx,
                from,
                table,
                out,
                false,
                lets,
                environment_scope,
            );
            out.last_mut()
                .expect("plan_element_in_scope pushed the child root")
                .stored = true;
            resolved
        }
        ChildEntry::Ref(name) => lets.get(name).cloned().unwrap_or_else(|| {
            panic!("`{name}` does not refer to an earlier `let` binding in this view")
        }),
        ChildEntry::If { .. } | ChildEntry::Match { .. } | ChildEntry::For { .. } => {
            plan_dynamic_entry(
                entry,
                parent_type_path,
                ctx,
                from,
                table,
                out,
                lets,
                environment_scope,
            )
        }
    }
}

/// Plans an `If`/`Match`/`For` region into a transparent `DYNAMIC_CHILD_SLOT_MARKER` `PlannedNode`
/// (see that constant's own doc comment) — shared by `plan_element`'s own children loop (a
/// top-level dynamic region, directly under a real element) and `plan_child_entry` (a *nested*
/// region, inside another dynamic region's own branch/arm/body). `parent_type_path` is always the
/// nearest real (non-dynamic) ancestor *element*'s type — for a nested region that's the same real
/// ancestor its enclosing dynamic region was itself planned against, threaded through unchanged
/// (see `plan_child_entry`'s own call site) — never the immediately-enclosing `If`/`Match`/`For`,
/// which has no collection of its own to resolve an item trait against. Only used here for `For`'s
/// own `dynamic_collection_item_trait` lookup; `__refresh_dynamic_regions` (`emit_dynamic_region_refresh`)
/// separately re-derives each region's real host/insertion-point at generation time by walking
/// `plan` itself, so this function does not need to record the parent any more permanently than that.
#[allow(clippy::too_many_arguments)]
fn plan_dynamic_entry(
    entry: &ChildEntry,
    parent_type_path: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    lets: &HashMap<String, (syn::Ident, String)>,
    environment_scope: Option<&syn::Ident>,
) -> (syn::Ident, String) {
    match entry {
        ChildEntry::Literal(_) | ChildEntry::Ref(_) => {
            unreachable!("plan_dynamic_entry is only called for If/Match/For entries")
        }
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // Fixed before either branch is planned: lazy planning never grows `out` (a lazy
            // branch's own leaves live in its own local plan instead), so this stays a stable,
            // collision-free seed for both branches' `lazy_branch_plan` calls regardless of which
            // one (if either) ends up falling back to eager and pushing into `out` itself.
            let marker_index = out.len();
            let then_lazy_prefix = format!("__lazyif{marker_index}_then");
            let (then_bindings, then_lazy) = match lazy_branch_plan(
                then_branch,
                parent_type_path,
                ctx,
                from,
                table,
                lets,
                &then_lazy_prefix,
                environment_scope,
            ) {
                Some((bindings, leaves)) => (bindings, Some(leaves)),
                None => (
                    then_branch
                        .iter()
                        .map(|e| {
                            plan_child_entry(
                                e,
                                parent_type_path,
                                ctx,
                                from,
                                table,
                                out,
                                lets,
                                environment_scope,
                            )
                        })
                        .collect(),
                    None,
                ),
            };
            let else_lazy_prefix = format!("__lazyif{marker_index}_else");
            let (else_bindings, else_lazy) = match lazy_branch_plan(
                else_branch,
                parent_type_path,
                ctx,
                from,
                table,
                lets,
                &else_lazy_prefix,
                environment_scope,
            ) {
                Some((bindings, leaves)) => (bindings, Some(leaves)),
                None => (
                    else_branch
                        .iter()
                        .map(|e| {
                            plan_child_entry(
                                e,
                                parent_type_path,
                                ctx,
                                from,
                                table,
                                out,
                                lets,
                                environment_scope,
                            )
                        })
                        .collect(),
                    None,
                ),
            };
            let binding = format_ident!("__node_{}", out.len());
            out.push(PlannedNode {
                binding: binding.clone(),
                type_path: DYNAMIC_CHILD_SLOT_MARKER.to_string(),
                attributes: Vec::new(),
                attached: Vec::new(),
                attribute_shortcuts: HashMap::new(),
                child_bindings: Vec::new(),
                element_attr_bindings: HashMap::new(),
                stored: true,
                id: None,
                dynamic: Some(DynamicPlan::If {
                    condition: condition.clone(),
                    then_bindings,
                    else_bindings,
                    then_lazy,
                    else_lazy,
                }),
                environment_scope: None,
            });
            (binding, DYNAMIC_CHILD_SLOT_MARKER.to_string())
        }
        ChildEntry::Match { value, arms } => {
            // See the `If` arm's matching comment: fixed before any arm is planned, stable across
            // however many of them do or don't end up lazy.
            let marker_index = out.len();
            let arms = arms
                .iter()
                .enumerate()
                .map(|(arm_index, arm)| {
                    let pattern =
                        syn::parse::Parser::parse_str(syn::Pat::parse_single, &arm.pattern)
                            .unwrap_or_else(|error| {
                                panic!("invalid match pattern `{}`: {error}", arm.pattern)
                            });
                    let lazy_prefix = format!("__lazymatch{marker_index}_arm{arm_index}");
                    let (children, lazy) = match lazy_branch_plan(
                        &arm.body,
                        parent_type_path,
                        ctx,
                        from,
                        table,
                        lets,
                        &lazy_prefix,
                        environment_scope,
                    ) {
                        Some((bindings, leaves)) => (bindings, Some(leaves)),
                        None => (
                            arm.body
                                .iter()
                                .map(|e| {
                                    plan_child_entry(
                                        e,
                                        parent_type_path,
                                        ctx,
                                        from,
                                        table,
                                        out,
                                        lets,
                                        environment_scope,
                                    )
                                })
                                .collect(),
                            None,
                        ),
                    };
                    (pattern, children, lazy)
                })
                .collect();
            let binding = format_ident!("__node_{}", out.len());
            out.push(PlannedNode {
                binding: binding.clone(),
                type_path: DYNAMIC_CHILD_SLOT_MARKER.to_string(),
                attributes: Vec::new(),
                attached: Vec::new(),
                attribute_shortcuts: HashMap::new(),
                child_bindings: Vec::new(),
                element_attr_bindings: HashMap::new(),
                stored: true,
                id: None,
                dynamic: Some(DynamicPlan::Match {
                    value: value.clone(),
                    arms,
                }),
                environment_scope: None,
            });
            (binding, DYNAMIC_CHILD_SLOT_MARKER.to_string())
        }
        ChildEntry::For {
            binding,
            collection,
            body,
        } => {
            if body.is_empty()
                || !body
                    .iter()
                    .all(|entry| matches!(entry, ChildEntry::Literal(_)))
            {
                panic!("a `for` body currently requires one or more literal element templates");
            }
            let parent = PlannedNode {
                binding: format_ident!("__for_parent"),
                type_path: parent_type_path.to_string(),
                attributes: Vec::new(),
                attached: Vec::new(),
                attribute_shortcuts: HashMap::new(),
                child_bindings: Vec::new(),
                element_attr_bindings: HashMap::new(),
                stored: false,
                id: None,
                dynamic: None,
                environment_scope: None,
            };
            let item_trait = dynamic_collection_item_trait_ty(&parent, from, table);
            let rc_identity =
                collection_uses_rc_identity(collection, body, binding, ctx, from, table);
            let renderer =
                emit_for_renderer(binding, body, ctx, from, table, &item_trait, rc_identity);
            let node_binding = format_ident!("__node_{}", out.len());
            out.push(PlannedNode {
                binding: node_binding.clone(),
                type_path: DYNAMIC_CHILD_SLOT_MARKER.to_string(),
                attributes: Vec::new(),
                attached: Vec::new(),
                attribute_shortcuts: HashMap::new(),
                child_bindings: Vec::new(),
                element_attr_bindings: HashMap::new(),
                stored: true,
                id: None,
                dynamic: Some(DynamicPlan::For {
                    collection: collection.clone(),
                    renderer,
                    rc_identity,
                }),
                environment_scope: None,
            });
            (node_binding, DYNAMIC_CHILD_SLOT_MARKER.to_string())
        }
    }
}

fn find_attr<'a>(node: &'a PlannedNode, name: &str) -> Option<&'a ViewExpr> {
    node.attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| &attribute.value)
}

/// Emits `binding.as_ui_element().set_attached::<T>(owner, field, value)` for every `Owner::field: value`
/// attached-property setter on `node` (§3) — completely owner/field-name-agnostic on this side,
/// Adding a future attached-property owner besides
/// `Grid` needs no change here at all, only a new `#[attached]` declaration on that owner and a
/// reader on it analogous to `elwindui_core::ui::grid_cell_of`.
///
/// `T` is picked via an explicit turbofish from `owner`'s own declared field type
/// (`TypeInfo::attached_field_types`), never inferred from `value` alone — `UIElementImpl::
/// set_attached`'s own doc comment explains why an inferred mismatch here would silently corrupt
/// the read side (`get_attached`'s `downcast_ref` would just miss and fall back to its caller's
/// default). `owner`/`field` are validated to refer to a real `#[attached]` field already (§14,
/// `validate.rs`), so the `unwrap_or_else` panics here are unreachable in practice, not user-facing
/// error paths.
///
/// Scope note: only ever called from `emit_virtual_construction`, `emit_construction`'s
/// `is_native_control_leaf` branch, and (for non-native-rooted `has_view` components) its plain-
/// component branch — see those call sites' own doc comments for exactly which child kinds this
/// reaches. Verified end-to-end by launching the notepad example with a temporary `Grid` in its
/// status bar (Fixed/Star/Fixed columns rendered with correct proportional widths).
fn emit_attached_setters(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    mode: &EmitMode,
    binding: &TokenStream,
) -> TokenStream {
    let mut out = TokenStream::new();
    for (owner, field, value) in &node.attached {
        let value_ts = emit_expr(value, ctx, mode);
        match table
            .resolve(from, owner)
            .and_then(|info| info.attached_field_types.get(field))
        {
            Some(ty_str) => {
                let ty: syn::Type = syn::parse_str(ty_str)
                    .unwrap_or_else(|e| panic!("invalid attached field type `{ty_str}`: {e}"));
                out.extend(quote! {
                    #binding.as_ui_element().set_attached::<#ty>(#owner, #field, #value_ts);
                });
            }
            // `owner` has no local `TypeInfo` (a real builtin — `Grid`, same as every other
            // builtin since the builtin shape source was deleted, §see `emit_external_construction`'s
            // own doc comment): its `#[attached]` field's declared type can't be looked up here
            // any more, only inside `owner`'s own `#[elwindui_macros::class]`-generated
            // `__elwindui_props_{owner}!` macro (`elwindui-macros::class::build_props_macro`'s
            // `@attached_set` arm, which picks the turbofish `T` from the very same `#[prop]`
            // declaration this used to read out of `TypeInfo`) — so hand the whole call to that
            // macro instead of resolving `T` here. `field` is spliced as a bare identifier
            // (matching `@attached_set`'s own `#name` arm pattern), not a string, since the arm
            // is one-per-declared-field rather than taking `field` as data.
            None => {
                let props_macro = dsl_props_macro_path(owner, None);
                let field_ident = format_ident!("{field}");
                out.extend(quote! {
                    #props_macro!(@attached_set #field_ident, #binding, #value_ts);
                });
            }
        }
    }
    out
}

/// `Option<Foo>` -> `("Foo", true)`; anything else -> `(ty, false)` unchanged.
pub(crate) fn strip_option(ty: &str) -> (&str, bool) {
    let trimmed = ty.trim();
    match trimmed
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        Some(inner) => (inner.trim(), true),
        None => (trimmed, false),
    }
}

/// Returns whether a generated type spelling denotes an erased UI-element trait object.
///
/// Type strings come from both local metadata and synthesized cross-crate declarations.  The
/// latter may include a fully-qualified path between `dyn` and `UIElementExt`, so checking only
/// for the contiguous `dyn UIElement` spelling is insufficient.
fn is_ui_element_type(ty: &str) -> bool {
    ty.contains("dyn UIElement") || ty.contains("UIElementExt")
}

fn is_brush_type(ty: &str) -> bool {
    ty.trim() == "elwindui::core::graphics::Brush"
}

/// Parses a `"#rrggbb"`/`"#rrggbbaa"` hex string into its four byte components — the same rule
/// `elwindui_core::graphics::Color::parse_hex` implements at runtime, duplicated here (rather than
/// depending on `elwindui-core` from this crate just for this) since it's a tiny, stable parsing
/// rule and `elwindui-codegen` otherwise has zero runtime dependency on the crate whose code it
/// generates calls into.
fn parse_hex_color_literal(s: &str) -> Result<(u8, u8, u8, u8), String> {
    let s = s.trim_start_matches('#');
    let byte = |slice: &str| {
        u8::from_str_radix(slice, 16).map_err(|_| format!("invalid hex color literal `#{s}`"))
    };
    match s.len() {
        6 => Ok((byte(&s[0..2])?, byte(&s[2..4])?, byte(&s[4..6])?, 0xff)),
        8 => Ok((
            byte(&s[0..2])?,
            byte(&s[2..4])?,
            byte(&s[4..6])?,
            byte(&s[6..8])?,
        )),
        _ => Err(format!(
            "invalid hex color literal `#{s}`: expected 6 or 8 hex digits"
        )),
    }
}

/// A string literal assigned to a `Brush`/`Color`(-in-`Option<..>`)-typed attribute (e.g.
/// `Rectangle { fill: "#3a3a3c" }`, `TextBlock { color: "#ffffff" }`) is validated and converted
/// to `Brush::Solid(Color::rgba(..))`/`Color::rgba(..)` **at codegen time** rather than spliced
/// through as a raw string — the generated code never calls a fallible/panicking hex parser at
/// runtime, and a malformed literal becomes a codegen-time error (this function `panic!`s, which
/// surfaces as a proc-macro/build-script failure — a compile error in every practical sense) since
/// the literal's well-formedness is fully knowable at compile time. Returns `None` (leaving the
/// caller to fall through to its normal expression-emission path) for anything that isn't a bare
/// string literal against one of these two target types — a dynamic (non-literal) `Brush`/`Color`-
/// typed expression is out of scope for this coercion; the caller is expected to already produce a
/// correctly-typed `Brush`/`Color` value itself.
fn coerce_color_literal(inner_ty: &str, value: &ViewExpr) -> Option<TokenStream> {
    let ViewExpr::Expr(expr) = value else {
        return None;
    };
    // Unwrap any `Group`/`Paren` nesting a proc-macro token stream (the `#[elwindui::component]` +
    // `view! { .. }` frontend, `component_frontend.rs`) can introduce around a literal that a
    // freshly `syn::parse_str`-parsed DSL text expression never has — the underlying literal
    // is the same either way, so this coercion should recognize both uniformly.
    let mut expr = expr;
    while let syn::Expr::Group(group) = expr {
        expr = &group.expr;
    }
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(lit_str),
        ..
    }) = expr
    else {
        return None;
    };
    let is_brush = inner_ty.trim() == "elwindui::core::graphics::Brush";
    let is_color = inner_ty.trim() == "elwindui::core::graphics::Color";
    if !is_brush && !is_color {
        return None;
    }
    let hex = lit_str.value();
    let (r, g, b, a) = parse_hex_color_literal(&hex).unwrap_or_else(|e| panic!("{e}"));
    Some(if is_brush {
        quote! { elwindui::core::graphics::Brush::Solid(elwindui::core::graphics::Color::rgba(#r, #g, #b, #a)) }
    } else {
        quote! { elwindui::core::graphics::Color::rgba(#r, #g, #b, #a) }
    })
}

/// Converts a constructed child binding into `AnyView` when the resolved shape actually wants one
/// (its declared type mentions `AnyView` — `VerticalLayout`/`HorizontalLayout`'s
/// `children: Vec<AnyView>`); some containers want a *concrete* child type instead (`MenuBar`'s
/// `children: Vec<MenuBarItem>`, `MenuBarItem`'s `submenu: Menu`), in which case the binding is
/// used as-is. `.into_any_view()` (not a `From`/`Into` impl) because `Rc<Target>` can't get one —
/// see `generate_view`'s `root_embed_method` doc comment for why (Rust orphan rules).
fn into_any_view_if_needed(base: TokenStream, ty: &str) -> TokenStream {
    if ty.contains("AnyView") {
        quote! { #base.clone().into_any_view() }
    } else {
        quote! { #base.clone() }
    }
}

// Virtual builtins have no backend constructor and are built through `build_virtual_value`.
// Components with a `view`, including `ContentControl`, use normal component generation.

/// Sentinel `source_type_path` passed to `into_node_if_needed` for a value that is *already* an
/// `Rc<dyn UIElement>` with no associated component type name to resolve (a `#[param]` field of
/// that type, forwarded as a bare child in the component's own `view` — e.g. `ContentControl`'s
/// `content` forwarded into `Control { content }`). `into_node_if_needed` treats it as an
/// unconditional pass-through instead of trying (and failing) to resolve it via `SymbolTable`.
const PASSTHROUGH_NODE: &str = "__passthrough_node__";

/// Converts a constructed child binding into `Rc<dyn elwindui::core::ui::UIElementExt>` for a slot
/// that wants one (`Window`'s `content`, a callback content return, or a virtual
/// builtin's own `children: Vec<Rc<dyn UIElement>>` — anywhere the declared type mentions `dyn
/// UIElement`, checked by the caller before calling this). Four cases, by `source_type_path`'s
/// resolved `is_native`/`is_native_control_leaf`:
/// - A virtual builtin (`is_virtual_builtin`, always `!is_native`): `base` is a concrete
///   `Rc<XxxImpl>` local value (built by `emit_virtual_construction`, kept unerased so a `stored`
///   node's struct field and `emit_resync`'s `set_*` calls both see the real type) — upcast to
///   `Rc<dyn UIElement>` the same way the native-control-leaf case below is, via unsized coercion.
/// - A user-defined component whose own `view` root is virtual (`!is_native`, e.g. `DocumentView`,
///   whose root is `VerticalLayout`): its generated `into_node(self: Rc<Self>)` (see
///   `generate_view`) produces the `Rc<dyn UIElement>` value — same `.clone()` convention as
///   `into_any_view_if_needed` so the original binding stays valid for any later reference.
/// - `Button`/`TextArea`/`TabView` (`TypeInfo::is_native_control_leaf`): already implements
///   `UIElement` directly — its own `base` (a backend-owned `NativeControlImpl`, composed via
///   `inherits = NativeControl` — see `elwindui_core::ui::NativeControl`'s own doc comment) was
///   already built at construction time from this exact use site's margin/alignment/
///   `routed_handlers` (see `emit_construction`'s `build_ui_element_base` argument) — so this is a
///   plain upcast, no fresh wrapper needed.
/// - Other native values (`MenuBar`, `Menu`, or `Window`) are unsupported in UI-element slots.
/// For a bare single-segment `ViewExpr::Path` (`content: canvas`), the referenced field's own
/// declared type — reduced to a plausible symbol-table lookup key by stripping one layer of
/// smart-pointer/`Option` wrapper and any module-path prefix (`std :: rc :: Rc < GraphicsDemoCanvas
/// >` -> `GraphicsDemoCanvas`; `ctx.own_fields` stores types as `quote!`-stringified text, hence the
/// stray spaces). Returns `None` for anything else (a multi-segment path, `vm.field`, or a name
/// `ctx.own_fields` doesn't have) — `into_node_if_needed`'s caller already treats an empty/
/// unresolvable string as "not a known symbol-table type", so this doesn't need to distinguish
/// those cases itself.
fn bare_own_field_type(expr: &ViewExpr, ctx: &ViewCtx) -> Option<String> {
    let ViewExpr::Path(path) = expr else {
        return None;
    };
    let [name] = path.as_slice() else {
        return None;
    };
    let ty = ctx.own_fields.get(name)?;
    let inner = match ty.find('<') {
        Some(open) if ty.trim_end().ends_with('>') => {
            let close = ty.trim_end().len() - 1;
            &ty[open + 1..close]
        }
        _ => ty.as_str(),
    };
    Some(
        inner
            .rsplit("::")
            .next()
            .unwrap_or(inner)
            .trim()
            .to_string(),
    )
}

pub(crate) fn into_node_if_needed(
    base: TokenStream,
    source_type_path: &str,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    if source_type_path == PASSTHROUGH_NODE {
        // `.clone()` (an `Rc` refcount bump), not a bare move — the same param is also stored
        // verbatim on `Self` (`generate_view`'s `Self { #(#param_names,)* .. }`), so the original
        // binding must stay valid for that later use.
        return quote! { #base.clone() };
    }
    let info = table.resolve(from, source_type_path);
    let is_native = info.is_some_and(|i| i.is_native);
    let is_native_control_leaf = info.is_some_and(|i| i.is_native_control_leaf);
    if info.is_none() {
        // External (no local `TypeInfo`): assume it's UIElement-implementing, the same shape
        // `is_native_control_leaf`/`is_virtual_builtin` both already get below — every *actual*
        // builtin either is one of those or doesn't implement `UIElementExt` at all (`Window`/
        // `MenuBar`/...), and this function has no ancestor/classification info left to tell them
        // apart. Genuine misuse (nesting a non-UIElement builtin in a `dyn UIElement` slot) fails to
        // compile on the coercion below with a real type error instead of this function's own
        // `panic!` — the same "defer validation to rustc" tradeoff the rest of this migration makes.
        quote! {
            {
                let __node: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #base.clone();
                __node
            }
        }
    } else if is_native_control_leaf {
        quote! {
            {
                let __node: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #base.clone();
                __node
            }
        }
    } else if is_native {
        // Native values that do not implement `UIElement` cannot occupy UI-element slots.
        panic!(
            "`{source_type_path}`: native-but-not-NativeControl-leaf child (e.g. `MenuBar`/`Window`) in a `dyn \
             UIElement` slot isn't supported yet — this codegen path has no real implementation"
        )
    } else if info.is_some_and(|i| i.is_virtual_builtin) {
        quote! {
            {
                let __node: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #base.clone();
                __node
            }
        }
    } else {
        quote! { #base.clone().into_node() }
    }
}

/// `|param| <body>` -> `Box::new(move |param| { <body> })` — a real, ordinary Rust closure value,
/// usable as any `Box<dyn Fn(..) -> ..>`-typed constructor argument (`TabView`'s `key`/
/// `render_label`/`render_content`, or any future widget with a per-item callback param). Always
/// exactly one parameter for this value-computation category of callback (unlike `on_*` event
/// attributes, generalized separately in `emit_wiring`); the parameter needs no type annotation —
/// it's inferred from the constructor parameter's declared `Box<dyn Fn(&Rc<T>) -> R>` type at the
/// call site.
fn emit_closure_value(
    params: &[String],
    body: &ClosureBody,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let [param] = params else {
        panic!(
            "expected exactly one closure parameter here (e.g. `key: |item| ...`), got {}",
            params.len()
        );
    };
    let param_ident = format_ident!("{}", param);
    let closure_ctx = ctx.with_closure_param(param);
    let body_expr = match body {
        ClosureBody::Expr(expr) => emit_expr(expr, &closure_ctx, &EmitMode::Construction),
        ClosureBody::Block(_) => panic!(
            "a block-bodied closure (`{{ .. }}`) isn't supported for this value-computation \
             callback — use a single expression, e.g. `|item| item.file_name`"
        ),
        ClosureBody::Element(elem) => {
            let mut plan = Vec::new();
            // No outer `let`-bound names are visible inside a template closure body — it runs in a
            // separate per-item instantiation context, not the enclosing view's own construction.
            plan_element(
                elem,
                &closure_ctx,
                from,
                table,
                &mut plan,
                true,
                &HashMap::new(),
            );
            let mut construct = TokenStream::new();
            for planned in &plan {
                emit_construction(planned, &closure_ctx, from, table, &mut construct, &plan);
            }
            let root = plan.last().expect("closure element body must have a root");
            // A closure content field's declared return type is `Rc<dyn UIElement>`, not a bare
            // `AnyView` — so a body rooted in a virtual
            // builtin/component (a `VerticalLayout`, or a `DocumentView`-style user component)
            // works exactly like any other embedding slot, via the same `is_native` dispatch
            // `into_node_if_needed` uses elsewhere.
            let root_binding = &root.binding;
            let converted =
                into_node_if_needed(quote! { #root_binding }, &root.type_path, from, table);
            quote! { { #construct #converted } }
        }
    };
    // `: &_` (not left fully unannotated) — a generic function call with several closure
    // arguments that all share the same inferred type parameter (`TabView::new`'s `key`/
    // `render_label`/`render_content`, all `Fn(&Rc<T>) -> _`) doesn't always let rustc pin down
    // an entirely-unannotated closure parameter's type from the surrounding call alone; stating
    // "a reference to something" is enough of a hint for the rest to unify correctly.
    quote! { Box::new(move |#param_ident: &_| { #body_expr }) }
}

/// Issue #162 §3.8/§4.7: emits the `ViewTemplate::new(move |ctx| { .. })` factory for a lowered
/// `ViewExpr::DeferredView` (`context_popup: view! { .. }`) attribute value. Called from every
/// deferred-`Option<T>`-field value-computation site (`build_virtual_value`/
/// `build_component_setters`/`build_component_optional_setters`) in place of the ordinary
/// `emit_expr` call those sites otherwise use for a ViewTemplate-typed field's value — mirrors
/// those functions' own `ViewExpr::Element`/`ViewExpr::Closure` special-casing, since a deferred
/// view is likewise "not a plain reactive value expression".
///
/// This function's own value-computation call sites run in different generated scopes depending on
/// this component's own shape (a shape/host-composed component's non-root children build inside
/// `__build_view(&self)`, everything else inside `__build_view(self: &Rc<Self>)` — `generate_view`'s
/// own two `__build_view` shapes) and there is no single already-in-scope `self`/`__self_weak`
/// binding whose *type* is uniformly `Weak<Self>` across both. So this reuses `__build_view`'s own
/// existing "most-derived self" recovery idiom instead of assuming either shape directly: read the
/// type-erased `self.__self_weak` field every component carries (populated once, during
/// `Rc::new_cyclic`, regardless of composition style), upgrade, `downcast::<#target>()`.
///
/// PR #165 review remediation, A3: the downcast target is `deferred.lexical_owner` — the real
/// source Component this `DeferredView` was written inside — **not** `ctx.target` (`generate_view`'s
/// own target identifier, the concrete type *currently being generated*). For a top-level deferred
/// view these are the same identifier, since `ctx.target` at that point *is* the source Component.
/// But for a `DeferredView` nested inside another `DeferredView`'s own body, this factory expression
/// is emitted while generating the *outer* hidden Component's own code — `ctx.target` there is that
/// outer hidden Component, not the true source Component `lexical_owner` still correctly names (see
/// `lib.rs`'s `lower_deferred_views_in_expr` and `DeferredViewExpr::lexical_owner`'s own doc
/// comment). Using `ctx.target` there would build a `Weak<OuterHiddenComponent>` and hand it to a
/// hidden-Component constructor whose own field expects `Weak<SourceComponent>` — a straight type
/// mismatch, not merely a semantic one.
///
/// The factory is an `Fn`, not `FnOnce` (`ViewTemplate::new`'s own bound) — cloning the captured
/// weak owner on every call (Issue #162 §3.8) is required, not merely an optimization choice: the
/// same `ViewTemplate` value is built once and may be `.build()` many times (once per popup-open).
fn emit_deferred_view_value(
    deferred: &DeferredViewExpr,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    // An expression-form `template_view!` has no surrounding component item from which the
    // deferred-view preprocessing pass can synthesize a hidden component.  Lower its nested body
    // directly through the same shared template compiler instead; component-owned deferred views
    // still take the preprocessed hidden-component path below.
    if ctx.is_template_storage() && deferred.hidden_component.is_none() {
        let target = ctx
            .template_target
            .clone()
            .expect("template storage must carry a typed target");
        let parent = ctx
            .template_parent
            .clone()
            .expect("template storage must carry a typed parent");
        let compiled = crate::compile_template_body(
            &deferred.body.root,
            &deferred.body.lets,
            deferred.body.on_mount.as_ref(),
            deferred.body.on_unmount.as_ref(),
            deferred.body.on_update.as_ref(),
            from.clone(),
            table.clone(),
            target.clone(),
            ctx.template_bare_parent_fields.clone(),
        )
        .unwrap_or_else(|error| panic!("invalid nested template_view body: {error}"));
        return crate::emit_view_template_factory(&compiled, target, &parent);
    }
    let hidden_name = deferred.hidden_component.as_deref().unwrap_or_else(|| {
        panic!(
            "a `ViewExpr::DeferredView` reached codegen without being lowered first (Issue #162 \
             Step 6) — this is an elwindui-codegen bug, not a user error"
        )
    });
    let hidden_ident = format_ident!("{}", hidden_name);
    let lexical_owner_name = deferred.lexical_owner.as_deref().unwrap_or_else(|| {
        panic!(
            "a `ViewExpr::DeferredView` reached codegen without `lexical_owner` set by lowering \
             (Issue #162 Step 6 / PR #165 A3) — this is an elwindui-codegen bug, not a user error"
        )
    });
    let target = format_ident!("{}", lexical_owner_name);
    // PR #165 review remediation, A3 (second half): when this factory expression is itself being
    // emitted *inside an already-lowered hidden Component* (`ctx.implicit_owner.is_some()` — a
    // `DeferredView` nested inside another `DeferredView`'s own body), `self` here is that outer
    // hidden Component, not the true lexical owner — `self.__self_weak` downcast to `#target`
    // (the original source Component) would never succeed, since `self` really is an instance of
    // the *outer* hidden Component's own type, not `#target`. But the outer hidden Component
    // already carries exactly the right value in its own implicit-owner field (`self.__view_owner:
    // Weak<#target>`, by the same A3 lowering guarantee that keeps every level's `lexical_owner`
    // equal to the same original source Component) — reuse that directly instead of re-deriving it
    // through a downcast that can only work at the top level.
    let owner_capture = match &ctx.implicit_owner {
        Some(owner) => {
            let owner_field = format_ident!("{}", owner.field_name);
            quote! { let __view_owner_weak: std::rc::Weak<#target> = self.#owner_field.clone(); }
        }
        None => quote! {
            let __view_owner_weak: std::rc::Weak<#target> = self
                .__self_weak
                .borrow()
                .upgrade()
                .and_then(|__rc| __rc.downcast::<#target>().ok())
                .map(|__rc| std::rc::Rc::downgrade(&__rc))
                .unwrap_or_else(std::rc::Weak::new);
        },
    };
    quote! {
        {
            #owner_capture
            elwindui::core::ui::ViewTemplate::new(move |ctx| {
                // `ViewTemplate::build` has already checked `ctx.owner`'s liveness before ever
                // invoking this factory (docs/design/runtime/view_template_design.md §2) — this
                // upgrade is for the *lexical* enclosing Component (`__view_owner_weak`), a
                // distinct liveness check (Issue #162 §3.7).
                __view_owner_weak.upgrade()?;
                let __instance = #hidden_ident::__new_unmounted(__view_owner_weak.clone());
                __instance.mount(ctx.environment);
                Some(__instance.into_node())
            })
        }
    }
}

/// Whether `info` names a hand-written native type with no generated Rust of its own
/// (`is_native && !has_view` — `Button`/`TextArea`/`TabView`/`TabViewItem` via `inherits
/// NativeControl`, and `Window`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem` via `#[native]`
/// directly). These are the only components whose own `Type::new(..)` is hand-written Rust rather
/// than `generate_view`-produced — `emit_construction` uses this to decide between the
/// zero-argument-constructor-plus-setters convention (`build_component_setters`, docs/
/// docs/design/runtime/ui_tree_design.md's post-construction setter convention extended to every builtin
/// property) and the ordinary positional-argument `Type::new(args)` every `has_view` component
/// (embedded/composed like `ContentControl`, or a plain user-defined component) still uses —
/// unchanged, since `generate_view`'s own construction isn't part of this pass (see this crate's
/// own follow-up plan notes on the deferred, much larger user-component field-storage rewrite).
fn is_hand_written_native(info: &TypeInfo) -> bool {
    info.is_native && !info.has_view
}

/// A hand-written native's own DSL-attribute-driven setters (`build_component_setters`), or a
/// virtual builtin's own `set_*` calls (`build_virtual_value`/`emit_resync`), may call one of
/// `elwindui::core::ui`'s shared property-setter traits' methods via dot-syntax — declared there
/// (docs/design/runtime/ui_tree_design.md) rather than as a wrapper-only inherent method, so the trait
/// needs to be in scope wherever that dot-call happens. Emitted as an anonymous `use ... as _;`
/// (never binds a name of its own, so repeating it for multiple bindings of the same type in one
/// function is harmless) right alongside `#binding`'s own `let` in `emit_construction`, which keeps
/// it in scope for `emit_wiring`'s later calls on the same binding too (both live in the same
/// enclosing function body) — and again verbatim in `emit_resync`'s own separate function scope
/// (`emit_resync`'s own doc comment), since a virtual builtin's `set_*` calls there need the same
/// trait but `build_virtual_value`'s own inline `use` (construction time only) doesn't reach that
/// far. `Button`/`TextArea`/`MenuItem`/`MenuBarItem`/`Window` (hand-written natives) and every
/// virtual builtin (`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape`) route
/// their own DSL properties through a shared trait method this way — `TabView`/`TabViewItem`'s own
/// properties, and `Menu`/`MenuBar`'s `children`, are all wrapper-only inherent methods (no shared
/// trait involved), so nothing needs importing for those.
/// Tags every top-level `fn` in `tokens` (a concatenation of already-fully-formed method item token
/// streams — `root_embed_method`/`named_accessors`/`methods`/`shadow_hooks`/`on_unmount_method`, plus
/// `resync` itself) with `#[inherent]`, so `#[class]` (this function's own caller,
/// `generate_view`'s composed-target branch) routes them into its own single `#[elwindui::class] impl
/// #target { .. }` block as plain inherent methods, rather than needing a second, separate, plain
/// `impl #target { .. }` block purely to hold them (none of them are part of `#target`'s own generated
/// trait). Parses `tokens` by wrapping it in a throwaway `impl` block — mechanical, not semantic:
/// every method here was already valid to splice directly into an `impl` block as-is.
fn mark_inherent(tokens: TokenStream) -> TokenStream {
    if tokens.is_empty() {
        return tokens;
    }
    let wrapped: syn::ItemImpl = syn::parse_quote! { impl __MarkInherentTarget { #tokens } };
    let items: Vec<TokenStream> = wrapped
        .items
        .into_iter()
        .map(|item| match item {
            syn::ImplItem::Fn(mut f) => {
                f.attrs.insert(0, syn::parse_quote!(#[inherent]));
                quote! { #f }
            }
            other => quote! { #other },
        })
        .collect();
    quote! { #(#items)* }
}

/// Emits `use elwindui::core::ui::{type_path}Ext as _;` for every hand-written native or virtual
/// builtin — needed so that type's shared-trait setter methods (dot-call syntax) resolve at both
/// the construction site (`emit_construction`/`emit_virtual_construction`) and the separate
/// `resync()`/`emit_wiring` function scopes (`use`s don't cross fn bodies). Every one of these
/// types has a real `{Name}Ext` trait at `elwindui_core::ui::{Name}Ext` — including `TabView`/
/// `TabViewItem`, whose own trait is deliberately empty (see their own doc comments in
/// `elwindui-core`) purely so this holds with no exceptions — so this is a single mechanical
/// `format_ident!("{type_path}Ext")`, gated on `is_native || is_virtual_builtin`, not an
/// enumerated name list. `None`/a plain `has_view` component (e.g. `ContentControl`/`Rectangle`,
/// or any user component) needs no `use` here at all — its own setters are either derived
/// generically by `generate_view` (no shared trait involved) or, for a `has_view` builtin,
/// hand-written directly in `elwindui_core::ui` and called without a trait import.
/// Emits the setter call for `name` on `receiver` (a value of `node_type`'s own concrete type),
/// disambiguating against `E0034 "multiple applicable items in scope"` whenever `name` is actually
/// declared by some *ancestor* of `node_type` rather than `node_type` itself.
///
/// Why this is needed at all: every `#[class]`-managed component's own generated `{Name}Ext` trait
/// re-implements (forwards) *every* ancestor method, including ones it never overrides — so a
/// composed/host-composition component (`CustomCheckBox inherits ContentControl`, `self_is_node`)
/// ends up with both `impl CustomCheckBoxExt for CustomCheckBox` *and* `impl UIElementExt for
/// CustomCheckBox` (and `ControlExt`, `ContentControlExt`, ...) simultaneously providing the exact
/// same default-bodied `set_<name>` for any field `UIElement`/`Control`/... declared — calling
/// `receiver.set_<name>(..)` directly is ambiguous the moment more than one of those traits is in
/// scope, which is exactly the case inside that component's own `#[class]`-processed `impl` block.
/// This is *not* specific to fields `UIElement` itself declares (an earlier, narrower version of
/// this fix only handled those) — the identical ambiguity happens for any ancestor's own field
/// (`Control`'s `padding`, `Layout`'s `children`, a user-defined intermediate component's own
/// fields, ...), so the fix has to be equally general: name the *actual declaring type's* trait
/// explicitly via UFCS (`{Declarer}Ext::set_<name>(&receiver, value)`), which sidesteps method-call
/// ambiguity entirely (no candidate search — the trait is named outright) regardless of which level
/// of the hierarchy actually owns the field. A field `node_type` declares itself needs no
/// disambiguation at all (nothing else provides it), so this only special-cases the inherited case.
fn emit_field_setter_call(
    name: &str,
    node_type: &str,
    setter: &syn::Ident,
    args: TokenStream,
    receiver: &TokenStream,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    // One of the seven `#[text_style]`-injected properties: the real setter always lives on the
    // hand-written `TextStyleOwner` trait (`crates/elwindui-core/src/ui.rs`), never on whichever
    // builtin's own `..Ext` trait `declaring_types` would otherwise point at (`NativeControlExt`/
    // `ControlExt`/`TextBlockExt` don't declare `set_font_size` etc. at all — `#[class]` never
    // generated them, `TextStyleOwner` is a plain hand-written trait orthogonal to that whole
    // mechanism). `receiver`'s own concrete type is *not* guaranteed to implement `TextStyleOwner`
    // directly — a user `component X inherits Control` (or `inherits ContentControl`) generates a
    // struct composing `base: Control` (shape/template composition, `ComponentDef`'s own doc
    // comment) with no `impl TextStyleOwner for X` of its own. `UIElementExt::as_text_style_owner`
    // *is* part of the `#[class]` ancestor-forwarding chain (declared `#[overridable]` on
    // `UIElement`, overridden by `Control`/`TextBlock`/each backend's `NativeControl`), so it
    // already resolves correctly through any such composition without needing a matching
    // `TextStyleOwner` impl on every intermediate generated struct — going through it here (rather
    // than `TextStyleOwner::#setter` directly) is what makes this work for a composed type, not
    // just the three classes that implement `TextStyleOwner` by hand. Always fully path-qualified,
    // so no `use` needs to be threaded through for it (mirrors the ordinary UFCS branch below).
    if table
        .resolve(from, node_type)
        .is_some_and(|info| info.text_style_fields.contains(name))
    {
        return quote! {
            elwindui::core::ui::UIElementExt::as_text_style_owner(&*(#receiver))
                .expect(concat!(
                    "`", #name, "` was declared with #[text_style] but the resolved node has no \
                     TextStyleOwner at runtime — this is an elwindui-codegen bug, not a user error"
                ))
                .#setter(#args);
        };
    }
    let declaring_type = table
        .resolve(from, node_type)
        .and_then(|info| info.declaring_types.get(name));
    match declaring_type {
        // Named via UFCS (`{Ext}::method(&receiver, ..)`, fully path-qualified) rather than
        // `receiver.method(..)` — naming the trait explicitly means there is no candidate *search*
        // for Rust to find ambiguous in the first place, regardless of how many other `..Ext`
        // traits `receiver`'s own concrete type also happens to implement. No `use` needed since
        // the path is already fully qualified here. `&*(#receiver)` (not a bare `&#receiver`):
        // unlike ordinary method-call syntax, UFCS does *not* auto-deref its receiver argument, so
        // this needs to land on exactly `&ConcreteType` itself regardless of whether `receiver` is
        // already `&Self` (`emit_resync`'s `self`, where `&*self` is just a re-borrow) or an owned
        // `Rc<ConcreteType>` (`build_component_setters`/`build_component_optional_setters`'s own
        // `binding`, where `&*binding` derefs through `Rc`'s own `Deref` impl).
        Some(declarer) if declarer != node_type => {
            let declarer_info = table.resolve(from, declarer);
            let ext_ident = format_ident!("{declarer}Ext");
            let ext_path = if declarer_info.is_some_and(|i| i.is_builtin) {
                quote! { elwindui::ui::#ext_ident }
            } else {
                quote! { #ext_ident }
            };
            quote! { #ext_path::#setter(&*(#receiver), #args); }
        }
        _ => quote! { #receiver.#setter(#args); },
    }
}

fn emit_field_clear_call(
    name: &str,
    node_type: &str,
    receiver: &TokenStream,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let clear = format_ident!("clear_{name}");
    if table
        .resolve(from, node_type)
        .is_some_and(|info| info.text_style_fields.contains(name))
    {
        return quote! {
            elwindui::core::ui::UIElementExt::as_text_style_owner(&*(#receiver))
                .expect(concat!(
                    "`", #name, "` was declared with #[text_style] but the resolved node has no \
                     TextStyleOwner at runtime — this is an elwindui-codegen bug, not a user error"
                ))
                .#clear();
        };
    }
    let declaring_type = table
        .resolve(from, node_type)
        .and_then(|info| info.declaring_types.get(name));
    match declaring_type {
        Some(declarer) if declarer != node_type => {
            let declarer_info = table.resolve(from, declarer);
            let ext_ident = format_ident!("{declarer}Ext");
            let ext_path = if declarer_info.is_some_and(|i| i.is_builtin) {
                quote! { elwindui::ui::#ext_ident }
            } else {
                quote! { #ext_ident }
            };
            quote! { #ext_path::#clear(&*(#receiver)); }
        }
        _ => quote! { #receiver.#clear(); },
    }
}

fn is_semantic_brush_property(info: &TypeInfo, name: &str) -> bool {
    info.semantic_brush_fields.contains(name)
}

fn semantic_brush_construction_environment(node: &PlannedNode, ctx: &ViewCtx) -> TokenStream {
    match &node.environment_scope {
        Some(scope) => quote! { &#scope },
        None if ctx.is_template_storage() => {
            let environment = ctx
                .template_environment()
                .expect("template storage must carry an environment binding");
            quote! { &#environment }
        }
        None => quote! {
            self.__mount_environment
                .get()
                .expect("semantic brush resolution: component is not yet mounted")
        },
    }
}

fn semantic_brush_resync_environment(node: &PlannedNode, ctx: &ViewCtx) -> TokenStream {
    match &node.environment_scope {
        Some(scope) if ctx.is_template_storage() => quote! { &#scope },
        Some(scope) => quote! {
            self.#scope
                .get()
                .expect("semantic brush resolution: EnvironmentScope is not yet mounted")
        },
        None if ctx.is_template_storage() => {
            let environment = ctx
                .template_environment()
                .expect("template storage must carry an environment binding");
            quote! { &#environment }
        }
        None => quote! {
            self.__mount_environment
                .get()
                .expect("semantic brush resolution: component is not yet mounted")
        },
    }
}

fn emit_semantic_brush_resolution(
    raw: TokenStream,
    environment: TokenStream,
    set: TokenStream,
    clear: TokenStream,
) -> TokenStream {
    quote! {
        match ::core::convert::Into::<elwindui::core::theme::BrushStyle>::into(#raw)
            .resolve(#environment)
        {
            elwindui::core::theme::ResolvedValue::Value(__elwindui_semantic_brush) => {
                #set
            }
            elwindui::core::theme::ResolvedValue::PlatformDefault => {
                #clear
            }
        }
    }
}

fn builtin_trait_use(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    if info.is_some_and(|i| i.is_native || i.is_virtual_builtin) {
        let ext_ident = format_ident!("{type_path}Ext");
        // Emitted unconditionally for every eligible type now (not gated per-field on whether
        // *this* call site's own attributes happen to need a shared-trait method) — harmless when
        // unused (e.g. a `Menu`/`MenuBar` construction site with no other `MenuExt`/`MenuBarExt`
        // method call besides the `#[inherent]` `items()`/`add_item` this crate itself never
        // routes through the trait), so silence the warning rather than re-deriving per-site
        // whether it's actually exercised.
        quote! { #[allow(unused_imports)] use elwindui::core::ui::#ext_ident as _; }
    } else {
        TokenStream::new()
    }
}

/// The only construction mechanism left: resolve `node.type_path` via `SymbolTable` (every
/// resolved type — a plain user component, a component-with-view, or a builtin shape backed by
/// hand-written Rust in an `elwindui-backend-*` crate — is treated identically) and either:
/// - (`is_hand_written_native`) call `Type::new()` (no arguments) followed by whichever
///   `set_<field>(..)` calls this use site's own attributes supply (`build_component_setters`); or
/// - (everything else — `generate_view`-produced, `has_view == true`) call `Type::new(args)`,
///   `args` built from `info.param_fields` in declaration order (`build_component_args`):
///   - a param named `children` is filled from the element's bare nested children (a `Vec`,
///     `AnyView`-converted per element only if the declared type says `AnyView`);
///   - a `ViewExpr::Element`-valued attribute (`menu_bar: MenuBar { .. }`) is filled from its own
///     already-planned/constructed binding (`element_attr_bindings`);
///   - a `ViewExpr::Closure`-valued attribute compiles to a real boxed closure (`emit_closure_value`);
///   - an `Option<..>`-typed param with no matching attribute becomes `None`;
///   - the param named by the component's own `#[content(field_name)]` (docs/specs/dsl_spec.md 付録A,
///     `TypeInfo::content_field`) with no matching attribute binds the element's single bare nested
///     child (`MenuBarItem`'s single nested `Menu`, bound to its `#[content(submenu)]` field);
///   - anything else is an ordinary `emit_expr` value.
/// Constructs an *external* element: `table.resolve` found nothing for `node.type_path` in this
/// compilation's own `#[component]` AST, so it is treated as a builtin declared entirely
/// elsewhere (`elwindui-core`'s `#[class]`/`#[prop]` items, or a future crate declaring one the same
/// way) — the counterpart to `is_hand_written_native`'s branch below, but knowing none of what
/// `TypeInfo` used to supply. Every DSL-surface decision that used to come from a builtin's
/// `TypeInfo` (which properties exist, their types, whether they're `#[routed]`, how a value should
/// be wrapped before reaching the real setter) is deferred to the `__elwindui_props_{Name}!` macro
/// (`elwindui_macros::class::build_props_macro`) — this function's only job is to compute *values*
/// from the DSL's own syntax (an `emit_expr`/theme-token/closure concern, never a type concern) and
/// hand them to `@set`/`@clear`/`@children`.
///
/// `on_*`-prefixed attributes are skipped entirely here, mirroring `param_fields`'s own exclusion —
/// `emit_wiring` handles every callback attribute (routed or not) on its own, via the same `@set`
/// unification (see `build_props_macro`'s own doc comment on why `@set` accepts a bare callable for
/// a `#[routed]` property too).
///
/// **Known gaps, deliberately not yet handled** (no current builtin construction needs them — see
/// `docs/status/implementation_status.md`'s tracking of this rewrite): a named attribute matching a
/// `#[content(..)]`-designated property (`Window { content: SomeElement { .. } }` — as opposed to a
/// *bare* nested child, which `emit_construction`'s own caller already routes through `@children`
/// separately), a `ViewExpr::Element`-valued named attribute (`menu_bar: MenuBar { .. }`), and a
/// dynamic/`for`-driven single-child content slot. Each panics with a specific message rather than
/// silently emitting something wrong.
fn emit_external_construction(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut TokenStream,
) {
    let binding = &node.binding;
    let type_path = dsl_concrete_type_path(&node.type_path, None);
    let sets = emit_external_attribute_sets(node, ctx, from, table);
    let binding_ts = quote! { #binding };
    // `Grid::row`/`Grid::column`/... — every real builtin now goes through this construction
    // path (no `TypeInfo` left to route a virtual builtin through `emit_virtual_construction`'s
    // own, otherwise-identical `emit_common_ui_element_setters` call instead), so this is the
    // *only* place left that can apply a use site's attached properties to a builtin — omitting
    // it silently drops them, leaving every builtin child at `GridCell::default()` (row 0, column
    // 0) regardless of what the DSL wrote (`TabView { Grid::row: 1, .. }` colliding with row 0's
    // own content instead of occupying its own row).
    let attached = emit_common_ui_element_setters(node, ctx, from, table, &binding_ts);
    out.extend(quote! {
        // Brings every builtin's `{Name}Ext` trait into scope at once — `#sets`'s method calls may
        // resolve against *any* ancestor's trait (`background` lives on `NativeControlExt`, not
        // `ButtonExt`), and without `TypeInfo` this function has no ancestor chain to import
        // specific traits from. Scoped to this block only.
        #[allow(unused_imports)]
        use elwindui::ui::*;
        let #binding = #type_path::new();
        #attached
        #sets
    });
}

/// The attribute-handling core of `emit_external_construction`, factored out so
/// `generate_view`'s host-composition root branch can reuse it too (`Type::construct()` there
/// instead of `Type::new()`, everything else identical — see that branch's own doc comment). Builds
/// every plain-value `@set` call (theme tokens, `#[text_style]`-injected properties, named
/// element-valued slots, ordinary expressions/closures) for `node`'s attributes; `on_*` attributes
/// are skipped (event wiring is `emit_wiring`'s own, separate pass, uniformly for every construction
/// path). Does not itself construct `node` or open the `use elwindui::ui::*;` scope its emitted
/// method calls need — callers own their own construction statement and glob import.
fn emit_external_attribute_sets(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let binding = &node.binding;
    let props_macro = dsl_props_macro_path(&node.type_path, None);
    let mut sets = TokenStream::new();
    for attribute in &node.attributes {
        let name = &attribute.name;
        let expr = &attribute.value;
        if name.starts_with("on_") {
            continue;
        }
        let name_ident = format_ident!("{name}");
        // A named single-child slot (`content: Grid { .. }`, `menu_bar: MenuBar { .. }`) —
        // `plan_element` already planned and will separately construct the nested element as its
        // own `PlannedNode`, recorded here by name (mirrors `build_component_args`'s own
        // `Some(ViewExpr::Element(_))` arm, the known-`TypeInfo` equivalent of this same case).
        // Without this external parent's own `TypeInfo`, this function has no declared field type
        // to convert *to* (`content` wants `Rc<dyn UIElementExt>`, `menu_bar` wants `Rc<dyn
        // MenuBarExt>`, ...) — but it doesn't need one: whether the *nested value itself* needs
        // `.into_node()` at all is a property of what that value resolves to, not of which slot
        // it's headed for. A real builtin (native leaf, or anything genuinely external with no
        // local `TypeInfo`, like `Grid`/`MenuBar` now that builtins live outside this table
        // entirely) always `impl`s whatever trait its own real setter declares directly (`#[class]`
        // gives it one), so plain unsized coercion at the eventual setter call already handles it.
        // Only an ordinary `#[elwindui::component]`-frontend user struct (has local `TypeInfo`,
        // isn't itself native/a virtual builtin) never `impl`s a `dyn` target trait directly —
        // its own generated `into_node()` is the only way to erase it, and skipping that call
        // here used to silently pass the raw concrete value through as an opaque, un-attached
        // value (compiled fine, `@set`'s generated setter call is generic enough to accept it, but
        // never actually reached the visual tree as a `dyn UIElementExt` child). Found via a real
        // notepad screenshot regression (Refs #14) — `cargo test`'s string-level codegen assertions
        // never caught this since the generated code is syntactically valid either way.
        if let ViewExpr::Element(_) = expr {
            let (nested_binding, nested_ty) = node
                .element_attr_bindings
                .get(name.as_str())
                .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
            let info = resolve_context_info(ctx, from, table, nested_ty);
            let needs_into_node = info.is_some_and(|i| !i.is_native && !i.is_virtual_builtin);
            let value = if needs_into_node {
                quote! { #nested_binding.clone().into_node() }
            } else {
                quote! { #nested_binding.clone() }
            };
            sets.extend(quote! {
                #props_macro!(@set #binding, #name_ident, #value);
            });
            continue;
        }
        let value = match expr {
            ViewExpr::Closure { params, body } => {
                emit_closure_value(params, body, ctx, from, table)
            }
            // Issue #162: every real builtin (`TextBlock`, `Window`, ...) goes through this
            // `TypeInfo`-less path in a real (non-`elwindui-codegen`-internal-test) compilation —
            // `context_popup`/any other `ViewTemplate`-typed field on one of them reaches codegen
            // here, not through `build_virtual_value`/`build_component_setters` (those only ever
            // run for a component whose shape *does* have a local `TypeInfo`, e.g. this crate's own
            // `#[cfg(test)]` builtin-module fixtures). No `TypeInfo` here means no `is_option` to
            // check either — `wrap_prop_value` (`elwindui-macros`) only auto-`Some(..)`-wraps a
            // handful of recognized shapes (`String`/`Vec`/`BareFn`/`Brush`/`Color`/`Rc<dyn Trait>`),
            // none of which `ViewTemplate` is, so this wraps unconditionally: every real target a
            // `ViewExpr::DeferredView` validates against (`validate::check_deferred_view_assignment`,
            // §3.13) is `Option<ViewTemplate>` in practice — `context_popup` is still the only
            // production consumer.
            ViewExpr::DeferredView(deferred) => {
                let factory = emit_deferred_view_value(deferred, ctx, from, table);
                // PR #165 review remediation, A4 (round 2): `validate::check_deferred_view_
                // assignment` only catches a mismatched target when the target component has a
                // local `TypeInfo` — a no-op for this real-builtin path (no `TypeInfo` exists
                // here at all). Convert the built factory to the target's own *real* declared
                // type here instead, read through the same cross-crate `@field_type` transport
                // `synthesize_external_base_fields` already uses (Refs #90) — an unknown property
                // name falls through to `@field_type`'s own existing terminal `compile_error!`
                // unchanged (never reaches this call at all), and a known property whose declared
                // type isn't `ViewTemplate`/`Option<ViewTemplate>` fails to compile with
                // `DeferredViewAssignmentTarget`'s own `#[diagnostic::on_unimplemented]` message
                // naming both the field and the required type (`docs/specs/dsl_spec.md` rule 37).
                // Unlike the round-1 version (which asserted the target type but then always
                // wrapped in `Some(..)` regardless), this genuinely *produces* whichever of the
                // two accepted shapes the property declares — correct for a real builtin property
                // declared bare `ViewTemplate`, not only `Option<ViewTemplate>`.
                quote! {
                    elwindui::core::ui::__coerce_deferred_view_assignment_target::<
                        #props_macro!(@field_type #name_ident)
                    >(#factory)
                }
            }
            other => {
                let value = emit_expr(other, ctx, &EmitMode::Construction);
                // A bare-forwarded own field (`content: fills_canvas`, `ViewExpr::Path`) — mirrors
                // `build_component_args`/`build_virtual_value`'s identically-named branch
                // (`bare_own_field_type`'s own doc comment): the referenced field is *also* kept on
                // `Self` (`generate_view`'s own `field_inits`), so this needs `.clone()` (an `Rc`
                // refcount bump, not a bare move) the same way any other reused planned binding
                // does — see this fn's own `element_attr_bindings` branch above. Unlike that known-
                // `TypeInfo` branch, this one can't check whether the *target* field wants `dyn
                // UIElement` specifically (no shape table here at all) — `.clone()` is correct
                // either way, since the target's real setter's own unsized coercion (or an identical
                // concrete type) accepts it regardless.
                if bare_own_field_type(other, ctx).is_some() {
                    quote! { (#value).clone() }
                } else {
                    value
                }
            }
        };
        // Same `synthesize_external_base_fields`-synthesized-field unwrap as `emit_resync`'s own
        // `info.is_none()` branch (see that call site's doc comment) — construction time needs the
        // identical `Option<T>`-vs-bare-`T` normalization for the exact same reason, since this is
        // the *other* place a bare-forwarded own field's value reaches `@set`.
        if bare_own_field_type(expr, ctx).is_some_and(|ty| ty.contains('!')) {
            let environment = semantic_brush_construction_environment(node, ctx);
            sets.extend(quote! {
                if let ::std::option::Option::Some(__v) = ::std::option::Option::from(#value) {
                    #props_macro!(
                        @set_with_environment #binding, #name_ident, __v, #environment
                    );
                }
            });
        } else {
            let environment = semantic_brush_construction_environment(node, ctx);
            sets.extend(quote! {
                #props_macro!(
                    @set_with_environment #binding, #name_ident, #value, #environment
                );
            });
        }
    }
    sets
}

/// Emits an `EnvironmentScope { key: value, .. }` node's own statement (CI-7 of #80, closes #100):
/// `let #binding = <outer>.derive(); #binding.set::<Key1>(v1); #binding.set::<Key2>(v2); ...;`.
/// `<outer>` is either the enclosing `EnvironmentScope`'s own already-`let`-bound local variable
/// (`node.environment_scope: Some(outer_var)`, for a nested scope) or, for a top-level scope,
/// `self.__mount_environment.get().expect(..)` (the enclosing *component's* own effective
/// Environment, established by `mount()` before `__build_view()` — see
/// docs/design/runtime/component_lifecycle_design.md §4a). Each override's key name is checked
/// against the same-crate `#[elwindui::environment_key(name = ..)]` registry
/// (`component_frontend::lookup_same_crate_environment_key`, mirroring `environment_key_type`'s own
/// resolution for `#[environment(name)]` fields) — an unknown name is a `compile_error!` (spec §13
/// rule 35), not a runtime panic; a type-mismatched value surfaces as an ordinary `rustc` type
/// error on the generated `.set::<Key>(value)` call itself, the same way every other DSL value-type
/// mismatch in this codebase does.
///
/// A qualified cross-crate override (`EnvironmentScope { some_crate::name: value }`, Issue #129,
/// `plan_environment_scope`'s own doc comment) arrives as `node.attached` instead of
/// `node.attributes`, resolved via `environment_key_type_by_name(name, Some(prefix))` — a
/// type-position invocation of the declaring crate's exported
/// `__elwindui_environment_key_{name}!` macro. There is no same-crate-style `compile_error!` for an
/// unresolvable qualified name: `rustc` itself reports "cannot find macro" once the generated code
/// is compiled (see `environment_key_type_by_name`'s own doc comment) — a type mismatch still
/// surfaces as an ordinary `rustc` type error on `.set::<Key>(value)`, same as the bare form.
fn emit_environment_scope_construction(node: &PlannedNode, ctx: &ViewCtx, out: &mut TokenStream) {
    let binding = &node.binding;
    let outer = match &node.environment_scope {
        Some(outer_var) => quote! { #outer_var },
        None if ctx.is_template_storage() => {
            let environment = ctx
                .template_environment()
                .expect("template storage must carry an environment binding");
            quote! { #environment }
        }
        None => quote! {
            self.__mount_environment
                .get()
                .expect("EnvironmentScope: component is not yet mounted")
        },
    };
    let mut sets = TokenStream::new();
    for attribute in &node.attributes {
        // Writable resolver, not `lookup_environment_key`: `EnvironmentScope` *writes* a value, and
        // the framework's `popup_dismiss` builtin (readable via `#[environment(popup_dismiss)]`) is
        // installed only by `ContextMenuService::open_custom_popup` — a DSL author must not be able
        // to overwrite the active `PopupDismissAction` this way (`lookup_writable_environment_key`'s
        // own doc comment).
        match crate::component_frontend::lookup_writable_environment_key(&attribute.name) {
            Some((key_type_name, _value_type)) => {
                let key_type: syn::Type = syn::parse_str(&key_type_name)
                    .expect("registered environment key type name must parse");
                let value = emit_expr(&attribute.value, ctx, &EmitMode::Construction);
                // `.into()`, not the bare value: a DSL author writes a bare literal (`"ja-JP"`,
                // parsed as `&str`) the same way any other DSL attribute value does
                // (`build_component_setters`'s own `wrap_prop_value` applies the identical
                // convention for a `String`-typed property/Brush/Color) — `EnvironmentContext::set`
                // needs an owned `K::Value`, and `Into` covers both this coercion and the
                // reflexive already-typed case for free.
                sets.extend(quote! {
                    #binding.set::<#key_type>((#value).into());
                });
            }
            None => {
                let name = &attribute.name;
                let msg = format!(
                    "EnvironmentScope: `{name}` is not declared by any \
                     #[elwindui::environment_key(name = {name}, ..)] earlier in this crate, is not \
                     a writable framework built-in key (docs/specs/theme_environment_spec.md §2 — \
                     `popup_dismiss` is framework-installed and readable only, not settable via \
                     EnvironmentScope), or is not writable at all \
                     (docs/specs/dsl_spec.md §13 rule 35)"
                );
                sets.extend(quote! { compile_error!(#msg); });
            }
        }
    }
    for (owner, field, value_expr) in &node.attached {
        // `alias_seed` includes `owner`, not just `field`: two overrides in the same
        // `EnvironmentScope` block could otherwise reference same-named keys from two different
        // crates (`crate_a::locale`, `crate_b::locale`) and collide on the local type alias
        // `environment_key_type_by_name` generates (see its own doc comment).
        let alias_seed = format!("{owner}_{field}");
        let (key_type_preamble, key_type) =
            environment_key_type_by_name(field, Some(owner), &alias_seed);
        let value = emit_expr(value_expr, ctx, &EmitMode::Construction);
        sets.extend(quote! {
            #key_type_preamble
            #binding.set::<#key_type>((#value).into());
        });
    }
    out.extend(quote! {
        let #binding = #outer.derive();
        #sets
    });
}

fn emit_environment_scope_resync(node: &PlannedNode, ctx: &ViewCtx, out: &mut TokenStream) {
    let binding = &node.binding;
    let environment = if ctx.is_template_storage() {
        quote! { #binding }
    } else {
        quote! {
            self.#binding
                .get()
                .expect("EnvironmentScope resync: scope is not yet mounted")
        }
    };
    let self_mode = if ctx.is_template_storage() {
        EmitMode::WithSelf(quote! { this })
    } else {
        EmitMode::WithSelf(quote! { self })
    };
    for attribute in &node.attributes {
        // Writable resolver — see `emit_environment_scope_construction`'s matching call for why
        // (this is the same `EnvironmentScope` write path, just re-run on resync).
        let Some((key_type_name, _value_type)) =
            crate::component_frontend::lookup_writable_environment_key(&attribute.name)
        else {
            continue;
        };
        let key_type: syn::Type = syn::parse_str(&key_type_name)
            .expect("registered environment key type name must parse");
        let value = emit_expr(&attribute.value, ctx, &self_mode);
        out.extend(quote! {
            #environment.set::<#key_type>((#value).into());
        });
    }
    for (owner, field, value_expr) in &node.attached {
        let alias_seed = format!("{owner}_{field}");
        let (key_type_preamble, key_type) =
            environment_key_type_by_name(field, Some(owner), &alias_seed);
        let value = emit_expr(value_expr, ctx, &self_mode);
        out.extend(quote! {
            #key_type_preamble
            #environment.set::<#key_type>((#value).into());
        });
    }
}

fn emit_construction(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut TokenStream,
    plan: &[PlannedNode],
) {
    if node.type_path == ENVIRONMENT_SCOPE_MARKER {
        emit_environment_scope_construction(node, ctx, out);
        return;
    }
    let resolved_info = resolve_context_info(ctx, from, table, &node.type_path);
    if resolved_info.is_none() {
        emit_external_construction(node, ctx, from, table, out);
        // `DYNAMIC_CHILD_SLOT_MARKER` bindings are never actually constructed (`build_component_setters`'s
        // own identically-filtered `children` loop doc comment: a list-based dynamic region starts
        // genuinely empty at construction, populated for the first time by the initial
        // `__refresh_dynamic_regions()` call `new()` already makes) — embedding one here would
        // reference a binding that was never bound at all.
        let non_dynamic_children: Vec<_> = node
            .child_bindings
            .iter()
            .filter(|(_, child_ty)| child_ty != DYNAMIC_CHILD_SLOT_MARKER)
            .collect();
        if !non_dynamic_children.is_empty() {
            let binding = &node.binding;
            let props_macro = dsl_props_macro_path(
                &node.type_path,
                resolve_context_info(ctx, from, table, &node.type_path),
            );
            let items = non_dynamic_children
                .iter()
                .map(|(c, _)| quote! { #c.clone() });
            out.extend(quote! {
                #props_macro!(@children #binding, [#(#items),*]);
            });
        }
        return;
    }
    if table
        .resolve(from, &node.type_path)
        .is_some_and(|i| i.is_virtual_builtin)
    {
        emit_virtual_construction(node, ctx, from, table, out);
        return;
    }

    let binding = &node.binding;
    let info = resolved_info.unwrap_or_else(|| {
        panic!(
            "unknown or out-of-scope element `{}` — is a `use` for it missing?",
            node.type_path
        )
    });
    let type_ident = concrete_type_ident(&node.type_path, Some(info));

    if is_hand_written_native(info) {
        let setters = build_component_setters(node, ctx, from, table, info, plan);
        let trait_use = builtin_trait_use(&node.type_path, Some(info));
        out.extend(quote! {
            #trait_use
            // See the matching `use` in this function's `else` branch below — a field inherited
            // from `UIElement` itself (`margin`/`width`/`height`/...) needs `UIElementExt` in scope
            // for `setters` (below) to call its shared-trait setter.
            #[allow(unused_imports)]
            use elwindui::core::ui::UIElementExt as _;
            let #binding = #type_ident::new();
            #(#setters)*
        });
    } else {
        // `has_view`/plain-component construction (docs/design/runtime/ui_tree_design.md's
        // post-construction setter convention): `build_component_args` omits this
        // target's own deferred `Option<T>` fields (`is_deferred_field`) from the positional list —
        // `build_component_optional_setters` supplies the matching trailing `.set_<field>(value)`
        // calls for whichever of them this use site actually gives a value.
        let args = build_component_args(node, ctx, from, table, info, plan, false);
        let optional_setters = build_component_optional_setters(node, ctx, from, table, info);
        // CI-7 of #80 (docs/design/runtime/component_lifecycle_design.md §4f): a node declared
        // inside an `EnvironmentScope` constructs via `__new_unmounted` (no automatic self-mount)
        // and is then mounted explicitly, right here, against the scope's own derived
        // `EnvironmentContext` local variable — instead of `new()`'s ordinary path, which would
        // otherwise auto-mount it against `application_environment()` before this statement even
        // gets a chance to override anything.
        let construct_call = if ctx.is_template_storage() {
            let environment = node.environment_scope.as_ref().map_or_else(
                || {
                    ctx.template_environment()
                        .expect("template storage must carry an environment binding")
                },
                Clone::clone,
            );
            quote! {
                let #binding = #type_ident::__new_unmounted(#(#args),*);
                #type_ident::__mount(&#binding, #environment.clone());
            }
        } else {
            match &node.environment_scope {
                Some(scope_var) => quote! {
                    let #binding = #type_ident::__new_unmounted(#(#args),*);
                    #type_ident::__mount(&#binding, #scope_var.clone());
                },
                None => quote! {
                    let #binding = #type_ident::new(#(#args),*);
                },
            }
        };
        // A component that composes over a base with an inherited content destination may not
        // expose that base field in its generated constructor (the base is external to this
        // codegen table). Keep caller bare-child lowering separate from the component's own body
        // lowering by attaching those children after construction through the composed base's
        // exported metadata protocol. An own `#[content(..)]` declaration wins and is handled by
        // the ordinary constructor/setter machinery above.
        let composed_content_attach = if info.content_field.is_none()
            && info.composed_shape.is_some()
            && node
                .child_bindings
                .iter()
                .any(|(_, child_ty)| child_ty != DYNAMIC_CHILD_SLOT_MARKER)
        {
            let base_name = info
                .composed_shape
                .as_deref()
                .expect("composed shape name")
                .to_string();
            let values = node
                .child_bindings
                .iter()
                .filter(|(_, child_ty)| *child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                .map(|(child, _)| quote! { #child.clone() });
            let base_info = resolve_context_info(ctx, from, table, &base_name);
            let macro_path = dsl_props_macro_path(&base_name, base_info);
            quote! {
                #macro_path!(@children #binding, [#(#values),*]);
            }
        } else {
            TokenStream::new()
        };
        // A generated component may declare an initialized `#[content(...)]` property (for
        // example a user `Rc<ListExt<_>>` collection). Such a field is intentionally absent from
        // `param_fields`, so `build_component_args` cannot consume bare children positionally. For
        // a locally resolved generated component, attach through its own typed extension trait so
        // the component's content destination wins over an inherited `Control::visual_root` slot.
        // Unresolved/external hosts retain the exported shape-macro protocol.
        let content_children_attach = if info.content_field.is_some()
            && !info
                .param_fields
                .iter()
                .any(|(name, _)| info.content_field.as_deref() == Some(name.as_str()))
        {
            let children: Vec<_> = node
                .child_bindings
                .iter()
                .filter(|(_, child_ty)| child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                .map(|(child, _)| quote! { #child.clone() })
                .collect();
            if children.is_empty() {
                TokenStream::new()
            } else if !info.is_builtin
                && effective_content_shape(info) != EffectiveContentShape::External
            {
                let field = info
                    .content_field
                    .as_deref()
                    .expect("content shape requires a content field");
                let field_ident = format_ident!("{field}");
                let ext_path = template_dynamic_ext_path(&node.type_path, Some(info));
                let field_ty = info
                    .field_types
                    .get(field)
                    .or_else(|| info.value_field_types.get(field));
                let is_vec = field_ty.is_some_and(|ty| {
                    ty.chars()
                        .filter(|ch| !ch.is_whitespace())
                        .collect::<String>()
                        .starts_with("Vec<")
                });
                match effective_content_shape(info) {
                    EffectiveContentShape::Collection if is_vec => {
                        let setter = format_ident!("set_{field}");
                        quote! {
                            {
                                use #ext_path as _;
                                #binding.#setter(::std::vec![#(#children),*]);
                            }
                        }
                    }
                    EffectiveContentShape::Collection => {
                        quote! {
                            {
                                use #ext_path as _;
                                #( #binding.#field_ident().add(#children); )*
                            }
                        }
                    }
                    EffectiveContentShape::Scalar => {
                        let (child, child_ty) = node
                            .child_bindings
                            .iter()
                            .find(|(_, child_ty)| *child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                            .expect("scalar content must have one static child");
                        let child =
                            into_node_if_needed(quote! { #child.clone() }, child_ty, from, table);
                        let setter = format_ident!("set_{field}");
                        quote! {
                            {
                                use #ext_path as _;
                                #binding.#setter(#child);
                            }
                        }
                    }
                    EffectiveContentShape::External => unreachable!(),
                }
            } else {
                let macro_path = dsl_props_macro_path(&node.type_path, Some(info));
                quote! {
                    #macro_path!(@children #binding, [#(#children),*]);
                }
            }
        } else {
            TokenStream::new()
        };
        out.extend(quote! {
            // A deferred field inherited from `UIElement` itself (`margin`/`width`/`height`/... —
            // `resolve_effective_fields`'s own doc comment) is set through `UIElementExt`, a shared
            // trait method rather than an inherent one — needs this in scope wherever
            // `optional_setters` (below) calls one. Harmless when unused (every other deferred
            // field's own setter is inherent), same as `builtin_trait_use`'s own unconditional
            // `#[allow(unused_imports)]`.
            #[allow(unused_imports)]
            use elwindui::core::ui::UIElementExt as _;
            #construct_call
            #(#optional_setters)*
            #composed_content_attach
            #content_children_attach
        });
        // A non-native component exposes its view root through `into_node()`, allowing attached
        // property setters to target that root. Native non-`NativeControl` roots are unsupported.
        if !info.is_native && !node.attached.is_empty() {
            let erased = format_ident!("{}_erased", binding);
            let erased_ts = quote! { #erased };
            let setters =
                emit_attached_setters(node, ctx, from, table, &EmitMode::Construction, &erased_ts);
            out.extend(quote! {
                let #erased: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #binding.clone().into_node();
                #setters
            });
        }
    }
    // `Button`/`TextArea`/`TabView` (`inherits NativeControl`, `TypeInfo::is_native_control_leaf`)
    // own a real `base` (a backend-owned `NativeControlImpl`) field (docs/design/README.md
    // §5.1) — this use site's margin/attached properties are applied to it right
    // here, post-construction, exactly like `emit_virtual_construction` does for virtual builtins
    // (see `emit_common_ui_element_setters`). `MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`Window`
    // (`#[native]` directly, never entering the `UIElement` tree) don't get this at all.
    if info.is_native_control_leaf {
        let binding_ts = quote! { #binding };
        out.extend(emit_common_ui_element_setters(
            node,
            ctx,
            from,
            table,
            &binding_ts,
        ));
        // `Button`'s own `on_click` is a real `#[routed]` field (`info.routed_fields`), already
        // wired by `emit_wiring`'s dedicated `is_routed` branch — applying the generic mechanism
        // here too would register the same callback twice.
        if !info.routed_fields.contains("on_click") {
            out.extend(emit_generic_on_click_routing(node, ctx, &binding_ts));
        }
    }
}

/// Whether `name` (declared type `ty`) is a *deferred* field on a `has_view`/plain (non-hand-
/// written-native) component — `generate_view`'s own `is_deferred_own_field`/`generate_component`'s
/// matching field split, mirrored here for the calling side so `build_component_args`/
/// `build_component_optional_setters` agree with what that target's own generated `new(..)`
/// actually still accepts positionally. `Option<T>`-typed, and (when the target has a `view`) not
/// referenced anywhere in its own effective view (`view_references_name_anywhere` — not just a
/// *bare* forward like `ContentControl`'s `padding: padding` into `Control { padding: padding }`,
/// but also as a sub-expression identifier, e.g. `Rectangle`'s own
/// `corner_radius.unwrap_or(0.0)`) — either way the value is needed eagerly, before that target's
/// own `Self` exists, so it can't be deferred to a setter. A `None` effective view (a plain
/// component with no `view` at all) has no such construction-time reference to worry about, so
/// `Option`-ness alone decides. Never true for a hand-written native (`is_hand_written_native`) —
/// that family defers *every* field unconditionally via the separate
/// `build_component_setters` path, not this one.
/// `component_name` disambiguates a field this exact type declares itself (`info.declaring_types`)
/// from one it merely inherited — declared-here-directly fields are never deferred even with a
/// `view` and no bare-forward, since (unlike an *inherited* field, which needs a `view`-level
/// forward to prove its value is actually threaded through to construction) there's no base class
/// to forward *from* in the first place: `Rectangle`'s own `corner_radius` (composed over `Shape`,
/// which has no `corner_radius` field of its own to bare-forward) is the motivating case — its
/// real `elwindui_core::ui::Rectangle::construct` signature always takes it positionally and has
/// no `set_corner_radius`, so treating it as deferred would emit a call to a setter that doesn't
/// exist.
fn is_deferred_field(info: &TypeInfo, component_name: &str, name: &str, ty: &str) -> bool {
    if is_hand_written_native(info) || !strip_option(ty).1 {
        return false;
    }
    match &info.effective_view {
        // Only *this* branch gets the "declared directly on this type" exemption — a type with no
        // `view` at all (e.g. `TextBlock`, whose real constructor takes no arguments and whose own
        // `color`/`text_alignment` genuinely do have real `set_<name>` setters) must keep every
        // `Option<T>` field deferred regardless of who declares it, so this must never affect the
        // `None` arm below.
        Some(view) => {
            let declared_here = info
                .declaring_types
                .get(name)
                .is_some_and(|owner| owner == component_name);
            !declared_here && !view_references_name_anywhere(view, name)
        }
        None => true,
    }
}

/// Whether a `has_view` target's own `param_fields` member `name` (no initializer, so ordinarily
/// construction-only — see `emit_resync`'s param-skip guard) still gets a real generated `set_<name>`
/// despite that, so `emit_resync` should keep resyncing it rather than skip it. Two independent
/// reasons a no-initializer field ends up with a setter after all — mirrors `generate_view`'s own
/// field split, from `TypeInfo` alone (no local `generate_view` state needed):
/// - It's *deferred* (`is_deferred_field`): `Option<T>`, never referenced in its own view, so
///   `generate_view` drops it from `new(..)`'s positional args entirely and gives it a setter
///   instead.
/// - It's a required `prop` (not `#[param]`) field (`generate_view`'s `mutable_required_names`):
///   needed eagerly at construction (so it can't be deferred), but declared runtime-mutable per
///   docs/specs/dsl_spec.md §4's param/prop split — `generate_view` keeps it a positional `new(..)`
///   argument *and* gives it a resync-triggering setter. Gated on `!info.is_builtin`: this rule
///   only holds for a genuinely `generate_view`-generated user component — `elwindui-codegen`'s own
///   the builtin shape set also declares a `view` for `Rectangle`/`Ellipse`/`ContentControl`
///   (`has_view: true` too), but purely for symbol-table/validation purposes (docs/
///   docs/design/runtime/ui_tree_design.md) — their real implementation is hand-written directly in
///   `elwindui_core::ui`, never run through `generate_view`, so a "no `#[param]`" field there
///   (e.g. `Rectangle::corner_radius`) may have no real setter at all regardless of `FieldKind`.
fn is_settable_field(info: &TypeInfo, component_name: &str, name: &str, ty: &str) -> bool {
    is_deferred_field(info, component_name, name, ty)
        || (!info.is_builtin
            && info
                .effective_fields
                .iter()
                .any(|f| f.name == name && f.kind == FieldKind::Prop))
}

/// Whether `name` is a `has_view` target's own field carrying a plain default expression
/// (`FieldKind::Prop`, `Initializer::Expr(..)` — e.g. `#[prop(default = false)]`/`label: String =
/// "".to_string()`) — as opposed to a `#[param]`/no-initializer field (already in `param_fields`),
/// a `#[computed]` field (never independently settable from outside), or an
/// action. Such a field has an initializer, so `param_fields` (only ever "every no-initializer
/// field") never includes it, and unlike an `Option<..>`-typed deferred field
/// (`is_deferred_field`) it previously had **no** way to be overridden from a use site at all —
/// its own declared default was the only value it could ever have, even though `generate_view`
/// already gives it a real `set_<name>` (the same one `#[computed]`'s own recompute cascade and a
/// same-component bare-identifier reference both call). `build_component_optional_setters` (below)
/// closes that gap: a use site providing an explicit attribute for one of these now gets a real
/// post-construction `set_<name>(value)` call, exactly like a deferred `Option<..>` field already
/// does. Gated on `!info.is_builtin` for the same reason `is_settable_field` is — this crate's own
/// the builtin shape set never declares a defaulted `Prop` field today (only `#[attached]`
/// fields have a `= expr` default, a wholly separate mechanism — `emit_attached_setters`), but
/// nothing here should assume a hand-written native's own defaulted field (if one existed) works
/// this same way.
fn is_defaulted_settable_field(info: &TypeInfo, name: &str) -> bool {
    !info.is_builtin
        && info.effective_fields.iter().any(|f| {
            f.name == name
                && f.kind == FieldKind::Prop
                && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
}

/// Evaluates a resolved user-component node's own attributes into the positional argument list its
/// generated `new(..)`/`create_<snake case>(..)` (docs/design/runtime/ui_tree_design.md) expects, in
/// `info.param_fields`'s declared order — shared by `emit_construction` (wraps as `Type::new(args)`)
/// and `build_component_value` (wraps as `create_<snake case>(args)`, for a shape-composition root
/// whose base is itself a DSL component rather than a hand-written `elwindui::core::ui` primitive).
/// Skips a deferred field (`is_deferred_field`) entirely — no positional slot at all, not even a
/// placeholder `None` — since that target's own `new(..)` does not declare one; the matching
/// value (if this use site supplies one) is applied afterward instead, via
/// `build_component_optional_setters`.
fn build_component_args(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    info: &TypeInfo,
    plan: &[PlannedNode],
    defer_content: bool,
) -> Vec<TokenStream> {
    // A bare nested child element (no `name:` attribute) only ever has somewhere to go if this
    // component declares a `children`-named param (a list, consumed in full below) or a
    // `#[content(field_name)]` (a single slot, consumed further down) — anything else, with no
    // declared destination at all, is a codegen-time authoring mistake, not a silently-guessed
    // field declaration order.
    let has_children_field = info.param_fields.iter().any(|(name, _)| name == "children");
    // A composed component routes bare children into whatever content slot the shape it composes
    // over declares (`ContentControl`'s `content`, ...), reached at runtime through that shape's own
    // `__elwindui_shape_*!` protocol rather than through a local `TypeInfo` — so there is nothing to
    // check here, and `content_field` being `None` says nothing about whether a destination exists.
    // Without this, `Derived inherits <user component>` was rejected outright whenever the base
    // composed over something with a content slot, since the composition plans the base's own view
    // root as a bare child of the base node.
    let routes_children_through_composition =
        info.composed_shape.is_some() || info.host_composition_base.is_some();
    if !has_children_field
        && info.content_field.is_none()
        && !routes_children_through_composition
        && !node.child_bindings.is_empty()
    {
        panic!(
            "`{}` has no `children` field or `#[content(field_name)]` to receive its {} bare nested child element(s) — \
             add an explicit `name: value` attribute for each, or declare `#[content(field_name)]` on the component",
            node.type_path,
            node.child_bindings.len()
        );
    }

    let mut args = Vec::new();
    for (name, ty) in &info.param_fields {
        if is_deferred_field(info, &node.type_path, name, ty) {
            continue;
        }
        // A shape-composition root is first constructed as a plain base value and only receives
        // its effective content after the enclosing component has an `Rc`/self-weak. This keeps
        // parent links and scalar root replacement on the same generic post-construction path as
        // collection insertion. Ordinary element construction keeps the historical positional
        // argument behavior (`defer_content == false`).
        if defer_content
            && info.content_field.as_deref() == Some(name.as_str())
            && !node.child_bindings.is_empty()
        {
            continue;
        }
        if name == "children" {
            let wants_node = is_ui_element_type(ty);
            let items = node
                .child_bindings
                .iter()
                .filter(|(_, child_ty)| child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                .map(|(c, child_ty)| {
                    if wants_node {
                        into_node_if_needed(quote! { #c }, child_ty, from, table)
                    } else {
                        into_any_view_if_needed(quote! { #c }, ty)
                    }
                });
            args.push(quote! { vec![ #(#items),* ] });
            continue;
        }

        let (inner_ty, is_option) = strip_option(ty);
        let attr = find_attr(node, name);
        let value = match attr {
            Some(ViewExpr::Element(_)) => {
                let (nested_binding, nested_ty) = node
                    .element_attr_bindings
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
                if is_ui_element_type(inner_ty) {
                    into_node_if_needed(quote! { #nested_binding }, nested_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
                }
            }
            Some(ViewExpr::Closure { params, body }) => {
                emit_closure_value(params, body, ctx, from, table)
            }
            // `is_deferred_field` always routes a `ViewTemplate`-typed (`Option<T>`, never
            // referenced elsewhere in its own view — Issue #162 §3.9) field's value here only when
            // that guard doesn't apply; kept for defense-in-depth/exhaustiveness, not a normally-
            // reached path for `context_popup` itself. `emit_deferred_view_value`'s own `Some(..)`-
            // wrap is applied uniformly below with every other value.
            Some(ViewExpr::DeferredView(deferred)) => {
                emit_deferred_view_value(deferred, ctx, from, table)
            }
            Some(other) => {
                if let Some(coerced) = coerce_color_literal(inner_ty, other) {
                    if is_option && is_brush_type(inner_ty) {
                        quote! { Some(#coerced) }
                    } else {
                        coerced
                    }
                } else {
                    let value = emit_expr(other, ctx, &EmitMode::Construction);
                    // A `String`-shaped param takes `&str` in every *hand-written* builtin (matching
                    // the shape declaration's `String`/`Option<String>` — see this crate's own
                    // `elwindui-core::ui`), so the value is wrapped in `&(..)` here regardless of
                    // whether the DSL expression itself is a `&str` literal or a computed `String`
                    // (e.g. `t!(...)`) — Rust's deref coercion accepts either as `&str` at the call
                    // site. A `view`-having (`info.has_view`) component's
                    // *generated* `new(..)` instead takes the field's literal declared type verbatim
                    // (`generate_view`'s `param_types`) — for a plain `#[param] label: String` that's an
                    // owned `String`, so a `&str` literal (e.g. `Rectangle { fill: "#3a3a3c" }`) needs
                    // `.to_string()` instead of `&(..)` to match it; `.to_string()` is just as happy
                    // taking an already-owned `String` expression (a fresh, harmless copy), so this
                    // applies uniformly regardless of which shape the DSL expression itself has.
                    if inner_ty == "String" {
                        if info.has_view {
                            quote! { (#value).to_string() }
                        } else {
                            quote! { &(#value) }
                        }
                    } else if is_ui_element_type(inner_ty) {
                        // A bare-forwarded own field (`content: canvas`, `ViewExpr::Path`) whose
                        // *target* wants `dyn UIElement` but whose own declared type is some
                        // concrete element (own `#[param] canvas: Rc<SomeConcreteElement>`) needs
                        // the same `.into_node()` conversion a literal nested element already gets
                        // via `into_node_if_needed` (`Some(ViewExpr::Element(_))`'s own arm, above)
                        // — a bare `ViewExpr::Path` never went through that arm at all, so without
                        // this the raw concrete-typed value hits the `dyn UIElement`-typed setter
                        // straight, a type mismatch. `bare_own_field_type` resolves the *source*
                        // field's own declared type from `ctx.own_fields`; `into_node_if_needed`
                        // itself safely degrades to an unconditional `.into_node()` call when that
                        // type doesn't resolve as a real symbol-table entry (e.g. a hand-written,
                        // non-DSL `#[elwindui::class]` leaf like a demo's own drawing canvas).
                        let source_type = bare_own_field_type(other, ctx).unwrap_or_default();
                        into_node_if_needed(value, &source_type, from, table)
                    } else {
                        value
                    }
                }
            }
            None if is_option => {
                args.push(quote! { None });
                continue;
            }
            None if info.content_field.as_deref() == Some(name.as_str())
                && !node.child_bindings.is_empty() =>
            {
                if node.child_bindings.len() > 1 {
                    panic!(
                        "`{}`'s `#[content({name})]` field can only bind a single nested child element, found {}",
                        node.type_path,
                        node.child_bindings.len()
                    );
                }
                let (child, child_ty) = &node.child_bindings[0];
                if child_ty == DYNAMIC_CHILD_SLOT_MARKER {
                    initial_dynamic_content_value(plan, child, inner_ty, ctx, from, table)
                } else if is_ui_element_type(inner_ty) {
                    into_node_if_needed(quote! { #child }, child_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #child }, inner_ty)
                }
            }
            None => panic!("`{}` requires attribute `{name}`", node.type_path),
        };
        // `ty` is one of `synthesize_external_base_fields`'s synthesized fields (a type-position
        // macro invocation, always containing a literal `!` — Refs #90): `strip_option`'s string
        // matching can't tell whether the base declared it `Option<T>` or bare `T` (`padding` is the
        // former; collection fields such as Layout's `children` are the latter), so `is_option`
        // above is unreliable here —
        // always `false`, since the opaque string never literally starts with `"Option<"`. `.into()`
        // resolves this generically at the *consumer's* own expansion time instead of guessing here:
        // the blanket `impl<T> From<T> for Option<T>` handles the `Option<T>`-declared case (`value`
        // is always a bare `T` at this point — a DSL literal/computed expression, never already
        // `Option`-wrapped, unlike the bare-own-field-forward value `emit_resync`'s matching branch
        // has to handle), and `core`'s own reflexive `impl<T> From<T> for T` makes it a no-op for the
        // bare-`T`-declared case — so this is correct either way, with no need to know which.
        if ty.contains('!') {
            args.push(quote! { (#value).into() });
        } else {
            args.push(if is_option {
                quote! { Some(#value) }
            } else {
                value
            });
        }
    }
    args
}

/// The post-construction-setter analog of `build_component_args` — used by `emit_construction`'s
/// `is_hand_written_native` branch instead of positional constructor args (docs/design/README.md
/// §5.1's post-construction setter convention, extended to every builtin's own declared
/// `#[param]`s, the same way `emit_common_ui_element_setters` already applies it to
/// margin/grid_cell). Mirrors `build_component_args`'s field-by-field value
/// computation exactly (same bare-children/`ViewExpr::Element`/`ViewExpr::Closure`/
/// `#[content(field_name)]` handling), except:
/// - an absent `Option<..>`-typed attribute emits **no call at all** (the zero-argument
///   constructor's own default already applies) rather than a placeholder `None`;
/// - an `Option<..>`-typed attribute that *is* present is passed to the setter **unwrapped**
///   (its inner type), never `Some(..)`-wrapped, matching the setters used by `emit_resync`;
/// - a `String`-shaped param still takes `&str` at the hand-written setter (unlike
///   `build_component_args`'s `has_view`-conditional `.to_string()`, which never applies here
///   since `is_hand_written_native` implies `!info.has_view`);
fn build_component_setters(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    info: &TypeInfo,
    plan: &[PlannedNode],
) -> Vec<TokenStream> {
    let has_children_field = info.param_fields.iter().any(|(name, _)| name == "children");
    // A composed component routes bare children into whatever content slot the shape it composes
    // over declares (`ContentControl`'s `content`, ...), reached at runtime through that shape's own
    // `__elwindui_shape_*!` protocol rather than through a local `TypeInfo` — so there is nothing to
    // check here, and `content_field` being `None` says nothing about whether a destination exists.
    // Without this, `Derived inherits <user component>` was rejected outright whenever the base
    // composed over something with a content slot, since the composition plans the base's own view
    // root as a bare child of the base node.
    let routes_children_through_composition =
        info.composed_shape.is_some() || info.host_composition_base.is_some();
    if !has_children_field
        && info.content_field.is_none()
        && !routes_children_through_composition
        && !node.child_bindings.is_empty()
    {
        panic!(
            "`{}` has no `children` field or `#[content(field_name)]` to receive its {} bare nested child element(s) — \
             add an explicit `name: value` attribute for each, or declare `#[content(field_name)]` on the component",
            node.type_path,
            node.child_bindings.len()
        );
    }

    let binding = &node.binding;
    let mut setters = Vec::new();
    for (name, ty) in &info.param_fields {
        let setter_ident = format_ident!("set_{}", name);
        let is_this_field_content = info.content_field.as_deref() == Some(name.as_str());
        // `docs/specs/dsl_spec.md` §3 (`#[content(field_name)]`'s own paragraph): bare nested
        // children bind to *some* field either via an explicit `#[content(field_name)]`, or — the
        // spec's documented fallback — a plain field literally named `children` with a list type.
        // Which of the two *emission* shapes applies (bulk `set_<field>(vec![...])` vs a
        // `.{field}().add(child)` loop against a live accessor) is derived purely from the
        // destination field's own declared type, not from which of the two mechanisms named it —
        // `Vec<T>` (e.g. `TabView`'s `children`) uses the former; `ListExt<T>` (e.g. `Menu`/
        // `MenuBar`'s `#[content(items)]` `items: ListExt<MenuItem>`, docs/specs/ui_spec.md#menu)
        // uses the latter, mirroring `Layout`'s own `.children().add(..)`
        // convention for virtual builtins (`build_virtual_value`) one level up.
        if (name == "children" || is_this_field_content) && ty.trim_start().starts_with("Vec<") {
            let wants_node = is_ui_element_type(ty);
            let items = node
                .child_bindings
                .iter()
                .filter(|(_, child_ty)| child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                .map(|(c, child_ty)| {
                    if wants_node {
                        into_node_if_needed(quote! { #c }, child_ty, from, table)
                    } else {
                        into_any_view_if_needed(quote! { #c }, ty)
                    }
                });
            setters.push(quote! { #binding.#setter_ident(vec![ #(#items),* ]); });
            continue;
        }
        if is_this_field_content && ty.contains("ListExt<") {
            let accessor_ident = format_ident!("{name}");
            // `.clone()` (an `Rc` refcount bump), not a bare move — each child binding is also
            // separately stored as its own struct field (`generate_view`'s `Self { #(#field_inits,)*
            // .. }`), so the original binding must stay valid for that later use, exactly like
            // `into_any_view_if_needed`'s own default (non-`AnyView`) clone convention just above.
            let items = node
                .child_bindings
                .iter()
                .map(|(c, _)| quote! { #c.clone() });
            setters.push(quote! {
                for __c in vec![ #(#items),* ] { #binding.#accessor_ident().add(__c); }
            });
            continue;
        }

        let (inner_ty, is_option) = strip_option(ty);
        let attr = find_attr(node, name);
        if let Some(expr) = attr.filter(|_| is_semantic_brush_property(info, name)) {
            let raw = emit_expr(expr, ctx, &EmitMode::Construction);
            let environment = semantic_brush_construction_environment(node, ctx);
            let receiver = quote! { #binding };
            let set = emit_field_setter_call(
                name,
                &node.type_path,
                &setter_ident,
                quote! { Some(__elwindui_semantic_brush) },
                &receiver,
                from,
                table,
            );
            let clear = emit_field_clear_call(name, &node.type_path, &receiver, from, table);
            setters.push(emit_semantic_brush_resolution(raw, environment, set, clear));
            continue;
        }
        let value = match attr {
            Some(ViewExpr::Element(_)) => {
                let (nested_binding, nested_ty) = node
                    .element_attr_bindings
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
                if is_ui_element_type(inner_ty) {
                    into_node_if_needed(quote! { #nested_binding }, nested_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
                }
            }
            Some(ViewExpr::Closure { params, body }) => {
                emit_closure_value(params, body, ctx, from, table)
            }
            Some(ViewExpr::DeferredView(deferred)) => {
                emit_deferred_view_value(deferred, ctx, from, table)
            }
            Some(other) => {
                let value = if let Some(coerced) = coerce_color_literal(inner_ty, other) {
                    coerced
                } else {
                    emit_expr(other, ctx, &EmitMode::Construction)
                };
                if is_option && is_brush_type(inner_ty) {
                    quote! { Some(#value) }
                } else if inner_ty == "String" {
                    quote! { &(#value) }
                } else if is_ui_element_type(inner_ty) {
                    // Mirrors `build_component_args`/`build_virtual_value`'s identically-named
                    // branch — see that one's own doc comment.
                    let source_type = bare_own_field_type(other, ctx).unwrap_or_default();
                    into_node_if_needed(value, &source_type, from, table)
                } else {
                    value
                }
            }
            None if is_option => continue,
            None if is_this_field_content && !node.child_bindings.is_empty() => {
                if node.child_bindings.len() > 1 {
                    panic!(
                        "`{}`'s `#[content({name})]` field can only bind a single nested child element, found {}",
                        node.type_path,
                        node.child_bindings.len()
                    );
                }
                let (child, child_ty) = &node.child_bindings[0];
                if child_ty == DYNAMIC_CHILD_SLOT_MARKER {
                    initial_dynamic_content_value(plan, child, inner_ty, ctx, from, table)
                } else if is_ui_element_type(inner_ty) {
                    into_node_if_needed(quote! { #child }, child_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #child }, inner_ty)
                }
            }
            None => panic!("`{}` requires attribute `{name}`", node.type_path),
        };
        setters.push(emit_field_setter_call(
            name,
            &node.type_path,
            &setter_ident,
            value,
            &quote! { #binding },
            from,
            table,
        ));
    }
    setters
}

/// Builds trailing `.set_<field>(value)` calls for a `has_view`/plain component's own *deferred*
/// `Option<T>` fields (`is_deferred_field`, used alongside `build_component_args`'s now-shrunk
/// positional list — see `emit_construction`'s non-`is_hand_written_native` branch) *and* its own
/// defaulted `Prop` fields (`is_defaulted_settable_field` — a field with a plain default expression,
/// e.g. `#[prop(default = false)]`, which never becomes a positional `new(..)` argument at all).
/// Only ever emits a call when this use site actually supplies a value for the field — an absent
/// one leaves that field's own already-applied default (`RefCell::new(None)`/`Cell::new(None)` for
/// a deferred field, or the declared default expression itself for a defaulted one) in place
/// (`generate_view`/`generate_component`'s own field-splitting doc comment).
fn build_component_optional_setters(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    info: &TypeInfo,
) -> Vec<TokenStream> {
    let binding = &node.binding;
    let deferred_fields = info
        .param_fields
        .iter()
        .filter(|(name, ty)| is_deferred_field(info, &node.type_path, name, ty))
        .map(|(name, ty)| (name.as_str(), ty.as_str()));
    let defaulted_fields = info
        .effective_fields
        .iter()
        .filter(|f| is_defaulted_settable_field(info, &f.name))
        .map(|f| (f.name.as_str(), f.ty.as_str()));

    let mut setters = Vec::new();
    for (name, ty) in deferred_fields.chain(defaulted_fields) {
        let setter_ident = format_ident!("set_{}", name);
        // A deferred field is always `Option<..>` (`is_deferred_field`'s own guard); a defaulted
        // field (`is_defaulted_settable_field`) is whatever plain type it was declared with —
        // `strip_option` is a no-op for the latter, so `inner_ty` is always the right type either
        // way.
        let (inner_ty, _) = strip_option(ty);
        if let Some(expr) = find_attr(node, name).filter(|_| is_semantic_brush_property(info, name))
        {
            let raw = emit_expr(expr, ctx, &EmitMode::Construction);
            let environment = semantic_brush_construction_environment(node, ctx);
            let receiver = quote! { #binding };
            let set = emit_field_setter_call(
                name,
                &node.type_path,
                &setter_ident,
                quote! { Some(__elwindui_semantic_brush) },
                &receiver,
                from,
                table,
            );
            let clear = emit_field_clear_call(name, &node.type_path, &receiver, from, table);
            setters.push(emit_semantic_brush_resolution(raw, environment, set, clear));
            continue;
        }
        let value = match find_attr(node, name) {
            Some(ViewExpr::Element(_)) => {
                let (nested_binding, nested_ty) = node
                    .element_attr_bindings
                    .get(name)
                    .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
                if is_ui_element_type(inner_ty) {
                    into_node_if_needed(quote! { #nested_binding }, nested_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
                }
            }
            Some(ViewExpr::Closure { params, body }) => {
                emit_closure_value(params, body, ctx, from, table)
            }
            Some(ViewExpr::DeferredView(deferred)) => {
                emit_deferred_view_value(deferred, ctx, from, table)
            }
            Some(other) => {
                if let Some(coerced) = coerce_color_literal(inner_ty, other) {
                    coerced
                } else {
                    let value = emit_expr(other, ctx, &EmitMode::Construction);
                    // The generated `set_<field>` setter takes the field's own declared (owned) inner
                    // type, e.g. `String` — not `&str` the way a hand-written builtin's setter does
                    // (`build_component_setters`) — matching `build_component_args`'s own
                    // `has_view`-conditional `.to_string()` convention.
                    if inner_ty == "String" {
                        quote! { (#value).to_string() }
                    } else {
                        value
                    }
                }
            }
            None => continue,
        };
        setters.push(emit_field_setter_call(
            name,
            &node.type_path,
            &setter_ident,
            value,
            &quote! { #binding },
            from,
            table,
        ));
    }
    setters
}

/// Builds the plain (not yet `Rc`-wrapped) `create_<snake case>(args)` call for a shape-composition
/// root whose base is a resolved DSL component (rather than a hand-written `elwindui::core::ui`
/// primitive — see `build_virtual_value` for that case) — e.g. `RoundedPanel inherits ContentControl`,
/// whose own `view` root literally constructs `ContentControl`. Mirrors `emit_construction`'s
/// `Type::new(args)` shape exactly, just calling the base's own plain factory instead (see
/// `generate_view`'s `is_shape_composition` branch).
///
/// Deferred fields of a composed base are not supported at this expression-only call site.
///
/// `component` is the *caller's own* component being generated — its only use here is
/// `immediate_base_qualified_path`/`qualified_construct_path`, which only ever fire when `node`
/// (always the shape-composition root, this function's one call site) is literally `component`'s
/// own `inherits` base written as a qualified path (Refs #25); every other case falls through to
/// the existing bare/builtin `dsl_construct_path` resolution, unchanged.
fn build_component_value(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    plan: &[PlannedNode],
    component: &ComponentDef,
) -> TokenStream {
    let Some(info) = table.resolve(from, &node.type_path) else {
        // External (no local `TypeInfo`) — same rule every other construction path applies
        // (`concrete_type_ident`'s own doc comment): unresolved here means declared entirely
        // outside this compilation's own AST, so treated the same as a known builtin. Every real
        // builtin's own `construct()` takes no arguments (the project-wide "no-args factory +
        // setters" convention this crate's own doc comment on `emit_hand_written_native`/
        // `TypeInfo` references) — the same shape this function already special-cased for
        // `ContentControl` specifically before any builtin lost its `TypeInfo`; that special case
        // was never really about `ContentControl`'s *name*, just the only shape-composition base a
        // real `#[component]` file happened to use with args. `build_component_args`
        // needs a real field list to decide what's required/positional, which — as with every other
        // external path — no longer exists to consult.
        let construct_path = qualified_construct_path(component, &node.type_path)
            .unwrap_or_else(|| dsl_construct_path(&node.type_path, None));
        return quote! { #construct_path() };
    };
    let construct_path = qualified_construct_path(component, &node.type_path)
        .unwrap_or_else(|| dsl_construct_path(&node.type_path, Some(info)));
    let args = build_component_args(node, ctx, from, table, info, plan, true);
    quote! { #construct_path(#(#args),*) }
}

/// Emits post-construction `set_attached::<T>(..)` calls (docs/design/runtime/ui_tree_design.md) for
/// whichever attached properties `node` actually specifies — shared by `emit_virtual_construction`
/// (virtual builtins) and `emit_construction`'s native-control-leaf branch (`Button`/`TextArea`/
/// `TabView` — see `TypeInfo::is_native_control_leaf`). `margin`/`width`/`height`/... (every other
/// common `UIElement` attribute) no longer need a separate call here — they're ordinary
/// `param_fields` members now (`resolve_effective_fields`'s own exemption for fields declared
/// directly on `UIElement`), so `build_component_setters`/`build_virtual_value`'s own generic,
/// field-name-agnostic per-field loops already emit their setter calls. A use site with no attached
/// properties at all emits nothing.
fn emit_common_ui_element_setters(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    binding: &TokenStream,
) -> TokenStream {
    let out = emit_attached_setters(node, ctx, from, table, &EmitMode::Construction, binding);
    // `.as_ui_element()` (called inside `emit_attached_setters`'s own `set_attached::<T>(..)`) is a
    // trait method (`elwindui::core::ui::UIElementExt`), not an inherent one — needs the trait in
    // scope here since `binding` is a concrete type in both of this function's callers (never a
    // `dyn UIElementExt` trait object, which wouldn't need the import at all). A no-op (empty `out`)
    // skips it — no `.as_ui_element()` call to guard.
    if out.is_empty() {
        out
    } else {
        quote! {
            {
                use elwindui::core::ui::UIElementExt as _;
                #out
            }
        }
    }
}

/// Emits `binding.as_ui_element().register_routed_handler::<()>("on_click", ..)` for the generic "any
/// element can catch a routed `on_click`" common attribute (docs/specs/dsl_spec.md §12) — used by
/// `emit_virtual_construction` unconditionally, and by `emit_construction`'s native-control-leaf
/// branch only when the type doesn't *already* declare `on_click` as a real `#[routed]` field of
/// its own (`Button` — wired instead by `emit_wiring`'s dedicated `is_routed` branch; applying this
/// generic mechanism too would register the same callback twice).
fn emit_generic_on_click_routing(
    node: &PlannedNode,
    ctx: &ViewCtx,
    binding: &TokenStream,
) -> TokenStream {
    match find_attr(node, "on_click") {
        Some(expr) => {
            let call = emit_expr(expr, ctx, &EmitMode::Construction);
            // `.as_ui_element()` is a trait method (`elwindui::core::ui::UIElement`) — see
            // `emit_common_ui_element_setters`'s own matching guard for why this needs its own
            // local `use`.
            quote! {
                {
                    use elwindui::core::ui::UIElementExt as _;
                    #binding.as_ui_element().register_routed_handler::<()>("on_click", Box::new(move |_: &(), _args: &elwindui::core::input::RoutedEventArgs| { #call; }));
                }
            }
        }
        None => quote! {},
    }
}

/// A `#[shortcut("Ctrl+Shift+S")]` key spec, parsed once and shared between `validate.rs` (checks
/// the spec is well-formed before codegen ever runs) and `emit_shortcut_chord_expr` (turns it into
/// an `elwindui::core::input::KeyChord` expression) — see `ast::Attr::Shortcut`'s own doc comment.
pub(crate) struct ParsedShortcut {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
    pub key: ShortcutKey,
}

pub(crate) enum ShortcutKey {
    /// One of `elwindui_core::input::Key`'s named variants, spelled exactly as declared there
    /// (`"Enter"`, `"F1"`, ...) — interpolated directly as `Key::#ident`.
    Named(&'static str),
    Character(char),
}

/// Every `elwindui_core::input::Key` variant other than `Character` — see `ShortcutKey::Named`.
const SHORTCUT_NAMED_KEYS: &[&str] = &[
    "Enter",
    "Escape",
    "Tab",
    "Backspace",
    "Delete",
    "Space",
    "Up",
    "Down",
    "Left",
    "Right",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
];

/// Parses one `+`-separated `#[shortcut(...)]` key spec (`"Ctrl+Shift+S"`) into modifier flags plus
/// the key itself. The last `+`-separated part is always the key; every part before it must be one
/// of `Ctrl`/`Shift`/`Alt`/`Meta` (docs/design/runtime/input_focus_design.md's platform-neutral
/// modifier vocabulary — never `Cmd`, which only exists as codegen's own macOS remap of `Ctrl`, see
/// `resolve_shortcut_chord`).
pub(crate) fn parse_shortcut_spec(spec: &str) -> Result<ParsedShortcut, String> {
    let mut shift = false;
    let mut control = false;
    let mut alt = false;
    let mut meta = false;
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    let Some((key_part, modifier_parts)) = parts.split_last() else {
        return Err(format!("empty #[shortcut] key spec `{spec}`"));
    };
    for m in modifier_parts {
        match *m {
            "Ctrl" => control = true,
            "Shift" => shift = true,
            "Alt" => alt = true,
            "Meta" => meta = true,
            other => {
                return Err(format!(
                    "unknown #[shortcut] modifier `{other}` in `{spec}` (expected Ctrl/Shift/Alt/Meta)"
                ));
            }
        }
    }
    let key = if let Some(named) = SHORTCUT_NAMED_KEYS.iter().find(|n| **n == *key_part) {
        ShortcutKey::Named(named)
    } else {
        let mut chars = key_part.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => ShortcutKey::Character(c.to_ascii_lowercase()),
            _ => {
                return Err(format!(
                    "unknown #[shortcut] key `{key_part}` in `{spec}` (expected a single character or one of {SHORTCUT_NAMED_KEYS:?})"
                ));
            }
        }
    };
    Ok(ParsedShortcut {
        shift,
        control,
        alt,
        meta,
        key,
    })
}

/// One `#[shortcut(...)]` chord spec resolved for a specific backend — see
/// `resolve_shortcut_chord`'s own doc comment for `Specific` vs `Fallback`.
enum ResolvedShortcutChord<'a> {
    Specific(&'a str),
    Fallback(&'a str),
}

/// Picks which chord spec applies to `backend_name` (`"appkit"`/`"winui3"`) out of a
/// `#[shortcut(...)]` field's own declared `chords` list: that backend's own explicit entry if
/// present (`Specific` — used verbatim, no remapping), else the first backend-agnostic (`None`-
/// keyed) entry if any (`Fallback` — `emit_shortcut_chord_expr` applies `resolve_shortcut_chord`'s
/// platform remap to this case only), else `None` (no applicable chord at all for this backend —
/// `emit_shortcut_registration` skips emitting anything under that backend's own `#[cfg(...)]`).
fn resolve_shortcut_chord<'a>(
    chords: &'a [(Option<String>, String)],
    backend_name: &str,
) -> Option<ResolvedShortcutChord<'a>> {
    if let Some((_, spec)) = chords
        .iter()
        .find(|(b, _)| b.as_deref() == Some(backend_name))
    {
        return Some(ResolvedShortcutChord::Specific(spec));
    }
    chords
        .iter()
        .find(|(b, _)| b.is_none())
        .map(|(_, spec)| ResolvedShortcutChord::Fallback(spec))
}

/// Builds the `elwindui::core::input::KeyChord { .. }` expression for `resolved`, applying
/// docs/design/runtime/input_focus_design.md's platform remap ("macOS向けビルドでは`Ctrl`が自動的に
/// `Cmd`に読み替えられる") only to a `Fallback` chord (a backend-agnostic spec picking up macOS's own
/// idiom automatically) on `backend_name == "appkit"` — an explicit `Specific` override (the author
/// wrote `appkit: "..."` themselves) is always used exactly as written, remap or not.
fn emit_shortcut_chord_expr(resolved: &ResolvedShortcutChord, backend_name: &str) -> TokenStream {
    let (spec, remap_ctrl_to_meta) = match resolved {
        ResolvedShortcutChord::Specific(spec) => (*spec, false),
        ResolvedShortcutChord::Fallback(spec) => (*spec, backend_name == "appkit"),
    };
    let parsed =
        parse_shortcut_spec(spec).unwrap_or_else(|e| panic!("invalid #[shortcut] key spec: {e}"));
    let control = parsed.control && !remap_ctrl_to_meta;
    let meta = parsed.meta || (parsed.control && remap_ctrl_to_meta);
    let shift = parsed.shift;
    let alt = parsed.alt;
    let key_expr = match parsed.key {
        ShortcutKey::Named(name) => {
            let ident = format_ident!("{name}");
            quote! { elwindui::core::input::Key::#ident }
        }
        ShortcutKey::Character(c) => quote! { elwindui::core::input::Key::Character(#c) },
    };
    quote! {
        elwindui::core::input::KeyChord {
            key: #key_expr,
            modifiers: elwindui::core::input::KeyModifiers {
                shift: #shift,
                control: #control,
                alt: #alt,
                meta: #meta,
            },
        }
    }
}

/// Emits `<binding>.as_ui_element().declare_shortcut(..)` for every backend covered by `chords`
/// (`resolve_shortcut_chord`), each under its own `#[cfg(feature = "backend-<name>")]` — mirrors the
/// existing Cargo-feature-flag-driven backend selection (`docs/status/implementation_status.md`'s
/// noted stand-in for the not-yet-implemented `target::backend()`), not a `match` over some runtime
/// backend enum. A backend with no applicable chord at all (`resolve_shortcut_chord` returning
/// `None`) is silently skipped — `validate::validate_shortcut_fields` warns about that case ahead of
/// time so it's never a silent surprise.
pub(crate) fn emit_shortcut_registration(
    name: &str,
    chords: &[(Option<String>, String)],
    scope: ShortcutScope,
    binding: &TokenStream,
) -> TokenStream {
    let scope_expr = match scope {
        ShortcutScope::Global => quote! { elwindui::core::input::ShortcutScope::Global },
        ShortcutScope::Local => quote! { elwindui::core::input::ShortcutScope::Local },
    };
    let mut out = TokenStream::new();
    for backend_name in ["appkit", "winui3"] {
        let Some(resolved) = resolve_shortcut_chord(chords, backend_name) else {
            continue;
        };
        let chord_expr = emit_shortcut_chord_expr(&resolved, backend_name);
        let feature = format!("backend-{backend_name}");
        out.extend(quote! {
            #[cfg(feature = #feature)]
            #binding.as_ui_element().declare_shortcut(elwindui::core::input::ShortcutDecl {
                chord: #chord_expr,
                scope: #scope_expr,
                event_name: #name,
            });
        });
    }
    out
}

/// Emits `<binding>.register_routed_handler::<T>(name, ..)` for one `#[routed]` field —
/// `param_types` (the field's own declared `fn(T0, ..)` sugar, already parsed by
/// `callback_param_types`) is the *only* source of `T`; this function never hardcodes an event
/// name or payload type of its own. Empty `param_types` -> `T = ()`, matching a bare expression or
/// zero-arg closure (`on_click`'s own established shape). Exactly one -> `T` is that declared
/// type, and `expr` must be an explicit 1-parameter closure (`on_tapped: |e| ...`) — matching
/// `TabView.on_select: fn(usize)`'s own established convention for typed callback fields (see the
/// non-routed branch in `emit_wiring`, just below this function's own caller). `binding` is an
/// already-valid receiver expression (a local `widget` variable `emit_wiring`'s own `is_routed`
/// branch already bound, alongside the `this`-capturing wrapper block that binding's closure body
/// may itself need — this function doesn't manage any of that, only the registration call itself).
fn emit_routed_registration(
    name: &str,
    expr: &ViewExpr,
    param_types: &[syn::Type],
    ctx: &ViewCtx,
    mode: &EmitMode,
    binding: &TokenStream,
) -> TokenStream {
    match param_types {
        [] => {
            let call = match expr {
                ViewExpr::Closure { params, body } if params.is_empty() => {
                    emit_on_event_closure_body(body, params, ctx, mode)
                }
                ViewExpr::Closure { params, .. } => panic!(
                    "`{name}` is #[routed] and takes no parameters, but a closure with {} \
                     parameter(s) was given",
                    params.len()
                ),
                other => emit_expr(other, ctx, mode),
            };
            quote! {
                #binding.register_routed_handler::<()>(#name, Box::new(move |_: &(), _args: &elwindui::core::input::RoutedEventArgs| {
                    #call;
                }));
            }
        }
        [payload_ty] => {
            let ViewExpr::Closure { params, body } = expr else {
                panic!(
                    "`{name}` is #[routed] and declares 1 parameter; write an explicit closure, \
                     e.g. `{name}: |e| ...`"
                );
            };
            if params.len() != 1 {
                panic!(
                    "`{name}`'s closure takes {} parameter(s) but the field declares 1",
                    params.len()
                );
            }
            let param_ident = format_ident!("{}", params[0]);
            let call = emit_on_event_closure_body(body, params, ctx, mode);
            quote! {
                #binding.register_routed_handler::<#payload_ty>(#name, Box::new(move |__payload: &#payload_ty, _args: &elwindui::core::input::RoutedEventArgs| {
                    let #param_ident = *__payload;
                    #call;
                }));
            }
        }
        _ => panic!(
            "`{name}` is #[routed] with {} parameters — routed fields support at most 1 today",
            param_types.len()
        ),
    }
}

/// Builds an `Rc<ConcreteImpl>` value for a virtual builtin (`VerticalLayout`/`HorizontalLayout`/
/// `TextBlock`/`Control`/`Grid`/`Shape` — see `is_virtual_builtin`) directly from its own
/// attributes, instead of calling a positional `Type::new(args)`. Kept at its own concrete type
/// so a `stored` node can
/// be kept on `Self` the same way any other builtin's stored field is (`generate_view`'s
/// `struct_fields`/`field_inits`, which expect `Rc<#type_ident>`) and so `emit_resync` can call its
/// real `set_*` setters later — erasure into `Rc<dyn UIElement>` happens lazily at whichever use
/// site actually needs it (`into_node_if_needed`'s own virtual-builtin branch).
fn emit_virtual_construction(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut TokenStream,
) {
    let binding = &node.binding;
    let value = build_virtual_value(node, ctx, from, table, false);
    let info = resolve_context_info(ctx, from, table, &node.type_path);
    let concrete_ty = concrete_type_ident(&node.type_path, info);
    out.extend(quote! {
        let #binding: std::rc::Rc<#concrete_ty> = #value;
    });
    let binding_ts = quote! { #binding };
    out.extend(emit_common_ui_element_setters(
        node,
        ctx,
        from,
        table,
        &binding_ts,
    ));
    out.extend(emit_generic_on_click_routing(node, ctx, &binding_ts));
}

/// Builds the plain `elwindui::core::ui::create_xxx()` (empty
/// argument — docs/design/runtime/ui_tree_design.md's post-construction setter convention, extended to
/// every builtin property) followed by whichever `set_<field>(..)` calls this use site's own
/// attributes supply, as a single block expression evaluating to the fully-configured value — the
/// value `emit_virtual_construction` normally stores directly, but which a
/// `component X inherits Y` shape-composition root (docs/design/runtime/ui_tree_design.md) needs
/// unwrapped so it can be embedded directly as `X`'s own `base` field instead of erased into
/// `Rc<dyn UIElement>` (see `generate_view`'s `is_shape_composition` branch).
fn build_virtual_value(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    defer_content: bool,
) -> TokenStream {
    let info = resolve_context_info(ctx, from, table, &node.type_path)
        .unwrap_or_else(|| panic!("unknown virtual builtin `{}`", node.type_path));
    debug_assert!(info.is_virtual_builtin);
    let type_ident = format_ident!("{}", node.type_path);
    let ext_ident = format_ident!("{}Ext", node.type_path);
    let common_field_names: HashSet<&str> = table
        .resolve(from, "UIElement")
        .map(|ui_element| {
            ui_element
                .param_fields
                .iter()
                .map(|(name, _)| name.as_str())
                .collect()
        })
        .unwrap_or_default();

    let is_own_option_field = |expr: &ViewExpr| match expr {
        ViewExpr::Path(segments) => match segments.as_slice() {
            [only] => ctx
                .own_fields
                .get(only)
                .is_some_and(|ty| ty.starts_with("Option<")),
            _ => false,
        },
        _ => false,
    };
    let mut setters = TokenStream::new();
    let mut needs_type_trait = false;
    let mut needs_ui_element_trait = false;
    // Whether any field set below is one of `UIElement`'s own (`margin`/`width`/`height`/... —
    // `common_field_names`, already resolved generically above) rather than this type's own —
    // its setter lives on `UIElementExt`, not `#ext_ident` (`{type_path}Ext`), so it needs its own
    // trait import.
    let mut needs_ui_element_ext = false;
    // One of `#[text_style]`'s seven injected properties (`font_size`/`foreground`/...) — its
    // setter lives on the hand-written `TextStyleOwner` trait (`crates/elwindui-core/src/ui.rs`),
    // never on `#ext_ident` (`#[class]` never generates a text-style setter onto `TextBlockExt`/
    // `ControlExt` — see `emit_field_setter_call`'s own matching branch for the native-leaf/
    // `emit_resync` side of this same rule), so a dot-call here needs its own trait import too.
    let mut needs_text_style_trait = false;
    for (name, ty) in &info.param_fields {
        let setter = format_ident!("set_{name}");
        let field_ident = format_ident!("{name}");
        let is_content = info.content_field.as_deref() == Some(name.as_str());
        // Shape-composition content is attached after the outer component has an `Rc`/self-weak.
        // Leave the temporary plain value empty so the generic post-construction path owns the
        // child exactly once. Ordinary virtual elements (including a nested `Control`) attach
        // through the same metadata-selected setter while their `Rc` already exists.
        if is_content && find_attr(node, name).is_none() && !node.child_bindings.is_empty() {
            if defer_content {
                continue;
            }
            let children = node
                .child_bindings
                .iter()
                .filter(|(_, child_ty)| *child_ty != DYNAMIC_CHILD_SLOT_MARKER);
            if ty == "UIElementCollection" {
                needs_ui_element_trait = true;
                let children = children.map(|(binding, child_ty)| {
                    into_node_if_needed(quote! { #binding }, child_ty, from, table)
                });
                setters.extend(
                    quote! { for __child in vec![ #(#children),* ] { __v.#field_ident().add(__child); } },
                );
            } else if let Some((binding, child_ty)) = children.clone().next() {
                let value = into_node_if_needed(quote! { #binding }, child_ty, from, table);
                needs_type_trait = true;
                setters.extend(quote! { __v.#setter(#value); });
            }
            continue;
        }
        if is_content && ty == "UIElementCollection" {
            needs_ui_element_trait = true;
            let children = node
                .child_bindings
                .iter()
                .filter(|(_, child_ty)| child_ty != DYNAMIC_CHILD_SLOT_MARKER)
                .map(|(binding, child_ty)| {
                    into_node_if_needed(quote! { #binding }, child_ty, from, table)
                });
            setters.extend(
                quote! { for __child in vec![ #(#children),* ] { __v.#field_ident().add(__child); } },
            );
            continue;
        }

        let (inner_ty, is_option) = strip_option(ty);
        let Some(expr) = find_attr(node, name) else {
            if is_option {
                continue;
            }
            panic!("`{}` requires attribute `{name}`", node.type_path);
        };
        if is_semantic_brush_property(info, name) {
            if info.text_style_fields.contains(name) {
                needs_text_style_trait = true;
            } else if common_field_names.contains(name.as_str()) {
                needs_ui_element_ext = true;
            } else {
                needs_type_trait = true;
            }
            let raw = emit_expr(expr, ctx, &EmitMode::Construction);
            let environment = semantic_brush_construction_environment(node, ctx);
            let receiver = quote! { __v };
            let set = emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                quote! { Some(__elwindui_semantic_brush) },
                &receiver,
                from,
                table,
            );
            let clear = emit_field_clear_call(name, &node.type_path, &receiver, from, table);
            setters.extend(emit_semantic_brush_resolution(raw, environment, set, clear));
            continue;
        }
        let value = if let ViewExpr::DeferredView(deferred) = expr {
            // Issue #162: a virtual builtin (e.g. `TextBlock`) has no `ViewExpr::Element`/
            // `ViewExpr::Closure` special-casing elsewhere in this function (none of its own
            // fields take one) — `context_popup`/any other `ViewTemplate`-typed field is the
            // first one that needs a non-`emit_expr` value here.
            let factory = emit_deferred_view_value(deferred, ctx, from, table);
            if is_option {
                quote! { Some(#factory) }
            } else {
                factory
            }
        } else if let Some(coerced) = coerce_color_literal(inner_ty, expr) {
            if is_option {
                quote! { Some(#coerced) }
            } else {
                coerced
            }
        } else {
            let value = emit_expr(expr, ctx, &EmitMode::Construction);
            if is_option && is_brush_type(inner_ty) {
                quote! { Some(#value) }
            } else if is_option && inner_ty == "String" {
                if is_own_option_field(expr) {
                    value
                } else {
                    quote! { Some((#value).to_string()) }
                }
            } else if is_option && is_own_option_field(expr) {
                quote! { (#value).unwrap_or_default() }
            } else if inner_ty == "String" {
                quote! { (#value).to_string() }
            } else if inner_ty.starts_with("Vec<") {
                quote! { (#value).to_vec() }
            } else if is_ui_element_type(inner_ty) {
                // Mirrors `build_component_args`'s own identically-named branch — a bare-forwarded
                // own field (`content: canvas`) whose target wants `dyn UIElement` needs the same
                // `.into_node()` conversion a literal nested element gets, which a bare
                // `ViewExpr::Path` never goes through on its own. See that branch's own doc comment.
                let source_type = bare_own_field_type(expr, ctx).unwrap_or_default();
                into_node_if_needed(value, &source_type, from, table)
            } else {
                value
            }
        };
        if info.text_style_fields.contains(name) {
            needs_text_style_trait = true;
        } else if common_field_names.contains(name.as_str()) {
            needs_ui_element_ext = true;
        } else {
            needs_type_trait = true;
        }
        setters.extend(quote! { __v.#setter(#value); });
    }

    let type_trait_use =
        needs_type_trait.then(|| quote! { use elwindui::core::ui::#ext_ident as _; });
    let ui_element_trait_use = needs_ui_element_trait.then(|| {
        quote! {
            use elwindui::core::ui::LayoutExt as _;
        }
    });
    let ui_element_ext_use =
        needs_ui_element_ext.then(|| quote! { use elwindui::core::ui::UIElementExt as _; });
    let text_style_trait_use =
        needs_text_style_trait.then(|| quote! { use elwindui::core::ui::TextStyleOwner as _; });

    quote! {
        {
            #type_trait_use
            #ui_element_trait_use
            #ui_element_ext_use
            #text_style_trait_use
            let __v = elwindui::core::ui::#type_ident::new();
            #setters
            __v
        }
    }
}

/// The concrete Rust struct to construct/store for a resolved component named `type_path` — plain
/// `format_ident!("{type_path}")` (docs/design/runtime/ui_tree_design.md: every `#[class]`-managed
/// struct, composed or not, compiles under exactly its own bare DSL name now), qualified with
/// `elwindui::ui::` when `info` says it's a builtin (a consumer-defined component has no such fixed
/// path, so it stays bare, resolved via the existing flat crate-root convention instead).
fn concrete_type_ident(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    dsl_concrete_type_path(type_path, info)
}

/// Resolves the `construct` factory through the same type-origin decision as every other
/// construction path. A qualified external/local component keeps the authored type path; a
/// builtin or unresolved unqualified name retains the existing facade fallback.
fn dsl_construct_path(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    let ty = dsl_concrete_type_path(type_path, info);
    quote! { #ty::construct }
}

/// Resolves a virtual builtin's core struct path for shape composition.
fn shape_composition_base_type(base: &str) -> TokenStream {
    let ident = format_ident!("{base}");
    quote! { elwindui::core::ui::#ident }
}

/// `name`'s own fully-qualified path, but only when `name` is literally `component`'s *immediate*
/// `inherits` base (`component.base`) *and* the DSL author wrote that base as a qualified path
/// (`ComponentDef::base_path`) — i.e. exactly the case `component_frontend::split_base_path`
/// produces for a user-defined base (Refs #25). `None` for every other name (a builtin base,
/// written bare; a base two or more `inherits` hops up; an ordinary same-crate sibling referenced
/// as a plain view element) — every call site below falls back to its own existing builtin/bare
/// resolution (`dsl_concrete_type_path`/`dsl_construct_path`/`base_trait_path`'s own origin
/// rule) in that case, unchanged from before this function existed.
///
/// A free function, not a `generate_view`-local closure: needed both inside `generate_view` itself
/// (before `component`'s own composition kind is even known, for the shape-composition root's type
/// annotation) and from `build_component_value` — a separate top-level function with no closure
/// access of its own.
///
/// Every use site embeds `name` into code emitted at (or, for the `#[class(inherits = ..)]`
/// argument this ultimately feeds, expanded from) an arbitrary module — never necessarily the same
/// module the DSL author's own `use`s are visible from — so a bare consumer-defined name can't be
/// trusted to resolve on its own; see `elwindui_macros::class::validate_fully_qualified_path`'s own
/// doc comment for the fully general version of this same requirement.
pub(crate) fn immediate_base_qualified_path(
    component: &ComponentDef,
    name: &str,
) -> Option<TokenStream> {
    if component.base.as_deref() != Some(name) {
        return None;
    }
    let raw = component.base_path.as_ref()?;
    let parsed: syn::Path = syn::parse_str(raw).unwrap_or_else(|e| {
        panic!(
            "`{name}`'s own `inherits` path `{raw}` should already be valid Rust syntax — it was \
             parsed as a `syn::Path` once already, by `elwindui_macros::parse_inherits_arg`, before \
             being stringified: {e}"
        )
    });
    Some(quote! { #parsed })
}

/// The trait path corresponding to an immediate qualified component base.  The `Ext` suffix is
/// attached to the final path segment (`crate::BaseExt`), not appended as a module segment
/// (`crate::Base::BaseExt`), because consumer components are emitted at the crate/module scope
/// alongside their generated traits.
fn immediate_base_qualified_ext_path(component: &ComponentDef, name: &str) -> Option<TokenStream> {
    if component.base.as_deref() != Some(name) {
        return None;
    }
    let raw = component.base_path.as_ref()?;
    let mut parsed: syn::Path = syn::parse_str(raw).unwrap_or_else(|e| {
        panic!("`{name}`'s own `inherits` path `{raw}` should already be valid Rust syntax: {e}")
    });
    let last = parsed
        .segments
        .last_mut()
        .expect("a qualified base path must have a final segment");
    last.ident = format_ident!("{}Ext", last.ident);
    Some(quote! { #parsed })
}

/// `immediate_base_qualified_path`, with the base's own `::construct` factory function appended —
/// the qualified counterpart to `dsl_construct_path`'s bare-name fallback. `None` under the
/// same conditions `immediate_base_qualified_path` returns `None` under.
fn qualified_construct_path(component: &ComponentDef, name: &str) -> Option<TokenStream> {
    let path = immediate_base_qualified_path(component, name)?;
    Some(quote! { #path::construct })
}

fn emit_content_presenter_wiring(plan: &[PlannedNode], ctx: &ViewCtx, out: &mut TokenStream) {
    for node in plan {
        if node.dynamic.is_some()
            || !node
                .type_path
                .rsplit("::")
                .next()
                .is_some_and(|name| name == "ContentPresenter")
        {
            continue;
        }

        let presenter = &node.binding;
        if let Some(parent) = &ctx.template_parent {
            out.extend(quote! {
                elwindui::core::ui::ContentPresenter::__bind_templated_parent(
                    &#presenter,
                    &#parent,
                );
            });
        } else if ctx.weak_bindable_owners.contains("templated_parent") {
            out.extend(quote! {
                {
                    let templated_parent = this.templated_parent.upgrade().expect(
                        "ControlTemplate templated_parent was dropped before ContentPresenter wiring"
                    );
                    elwindui::core::ui::ContentPresenter::__bind_templated_parent(
                        &#presenter,
                        &templated_parent,
                    );
                }
            });
        } else {
            out.extend(quote! {
                elwindui::core::ui::ContentPresenter::__bind_templated_parent(
                    &#presenter,
                    &this,
                );
            });
        }
    }
}

/// Attaches callbacks (`on_*`) and two-way change-back wiring to widgets that were stored on
/// `self`, each capturing a fresh `Rc::clone`. State-changing callbacks rely on their setter's
/// typed PropertyChanged notification; they must not force a blanket `resync()` afterward. No
/// per-type dispatch: any attribute named `on_*` is a callback (its shape's declared param type
/// decides whether the callback takes an index — see `emit_wiring`'s doc on `takes_index` below);
/// any attribute whose shape field is `#[two_way]` gets a `set_on_<attr>_change` callback wired
/// back into its bound path.
fn emit_wiring(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut TokenStream,
    self_is_node: bool,
) {
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { this });
    let info = resolve_context_info(ctx, from, table, &node.type_path);
    // A shape/host-composition root (`generate_view`'s `is_shape_composition`/`is_host_composition`
    // — `node.binding == root_binding`) has no separately-stored `self.#binding` field of its own:
    // it's moved into `self.base` at construction, and `self`/`this` itself *is* the tree node (see
    // that code's own doc comment). Every `let widget = this.#binding.clone();` below needs `this`
    // itself in that case instead.
    let widget_binding = if self_is_node {
        quote! { this.clone() }
    } else if ctx.is_template_storage() {
        quote! { #binding.clone() }
    } else {
        quote! {
            this.#binding
                .get()
                .expect("emit_wiring: component is not yet mounted")
                .clone()
        }
    };
    // Only inject the trait `use` when this node actually has something to wire up below — an
    // unconditional injection here left an always-unused import on any stored node with no `on_*`/
    // `#[two_way]` attribute at all (every branch of the loop below that actually emits tokens is
    // mirrored by one of these two conditions). An external (`info.is_none()`) target has no local
    // `TypeInfo` to check `two_way_fields` against (a builtin's shape lives only in its own
    // `__elwindui_props_{Name}!` macro now — see that macro's `@set_on_change` arms' own doc
    // comment), so every bare-path attribute is a *candidate* here; the macro itself silently
    // no-ops for whichever of those turn out not to be two-way.
    let needs_wiring = node.attributes.iter().any(|attribute| {
        attribute.name.starts_with("on_")
            || (attribute.kind == AssignmentKind::TwoWay
                && (info.is_some_and(|i| i.two_way_fields.contains(&attribute.name))
                    || info.is_none()))
    });
    if !needs_wiring {
        return;
    }
    let refresh_capture = ctx.refresh_capture();
    let refresh_statement = ctx.refresh_statement();
    // `emit_wiring`'s own output lands in `NotepadWindowImpl::new()`, a *different* function from
    // wherever `emit_construction` ran (for a composed/host-composed target, that's the separate
    // `create_<snake case>(..)` free function — see `generate_view`'s `create_fn`/
    // `new_construct_stmt` split) — so the `use` injected there doesn't carry over here. See
    // `emit_resync`'s own copy of this same comment.
    out.extend(builtin_trait_use(&node.type_path, info));

    // The widget handle is cloned out to its own binding *before* `this` is cloned into the
    // closure: `this.#binding.set_on_click(Box::new(move || { ...this... }))` would try to
    // borrow `this` for the method receiver while also moving it into the same statement's
    // closure argument, which the borrow checker rejects.
    for attribute in &node.attributes {
        let name = &attribute.name;
        let expr = &attribute.value;
        if let Some(_event) = name.strip_prefix("on_") {
            // External (`info.is_none()`) target: no `TypeInfo` to check `routed_fields`/
            // `field_types` against, so this builds the same bare closure/callable
            // `build_props_macro`'s `@set` arm for a `#[routed]` property now accepts (see that
            // function's own doc comment) and lets the declaration — not this call site — decide
            // whether it becomes a direct setter call or a routed registration. Always followed by
            // `__refresh_dynamic_regions()`: the non-routed branch below already always does this;
            // unconditionally doing it here too (rather than only for whichever shape the property
            // happens to have, which this function no longer knows) costs nothing when it turns out
            // to be a no-op and is never wrong to call after a user action.
            if info.is_none() {
                let name_ident = format_ident!("{name}");
                let props_macro = dsl_props_macro_path(&node.type_path, None);
                let call = match expr {
                    ViewExpr::Closure { params, body } => {
                        emit_on_event_closure_body(body, params, ctx, &self_mode)
                    }
                    other => emit_expr(other, ctx, &self_mode),
                };
                let closure_params = match expr {
                    ViewExpr::Closure { params, .. } => params
                        .iter()
                        .map(|p| format_ident!("{p}"))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                // A single-param DSL closure (`|e| { ... e.key ... }`) needs `e`'s own type
                // annotated here — without `TypeInfo`, this is the only place that can still do it
                // (`elwindui-macros`' own `routed_handler_from_bare_callable` receives this whole
                // closure as an opaque, already-parsed `$value:expr` fragment, so it can't reach
                // inside to add one). Whenever the DSL body uses the payload only as a value passed
                // straight to an already-typed function (`vm.select_tab(index)`), rustc infers `e`'s
                // type from that call site and an explicit annotation isn't needed — but a *field*
                // access (`e.key`) can't work backwards like that (a field projection needs its
                // base's type known first, unlike a call argument), so it must be spelled out. Only
                // `UIElement`'s own common `#[routed]` properties (never a per-builtin-varying one)
                // reach this — their payload type is fixed and universal regardless of which
                // concrete element `node.type_path` names, so no shape table is needed to know it.
                let annotated_params: Vec<TokenStream> = closure_params
                    .iter()
                    .map(|p| match common_routed_payload_type(name) {
                        Some(ty) if closure_params.len() == 1 => quote! { #p: #ty },
                        _ => quote! { #p },
                    })
                    .collect();
                out.extend(quote! {
                    {
                        #[allow(unused_imports)]
                        use elwindui::ui::*;
                        let widget = #widget_binding;
                        #refresh_capture
                        let this = std::rc::Rc::clone(&this);
                        #props_macro!(@set widget, #name_ident, move |#(#annotated_params),*| {
                            #call;
                            #refresh_statement
                        });
                    }
                });
                continue;
            }
            let setter = format_ident!("set_{name}");
            // `#[routed]` (docs/specs/dsl_spec.md §12): registered on the widget's own storage
            // (`Button::register_routed_handler`, delegating to its own `routed_handlers`) instead
            // of calling `set_<attr>` directly — `dispatch_routed` invokes it later, bubbling
            // through ancestors too, rather than this being the only thing that ever runs. The
            // payload type is never hardcoded here — `emit_routed_registration` derives it purely
            // from the field's own declared `fn(T)` sugar (`callback_param_types`, the same
            // mechanism the non-routed branch just below already uses for `TabView.on_select`).
            let is_routed = info.is_some_and(|i| i.routed_fields.contains(name));
            if is_routed {
                let param_types = info
                    .and_then(|i| i.field_types.get(name))
                    .map(|ty| callback_param_types(ty))
                    .unwrap_or_default();
                // `.as_ui_element()` (not a bare `widget.register_routed_handler(..)` call): a
                // native leaf's own `register_routed_handler` (`ButtonImpl` etc., hand-written in
                // `elwindui-backend-*`'s `native_ui.rs`) is a genuine inherent method, but a
                // virtual builtin's is only ever `UIElementExt`'s own default method — reachable
                // uniformly through `.as_ui_element()` regardless of which concrete type `widget`
                // is, matching `emit_generic_on_click_routing`'s own established pattern (see that
                // function's doc comment) — hence the matching local `use` just below too.
                let registration = emit_routed_registration(
                    name,
                    expr,
                    &param_types,
                    ctx,
                    &self_mode,
                    &quote! { widget.as_ui_element() },
                );
                // `#[shortcut(...)]` (docs/design/runtime/input_focus_design.md) — a per-usage-
                // site annotation on *this* element's own `on_click`/etc. attribute (`node.
                // attribute_shortcuts`, not `TypeInfo`'s field-declaration-level metadata — see
                // `ast::ElementNode::attribute_shortcuts`'s own doc comment for why). A host's own
                // `set_tree` later harvests the registration into a live `ShortcutRegistry` (see
                // `UIElement::declared_shortcuts`'s own doc comment).
                let shortcut_registration = node
                    .attribute_shortcuts
                    .get(name)
                    .map(|(chords, scope)| {
                        emit_shortcut_registration(
                            name,
                            chords,
                            *scope,
                            &quote! { widget.as_ui_element() },
                        )
                    })
                    .unwrap_or_default();
                out.extend(quote! {
                    {
                        use elwindui::core::ui::UIElementExt as _;
                        let widget = #widget_binding;
                        #refresh_capture
                        let this = std::rc::Rc::clone(&this);
                        #registration
                        #shortcut_registration
                    }
                });
                continue;
            }
            // The callback's declared arity/types (from its `fn(T0, T1, ...)` sugar, e.g.
            // `TabView`'s per-tab `on_select: fn(usize)`) drive both how many closure parameters
            // are expected and what to type them as — no more hardcoded `usize` sniffing.
            let param_types = info
                .and_then(|i| i.field_types.get(name))
                .map(|ty| callback_param_types(ty))
                .unwrap_or_default();
            if param_types.is_empty() {
                let call = match expr {
                    ViewExpr::Closure { params, body } if params.is_empty() => {
                        emit_on_event_closure_body(body, params, ctx, &self_mode)
                    }
                    ViewExpr::Closure { params, .. } => panic!(
                        "`{name}` takes no parameters, but a closure with {} parameter(s) was given",
                        params.len()
                    ),
                    other => emit_expr(other, ctx, &self_mode),
                };
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        #refresh_capture
                        let this = std::rc::Rc::clone(&this);
                        widget.#setter(Box::new(move || {
                            #call;
                            // An action can mutate a collection used by a dynamic child range.
                            // Observable collection helpers normally publish that change too, but
                            // the event callback is not the only supported action path (and a
                            // user-defined action need not mutate through a generated setter).
                            // Reconcile the owned child ranges here as well. `DynamicChildSlot`
                            // preserves unchanged Rc children, so this does not recreate an
                            // existing tab or reset its native editing state.
                            #refresh_statement
                        }));
                    }
                });
            } else {
                let ViewExpr::Closure { params, body } = expr else {
                    panic!(
                        "`{name}` needs {} parameter(s); write an explicit closure, e.g. `{name}: |x| ...`",
                        param_types.len()
                    );
                };
                if params.len() != param_types.len() {
                    panic!(
                        "`{name}`'s closure takes {} parameter(s) but the callback field declares {}",
                        params.len(),
                        param_types.len()
                    );
                }
                let param_decls = params.iter().zip(&param_types).map(|(name, ty)| {
                    let ident = format_ident!("{}", name);
                    quote! { #ident: #ty }
                });
                let call = emit_on_event_closure_body(body, params, ctx, &self_mode);
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        #refresh_capture
                        let this = std::rc::Rc::clone(&this);
                        widget.#setter(Box::new(move |#(#param_decls),*| {
                            #call;
                        }));
                    }
                });
            }
            continue;
        }

        if attribute.kind != AssignmentKind::TwoWay {
            continue;
        }
        let Some(path) = (match expr {
            ViewExpr::Path(path) => Some(path),
            _ => None,
        }) else {
            continue;
        };
        let setter = match path.as_slice() {
            [field] if ctx.mutable_own_fields.contains(field) => {
                let setter = format_ident!("set_{}", field);
                quote! { this.#setter(new_value); }
            }
            [field]
                if ctx.template_parent.is_some()
                    && ctx.template_bare_parent_fields.contains(field) =>
            {
                emit_template_setter_call(
                    &["templated_parent".to_string(), field.clone()],
                    ctx,
                    &self_mode,
                    quote! { new_value },
                )
                .expect("bare template property must lower to a setter call")
            }
            [_, _] => {
                let Some(setter) =
                    emit_template_setter_call(path, ctx, &self_mode, quote! { new_value })
                else {
                    continue;
                };
                setter
            }
            _ => continue,
        };
        let on_change = quote! {
            Box::new(move |new_value| {
                #setter
                // The model setter synchronously emits PropertyChanged. Its owning view
                // subscription applies the model→widget update; forcing a second blanket resync
                // here resets native editing state on AppKit.
            })
        };
        match info {
            // Local (known) target: `TypeInfo.two_way_fields` says definitively whether `name` is
            // two-way, so only emit the wiring when it actually is.
            Some(info) if info.two_way_fields.contains(name) => {
                let change_setter = format_ident!("set_on_{name}_change");
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        let this = std::rc::Rc::clone(&this);
                        widget.#change_setter(#on_change);
                    }
                });
            }
            Some(_) => {}
            // External (`info.is_none()`) target: no local `TypeInfo` to check `two_way_fields`
            // against, so every bare-path attribute is a candidate — hand the decision to
            // `owner`'s own props macro's `@set_on_change` arms (`elwindui-macros::class::
            // build_props_macro`, which *does* know per-property two-way-ness at its own
            // declaration site), which either splices this closure into a real `set_on_change`
            // call or silently discards it. `#[allow(unused)]`: the common (non-two-way) case
            // constructs `widget`/`this`/the closure only to have the macro's own fallback arm
            // throw them away unused — expected, not a bug. `use elwindui::ui::*;`: `set_on_change`
            // is a trait method (`{Name}Ext`), not inherent — without `TypeInfo` this function has
            // no ancestor chain to import a specific trait from, matching `emit_external_
            // construction`'s own identical glob-import rationale.
            None => {
                let name_ident = format_ident!("{name}");
                let props_macro = dsl_props_macro_path(&node.type_path, None);
                out.extend(quote! {
                    {
                        #[allow(unused, clippy::redundant_clone, unused_imports)]
                        {
                            use elwindui::ui::*;
                            let widget = #widget_binding;
                            let this = std::rc::Rc::clone(&this);
                            #props_macro!(@set_on_change #name_ident, widget, #on_change);
                        }
                    }
                });
            }
        }
    }
}

/// The payload type of `UIElement`'s own common single-param `#[routed]` properties (`ui.rs`'s own
/// `#[prop(routed, on_key_down: fn(crate::input::KeyEventArgs))]` and its siblings) — the only
/// ones `emit_wiring`'s external (`info.is_none()`) branch can name without a shape table, since
/// they're fixed and universal across every `UIElement`, never varying per concrete builtin the
/// way `Button.on_click`/`TabView.on_select` do. See that branch's own doc comment for why this is
/// needed at all: a DSL closure body that uses its payload parameter via field access (`e.key`)
/// can't have its type inferred from later usage the way one passed straight to an already-typed
/// function call can.
fn common_routed_payload_type(attr_name: &str) -> Option<TokenStream> {
    Some(match attr_name {
        "on_key_down" | "on_key_up" => quote! { elwindui::core::input::KeyEventArgs },
        "on_text_input" => quote! { elwindui::core::input::TextInputEventArgs },
        "on_pointer_pressed"
        | "on_pointer_released"
        | "on_pointer_moved"
        | "on_pointer_entered"
        | "on_pointer_exited" => quote! { elwindui::core::input::PointerEventArgs },
        "on_pointer_wheel_changed" => quote! { elwindui::core::input::PointerWheelEventArgs },
        "on_tapped" | "on_double_tapped" => quote! { elwindui::core::input::TappedEventArgs },
        _ => return None,
    })
}

/// Parses an `on_*` field's declared `fn(T0, T1, ...)` sugar type string (stored raw in
/// `TypeInfo::field_types`, e.g. `"fn(usize)"`, `"fn()"`) into its parameter types — drives how
/// many parameters `emit_wiring` expects an explicit closure attribute value to declare, and what
/// to type each one as. Splits on top-level commas only (bracket-depth-aware), so a parameter
/// type that itself contains a comma (e.g. a generic) isn't split incorrectly.
fn callback_param_types(ty: &str) -> Vec<syn::Type> {
    let inner = ty
        .trim()
        .strip_prefix("fn")
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.rsplit_once(')'))
        .map(|(inner, _)| inner)
        .unwrap_or("");
    let mut params = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                params.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        params.push(&inner[start..]);
    }
    params
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            syn::parse_str::<syn::Type>(s)
                .unwrap_or_else(|e| panic!("invalid callback parameter type `{s}`: {e}"))
        })
        .collect()
}

/// Emits an `on_*` event handler's body. `ClosureBody::Expr`'s DSL-native shapes (`vm.save`,
/// `t!(...)`, ...) already resolve correctly through the ordinary `emit_expr` — only the `syn::Expr`
/// fallback (e.g. `vm.close_tab(index)`, which the DSL's own dotted-path grammar can't fully
/// consume — see `parser::Parser::parse_closure_expr_body`) needs the same bare-owner-reference
/// rewriting `ClosureBody::Block` gets. An `Element` body makes no sense for an event handler (it's
/// a value-computation shape, `key`/`render_label`/`render_content`'s own use of `ClosureBody`).
fn emit_on_event_closure_body(
    body: &ClosureBody,
    closure_params: &[String],
    ctx: &ViewCtx,
    mode: &EmitMode,
) -> TokenStream {
    match body {
        ClosureBody::Expr(inner) => match inner.as_ref() {
            ViewExpr::Expr(raw) => {
                rewrite_view_closure_expr(raw.clone(), closure_params, ctx, mode)
            }
            other => emit_expr(other, ctx, mode),
        },
        ClosureBody::Block(block) => {
            rewrite_view_closure_block(block.clone(), closure_params, ctx, mode)
        }
        ClosureBody::Element(_) => panic!(
            "an `on_*` event handler's closure body must be an expression or `{{ .. }}` block, \
             not a nested element"
        ),
    }
}

/// Rewrites bare references to one of this component's own fields (for example `vm`) inside an
/// `on_*` event handler's closure body, or (Issue #162, PR #165 A2) an `on_mount`/`on_unmount`/
/// `on_update` lifecycle hook block, into the same `self.vm.field()`/`self.vm` forms every other
/// DSL attribute value resolves to — and, inside a lowered deferred view (`ViewCtx::
/// implicit_owner`), an otherwise-unresolved bare name that is a known-readable/-writable field of
/// the source lexical owner Component into the matching `<owner>.field()` getter / `<owner>.
/// set_field(..)` setter call (PR #165 final rereview remediation, A2 — `resolved_implicit_owner_
/// field`/`resolved_implicit_owner_setter`, schema-gated by `ImplicitOwnerCtx`).
///
/// PR #165 review remediation round 2, A2: a name is shadowed only where real Rust lexical
/// scoping would actually consider it in scope — tracked with a genuine scope stack (`scopes`,
/// innermost last), not a single block-wide flat set. The closure's own bound parameters
/// (`closure_params` at construction time, e.g. `index`) seed the outermost scope; every deeper
/// scope (a nested `{ .. }` block, an `if let`/`while let`/let-chain condition's own bindings —
/// visible only in the following block, a `for` loop's own pattern, each `match` arm's own
/// pattern independently of every other arm, a nested closure's own parameters) is pushed and
/// popped by the `VisitMut` overrides below at exactly the points real Rust scoping would enter
/// and leave them. A `let` statement's own initializer (and, for a let-else, its diverging
/// branch) is rewritten *before* its pattern's bindings are added to the current scope, so
/// `let x = x.clone();` correctly reads the *outer* `x` on the right-hand side. An earlier
/// revision (PR #165 review remediation round 1, A2) tracked shadowing with a single block-wide
/// flat `HashSet` instead — a name bound only inside one `if`/`match` arm was treated as shadowed
/// for the *entire* surrounding block, not just that arm's own body. The rereview that found this
/// (PR #165 final rereview remediation, A2) required the real scope stack this struct now uses;
/// the flat-set collector helper that revision relied on no longer exists.
struct ViewClosureRewriter<'a> {
    scopes: Vec<HashSet<String>>,
    ctx: &'a ViewCtx,
    mode: &'a EmitMode,
}

impl<'a> ViewClosureRewriter<'a> {
    fn new(closure_params: &[String], ctx: &'a ViewCtx, mode: &'a EmitMode) -> Self {
        Self {
            scopes: vec![closure_params.iter().cloned().collect()],
            ctx,
            mode,
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Adds every name `pat` binds (recursively — a single `syn::Pat::Ident` at any nesting
    /// depth inside a tuple/struct/reference/or-pattern, ...) to the *current* (innermost) scope.
    fn bind_pattern(&mut self, pat: &syn::Pat) {
        struct Collector {
            found: HashSet<String>,
        }
        impl<'ast> Visit<'ast> for Collector {
            fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
                self.found.insert(node.ident.to_string());
                syn::visit::visit_pat_ident(self, node);
            }
        }
        let mut collector = Collector {
            found: HashSet::new(),
        };
        collector.visit_pat(pat);
        if let Some(scope) = self.scopes.last_mut() {
            scope.extend(collector.found);
        }
    }

    /// Rewrites an `if`/`while` condition left-to-right, binding each `&&`-chained `Expr::Let`'s
    /// own pattern into the *caller's* current (already-pushed) scope as it goes — matching
    /// Rust's own let-chain semantics, where a later chained condition may observe an earlier
    /// one's bindings, but an ordinary (non-`let`) condition needs no scope change, only
    /// rewriting. The caller is responsible for pushing a scope before calling this and popping
    /// it after the following block has been visited (so the bindings are visible in exactly
    /// that block, never in an `else` branch or after the `if`/`while`).
    fn rewrite_let_chain_condition(&mut self, cond: &mut syn::Expr) {
        match cond {
            syn::Expr::Let(expr_let) => {
                self.visit_expr_mut(&mut expr_let.expr);
                self.bind_pattern(&expr_let.pat);
            }
            syn::Expr::Binary(bin) if matches!(bin.op, syn::BinOp::And(_)) => {
                self.rewrite_let_chain_condition(&mut bin.left);
                self.rewrite_let_chain_condition(&mut bin.right);
            }
            other => self.visit_expr_mut(other),
        }
    }

    /// A name currently shadowed by a genuine lexical binding anywhere on the current scope
    /// stack (a closure parameter, `let`/`if let`/`while let`/`match`/`for`/a nested closure).
    /// Checked before every field/implicit-owner resolution below, so ordinary Rust lexical
    /// shadowing always wins.
    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    fn resolved_owner(&self, name: &str) -> Option<TokenStream> {
        if self.is_shadowed(name) {
            return None;
        }
        if name == "templated_parent" {
            if let Some(parent) = &self.ctx.template_parent {
                return Some(quote! { #parent });
            }
        }
        if self.ctx.own_fields.contains_key(name) {
            return Some(owner_value_tokens(self.ctx, self.mode, name));
        }
        None
    }

    /// A bare 1-segment reference to one of this component's own *mutable* (`#[prop]`,
    /// `ViewCtx::mutable_own_fields`) fields, used as a *value* (read) rather than as the setter
    /// target of an assignment — mirrors `emit_expr`'s own identical `.get()`/`.borrow().clone()`
    /// handling for the same field kind (that function's own doc comment on `ctx.mutable_own_fields`
    /// explains why: it's `Cell`/`RefCell`-backed, so `self.<name>` alone would hand back the cell
    /// itself, not its value). Only matters in `WithSelf` mode — see that same comment for why
    /// `Construction` mode's raw, not-yet-cell-wrapped local needs no such unwrapping.
    fn resolved_mutable_field_read(&self, name: &str) -> Option<TokenStream> {
        if self.is_shadowed(name) {
            return None;
        }
        if !self.ctx.mutable_own_fields.contains(name) {
            return None;
        }
        let EmitMode::WithSelf(self_tok) = self.mode else {
            return None;
        };
        let ident = format_ident!("{}", name);
        let ty_str = self.ctx.own_fields.get(name)?;
        Some(if is_copy_type(ty_str) {
            quote! { #self_tok.#ident.get() }
        } else {
            quote! { #self_tok.#ident.borrow().clone() }
        })
    }

    /// PR #165 review remediation, A2 (schema-gated by PR #165 final rereview remediation, A2): a
    /// bare 1-segment name that is *not* a closure parameter, *not* one of this component's own
    /// fields, and *is* a known-readable field of the source lexical owner Component
    /// (`ViewCtx::implicit_owner`'s own `readable_fields`, `__view_owner` inside a lowered deferred
    /// view — Issue #162 §3.10-§3.11) falls back to that owner, the same fallback `emit_expr`'s own
    /// `ViewExpr::Path` handling already applies for an ordinary DSL attribute-value expression.
    /// `name` becomes `<owner>.name()` — a getter call on the (weak-upgraded, via `resolved_owner`/
    /// `owner_value_tokens`/`ctx.weak_bindable_owners`) owner value — generalizing the exact same
    /// 2-segment `owner.field` machinery `resolved_owner` already reuses, rather than duplicating
    /// it. Only reached once the closure-param/own-field/mutable-field checks above have already
    /// ruled out a local binding, preserving ordinary lexical shadowing (a component's own field of
    /// the same name always wins). The `readable_fields` membership check is what keeps an ordinary
    /// Rust name unrelated to the source Component (a module constant, `None`, a free value) from
    /// being misread as an owner field — an earlier revision fell back to the owner for *any*
    /// unshadowed bare name, which silently miscompiled `on_mount { let _ = SOME_CONST; }`-shaped
    /// code the moment `SOME_CONST` wasn't itself a real source-Component field.
    fn resolved_implicit_owner_field(&self, name: &str) -> Option<TokenStream> {
        if self.is_shadowed(name) {
            return None;
        }
        let owner = self.ctx.implicit_owner.as_ref()?;
        if !owner.readable_fields.contains(name) {
            return None;
        }
        let base = self.resolved_owner(&owner.field_name)?;
        let getter = format_ident!("{}", name);
        Some(quote! { #base.#getter() })
    }

    /// PR #165 final rereview remediation, A2 (§6): the write-side counterpart to
    /// `resolved_implicit_owner_field` — a bare 1-segment assignment target that is a known-
    /// *writable* field of the source lexical owner Component (`Prop`/`State` only, never `Param`/
    /// `Computed`/`Environment` — `implicit_owner_schema`'s own doc comment) routes through that
    /// owner's own generated `set_<name>` setter instead of being left as an invalid plain-Rust
    /// assignment (the hidden Component's own struct has no field named `name` at all in this
    /// case, so leaving it unrewritten would simply fail to compile — or worse, silently resolve
    /// to an unrelated same-named local/module item). Consulted by `Expr::Assign` handling in
    /// `visit_expr_mut`, after the hidden Component's own mutable-field case has already been ruled
    /// out.
    fn resolved_implicit_owner_setter(&self, name: &str) -> Option<TokenStream> {
        if self.is_shadowed(name) {
            return None;
        }
        let owner = self.ctx.implicit_owner.as_ref()?;
        if !owner.writable_fields.contains(name) {
            return None;
        }
        let base = self.resolved_owner(&owner.field_name)?;
        let setter = format_ident!("set_{}", name);
        Some(quote! { #base.#setter })
    }

    /// PR #165 post-final rereview remediation, A8: the 2-segment (`vm.label`, `vm.save`) raw-Rust
    /// counterpart to `resolved_implicit_owner_field` — reached only after `resolved_owner(owner)`
    /// has already failed to resolve `owner` as a real field of the *current* generated Component
    /// (e.g. inside a lowered `DeferredView` hidden Component, whose only real field is
    /// `__view_owner`). If `owner` is instead a known source-Component `#[bindable]` field
    /// (`ImplicitOwnerCtx::bindable_fields`), bridge through the source lexical owner
    /// (`__view_owner.upgrade().vm()`) rather than leaving `owner` to resolve as a nonexistent
    /// `self.vm` — the same bug `path_owner_value_tokens` fixes for ordinary DSL attribute values,
    /// mirrored here for raw `on_mount`/`on_unmount`/`on_update`/event-handler Rust.
    fn resolved_implicit_bindable_owner(&self, owner: &str) -> Option<TokenStream> {
        if self.is_shadowed(owner) {
            return None;
        }
        let implicit = self.ctx.implicit_owner.as_ref()?;
        if !implicit.bindable_fields.contains(owner) {
            return None;
        }
        let source = self.resolved_owner(&implicit.field_name)?;
        let getter = format_ident!("{}", owner);
        Some(quote! { #source.#getter() })
    }
}

impl<'a> VisitMut for ViewClosureRewriter<'a> {
    /// Every block introduces its own scope — a bare nested `{ .. }` (reached generically, as an
    /// ordinary `Expr::Block`, through `visit_expr_mut`'s own default recursive fallback) equally
    /// as the outermost hook/closure body (`rewrite_view_closure_block`'s own entry point).
    /// Bindings a block introduces never escape it.
    fn visit_block_mut(&mut self, block: &mut syn::Block) {
        self.push_scope();
        for stmt in block.stmts.iter_mut() {
            self.visit_stmt_mut(stmt);
        }
        self.pop_scope();
    }

    /// A `let` statement is the one place statement-order matters: the initializer (and, for a
    /// let-else, the diverging branch) must be rewritten *before* the pattern's own bindings are
    /// added to scope, so `let x = x.clone();` reads the *outer* `x`. Every other statement kind
    /// falls through to `syn`'s own default dispatch (which already reaches `visit_expr_mut` for
    /// an expression statement, `visit_block_mut` is not otherwise involved here).
    fn visit_stmt_mut(&mut self, stmt: &mut syn::Stmt) {
        if let syn::Stmt::Local(local) = stmt {
            if let Some(init) = &mut local.init {
                self.visit_expr_mut(&mut init.expr);
                if let Some((_, diverge)) = &mut init.diverge {
                    self.visit_expr_mut(diverge);
                }
            }
            self.bind_pattern(&local.pat);
            return;
        }
        syn::visit_mut::visit_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
        // `if`/`if let`/let-chain: a pattern-introducing condition's own bindings are visible
        // only in `then_branch`, never in `else_branch` or after the `if` — scoped by pushing
        // before the condition and popping after `then_branch` (which may itself push further
        // nested scopes for its own body, via `visit_block_mut` above), *before* `else_branch` is
        // ever visited.
        if let syn::Expr::If(expr_if) = node {
            self.push_scope();
            self.rewrite_let_chain_condition(&mut expr_if.cond);
            self.visit_block_mut(&mut expr_if.then_branch);
            self.pop_scope();
            if let Some((_, else_branch)) = &mut expr_if.else_branch {
                self.visit_expr_mut(else_branch);
            }
            return;
        }
        // `while`/`while let`: the condition's own bindings are visible only in the loop body.
        if let syn::Expr::While(expr_while) = node {
            self.push_scope();
            self.rewrite_let_chain_condition(&mut expr_while.cond);
            self.visit_block_mut(&mut expr_while.body);
            self.pop_scope();
            return;
        }
        // `for pat in iter { body }`: the iterator expression is evaluated in the *outer* scope;
        // the loop pattern's own bindings are visible only in `body`.
        if let syn::Expr::ForLoop(for_loop) = node {
            self.visit_expr_mut(&mut for_loop.expr);
            self.push_scope();
            self.bind_pattern(&for_loop.pat);
            self.visit_block_mut(&mut for_loop.body);
            self.pop_scope();
            return;
        }
        // `match scrutinee { pat1 if guard1 => body1, pat2 => body2, .. }`: the scrutinee is
        // evaluated in the outer scope; each arm's own pattern (and therefore its guard and
        // body) gets its *own independent* scope — one arm's bindings must never leak into
        // another arm, even one with a colliding name.
        if let syn::Expr::Match(expr_match) = node {
            self.visit_expr_mut(&mut expr_match.expr);
            for arm in expr_match.arms.iter_mut() {
                self.push_scope();
                self.bind_pattern(&arm.pat);
                if let Some((_, guard)) = &mut arm.guard {
                    self.visit_expr_mut(guard);
                }
                self.visit_expr_mut(&mut arm.body);
                self.pop_scope();
            }
            return;
        }
        // A nested closure's own parameters are visible only inside its own body — must not
        // shadow anything before it, and must not leak out after it.
        if let syn::Expr::Closure(closure) = node {
            self.push_scope();
            for input in closure.inputs.iter() {
                self.bind_pattern(input);
            }
            self.visit_expr_mut(&mut closure.body);
            self.pop_scope();
            return;
        }
        // `x = <rhs>` where `x` is a bare 1-segment reference to one of this component's own
        // mutable fields (`#[prop] is_checked: bool` mutated as `is_checked = !is_checked`) has no
        // real lvalue to assign into — the field's actual storage is `Cell`/`RefCell`-backed, only
        // reachable through its generated `set_<name>` setter. Rewritten to a setter call before
        // the generic `Expr::Path` handling below ever sees the (otherwise ordinary-looking) left-
        // hand side. PR #165 final rereview remediation, A2 (§6.3): failing that, a bare 1-segment
        // *writable source lexical-owner* field (`Prop`/`State` on the Component this deferred view
        // was lexically written inside, e.g. `selected = true;` inside a `context_popup: view! { ..
        // }` block mutating the enclosing Component's own `#[state] selected: bool`) routes through
        // that owner's own weak-upgraded setter instead — `resolved_implicit_owner_setter`. Any
        // other assignment (a genuine local variable, `+=`-style compound assignment which `syn`
        // represents as `Expr::Binary` and never reaches here, ...) falls through to the default
        // recursive visit at the bottom, unchanged.
        if let syn::Expr::Assign(assign) = node {
            if let syn::Expr::Path(p) = assign.left.as_ref() {
                if let Some(ident) = p.path.get_ident() {
                    let name = ident.to_string();
                    if !self.is_shadowed(&name) && self.ctx.mutable_own_fields.contains(&name) {
                        if let EmitMode::WithSelf(self_tok) = self.mode {
                            self.visit_expr_mut(&mut assign.right);
                            let setter = format_ident!("set_{}", name);
                            let rhs = &assign.right;
                            *node = syn::parse_quote! { #self_tok.#setter(#rhs) };
                            return;
                        }
                    } else if !self.is_shadowed(&name)
                        && self.ctx.template_parent.is_some()
                        && self.ctx.template_bare_parent_fields.contains(&name)
                    {
                        // Component default templates retain the ordinary bare-property
                        // shorthand (`is_checked = !is_checked`).  The template closure is
                        // evaluated against its captured parent, so assignments must target that
                        // parent's generated setter just like an explicit `templated_parent`
                        // setter call.  Keep this schema-driven; no component/type names are
                        // involved in the lowering decision.
                        self.visit_expr_mut(&mut assign.right);
                        let rhs = &assign.right;
                        let setter = emit_template_setter_call(
                            &["templated_parent".to_string(), name],
                            self.ctx,
                            self.mode,
                            quote! { #rhs },
                        )
                        .expect("bare template parent assignment must lower to a setter call");
                        *node = syn::parse2(setter)
                            .expect("generated bare template parent setter parses");
                        return;
                    } else if let Some(setter) = self.resolved_implicit_owner_setter(&name) {
                        self.visit_expr_mut(&mut assign.right);
                        let rhs = &assign.right;
                        *node = syn::parse_quote! { #setter(#rhs) };
                        return;
                    }
                }
            }
        }
        // A bare 1-segment call callee (`record_unmount("...")`, `some_free_function()`) is
        // never how this DSL's own field-backed values are invoked (a viewmodel action always
        // reads `vm.save()` — a 2-segment `owner.method()` `syn::Expr::MethodCall`, an entirely
        // different node shape — never a bare `save()`), so `resolved_implicit_owner_field`'s
        // "unresolved bare name must be an implicit-owner field" fallback must never be applied to
        // one — unlike a `view!` DSL attribute value (`emit_expr`'s own `ViewExpr::Path` handling),
        // whose grammar structurally cannot contain a free-function call in this position at all,
        // this rewriter walks *arbitrary* raw Rust (`on_mount`/`on_unmount`/`on_update`/event
        // closure bodies), where a bare call to a genuine free function/helper is completely
        // ordinary and must be left untouched (found via a real regression: PR #165 review
        // remediation, A6's own `record_unmount("PopupContent")` test fixture, a plain free
        // function called directly inside a declarative popup's own `on_unmount`, was rewritten
        // into a bogus `__view_owner.record_unmount()` method call before this fix). A genuine
        // mutable-own-field or own-field callee (rare, but not nonsensical — e.g. a `Fn`-typed
        // prop field called bare) is still resolved normally; only the *implicit-owner* fallback
        // is withheld here. Call *arguments* are ordinary value positions and are rewritten as
        // usual, including through the implicit-owner fallback.
        if let syn::Expr::Call(call) = node {
            if let syn::Expr::Path(p) = call.func.as_ref() {
                if let Some(ident) = p.path.get_ident() {
                    let name = ident.to_string();
                    if !self.is_shadowed(&name) {
                        if let Some(value) = self.resolved_mutable_field_read(&name) {
                            *call.func = syn::parse_quote! { #value };
                        } else if let Some(base) = self.resolved_owner(&name) {
                            *call.func = syn::parse_quote! { #base };
                        }
                    }
                    for arg in call.args.iter_mut() {
                        self.visit_expr_mut(arg);
                    }
                    return;
                }
            }
        }
        // Setter calls in raw template closures use the same typed property bridge as reads.  A
        // generic `C` cannot invoke an inherent `set_<name>` method directly, so translate the
        // explicit templated-parent setter spelling into its compile-time keyed operation.  This
        // is metadata-driven by the property name supplied in the source expression; no target
        // type or control-name dispatch is involved.
        if let syn::Expr::MethodCall(call) = node {
            if let syn::Expr::Path(receiver) = call.receiver.as_ref() {
                if receiver.path.segments.len() == 1
                    && receiver.path.segments[0].ident == "templated_parent"
                    && call.method.to_string().starts_with("set_")
                    && (self.ctx.template_property_bounds.is_some()
                        || self.ctx.default_template_parent)
                {
                    let method = call.method.to_string();
                    let property = method.trim_start_matches("set_");
                    let key = crate::template_property_key(property);
                    if let Some(bounds) = &self.ctx.template_property_bounds {
                        bounds.borrow_mut().entry(key).or_insert(None);
                    }
                    for argument in &mut call.args {
                        self.visit_expr_mut(argument);
                    }
                    let args = &call.args;
                    if args.len() == 1 {
                        let value = args.first().expect("one setter argument");
                        if self.ctx.default_template_parent {
                            let receiver = match self.mode {
                                EmitMode::Construction => quote! { self },
                                EmitMode::WithSelf(self_token) => self_token.clone(),
                            };
                            let setter = format_ident!("set_{property}");
                            *node = syn::parse_quote! { #receiver.#setter(#value) };
                        } else {
                            let parent = self.ctx.template_parent.as_ref().expect(
                                "template parent binding is available for template expressions",
                            );
                            let receiver = match self.mode {
                                EmitMode::Construction => quote! { #parent },
                                EmitMode::WithSelf(self_token) => self_token.clone(),
                            };
                            let template_target = self
                                .ctx
                                .template_target
                                .clone()
                                .unwrap_or_else(|| quote! { C });
                            *node = syn::parse_quote! {
                                <#template_target as elwindui::core::ui::WritableTemplateProperty<#key>>::__template_set(&*#receiver, #value)
                            };
                        }
                        return;
                    }
                }
            }
        }
        // A raw Rust template expression spells a parent property as
        // `templated_parent.field`, which syn represents as `Expr::Field` rather than the DSL
        // `ViewExpr::Path` form handled by `emit_path_get`.  Lower it through the same typed
        // TemplateProperty bridge before recursively visiting the base expression; otherwise a
        // standalone event/lifecycle closure would try to access a field on the generic `C`
        // directly and lose both the compile-time bound and resync dependency.
        if let syn::Expr::Field(field) = node {
            if let syn::Expr::Path(base) = field.base.as_ref() {
                if base.path.segments.len() == 1
                    && base.path.segments[0].ident == "templated_parent"
                {
                    if let syn::Member::Named(member) = &field.member {
                        let path = vec!["templated_parent".to_string(), member.to_string()];
                        let value = emit_path_get(&path, self.ctx, self.mode);
                        *node = syn::parse2(value)
                            .expect("generated template parent field expression parses");
                        return;
                    }
                }
            }
            syn::visit_mut::visit_expr_field_mut(self, field);
            return;
        }
        if let syn::Expr::Path(p) = node {
            let segments: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if self.ctx.template_parent.is_some() && segments.len() == 1 && segments[0] == "self" {
                let parent =
                    self.ctx.template_parent.as_ref().expect(
                        "template parent binding is present for template closure rewriting",
                    );
                // Template lifecycle/event blocks historically used `self` as the borrowed
                // component receiver.  The shared template factory owns that receiver as an
                // `Rc`, so preserve the same reference semantics when rewriting raw `self` rather
                // than exposing the owning `Rc` to APIs that expect `&dyn UIElementExt`.
                *node = syn::parse_quote! { &*#parent };
                return;
            }
            if let [only] = segments.as_slice() {
                if self.is_shadowed(only) {
                    return;
                }
                if let Some(value) = self.resolved_mutable_field_read(only) {
                    *node = syn::parse_quote! { #value };
                    return;
                }
                if let Some(base) = self.resolved_owner(only) {
                    *node = syn::parse_quote! { #base };
                    return;
                }
                if self.ctx.template_parent.is_some()
                    && self.ctx.template_bare_parent_fields.contains(only)
                {
                    let path = ["templated_parent".to_string(), only.to_string()];
                    let value = emit_path_get(&path, self.ctx, self.mode);
                    *node = syn::parse2(value)
                        .expect("generated bare template parent expression parses");
                    return;
                }
                if let Some(value) = self.resolved_implicit_owner_field(only) {
                    *node = syn::parse_quote! { #value };
                }
                return;
            }
            if let [owner, field] = segments.as_slice() {
                if owner == "templated_parent"
                    && (self.ctx.template_property_bounds.is_some()
                        || self.ctx.default_template_parent)
                {
                    let value = emit_path_get(&segments, self.ctx, self.mode);
                    *node =
                        syn::parse2(value).expect("generated template parent expression parses");
                    return;
                }
                let base = self
                    .resolved_owner(owner)
                    .or_else(|| self.resolved_implicit_bindable_owner(owner));
                if let Some(base) = base {
                    let getter = format_ident!("{}", field);
                    *node = syn::parse_quote! { #base.#getter() };
                    return;
                }
            }
        }
        syn::visit_mut::visit_expr_mut(self, node);
    }

    fn visit_expr_macro_mut(&mut self, node: &mut syn::ExprMacro) {
        if let Some(mut arguments) = supported_macro_expr_arguments(node) {
            for argument in &mut arguments {
                if let syn::Expr::Assign(named) = argument {
                    self.visit_expr_mut(&mut named.right);
                } else {
                    self.visit_expr_mut(argument);
                }
            }
            // `format!("{field}!")`'s inline capture (RFC 2795) only ever sees whatever raw local
            // happens to be in scope at the exact point this call gets embedded — for a
            // component's own field that's never a real local (it's `self`-backed storage, or a
            // constructor parameter that's already been moved elsewhere by the time a second
            // element also needs it), so this used to compile only by accident (Issue #68 bug 5).
            // Naming the same resolved value as an *explicit* named argument
            // (`field = self.field()`) satisfies the placeholder directly — an explicit named
            // argument is checked before format_args! ever falls back to scope capture (verified:
            // `format!("{field}!", field = get())` needs no local named `field` at all) — without
            // touching the format string's own text.
            if is_format_macro(node) {
                if let Some(fmt) = arguments.first().and_then(expr_as_lit_str) {
                    let already_named: std::collections::HashSet<String> = arguments
                        .iter()
                        .filter_map(|argument| match argument {
                            syn::Expr::Assign(assign) => match assign.left.as_ref() {
                                syn::Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect();
                    for name in format_str_inline_idents(&fmt.value()) {
                        if already_named.contains(&name) {
                            continue;
                        }
                        let Some(value) = self
                            .resolved_mutable_field_read(&name)
                            .or_else(|| self.resolved_owner(&name))
                            .or_else(|| self.resolved_implicit_owner_field(&name))
                        else {
                            continue;
                        };
                        let ident = format_ident!("{}", name);
                        arguments.push(syn::parse_quote! { #ident = #value });
                    }
                }
            }
            node.mac.tokens = quote! { #(#arguments),* };
        }
    }
}

fn rewrite_view_closure_expr(
    mut expr: syn::Expr,
    closure_params: &[String],
    ctx: &ViewCtx,
    mode: &EmitMode,
) -> TokenStream {
    ViewClosureRewriter::new(closure_params, ctx, mode).visit_expr_mut(&mut expr);
    quote! { #expr }
}

fn rewrite_view_closure_block(
    mut block: syn::Block,
    closure_params: &[String],
    ctx: &ViewCtx,
    mode: &EmitMode,
) -> TokenStream {
    // `visit_block_mut` itself pushes a *further* nested scope for the block's own top-level
    // statements (see that override's own doc comment) — the scope seeded here by `new` is only
    // the closure/hook's own parameter scope, layered *outside* the block's own.
    ViewClosureRewriter::new(closure_params, ctx, mode).visit_block_mut(&mut block);
    quote! { #block }
}

/// Re-pushes every dynamic (non-callback, non-`Element`/`Closure`-valued) attribute of every
/// stored widget from current model state, calling `set_<attr>(value)` on its resolved type.
/// `#[two_way]` attributes (e.g. `TextArea`'s `text`) are resynced the same as any other — this
/// pushes model→widget; `emit_wiring`'s separate `set_on_<attr>_change` callback is what pushes
/// widget→model.
///
/// Collects every distinct property name `expr` references as `<owner>.<property>` (or
/// `<owner>.<property>(...)`) — walks the same shapes `view_expr_depends_on` tests one candidate
/// at a time, but gathers names instead of testing a single one. Needed by
/// `property_resync_methods_for`, which (unlike the `owner_info.fields`-driven code it replaces)
/// has no symbol-table-derived list of "every field `owner`'s type could have" to check candidates
/// against in the first place — see `ast::Attr::Bindable`'s doc comment for why that lookup can't
/// be relied on for a `#[bindable]` field. Unlike `view_expr_depends_on`, an opaque macro call
/// nested inside a plain `syn::Expr` (not the DSL's own recognized `t!(...)` sugar,
/// `ViewExpr::TFluent`, already handled below) contributes no name here — there is no property
/// *name* to collect from "this might depend on something", only from an actual `owner.property`
/// path.
/// PR #165 post-final rereview remediation, A9 (§9.3): whether `ctx.implicit_owner` names `owner`
/// as its own physical field (i.e. `owner == "__view_owner"`, the only case this ever matters for)
/// — the shared guard `collect_view_expr_owner_properties`/`view_expr_depends_on` use before
/// treating a *direct* bare source-field reference as belonging to `owner`'s own resync method.
fn implicit_owner_matches<'a>(ctx: &'a ViewCtx, owner: &str) -> Option<&'a ImplicitOwnerCtx> {
    ctx.implicit_owner
        .as_ref()
        .filter(|implicit| implicit.field_name == owner)
}

fn collect_view_expr_owner_properties(
    expr: &ViewExpr,
    ctx: &ViewCtx,
    owner: &str,
    out: &mut std::collections::BTreeSet<String>,
) {
    match expr {
        ViewExpr::Path(path) => match path.as_slice() {
            // PR #165 post-final rereview remediation, A9: a *direct* bare source field
            // (`TextBlock { text: label }`, no `vm.` qualification) inside a lowered `DeferredView`
            // is a dependency of `__view_owner`'s own resync method — canonicalized to
            // `(__view_owner, label)` — whenever `label` is one of the source Component's own
            // `reactive_fields`. Before this, only `[path_owner, path_property, ..]` (2-segment)
            // paths were ever recognized here at all, so a direct bare source field never got a
            // resync-method arm and therefore never live-updated while the popup stayed open.
            [field] => {
                if let Some(implicit) = implicit_owner_matches(ctx, owner) {
                    if implicit.reactive_fields.contains(field) {
                        out.insert(field.clone());
                    }
                }
            }
            [path_owner, path_property, ..] => {
                if path_owner == owner {
                    out.insert(path_property.clone());
                }
            }
            [] => {}
        },
        ViewExpr::TFluent(_, args) => {
            for (_, value) in args {
                collect_view_expr_owner_properties(value, ctx, owner, out);
            }
        }
        ViewExpr::Expr(expr) => {
            struct Collector<'a> {
                ctx: &'a ViewCtx,
                owner: &'a str,
                out: &'a mut std::collections::BTreeSet<String>,
            }
            impl<'ast> Visit<'ast> for Collector<'_> {
                fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                    let segments: Vec<_> = node.path.segments.iter().collect();
                    if segments.len() >= 2 && segments[0].ident == self.owner {
                        self.out.insert(segments[1].ident.to_string());
                    } else if segments.len() == 1 {
                        if let Some(implicit) = implicit_owner_matches(self.ctx, self.owner) {
                            let name = segments[0].ident.to_string();
                            if implicit.reactive_fields.contains(&name) {
                                self.out.insert(name);
                            }
                        }
                    }
                    syn::visit::visit_expr_path(self, node);
                }

                fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
                    if let Some(arguments) = supported_macro_expr_arguments(node) {
                        if is_format_macro(node) {
                            if let Some(fmt) = arguments.first().and_then(expr_as_lit_str) {
                                if let Some(implicit) = implicit_owner_matches(self.ctx, self.owner)
                                {
                                    for name in format_str_inline_idents(&fmt.value()) {
                                        if implicit.reactive_fields.contains(&name) {
                                            self.out.insert(name);
                                        }
                                    }
                                }
                            }
                        }
                        for argument in &arguments {
                            self.visit_expr(argument);
                        }
                    }
                }
            }
            let mut collector = Collector { ctx, owner, out };
            collector.visit_expr(expr);
        }
        ViewExpr::Element(_) | ViewExpr::Closure { .. } | ViewExpr::DeferredView(_) => {}
    }
}

fn supported_macro_expr_arguments(node: &syn::ExprMacro) -> Option<Vec<syn::Expr>> {
    let name = node.mac.path.segments.last()?.ident.to_string();
    if !matches!(name.as_str(), "format" | "format_args" | "vec") {
        return None;
    }
    use syn::parse::Parser as _;
    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
        .parse2(node.mac.tokens.clone())
        .ok()
        .map(|arguments| arguments.into_iter().collect())
}

/// Every `{ident}` / `{ident:spec}` inline-capture name (RFC 2795) in a `format!`/`format_args!`
/// literal format string, deduplicated in first-seen order — skips `{{`/`}}` escapes and
/// positional/empty (`{}`, `{0}`) placeholders, none of which name anything. `format!` captures
/// only ever bind a bare single-segment identifier (`format!("{a.b}")` isn't valid Rust), so this
/// never needs to return anything but plain names. Issue #68 bug 5: without this, a field
/// reference hidden inside a format string's own text was invisible to every dependency scanner
/// below and to `ViewClosureRewriter`, which left the *generated* code relying on an ambient local
/// variable happening to still be in scope at that exact point — broke as soon as a second
/// element's own construction also needed (and consumed) a same-named local.
fn format_str_inline_idents(value: &str) -> Vec<String> {
    let mut idents = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                continue;
            }
            let mut name = String::new();
            while matches!(chars.peek(), Some(next) if *next != '}' && *next != ':') {
                name.push(chars.next().unwrap());
            }
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
            }
            let is_ident = !name.is_empty()
                && name.starts_with(|c: char| c.is_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_ident && seen.insert(name.clone()) {
                idents.push(name);
            }
        } else if c == '}' && chars.peek() == Some(&'}') {
            chars.next();
        }
    }
    idents
}

/// The `syn::LitStr` inside a bare string-literal `syn::Expr`, if that's what `expr` is — used to
/// pull a `format!`/`format_args!` call's own format-string argument out for
/// `format_str_inline_idents` to scan, never anything else.
fn expr_as_lit_str(expr: &syn::Expr) -> Option<&syn::LitStr> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) => Some(s),
        _ => None,
    }
}

/// Whether `node` is specifically a `format!`/`format_args!` call — narrower than
/// `supported_macro_expr_arguments`'s gate (which also allows `vec!`), since only these two ever
/// have inline-capture format-string semantics worth scanning for.
fn is_format_macro(node: &syn::ExprMacro) -> bool {
    node.mac
        .path
        .segments
        .last()
        .is_some_and(|s| matches!(s.ident.to_string().as_str(), "format" | "format_args"))
}

/// Whether `name` is a source-Component field this hidden Component's implicit-owner fallback may
/// treat as reactive (`ImplicitOwnerCtx::reactive_fields`) — PR #165 post-final rereview
/// remediation, A9.
fn is_implicit_reactive_field(ctx: &ViewCtx, name: &str) -> bool {
    ctx.implicit_owner
        .as_ref()
        .is_some_and(|implicit| implicit.reactive_fields.contains(name))
}

/// Whether `name` is a source-Component `#[bindable]` field reachable through this hidden
/// Component's implicit owner (`ImplicitOwnerCtx::bindable_fields`) — PR #165 post-final rereview
/// remediation, A9.
fn is_implicit_bindable_owner(ctx: &ViewCtx, name: &str) -> bool {
    ctx.implicit_owner
        .as_ref()
        .is_some_and(|implicit| implicit.bindable_fields.contains(name))
}

fn view_expr_has_reactive_dependency(expr: &ViewExpr, ctx: &ViewCtx) -> bool {
    match expr {
        ViewExpr::Path(path) => match path.as_slice() {
            [field] => {
                ctx.mutable_own_fields.contains(field) || is_implicit_reactive_field(ctx, field)
            }
            [owner, ..] => {
                (ctx.default_template_parent && owner == "templated_parent")
                    || ctx.bindable_owners.contains(owner)
                    || is_implicit_bindable_owner(ctx, owner)
            }
            [] => false,
        },
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, value)| view_expr_has_reactive_dependency(value, ctx)),
        ViewExpr::Expr(expr) => {
            struct Collector<'a> {
                ctx: &'a ViewCtx,
                found: bool,
            }
            impl<'ast> Visit<'ast> for Collector<'_> {
                fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                    let segments: Vec<_> = node.path.segments.iter().collect();
                    if segments.len() == 1 {
                        let name = segments[0].ident.to_string();
                        if self.ctx.mutable_own_fields.contains(&name)
                            || is_implicit_reactive_field(self.ctx, &name)
                        {
                            self.found = true;
                        }
                    } else if segments.len() >= 2 {
                        let owner = segments[0].ident.to_string();
                        if (self.ctx.default_template_parent && owner == "templated_parent")
                            || self.ctx.bindable_owners.contains(&owner)
                            || is_implicit_bindable_owner(self.ctx, &owner)
                        {
                            self.found = true;
                        }
                    }
                    syn::visit::visit_expr_path(self, node);
                }

                fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
                    if let Some(arguments) = supported_macro_expr_arguments(node) {
                        if is_format_macro(node) {
                            if let Some(fmt) = arguments.first().and_then(expr_as_lit_str) {
                                if format_str_inline_idents(&fmt.value()).iter().any(|name| {
                                    self.ctx.mutable_own_fields.contains(name)
                                        || is_implicit_reactive_field(self.ctx, name)
                                }) {
                                    self.found = true;
                                }
                            }
                        }
                        for argument in &arguments {
                            self.visit_expr(argument);
                        }
                    } else {
                        self.found = true;
                    }
                }
            }
            let mut collector = Collector { ctx, found: false };
            collector.visit_expr(expr);
            collector.found
        }
        ViewExpr::Element(_) | ViewExpr::Closure { .. } | ViewExpr::DeferredView(_) => false,
    }
}

/// Builds one `fn __resync_<owner>(&self, property: &'static str)` per `bind_owners` entry — a
/// `match` arm per distinct `<owner>.<property>` path this component's view body actually
/// references (`collect_view_expr_owner_properties`), string-keyed rather than the per-viewmodel
/// `XProperty` enum the code this replaces matched on (`ast::Attr::Bindable`'s doc comment explains
/// why: this component's own codegen has no name for that enum to write a match arm against when
/// `owner`'s concrete type is declared by a separate macro invocation). `include_refresh` mirrors
/// the pre-existing composed/non-composed difference at each of this function's two call sites: a
/// non-composed component needs an explicit `self.__refresh_dynamic_regions()` after each
/// property's own statements; a composed one's `new()` already covers this elsewhere.
fn property_resync_methods_for(
    bind_owners: &[syn::Ident],
    plan: &[PlannedNode],
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    include_refresh: bool,
    // Whether `plan`'s own root (`plan.last()`) is a shape/host-composition root with no separate
    // `self.#binding` field of its own — see `emit_wiring`/`emit_resync`'s matching doc comments.
    root_is_self: bool,
) -> TokenStream {
    let root_binding = plan.last().map(|r| r.binding.clone());
    let lazy_leaves = collect_lazy_leaves(plan);
    bind_owners
        .iter()
        .map(|owner_ident| {
            let owner_name = owner_ident.to_string();
            let mut properties: std::collections::BTreeSet<String> = Default::default();
            for node in plan {
                for attribute in &node.attributes {
                    if attribute.kind != AssignmentKind::Once {
                        collect_view_expr_owner_properties(
                            &attribute.value,
                            ctx,
                            &owner_name,
                            &mut properties,
                        );
                    }
                }
                // `plan` is already flat — `plan_dynamic_entry` plans every eager branch/arm's own
                // content straight into the same `out` before pushing the region's own marker node,
                // so this single pass also reaches nested dynamic regions without recursing. Only
                // the condition/value/collection expression itself is missing from the attribute
                // scan above; a bind owner referenced *only* there (never in a sibling attribute)
                // must still get a `__resync_<owner>` arm, or its `PropertyChanged` notifications
                // never reach `__refresh_dynamic_regions()` (issue #58).
                match node.dynamic.as_ref() {
                    Some(DynamicPlan::If { condition, .. }) => {
                        collect_view_expr_owner_properties(
                            condition,
                            ctx,
                            &owner_name,
                            &mut properties,
                        );
                    }
                    Some(DynamicPlan::Match { value, .. }) => {
                        collect_view_expr_owner_properties(
                            value,
                            ctx,
                            &owner_name,
                            &mut properties,
                        );
                    }
                    Some(DynamicPlan::For { collection, .. }) => {
                        collect_view_expr_owner_properties(
                            collection,
                            ctx,
                            &owner_name,
                            &mut properties,
                        );
                    }
                    None => {}
                }
            }
            for (_, leaf) in &lazy_leaves {
                for attribute in &leaf.attributes {
                    if attribute.kind != AssignmentKind::Once {
                        collect_view_expr_owner_properties(
                            &attribute.value,
                            ctx,
                            &owner_name,
                            &mut properties,
                        );
                    }
                }
            }
            let method = format_ident!("__resync_{}", owner_ident);
            let branches: TokenStream = properties
                .iter()
                .map(|property_name| {
                    let mut statements = TokenStream::new();
                    for node in plan {
                        let self_is_node =
                            root_is_self && root_binding.as_ref() == Some(&node.binding);
                        emit_resync(
                            node,
                            ctx,
                            from,
                            table,
                            ResyncFilter::Property(&owner_name, property_name),
                            &mut statements,
                            self_is_node,
                        );
                    }
                    for (cache_field, leaf) in &lazy_leaves {
                        emit_lazy_branch_resync(
                            cache_field,
                            leaf,
                            ctx,
                            from,
                            table,
                            ResyncFilter::Property(&owner_name, property_name),
                            &mut statements,
                        );
                    }
                    let refresh =
                        include_refresh.then(|| quote! { self.__refresh_dynamic_regions(); });
                    quote! { #property_name => { #statements #refresh } }
                })
                .collect();
            quote! {
                fn #method(&self, property: &'static str) {
                    match property {
                        #branches
                        _ => {}
                    }
                }
            }
        })
        .collect()
}

/// When `filter` is present, only attributes that statically reference that owner/property are
/// emitted.  Expression macros that the DSL cannot inspect are deliberately conservative: they
/// remain attached to that owner's notifications rather than risking a stale UI value.
fn view_expr_depends_on(expr: &ViewExpr, ctx: &ViewCtx, owner: &str, property: &str) -> bool {
    // PR #165 post-final rereview remediation, A9: when `owner` names the hidden Component's own
    // implicit lexical owner (`__view_owner`), a *direct* bare source field (`[field]`, no `vm.`
    // qualification) depends on `(owner, property)` too — canonicalizing the same dependency
    // identity `collect_view_expr_owner_properties`/`view_expr_has_reactive_dependency` already use
    // — provided `field` is actually one of the source Component's own `reactive_fields` (never for
    // an ordinary Component with no implicit owner at all, where this is always `None`).
    let implicit_direct_field_matches = |field: &str| {
        field == property
            && implicit_owner_matches(ctx, owner)
                .is_some_and(|implicit| implicit.reactive_fields.contains(field))
    };
    match expr {
        ViewExpr::Path(path) => {
            if owner.is_empty() {
                matches!(path.as_slice(), [path_property] if path_property == property)
                    || (ctx.default_template_parent
                        && matches!(path.as_slice(), [path_owner, path_property, ..]
                            if path_owner == "templated_parent" && path_property == property))
            } else {
                matches!(path.as_slice(), [path_owner, path_property, ..] if path_owner == owner && path_property == property)
                    || matches!(path.as_slice(), [field] if implicit_direct_field_matches(field))
            }
        }
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, value)| view_expr_depends_on(value, ctx, owner, property)),
        ViewExpr::Expr(expr) => {
            struct Collector<'a> {
                ctx: &'a ViewCtx,
                owner: &'a str,
                property: &'a str,
                found: bool,
                opaque_macro: bool,
            }
            impl<'ast> Visit<'ast> for Collector<'_> {
                fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                    let segments: Vec<_> = node.path.segments.iter().collect();
                    if (self.owner.is_empty()
                        && segments.len() == 1
                        && segments[0].ident == self.property)
                        || (segments.len() >= 2
                            && segments[0].ident == self.owner
                            && segments[1].ident == self.property)
                        || (self.owner.is_empty()
                            && self.ctx.default_template_parent
                            && segments.len() >= 2
                            && segments[0].ident == "templated_parent"
                            && segments[1].ident == self.property)
                        || (segments.len() == 1
                            && segments[0].ident == self.property
                            && implicit_owner_matches(self.ctx, self.owner).is_some_and(
                                |implicit| implicit.reactive_fields.contains(self.property),
                            ))
                    {
                        self.found = true;
                    }
                    syn::visit::visit_expr_path(self, node);
                }

                fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
                    if let Some(arguments) = supported_macro_expr_arguments(node) {
                        if self.owner.is_empty() && is_format_macro(node) {
                            if let Some(fmt) = arguments.first().and_then(expr_as_lit_str) {
                                if format_str_inline_idents(&fmt.value())
                                    .iter()
                                    .any(|name| name == self.property)
                                {
                                    self.found = true;
                                }
                            }
                        } else if is_format_macro(node) {
                            if let Some(fmt) = arguments.first().and_then(expr_as_lit_str) {
                                if format_str_inline_idents(&fmt.value()).iter().any(|name| {
                                    name == self.property
                                        && implicit_owner_matches(self.ctx, self.owner).is_some_and(
                                            |implicit| {
                                                implicit.reactive_fields.contains(self.property)
                                            },
                                        )
                                }) {
                                    self.found = true;
                                }
                            }
                        }
                        for argument in &arguments {
                            self.visit_expr(argument);
                        }
                    } else {
                        self.opaque_macro = true;
                    }
                }
            }
            let mut collector = Collector {
                ctx,
                owner,
                property,
                found: false,
                opaque_macro: false,
            };
            collector.visit_expr(expr);
            collector.found || collector.opaque_macro
        }
        ViewExpr::Element(_) | ViewExpr::Closure { .. } | ViewExpr::DeferredView(_) => false,
    }
}

#[derive(Clone, Copy)]
enum ResyncFilter<'a> {
    All,
    Property(&'a str, &'a str),
}

fn emit_resync(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    filter: ResyncFilter<'_>,
    out: &mut TokenStream,
    self_is_node: bool,
) {
    emit_resync_with_receiver(node, ctx, from, table, filter, out, self_is_node, None);
}

// Same as `emit_resync`, but for a node that isn't reachable as `self`/`self.#binding` at all —
// a lazy-once `if`/`match` branch leaf, whose only live handle is the `Rc<T>` sitting in its own
// `RefCell<Option<Rc<T>>>` cache field once materialized (see `lazy_branch_cache_ident`). The
// caller is expected to have already unwrapped that cache and bound the clone to `receiver_override`
// under an `if let Some(..)` guard — this function itself has no opinion on cache presence, it just
// emits setter calls against whatever receiver expression it's given.
fn emit_resync_with_receiver(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    filter: ResyncFilter<'_>,
    out: &mut TokenStream,
    self_is_node: bool,
    receiver_override: Option<TokenStream>,
) {
    if node.type_path == ENVIRONMENT_SCOPE_MARKER {
        emit_environment_scope_resync(node, ctx, out);
        return;
    }
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = if ctx.is_template_storage() {
        EmitMode::WithSelf(quote! { this })
    } else {
        EmitMode::WithSelf(quote! { self })
    };
    let info = resolve_context_info(ctx, from, table, &node.type_path);
    // See `emit_wiring`'s matching `widget_binding`/`self_is_node` doc comment — a shape/host-
    // composition root has no separately-stored `self.#binding` field; `self` itself already *is*
    // the tree node.
    let receiver = ctx.node_receiver(binding, self_is_node, receiver_override);
    // `resync()` is its own function, a separate lexical scope from `new()` — the `use` already
    // injected alongside construction (`emit_construction`'s `builtin_trait_use`, or
    // `build_virtual_value`'s own inline copy for a virtual builtin) doesn't carry over here, so
    // any hand-written native or virtual builtin whose setters are shared-trait-only needs its own
    // copy of the same import for this function's own `self.#binding.#setter(..)` calls below.
    //
    // `info.is_none()` (external/builtin, no local `TypeInfo`): a resync'd theme value is already
    // fully typed (resolved from a real theme definition, never a DSL hex-string literal needing
    // `wrap_prop_value`'s `.into()` conversion), so `emit_field_setter_call`'s existing
    // `declaring_type`-less fallback (a plain `#receiver.#setter(#args)` call — no `@set` needed
    // here) is already the right shape; it only needs *some* ancestor's trait in scope, and without
    // `TypeInfo` this function has no ancestor chain to name specific ones from — same glob-import
    // reasoning as `emit_external_construction`'s own `use elwindui::ui::*;`.
    out.extend(if info.is_none() {
        quote! {
            #[allow(unused_imports)]
            use elwindui::ui::*;
        }
    } else {
        builtin_trait_use(&node.type_path, info)
    });
    // A deferred field inherited from `UIElement` itself (`margin`/`width`/`height`/... —
    // `resolve_effective_fields`'s own doc comment) is set through `UIElementExt`, a shared trait
    // method — needed here for the same reason as `emit_construction`'s own matching `use` (this
    // function is a separate scope, so that one doesn't carry over). Harmless when unused, same as
    // `builtin_trait_use` itself; picked up by the main per-attribute loop below (`margin`/`width`/
    // `height`/... are now ordinary `field_types` members, no separate resync path needed for them).
    out.extend(quote! {
        #[allow(unused_imports)]
        use elwindui::core::ui::UIElementExt as _;
    });

    // Every codegen-*generated* setter (a virtual builtin's own `elwindui_core::ui` setters, or a
    // `has_view` component's own generated `set_<name>` — both the deferred and the mutable-
    // required kind, see `is_settable_field`) takes its non-Copy argument *by value*. Only a
    // hand-written native's shared-trait setter (`Button`/`TextArea`/`MenuItem`/`MenuBarItem`'s
    // `&str`-taking `set_text`/etc.) wants the `&(..)`-wrapped reference the `else` branch below
    // still uses.
    let node_uses_owned_setters = info.is_some_and(|i| i.is_virtual_builtin || i.has_view);
    for attribute in &node.attributes {
        let name = &attribute.name;
        let expr = &attribute.value;
        if !matches!(filter, ResyncFilter::All)
            && (attribute.kind == AssignmentKind::Once
                || (attribute.kind == AssignmentKind::Normal
                    && !view_expr_has_reactive_dependency(expr, ctx)))
        {
            continue;
        }
        if info.is_some_and(|i| !i.field_types.contains_key(name)) {
            continue;
        }
        if name.starts_with("on_") {
            continue;
        }
        if matches!(
            expr,
            ViewExpr::Element(_) | ViewExpr::Closure { .. } | ViewExpr::DeferredView(_)
        ) {
            continue;
        }
        match filter {
            ResyncFilter::All => {}
            ResyncFilter::Property(owner, property)
                if !view_expr_depends_on(expr, ctx, owner, property) =>
            {
                continue;
            }
            ResyncFilter::Property(_, _) => {}
        }
        // `#[onetime]` fields (`Window`'s own `left`/`top`/`width`/`height`,
        // docs/specs/ui_spec.md#window) are one-time initial-placement/size setters,
        // applied once at construction (`build_component_setters`) — never re-pushed here.
        // Re-applying them on every resync() would fight the OS window manager, snapping a
        // user-dragged/resized window back to its originally-declared value the next time
        // *anything else* triggers resync() (e.g. `TabView`'s `on_select` wiring). The live native
        // frame is available separately via `Window`'s own `left()`/`top()`/`width()`/`height()`
        // getters for whoever wants current state. Declarative (`info.onetime_fields`, from this
        // field's own `#[onetime]` attribute in the builtin's own `#[class]` declaration) rather than a hardcoded
        // type-name + field-name tuple — see `ast::Attr::Onetime`'s own doc comment.
        if info.is_some_and(|i| i.onetime_fields.contains(name)) {
            continue;
        }
        // A `view`-having (`has_view`) target's own no-initializer field ordinarily has no
        // `set_<name>` at all (unlike every hand-written builtin, which by convention always
        // defines one, even a no-op, for the "blanket resync" rule above to call generically) — so
        // resyncing it here would be calling a method that simply doesn't exist. `is_settable_field`
        // carves out the two cases that *do* get a real setter despite having no initializer
        // (deferred `Option<T>` fields, and required `prop` fields — see its own doc comment), which
        // this loop should keep resyncing normally.
        if info.is_some_and(|i| {
            i.has_view
                && i.param_fields.iter().any(|(n, _)| n == name)
                && !is_settable_field(
                    i,
                    &node.type_path,
                    name,
                    i.field_types.get(name).map(String::as_str).unwrap_or(""),
                )
        }) {
            continue;
        }

        let setter = format_ident!("set_{name}");
        // The resync value itself is never `Option`-wrapped (only construction-time args are, per
        // the shape's own `Option<..>` convention for "may be absent"), so copy-ness is judged on
        // the stripped inner type — `Option<String>`'s runtime value here is a plain `String`.
        let field_ty = info
            .and_then(|i| i.field_types.get(name))
            .map(String::as_str);
        if info.is_some_and(|info| is_semantic_brush_property(info, name)) {
            let raw = emit_expr(expr, ctx, &self_mode);
            let environment = semantic_brush_resync_environment(node, ctx);
            let is_text_style = info.is_some_and(|info| info.text_style_fields.contains(name));
            let (set, clear) = if info.is_none() && !is_text_style {
                let props_macro = dsl_props_macro_path(&node.type_path, None);
                let name_ident = format_ident!("{name}");
                (
                    quote! {
                        #props_macro!(
                            @set #receiver, #name_ident, __elwindui_semantic_brush
                        );
                    },
                    quote! {
                        #props_macro!(@clear #receiver, #name_ident);
                    },
                )
            } else {
                (
                    emit_field_setter_call(
                        name,
                        &node.type_path,
                        &setter,
                        quote! { Some(__elwindui_semantic_brush) },
                        &receiver,
                        from,
                        table,
                    ),
                    emit_field_clear_call(name, &node.type_path, &receiver, from, table),
                )
            };
            out.extend(emit_semantic_brush_resolution(raw, environment, set, clear));
            continue;
        }
        if let Some(coerced) = coerce_color_literal(strip_option(field_ty.unwrap_or("")).0, expr) {
            // `virtual_builtin_resync_value` would otherwise splice the raw (uncoerced) literal
            // straight into `Some(..)`/the bare setter argument — this mirrors its own
            // `Option<..>`-wrapping decision, just starting from the already-coerced value instead
            // of a fresh `emit_expr` call.
            let value = if strip_option(field_ty.unwrap_or("")).1 {
                quote! { Some(#coerced) }
            } else {
                coerced
            };
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                value,
                &receiver,
                from,
                table,
            ));
            continue;
        }
        let value = emit_expr(expr, ctx, &self_mode);
        if info.is_some_and(|info| info.text_style_fields.contains(name)) {
            // The style-owner API consumes the six font values. `foreground` keeps its Option so
            // a caller can distinguish an explicit local brush from an unset inherited value.
            // Do this before the generic native-control branch, which normally borrows non-Copy
            // values for `&str`-style setters and would make `FontFamily`/`Brush` fail to compile.
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                value,
                &receiver,
                from,
                table,
            ));
            continue;
        }
        if info.is_none() {
            // External (no local `TypeInfo`): none of `is_copy`/`Vec<..>`/`node_uses_owned_setters`
            // below can be decided (`field_ty` is always `None`, so every one of those checks would
            // silently default to the *last*, `&(..)`-wrapping `else` branch regardless of the
            // property's real type — the exact bug this branch exists to avoid: a `bool`/`f32`/enum
            // resync value would get wrongly borrowed as if it were `String`-shaped). `@set` already
            // carries the right wrapping decision from the `#[prop]` declaration itself — reuse it
            // here instead of re-deriving anything.
            let props_macro = dsl_props_macro_path(&node.type_path, None);
            let name_ident = format_ident!("{name}");
            // A bare reference to one of `synthesize_external_base_fields`'s synthesized fields
            // (`ty.contains('!')` — a type-position macro invocation, never a real Rust type
            // spelling in this codebase) carries whatever shape the *base* declared it as — which,
            // for an `Option<T>`-declared prop (`Control`'s own `padding`, Refs #90), is `Option<T>`
            // itself, not the bare `T` every other `@set` call site here already supplies (a DSL
            // literal, a themed value, ...). `@set`'s own `wrap_prop_value` expects that bare-`T`
            // shape uniformly — it doesn't (and, being a `#[class]`-side macro with no visibility
            // into which call sites forward an own field, structurally can't) special-case this —
            // so unwrap here, at the one call site that can actually be in this position: skip the
            // push entirely on `None` (this field was never set, so the base keeps its own default —
            // the same "absent = leave default" convention `deferred_own_names`'s own setters
            // already use elsewhere) rather than trying to call a `clear_<name>` that, for most
            // props (anything not theme-capable), was never hand-written to begin with.
            // `Option::from` — not a direct `if let Some(..) = #value` — because `#value` here is
            // always `self.<field>` for one of *these* fields specifically (never a bare `T`, see
            // `synthesize_external_base_fields`'s doc comment: every synthesized field is `Option<T>`
            // only if that's genuinely what the base declared; this branch only ever fires for one
            // of them), so a plain `Option<T>` match would already do — `Option::from` is kept anyway
            // for symmetry with nothing else needing it, and to fail loudly (a real type error) if
            // that invariant is ever violated instead of silently miscompiling.
            if bare_own_field_type(expr, ctx).is_some_and(|ty| ty.contains('!')) {
                let environment = semantic_brush_resync_environment(node, ctx);
                out.extend(quote! {
                    if let ::std::option::Option::Some(__v) = ::std::option::Option::from(#value) {
                        #props_macro!(
                            @set_with_environment #receiver, #name_ident, __v, #environment
                        );
                    }
                });
            } else {
                let environment = semantic_brush_resync_environment(node, ctx);
                out.extend(quote! {
                    #props_macro!(
                        @set_with_environment #receiver, #name_ident, #value, #environment
                    );
                });
            }
            continue;
        }
        let is_copy = field_ty.is_some_and(|ty| is_copy_type(strip_option(ty).0));
        if is_copy {
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                value,
                &receiver,
                from,
                table,
            ));
        } else if field_ty.is_some_and(|ty| strip_option(ty).0.starts_with("Vec<")) {
            // A `Vec<T>` field's real setter always takes it *by value* everywhere in this
            // framework (for example `GridImpl::set_rows`/`set_columns`), so
            // this isn't gated on `node_uses_owned_setters` — `.to_vec()` coerces a DSL
            // array-literal value into an owned `Vec<T>` and is a harmless no-op clone when the
            // value is already one (e.g. `vm.documents()`).
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                quote! { (#value).to_vec() },
                &receiver,
                from,
                table,
            ));
        } else if node_uses_owned_setters {
            // Every codegen-generated `set_*` setter (a virtual builtin's `TextBlockImpl::set_text`/
            // `ShapeImpl::set_fill`/..., or a `has_view` component's own generated `set_<name>` —
            // `is_settable_field`'s two cases) takes its non-Copy argument *by value* — never by
            // reference like a hand-written native's shared-trait setters (`&str`) — so this
            // branch derives the right owned shape purely from the field's own declared type
            // string (`virtual_builtin_resync_value`, despite the name — the conversion rules are
            // identical for both) instead of the `&(..)`-wrapping the `else` branch below uses.
            let converted = virtual_builtin_resync_value(field_ty.unwrap_or(""), value);
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                converted,
                &receiver,
                from,
                table,
            ));
        } else if field_ty.is_some_and(|ty| is_ui_element_type(strip_option(ty).0)) {
            // A hand-written native's `dyn UIElement`-typed setter (`Window::set_content`) takes
            // its argument *by value*, unlike the `&str`-taking convention the blanket `else`
            // branch below assumes for every other hand-written-native field — and, same as
            // `build_component_args`/`build_virtual_value`/`build_component_setters`'s identically
            // -named branches, a bare-forwarded own field whose own type is some concrete element
            // still needs `.into_node()` to satisfy that `dyn UIElement` target at all.
            let source_type = bare_own_field_type(expr, ctx).unwrap_or_default();
            let converted = into_node_if_needed(value, &source_type, from, table);
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                converted,
                &receiver,
                from,
                table,
            ));
        } else {
            out.extend(emit_field_setter_call(
                name,
                &node.type_path,
                &setter,
                quote! { &(#value) },
                &receiver,
                from,
                table,
            ));
        }
    }
}

/// Converts a resync value into a virtual-builtin setter's by-value parameter shape, derived
/// purely from the field's own declared type string (`TypeInfo::field_types`, sourced from
/// the builtin `#[class]` declarations) — no per-widget-type or per-field-name table to maintain: any current or
/// future virtual builtin's non-Copy field is covered automatically as long as its declared type
/// matches one of these two shapes, mirroring `build_virtual_value`'s own construction-time
/// conversions (a `Vec<T>`-typed field is handled earlier, by the caller's own type-agnostic
/// `.to_vec()` branch — see that call site's doc comment):
/// - `Option<String>` (`Shape::fill`/`stroke`, `TextBlock::color`) — the real setter takes an owned
///   `Option<String>`, so a supplied (non-absent, since this is only reached when the attribute was
///   actually given) value is `Some`-wrapped and `.to_string()`-coerced.
/// - bare `String` (`TextBlock::text`) — the real setter takes an owned `String`.
///
/// Every other non-Copy shape that can appear in `field_types` (a `fn(..)` callback, an `Element`/
/// `Closure` value) never reaches this function — `emit_resync`'s own loop already filters those
/// out before computing `is_copy`. Any *Copy* field (`f32`/`bool`/an enum, `Option<f32>` included —
/// see `is_copy_type`'s own doc comment) is handled by the caller's separate `is_copy` branch and
/// never reaches here either, since a virtual-builtin setter always stores those bare regardless of
/// whether the field is optional at the DSL level.
fn virtual_builtin_resync_value(ty: &str, value: TokenStream) -> TokenStream {
    let ty = ty.trim();
    if let Some(inner) = ty.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        if inner.trim() == "String" {
            quote! { Some((#value).to_string()) }
        } else {
            quote! { Some(#value) }
        }
    } else if ty == "String" {
        quote! { (#value).to_string() }
    } else {
        quote! { #value }
    }
}

fn emit_expr(expr: &ViewExpr, ctx: &ViewCtx, mode: &EmitMode) -> TokenStream {
    match expr {
        ViewExpr::Expr(e) => rewrite_view_closure_expr(e.clone(), &[], ctx, mode),
        ViewExpr::Path(path) => {
            let path: &[String] = path.as_slice();
            // A bare reference to the closure's own bound parameter (e.g. `doc` in
            // `render_content: |doc| DocumentView { doc: doc }`) passes the value straight
            // through — it isn't a `vm`-style field with a generated getter, so it must be
            // handled before `emit_path_get` (which has no 1-segment path shape).
            if let [only] = path {
                if ctx.closure_param.as_deref() == Some(only.as_str()) {
                    // The closure parameter itself is always a reference (`&Rc<T>`, `&_` —
                    // `emit_closure_value`'s deliberately-typed closure param), but a passthrough
                    // like `doc: doc` needs to hand an *owned* `Rc<T>` to the target constructor —
                    // `.clone()` is the cheap `Rc` refcount bump that bridges the two.
                    let ident = format_ident!("{}", only);
                    return quote! { #ident.clone() };
                }
                // A bare reference to one of this component's own `#[param]` fields, used as a
                // value in its own right (e.g. `RoundedPanel`'s `TextBlock { text: label }`) rather than
                // as the owner of a `.getter()` call — the field/constructor-parameter itself, not
                // `emit_path_get`'s `vm.something`-shaped 2-segment machinery.
                if ctx.own_fields.contains_key(only) {
                    // A mutable-required own field (`ViewCtx::mutable_own_fields`,
                    // `generate_view`'s `mutable_required_names`) is Cell/RefCell-backed, not a
                    // bare field — `self.<name>` alone would hand back the cell itself, not its
                    // value. Only matters in `WithSelf` mode (`resync()`/a stored closure); at
                    // `Construction` time the value is still the raw, not-yet-cell-wrapped
                    // constructor-argument local, read the ordinary bare way.
                    if let EmitMode::WithSelf(self_tok) = mode {
                        if ctx.mutable_own_fields.contains(only) {
                            let ident = format_ident!("{}", only);
                            let ty_str = ctx.own_fields.get(only).unwrap();
                            return if is_copy_type(ty_str) {
                                quote! { #self_tok.#ident.get() }
                            } else {
                                quote! { #self_tok.#ident.borrow().clone() }
                            };
                        }
                    }
                    return mode.owner_tokens(only);
                }
                if ctx.template_parent.is_some() && ctx.template_bare_parent_fields.contains(only) {
                    let parent_path = ["templated_parent".to_string(), only.to_string()];
                    return emit_path_get(&parent_path, ctx, mode);
                }
                // Issue #162 §3.11: inside a lowered deferred view (`ViewDef::implicit_owner`), a
                // bare name that is a known-*readable* field of the source lexical owner Component
                // (PR #165 final rereview remediation, A2 — `ImplicitOwnerCtx::readable_fields`,
                // the same schema-membership check `resolved_implicit_owner_field` applies to raw
                // Rust blocks, so a DSL attribute value and a raw `on_*` block agree on exactly
                // which bare names fall back to the owner) falls back to the implicit weak lexical
                // owner — `selected_item` becomes semantically `__view_owner.selected_item`,
                // generalizing `emit_path_get`'s existing 2-segment `owner.field` machinery (weak
                // upgrade included, via `owner_value_tokens`/`ctx.weak_bindable_owners`) rather than
                // duplicating it. Only reached once the closure-param/own-field checks above have
                // already ruled out a local binding, preserving ordinary lexical shadowing.
                if let Some(owner) = &ctx.implicit_owner {
                    if owner.readable_fields.contains(only) {
                        let owner_path = [owner.field_name.clone(), only.clone()];
                        return emit_path_get(&owner_path, ctx, mode);
                    }
                }
            }
            emit_path_get(path, ctx, mode)
        }
        ViewExpr::TFluent(key, args) => {
            let arg_pairs = args.iter().map(|(name, value)| {
                let value_tokens = emit_expr(value, ctx, mode);
                quote! { (#name, elwindui::i18n::FluentValue::from(#value_tokens)) }
            });
            quote! { elwindui::i18n::t(#key, &[ #(#arg_pairs),* ]) }
        }
        ViewExpr::Closure { .. } => {
            panic!("a closure (`|param| ...`) cannot itself be used as a value expression here")
        }
        ViewExpr::Element(_) => {
            panic!("an element (`Type {{ .. }}`) cannot itself be used as a value expression here")
        }
        ViewExpr::DeferredView(_) => {
            panic!(
                "a deferred view (`view! {{ .. }}`) cannot itself be used as a value expression \
                 here — it is only valid as a whole attribute value (e.g. `context_popup: view! \
                 {{ .. }}`), emitted via its own dedicated ViewTemplate construction path, never \
                 nested inside a larger expression"
            )
        }
    }
}

/// A resolved `["vm", "content"]`-style path -> `vm.content()` (construction) /
/// `self.vm.content()` (with self). A viewmodel action (`vm.save`) resolves through this exact
/// same 2-segment shape — there is no separate `Command`-wrapper indirection to fold in.
fn owner_value_tokens(ctx: &ViewCtx, mode: &EmitMode, owner: &str) -> TokenStream {
    if ctx.default_template_parent && owner == "templated_parent" {
        return match mode {
            EmitMode::Construction => quote! { self },
            EmitMode::WithSelf(self_tok) => self_tok.clone(),
        };
    }
    if owner == "templated_parent" {
        if let EmitMode::WithSelf(self_tok) = mode {
            // Callback/resync bodies are emitted inside a closure that owns a cloned parent.  Use
            // that closure-local receiver instead of spelling the factory's outer binding again;
            // otherwise the closure would move the factory's only parent `Rc` and later
            // subscriptions/cleanup would fail with E0382.
            return self_tok.clone();
        }
        if let Some(parent) = &ctx.template_parent {
            return quote! { #parent };
        }
    }
    let base = mode.owner_tokens(owner);
    if ctx.weak_bindable_owners.contains(owner) {
        let upgrade_panic_message =
            format!("weak owner `{owner}` was dropped before its template instance");
        quote! {
            #base.upgrade().expect(#upgrade_panic_message)
        }
    } else {
        base
    }
}

/// PR #165 post-final rereview remediation, A8: the shared resolver behind both `emit_path_get`'s
/// and `emit_setter`'s 2-segment `owner.field` path handling. Ordinarily `owner` is simply this
/// generated Component's own field/bindable-owner, and `owner_value_tokens` blindly emits
/// `self.#owner`/`#owner` — valid because `owner` really is a struct field there (`ctx.own_fields`
/// covers every one of this Component's own literal fields, `Prop`/`State`/`Param`/`Computed`/
/// `Environment` alike — see its own construction site's `own_fields.extend(..)`).
///
/// Inside a lowered `DeferredView` hidden Component, `owner` may instead be a *source*-lexical-owner
/// `#[bindable]` field (`vm.label`, `vm.save`) that the hidden Component itself never physically
/// declares — its only real field is `__view_owner`. Before this fix, `emit_path_get`/`emit_setter`
/// called `owner_value_tokens` unconditionally, so `vm.label` was emitted as `self.vm.label()` on a
/// struct with no `vm` field at all — syntactically valid tokens (so `assert_valid_rust`'s
/// `syn::parse2`-only check missed it) but a genuine `rustc` compile error (`no field \`vm\` on type
/// ..`). This resolver checks `ctx.own_fields` first (preserving every existing case unchanged,
/// including `ControlTemplate`'s own `templated_parent` and `__view_owner` itself, both real fields
/// of their own hidden Component); only when `owner` is *not* a real field of the current Component
/// but *is* a known source-Component `#[bindable]` field (`ImplicitOwnerCtx::bindable_fields`) does
/// it bridge through the source lexical owner instead: `__view_owner.upgrade().vm()`. Any other,
/// genuinely unresolved `owner` falls through to the original `owner_value_tokens` call unchanged
/// (preserving whatever diagnostic/behavior that already produced).
fn path_owner_value_tokens(ctx: &ViewCtx, mode: &EmitMode, owner: &str) -> TokenStream {
    if ctx.own_fields.contains_key(owner) {
        return owner_value_tokens(ctx, mode, owner);
    }
    if let Some(implicit) = &ctx.implicit_owner {
        if implicit.bindable_fields.contains(owner) {
            let source = owner_value_tokens(ctx, mode, &implicit.field_name);
            let getter = format_ident!("{}", owner);
            return quote! { #source.#getter() };
        }
    }
    owner_value_tokens(ctx, mode, owner)
}

fn emit_path_get(path: &[String], ctx: &ViewCtx, mode: &EmitMode) -> TokenStream {
    match path {
        [owner, field, rest @ ..]
            if owner == "templated_parent"
                && ctx.template_property_bounds.is_some()
                && !ctx.default_template_parent =>
        {
            let base = path_owner_value_tokens(ctx, mode, owner);
            let key = crate::template_property_key(field);
            if let Some(bounds) = &ctx.template_property_bounds {
                bounds.borrow_mut().entry(key).or_insert(None);
            }
            let template_target = ctx.template_target.clone().unwrap_or_else(|| quote! { C });
            let mut value = quote! {
                <#template_target as elwindui::core::ui::TemplateProperty<#key>>::__template_get(&*#base)
            };
            for segment in rest {
                let ident = format_ident!("{segment}");
                value = quote! { #value.#ident() };
            }
            value
        }
        [owner, field] => {
            if ctx.template_parent.is_some() && ctx.template_bare_parent_fields.contains(owner) {
                let parent_value =
                    emit_path_get(&["templated_parent".to_string(), owner.clone()], ctx, mode);
                let getter = format_ident!("{}", field);
                return quote! { (#parent_value).#getter() };
            }
            let base = path_owner_value_tokens(ctx, mode, owner);
            let getter = format_ident!("{}", field);
            if ctx.default_template_parent
                && owner == "templated_parent"
                && ctx.template_base_fields.contains(field)
            {
                return quote! { (#base).base.#getter() };
            }
            quote! { #base.#getter() }
        }
        other => panic!(
            "unsupported path shape after bind resolution: `{}`",
            other.join(".")
        ),
    }
}

fn emit_template_setter_call(
    path: &[String],
    ctx: &ViewCtx,
    mode: &EmitMode,
    value: TokenStream,
) -> Option<TokenStream> {
    let [owner, field] = path else {
        return None;
    };
    let base = path_owner_value_tokens(ctx, mode, owner);
    if owner == "templated_parent" && ctx.template_property_bounds.is_some() {
        let key = crate::template_property_key(field);
        let template_target = ctx.template_target.clone().unwrap_or_else(|| quote! { C });
        Some(quote! {
            <#template_target as elwindui::core::ui::WritableTemplateProperty<#key>>::__template_set(
                &*#base,
                #value,
            )
        })
    } else {
        let setter = format_ident!("set_{}", field);
        Some(quote! { #base.#setter(#value) })
    }
}

/// Concrete-target lifecycle/event body lowering with a component-default bare-property schema.
pub(crate) fn emit_template_event_closure_body_for_target_with_fields(
    body: &ClosureBody,
    closure_params: &[String],
    parent: &syn::Ident,
    property_bounds: &Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
    template_target: TokenStream,
    bare_parent_fields: HashSet<String>,
) -> TokenStream {
    let ctx = ViewCtx {
        closure_param: None,
        own_fields: HashMap::new(),
        mutable_own_fields: HashSet::new(),
        bindable_owners: HashSet::new(),
        weak_bindable_owners: HashSet::new(),
        default_template_parent: false,
        template_base_fields: HashSet::new(),
        implicit_owner: None,
        target: format_ident!("__ElwinduiTemplateTarget"),
        template_parent: Some(parent.clone()),
        template_property_bounds: Some(property_bounds.clone()),
        template_target: Some(template_target),
        template_bare_parent_fields: bare_parent_fields,
        storage: ViewStorage::Template {
            environment: format_ident!("__environment"),
            refresh_cell: format_ident!("__elwindui_template_refresh_cell"),
        },
    };
    emit_on_event_closure_body(body, closure_params, &ctx, &EmitMode::Construction)
}

/// Collects typed parent property keys using the same recursive AST traversal used by the normal
/// View backend.  This is intentionally generic over expression shape (including raw Rust
/// expressions and Fluent arguments) so a standalone frontend does not need a second visitor.
pub(crate) fn collect_template_property_keys(expr: &ViewExpr, out: &mut BTreeSet<u64>) {
    match expr {
        ViewExpr::Path(path) => {
            if path.len() >= 2 && path.first().is_some_and(|name| name == "templated_parent") {
                out.insert(crate::template_property_key(&path[1]));
            }
        }
        ViewExpr::TFluent(_, args) => {
            for (_, value) in args {
                collect_template_property_keys(value, out);
            }
        }
        ViewExpr::Expr(expr) => {
            collect_template_rust_expr_property_keys(expr, out);
        }
        ViewExpr::Element(element) => {
            for attribute in &element.attributes {
                collect_template_property_keys(&attribute.value, out);
            }
            for child in &element.children {
                match child {
                    ChildEntry::Literal(element) => {
                        for attribute in &element.attributes {
                            collect_template_property_keys(&attribute.value, out);
                        }
                    }
                    ChildEntry::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        collect_template_property_keys(condition, out);
                        for entry in then_branch.iter().chain(else_branch) {
                            if let ChildEntry::Literal(element) = entry {
                                for attribute in &element.attributes {
                                    collect_template_property_keys(&attribute.value, out);
                                }
                            }
                        }
                    }
                    ChildEntry::Match { value, arms } => {
                        collect_template_property_keys(value, out);
                        for arm in arms {
                            for entry in &arm.body {
                                if let ChildEntry::Literal(element) = entry {
                                    for attribute in &element.attributes {
                                        collect_template_property_keys(&attribute.value, out);
                                    }
                                }
                            }
                        }
                    }
                    ChildEntry::For {
                        collection, body, ..
                    } => {
                        collect_template_property_keys(collection, out);
                        for entry in body {
                            if let ChildEntry::Literal(element) = entry {
                                for attribute in &element.attributes {
                                    collect_template_property_keys(&attribute.value, out);
                                }
                            }
                        }
                    }
                    ChildEntry::Ref(_) => {}
                }
            }
        }
        ViewExpr::Closure { params: _, body } => {
            collect_template_closure_property_keys(body, out);
        }
        ViewExpr::DeferredView(_) => {}
    }
}

/// Collects `templated_parent.<property>` dependencies from an arbitrary Rust expression.  The
/// structural expression visitor is shared by all template frontends; the standalone adapter uses
/// this helper for the `ViewExpr::Expr` variant instead of maintaining a second path visitor.
pub(crate) fn collect_template_rust_expr_property_keys(expr: &syn::Expr, out: &mut BTreeSet<u64>) {
    struct Collector<'a> {
        out: &'a mut BTreeSet<u64>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_expr_field(&mut self, node: &'ast syn::ExprField) {
            if let syn::Expr::Path(base) = node.base.as_ref() {
                if base.path.segments.len() == 1
                    && base.path.segments[0].ident == "templated_parent"
                {
                    if let syn::Member::Named(field) = &node.member {
                        self.out
                            .insert(crate::template_property_key(&field.to_string()));
                    }
                }
            }
            syn::visit::visit_expr_field(self, node);
        }
    }
    Collector { out }.visit_expr(expr);
}

/// Collects `templated_parent` dependencies from a template lifecycle/event closure body using
/// the same expression visitor as property values.  Keeping this operation here means
/// template_view! does not need a lifecycle-specific dependency scanner of its own.
pub(crate) fn collect_template_closure_property_keys(body: &ClosureBody, out: &mut BTreeSet<u64>) {
    match body {
        ClosureBody::Expr(expr) => collect_template_property_keys(expr, out),
        ClosureBody::Element(element) => {
            for attribute in &element.attributes {
                collect_template_property_keys(&attribute.value, out);
            }
            for child in &element.children {
                if let ChildEntry::Literal(element) = child {
                    collect_template_closure_element_property_keys(element, out);
                } else {
                    collect_template_child_property_keys(child, out);
                }
            }
        }
        ClosureBody::Block(block) => collect_template_rust_block_property_keys(block, out),
    }
}

fn collect_template_closure_element_property_keys(element: &ElementNode, out: &mut BTreeSet<u64>) {
    for attribute in &element.attributes {
        collect_template_property_keys(&attribute.value, out);
    }
    for child in &element.children {
        collect_template_child_property_keys(child, out);
    }
}

fn collect_template_child_property_keys(child: &ChildEntry, out: &mut BTreeSet<u64>) {
    match child {
        ChildEntry::Literal(element) => {
            collect_template_closure_element_property_keys(element, out)
        }
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_template_property_keys(condition, out);
            for child in then_branch.iter().chain(else_branch) {
                collect_template_child_property_keys(child, out);
            }
        }
        ChildEntry::Match { value, arms } => {
            collect_template_property_keys(value, out);
            for arm in arms {
                for child in &arm.body {
                    collect_template_child_property_keys(child, out);
                }
            }
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            collect_template_property_keys(collection, out);
            for child in body {
                collect_template_child_property_keys(child, out);
            }
        }
        ChildEntry::Ref(_) => {}
    }
}

/// Collects template-parent paths from a lifecycle block.  This is the block counterpart of
/// `collect_template_rust_expr_property_keys` and deliberately delegates all lexical traversal to
/// `syn::visit`.
pub(crate) fn collect_template_rust_block_property_keys(
    block: &syn::Block,
    out: &mut BTreeSet<u64>,
) {
    struct Collector<'a> {
        out: &'a mut BTreeSet<u64>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_expr_field(&mut self, node: &'ast syn::ExprField) {
            if let syn::Expr::Path(base) = node.base.as_ref() {
                if base.path.segments.len() == 1
                    && base.path.segments[0].ident == "templated_parent"
                {
                    if let syn::Member::Named(field) = &node.member {
                        self.out
                            .insert(crate::template_property_key(&field.to_string()));
                    }
                }
            }
            syn::visit::visit_expr_field(self, node);
        }
    }
    Collector { out }.visit_block(block);
}

/// Collects the template-parent properties that are used as write endpoints.  Reads and writes
/// intentionally have separate collections: a generic standalone factory needs only
/// `TemplateProperty<KEY>` for a read, while `<=>` and `templated_parent.set_<name>(..)` must add
/// the stronger `WritableTemplateProperty<KEY>` bound.  This visitor is metadata-only; it never
/// creates a runtime property registry or changes the lowering emitted by the rewriter.
pub(crate) fn collect_template_writable_property_keys(
    body: &crate::ast::ViewBody,
    lets: &[crate::ast::LetBinding],
    on_mount: Option<&syn::Block>,
    on_unmount: Option<&syn::Block>,
    on_update: Option<&crate::ast::OnUpdateHook>,
    bare_parent_fields: &HashSet<String>,
    out: &mut BTreeSet<u64>,
) {
    struct Collector<'a> {
        out: &'a mut BTreeSet<u64>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if let syn::Expr::Path(receiver) = node.receiver.as_ref()
                && receiver.path.segments.len() == 1
                && receiver.path.segments[0].ident == "templated_parent"
            {
                let method = node.method.to_string();
                if let Some(property) = method.strip_prefix("set_") {
                    self.out.insert(crate::template_property_key(property));
                }
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    fn collect_expr_writes(expr: &syn::Expr, out: &mut BTreeSet<u64>) {
        Collector { out }.visit_expr(expr);
    }
    fn collect_block_writes(block: &syn::Block, out: &mut BTreeSet<u64>) {
        Collector { out }.visit_block(block);
    }

    fn collect_body_writes(
        body: &crate::ast::ViewBody,
        bare_parent_fields: &HashSet<String>,
        out: &mut BTreeSet<u64>,
    ) {
        for attribute in &body.attributes {
            collect_attribute_writes(attribute, bare_parent_fields, out);
        }
        for (_, _, value) in &body.attached {
            collect_view_expr_writes(value, bare_parent_fields, out);
        }
        for child in &body.children {
            collect_child_writes(child, bare_parent_fields, out);
        }
    }

    fn collect_attribute_writes(
        attribute: &crate::ast::ViewAttribute,
        bare_parent_fields: &HashSet<String>,
        out: &mut BTreeSet<u64>,
    ) {
        if attribute.kind == crate::ast::AssignmentKind::TwoWay {
            if let crate::ast::ViewExpr::Path(path) = &attribute.value {
                let property = match path.as_slice() {
                    [property] if bare_parent_fields.contains(property) => Some(property),
                    [owner, property, ..] if owner == "templated_parent" => Some(property),
                    _ => None,
                };
                if let Some(property) = property {
                    out.insert(crate::template_property_key(property));
                }
            }
        }
        collect_view_expr_writes(&attribute.value, bare_parent_fields, out);
    }

    fn collect_view_expr_writes(
        expr: &crate::ast::ViewExpr,
        bare_parent_fields: &HashSet<String>,
        out: &mut BTreeSet<u64>,
    ) {
        match expr {
            crate::ast::ViewExpr::Expr(expression) => collect_expr_writes(expression, out),
            crate::ast::ViewExpr::TFluent(_, args) => {
                for (_, value) in args {
                    collect_view_expr_writes(value, bare_parent_fields, out);
                }
            }
            crate::ast::ViewExpr::Closure { body, .. } => {
                collect_closure_writes(body, bare_parent_fields, out);
            }
            crate::ast::ViewExpr::Element(element) => {
                collect_element_writes(element, bare_parent_fields, out);
            }
            crate::ast::ViewExpr::DeferredView(_) | crate::ast::ViewExpr::Path(_) => {}
        }
    }

    fn collect_closure_writes(
        body: &crate::ast::ClosureBody,
        bare_parent_fields: &HashSet<String>,
        out: &mut BTreeSet<u64>,
    ) {
        match body {
            crate::ast::ClosureBody::Expr(expr) => {
                collect_view_expr_writes(expr, bare_parent_fields, out)
            }
            crate::ast::ClosureBody::Element(element) => {
                collect_element_writes(element, bare_parent_fields, out)
            }
            crate::ast::ClosureBody::Block(block) => collect_block_writes(block, out),
        }
    }

    fn collect_element_writes(
        element: &crate::ast::ElementNode,
        bare_parent_fields: &HashSet<String>,
        out: &mut BTreeSet<u64>,
    ) {
        for attribute in &element.attributes {
            collect_attribute_writes(attribute, bare_parent_fields, out);
        }
        for (_, _, value) in &element.attached {
            collect_view_expr_writes(value, bare_parent_fields, out);
        }
        for child in &element.children {
            collect_child_writes(child, bare_parent_fields, out);
        }
    }

    fn collect_child_writes(
        child: &crate::ast::ChildEntry,
        bare_parent_fields: &HashSet<String>,
        out: &mut BTreeSet<u64>,
    ) {
        match child {
            crate::ast::ChildEntry::Literal(element) => {
                collect_element_writes(element, bare_parent_fields, out)
            }
            crate::ast::ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            } => {
                collect_view_expr_writes(condition, bare_parent_fields, out);
                for child in then_branch.iter().chain(else_branch) {
                    collect_child_writes(child, bare_parent_fields, out);
                }
            }
            crate::ast::ChildEntry::Match { value, arms } => {
                collect_view_expr_writes(value, bare_parent_fields, out);
                for arm in arms {
                    for child in &arm.body {
                        collect_child_writes(child, bare_parent_fields, out);
                    }
                }
            }
            crate::ast::ChildEntry::For {
                collection, body, ..
            } => {
                collect_view_expr_writes(collection, bare_parent_fields, out);
                for child in body {
                    collect_child_writes(child, bare_parent_fields, out);
                }
            }
            crate::ast::ChildEntry::Ref(_) => {}
        }
    }

    collect_body_writes(body, bare_parent_fields, out);
    for binding in lets {
        collect_element_writes(&binding.element, bare_parent_fields, out);
    }
    if let Some(block) = on_mount {
        collect_block_writes(block, out);
    }
    if let Some(block) = on_unmount {
        collect_block_writes(block, out);
    }
    if let Some(hook) = on_update {
        collect_block_writes(&hook.block, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builtins (`Window`/`VerticalLayout`/`TextArea`/etc.) only resolve when their shape modules
    /// (`crate::builtin_modules`) are part of the symbol table — `compile_dir`/`generate_from_source`
    /// do this automatically, but a test building its own table directly needs to opt in explicitly.
    fn build_symbol_table_with_builtins(modules: &[Module]) -> SymbolTable {
        let all: Vec<Module> = modules
            .iter()
            .cloned()
            .chain(crate::test_builtin_modules())
            .collect();
        build_symbol_table(&all)
    }

    #[test]
    fn qualified_external_paths_use_authored_type_and_defining_crate_macro_paths() {
        let type_path = "some_alias::widgets::Thing";

        assert_eq!(
            dsl_props_macro_path(type_path, None).to_string(),
            "some_alias :: __elwindui_props_Thing"
        );
        assert_eq!(
            dsl_concrete_type_path(type_path, None).to_string(),
            "some_alias :: widgets :: Thing"
        );
        assert_eq!(
            dsl_ext_path(type_path, None).to_string(),
            "some_alias :: widgets :: ThingExt"
        );
        assert_eq!(
            dsl_construct_path(type_path, None).to_string(),
            "some_alias :: widgets :: Thing :: construct"
        );
    }

    #[test]
    fn local_and_builtin_paths_keep_their_existing_resolution_rules() {
        assert_eq!(
            dsl_props_macro_path("crate::widgets::Thing", None).to_string(),
            "crate :: __elwindui_props_Thing"
        );
        assert_eq!(
            dsl_concrete_type_path("crate::widgets::Thing", None).to_string(),
            "crate :: widgets :: Thing"
        );
        assert_eq!(
            dsl_ext_path("crate::widgets::Thing", None).to_string(),
            "crate :: widgets :: ThingExt"
        );
        assert_eq!(
            dsl_props_macro_path("TextBlock", None).to_string(),
            "elwindui :: core :: __elwindui_props_TextBlock"
        );
        assert_eq!(
            dsl_concrete_type_path("TextBlock", None).to_string(),
            "elwindui :: ui :: TextBlock"
        );
        assert_eq!(
            dsl_ext_path("TextBlock", None).to_string(),
            "elwindui :: core :: ui :: TextBlockExt"
        );
    }

    fn minimal_component_def(
        name: &str,
        base: Option<&str>,
        fields: Vec<FieldDef>,
    ) -> ComponentDef {
        ComponentDef {
            name: name.to_string(),
            base: base.map(str::to_string),
            base_path: None,
            fields,
            methods: Vec::new(),
            embedded: false,
            sealed: false,
            native: false,
            is_abstract: false,
            text_style: false,
            content_field: None,
        }
    }

    fn param_field(name: &str, ty: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: ty.to_string(),
            kind: FieldKind::Param,
            attrs: Vec::new(),
            initializer: None,
        }
    }

    #[test]
    fn builtin_content_metadata_is_scalar_or_collection() {
        let modules = crate::test_builtin_modules();
        let table = build_symbol_table(&modules);
        let module = modules.first().expect("builtin fixture module");
        let control = table.resolve(module, "Control").expect("Control metadata");
        assert_eq!(control.content_field.as_deref(), Some("visual_root"));
        assert!(
            control
                .field_types
                .get("visual_root")
                .is_some_and(|ty| is_ui_element_type(ty)),
            "{:#?}",
            control.field_types
        );
        let content_control = table
            .resolve(module, "ContentControl")
            .expect("ContentControl metadata");
        assert_eq!(content_control.content_field.as_deref(), Some("content"));
        assert!(
            content_control
                .field_types
                .get("content")
                .is_some_and(|ty| is_ui_element_type(ty))
        );
        let layout = table.resolve(module, "Layout").expect("Layout metadata");
        assert_eq!(layout.content_field.as_deref(), Some("children"));
        assert_eq!(
            layout.field_types.get("children").map(String::as_str),
            Some("UIElementCollection")
        );
    }

    #[test]
    fn derived_content_metadata_overrides_control_visual_root() {
        let module = multi_item_module(&[
            TestItem::Component(
                Some("Control"),
                r#"
                #[content(children)]
                struct CustomTabView {
                    children: Vec<std::rc::Rc<dyn UIElement>>,

                    body: view! {
                        VerticalLayout { }
                    },
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct Host {
                    body: view! {
                        CustomTabView {
                            TextBlock { text: "tab" }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let info = table
            .resolve(&module, "CustomTabView")
            .expect("derived component metadata");
        assert_eq!(info.content_field.as_deref(), Some("children"));
        assert!(
            info.field_types
                .get("children")
                .is_some_and(|ty| ty.trim_start().starts_with("Vec<"))
        );
        let generated = generate_module(&module, &table);
        assert_valid_rust("derived_content_override", &generated);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("CustomTabView :: new (vec !"),
            "nested children must lower through the derived collection: {rendered}"
        );
    }

    #[test]
    fn template_parent_path_keeps_inherited_writable_property_metadata() {
        let module = multi_item_module(&[
            TestItem::Component(
                Some("Control"),
                r#"
                struct TemplateBase {
                    #[prop]
                    value: String,
                    body: view! {
                        TextBlock { text: value }
                    },
                }
                "#,
            ),
            TestItem::Component(
                Some("TemplateBase"),
                r#"
                struct TemplateDerived {
                    template: template_view! {
                        value: templated_parent.value
                        TextBlock { text: templated_parent.value }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let derived = table
            .resolve(&module, "TemplateDerived")
            .expect("derived template metadata");
        assert_eq!(
            derived.declaring_types.get("value").map(String::as_str),
            Some("TemplateBase")
        );
        assert!(
            derived
                .effective_fields
                .iter()
                .any(|field| field.name == "value" && field.kind == FieldKind::Prop),
            "an explicit templated_parent path must retain the inherited property: {:#?}",
            derived.effective_fields
        );

        let generated = generate_module(&module, &table);
        assert_valid_rust("template_parent_inherited_writable_property", &generated);
        let rendered = generated.to_string();
        let key = crate::template_property_key("value");
        assert!(
            rendered.contains(&format!("WritableTemplateProperty < {key}u64 >")),
            "the inherited property must expose the writable template capability: {rendered}"
        );
        assert!(
            rendered.contains("TemplateBaseExt :: set_value"),
            "the writable bridge must delegate to the declaring base setter: {rendered}"
        );
    }

    #[test]
    fn composed_component_methods_keep_generic_override_metadata() {
        let module = crate::test_module(&[(
            Some("VerticalLayout"),
            r#"
                struct MetadataBridgeProbe {
                    body: view! {
                        Rectangle { }
                    },
                }
            "#,
            Some(
                r#"
                    impl MetadataBridgeProbe {
                        #[overridable]
                        fn marker(&self) -> bool { true }
                    }
                "#,
            ),
        )])
        .expect("component with an overridable method should parse");
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("composed_component_override_metadata", &generated);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("# [overridable] fn marker")
                || rendered.contains("#[overridable] fn marker"),
            "component metadata must reach the generated class impl: {rendered}"
        );
    }

    #[test]
    fn nested_control_scalar_content_uses_the_generic_setter() {
        let src = r#"
        struct Host {
            body: view! {
                VerticalLayout {
                    Control {
                        TextBlock { text: "content" }
                    }
                }
            },
        }
        "#;
        let module = crate::test_module(&[(None, src, None)]).expect("nested Control should parse");
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("nested_control_scalar_content", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("set_visual_root"), "{rendered}");
        assert!(!rendered.contains("set_children"), "{rendered}");
    }

    /// PR #169 review remediation, round 4, T-R4-6 (AD-R4-8): `generate_component`'s own-field
    /// membership must come only from `component_public_shape(source_component, None)` — an
    /// inherited field (present in the effective/flattened `c` this function's other logic still
    /// uses, absent from `source_component.fields`) must stay outside the shape, while real
    /// generation still forwards/stores it exactly as it did before this round's refactor. Built
    /// directly against `generate_component` with hand-crafted source/effective `ComponentDef`s
    /// (the same pattern `embedded_attribute_is_the_builtin_boundary_within_builtin_module`, just
    /// below, already uses) rather than through the full macro/validate pipeline — a *view-less*
    /// Component inheriting another Component with no `view` of its own is not expressible through
    /// the real DSL frontend at all (`validate.rs` rejects it: "must declare one composing over
    /// ..."), so this is the only way to exercise `generate_component`'s own inherited-field
    /// fallback boundary directly.
    #[test]
    fn t_r4_6_inherited_field_stays_outside_shape_but_real_generation_is_unchanged() {
        let source_derived = minimal_component_def(
            "TR46Derived",
            Some("TR46Base"),
            vec![param_field("own_value", "i32")],
        );
        let effective_derived = minimal_component_def(
            "TR46Derived",
            Some("TR46Base"),
            vec![
                param_field("base_value", "i32"),
                param_field("own_value", "i32"),
            ],
        );

        let shape = crate::component_frontend::component_public_shape(&source_derived, None);
        assert!(
            shape
                .constructor_params
                .iter()
                .any(|(n, _)| n == "own_value"),
            "{:?}",
            shape.constructor_params
        );
        assert!(
            !shape
                .constructor_params
                .iter()
                .any(|(n, _)| n == "base_value"),
            "the shape must never contain the inherited field: {:?}",
            shape.constructor_params
        );

        let table = build_symbol_table(&[]);
        let generated = generate_component(&source_derived, &effective_derived, &table).to_string();
        let real_ctor = generated
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        assert!(
            real_ctor.contains("base_value"),
            "real generation must still include the inherited field, unaffected by the \
             source-local shape: {real_ctor}"
        );
        assert!(
            real_ctor.contains("own_value"),
            "real generation must still include the derived's own field: {real_ctor}"
        );
        assert!(generated.contains("pub fn base_value"), "{generated}");
        assert!(generated.contains("pub fn own_value"), "{generated}");
    }

    /// Issue #68 bug 5's underlying scanner: `{{`/`}}` escapes, positional/empty (`{}`/`{0}`)
    /// placeholders, and `{ident:spec}` format specs must not be reported as captured names — only
    /// bare `{ident}` (or `{ident:spec}`) ones — and a name repeated across the string must be
    /// reported once, not once per occurrence (an `arguments.push` per occurrence would otherwise
    /// emit a "duplicate named argument" `rustc` error in `ViewClosureRewriter`).
    #[test]
    fn format_str_inline_idents_finds_only_real_named_captures() {
        assert_eq!(
            format_str_inline_idents("{volume}%"),
            vec!["volume".to_string()]
        );
        assert_eq!(
            format_str_inline_idents("{{literal braces}} and {field:>5}"),
            vec!["field".to_string()]
        );
        assert_eq!(
            format_str_inline_idents("{} and {0} but not {field}"),
            vec!["field".to_string()]
        );
        assert_eq!(
            format_str_inline_idents("{field} appears twice: {field}"),
            vec!["field".to_string()]
        );
        assert!(format_str_inline_idents("no placeholders here").is_empty());
    }

    #[test]
    fn embedded_attribute_is_the_builtin_boundary_within_builtin_module() {
        // `#[embedded]` has no current-syntax spelling at all (`component_frontend.rs`'s real
        // frontend never recognizes the attribute name — see `ComponentDef::embedded`'s own doc
        // comment) — built directly as `ComponentDef` struct literals instead, the same way
        // `testdata.rs` builds the real builtins' `embedded`/`native` flags.
        fn minimal_component(name: &str, embedded: bool) -> ComponentDef {
            ComponentDef {
                name: name.to_string(),
                base: None,
                base_path: None,
                fields: Vec::new(),
                methods: Vec::new(),
                embedded,
                sealed: false,
                native: false,
                is_abstract: false,
                text_style: false,
                content_field: None,
            }
        }
        let module = Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: vec![
                Item::Component(minimal_component("EmbeddedShape", true)),
                Item::Component(minimal_component("OrdinaryComponent", false)),
            ],
            // `Module::is_builtin` only authorizes `#[embedded]`; it must not by itself turn every
            // declaration in the source into a builtin.
            is_builtin: true,
            allows_external_builtins: false,
        };

        let table = build_symbol_table(&[module.clone()]);
        assert!(table.resolve(&module, "EmbeddedShape").unwrap().is_builtin);
        assert!(
            !table
                .resolve(&module, "OrdinaryComponent")
                .unwrap()
                .is_builtin
        );
    }

    /// Actions can't be declared in the DSL text form's `viewmodel` (only `#[observable]`/
    /// `#[computed]` can); a viewmodel with actions is always built via the Rust-native
    /// `attr_frontend` frontend (`mod { struct .. impl .. }`) instead, same as the real
    /// `#[elwindui::viewmodel]` macro — see `attr_frontend::viewmodel_def_from_item_mod`. `path:
    /// Vec::new()` matches the DSL's own crate-root placement (`parse_module`'s modules are also
    /// always `path: []`), so `use crate::NotepadViewModel;` elsewhere resolves against it exactly
    /// the same way.
    fn viewmodel_module_from_rust(src: &str) -> Module {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse as valid Rust");
        let def = crate::attr_frontend::viewmodel_def_from_item_mod(&item_mod)
            .expect("should build a ViewModelDef");
        Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: vec![Item::ViewModel(def)],
            ..Default::default()
        }
    }

    /// Builds one `Module` combining a `#[elwindui::viewmodel]`-style `mod` and a
    /// `#[elwindui::component]`-style `struct`, mirroring how one old `parser::parse_module` call
    /// could declare a `viewmodel` and a `component` together in one source blob.
    fn viewmodel_and_component_module(
        vm_src: &str,
        base: Option<&str>,
        struct_src: &str,
    ) -> Module {
        let item_mod: syn::ItemMod = syn::parse_str(vm_src).expect("mod should parse");
        let vm_def = crate::attr_frontend::viewmodel_def_from_item_mod(&item_mod)
            .expect("viewmodel should build");
        let item_struct: syn::ItemStruct = syn::parse_str(struct_src).expect("struct should parse");
        let (component_def, view_def) =
            crate::component_frontend::component_and_view_from_item_struct(
                base.map(str::to_string),
                &item_struct,
            )
            .expect("component should build");
        let mut items = vec![Item::ViewModel(vm_def)];
        items.extend(crate::component_frontend::component_module_items(
            component_def,
            view_def,
        ));
        Module {
            path: Vec::new(),
            uses: Vec::new(),
            items,
            is_builtin: false,
            allows_external_builtins: false,
        }
    }

    /// One entry in a [`multi_item_module`] fixture list — either a `#[elwindui::viewmodel]`-style
    /// `mod` source or a `#[elwindui::component]`-style `struct` source (with its `inherits` base,
    /// if any, passed separately since the bare `struct` text carries no macro attribute).
    enum TestItem<'a> {
        ViewModel(&'a str),
        Enum(&'a str),
        Component(Option<&'a str>, &'a str),
    }

    /// Generalizes [`viewmodel_and_component_module`] to an arbitrary ordered mix of `viewmodel`/
    /// `enum`/`component` declarations, mirroring how one old `parser::parse_module` call could
    /// declare several top-level items together in one source blob.
    fn multi_item_module(items: &[TestItem]) -> Module {
        let mut out = Vec::new();
        for item in items {
            match item {
                TestItem::ViewModel(src) => {
                    let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse");
                    let def = crate::attr_frontend::viewmodel_def_from_item_mod(&item_mod)
                        .expect("viewmodel should build");
                    out.push(Item::ViewModel(def));
                }
                TestItem::Enum(src) => {
                    let item_enum: syn::ItemEnum = syn::parse_str(src).expect("enum should parse");
                    let def = crate::component_frontend::enum_def_from_item_enum(&item_enum)
                        .expect("enum should build");
                    out.push(Item::Enum(def));
                }
                TestItem::Component(base, src) => {
                    let item_struct: syn::ItemStruct =
                        syn::parse_str(src).expect("struct should parse");
                    let (component_def, view_def) =
                        crate::component_frontend::component_and_view_from_item_struct(
                            base.map(|b| b.to_string()),
                            &item_struct,
                        )
                        .expect("component should build");
                    out.extend(crate::component_frontend::component_module_items(
                        component_def,
                        view_def,
                    ));
                }
            }
        }
        Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: out,
            is_builtin: false,
            allows_external_builtins: false,
        }
    }

    /// Builds a `Module` containing exactly one `#[elwindui::component]`-style `struct` — the
    /// single-component case of [`multi_item_module`], kept separate (rather than merged into a
    /// combined `viewmodel_and_component_module`/`multi_item_module` call) whenever a test needs its
    /// `viewmodel`/`component` `Module`s to stay independently `generate_module`-able, mirroring how
    /// `parse_module` used to be called once per source blob rather than once for everything.
    fn component_module(base: Option<&str>, struct_src: &str) -> Module {
        let item_struct: syn::ItemStruct = syn::parse_str(struct_src).expect("struct should parse");
        let (component_def, view_def) =
            crate::component_frontend::component_and_view_from_item_struct(
                base.map(|b| b.to_string()),
                &item_struct,
            )
            .expect("component should build");
        Module {
            path: Vec::new(),
            uses: Vec::new(),
            items: crate::component_frontend::component_module_items(component_def, view_def),
            is_builtin: false,
            allows_external_builtins: false,
        }
    }

    /// Like [`component_module`], but also attaches `uses` (each a `::`-separated path, e.g.
    /// `"crate::document_view_model::Document"`) — needed only for a test whose fixture actually
    /// exercises cross-module `use`-based type resolution (a type declared at a non-crate-root
    /// `Module::path`, `viewmodel_module_from_rust_at_its_own_module_path`'s whole reason to exist);
    /// every other `component_module` caller's referenced types live at the crate-root path (`[]`,
    /// `Module`'s own default), where `validate::validate`'s bindable-owner-in-scope check never
    /// needs a `use` at all — see `ast::Module::path`'s own doc comment on crate-root placement.
    fn component_module_with_uses(base: Option<&str>, struct_src: &str, uses: &[&str]) -> Module {
        let mut module = component_module(base, struct_src);
        module.uses = uses
            .iter()
            .map(|path| crate::ast::UseDecl {
                path: path.split("::").map(str::to_string).collect(),
            })
            .collect();
        module
    }

    fn notepad_viewmodel_module() -> Module {
        viewmodel_module_from_rust(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = String::new())]
                    content: String,

                    #[observable(default = "untitled.txt")]
                    file_name: String,

                    #[observable(default = SaveState::Unsaved)]
                    state: SaveState,

                    #[computed(expr = content.chars().count() as i32)]
                    char_count: i32,

                    #[computed(expr = t!("notepad-window-title", file_name: file_name))]
                    window_title: String,

                    #[computed(expr = state != SaveState::Saving)]
                    save_can_execute: bool,
                }

                impl NotepadViewModel {
                    fn save(&self) {
                        state = SaveState::Saving;
                        document::save(&content);
                        state = SaveState::Saved;
                    }

                    fn open(&self) {
                        content = document::open_dialog();
                        state = SaveState::Unsaved;
                    }
                }
            }
        "#,
        )
    }

    // The old DSL text form's own top-level `use` declaration (§12) has no counterpart on this
    // (real, production) frontend — an ordinary Rust `use` in the surrounding source file is
    // already resolved by `rustc` itself, with no DSL-side parsing involved at all.
    const WINDOW_SRC: &str = r#"
struct NotepadWindow {
    #[bindable]
    vm: NotepadViewModel,

    body: view! {
        Window {
            title: vm.window_title

            VerticalLayout {
                HorizontalLayout {
                    Button {
                        text: t!("notepad-menu-save")
                        on_click: vm.save
                        enabled: vm.save_can_execute
                    }
                    Button {
                        text: t!("notepad-menu-open")
                        on_click: vm.open
                    }
                }

                TextArea { text <=> vm.content }

                HorizontalLayout {
                    TextBlock { text: t!("notepad-status-chars", count: vm.char_count) }
                }
            }
        }
    },
}
"#;

    fn assert_valid_rust(label: &str, ts: &TokenStream) {
        if let Err(e) = syn::parse2::<syn::File>(ts.clone()) {
            panic!("{label} did not generate valid Rust: {e}\n---\n{ts}");
        }
    }

    // --- Font/text-style codegen tests (指示書 §32) ---------------------------------------------

    #[test]
    fn text_block_font_size_emits_as_text_style_owner_dispatch() {
        let module = crate::test_module(&[(
            None,
            r#"
                struct FontHost {
                    body: view! {
                        TextBlock { text: "hi" font_size: 20.0 }
                    },
                }
            "#,
            None,
        )])
        .expect("source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("text_block_font_size", &generated);
        let rendered = generated.to_string();
        // The seven text-style properties always dispatch through `as_text_style_owner()` — never
        // a bare `.set_font_size(..)` dot-call or a `TextBlockExt`-qualified one, since the real
        // setter lives on `TextStyleOwner`, not any `#[class]`-generated `..Ext` trait (see
        // `emit_field_setter_call`'s own doc comment).
        assert!(rendered.contains("as_text_style_owner"));
        assert!(rendered.contains("set_font_size"));
        assert!(rendered.contains("20.0"));
    }

    #[test]
    fn environment_scope_rejects_writing_the_popup_dismiss_builtin_key() {
        // The framework built-in `popup_dismiss` key is readable via `#[environment(popup_dismiss)]`
        // but must not be writable through `EnvironmentScope` (`lookup_writable_environment_key`
        // deliberately omits it) — `ContextMenuService::open_custom_popup` is the only thing allowed
        // to install a `PopupDismissAction`. Verifies elwindui-codegen itself rejects this with a
        // `compile_error!` at codegen time, distinct from an ordinary `rustc` type mismatch the RHS
        // value (`0`, not a real `PopupDismissAction`) would otherwise trigger if the write were
        // ever attempted.
        let module = crate::test_module(&[(
            Some("VerticalLayout"),
            r#"
                struct EnvironmentScopePopupDismissRejectionFixture {
                    body: view! {
                        EnvironmentScope {
                            popup_dismiss: 0
                            TextBlock { text: "hi" }
                        }
                    },
                }
            "#,
            None,
        )])
        .expect("source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("compile_error"),
            "writing popup_dismiss via EnvironmentScope must be rejected by elwindui-codegen's own \
             compile_error!, not silently accepted or deferred purely to rustc\n---\n{rendered}"
        );
        assert!(
            rendered.contains("popup_dismiss"),
            "the compile_error! message should name the rejected key\n---\n{rendered}"
        );
    }

    #[test]
    fn foreground_hex_literal_coerces_to_brush_solid() {
        let module = crate::test_module(&[(
            None,
            r##"
                struct FontHost {
                    body: view! {
                        TextBlock { text: "hi" foreground: "#3a3a3c" }
                    },
                }
            "##,
            None,
        )])
        .expect("source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("foreground_hex_literal", &generated);
        let rendered = generated.to_string();
        // Brush-valued DSL properties now normalize concrete values through `BrushStyle::Value`
        // before resolving. The string literal remains supported through `From<&str>` and reaches
        // the same concrete `set_foreground(Some(Brush))` boundary.
        assert!(rendered.contains("BrushStyle"));
        assert!(rendered.contains("#3a3a3c"));
        assert!(rendered.contains("ResolvedValue :: Value"));
        assert!(rendered.contains("set_foreground"));
    }

    #[test]
    fn dynamic_font_family_and_foreground_are_owned_text_style_arguments() {
        let module = crate::test_module(&[(
            None,
            r#"
                struct FontHost {
                    #[bindable]
                    vm: FontDemoViewModel,

                    body: view! {
                        VerticalLayout {
                            TextBlock {
                                text: "sample"
                                font_family: vm.font_family
                                foreground: vm.foreground
                            }
                            Button {
                                text: "sample"
                                font_family: vm.font_family
                                foreground: vm.foreground
                            }
                        }
                    },
                }
            "#,
            None,
        )])
        .expect("source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("dynamic_text_style_values", &generated);
        let rendered = generated.to_string();
        // `FontFamily` remains passed by value. `foreground` is normalized to BrushStyle, resolved
        // against the component's effective Environment, and the concrete result preserves the
        // local-value marker expected by TextStyleOwner.
        assert!(rendered.contains("set_font_family (self . vm . font_family ())"));
        assert!(rendered.contains("BrushStyle > :: into (self . vm . foreground ())"));
        assert!(rendered.contains("set_foreground (Some (__elwindui_semantic_brush))"));
        assert!(!rendered.contains("set_font_family (& (self . vm . font_family ()))"));
    }

    #[test]
    fn font_family_and_brush_are_not_assumed_copy_by_viewmodels() {
        assert!(!is_copy_type("FontFamily"));
        assert!(!is_copy_type("Brush"));
        assert!(!is_copy_type("BrushStyle"));
        assert!(is_copy_type("FontWeight"));
    }

    #[test]
    fn button_font_size_dispatches_through_native_control_owner() {
        // `Button` doesn't declare `font_size` itself (`#[text_style]` is only on `NativeControl`,
        // §E's own rationale) — its use site must still compile and dispatch through
        // `as_text_style_owner()`, not a `ButtonExt`-qualified call (which doesn't exist).
        let module = crate::test_module(&[(
            None,
            r#"
                struct FontHost {
                    body: view! {
                        Button { text: "Click" font_size: 16.0 }
                    },
                }
            "#,
            None,
        )])
        .expect("source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("button_font_size", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("as_text_style_owner"));
        assert!(!rendered.contains("ButtonExt :: set_font_size"));
    }

    #[test]
    fn content_control_declares_seven_text_style_fields_via_control_base() {
        // Regression guard for the `Attr::TextStyle` exemption in `resolve_effective_fields`/
        // `resolve_field_declaring_types` — `ContentControl` has its own `view`
        // that never bare-references `font_size`/etc., so without the exemption these seven fields
        // would silently vanish from its effective field set (no compile error — just a missing
        // setter downstream).
        let table = build_symbol_table_with_builtins(&[]);
        let builtins = crate::test_builtin_modules();
        let module = builtins.first().expect("builtins module should exist");
        let info = table
            .resolve(module, "ContentControl")
            .expect("ContentControl should resolve");
        for name in [
            "font_family",
            "font_size",
            "font_weight",
            "font_style",
            "font_stretch",
            "character_spacing",
            "foreground",
        ] {
            assert!(
                info.declaring_types.contains_key(name),
                "ContentControl should inherit `{name}` from Control"
            );
            assert_eq!(info.declaring_types[name], "Control");
        }
    }

    #[test]
    fn external_base_bare_attribute_forward_synthesizes_field_type_macro_call() {
        // Refs #90: `Control` here has *no* local `TypeInfo` at all (unlike
        // `content_control_declares_seven_text_style_fields_via_control_base`'s builtins-chained
        // setup, which is exactly why that test never caught this) — `find_component_and_module`
        // fails to find it, so `resolve_effective_fields` must fall back to
        // `synthesize_external_base_fields`, recovering `padding` from the view's own bare same-
        // name reference (`padding: padding`) rather than silently dropping it — the pre-fix
        // behavior that made `emit_path_get` panic downstream with "unsupported path shape after
        // bind resolution: `padding`" the moment a real consumer crate actually compiled this
        // dsl_spec.md §3 pattern against a genuinely external builtin.
        let module = crate::test_module(&[(
            Some("Control"),
            r#"
                struct Wrapper {
                    content: std::rc::Rc<dyn UIElement>,

                    body: view! {
                        padding: padding
                        content
                    },
                }
            "#,
            None,
        )])
        .expect("source should parse");
        let table = build_symbol_table(&[module.clone()]);
        let info = table
            .resolve(&module, "Wrapper")
            .expect("Wrapper should resolve");
        let (_, padding_ty) = info
            .param_fields
            .iter()
            .find(|(name, _)| name == "padding")
            .expect("padding should be synthesized as an effective field, not dropped");
        assert!(
            padding_ty.contains("__elwindui_props_Control!") && padding_ty.contains("@field_type"),
            "padding's synthesized type should defer to Control's own shape macro at the \
             consumer's expansion time, got `{padding_ty}`"
        );
        assert_eq!(
            info.declaring_types.get("padding").map(String::as_str),
            Some("Wrapper"),
            "a synthesized field has no findable ancestor to declare it, so it counts as \
             `Wrapper`'s own — never ambiguous, so `emit_field_setter_call` needs no UFCS \
             disambiguation for it"
        );
        let generated = generate_module(&module, &table);
        assert_valid_rust("external_base_bare_attribute_forward", &generated);
    }

    #[test]
    fn qualified_external_base_bare_attribute_uses_the_external_shape_macro_root() {
        let module = crate::test_module(&[(
            Some("external_widgets::ExternalBase"),
            r#"
                struct Wrapper {
                    body: view! {
                        value: value
                    },
                }
            "#,
            None,
        )])
        .expect("qualified external base source should parse");
        let table = build_symbol_table(std::slice::from_ref(&module));
        let info = table
            .resolve(&module, "Wrapper")
            .expect("Wrapper should resolve");
        let (_, value_ty) = info
            .param_fields
            .iter()
            .find(|(name, _)| name == "value")
            .expect("value should be synthesized from the qualified external base");
        let compact_value_ty = value_ty.replace(' ', "");
        assert!(
            compact_value_ty.contains("external_widgets::__elwindui_props_ExternalBase!")
                && compact_value_ty.contains("@field_type"),
            "qualified external base fields must use the defining crate's shape macro root, got `{value_ty}`"
        );
        let generated = generate_module(&module, &table);
        assert_valid_rust("qualified_external_base_shape_macro", &generated);
    }

    #[test]
    fn generates_dynamic_if_region_that_reads_the_current_property() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod {
                struct DynamicViewModel {
                    #[observable(default = true)]
                    show: bool,
                }
            }
            "#,
            None,
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    VerticalLayout {
                        if vm.show {
                            TextBlock { text: "shown" }
                        } else {
                            TextBlock { text: "hidden" }
                        }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("dynamic_if", &generated);

        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
        assert!(!rendered.contains("__dynamic_child_slot"));
    }

    /// Phase 1 (memory/elwindui_dynamic_controls_progress.md's "known unaddressed" item): `else if`
    /// (`parser.rs`'s `parse_control_child` already parses this as a `ChildEntry::If` nested in the
    /// outer `If`'s own `else_branch`, line 645-651) used to panic in `plan_child_entry` — this is
    /// the most basic case of the nesting this phase fixes.
    #[test]
    fn generates_else_if_chain() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod {
                struct DynamicViewModel {
                    #[observable(default = true)]
                    is_zero: bool,
                    #[observable(default = false)]
                    is_one: bool,
                }
            }
            "#,
            None,
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    VerticalLayout {
                        if vm.is_zero {
                            TextBlock { text: "zero" }
                        } else if vm.is_one {
                            TextBlock { text: "one" }
                        } else {
                            TextBlock { text: "many" }
                        }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("else_if_chain", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
    }

    /// Issue #52's lazy-once materialization (`lazy_branch_plan`): a childless-literal-only branch
    /// gets its own `RefCell<Option<Rc<..>>>` cache field and is constructed only once actually
    /// selected, instead of unconditionally at `new()` time like every branch used to be.
    #[test]
    fn generates_lazy_branch_cache_for_a_childless_literal_branch() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod {
                struct DynamicViewModel {
                    #[observable(default = true)]
                    show: bool,
                }
            }
            "#,
            None,
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    VerticalLayout {
                        if vm.show {
                            TextBlock { text: "shown" }
                        } else {
                            TextBlock { text: "hidden" }
                        }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("lazy_branch_cache", &generated);

        let rendered = generated.to_string();
        assert!(
            rendered.contains("__lazy_branch_"),
            "both branches are childless literals, so both should be lazily cached: {rendered}"
        );
        assert!(rendered.contains("RefCell < Option < std :: rc :: Rc"));
    }

    /// Task #14 (Issue #52): `initial_dynamic_content_value`'s construction-time value for a
    /// scalar `#[content(...)]` field must evaluate the region's condition once and construct only
    /// the selected branch — not the old unconditional "always the `then`/first-arm branch"
    /// shortcut, which used to compile only because every branch happened to be an already-
    /// unconditionally-constructed eager local. Here `ContentControl` is used as an ordinary
    /// *nested* literal child (not `inherits`, unlike the sibling
    /// `generates_scalar_content_dynamic_region_via_content_control` test, whose root-level
    /// implicit-composition sugar takes a different, unrelated emission path and never calls
    /// `initial_dynamic_content_value` at all) — its bare `if` child is exactly the shape that
    /// reaches `build_component_args`'s `#[content(field_name)]` branch. Both branches are
    /// childless literals, so both are lazily cached; if `initial_dynamic_content_value` still blindly
    /// picked the `then` branch's binding regardless of the condition, this would only accidentally
    /// still work while the condition happens to be `true` by construction-time default — flipping
    /// the DSL's own default below to `false` makes a wrong, unconditional guess fail loudly instead
    /// (a real compile error, referencing an unpopulated lazy cache's binding) rather than silently
    /// mis-selecting the branch.
    #[test]
    fn scalar_content_field_on_a_nested_literal_evaluates_condition_once_at_construction() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod {
                struct DynamicViewModel {
                    #[observable(default = false)]
                    show_a: bool,
                }
            }
            "#,
            None,
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    ContentControl {
                        if vm.show_a {
                            TextBlock { text: "a" }
                        } else {
                            TextBlock { text: "b" }
                        }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("nested_scalar_dynamic_content_construction", &generated);

        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
        assert!(
            rendered.contains("__lazy_branch_"),
            "both branches are childless literals: {rendered}"
        );
    }

    /// Control's internal scalar content destination follows the same metadata-driven dynamic
    /// path as ContentControl: no collection slot is allocated, and each branch replaces the
    /// authored visual root through `set_visual_root`.
    #[test]
    fn generates_scalar_content_dynamic_region_via_control() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod_control {
                struct DynamicViewModel {
                    #[observable(default = true)]
                    show_a: bool,
                }
            }
            "#,
            Some("Control"),
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    if vm.show_a {
                        TextBlock { text: "a" }
                    } else {
                        TextBlock { text: "b" }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(crate::validate::validate(&all_modules), Ok(()));
        let generated = generate_module(&module, &table);
        assert_valid_rust("scalar_dynamic_control", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("set_visual_root"), "{rendered}");
        assert!(!rendered.contains("DynamicChildSlot"), "{rendered}");
    }

    /// A `match` uses the same scalar replacement lowering as an `if`; the target is selected by
    /// effective content metadata rather than by the inherited base's type name.
    #[test]
    fn generates_scalar_content_dynamic_match_via_control() {
        let module = multi_item_module(&[
            TestItem::Enum("enum ControlState { First, Second }"),
            TestItem::ViewModel(
                r#"
                mod dynamic_view_model_mod_control_match {
                    struct DynamicViewModel {
                        #[observable(default = ControlState::First)]
                        state: ControlState,
                    }
                }
                "#,
            ),
            TestItem::Component(
                Some("Control"),
                r#"
                struct DynamicMatchHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,

                    body: view! {
                        match vm.state {
                            ControlState::First => { TextBlock { text: "first" } }
                            ControlState::Second => { TextBlock { text: "second" } }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(crate::validate::validate(&all_modules), Ok(()));
        let generated = generate_module(&module, &table);
        assert_valid_rust("scalar_dynamic_control_match", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("set_visual_root"), "{rendered}");
        assert!(!rendered.contains("DynamicChildSlot"), "{rendered}");
    }

    // Issue #52 §4's documented asymmetry (`lazy_branch_plan`'s own doc comment) turns out to be
    // unreachable through either DSL frontend today, confirmed by direct inspection rather than
    // assumed: `parser.rs`'s `parse_child_block` (the only parser used for `if`/`match`/`for`
    // branch bodies) unconditionally calls `parse_element_node()` for every non-control-flow
    // entry, never routing through the `ChildEntry::Ref` arm that ordinary (non-branch) element
    // bodies support (`parser.rs` around the `Column { editor, StatusBar {} }` doc example) — so
    // an `#[id("...")]`-bound `let` reference inside a branch is a parse error, not a valid
    // construct, in the text frontend. `attr_frontend.rs` (the real `#[elwindui::component]`
    // macro path every example app uses) never constructs `ChildEntry::Ref` at all, in any
    // position. `lazy_branch_plan`'s own `ChildEntry::Ref` exclusion in its eligibility check is
    // therefore a correct, harmless defensive guard for a case no current DSL input can trigger —
    // worth keeping (forward-compatible if either frontend ever gains branch-local `Ref` support)
    // but nothing here to regression-test against today without hand-constructing a `ChildEntry`
    // tree directly, which isn't warranted for logic no real input reaches.

    /// A `for` nested inside an `if`'s then-branch: the outer `if` toggles between the `for` region
    /// and a static fallback, so the nested `for`'s own `DynamicChildSlot` must be forced empty
    /// (`replace_children` with an empty `vec`) whenever the `if` picks the static branch instead.
    #[test]
    fn generates_nested_for_inside_if_then_branch() {
        let module = multi_item_module(&[
            TestItem::ViewModel(r#"mod item_mod { struct Item { } }"#),
            TestItem::ViewModel(
                r#"
                mod dynamic_view_model_mod {
                    struct DynamicViewModel {
                        #[observable(default = true)]
                        show_list: bool,
                        #[observable(default = Vec::new())]
                        items: Vec<std::rc::Rc<Item>>,
                    }
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct ItemView {
                    #[param]
                    item: std::rc::Rc<Item>,

                    body: view! { TextBlock { text: "item" } },
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,

                    body: view! {
                        VerticalLayout {
                            if vm.show_list {
                                for item in vm.items { ItemView { item: item } }
                            } else {
                                TextBlock { text: "empty" }
                            }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("nested_for_in_if", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
        assert!(rendered.contains("replace_rc_items"));
        // The nested `for`'s own slot must be independently clearable (empty `vec![]`) when the
        // outer `if` picks the static `else` branch instead.
        assert!(rendered.contains("replace_children") && rendered.contains("Vec :: new ()"));
    }

    /// An `if` nested inside one `match` arm: exercises `plan_dynamic_entry`'s `Match` case
    /// delegating to `plan_child_entry` for a nested control-flow entry, and the generated
    /// `__refresh_dynamic_regions`'s per-arm "clear every *other* arm's own nested markers" logic.
    #[test]
    fn generates_nested_if_inside_match_arm() {
        let module = multi_item_module(&[
            TestItem::Enum("enum Status { Ready, Busy }"),
            TestItem::ViewModel(
                r#"
                mod dynamic_view_model_mod {
                    struct DynamicViewModel {
                        #[observable(default = Status::Ready)]
                        status: Status,
                        #[observable(default = false)]
                        urgent: bool,
                    }
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,

                    body: view! {
                        VerticalLayout {
                            match vm.status {
                                Status::Ready => {
                                    if vm.urgent {
                                        TextBlock { text: "ready-urgent" }
                                    } else {
                                        TextBlock { text: "ready" }
                                    }
                                }
                                Status::Busy => { TextBlock { text: "busy" } }
                            }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("nested_if_in_match", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
    }

    /// Phase 2 (docs/design/runtime/ui_tree_design.md): a scalar `#[content(...)]` field
    /// (`ContentControl`'s `content: Rc<dyn UIElement>`) can host an `if`/`match` dynamic region —
    /// combined with Phase 0's implicit-composition sugar, this is what used to be called "root
    /// self-dynamism": `component X inherits ContentControl { view X { if .. { A } else { B } } }`
    /// with no wrapper element written at all. Swapping must go through `set_content`, never
    /// `DynamicChildSlot` (there is nowhere to keep a list position for a single-value field).
    #[test]
    fn generates_scalar_content_dynamic_region_via_content_control() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod {
                struct DynamicViewModel {
                    #[observable(default = true)]
                    show_a: bool,
                }
            }
            "#,
            Some("ContentControl"),
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    if vm.show_a {
                        TextBlock { text: "a" }
                    } else {
                        TextBlock { text: "b" }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(crate::validate::validate(&all_modules), Ok(()));
        let generated = generate_module(&module, &table);
        assert_valid_rust("scalar_dynamic_content_control", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
        assert!(rendered.contains("set_content"));
        assert!(!rendered.contains("DynamicChildSlot"));
    }

    /// Same as above, but composing over `Window` (host composition) instead of `ContentControl` —
    /// confirms the scalar swap path works uniformly regardless of which composed base declares the
    /// scalar `#[content(...)]` field (`effective_content_shape`/
    /// `emit_scalar_dynamic_node_refresh` don't special-case either type by name).
    #[test]
    fn generates_scalar_content_dynamic_region_via_window_host_composition() {
        let module = viewmodel_and_component_module(
            r#"
            mod dynamic_view_model_mod {
                struct DynamicViewModel {
                    #[observable(default = true)]
                    show_a: bool,
                }
            }
            "#,
            Some("Window"),
            r#"
            struct DynamicHost {
                #[param]
                #[inject]
                vm: DynamicViewModel,

                body: view! {
                    title: "Dynamic"
                    if vm.show_a {
                        TextBlock { text: "a" }
                    } else {
                        TextBlock { text: "b" }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(crate::validate::validate(&all_modules), Ok(()));
        let generated = generate_module(&module, &table);
        assert_valid_rust("scalar_dynamic_content_window", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
        assert!(rendered.contains("set_content"));
        assert!(!rendered.contains("DynamicChildSlot"));
    }

    #[test]
    fn generates_dynamic_match_region() {
        let module = multi_item_module(&[
            TestItem::Enum("enum Status { Ready, Busy }"),
            TestItem::ViewModel(
                r#"
                mod dynamic_view_model_mod {
                    struct DynamicViewModel {
                        #[observable(default = Status::Ready)]
                        status: Status,
                    }
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,

                    body: view! {
                        VerticalLayout {
                            match vm.status {
                                Status::Ready => { TextBlock { text: "ready" } }
                                Status::Busy => { TextBlock { text: "busy" } }
                            }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("dynamic_match", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
    }

    #[test]
    fn generates_dynamic_for_region_with_an_item_local_template() {
        let module = multi_item_module(&[
            TestItem::ViewModel(r#"mod item_mod { struct Item { } }"#),
            TestItem::ViewModel(
                r#"
                mod dynamic_view_model_mod {
                    struct DynamicViewModel {
                        #[observable(default = Vec::new())]
                        items: Vec<std::rc::Rc<Item>>,
                    }
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct ItemView {
                    #[param]
                    item: std::rc::Rc<Item>,

                    body: view! { TextBlock { text: "item" } },
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,

                    body: view! {
                        VerticalLayout {
                            for item in vm.items { ItemView { item: item } }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("dynamic_for", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("replace_rc_items"));
        assert!(rendered.contains("item . clone"));
    }

    // --- Issue #58: bind-owner-only dynamic region conditions must still resync -----------------
    //
    // `property_resync_methods_for` used to scan only `PlannedNode::attributes` (via
    // `collect_view_expr_owner_properties`) to decide which properties a bind owner's
    // `__resync_<owner>` reacts to, never a dynamic region's own `DynamicPlan::If.condition` /
    // `Match.value` / `For.collection`. A bind-owner property referenced *only* there (never in a
    // sibling attribute) therefore never got a `match property { .. }` arm at all — its
    // `PropertyChanged` notification reached `__resync_<owner>` (the subscription itself was fine)
    // but fell through the `_ => {}` catch-all, so `__refresh_dynamic_regions()` never ran. The
    // property name below is chosen so it appears in the rendered output *only* if a resync arm was
    // actually generated for it (nothing else in the generated code would ever spell it out).

    #[test]
    fn resyncs_bind_owner_property_used_only_in_if_condition() {
        let module = viewmodel_and_component_module(
            r#"
            mod toggle_view_model_mod {
                struct ToggleViewModel {
                    #[observable(default = true)]
                    show_then: bool,
                }
            }
            "#,
            None,
            r#"
            struct ToggleHost {
                #[bindable]
                vm: Rc<ToggleViewModel>,

                body: view! {
                    VerticalLayout {
                        if vm.show_then {
                            TextBlock { text: "then" }
                        } else {
                            TextBlock { text: "else" }
                        }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("bind_owner_if_condition", &generated);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("fn __resync_vm (& self , property : & 'static str)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\"show_then\""),
            "condition-only property must still get a __resync_vm arm: {rendered}"
        );
        assert!(rendered.contains("__refresh_dynamic_regions"), "{rendered}");
    }

    #[test]
    fn resyncs_bind_owner_property_used_only_in_match_value() {
        let module = multi_item_module(&[
            TestItem::Enum("enum Status { Ready, Busy }"),
            TestItem::ViewModel(
                r#"
                mod status_view_model_mod {
                    struct StatusViewModel {
                        #[observable(default = Status::Ready)]
                        current_status: Status,
                    }
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct StatusHost {
                    #[bindable]
                    vm: Rc<StatusViewModel>,

                    body: view! {
                        VerticalLayout {
                            match vm.current_status {
                                Status::Ready => { TextBlock { text: "ready" } }
                                Status::Busy => { TextBlock { text: "busy" } }
                            }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("bind_owner_match_value", &generated);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("fn __resync_vm (& self , property : & 'static str)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\"current_status\""),
            "value-only property must still get a __resync_vm arm: {rendered}"
        );
        assert!(rendered.contains("__refresh_dynamic_regions"), "{rendered}");
    }

    #[test]
    fn resyncs_bind_owner_property_used_only_in_for_collection() {
        let module = multi_item_module(&[
            TestItem::ViewModel(r#"mod row_item_mod { struct RowItem { } }"#),
            TestItem::ViewModel(
                r#"
                mod rows_view_model_mod {
                    struct RowsViewModel {
                        #[observable(default = Vec::new())]
                        row_items: Vec<std::rc::Rc<RowItem>>,
                    }
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct RowView {
                    #[param]
                    item: std::rc::Rc<RowItem>,

                    body: view! { TextBlock { text: "row" } },
                }
                "#,
            ),
            TestItem::Component(
                None,
                r#"
                struct RowsHost {
                    #[bindable]
                    vm: Rc<RowsViewModel>,

                    body: view! {
                        VerticalLayout {
                            for item in vm.row_items { RowView { item: item } }
                        }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("bind_owner_for_collection", &generated);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("fn __resync_vm (& self , property : & 'static str)"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\"row_items\""),
            "collection-only property must still get a __resync_vm arm: {rendered}"
        );
        assert!(rendered.contains("__refresh_dynamic_regions"), "{rendered}");
    }

    #[test]
    fn resyncs_bind_owner_property_used_only_in_nested_if_condition() {
        let module = viewmodel_and_component_module(
            r#"
            mod nested_view_model_mod {
                struct NestedViewModel {
                    #[observable(default = true)]
                    outer_flag: bool,
                    #[observable(default = true)]
                    inner_flag: bool,
                }
            }
            "#,
            None,
            r#"
            struct NestedHost {
                #[bindable]
                vm: Rc<NestedViewModel>,

                body: view! {
                    VerticalLayout {
                        if vm.outer_flag {
                            VerticalLayout {
                                if vm.inner_flag {
                                    TextBlock { text: "inner-then" }
                                } else {
                                    TextBlock { text: "inner-else" }
                                }
                            }
                        } else {
                            TextBlock { text: "outer-else" }
                        }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("bind_owner_nested_if_condition", &generated);
        let rendered = generated.to_string();
        assert!(
            rendered.contains("\"outer_flag\""),
            "outer condition property must get a __resync_vm arm: {rendered}"
        );
        assert!(
            rendered.contains("\"inner_flag\""),
            "nested condition property must also get a __resync_vm arm: {rendered}"
        );
        assert!(rendered.contains("__refresh_dynamic_regions"), "{rendered}");
    }

    #[test]
    fn generates_two_way_wiring_for_an_rc_for_item() {
        let module = multi_item_module(&[
            TestItem::ViewModel(
                r#"
                mod row_mod {
                    struct Row {
                        #[observable(default = String::new())]
                        content: String,
                    }
                }
                "#,
            ),
            TestItem::ViewModel(
                r#"
                mod rows_mod {
                    struct Rows {
                        #[observable(default = Vec::new())]
                        rows: Vec<Row>,
                    }
                }
                "#,
            ),
            TestItem::Component(
                Some("VerticalLayout"),
                r#"
                struct Search {
                    #[bindable]
                    vm: Rc<Rows>,

                    body: view! {
                        for row in vm.rows { TextArea { text <=> row.content } }
                    },
                }
                "#,
            ),
        ]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::test_builtin_modules())
            .collect();
        crate::validate::validate(&all_modules)
            .expect("direct observable for-item field must validate");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("for_item_two_way", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("replace_rc_items"), "{rendered}");
        assert!(rendered.contains("set_on_text_change"), "{rendered}");
        assert!(rendered.contains("source . set_content"), "{rendered}");
        assert!(
            rendered.contains("__dynamic_item_subscriptions"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("__elwindui_for_item_this"),
            "pure TwoWay wiring must not retain the enclosing component: {rendered}"
        );
    }

    #[test]
    fn rebuilds_only_the_for_slot_for_non_rc_items() {
        let module = multi_item_module(&[
            TestItem::ViewModel(
                r#"
                mod dynamic_view_model_mod {
                    struct DynamicViewModel {
                        #[observable(default = Vec::new())]
                        items: Vec<String>,
                    }
                }
                "#,
            ),
            TestItem::Component(
                Some("VerticalLayout"),
                r#"
                struct ItemView {
                    #[param]
                    item: String,

                    body: view! { TextBlock { text: item } },
                }
                "#,
            ),
            TestItem::Component(
                Some("VerticalLayout"),
                r#"
                struct DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,

                    body: view! {
                        for item in vm.items { ItemView { item: item } }
                    },
                }
                "#,
            ),
        ]);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        assert_eq!(crate::validate::validate(&[module.clone()]), Ok(()));
        let generated = generate_module(&module, &table);
        assert_valid_rust("plain_dynamic_for", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("replace_items"));
        assert!(!rendered.contains("replace_rc_items"));
    }

    #[test]
    fn generates_valid_rust_for_notepad() {
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = component_module(None, WINDOW_SRC);
        let table =
            build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let viewmodel_code = generate_module(&viewmodel_module, &table);
        assert_valid_rust("notepad_viewmodel", &viewmodel_code);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("notepad_window", &window_code);

        let window_str = window_code.to_string();
        assert!(window_str.contains("struct NotepadWindow"));
        assert!(window_str.contains("fn resync"));
        assert!(window_str.contains("save_can_execute"));
        // `#[bindable] vm` (`WINDOW_SRC`) must wire an `ObservableExt`-based, string-keyed
        // subscription rather than the old per-viewmodel enum — see `ast::Attr::Bindable`.
        assert!(window_str.contains("ObservableExt :: subscribe_property_changed"));
        assert!(window_str.contains("fn __resync_vm (& self , property : & 'static str)"));
        assert!(window_str.contains("\"window_title\""));
        assert!(window_str.contains("\"char_count\""));
    }

    /// Generalized `on_*` closure wiring (replaces the old `usize`-sniffing `command_execute_call`
    /// special case): a zero-param closure with a multi-statement block body on a `#[routed]`
    /// field (`Button.on_click`), and a 1-param closure with a block body on a plain `fn(usize)`
    /// field (`TabView.on_select`) — both should resolve `vm.save`/`vm.select_tab(index)` bare
    /// references the same way a single-expression body already does.
    #[test]
    fn on_star_closures_support_block_bodies_and_generalized_arity() {
        let window_src = r#"
struct NotepadWindow {
    #[bindable]
    vm: std::rc::Rc<NotepadViewModel>,

    body: view! {
        Window {
            title: vm.window_title
            Button {
                text: t!("notepad-menu-save")
                on_click: || {
                    vm.save();
                    vm.save();
                }
            }
        }
    },
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = component_module(None, window_src);
        let table =
            build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("on_star_block_body_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains("register_routed_handler"));
        // Both statements' bare `vm` reference must have been rewritten to `this . vm`.
        assert_eq!(window_str.matches("this . vm . save ()").count(), 2);
    }

    /// The pointer/tap `#[routed]` fields added to the common `UIElement` component
    /// (docs/design/runtime/ui_tree_design.md) must be wired with the payload type each
    /// field itself declares (`fn(elwindui_core::input::PointerEventArgs)`/`TappedEventArgs`/...) —
    /// derived purely from `TypeInfo::field_types` via `callback_param_types`, never a hardcoded
    /// event-name/type table in `elwindui-codegen` itself (the codegen design doc's own no-
    /// hardcoding rule). Exercised on a plain virtual builtin (`VerticalLayout`), not `Button`.
    #[test]
    fn routed_pointer_event_derives_its_payload_type_from_the_field_declaration() {
        let window_src = r#"
struct NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    body: view! {
        Window {
            title: vm.window_title
            VerticalLayout {
                on_tapped: |e| { vm.save(); }
            }
        }
    },
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = component_module(None, window_src);
        let table =
            build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("routed_pointer_event_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains(
            "register_routed_handler :: < elwindui :: core :: input :: TappedEventArgs >"
        ));
        assert_eq!(window_str.matches("\"on_tapped\"").count(), 1);
        assert_eq!(window_str.matches("this . vm . save ()").count(), 1);
        // `VerticalLayout` (a virtual builtin, unlike `Button`) has no *inherent*
        // `register_routed_handler` of its own — only `UIElementExt`'s default method, reachable
        // via `.as_ui_element()` with that trait explicitly in scope. `assert_valid_rust` only
        // checks syntax (`syn`, no name resolution), so it alone would not have caught a
        // regression back to a bare `widget.register_routed_handler(..)` call here — this crate's
        // own `cargo build -p notepad` is what actually surfaced that failure mode originally.
        assert!(window_str.contains("widget . as_ui_element () . register_routed_handler"));
        assert!(window_str.contains("use elwindui :: core :: ui :: UIElementExt as _ ;"));
    }

    /// Two different `#[routed]` fields on the same element must each resolve to their *own*
    /// declared payload type, not share one — confirms the type derivation is genuinely per-field
    /// (`TypeInfo::field_types`), not a single guessed/default type.
    #[test]
    fn distinct_routed_pointer_events_each_resolve_their_own_distinct_payload_type() {
        let window_src = r#"
struct NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    body: view! {
        Window {
            title: vm.window_title
            VerticalLayout {
                on_pointer_entered: |e| { vm.save(); }
                on_pointer_wheel_changed: |e| { vm.save(); }
            }
        }
    },
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = component_module(None, window_src);
        let table =
            build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("distinct_routed_pointer_events_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains(
            "register_routed_handler :: < elwindui :: core :: input :: PointerEventArgs >"
        ));
        assert!(window_str.contains(
            "register_routed_handler :: < elwindui :: core :: input :: PointerWheelEventArgs >"
        ));
    }

    #[test]
    fn on_tapped_closure_with_wrong_param_count_panics() {
        let window_src = r#"
struct NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    body: view! {
        Window {
            title: vm.window_title
            VerticalLayout {
                on_tapped: || vm.save()
            }
        }
    },
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = component_module(None, window_src);
        let table =
            build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let result = std::panic::catch_unwind(|| generate_module(&window_module, &table));
        assert!(
            result.is_err(),
            "expected a panic for a 0-param closure on a #[routed] field declaring 1 parameter"
        );
    }

    #[test]
    fn on_select_closure_with_wrong_param_count_panics() {
        let window_src = r#"
struct NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    body: view! {
        Window {
            title: vm.window_title
            TabView {
                selected_index: 0
                on_select: || vm.save
            }
        }
    },
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = component_module(None, window_src);
        let table =
            build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let result = std::panic::catch_unwind(|| generate_module(&window_module, &table));
        assert!(
            result.is_err(),
            "expected a panic for a 0-param closure on a `fn(usize)` field"
        );
    }

    #[test]
    fn generates_valid_rust_for_menubar_and_tabview() {
        let document_module = viewmodel_module_from_rust(
            r#"
            mod document_mod {
                struct Document {
                    #[observable(default = String::new())]
                    content: String,

                    #[observable(default = "untitled.txt")]
                    file_name: String,
                }
            }
            "#,
        );
        let viewmodel_module = viewmodel_module_from_rust(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = Vec::new())]
                    documents: Vec<Document>,

                    #[observable(default = 0usize)]
                    active_tab: usize,
                }

                impl NotepadViewModel {
                    fn new_tab(&self) {
                        documents.push(std::rc::Rc::new(Document::new()));
                        active_tab = documents.len() - 1;
                    }

                    fn close_tab(&self, index: usize) {
                        documents.remove(index);
                    }

                    fn select_tab(&self, index: usize) {
                        active_tab = index;
                    }
                }
            }
        "#,
        );
        let window_src = r#"
struct NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    body: view! {
        Window {
            title: t!("notepad-window-title")

            menu_bar: MenuBar {
                MenuBarItem {
                    text: t!("menu-file")
                    Menu {
                        MenuItem { text: t!("menu-new"), shortcut: "n", on_select: vm.new_tab }
                    }
                }
            }

            content: TabView {
                for doc in vm.documents {
                    TabViewItem {
                        header: doc.file_name
                        TextArea { text: doc.content }
                    }
                }
                selected_index <=> vm.active_tab
                on_new_tab: vm.new_tab
            }
        }
    },
}
"#;
        let window_module = component_module(None, window_src);
        let table = build_symbol_table_with_builtins(&[
            document_module.clone(),
            viewmodel_module.clone(),
            window_module.clone(),
        ]);

        let viewmodel_code = generate_module(&viewmodel_module, &table);
        assert_valid_rust("menubar_tabview_viewmodel", &viewmodel_code);
        let viewmodel_str = viewmodel_code.to_string();
        assert!(viewmodel_str.contains("documents_push"));
        assert!(viewmodel_str.contains("documents_remove"));
        assert!(viewmodel_str.contains("Rc < Document >"));
        assert!(viewmodel_str.contains("fn close_tab (& self , index : usize)"));
        assert!(viewmodel_str.contains("NotepadViewModelProperty"));
        assert!(viewmodel_str.contains("subscribe_property_changed"));
        assert!(!viewmodel_str.contains("__resync_subscribers"));
        // Item updates are observed by their rendered view/template, never bubbled through the
        // owning collection as a synthetic parent change.
        assert!(!viewmodel_str.contains("item . subscribe"));

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("menubar_tabview_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains("MenuBar :: new"));
        assert!(window_str.contains("MenuItem :: new"));
        assert!(window_str.contains("set_shortcut"));
        assert!(window_str.contains("TabView :: new"));
        // `TabView`'s per-tab chip/content materialization (`insert_tab`, `__weak_self`) is no
        // longer generated here at all — it's hand-written Rust inside the corresponding
        // `elwindui-backend-*` crate now, reached generically the same way any other resolved
        // type's constructor is.
        assert!(!window_str.contains("insert_tab"));
        assert!(!window_str.contains("__weak_self"));
        assert!(!window_str.contains("set_items_source"));
        assert!(window_str.contains("set_selected_index"));
    }

    #[test]
    fn generates_dynamic_tabview_children_and_refreshes_after_new_tab() {
        let viewmodel_src = r#"
        mod document_mod {
            struct Document {
                #[observable(default = String::new())]
                content: String,

                #[observable(default = "untitled.txt")]
                file_name: String,
            }
        }
        "#;
        let notepad_viewmodel_module = viewmodel_module_from_rust(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = Vec::new())]
                    documents: Vec<std::rc::Rc<Document>>,

                    #[observable(default = 0usize)]
                    active_tab: usize,
                }

                impl NotepadViewModel {
                    fn new_tab(&self) {
                        documents.push(std::rc::Rc::new(Document::new()));
                        active_tab = documents.len() - 1;
                    }

                    fn close_tab(&self, index: usize) {
                        documents.remove(index);
                    }

                    fn select_tab(&self, index: usize) {
                        active_tab = index;
                    }
                }
            }
        "#,
        );
        let document_view_src = r#"
        struct DocumentView {
            #[bindable]
            doc: std::rc::Rc<Document>,

            body: view! {
                TextArea { text <=> doc.content }
            },
        }
        "#;
        let window_src = r#"
        struct NotepadWindow {
            #[bindable]
            vm: std::rc::Rc<NotepadViewModel>,

            body: view! {
                title: t!("notepad-window-title")

                TabView {
                    for doc in vm.documents {
                        TabViewItem {
                            header: doc.file_name
                            DocumentView { doc: doc }
                        }
                        TabViewItem {
                            header: "Details"
                            TextBlock { text: doc.file_name }
                        }
                    }
                    selected_index <=> vm.active_tab
                    on_new_tab: vm.new_tab
                }
            },
        }
        "#;
        let document_module = viewmodel_module_from_rust(viewmodel_src);
        let document_view_module = component_module(Some("VerticalLayout"), document_view_src);
        let window_module = component_module(Some("Window"), window_src);
        let modules = [
            document_module.clone(),
            notepad_viewmodel_module.clone(),
            document_view_module.clone(),
            window_module.clone(),
        ];
        let all_modules: Vec<_> = modules
            .iter()
            .cloned()
            .chain(crate::test_builtin_modules())
            .collect();
        let table = build_symbol_table(&all_modules);

        assert_eq!(crate::validate::validate(&all_modules), Ok(()));

        let document_view_code = generate_module(&document_view_module, &table);
        assert_valid_rust("document_view", &document_view_code);
        let document_view_str = document_view_code.to_string();
        // `DocumentView` now `inherits VerticalLayout` (shape composition), so its own
        // `#[elwindui::class]`-wrapped `impl` emits a private `construct(..)` entry point rather
        // than the base-less path's public `new(..)` — the public `new(..)` wrapper is synthesized
        // by the outer `#[elwindui_macros::class]` machinery this test doesn't expand.
        assert!(document_view_str.contains("fn construct (doc : std :: rc :: Rc < Document >)"));
        assert!(
            !document_view_str.contains("fn show"),
            "DocumentView's root isn't `Window` — `show()` shouldn't be generated"
        );
        // `VerticalLayout` is a hand-written *virtual* builtin (no backend struct — see
        // `is_virtual_builtin`), so `DocumentView`'s root is virtual too (recursively inferred,
        // `build_symbol_table`'s `resolve_is_native`) so it generates `into_node`.
        assert!(
            document_view_str.contains("fn into_node"),
            "document_view_str: {document_view_str}"
        );

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("tabview_render_content_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains("DynamicChildSlot"));
        assert!(window_str.contains("replace_rc_items"));
        assert!(window_str.contains("set_on_new_tab"));
        assert!(window_str.contains("new_tab"));
        assert!(window_str.contains("__refresh_dynamic_regions"));
        assert!(window_str.contains("DynamicChild :: with_children"));
        assert!(window_str.contains("__dynamic_item_subscriptions"));
        assert!(window_str.contains("source . subscribe_property_changed"));
        assert!(window_str.contains("item . set_header"));
        assert!(!window_str.contains("set_items_source"));
    }

    /// Regression test for a `for`-loop item template element's `on_*` attribute being silently
    /// dropped — `elwindui-backend-appkit`'s `native_ui::TabView`'s per-item `on_close` closure
    /// found `TabViewItem::on_close` always `None` at runtime because nothing ever called
    /// `set_on_close` on it: `emit_construction`/`build_component_setters` skip `on_*`-named
    /// fields outright (`emit_wiring` is supposed to handle them), but `emit_for_renderer` never
    /// called `emit_wiring` for elements inside a `for` body at all. Mirrors real
    /// `examples/notepad`'s own shape as closely as possible: a zero-arg `close_active_tab`
    /// bare-method-reference (not an explicit closure), set on a `for`-loop item element,
    /// referencing the enclosing component's own injected `vm`.
    #[test]
    fn generates_on_close_wiring_for_a_for_loop_item_template_element() {
        let viewmodel_src = r#"
        mod document_mod {
            struct Document {
                #[observable(default = "untitled.txt")]
                file_name: String,
            }
        }
        "#;
        let notepad_viewmodel_module = viewmodel_module_from_rust(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = Vec::new())]
                    documents: Vec<std::rc::Rc<Document>>,

                    #[observable(default = 0usize)]
                    active_tab: usize,
                }

                impl NotepadViewModel {
                    fn close_tab(&self, index: usize) {
                        documents.remove(index);
                    }

                    fn close_active_tab(&self) {
                        self.close_tab(active_tab);
                    }
                }
            }
        "#,
        );
        let window_src = r#"
        struct NotepadWindow {
            #[bindable]
            vm: std::rc::Rc<NotepadViewModel>,

            body: view! {
                title: t!("notepad-window-title")

                TabView {
                    for doc in vm.documents {
                        TabViewItem {
                            header: doc.file_name
                            closable: true
                            on_close: vm.close_active_tab
                            TextBlock { text: doc.file_name }
                        }
                    }
                    selected_index <=> vm.active_tab
                }
            },
        }
        "#;
        let document_module = viewmodel_module_from_rust(viewmodel_src);
        let window_module = component_module(Some("Window"), window_src);
        let modules = [
            document_module.clone(),
            notepad_viewmodel_module.clone(),
            window_module.clone(),
        ];
        let all_modules: Vec<_> = modules
            .iter()
            .cloned()
            .chain(crate::test_builtin_modules())
            .collect();
        let table = build_symbol_table(&all_modules);

        assert_eq!(crate::validate::validate(&all_modules), Ok(()));

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("for_loop_on_close_window", &window_code);
        let window_str = window_code.to_string();
        assert!(
            window_str.contains("set_on_close"),
            "window_str: {window_str}"
        );
        assert!(
            window_str.contains("close_active_tab"),
            "window_str: {window_str}"
        );
        // The upgrade-and-downcast prelude `emit_for_item_wiring` adds — proves the item's own
        // wiring closure captures an owned `Rc<NotepadWindow>` rather than trying to move a
        // borrowed `&self` into a `'static` closure.
        assert!(window_str.contains("__self_weak"));
        assert!(window_str.contains("downcast :: < NotepadWindow >"));
    }

    /// Unlike `viewmodel_module_from_rust` (used by other tests), registers the viewmodel module at
    /// `path: vec![mod_name]` — matching what `attr_frontend::viewmodel_defs_from_rs_file` (the real
    /// `compile_dir_with_extra_viewmodels` production path) actually does, not the shared test
    /// helper's simplified `path: []`. Needed by `for_loop_identity_survives_when_element_type_isnt_
    /// used_by_the_for_loops_own_file` below to reproduce the real bug: with `path: []` (same as
    /// every text-parsed module), the element type would be trivially visible to *any* module
    /// with no `use` needed at all, masking the exact cross-module scoping gap this test exists to
    /// catch.
    fn viewmodel_module_from_rust_at_its_own_module_path(src: &str) -> Module {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse as valid Rust");
        let mod_name = item_mod.ident.to_string();
        let def = crate::attr_frontend::viewmodel_def_from_item_mod(&item_mod)
            .expect("should build a ViewModelDef");
        Module {
            path: vec![mod_name],
            uses: Vec::new(),
            items: vec![Item::ViewModel(def)],
            ..Default::default()
        }
    }

    /// Regression test for the real `examples/notepad` bug this session root-caused: a `for` loop
    /// (`notepad_window.rs`) over a `#[elwindui::viewmodel]`-declared `Vec<DocumentViewModel>`
    /// (no `Rc<..>` spelled in the field type — the declaration-boundary shape `#[elwindui::
    /// viewmodel]` is documented to use) generated `replace_items` (full rebuild every refresh,
    /// discarding native control state) instead of `replace_rc_items`, because the *element* type
    /// (`Document`, standing in for `DocumentViewModel`) was never `use`d by the `for` loop's own
    /// file — only `DocumentView` (the child component actually receiving it) was. Fixed by basing
    /// the identity decision on `DocumentView.doc`'s `#[bindable]` marker (see `collection_uses_rc_
    /// identity`'s doc comment) instead of resolving the element type by name.
    #[test]
    fn for_loop_identity_survives_when_element_type_isnt_used_by_the_for_loops_own_file() {
        let notepad_viewmodel_module = viewmodel_module_from_rust_at_its_own_module_path(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = Vec::new())]
                    documents: Vec<Document>,

                    #[observable(default = 0usize)]
                    active_tab: usize,
                }

                impl NotepadViewModel {
                    fn new_tab(&self) {
                        documents.push(Document::new());
                        active_tab = documents.len() - 1;
                    }

                    fn select_tab(&self, index: usize) {
                        active_tab = index;
                    }
                }
            }
        "#,
        );
        let document_module = viewmodel_module_from_rust_at_its_own_module_path(
            r#"
            mod document_view_model {
                struct Document {
                    #[observable(default = String::new())]
                    content: String,

                    #[observable(default = "untitled.txt")]
                    file_name: String,
                }
            }
        "#,
        );
        // Mirrors `document_view.rs`: `#[bindable]` (not `#[param] #[inject]`). The old DSL text
        // form's own `use` (§12) has no counterpart on this (real, production) frontend — an
        // ordinary Rust `use` in the surrounding source file is already resolved by `rustc` itself.
        let document_view_src = r#"
        struct DocumentView {
            #[bindable]
            doc: std::rc::Rc<Document>,

            body: view! {
                TextArea { text <=> doc.content }
            },
        }
        "#;
        // Mirrors `notepad_window.rs`: never references `Document` directly — `doc` is only ever
        // referenced through the `for` loop's own binding.
        let window_src = r#"
        struct NotepadWindow {
            #[bindable]
            vm: std::rc::Rc<NotepadViewModel>,

            body: view! {
                title: t!("notepad-window-title")

                TabView {
                    for doc in vm.documents {
                        TabViewItem {
                            header: doc.file_name
                            DocumentView { doc: doc }
                        }
                    }
                    selected_index <=> vm.active_tab
                    on_new_tab: vm.new_tab
                }
            },
        }
        "#;
        let document_view_module = component_module_with_uses(
            Some("VerticalLayout"),
            document_view_src,
            &["crate::document_view_model::Document"],
        );
        let window_module = component_module_with_uses(
            Some("Window"),
            window_src,
            &[
                "crate::notepad_view_model::NotepadViewModel",
                "crate::DocumentView",
            ],
        );
        let modules = [
            notepad_viewmodel_module.clone(),
            document_module.clone(),
            document_view_module.clone(),
            window_module.clone(),
        ];
        let all_modules: Vec<_> = modules
            .iter()
            .cloned()
            .chain(crate::test_builtin_modules())
            .collect();
        let table = build_symbol_table(&all_modules);

        assert_eq!(crate::validate::validate(&all_modules), Ok(()));

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("for_loop_identity_window", &window_code);
        let window_str = window_code.to_string();
        assert!(
            window_str.contains("replace_rc_items"),
            "window_str: {window_str}"
        );
        assert!(!window_str.contains("replace_items"));
    }

    #[test]
    fn generate_view_ctor_uses_component_field_names_not_a_hardcoded_vm() {
        let module = viewmodel_and_component_module(
            r#"
            mod greeter_mod {
                struct Greeter {
                    #[observable(default = String::new())]
                    name: String,
                }
            }
            "#,
            None,
            r#"
            struct Greeting {
                #[param]
                #[inject]
                greeter: Greeter,

                body: view! {
                    TextBlock { text: greeter.name }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("greeting_ctor", &generated);

        let s = generated.to_string();
        assert!(
            s.contains("fn new (greeter : Greeter)"),
            "expected ctor param named `greeter`, got:\n{s}"
        );
        assert!(
            !s.contains("vm"),
            "ctor shouldn't hardcode a `vm` field name:\n{s}"
        );
        // `Greeting`'s view root is `TextBlock`, not `Window` — no top-level window to `show()`.
        assert!(!s.contains("fn show"));
        assert!(s.contains("fn into_node"));
    }

    #[test]
    fn property_update_does_not_reapply_unrelated_common_attributes() {
        let module = viewmodel_and_component_module(
            r#"
            mod document_mod {
                struct Document {
                    #[observable(default = String::new())]
                    content: String,

                    #[observable(default = String::new())]
                    file_name: String,
                }
            }
            "#,
            None,
            r#"
            struct DocumentView {
                #[param]
                #[inject]
                doc: Document,

                body: view! {
                    VerticalLayout {
                        TextArea { text: doc.content }
                        TextBlock { margin: 4.0, text: doc.file_name }
                    }
                },
            }
            "#,
        );
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("property_update_common_attributes", &generated);

        let generated = generated.to_string();
        // `margin` is set at construction and by the initial resync. Neither `content` nor
        // `file_name` notification may relayout this unrelated common UIElement property.
        assert_eq!(generated.matches("set_margin").count(), 2, "{generated}");
    }

    #[test]
    fn component_state_is_private_reactive_and_supports_explicit_two_way() {
        let src = r#"
            struct Search {
                #[state(default = String::new())]
                query: String,

                body: view! {
                    VerticalLayout {
                        TextArea { text <=> query }
                        TextBlock { text: format!("Live: {}", query) }
                        TextBlock { text: once!(format!("Snapshot: {}", query)) }
                    }
                },
            }
        "#;
        let module = crate::test_module(&[(None, src, None)]).expect("state source should parse");
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("component_state", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn new ()"), "{rendered}");
        assert!(!rendered.contains("fn new (query"), "{rendered}");
        assert!(!rendered.contains("pub fn query"), "{rendered}");
        assert!(rendered.contains("fn set_query"), "{rendered}");
        assert!(rendered.contains("set_on_text_change"), "{rendered}");
        assert!(rendered.contains("this . set_query"), "{rendered}");
        assert!(rendered.contains("format ! (\"Live: {}\""), "{rendered}");
        assert!(rendered.contains("Rc :: downgrade"), "{rendered}");
        assert!(rendered.contains("retain"), "{rendered}");
        assert!(rendered.contains("Rc :: ptr_eq"), "{rendered}");
    }

    #[test]
    fn normal_assignment_never_generates_reverse_wiring() {
        let src = r#"
            struct Search {
                #[state(default = String::new())]
                query: String,

                body: view! { TextArea { text: query } },
            }
        "#;
        let module =
            crate::test_module(&[(None, src, None)]).expect("one-way state source should parse");
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let rendered = generate_module(&module, &table).to_string();
        assert!(!rendered.contains("set_on_text_change"), "{rendered}");
    }

    #[test]
    fn generates_valid_rust_for_async_action_with_nested_t_macro() {
        let module = viewmodel_module_from_rust(
            r#"
            mod file_view_model {
                struct FileViewModel {
                    #[observable(default = String::new())]
                    content: String,

                    #[observable(default = String::new())]
                    status: String,
                }

                impl FileViewModel {
                    async fn open(&self) {
                        if let Some(path) = platform::file_dialog::open().await {
                            content = std::fs::read_to_string(&path).unwrap_or_default();
                            status = t!("opened-status", name: content);
                        }
                    }
                }
            }
        "#,
        );
        let table = build_symbol_table(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("async_action", &generated);

        let generated_str = generated.to_string();
        assert!(generated_str.contains("elwindui :: core :: task :: spawn_local"));
        assert!(
            generated_str.contains("__self . content ()"),
            "t!(...) args inside an async action body must resolve through `__self`, not a \
             borrowed `self` that can't outlive the call:\n{generated_str}"
        );
        assert!(generated_str.contains("async"));
        assert!(generated_str.contains("elwindui :: i18n :: t"));
        assert!(
            !generated_str.contains("t !"),
            "t!(...) should have been rewritten, not left as a macro call"
        );
    }

    /// `Rectangle { fill: "#3a3a3c" }` (a real usage — see `examples/notepad/src/ui/
    /// rounded_panel.rs`) — `fill`/`stroke` are `Brush`-typed (painter design doc §18's
    /// `Option<String>` → `Option<Brush>` migration), so a hex string literal must be validated
    /// and converted to `Brush::Solid(Color::rgba(..))` at codegen time (`coerce_color_literal`)
    /// rather than spliced through unchanged.
    #[test]
    fn rectangle_fill_hex_literal_is_coerced_to_a_brush() {
        let src = r##"
            struct Foo {
                body: view! {
                    Rectangle {
                        fill: "#3a3a3c"
                        corner_radius: 8.0
                    }
                },
            }
        "##;
        let module = crate::test_module(&[(None, src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("rectangle_fill_literal", &generated);
        let generated_str = generated.to_string();
        assert!(
            generated_str.contains(
                "elwindui :: core :: graphics :: Brush :: Solid (elwindui :: core :: graphics :: Color :: rgba (58u8 , 58u8 , 60u8 , 255u8))"
            ),
            "{generated_str}"
        );
    }

    /// `coerce_color_literal` must reject a malformed hex literal at codegen time rather than
    /// spliced through as-is (which would only fail much later, confusingly, at real `rustc` type-
    /// checking or — worse — silently compile if `Brush`/`Color` ever gained a `From<&str>` impl).
    #[test]
    #[should_panic(expected = "invalid hex color literal")]
    fn malformed_fill_hex_literal_panics_at_codegen_time() {
        let src = r##"
            struct Foo {
                body: view! {
                    Rectangle {
                        fill: "#zzzzzz"
                    }
                },
            }
        "##;
        let module = crate::test_module(&[(None, src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let _ = generate_module(&module, &table);
    }

    /// `ContentControl inherits Control` (docs/specs/ui_spec.md#contentcontrol) — the
    /// `#[param] content` field is forwarded as a bare child into `ContentControl`'s single content
    /// slot via the
    /// `PASSTHROUGH_NODE`-tagged `lets_map` seeding in `generate_view`, and every `#[param]` field
    /// (not just `#[id(...)]` lets) gets a generated named accessor.
    #[test]
    fn generates_valid_rust_for_content_control() {
        let src = r#"
            struct Foo {
                body: view! {
                    ContentControl {
                        padding: 8.0
                        TextBlock { text: "hi" }
                    }
                },
            }
        "#;
        let module = crate::test_module(&[(None, src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("content_control", &generated);

        let generated_str = generated.to_string();
        // `ContentControl` is composed (docs/design/runtime/ui_tree_design.md) — its real struct is
        // always its own bare name (`ContentControlExt` is its auto-derived trait), so `Foo`'s own
        // generated code, resolving `ContentControl` as a child element, must construct that
        // concrete type (`emit_construction`'s `concrete_type_ident`).
        assert!(
            generated_str.contains("ContentControl :: new"),
            "{generated_str}"
        );

        // `ContentControl`'s own generated code (produced when `builtin_modules()` is fed through
        // `generate_module` directly, mirroring how a real consumer's own component
        // would be generated) forwards `content` into `ContentControl`'s content slot and exposes both
        // `#[param]` fields as public accessors. The builtin shape source bundled every builtin into one
        // module, so only `ContentControl`'s own `Item::Component`/`Item::View` pair is kept —
        // `generate_module` would otherwise also try (and fail) to generate every shape-only
        // builtin sharing that module (mirroring `compile_dir_impl`'s own filtering in `lib.rs`).
        let builtins_module = crate::test_builtin_modules()
            .into_iter()
            .find(|m| {
                m.items
                    .iter()
                    .any(|i| matches!(i, Item::Component(c) if c.name == "ContentControl"))
            })
            .expect("ContentControl should be a registered builtin");
        let content_control_module = Module {
            items: builtins_module
                .items
                .iter()
                .filter(|i| {
                    matches!(i, Item::Component(c) if c.name == "ContentControl")
                        || matches!(i, Item::View(v) if v.target == "ContentControl")
                })
                .cloned()
                .collect(),
            ..builtins_module
        };
        let content_control_code = generate_module(&content_control_module, &table);
        assert_valid_rust("content_control_impl", &content_control_code);
        let content_control_str = content_control_code.to_string();
        assert!(content_control_str.contains("elwindui :: core :: ui :: Control :: new"));
        // `content` is a `#[class]`-managed own (untagged) method, while `padding` is inherited from
        // `Control`. The class macro derives the matching trait declaration/impl and inherited
        // method surface at expansion time; it is intentionally not duplicated in these
        // pre-expansion generated tokens.
        assert!(
            content_control_str
                .contains("fn content (& self) -> std :: rc :: Rc < dyn UIElement >")
        );
        assert!(
            content_control_str
                .contains("elwindui :: class (inherits = elwindui :: core :: ui :: Control)"),
            "ContentControl must inherit Control's padding surface through #[class]: {content_control_str}"
        );
        // Real struct is always the bare `ContentControl` name itself — the *source* `#[class]` is
        // written against that same bare name (docs/design/runtime/ui_tree_design.md); the macro derives
        // its `ContentControlExt` trait alongside at expansion time — no `struct`/`trait` namespace
        // clash since the two are different identifiers.
        assert!(
            content_control_str
                .contains("elwindui :: class (inherits = elwindui :: core :: ui :: Control)"),
            "{content_control_str}"
        );
        assert!(
            content_control_str.contains("pub struct ContentControl"),
            "{content_control_str}"
        );
        // `#[class]` forwards `ControlExt` through its `__dyn_control` accessor.
        assert!(
            !content_control_str.contains("# [ancestor]"),
            "{content_control_str}"
        );
    }

    /// A bare nested child element with nowhere to go (no `children` field, no
    /// `#[content(field_name)]` on the component being constructed — `Button` has neither) is a hard
    /// codegen-time error: `build_component_args` requires an explicit content destination.
    #[test]
    #[should_panic(expected = "has no `children` field or `#[content(field_name)]`")]
    fn panics_on_bare_child_with_no_content_field_declared() {
        let src = r#"
            struct Foo {
                body: view! {
                    Button {
                        TextBlock { text: "not a valid Button child" }
                    }
                },
            }
        "#;
        let module = crate::test_module(&[(None, src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        generate_module(&module, &table);
    }

    /// `#[content(field_name)]` names a *single* slot — `MenuBarItem`'s `#[content(submenu)]` can
    /// bind one bare nested `Menu`, but a second one has nowhere to go (unlike a `children: Vec<_>`
    /// list, which happily takes any number).
    #[test]
    #[should_panic(expected = "can only bind a single nested child element")]
    fn panics_on_multiple_bare_children_for_a_single_content_field() {
        let src = r#"
            struct Foo {
                body: view! {
                    MenuBarItem {
                        text: "File"
                        Menu { }
                        Menu { }
                    }
                },
            }
        "#;
        let module = crate::test_module(&[(None, src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        generate_module(&module, &table);
    }

    /// A component inheriting a logical base (`ContentControl`) with no own ordinary view still
    /// receives the inherited ordinary composition view used by this synthetic builtin fixture.
    /// Typed `template: template_view!` declarations are intentionally not copied across the
    /// component boundary (`resolve_view_for` returns no inherited typed template).
    #[test]
    fn generates_valid_rust_for_inherited_ordinary_view_with_no_own_view() {
        let src = r#"
            struct LabeledPanel {
            }
        "#;
        let module =
            crate::test_module(&[(Some("ContentControl"), src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("labeled_panel_ordinary_view_inheritance", &generated);

        let generated_str = generated.to_string();
        // The compiled struct is always the bare `LabeledPanel` name itself — the *source* `#[class]`
        // is written against that same bare name (docs/design/runtime/ui_tree_design.md) — same reasoning
        // as `ContentControl`, and the macro derives `pub trait LabeledPanelExt: ..` itself at
        // expansion time, invisible in these pre-expansion generated tokens.
        assert!(
            generated_str
                .contains("elwindui :: class (inherits = elwindui :: ui :: ContentControl)"),
            "{generated_str}"
        );
        assert!(
            generated_str.contains("pub struct LabeledPanel"),
            "{generated_str}"
        );
        // Real base composition one level deeper than `ContentControl` itself: `LabeledPanel`
        // embeds a real `base: ContentControl` (built by calling `ContentControl`'s own
        // `construct(..)`), not a copy of `Control`'s construction — `Control::construct` only
        // ever appears in `ContentControl`'s *own* generated code (not exercised by this test, which
        // only generates `LabeledPanel`).
        assert!(
            generated_str.contains("base : elwindui :: ui :: ContentControl :: construct"),
            "{generated_str}"
        );
        // The constructor imports ContentControlExt to attach the inherited content through the
        // Visual collection after the outer node has an owner.
        assert!(
            generated_str.contains("ContentControlExt"),
            "{generated_str}"
        );
        // `#[class]` forwards `ContentControlExt` through `__dyn_content_control`.
        assert!(!generated_str.contains("# [ancestor]"), "{generated_str}");
    }

    /// `#[override] fn` + `base::name(...)` (§3): the derived's override calls into a
    /// `__base_<name>`-shadowed copy of the base body, and `on_mount { base::on_mount(); }`
    /// is spliced into `new()` chaining into the base's own `on_mount`.
    #[test]
    fn generates_valid_rust_for_method_override_and_on_mount_base_call() {
        let module = crate::test_module(&[
            (
                None,
                r#"
                struct Base {
                    body: view! {
                        on_mount {
                            println!("base mounted");
                        }
                        VerticalLayout { }
                    },
                }
                "#,
                Some(
                    r#"
                    impl Base {
                        #[overridable]
                        fn label(&self) -> String {
                            "base".to_string()
                        }
                    }
                    "#,
                ),
            ),
            (
                Some("Base"),
                r#"
                struct Derived {
                    body: view! {
                        on_mount {
                            base::on_mount();
                            println!("derived mounted");
                        }
                        VerticalLayout { }
                    },
                }
                "#,
                Some(
                    r#"
                    impl Derived {
                        #[overrides]
                        fn label(&self) -> String {
                            format!("{}!", base::label())
                        }
                    }
                    "#,
                ),
            ),
        ])
        .expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("method_override_and_on_mount", &generated);

        let generated_str = generated.to_string();
        assert!(generated_str.contains("fn __base_label"), "{generated_str}");
        assert!(
            generated_str.contains("fn __base_on_mount"),
            "{generated_str}"
        );
        // PR #165 review remediation, A2: `on_mount`/`on_unmount`'s `base::name()` rewriting now
        // uses `self` as the receiver (valid in every generated shape, unlike `this`, which is
        // only ever bound in some of them) — see `generate_view`'s own comment on `self_ident`.
        assert!(
            generated_str.contains("self . __base_on_mount"),
            "{generated_str}"
        );
    }

    /// PR #165 review remediation, A3: a `context_popup: view! { .. }` nested inside *another*
    /// `context_popup: view! { .. }` must keep the *original source* Component as the lexical
    /// owner for both levels — not the first level's own generated hidden Component. Proven at the
    /// codegen level (not merely by end-to-end runtime behavior, `context_menu_and_popup.rs`'s own
    /// `declarative_context_popup_nested_popup_observes_current_outer_value`) by inspecting the
    /// generated `Weak<..>` field type on *both* hidden components' own struct definitions.
    #[test]
    fn nested_deferred_view_keeps_the_original_source_component_as_lexical_owner() {
        let mut module = crate::test_module(&[(
            None,
            r#"
            struct NestedPopupOwner {
                #[state(default = "outer".to_string())]
                value: String,
                body: view! {
                    TextBlock {
                        text: "Open popup",
                        context_popup: view! {
                            TextBlock {
                                text: "inner",
                                context_popup: view! {
                                    TextBlock { text: value }
                                },
                            }
                        },
                    }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        let pre_lowering_table = build_symbol_table(&[module.clone()]);
        let implicit_owner_schema =
            implicit_owner_schema(&pre_lowering_table, &module, "NestedPopupOwner");
        crate::lower_deferred_views_in_module(
            &mut module,
            "NestedPopupOwner",
            &implicit_owner_schema,
        );

        // Two hidden components must have been synthesized — one per `context_popup: view! { .. }`.
        let hidden_names: Vec<&str> = module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Component(c) if c.name.starts_with("__ElwinduiViewTemplateInstance") => {
                    Some(c.name.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            hidden_names.len(),
            2,
            "expected exactly 2 hidden components, got {hidden_names:?}"
        );

        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("nested_deferred_view_owner", &generated);
        let generated_str = generated.to_string();

        // Both hidden components' own `__view_owner` field (and its constructor/downcast plumbing)
        // must be typed against the *original* source Component (`NestedPopupOwner`) — never
        // against the other hidden component's own generated name. `Weak < NestedPopupOwner >`
        // must appear at least once per hidden component (each mentions it more than once —
        // struct field, `__new_unmounted` parameter, weak-upgrade downcast target — so this checks
        // a lower bound, not an exact count) — and no hidden-component name may appear as the
        // argument of a `Weak < .. >` anywhere (which would mean one hidden Component's own
        // `__view_owner` field was incorrectly typed against the *other* hidden Component).
        let owner_weak_count = generated_str.matches("Weak < NestedPopupOwner >").count();
        assert!(
            owner_weak_count >= 2,
            "expected both hidden components' __view_owner field to be Weak<NestedPopupOwner>, \
             got only {owner_weak_count} occurrence(s) in:\n{generated_str}"
        );
        for hidden_name in &hidden_names {
            let bad = format!("Weak < {hidden_name} >");
            assert!(
                !generated_str.contains(&bad),
                "a hidden component's own generated name must never appear as another deferred \
                 view's lexical owner type ({bad} found):\n{generated_str}"
            );
        }
    }

    /// PR #165 rereview remediation round 2, A2-T8: a deferred view's `on_update` block is
    /// rewritten through the same scope-aware `ViewClosureRewriter`/`rewrite_view_closure_block`
    /// machinery as `on_mount`/`on_unmount`/event closures — proven by inspecting the generated
    /// source directly, since a lowered hidden Component structurally has no own `#[prop]`/
    /// `#[state]`/`#[computed]`/`#[environment]` field to ever trigger `on_update`'s own
    /// `subscribe_property_changed` dispatch with at runtime (`DeferredViewBody` carries only
    /// `on_mount`/`on_unmount`/`on_update`/`lets`/`root` — no field declarations at all), making
    /// runtime firing unreachable by construction for *every* deferred view, not just this test's
    /// own fixture (see `declarative_context_popup_direct_on_update_compiles_and_resolves_the_
    /// enclosing_owner_field` in `context_menu_and_popup.rs` for the compile/construct-time
    /// counterpart this codegen-level test complements).
    #[test]
    fn deferred_view_on_update_block_resolves_unshadowed_names_through_the_implicit_owner() {
        let mut module = crate::test_module(&[(
            None,
            r#"
            struct A2OnUpdateOwner {
                #[state(default = "outer".to_string())]
                label: String,
                #[param]
                log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
                body: view! {
                    TextBlock {
                        text: "target",
                        context_popup: view! {
                            on_update: {
                                // Unshadowed — no local binds `label` anywhere in this block, so
                                // this must resolve through the implicit lexical owner.
                                log.borrow_mut().push(label.clone());
                            }
                            TextBlock { text: "popup" }
                        },
                    }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        let pre_lowering_table = build_symbol_table(&[module.clone()]);
        let implicit_owner_schema =
            implicit_owner_schema(&pre_lowering_table, &module, "A2OnUpdateOwner");
        crate::lower_deferred_views_in_module(
            &mut module,
            "A2OnUpdateOwner",
            &implicit_owner_schema,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("a2_t8_on_update_scope", &generated);
        let generated_str = generated.to_string();

        assert!(
            generated_str.contains("subscribe_property_changed"),
            "expected an on_update subscription to be generated: {generated_str}"
        );
        // The unshadowed `label` inside on_update's own block must have been rewritten into an
        // implicit-owner getter call (`<owner>.label()`) — proving on_update's block is routed
        // through the same scope-aware `rewrite_view_closure_block`/`ViewClosureRewriter` machinery
        // as on_mount/on_unmount/event closures, not spliced raw (its pre-A2 behavior, when a bare
        // `label` reference here would have failed to compile at all — there being no local of
        // that name in scope).
        assert!(
            generated_str.contains(". label ()"),
            "expected on_update's own unshadowed `label` reference to become an implicit-owner \
             getter call (`<owner>.label()`): {generated_str}"
        );
    }

    /// PR #165 rereview remediation round 2, A4-T5: `emit_external_attribute_sets` — the real
    /// production path for a `DeferredView` targeting a real builtin (`TextBlock`, `Window`, ...,
    /// none of which have a local `TypeInfo` for `elwindui-codegen` to check against directly) —
    /// must route the built factory through `__coerce_deferred_view_assignment_target::<@field_type
    /// ..>(..)`, never through an unconditional `Some(..)` wrap regardless of the target's real
    /// declared type. Regression test for the exact round-1 defect: a real builtin property
    /// declared bare `ViewTemplate` (not `Option<ViewTemplate>`) would have compiled (the round-1
    /// assertion only checked the type, never converted the value) and then failed at the
    /// generated setter call with a confusing type mismatch.
    ///
    /// `TextBlock` deliberately has no local `TypeInfo` here (the symbol table is built *without*
    /// chaining `test_builtin_modules()`, unlike every other codegen test in this module) — this
    /// is what actually forces `emit_external_attribute_sets` rather than the local-`TypeInfo`
    /// path (`build_virtual_value`/`build_component_setters`) every *other* codegen test in this
    /// module exercises via `test_builtin_modules()`'s own local shape table for `TextBlock`.
    #[test]
    fn external_deferred_view_target_uses_coercion_not_unconditional_some() {
        let mut module = crate::test_module(&[(
            None,
            r#"
            struct A4ExternalDeferredViewOwner {
                body: view! {
                    TextBlock {
                        text: "target",
                        context_popup: view! {
                            TextBlock { text: "popup" }
                        },
                    }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        // Deliberately `build_symbol_table` (not `build_symbol_table_with_builtins`) — no chained
        // builtin modules, so `TextBlock` has no local `TypeInfo` and `context_popup`'s value must
        // go through `emit_external_attribute_sets`, the real-builtin code path. Reused for the
        // pre-lowering schema table too, mirroring the production pipeline.
        let pre_lowering_table = build_symbol_table(&[module.clone()]);
        let implicit_owner_schema =
            implicit_owner_schema(&pre_lowering_table, &module, "A4ExternalDeferredViewOwner");
        crate::lower_deferred_views_in_module(
            &mut module,
            "A4ExternalDeferredViewOwner",
            &implicit_owner_schema,
        );
        let table = build_symbol_table(&[module.clone()]);
        let generated = generate_module(&module, &table);
        let generated_str = generated.to_string();

        assert!(
            generated_str.contains("__coerce_deferred_view_assignment_target"),
            "expected the external DeferredView target to be converted via \
             __coerce_deferred_view_assignment_target: {generated_str}"
        );
        assert!(
            generated_str.contains("@ field_type context_popup"),
            "expected the coercion's own type parameter to be read through @field_type \
             context_popup: {generated_str}"
        );
        // The old, round-1 defect: the built factory wrapped unconditionally in `Some(..)` before
        // the declared type was ever consulted, entirely independent of the coercion call above.
        assert!(
            !generated_str.contains("Some (elwindui :: core :: ui :: ViewTemplate :: new"),
            "the external DeferredView branch must not unconditionally wrap the factory in \
             Some(..) — the target's real declared type must decide the shape: {generated_str}"
        );
    }

    /// PR #165 final rereview remediation, A2 (A2-R1/R2/R3): the implicit-owner fallback is now
    /// schema-driven (`ImplicitOwnerCtx::readable_fields`), not "any unshadowed bare name falls
    /// back to the owner" — this proves all three membership outcomes in one fixture: an ordinary
    /// free Rust name unrelated to the source Component (R1) and a literal `None` (R2) must both
    /// remain untouched ordinary Rust, while a real source-owner field (R3, `label`) must still
    /// resolve through the owner.
    #[test]
    fn deferred_view_on_mount_free_names_stay_rust_while_owner_fields_still_resolve() {
        let mut module = crate::test_module(&[(
            None,
            r#"
            struct A2SchemaFreeNameOwner {
                #[state(default = "outer".to_string())]
                label: String,
                body: view! {
                    TextBlock {
                        text: "target",
                        context_popup: view! {
                            on_mount {
                                let _ = A2_R1_FREE_CONST;
                                let maybe: Option<i32> = None;
                                let _ = maybe.is_none();
                                let _ = label.clone();
                            }
                            TextBlock { text: "popup" }
                        },
                    }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        let pre_lowering_table = build_symbol_table(&[module.clone()]);
        let implicit_owner_schema =
            implicit_owner_schema(&pre_lowering_table, &module, "A2SchemaFreeNameOwner");
        crate::lower_deferred_views_in_module(
            &mut module,
            "A2SchemaFreeNameOwner",
            &implicit_owner_schema,
        );
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("a2_r1_r2_r3_free_names", &generated);
        let generated_str = generated.to_string();

        // A2-R1: a free Rust name unrelated to the source Component must never become a
        // `__view_owner` getter call — only checked as `<something> . A2_R1_FREE_CONST ()`, the
        // shape `resolved_implicit_owner_field`'s old, unconditional fallback would have produced.
        assert!(
            !generated_str.contains(". A2_R1_FREE_CONST ()"),
            "a free Rust name outside the source Component's own schema must not be rewritten \
             into an implicit-owner getter call: {generated_str}"
        );
        assert!(
            generated_str.contains("A2_R1_FREE_CONST"),
            "the free name should still appear, unrewritten: {generated_str}"
        );
        // A2-R2: `None` must remain the real `Option::None`, never rewritten into a getter call.
        assert!(
            generated_str.contains("let maybe : Option < i32 > = None ;"),
            "`None` must remain untouched: {generated_str}"
        );
        assert!(
            !generated_str.contains(". None ()"),
            "`None` must never be rewritten into an implicit-owner getter call: {generated_str}"
        );
        // A2-R3: a real source-owner field must still resolve through the owner, exactly as before
        // this schema was added.
        assert!(
            generated_str.contains(". label ()"),
            "a real source-owner field must still resolve as an owner getter call: {generated_str}"
        );
    }

    /// PR #165 final rereview remediation, A2-R6: `implicit_owner_schema` must classify each
    /// `FieldKind` exactly per its own doc comment — `Prop`/`State` readable+writable, `Param`/
    /// `Computed`/`Environment` readable-only, `Attached` excluded entirely. Inspects the schema
    /// value directly rather than generated code, since this is testing the classification rule
    /// itself, not any particular consumer of it.
    #[test]
    fn implicit_owner_schema_classifies_every_field_kind_correctly() {
        let module = crate::test_module(&[(
            None,
            r#"
            struct A2SchemaKindsOwner {
                #[state(default = 0i32)]
                state_field: i32,
                prop_field: i32,
                #[param]
                param_field: i32,
                #[computed(expr = state_field + 1)]
                computed_field: i32,
                #[environment(some_key)]
                environment_field: i32,
                #[attached(default = 0)]
                attached_field: i32,
            }
            "#,
            None,
        )])
        .expect("should parse");
        let table = build_symbol_table(&[module.clone()]);
        let schema = implicit_owner_schema(&table, &module, "A2SchemaKindsOwner");

        assert_eq!(schema.field_name, "__view_owner");
        for readable in [
            "state_field",
            "prop_field",
            "param_field",
            "computed_field",
            "environment_field",
        ] {
            assert!(
                schema.readable_fields.contains(readable),
                "{readable} should be readable: {:?}",
                schema.readable_fields
            );
        }
        assert!(
            !schema.readable_fields.contains("attached_field"),
            "an #[attached] field is not real instance data of its declaring component and must \
             not be readable: {:?}",
            schema.readable_fields
        );
        for writable in ["state_field", "prop_field"] {
            assert!(
                schema.writable_fields.contains(writable),
                "{writable} should be writable: {:?}",
                schema.writable_fields
            );
        }
        for not_writable in [
            "param_field",
            "computed_field",
            "environment_field",
            "attached_field",
        ] {
            assert!(
                !schema.writable_fields.contains(not_writable),
                "{not_writable} must not be writable: {:?}",
                schema.writable_fields
            );
        }
    }

    /// PR #165 final rereview remediation, A2-R7: the schema must be derived from the source
    /// Component's *effective* fields (`resolve_effective_fields`, inherited fields included), not
    /// merely its own literal `ComponentDef::fields` — an inherited base field must still be
    /// readable/writable through a derived Component's own deferred views. The base field is
    /// declared plain (`FieldKind::Prop`, not `#[state]`): `resolve_effective_fields` deliberately
    /// never inherits a `#[state]` field at all (`FieldKind::State`'s own doc comment — state is
    /// private to the exact component that declares it, never part of a derived component's own
    /// effective fields), so a `#[state]` fixture here would test that unrelated, pre-existing rule
    /// instead of A2's own schema-derivation logic. The base field is also referenced by a *literal
    /// bare forward* in the derived Component's own outer view (`text: shared_label`), not only
    /// inside the nested deferred view — `resolve_effective_fields` treats a deferred view as a
    /// dependency boundary, never itself counting as a forwarding reference (`view_expr_references_
    /// bare_name`'s own `ViewExpr::DeferredView(_) => false` arm, Issue #162 §3.9), so the outer
    /// forward is what actually makes `shared_label` part of `A2InheritedFieldDerived`'s own
    /// effective fields in the first place — independent of, and prior to, whatever the deferred
    /// view inside it goes on to do with that same name.
    #[test]
    fn implicit_owner_schema_includes_inherited_effective_fields() {
        let mut module = crate::test_module(&[
            (
                None,
                r#"
                struct A2InheritedFieldBase {
                    shared_label: String,
                }
                "#,
                None,
            ),
            (
                Some("A2InheritedFieldBase"),
                r#"
                struct A2InheritedFieldDerived {
                    body: view! {
                        TextBlock {
                            text: shared_label,
                            context_popup: view! {
                                TextBlock { text: shared_label }
                            },
                        }
                    },
                }
                "#,
                None,
            ),
        ])
        .expect("should parse");

        let pre_lowering_table = build_symbol_table_with_builtins(&[module.clone()]);
        let schema = implicit_owner_schema(&pre_lowering_table, &module, "A2InheritedFieldDerived");
        assert!(
            schema.readable_fields.contains("shared_label"),
            "an inherited base field (not literally declared on the derived Component's own \
             ComponentDef::fields) must still be part of the derived source Component's readable \
             schema, derived from its *effective* fields: {:?}",
            schema.readable_fields
        );
        assert!(
            schema.writable_fields.contains("shared_label"),
            "an inherited #[state] field must remain writable too: {:?}",
            schema.writable_fields
        );

        crate::lower_deferred_views_in_module(&mut module, "A2InheritedFieldDerived", &schema);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("a2_r7_inherited_field", &generated);
        let generated_str = generated.to_string();
        assert!(
            generated_str.contains("Weak < A2InheritedFieldDerived >"),
            "the hidden component's lexical owner must be the derived Component itself (whose \
             *effective* fields include the inherited one): {generated_str}"
        );
        assert!(
            generated_str.contains(". shared_label ()"),
            "the inherited field must resolve through the derived owner's own generated getter: \
             {generated_str}"
        );
    }

    /// PR #165 final rereview remediation, A2-R8: nested `DeferredView`s must reuse the *same*
    /// readable/writable schema at every nesting depth (never recomputed from a nested level's own
    /// synthetic, effectively field-less hidden Component) — the second-nesting-level counterpart
    /// to `nested_deferred_view_keeps_the_original_source_component_as_lexical_owner`'s own
    /// lexical-owner-*type* proof, this proves the lexical-owner *schema* survives nesting too.
    #[test]
    fn nested_deferred_view_reuses_the_same_source_readable_schema_at_every_level() {
        let mut module = crate::test_module(&[(
            None,
            r#"
            struct A2NestedSchemaOwner {
                #[state(default = "outer".to_string())]
                value: String,
                body: view! {
                    TextBlock {
                        text: "outer",
                        context_popup: view! {
                            TextBlock {
                                text: "inner",
                                context_popup: view! {
                                    on_mount {
                                        let _ = A2_NESTED_FREE_NAME;
                                    }
                                    TextBlock { text: value }
                                },
                            }
                        },
                    }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        let pre_lowering_table = build_symbol_table_with_builtins(&[module.clone()]);
        let schema = implicit_owner_schema(&pre_lowering_table, &module, "A2NestedSchemaOwner");
        crate::lower_deferred_views_in_module(&mut module, "A2NestedSchemaOwner", &schema);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("a2_r8_nested_schema", &generated);
        let generated_str = generated.to_string();

        assert!(
            generated_str.contains(". value ()"),
            "second-level nested deferred view must still resolve a known source-owner field: \
             {generated_str}"
        );
        assert!(
            !generated_str.contains(". A2_NESTED_FREE_NAME ()"),
            "second-level nested deferred view must not fall back to the owner for a name \
             outside its schema: {generated_str}"
        );
        assert!(
            generated_str.contains("A2_NESTED_FREE_NAME"),
            "the free name should still appear, unrewritten: {generated_str}"
        );
    }

    /// PR #165 post-final rereview remediation, A8/T27: a direct, source-qualified 2-segment path
    /// (`vm.label`) written straight inside a lowered `DeferredView` — with no intermediate nested
    /// Component to bridge it — must build through the source lexical owner
    /// (`__view_owner.upgrade().vm().label()`), never as `self.vm.label()` on the hidden Component
    /// (which has no physical `vm` field at all — that shape does not even parse against the real
    /// generated struct, a genuine `rustc` "no field `vm`" error `assert_valid_rust`'s
    /// `syn::parse2`-only check cannot catch, only `cargo build`/`cargo test` on the real crate can).
    #[test]
    fn deferred_view_direct_qualified_source_path_builds_through_source_owner() {
        let mut module = viewmodel_and_component_module(
            r#"
            #[elwindui::viewmodel]
            mod t27_vm_mod {
                struct T27Vm {
                    #[observable(default = String::new())]
                    label: String,
                }
            }
            "#,
            None,
            r#"
            struct T27Owner {
                #[bindable]
                vm: std::rc::Rc<T27Vm>,
                body: view! {
                    TextBlock {
                        text: "target",
                        context_popup: view! {
                            TextBlock { text: vm.label }
                        },
                    }
                },
            }
            "#,
        );
        let pre_lowering_table = build_symbol_table(&[module.clone()]);
        let schema = implicit_owner_schema(&pre_lowering_table, &module, "T27Owner");
        assert!(
            schema.bindable_fields.contains("vm"),
            "a #[bindable] field must be part of the source schema's bindable_fields: {:?}",
            schema.bindable_fields
        );
        crate::lower_deferred_views_in_module(&mut module, "T27Owner", &schema);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("t27_direct_qualified_source_path", &generated);
        let generated_str = generated.to_string();

        // `T27Owner` itself has a real `vm` field, so `self . vm` legitimately appears in *its own*
        // generated code (its own `vm()` accessor, its own `#[bindable]` subscription) — the bug
        // this test guards against is specific to the *hidden* Component's own generated section,
        // which begins at its own struct name and has no `vm` field of its own at all.
        let hidden_section_start = generated_str
            .find("struct __ElwinduiViewTemplateInstanceForT27Owner_1")
            .expect("a hidden Component must have been generated for the one context_popup");
        let hidden_section = &generated_str[hidden_section_start..];
        assert!(
            !hidden_section.contains("self . vm"),
            "a source-qualified path must never be emitted as a nonexistent physical `self.vm` \
             field access on the hidden Component: {hidden_section}"
        );
        assert!(
            hidden_section.contains(". vm () . label ()"),
            "the source-qualified path must bridge through the source lexical owner's own `vm()` \
             getter: {hidden_section}"
        );
    }

    /// PR #165 post-final rereview remediation, A9/T34: a *second*-nesting-level `DeferredView`'s
    /// own direct source-qualified path must still bridge through the *original* source
    /// Component's own `vm`, not the first-level hidden Component's (which has no `vm` field of
    /// its own either) — the schema/bindable-bridge counterpart to `nested_deferred_view_keeps_
    /// the_original_source_component_as_lexical_owner`'s lexical-owner-*type* proof.
    #[test]
    fn nested_deferred_view_direct_qualified_source_path_uses_the_original_source_owner() {
        let mut module = viewmodel_and_component_module(
            r#"
            #[elwindui::viewmodel]
            mod t34_vm_mod {
                struct T34Vm {
                    #[observable(default = String::new())]
                    label: String,
                }
            }
            "#,
            None,
            r#"
            struct T34Owner {
                #[bindable]
                vm: std::rc::Rc<T34Vm>,
                body: view! {
                    TextBlock {
                        text: "outer",
                        context_popup: view! {
                            TextBlock {
                                text: "inner",
                                context_popup: view! {
                                    TextBlock { text: vm.label }
                                },
                            }
                        },
                    }
                },
            }
            "#,
        );
        let pre_lowering_table = build_symbol_table(&[module.clone()]);
        let schema = implicit_owner_schema(&pre_lowering_table, &module, "T34Owner");
        crate::lower_deferred_views_in_module(&mut module, "T34Owner", &schema);
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("t34_nested_direct_qualified_source_path", &generated);
        let generated_str = generated.to_string();

        // `T34Owner` itself has a real `vm` field, so `self . vm` legitimately appears in *its own*
        // generated code — only the hidden components' own generated sections must never contain
        // it. Lowering recurses into a `DeferredView`'s own body *before* pushing its own hidden
        // Component/View pair, so the *inner* (second-level, `_2`) hidden component's own generated
        // section actually precedes the outer (`_1`) one in emission order — take whichever struct
        // declaration appears first so the slice always starts right after `T34Owner`'s own code,
        // regardless of that internal emission-order detail.
        let hidden_section_start = [
            "struct __ElwinduiViewTemplateInstanceForT34Owner_1",
            "struct __ElwinduiViewTemplateInstanceForT34Owner_2",
        ]
        .iter()
        .filter_map(|marker| generated_str.find(marker))
        .min()
        .expect("both hidden Components must have been generated for the nested context_popup");
        let hidden_section = &generated_str[hidden_section_start..];
        assert!(
            !hidden_section.contains("self . vm"),
            "no level's hidden Component ever has a physical `vm` field: {hidden_section}"
        );
        assert!(
            hidden_section.contains(". vm () . label ()"),
            "the second-level deferred view's own qualified path must still bridge through the \
             original source Component's own `vm()`: {hidden_section}"
        );
        assert!(
            hidden_section.matches("Weak < T34Owner >").count() >= 2,
            "both hidden components' own lexical-owner field must stay typed against the \
             original source Component: {hidden_section}"
        );
    }

    /// PR #165 review remediation, A6/T26: the generated `mount_override`'s close-request-handler
    /// closure must capture only a type-erased `Weak<dyn Any>` (`__weak_self_erased`, cloned from
    /// this component's own `__self_weak`), never a strong `Rc<Self>` — a strong capture would
    /// keep the generated Window alive for as long as the backend's own native close-request
    /// storage does, defeating the acyclic-ownership discipline every owner/callback capture in
    /// this codebase follows. Also proves `unmount_override` clears the handler
    /// (`set_close_request_handler(&self.base, None)`) *before* delegating to the backend's own
    /// `unmount_override` — a stale handler must never be reachable once the Window starts
    /// tearing down. Cannot be proven by constructing a real Window (native construction needs the
    /// main thread, unavailable in any `#[test]` harness — see `window_mount_hide_close.rs`'s own
    /// established type-check-only convention for that reason), so this inspects the generated
    /// Rust source directly instead.
    #[test]
    fn window_mount_override_close_handler_captures_only_a_weak_self_reference() {
        let module = crate::test_module(&[(
            Some("Window"),
            r#"
            struct T26TestWindow {
                body: view! {
                    title: "T26"
                    content: VerticalLayout { }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("t26_window_mount_override", &generated);
        let generated_str = generated.to_string();

        assert!(
            generated_str.contains("fn mount_override"),
            "{generated_str}"
        );
        // The handler closure captures `__weak_self_erased` (a `Weak` clone) — never
        // `Rc::clone(self)`/`self.clone()`/an owned `Rc<Self>` moved in directly.
        assert!(
            generated_str
                .contains("let __weak_self_erased = self . __self_weak . borrow () . clone ()"),
            "expected the close-request handler to capture a weak self reference: {generated_str}"
        );
        assert!(
            !generated_str.contains("std :: rc :: Rc :: clone (self)")
                && !generated_str.contains("std :: rc :: Rc :: clone (& self)"),
            "the close-request handler must never capture a strong Rc<Self>: {generated_str}"
        );

        // `unmount_override` clears the handler before forwarding to the backend.
        let unmount_override_pos = generated_str
            .find("fn unmount_override")
            .expect("generated code should contain fn unmount_override");
        let unmount_override_body = &generated_str[unmount_override_pos..];
        let clear_pos = unmount_override_body
            .find("set_close_request_handler (& self . base , None)")
            .expect("unmount_override should clear the close-request handler");
        let base_unmount_pos = unmount_override_body
            .find("self . base . unmount_override ()")
            .expect("unmount_override should forward to the backend's own unmount_override");
        assert!(
            clear_pos < base_unmount_pos,
            "the close-request handler must be cleared *before* forwarding to the backend's own \
             unmount_override, not after: {unmount_override_body}"
        );
    }

    /// PR #165 rereview remediation round 2, A6: parses `generated` as a `syn::File` and returns
    /// the pretty-printed source of the *first* `fn #method_name` found in *any* `impl` block
    /// (an `ImplItemFn`), for deterministic ordering assertions against the real generated Window
    /// lifecycle methods — real `NSWindow`/native-Window construction needs the main thread
    /// (unavailable in any `#[test]` harness, see `window_mount_hide_close.rs`'s own established
    /// type-check-only convention), so ordering must be proven by inspecting what the generator
    /// actually emits rather than by observing a constructed Window at runtime. Every T17-T21
    /// fixture below declares exactly one component, so a single unqualified match is
    /// unambiguous; a multi-component fixture would need to disambiguate by enclosing `impl`
    /// target, which none of these do.
    fn generated_method_body(generated: &TokenStream, method_name: &str) -> String {
        let file: syn::File = syn::parse2(generated.clone())
            .unwrap_or_else(|e| panic!("generated code should parse as a file: {e}\n{generated}"));
        struct Finder<'a> {
            name: &'a str,
            found: Vec<String>,
        }
        impl<'a, 'ast> Visit<'ast> for Finder<'a> {
            fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
                if node.sig.ident == self.name {
                    self.found.push(quote! { #node }.to_string());
                }
                syn::visit::visit_impl_item_fn(self, node);
            }
        }
        let mut finder = Finder {
            name: method_name,
            found: Vec::new(),
        };
        finder.visit_file(&file);
        finder.found.into_iter().next().unwrap_or_else(|| {
            panic!("method `{method_name}` not found in generated code:\n{generated}")
        })
    }

    /// Shared host-composition (`inherits Window`) fixture for the A6/T17-T21 deterministic
    /// lifecycle-ordering tests below — a single minimal component, so `generated_method_body`'s
    /// unqualified match stays unambiguous.
    fn generate_t17_t21_window_module() -> TokenStream {
        let module = crate::test_module(&[(
            Some("Window"),
            r#"
            struct T17T21TestWindow {
                body: view! {
                    title: "T17-T21"
                    content: VerticalLayout { }
                },
            }
            "#,
            None,
        )])
        .expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("t17_t21_window", &generated);
        generated
    }

    /// PR #165 final rereview remediation, A2's own §8.3: like `generated_method_body`, but scoped
    /// to a specific generated type's own `impl` block — required once a fixture declares more than
    /// one component (T17's own completed child-mount-before-user-on_mount proof, below, needs a
    /// real generated child Component in the same fixture as the Window, so `generated_method_body`'s
    /// "first match in any impl" convention is no longer unambiguous between the two types' own
    /// same-named methods). Fails loudly if zero or more than one exact `(impl_target, method_name)`
    /// match is found, rather than silently picking the first — same "prove a real position, don't
    /// assume" discipline `generated_method_body` already applies to method selection alone.
    fn generated_impl_method_body(
        generated: &TokenStream,
        impl_target: &str,
        method_name: &str,
    ) -> String {
        let file: syn::File = syn::parse2(generated.clone())
            .unwrap_or_else(|e| panic!("generated code should parse as a file: {e}\n{generated}"));
        struct Finder<'a> {
            impl_target: &'a str,
            method_name: &'a str,
            current_impl_target: Option<String>,
            found: Vec<String>,
        }
        impl<'a, 'ast> Visit<'ast> for Finder<'a> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                let self_ty = &node.self_ty;
                let previous = self
                    .current_impl_target
                    .replace(quote! { #self_ty }.to_string());
                syn::visit::visit_item_impl(self, node);
                self.current_impl_target = previous;
            }
            fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
                if node.sig.ident == self.method_name
                    && self.current_impl_target.as_deref() == Some(self.impl_target)
                {
                    self.found.push(quote! { #node }.to_string());
                }
                syn::visit::visit_impl_item_fn(self, node);
            }
        }
        let mut finder = Finder {
            impl_target,
            method_name,
            current_impl_target: None,
            found: Vec::new(),
        };
        finder.visit_file(&file);
        match finder.found.len() {
            1 => finder.found.into_iter().next().unwrap(),
            0 => panic!(
                "no `impl {impl_target} {{ fn {method_name} }}` found in generated code:\n{generated}"
            ),
            n => panic!(
                "expected exactly one `impl {impl_target} {{ fn {method_name} }}`, found {n} in \
                 generated code:\n{generated}"
            ),
        }
    }

    /// A6/T17 completion (PR #165 final rereview remediation, A2's own §8): the accepted portion of
    /// T17 (`t17_generated_mount_orders_state_then_mount_override_then_build_view`, above) only
    /// proved `Mounted < mount_override < __build_view`, relying on a comment for the second half of
    /// the contract's own required ordering (`mount_override < child mount < user on_mount`) — this
    /// completes it with a real assertion against the generated `__build_view` body of a Window that
    /// actually has a real generated child Component (`T17Child`) as its own `content`, and a unique
    /// marker call inside its own direct user `on_mount`.
    #[test]
    fn t17_generated_build_view_mounts_child_before_user_on_mount() {
        let module = crate::test_module(&[
            (
                None,
                r#"
                struct T17Child {
                    body: view! {
                        TextBlock { text: "child" }
                    },
                }
                "#,
                None,
            ),
            (
                Some("Window"),
                r#"
                struct T17ChildMountWindow {
                    body: view! {
                        on_mount {
                            t17_owner_on_mount_marker();
                        }
                        title: "T17-child-mount"
                        content: T17Child { }
                    },
                }
                "#,
                None,
            ),
        ])
        .expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("t17_child_mount_window", &generated);

        let build_view_body =
            generated_impl_method_body(&generated, "T17ChildMountWindow", "__build_view");

        let child_mount_pos = build_view_body
            .find("T17Child :: new")
            .expect("__build_view should construct (and thereby mount) the real child Component");
        let user_on_mount_pos = build_view_body
            .find("t17_owner_on_mount_marker ()")
            .expect("__build_view should splice the user's own on_mount marker call");

        assert!(
            child_mount_pos < user_on_mount_pos,
            "the generated __build_view must construct/mount the real child Component before \
             splicing the user's own on_mount body — completing T17's full required ordering \
             (mount_override < __build_view < child mount < user on_mount): {build_view_body}"
        );

        // Combine with the already-accepted first half of T17 for the complete chain in one place.
        // Target-qualified (not `generated_method_body`'s unqualified first match) since this
        // fixture, unlike T17-T21's own single-component one, declares two components — an
        // unqualified search could otherwise silently match `T17Child`'s own unrelated `mount()`.
        let mount_body = generated_impl_method_body(&generated, "T17ChildMountWindow", "mount");
        let state_pos = mount_body
            .find("ComponentLifecycleState :: Mounted")
            .expect("mount() should set the lifecycle state to Mounted");
        let override_pos = mount_body
            .find("WindowExt > :: mount_override")
            .expect("mount() should call mount_override");
        let build_pos = mount_body
            .find("__build_view (")
            .expect("mount() should call __build_view()");
        assert!(
            state_pos < override_pos && override_pos < build_pos,
            "mount() must order state = Mounted, then mount_override, then __build_view: \
             {mount_body}"
        );
    }

    /// A6/T17: first-mount ordering. Proves the generated `mount()` body sets the lifecycle state
    /// to `Mounted` *before* calling `mount_override(environment)`, which in turn happens *before*
    /// `__build_view()` (content construction, wiring, and — inside `__build_view` itself — the
    /// user's own `on_mount`, spliced in last, after every construction/wiring/subscribe step —
    /// see `generate_view`'s own splice order, unchanged by this delta).
    #[test]
    fn t17_generated_mount_orders_state_then_mount_override_then_build_view() {
        let generated = generate_t17_t21_window_module();
        let mount_body = generated_method_body(&generated, "mount");

        let state_pos = mount_body
            .find("ComponentLifecycleState :: Mounted")
            .expect("mount() should set the lifecycle state to Mounted");
        let override_pos = mount_body
            .find("WindowExt > :: mount_override")
            .expect("mount() should call mount_override");
        let build_pos = mount_body
            .find("__build_view (")
            .expect("mount() should call __build_view()");

        assert!(
            state_pos < override_pos,
            "lifecycle state must be set to Mounted before mount_override is called: {mount_body}"
        );
        assert!(
            override_pos < build_pos,
            "mount_override must be called before __build_view(): {mount_body}"
        );
        // Exactly one call in the generated mount() body — mount() itself is guarded (`if state
        // != Created { panic }`) so a real second mount() call never reaches this point again.
        assert_eq!(
            mount_body.matches("WindowExt > :: mount_override").count(),
            1,
            "mount_override must be called exactly once from mount(): {mount_body}"
        );
    }

    /// A6/T18: `show()`/`hide()`/`show()` structural ordering. Proves `hide()` never mounts,
    /// unmounts, or rebuilds — its own generated body contains no lifecycle-state transition, no
    /// `unmount`, and no `__build_view`/`mount(` call — so a second `show()` after `hide()` (which
    /// re-enters `show()`'s own `if self.__mount_environment.get().is_none()` guard, already
    /// `Some` after the first mount) structurally cannot rebuild either.
    #[test]
    fn t18_generated_hide_never_mounts_unmounts_or_rebuilds() {
        let generated = generate_t17_t21_window_module();
        let show_body = generated_method_body(&generated, "show");
        let hide_body = generated_method_body(&generated, "hide");

        assert!(
            show_body.contains("__mount_environment . get () . is_none ()"),
            "show() must only mount when not already mounted: {show_body}"
        );
        for forbidden in ["unmount", "__build_view", "ComponentLifecycleState"] {
            assert!(
                !hide_body.contains(forbidden),
                "hide() must never {forbidden}: {hide_body}"
            );
        }
        assert!(
            hide_body.contains("self . base . hide ()"),
            "hide() must forward to the backend: {hide_body}"
        );
    }

    /// A6/T19: programmatic `close()` ordering. Proves the generated `close()` body orders its own
    /// idempotency guard before `self.unmount()`, which itself happens before `self.base.close()`
    /// (the real native close) — and that the generated `unmount()` body orders the lifecycle
    /// transition to `Unmounting` before `unmount_override()` (closes any active popup — Issue
    /// #162 §3.18), before the owner's own content `unmount_subtree`, before local teardown
    /// (`__unmount_local`, which itself runs the user's `on_unmount` before clearing subscriptions
    /// before transitioning to `Unmounted`).
    #[test]
    fn t19_generated_close_and_unmount_order_teardown_before_native_close() {
        let generated = generate_t17_t21_window_module();
        let close_body = generated_method_body(&generated, "close");
        let guard_pos = close_body
            .find("__closed . replace (true)")
            .expect("close() should have an idempotency guard");
        let unmount_pos = close_body
            .find("self . unmount ()")
            .expect("close() should call self.unmount()");
        let base_close_pos = close_body
            .find("self . base . close ()")
            .expect("close() should forward to the backend's own close()");
        assert!(
            guard_pos < unmount_pos && unmount_pos < base_close_pos,
            "close() must order: idempotency guard, then unmount(), then base.close(): \
             {close_body}"
        );

        let unmount_body = generated_method_body(&generated, "unmount");
        let unmounting_pos = unmount_body
            .find("ComponentLifecycleState :: Unmounting")
            .expect("unmount() should transition to Unmounting");
        let override_pos = unmount_body
            .find("WindowExt > :: unmount_override")
            .expect("unmount() should call unmount_override");
        let subtree_pos = unmount_body
            .find("unmount_subtree")
            .expect("unmount() should unmount the owner's own content subtree");
        let local_pos = unmount_body
            .find("__unmount_local ()")
            .expect("unmount() should call __unmount_local()");
        assert!(
            unmounting_pos < override_pos && override_pos < subtree_pos && subtree_pos < local_pos,
            "unmount() must order: state = Unmounting, then unmount_override() (closes any active \
             popup), then the owner's own content unmount_subtree, then local teardown \
             (__unmount_local, which itself runs user on_unmount before Unmounted): \
             {unmount_body}"
        );
    }

    /// A6/T20: repeated-close idempotency. Proves `close()`'s own guard
    /// (`self.__closed.replace(true)`) syntactically *dominates* both `self.unmount()` and
    /// `self.base.close()` — an early `return` inside the guard's own `if` body, appearing before
    /// either call in the method's linear token order, so a second `close()` call can never reach
    /// either again.
    #[test]
    fn t20_generated_close_guard_dominates_unmount_and_base_close() {
        let generated = generate_t17_t21_window_module();
        let close_body = generated_method_body(&generated, "close");
        let guard_if_pos = close_body
            .find("if self . __closed . replace (true)")
            .expect("close() should guard on __closed.replace(true)");
        let guard_return_pos = close_body[guard_if_pos..]
            .find("return")
            .map(|p| p + guard_if_pos)
            .expect("the __closed guard should return early");
        let unmount_pos = close_body
            .find("self . unmount ()")
            .expect("close() should call self.unmount()");
        let base_close_pos = close_body
            .find("self . base . close ()")
            .expect("close() should forward to the backend's own close()");
        assert!(
            guard_return_pos < unmount_pos && guard_return_pos < base_close_pos,
            "the __closed guard's own early return must textually dominate both unmount() and \
             base.close(): {close_body}"
        );
    }

    /// A6/T21: close-before-first-show. Proves the generated `unmount()` body's `Created` match
    /// arm only sets the lifecycle state to `Unmounted` and returns — *before* `unmount_override`/
    /// `unmount_subtree`/`__unmount_local` (and therefore before `mount_override`/`on_mount`/
    /// `unmount_override`/`on_unmount` are ever reached) — so `close()` on a never-shown Window
    /// reaches the real native `base.close()` without any of those framework hooks or user
    /// lifecycle callbacks having run.
    #[test]
    fn t21_generated_unmount_created_branch_returns_before_any_override_or_hook() {
        let generated = generate_t17_t21_window_module();
        let unmount_body = generated_method_body(&generated, "unmount");

        let created_arm_pos = unmount_body
            .find("ComponentLifecycleState :: Created =>")
            .expect("unmount() should match on ComponentLifecycleState::Created");
        let created_arm_end = unmount_body[created_arm_pos..]
            .find("ComponentLifecycleState :: Mounted =>")
            .map(|p| p + created_arm_pos)
            .expect(
                "unmount() should also match on ComponentLifecycleState::Mounted, after Created",
            );
        let created_arm = &unmount_body[created_arm_pos..created_arm_end];

        assert!(
            created_arm.contains("ComponentLifecycleState :: Unmounted"),
            "the Created arm must transition directly to Unmounted: {created_arm}"
        );
        assert!(
            created_arm.contains("return"),
            "the Created arm must return immediately: {created_arm}"
        );
        for forbidden in ["unmount_override", "unmount_subtree", "__unmount_local"] {
            assert!(
                !created_arm.contains(forbidden),
                "the Created arm must not reach {forbidden}: {created_arm}"
            );
        }
    }

    /// A6/T25 (Layer 1 of 2 — Layer 2 is `elwindui-backend-appkit`'s/`elwindui-backend-winui3`'s
    /// own `close_active_popup_slot` unit tests): the generated `unmount()` body calls
    /// `unmount_override()` (closes any active declarative popup, Issue #162 §3.18) *before*
    /// `unmount_subtree` on the owner's own content — restated here, on its own, as a directly
    /// T25-traceable test, even though `t19_generated_close_and_unmount_order_teardown_before_
    /// native_close` already asserts this exact ordering as part of its own broader proof.
    #[test]
    fn t25_generated_unmount_override_runs_before_owner_content_unmount_subtree() {
        let generated = generate_t17_t21_window_module();
        let unmount_body = generated_method_body(&generated, "unmount");
        let override_pos = unmount_body
            .find("WindowExt > :: unmount_override")
            .expect("unmount() should call unmount_override");
        let subtree_pos = unmount_body
            .find("unmount_subtree")
            .expect("unmount() should unmount the owner's own content subtree");
        assert!(
            override_pos < subtree_pos,
            "unmount_override() (closes any active popup) must run before the owner's own \
             content unmount_subtree: {unmount_body}"
        );
    }

    /// `Grid` (§3) + attached properties (`Grid::row`/`Grid::column`, §3) end to end: a `view`
    /// using `Grid` with `rows`/`columns` array-literal params and attached setters on its children
    /// must generate valid Rust, constructing `elwindui::core::ui::Grid` directly (a virtual
    /// builtin, like `Control`/`Shape`) with each virtual child's own `grid_cell` populated.
    #[test]
    fn generates_valid_rust_for_grid_with_attached_properties() {
        let src = r##"
            struct Foo {
                body: view! {
                    Grid {
                        rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
                        columns: [elwindui::core::layout::GridLength::Fixed(120.0), elwindui::core::layout::GridLength::Star(1.0)]
                        TextBlock { text: "Header", Grid::row: 0, Grid::column: 0 }
                        Shape { fill: "#000000", Grid::row: 1, Grid::column: 1 }
                    }
                },
            }
        "##;
        let module = crate::test_module(&[(None, src, None)]).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("grid_with_attached_properties", &generated);

        let generated_str = generated.to_string();
        assert!(
            generated_str.contains("elwindui :: core :: ui :: Grid :: new"),
            "{generated_str}"
        );
        assert!(
            generated_str.contains("GridLength :: Auto"),
            "{generated_str}"
        );
        assert!(
            generated_str.contains("GridLength :: Fixed (120.0)"),
            "{generated_str}"
        );
        assert!(
            generated_str.contains(r#"set_attached :: < i32 > ("Grid" , "row" , 0)"#),
            "{generated_str}"
        );
        assert!(
            generated_str.contains(r#"set_attached :: < i32 > ("Grid" , "column" , 0)"#),
            "{generated_str}"
        );
        assert!(
            generated_str.contains(r#"set_attached :: < i32 > ("Grid" , "row" , 1)"#),
            "{generated_str}"
        );
        assert!(
            generated_str.contains(r#"set_attached :: < i32 > ("Grid" , "column" , 1)"#),
            "{generated_str}"
        );
    }

    /// Verifies the attached-property behavior specified in docs/specs/dsl_spec.md §3:
    /// a `has_view`/plain user-defined `component`+`view` pair (non-native-rooted, so it has a real
    /// `into_node()`) used as a `Grid` child must still have its `Grid::row`/`Grid::column` reach
    /// that child's own view-root `UIElementImpl`, not be silently dropped.
    #[test]
    fn generates_valid_rust_for_grid_child_that_is_a_user_component() {
        let module = crate::test_module(&[
            (
                None,
                r#"
                struct Cell {
                    body: view! {
                        TextBlock { text: "x" }
                    },
                }
                "#,
                None,
            ),
            (
                None,
                r#"
                struct Foo {
                    body: view! {
                        Grid {
                            rows: [elwindui::core::layout::GridLength::Auto]
                            columns: [elwindui::core::layout::GridLength::Auto]
                            Cell { Grid::row: 1, Grid::column: 2 }
                        }
                    },
                }
                "#,
                None,
            ),
        ])
        .expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("grid_child_that_is_a_user_component", &generated);

        let generated_str = generated.to_string();
        assert!(generated_str.contains("into_node ()"), "{generated_str}");
        assert!(
            generated_str.contains(r#"set_attached :: < i32 > ("Grid" , "row" , 1)"#),
            "{generated_str}"
        );
        assert!(
            generated_str.contains(r#"set_attached :: < i32 > ("Grid" , "column" , 2)"#),
            "{generated_str}"
        );
    }
}
