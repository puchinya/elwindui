//! AST(検証済み) → backend別Rustソース。`target::backend()`の定数畳み込みは付録Dの通りCargo
//! featureでの静的分岐に落とし込み、`elwindui-core`のトレイト境界に対して書かれたコードを生成する
//! (今回はelwindui-backend-appkitのAPIを直接呼ぶ)。
//! 依存関係グラフに基づくCell/RefCellベースの更新関数生成は付録O.5に対応する。

use crate::ast::{
    Attr, ChildEntry, ClosureBody, ComponentDef, ElementNode, EnumDef, FieldDef, FieldKind,
    Initializer, Item, MethodDef, Module, ShortcutScope, ViewBody, ViewDef, ViewExpr, ViewModelDef,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

/// What every `component`/`viewmodel` in the whole compilation unit looks like, so that
/// cross-file references (e.g. `notepad_window.elwind`'s `vm.window_title` referring to a
/// `#[computed]` field defined in `notepad_viewmodel.elwind`) can be resolved.
///
/// Keyed by `(module real path, item name)` — the same address Rust's own name resolution uses
/// (see `ast::Module::path`) — rather than a bare item name, so two same-named types defined in
/// different modules never collide, and a lookup must go through `resolve` (i.e. through a `use`,
/// or be in the same module) instead of being visible from anywhere in the compilation unit. See
/// docs/elwindui_spec.md §12, 付録B.1.
pub struct SymbolTable {
    types: HashMap<(Vec<String>, String), TypeInfo>,
}

pub struct TypeInfo {
    pub fields: HashMap<String, FieldKind>,
    /// `component` fields defined as `bind!(owner.target, mode)`: `field_name -> (owner, target)`.
    /// Lets the view generator resolve the DSL's bare-field sugar (`content`) straight through to
    /// the field it's actually bound to (`vm.content`) without needing `self` to exist yet.
    pub binds: HashMap<String, (String, String)>,
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
    /// Names of `#[routed]` fields (docs/elwindui_spec.md 4章) — a callback's opt-in to WinUI3-
    /// style bubbling via `elwindui::core::ui::dispatch_routed` instead of being called directly.
    /// Non-empty exactly when this type needs `into_node_if_needed` to share its own
    /// `routed_handlers()` into the `NativeControl`/virtual-builtin `UIElementBase` wrapping it,
    /// rather than starting that wrapper with a fresh, empty one.
    pub routed_fields: HashSet<String>,
    /// Names of `#[bindable]` fields (`ast::Attr::Bindable`'s own doc comment,
    /// `docs/elwindui_gui_framework_design.md` §7.2) — a component field injecting a viewmodel by
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
    /// components (only `builtins.elwind`'s `Window` declares any today: `left`/`top`/`width`/
    /// `height`).
    pub onetime_fields: HashSet<String>,
    /// Whether this type is one of the hand-written-in-`elwindui_core::ui` "virtual" builtins with
    /// no `Type::new(args)` constructor and no `view` of its own (`VerticalLayout`/
    /// `HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape` today) — computed structurally
    /// (`is_builtin && !has_view && !is_native_control_leaf && !` this component's own `#[native]`
    /// flag, at `TypeInfo` construction time) rather than an enumerated name list, so adding a
    /// future virtual builtin to `builtins.elwind` needs no matching change here. See
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
    /// `#[attached]` fields declared by this type (docs/elwindui_spec.md §3の添付プロパティ), mapped
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
    /// `generate_view`'s output, which doesn't). `bind!`'s owner may resolve to either kind
    /// (`validate_bind_path` calls it "any bindable owner"), so callers that want to auto-subscribe
    /// to a `bind!` source (see `generate_view`'s `bind_owners`) must check this first — emitting a
    /// `.subscribe(...)` call against a plain `component` type would be a compile error.
    pub is_viewmodel: bool,
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
    /// still correctly inferred as virtual). See docs/elwindui_spec.md 付録H.2.
    pub is_native: bool,
    /// Whether this component's own declaration literally reads `inherits NativeControl`
    /// (`Button`/`TextArea`/`TabView` — as opposed to `#[native]` directly, e.g. `Window` or
    /// `MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`TabViewItem`, which never enter the visual tree).
    /// Unlike `is_native` (a recursively-inferred structural property), this is purely a shape-only
    /// declaration flag — only ever `true` for a hand-written builtin whose backend `XxxImpl` struct
    /// owns a real `base` (a backend-owned `NativeControlImpl`) and implements
    /// `NativeControl`/`UIElement` by delegating to it (docs/elwindui_spec.md 付録H.2.1a).
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
    /// This component's effective `view` — its own, if it wrote one, otherwise its base's
    /// (recursively), retargeted to this component's name — see `resolve_view_for`. `None` for a
    /// component with no view anywhere in its `inherits` chain (a plain data component, or one
    /// inheriting a primitive shape family with no `view` of its own, e.g. `Control`/`Rectangle`).
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
    /// over via a real `base: <Impl>` field (docs/elwindui_spec.md 付録H.2.1a), if any — see
    /// `resolve_composed_shape`. `Some` in three cases, all "direct" ones collapsing into the same
    /// generated shape (`generate_view`'s `is_shape_composition` doesn't distinguish them):
    /// - Directly against a hand-written `elwindui::core::ui` primitive: this component's own
    ///   `view` root literally constructs that shape (`ContentControl inherits Control`).
    /// - Directly against another *already-composed* DSL component: same as above, one delegation
    ///   hop further out (`RoundedPanel inherits ContentControl`, own `view` root literally
    ///   `ContentControl`).
    /// - Transitively (`is_template_composition`): this component has no `view` of its own and
    ///   inherits an already-composed component (`LabeledPanel inherits ContentControl`).
    ///
    /// `None` for a plain component, one inheriting `NativeControl`, or one inheriting another
    /// component's *code* (a `#[virtual]`/`#[override]` method-hook base like `Derived inherits
    /// Base`) rather than its composed structure.
    pub composed_shape: Option<String>,
    /// The DSL name of a hand-written native host with no `UIElement` implementation of its own
    /// (only `Window` today — `is_native && !has_view && !is_native_control_leaf`) this component
    /// composes over via a real `base: <Impl>` field, "host composition" (docs/elwindui_spec.md
    /// 付録H.2.1a) — the same `base`-field shape as `composed_shape`, but for a base that isn't a
    /// `UIElement` at all (so no `impl UIElement` is generated), and kept as a separate resolution
    /// pass from `composed_shape` since the two bases are structurally distinct categories that
    /// never overlap. `Some` iff this component's own `view` root literally constructs the base
    /// (mirroring `resolve_composed_shape`'s own root-match requirement) — see
    /// `resolve_host_composition_base`.
    pub host_composition_base: Option<String>,
    /// Whether *this* type is itself referenced as some other component's `host_composition_base`
    /// (the base side of that pair, e.g. `Window` once `NotepadWindow inherits Window` exists) —
    /// `concrete_type_ident` renames such a type to `{Name}Impl` and expects a paired empty-marker
    /// trait to exist under its bare name, exactly like `composed_shape.is_some()` already does for
    /// the composing side.
    pub is_host_composition_base: bool,
    /// Whether this component is `#[sealed]` (docs/elwindui_spec.md 付録E) — `validate.rs`'s
    /// `validate_inherits` rejects `component X inherits Name` when this is `true`. `false` for a
    /// `viewmodel` (never a valid `inherits` target at all).
    pub sealed: bool,
    /// Whether this component is `#[abstract]` (docs/elwindui_spec.md 付録E) — a pure category tag
    /// (`UIElement`/`NativeControl`/`Layout`/`Shape` in `builtins.elwind`) that cannot be
    /// instantiated directly. `validate::check_element_value` rejects any `Type { .. }`/bare-child
    /// use site naming one; `generate_module` skips generating a `create_<snake case>(..)`/`new(..)`
    /// for it entirely. `false` for a `viewmodel`.
    pub is_abstract: bool,
    /// This component's own `#[content(field_name)]` (docs/elwindui_spec.md 付録E, WinUI3's
    /// `ContentPropertyAttribute` equivalent), copied verbatim from `ComponentDef::content_field` —
    /// no recursive resolution needed (unlike `is_native`/`composed_shape`), since a bare nested
    /// child element only ever binds to *this* component's own declared field, never inherited from
    /// a base. `build_component_args` reads this to know which field (if any) a bare nested child in
    /// a `view` construction of this component binds to, independent of field declaration order.
    /// ("first still-unclaimed non-`Option` field") fallback. `None` for a `viewmodel` and for any
    /// component that doesn't declare `#[content(..)]`.
    pub content_field: Option<String>,
    /// Whether this type is marked `#[embedded]` in `elwindui-codegen`'s own `builtins.elwind`,
    /// rather than being a consumer's own `.elwind`/`#[elwindui::component]` declaration. `Module::is_builtin`
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
}

/// Strips a single `Rc<...>`/`std::rc::Rc<...>` wrapper so a `#[param] #[inject]` field declared
/// as `doc: std::rc::Rc<DocumentViewModel>` still resolves against the bare `DocumentViewModel`
/// entry in the symbol table — fields are commonly `Rc`-wrapped since `#[inject]`'s whole purpose
/// is sharing one instance across owners (付録J.5/O.4). Leaves any other type string unchanged.
pub(crate) fn strip_rc_wrapper(ty: &str) -> &str {
    let ty = ty.trim();
    for prefix in ["std::rc::Rc<", "rc::Rc<", "Rc<"] {
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
                        .map(|f| (f.name.clone(), f.kind))
                        .collect();
                    let binds = effective_fields
                        .iter()
                        .filter_map(|f| match &f.initializer {
                            Some(Initializer::Bind { path, .. }) => {
                                let [owner, target] = path.as_slice() else {
                                    return None;
                                };
                                Some((f.name.clone(), (owner.clone(), target.clone())))
                            }
                            _ => None,
                        })
                        .collect();
                    // Kind-agnostic (not `f.kind == FieldKind::Param`): now that builtins.elwind's
                    // own fields are plain (unattributed) `prop`s rather than `#[param]` (their
                    // backing Rust types are all zero-arg-constructed with post-construction
                    // `set_<field>` setters regardless — docs/elwindui_spec.md 付録H.2.1a — so
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
                    let param_fields = effective_fields
                        .iter()
                        .filter(|f| f.initializer.is_none() && !f.name.starts_with("on_"))
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
                        .filter(|f| f.initializer.is_none())
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
                            binds,
                            param_fields,
                            two_way_fields,
                            routed_fields,
                            bindable_fields,
                            onetime_fields,
                            // A "virtual builtin" is exactly: an `#[embedded]` shape declaration
                            // from `builtins.elwind`, with no `view` of its own, that isn't native
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
                            is_host_composition_base: false,
                            sealed: c.sealed,
                            is_abstract: c.is_abstract,
                            content_field: c.content_field.clone(),
                            is_builtin: c.embedded,
                            declaring_types: resolve_field_declaring_types(module, c, modules),
                        },
                    );
                }
                Item::ViewModel(v) => {
                    let field_kinds = v.fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
                    let binds = v
                        .fields
                        .iter()
                        .filter_map(|f| match &f.initializer {
                            Some(Initializer::Bind { path, .. }) => {
                                let [owner, target] = path.as_slice() else {
                                    return None;
                                };
                                Some((f.name.clone(), (owner.clone(), target.clone())))
                            }
                            _ => None,
                        })
                        .collect();
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
                            binds,
                            param_fields,
                            two_way_fields,
                            routed_fields,
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
                            is_host_composition_base: false,
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
    let host_composition_base_keys: HashSet<(Vec<String>, String)> = host_composition_memo
        .values()
        .filter_map(|v| v.as_ref().map(|(_, base_key)| base_key.clone()))
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
        info.is_host_composition_base = host_composition_base_keys.contains(key);
    }
    SymbolTable { types }
}

/// Resolves `name` as seen from `from` directly against `modules`' raw AST (no `SymbolTable`
/// needed — this is what `build_symbol_table` itself uses to resolve an `inherits` base while
/// still building the table), mirroring `SymbolTable::resolve_key`'s own name-resolution rule:
/// defined locally — in *any* module sharing `from`'s real path, not just `from` itself, since
/// every builtin shape lives in the same same-path (`[]`), `use`-less `builtins.elwind` file
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
        return c.fields.clone();
    };
    let base_fields = resolve_effective_fields(base_module, base_c, modules);
    let base_fields: Vec<FieldDef> = match find_view(from, &c.name) {
        // `#[routed]` fields (docs/elwindui_gui_framework_design.md §5.10, e.g. `UIElement`'s own
        // `on_tapped`/`on_pointer_pressed`/...), and every field declared directly on the root
        // `UIElement` component itself (`margin`/`width`/`height`/... — `builtins.elwind`'s own doc
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
            base_fields
                .into_iter()
                .filter(|f| {
                    f.attrs.iter().any(|a| matches!(a, Attr::Routed))
                        || common_fields.contains(f.name.as_str())
                        || view_references_bare_name(view, &f.name)
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
                    let base_fields = resolve_effective_fields(base_module, base_c, modules);
                    let kept_names: HashSet<&str> = base_fields
                        .iter()
                        .filter(|f| {
                            f.attrs.iter().any(|a| matches!(a, Attr::Routed))
                                || common_fields.contains(f.name.as_str())
                                || view_references_bare_name(view, &f.name)
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
            .any(|(_, expr)| view_expr_references_bare_name(expr, name))
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
        .any(|(_, expr)| view_expr_references_bare_name(expr, name))
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
        ViewExpr::Expr(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_) | ClosureBody::Block(_),
            ..
        } => false,
    }
}

/// Whether `view`'s element tree references `name` *anywhere at all* — broader than
/// `view_references_bare_name`'s own notion (a *literal* same-name forward, `padding: padding`):
/// this also counts `name` appearing as a sub-expression identifier within a larger computed value
/// (e.g. `Rectangle`'s own `kind: ShapeKind::RoundedRect { corner_radius: corner_radius.unwrap_or
/// (0.0) }` — `corner_radius` is not a *bare* forward there, but its value is still read eagerly,
/// before `Self` exists). Used exclusively to decide whether a field's value is needed at
/// construction time (docs/elwindui_spec.md 付録H.2.1a's post-construction setter convention, Phase
/// 2's `is_deferred_field`/`generate_view`'s `is_deferred_own_field`) — deliberately *not* used by
/// `resolve_effective_fields`'s own inherited-field-forwarding decision, which specifically wants
/// the narrower "literal forward" notion (a field only *contributing* to some other computed value
/// isn't being forwarded unchanged, so shouldn't be silently treated as inherited).
fn view_references_name_anywhere(view: &ViewDef, name: &str) -> bool {
    view.lets
        .iter()
        .any(|l| element_references_name_anywhere(&l.element, name))
        || view
            .root
            .attributes
            .iter()
            .any(|(_, expr)| view_expr_references_name_anywhere(expr, name))
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
        .any(|(_, expr)| view_expr_references_name_anywhere(expr, name))
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
                        let mut shadow = bm.clone();
                        shadow.name = format!("__base_{}", bm.name);
                        shadow.is_virtual = false;
                        shadow.is_override = false;
                        result.push(shadow);
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

/// Resolves `c`'s effective `view`: its own literal `view` if it wrote one (a full template
/// override — no constraint on its root element beyond what `validate::validate_inherits` already
/// checks), otherwise its base's effective `view` (recursively), retargeted to `c.name`. Returns
/// `None` when there's no `view` anywhere in the chain — a plain data component, or one inheriting
/// a primitive shape family with no `view` of its own (`Control`/`Rectangle`; those still require
/// an explicit hand-written `view` — see
/// `validate::validate_inherits`).
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
    Some(ViewDef {
        target: c.name.clone(),
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
/// implicit-composition sugar, no wrapper element written (docs/elwindui_spec.md 付録H.2.1a).
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
    }
    let mut rewriter = Rewriter { receiver };
    rewriter.visit_block_mut(&mut block);
    block
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
            // check anymore (docs/elwindui_spec.md 付録H.2.1a) — a composable `base` always
            // composes, and `generate_view`'s `resolve_view_root_element` supplies the missing
            // `Type { .. }` wrapper the view body no longer writes.
            return Some(base.to_string());
        }

        let base_key = table.resolve_key(from, base)?;
        // Direct composition against an *already-composed DSL component*, one delegation hop
        // further out (`RoundedPanel inherits ContentControl`) — the same shape as the
        // virtual-builtin case above, just one level up the chain. `generate_view`'s
        // `is_shape_composition`/`is_template_composition` don't otherwise care whether `base` is a
        // hand-written primitive or another composed DSL component, since both always delegate
        // through `self.base` regardless of that type's own nature — see this function's own
        // `has_own_view` split there, not here.
        resolve_composed_shape(&base_key, component_meta, modules, table, memo)
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
    let base_key = table.resolve_key(from, base)?;
    let base_info = table.types.get(&base_key)?;
    let base_is_native = is_native_memo.get(&base_key).copied().unwrap_or(false);
    if base_is_native && !base_info.has_view && !base_info.is_native_control_leaf {
        Some((base.to_string(), base_key))
    } else {
        None
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
            Item::Component(c) => {
                let info = table.resolve(module, &c.name).unwrap_or_else(|| {
                    panic!("component `{}` missing from its own symbol table", c.name)
                });
                // `#[abstract]` (docs/elwindui_spec.md 付録E): a pure category tag
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
                    fields: info.effective_fields.clone(),
                    methods: info.effective_methods.clone(),
                    // Irrelevant downstream: `generate_component`/`generate_view` never consult
                    // `embedded`/`sealed`/`native`/`is_abstract`/`content_field` (only
                    // `validate::validate`/`TypeInfo::sealed`/`TypeInfo::is_native`/
                    // `TypeInfo::is_abstract`/`TypeInfo::content_field`, all already checked/computed
                    // against the *original* `c`, do).
                    embedded: false,
                    sealed: false,
                    native: false,
                    is_abstract: false,
                    content_field: None,
                };
                match &info.effective_view {
                    Some(view) => generate_view(view, &synthetic, module, table),
                    None => generate_component(&synthetic, table),
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

/// Copy-able field types get `Cell<T>`, everything else gets `RefCell<T>` (付録O.5).
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
            && ty != "String"
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
/// compiler feature; see docs/elwindui_builtins_spec.md 付録Y.2.
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
    // invocation (`#[elwindui::component]` + `body: view! { .. }`, or any `.elwind` `view`
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
            FieldKind::Observable | FieldKind::Computed => Some(format_ident!("{}", f.name)),
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
        if f.kind == FieldKind::Computed {
            if let Some(Initializer::Expr(expr)) = &f.initializer {
                for dep in referenced_fields(expr, &field_names) {
                    dependents_of.entry(dep).or_default().push(f.name.clone());
                }
            }
        }
    }

    let mut struct_fields = TokenStream::new();
    let mut ctor_fields = TokenStream::new();
    let mut accessors = TokenStream::new();
    let mut recompute_calls_after_new = TokenStream::new();

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
                    .map(|dep| {
                        let recompute = format_ident!("recompute_{}", dep);
                        let property = format_ident!("{}", dep);
                        quote! {
                            self.#recompute();
                            self.on_property_changed(#property_enum::#property);
                        }
                    })
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
                    .map(|dep| {
                        let recompute = format_ident!("recompute_{}", dep);
                        let property = format_ident!("{}", dep);
                        quote! {
                            self.#recompute();
                            self.on_property_changed(#property_enum::#property);
                        }
                    })
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
            FieldKind::Prop | FieldKind::Param | FieldKind::Attached => {
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
            // conflicting with a RefCell borrow held by the notifier.
            __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<std::cell::RefCell<Box<dyn Fn(#property_enum)>>>)>>>,
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
                std::rc::Rc::new_cyclic(|__self_weak| {
                    let instance = Self {
                        #ctor_fields
                        __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                        __self_weak: __self_weak.clone(),
                    };
                    #recompute_calls_after_new
                    instance
                })
            }

            /// Registers a typed PropertyChanged handler. Dropping the returned handle unregisters
            /// it, which is essential for dynamic view regions and item templates.
            pub fn subscribe_property_changed(
                &self,
                f: impl Fn(#property_enum) + 'static,
            ) -> elwindui::core::reactive::Subscription {
                let active = std::rc::Rc::new(std::cell::Cell::new(true));
                let handler = std::rc::Rc::new(std::cell::RefCell::new(Box::new(f) as Box<dyn Fn(#property_enum)>));
                self.__property_changed_handlers.borrow_mut().push((active.clone(), handler));
                elwindui::core::reactive::Subscription::new(move || {
                    active.set(false);
                })
            }

            fn on_property_changed(&self, property: #property_enum) {
                let handlers = self.__property_changed_handlers.borrow().clone();
                for (active, handler) in handlers {
                    if active.get() {
                        (handler.borrow())(property);
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
/// call to the generated `t()` i18n helper (see `i18n_prelude`). See docs/elwindui_spec.md §11.
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
                            *node = syn::parse_quote! { #receiver.#setter(#value) };
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
                            *node = syn::parse_quote! { #receiver.#helper(#args) };
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

fn generate_component(c: &ComponentDef, table: &SymbolTable) -> TokenStream {
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
            && matches!(f.kind, FieldKind::Prop | FieldKind::Computed)
    }) {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
        let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
            unreachable!("filtered to Some(Initializer::Expr(_)) above");
        };
        let init_expr = rewrite_t_macro_bare(raw_expr.clone());
        default_let_stmts.extend(quote! { let #field_ident: #ty = #init_expr; });
    }
    let component_property_enum = format_ident!("{}Property", c.name);
    let property_variants: Vec<syn::Ident> = c
        .fields
        .iter()
        .filter(|f| {
            matches!(f.initializer, Some(Initializer::Expr(_)))
                && matches!(f.kind, FieldKind::Prop | FieldKind::Computed)
        })
        .map(|f| format_ident!("{}", f.name))
        .collect();

    for f in &c.fields {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");

        match &f.initializer {
            None => {
                // `#[param] #[inject]` field: supplied by the caller. `Option<T>`-typed fields
                // (docs/elwindui_spec.md 付録H.2.1a's post-construction setter convention,
                // extended from builtins to plain `component`s) are deferred instead — dropped from
                // `new(..)`'s own argument list, stored `Cell`/`RefCell`-wrapped (`is_copy_type`)
                // defaulting to `None`, and given a `set_<name>(&self, value: T)` setter — `None`
                // is `Option<T>`'s own natural "not yet set" value, so (unlike a required field of
                // arbitrary, possibly non-`Default` type) there's always a sound value to start
                // from. A required (non-`Option`) field stays exactly as before: a `new(..)`
                // argument, plain storage, no setter. Every field is private (not `pub`) either
                // way — external and internal reads alike go through the accessor below, since a
                // deferred fields use storage specialized for post-construction mutation.
                let (inner_ty_str, is_option) = strip_option(&f.ty);
                if is_option {
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
                    accessors.extend(quote! {
                        pub fn #field_ident(&self) -> #ty { #get_body }
                        pub fn #set_name(&self, value: #inner_ty) { #set_body }
                    });
                } else {
                    struct_fields.extend(quote! { #field_ident: #ty, });
                    ctor_params.extend(quote! { #field_ident: #ty, });
                    ctor_field_inits.extend(quote! { #field_ident, });
                    accessors.extend(quote! {
                        pub fn #field_ident(&self) -> #ty { self.#field_ident.clone() }
                    });
                }
            }
            Some(Initializer::Bind { path, mode: _ }) => {
                // `content: String = bind!(vm.content, TwoWay)`: pure passthrough, no storage of
                // its own on this component.
                let [owner, target] = path.as_slice() else {
                    panic!("bind! path must be `owner.field`");
                };
                let owner_ident = format_ident!("{}", owner);
                let getter = format_ident!("{}", target);
                let setter = format_ident!("set_{}", target);
                let get_name = format_ident!("{}", f.name);
                let set_name = format_ident!("set_{}", f.name);
                accessors.extend(quote! {
                    pub fn #get_name(&self) -> #ty { self.#owner_ident.#getter() }
                    pub fn #set_name(&self, value: #ty) { self.#owner_ident.#setter(value); }
                });
            }
            Some(Initializer::Expr(raw_expr)) if f.kind == FieldKind::Prop => {
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
                accessors.extend(quote! {
                    pub fn #field_ident(&self) -> #ty { #get_body }
                    pub fn #set_name(&self, value: #ty) {
                        #set_body
                        #(#recompute_calls)*
                        self.on_property_changed(#component_property_enum::#field_ident);
                    }
                });
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
                accessors.extend(quote! {
                    pub fn #field_ident(&self) -> #ty { #get_body }
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
            __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<std::cell::RefCell<Box<dyn Fn(#component_property_enum)>>>)>>>,
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
                let handler = std::rc::Rc::new(std::cell::RefCell::new(
                    Box::new(f) as Box<dyn Fn(#component_property_enum)>
                ));
                self.__property_changed_handlers
                    .borrow_mut()
                    .push((active.clone(), handler));
                elwindui::core::reactive::Subscription::new(move || active.set(false))
            }

            #[allow(dead_code)]
            fn on_property_changed(&self, property: #component_property_enum) {
                let handlers = self.__property_changed_handlers.borrow().clone();
                for (active, handler) in handlers {
                    if active.get() {
                        (handler.borrow())(property);
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

fn generate_view(
    view: &ViewDef,
    component: &ComponentDef,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let target_name = view.target.clone();
    let target = format_ident!("{}", target_name);
    let has_own_view = find_view(from, &target_name).is_some();

    // `component X inherits Y` where `Y` is a virtual-builtin shape primitive (`Control`/
    // `Rectangle`/`Ellipse`/`TextBlock`/`Grid`/`VerticalLayout`/`HorizontalLayout` —
    // `is_virtual_builtin`) and `X`'s own view root is literally a construction of `Y`
    // (`validate::validate_inherits` already enforces this) — the real, load-bearing case of
    // docs/elwindui_spec.md 付録H.2.1a's `struct XImpl { base: YImpl, .. }` composition: `X`'s
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
    let is_template_composition = !has_own_view && composed_shape.is_some();
    // `component X inherits Y` where `Y` is a hand-written native host with no `UIElement`
    // implementation of its own (only `Window` today) and `X`'s own view root literally constructs
    // `Y` — "host composition" (docs/elwindui_spec.md 付録H.2.1a, `TypeInfo::host_composition_base`).
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

    // A `component`/`view` pair always shares one `.elwind` file (`generate_module`'s
    // `view_targets` check), so the target is always defined locally in `from` — no `use` needed.
    let binds = table
        .resolve(from, &target_name)
        .map(|t| t.binds.clone())
        .unwrap_or_default();

    // The component's own `#[param]`-shaped fields (no initializer) become `new`'s positional
    // arguments and private struct fields — e.g. `NotepadWindow`'s `#[param] #[inject] vm:
    // NotepadViewModel`, or `DocumentView`'s `#[param] #[inject] doc: Rc<DocumentViewModel>`.
    // Bind-sugar fields (`content: String = bind!(doc.content, TwoWay)`) need no storage of their
    // own here — `ctx.binds` already resolves them straight through wherever referenced below.
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
    let own_default_fields: Vec<&FieldDef> = component
        .fields
        .iter()
        .filter(|f| {
            f.kind == FieldKind::Prop && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
        .collect();
    let own_computed_fields: Vec<&FieldDef> = component
        .fields
        .iter()
        .filter(|f| {
            f.kind == FieldKind::Computed && matches!(f.initializer, Some(Initializer::Expr(_)))
        })
        .collect();
    own_fields.extend(
        own_default_fields
            .iter()
            .chain(own_computed_fields.iter())
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
    // dependencies before dependents (the same assumption `.elwind` authors already have to satisfy
    // for a `#[computed]` field to read sensibly top-to-bottom); this doesn't topologically sort.
    let mut own_default_construct_stmts = TokenStream::new();
    for f in component.fields.iter().filter(|f| {
        matches!(f.initializer, Some(Initializer::Expr(_)))
            && matches!(f.kind, FieldKind::Prop | FieldKind::Computed)
    }) {
        let field_ident = format_ident!("{}", f.name);
        let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
        let Some(Initializer::Expr(raw_expr)) = &f.initializer else {
            unreachable!("filtered to Some(Initializer::Expr(_)) above");
        };
        let init_expr = rewrite_t_macro_bare(raw_expr.clone());
        own_default_construct_stmts.extend(quote! { let #field_ident: #ty = #init_expr; });
    }

    // `mutable_own_fields` is populated below, once `mutable_required_names` is known (it needs
    // `required_own_names`/`deferred_own_names`, computed further down using `ctx.own_fields`
    // itself) — every `emit_expr`/`plan_element`/`emit_construction`/`emit_resync` call that could
    // actually observe it happens later still, so setting it after the fact here is sound.
    let mut ctx = ViewCtx {
        binds,
        closure_param: None,
        own_fields,
        mutable_own_fields: HashSet::new(),
    };

    // `on_*`-named fields are excluded here for the same reason `TypeInfo::param_fields` (built
    // separately, in `build_symbol_table`) already excludes them: a `#[routed]` field (`UIElement`'s
    // own `on_tapped`/`on_pointer_pressed`/... — inherited by every component through
    // `resolve_effective_fields`, not just ones that declare it directly, e.g. `Button.on_click`) is
    // wired through the `on_x: ..` DSL attribute + `register_routed_handler` (`emit_wiring`'s own
    // `is_routed` branch), never as a positional constructor argument — before this filter existed
    // here, every `has_view` composed component's `new(..)` silently gained 9 required
    // `fn(PointerEventArgs)`-typed parameters nothing ever supplied, breaking every existing call
    // site the moment these fields became inheritable (`RoundedPanel`/`DocumentView`, e.g.).
    let param_names: Vec<syn::Ident> = component
        .fields
        .iter()
        .filter(|f| f.initializer.is_none() && !f.name.starts_with("on_"))
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let param_types: Vec<syn::Type> = component
        .fields
        .iter()
        .filter(|f| f.initializer.is_none() && !f.name.starts_with("on_"))
        .map(|f| syn::parse_str(&f.ty).expect("field type must parse"))
        .collect();

    // Only meaningful when `is_template_composition`: `resolve_effective_fields` gives this
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
    // For `is_template_composition`: the forwarded params above are fully consumed building `base`
    // (`field_inits`'s `base: create_<base>(..)`) — storing them *again* as this component's own
    // top-level struct fields (the ordinary, non-composed shape every other component uses) would
    // both duplicate the data pointlessly and, since they're passed by value (not `.clone()`d) into
    // the base factory, be a use-after-move compile error. Only the genuinely-new fields this
    // component adds beyond its base (rare — empty for `LabeledPanel`) become its own struct fields;
    // reads of a forwarded name instead delegate to `self.base.<name>()` (`named_accessors`, below).
    let mut own_struct_param_names: Vec<syn::Ident> = if is_template_composition {
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
    // exactly that case. This covers both a field referenced only through `bind!(owner.field, ..)`
    // from another field's initializer, and one referenced directly in the view body (e.g.
    // `vm.active_tab`) — either way, "does this field need a subscription" now depends solely on
    // whether *it itself* is `#[bindable]`, not on how other fields/expressions reference it.
    let bind_owners: Vec<syn::Ident> = component
        .fields
        .iter()
        .filter(|f| f.attrs.iter().any(|a| matches!(a, Attr::Bindable)))
        .map(|f| format_ident!("{}", f.name))
        .collect();

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
        if field.initializer.is_none() && field.ty.contains("dyn UIElement") {
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

    // Phase 0 (docs/elwindui_spec.md 付録H.2.1a's "inherits" section): a composable `base` (virtual
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

    plan_element(
        &resolved_root,
        &ctx,
        from,
        table,
        &mut plan,
        true,
        &lets_map,
    );

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

    // A shape-composition `ContentControl` root's content child must also survive past
    // construction: `new()`'s post-construction `this.set_content(this.<content_binding>.clone())`
    // (below, `content_attach_stmt`) needs to reach it as a real `self.<binding>` field — but
    // `plan_element`'s ordinary `is_root || !attributes.is_empty()` rule leaves a bare child with no
    // attributes of its own (e.g. `HorizontalLayout { .. }`, just nested children, no top-level
    // attribute) unstored, since nothing about *that* node alone says it needs to live past
    // `construct()`. Forced here — before the `struct_fields`/`field_inits` loop below reads
    // `node.stored` — for the same reason `is_host_composition`'s rename just above runs this early.
    if is_shape_composition && resolved_root.type_path == "ContentControl" {
        if let Some(content_binding) = plan
            .last()
            .and_then(|root| root.child_bindings.first())
            .map(|(binding, _)| binding.clone())
        {
            if let Some(content_node) = plan.iter_mut().find(|n| n.binding == content_binding) {
                content_node.stored = true;
            }
        }
    }

    // `is_shape_composition`'s own analog of `is_template_composition`'s `forward_param_names`:
    // which of this component's own params are bare-forwarded (`fill: fill`) straight into the
    // shape-composition root's construction (`build_virtual_value`/`build_component_value`) —
    // consumed there by move (`EmitMode::Construction`'s bare-identifier emission, see `emit_expr`'s
    // `ctx.own_fields`-bare-path branch), unlike `is_template_composition`'s always-Copy `padding`
    // case. Rectangle's `fill`/`stroke`/`stroke_width` (`Option<String>`/`Option<f32>`, forwarded
    // verbatim into `Shape { fill: fill, .. }`) are the motivating case: storing them *again* as
    // `RectangleImpl`'s own top-level fields (the ordinary shorthand every other param gets) would be
    // a use-after-move compile error, exactly like `is_template_composition`'s forwarded fields.
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
                let is_copy = ctx.own_fields.get(name).is_some_and(|ty| is_copy_type(strip_option(ty).0));
                is_bare_forward && !is_copy
            })
            .collect()
    } else {
        HashSet::new()
    };
    own_struct_param_names.retain(|n| !shape_forwarded_names.contains(&n.to_string()));
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
    let is_deferred_own_field = |name: &syn::Ident| -> bool {
        let ty_str = ctx
            .own_fields
            .get(&name.to_string())
            .expect("own_struct_param_names names one of ctx.own_fields' own keys");
        strip_option(ty_str).1 && !view_references_name_anywhere(view, &name.to_string())
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
        .filter(|n| !deferred_own_names_set.contains(&n.to_string()))
        .cloned()
        .collect();
    let ctor_param_types: Vec<syn::Type> = param_names
        .iter()
        .zip(param_types.iter())
        .filter(|(n, _)| !deferred_own_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();

    // A required own field (can't be deferred — `is_deferred_own_field` above already excluded it
    // because it's referenced somewhere in this component's own view) that's declared a plain
    // `prop` (not `#[param]`, docs/elwindui_spec.md §4) still needs to stay externally updatable
    // after construction — a `prop` field is runtime-mutable *by definition*, and "referenced at
    // construction time" doesn't change that (e.g. `RoundedPanel`'s `label`, used immediately to
    // build its own internal `TextBlock` but also meant to change on every `resync()` of whichever
    // *other* component instantiated it — `document_view.elwind`'s `RoundedPanel { label:
    // t!("notepad-status-chars", count: doc.char_count) }`). Cell/RefCell-wrapped
    // (`is_copy_type`) like a deferred field's storage, but — unlike a deferred field — stays a
    // `new(..)` positional argument (its value is needed immediately, before `Self` exists) and its
    // setter also re-runs `self.resync()` (its own view, being required, is guaranteed to actually
    // reference it, so the change needs to reach the widgets built from it right away — see the
    // setter loop below).
    let mutable_required_names: Vec<syn::Ident> = required_own_names
        .iter()
        .filter(|n| {
            component
                .fields
                .iter()
                .any(|f| f.name == n.to_string() && f.kind == FieldKind::Prop)
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
    let own_default_names: Vec<syn::Ident> = own_default_fields
        .iter()
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let own_default_types: Vec<syn::Type> = own_default_fields
        .iter()
        .map(|f| syn::parse_str(&f.ty).expect("field type must parse"))
        .collect();
    let own_default_cell_types: Vec<TokenStream> = own_default_fields
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

    let mut component_property_variants = mutable_required_names.clone();
    component_property_variants.extend(own_default_names.iter().cloned());
    component_property_variants.extend(own_computed_names.iter().cloned());
    ctx.mutable_own_fields = mutable_required_names_set.clone();
    ctx.mutable_own_fields
        .extend(own_default_names.iter().map(|n| n.to_string()));
    ctx.mutable_own_fields
        .extend(own_computed_names.iter().map(|n| n.to_string()));
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
    let mut field_inits = TokenStream::new();
    let mut wiring_stmts = TokenStream::new();
    let mut resync_stmts = TokenStream::new();
    // `#[id("...")]` bindings (§13) — a monomorphized `pub fn <id>(&self) -> Rc<ConcreteType>`
    // per binding, not a runtime string-keyed lookup (every `#[id(...)]` name is fixed at compile
    // time, so a plain accessor is strictly sufficient — see docs/elwindui_spec.md §13 and
    // 付録O.5's avoid-type-erasure convention).
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
            let handler = std::rc::Rc::new(std::cell::RefCell::new(
                Box::new(f) as Box<dyn Fn(#component_property_enum)>
            ));
            self.__property_changed_handlers
                .borrow_mut()
                .push((active.clone(), handler));
            elwindui::core::reactive::Subscription::new(move || active.set(false))
        }

        #[allow(dead_code)]
        fn on_property_changed(&self, property: #component_property_enum) {
            let handlers = self.__property_changed_handlers.borrow().clone();
            for (active, handler) in handlers {
                if active.get() {
                    (handler.borrow())(property);
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
    // delegates to the base: a `is_template_composition` forward reads the base's own already-
    // generated accessor method of the same name (`self.base.<name>()`), while a
    // `shape_forwarded_names` one reads the field straight off the base's `elwindui::core::ui`
    // struct instead — those structs' non-`Copy` fields are `RefCell`-wrapped (docs/elwindui_spec.md
    // 付録H.2.1a's post-construction setter convention), so this reads `self.base.<name>.borrow()
    // .clone()`, not a plain `.clone()` (unlike a DSL-composed base's own accessor method).
    for (name, ty) in param_names.iter().zip(param_types.iter()) {
        let is_forwarded = !own_struct_param_names.contains(name);
        // A deferred field and a mutable-required one (`mutable_required_names`) are both
        // Cell/RefCell-backed storage read the same way — `strip_option` is a harmless no-op for
        // the latter (never `Option<T>`-typed itself), so one branch covers both.
        let is_cell_backed = deferred_own_names_set.contains(&name.to_string())
            || mutable_required_names_set.contains(&name.to_string());
        let body = if is_template_composition && is_forwarded {
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
        // A composed target's own class trait (docs/elwindui_spec.md 付録H.2.1a) gets this getter
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
    for (name, ty) in own_default_names.iter().zip(own_default_types.iter()) {
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
        if is_composed {
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
    // assignment to a `#[computed]` field is already a static error, docs/elwindui_spec.md 14章
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

    // `is_template_composition`'s `plan`/`view` are the *base's* own (cloned, `resolve_view_for`)
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
        if !table
            .resolve(from, &parent.type_path)
            .is_some_and(content_field_is_list)
        {
            continue;
        }
        let slot = dynamic_slot_ident(&node.binding);
        let item_ext = dynamic_collection_item_trait(parent, from, table);
        struct_fields.extend(quote! {
            #slot: elwindui::core::ui::DynamicChildSlot<dyn elwindui::core::ui::#item_ext>,
        });
        field_inits.extend(quote! {
            #slot: elwindui::core::ui::DynamicChildSlot::default(),
        });
    }
    if !is_template_composition {
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
                    let value = build_virtual_value(node, &ctx, from, table);
                    let base_impl_ty = shape_composition_base_type(&resolved_root.type_path);
                    construct_stmts.extend(quote! { let #binding: #base_impl_ty = #value; });
                } else {
                    let value = build_component_value(node, &ctx, from, table, &plan);
                    let base_impl_ty = concrete_type_ident(
                        &resolved_root.type_path,
                        table.resolve(from, &resolved_root.type_path),
                    );
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
                let info = table.resolve(from, &node.type_path).unwrap_or_else(|| {
                    panic!(
                        "unknown or out-of-scope element `{}` — is a `use` for it missing?",
                        node.type_path
                    )
                });
                let type_ident = concrete_type_ident(&node.type_path, Some(info));
                let setters = build_component_setters(node, &ctx, from, table, info, &plan);
                let trait_use = builtin_trait_use(&node.type_path, Some(info));
                construct_stmts.extend(quote! {
                    #trait_use
                    let #binding: #type_ident = #type_ident::construct();
                    #(#setters)*
                });
                continue;
            }
            emit_construction(node, &ctx, from, table, &mut construct_stmts, &plan);
            if node.stored {
                let binding = &node.binding;
                // Every resolved type (a `component`/`view` pair or a hand-written builtin in
                // an `elwindui-backend-*` crate) is constructed as `Rc<Self>` uniformly (see `emit_construction`
                // and this same convention below in `root_embed_method`), so a stored handle is always
                // just `Rc<Type>` — no backend-crate-qualified path, no per-type bookkeeping fields.
                let type_ident =
                    concrete_type_ident(&node.type_path, table.resolve(from, &node.type_path));
                struct_fields.extend(quote! { #binding: std::rc::Rc<#type_ident>, });
                field_inits.extend(quote! { #binding: #binding.clone(), });
                if let Some(id) = &node.id {
                    let accessor = format_ident!("{}", id);
                    named_accessors.extend(quote! {
                        pub fn #accessor(&self) -> std::rc::Rc<#type_ident> {
                            self.#binding.clone()
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
                None,
                &mut resync_stmts,
                self_is_node,
            );
        }
    }

    // `plan_element` pushes children before their parent (post-order), so the root is always last.
    // Irrelevant (the base's own root, not this component's) when `is_template_composition`.
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
    // inside `construct()`, needs assembling here. Template composition (`is_template_composition`)
    // is the same idea one level up: `base`'s type is the immediate DSL base's own struct (not an
    // `elwindui::core::ui` type), built by calling that base's own `construct(..)` directly rather
    // than constructing anything itself. Host composition (`is_host_composition`) reuses the exact
    // same "value only, no declaration" shape — its root was already built unwrapped, above.
    if is_template_composition {
        let base_name = component
            .base
            .as_deref()
            .expect("is_template_composition implies a base");
        // `base_name` (bare) is itself a composed component, so it's a real *trait* now, not a
        // struct (see `struct_ident`'s doc comment) — the field's concrete type must be its `Impl`
        // struct, exactly like `concrete_type_ident` resolves for any other reference to it.
        let base_info = table.resolve(from, base_name);
        let base_construct =
            composed_construct_path(base_name, base_info.is_some_and(|i| i.is_builtin));
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
    let root_is_native = !is_template_composition
        && table
            .resolve(from, &resolved_root.type_path)
            .is_some_and(|info| info.is_native);
    let root_embed_method = if is_template_composition || is_shape_composition {
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
        let root_expr = into_any_view_if_needed(quote! { self.#root_binding }, "AnyView");
        quote! {
            pub fn into_any_view(self: std::rc::Rc<Self>) -> elwindui::backend::AnyView {
                #root_expr
            }
        }
    } else {
        let root_expr = into_node_if_needed(
            quote! { self.#root_binding },
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
            quote! {
                {
                    let weak = std::rc::Rc::downgrade(&this);
                    let subscription = elwindui::core::reactive::ObservableExt::subscribe_property_changed(&*this.#owner_ident, move |property: &'static str| {
                        if let Some(this) = weak.upgrade() { this.#method(property); }
                    });
                    this.__property_changed_subscriptions.borrow_mut().push(subscription);
                }
            }
        })
        .collect();
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
            let parent_ext = format_ident!("{}Ext", parent.type_path);
            let item_ext = dynamic_collection_item_trait(parent, from, table);
            let parent_info = table.resolve(from, &parent.type_path);
            let body = if parent_info.is_some_and(content_field_is_list) {
                let host = quote! { self.#parent_binding.children() };
                emit_dynamic_node_refresh(&plan, node, &host, &item_ext, &ctx, from, table)
            } else {
                // Phase 2: a scalar `#[content(...)]` field needs no `DynamicChildSlot` at all —
                // every branch resolves to exactly one element (`validate::validate`'s
                // `dynamic_children_reduce_to_one_element` already guarantees this), so refreshing
                // is just picking the active branch's already-constructed value and swapping it in
                // via the field's own setter.
                let field = parent_info
                    .and_then(|i| i.content_field.as_deref())
                    .unwrap_or("children");
                let setter = format_ident!("set_{field}");
                emit_scalar_dynamic_node_refresh(
                    &plan,
                    node,
                    parent_binding,
                    &setter,
                    &item_ext,
                    &ctx,
                    from,
                    table,
                )
            };
            // `.children()` (called inside `#body`, when the parent is a `Layout` family type —
            // `VerticalLayout`/`HorizontalLayout`/`Grid`, always a virtual builtin) is `LayoutExt`'s
            // own default method, inherited (not redeclared) by each of those — `#parent_ext` alone
            // isn't enough to bring a default *ancestor* trait method into scope, the same reason
            // `emit_wiring`'s routed-handler registration needs its own explicit `UIElementExt`
            // import. Not needed for `TabView` (the only other `content_field_is_list` type), whose
            // own `children()` is declared directly on `TabViewExt` — gated instead of unconditional
            // to avoid an always-unused import there.
            let layout_children_use = parent_info
                .is_some_and(|i| i.is_virtual_builtin)
                .then(|| quote! { use elwindui::core::ui::LayoutExt as _; });
            Some(quote! {
                {
                    use elwindui::core::ui::#parent_ext as _;
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

    // §3/付録I.1's lifecycle hooks. `on_mount` is spliced directly into `new()` (against the local
    // `this: Rc<Self>`, the same receiver `base::on_mount()` rewrites to — see below); `on_unmount`
    // is codegen'd as a real (if presently uncalled) `__run_on_unmount` method — `elwindui::core::ui`
    // has no detach/teardown hook yet to wire it to, see docs/elwindui_spec.md 付録I.1.
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

    let this_ident = format_ident!("this");
    let on_mount_stmt = view
        .on_mount
        .as_ref()
        .map(|block| rewrite_base_calls(block.clone(), &this_ident));

    let mut shadow_hooks = TokenStream::new();
    if let Some(block) = &base_on_mount_block {
        shadow_hooks.extend(quote! { #[allow(dead_code)] fn __base_on_mount(&self) #block });
    }
    if let Some(block) = &base_on_unmount_block {
        shadow_hooks.extend(quote! { #[allow(dead_code)] fn __base_on_unmount(&self) #block });
    }
    let on_unmount_method = view.on_unmount.as_ref().map(|block| {
        let rewritten = rewrite_base_calls(block.clone(), &format_ident!("self"));
        quote! { #[allow(dead_code)] fn __run_on_unmount(&self) #rewritten }
    });

    let methods = emit_methods(&component.methods);

    // A composed ContentControl starts with an empty bare base. Content can only be attached once
    // the outer `Rc` exists (`set_content`'s own parent-pointer wiring needs a real, upgradable
    // self-weak — see `UIElement::construct`'s own `__self_weak` doc comment), so this always runs
    // from the generated `on_constructed`, never from `construct` itself.
    let (content_capture_stmt, content_attach_stmt) =
        if is_shape_composition && resolved_root.type_path == "ContentControl" {
            let (content_binding, content_type) = plan
                .last()
                .and_then(|root| root.child_bindings.first())
                .unwrap_or_else(|| panic!("ContentControl composition requires one content child"));
            // `#content_binding` is a real, already-stored struct field (see `struct_fields`/
            // `field_inits` above), reachable directly off `&self` — no capture step needed.
            let content = into_node_if_needed(
                quote! { self.#content_binding.clone() },
                content_type,
                from,
                table,
            );
            (
                TokenStream::new(),
                quote! {
                    {
                        use elwindui::core::ui::ContentControlExt as _;
                        self.set_content(#content);
                    }
                },
            )
        } else if is_template_composition && component.base.as_deref() == Some("ContentControl") {
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
                quote! {
                    if let Some((padding, content)) = self.__deferred_content_attach.borrow_mut().take() {
                        use elwindui::core::ui::ContentControlExt as _;
                        self.set_padding(padding.unwrap_or_default());
                        self.set_content(content);
                    }
                },
            )
        } else {
            (TokenStream::new(), TokenStream::new())
        };

    // `#target`'s own class-hierarchy declaration (docs/elwindui_spec.md 付録H.2.1a). A composed
    // component (`is_shape_composition`/`is_template_composition`/`is_host_composition`) is declared
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
        let ident = format_ident!("{}", name);
        if table.resolve(from, name).is_some_and(|i| i.is_builtin) {
            quote! { elwindui::ui::#ident }
        } else {
            quote! { #ident }
        }
    };
    // The literal name (`.elwind`-level, e.g. `"ContentControl"`/`"Rectangle"`/`"Window"`) this
    // component's own generated trait bound (`inherits_path`) is keyed off — the *immediate* base
    // actually embedded as this component's own `base: <BaseImpl>` field (`resolved_root.type_path` for
    // shape composition,
    // `component.base` for template composition, `"Window"` for host composition), deliberately
    // *not* the transitively-resolved `composed_shape`.
    let immediate_base_name: Option<String> = if is_shape_composition {
        Some(resolved_root.type_path.clone())
    } else if is_template_composition {
        component.base.clone()
    } else {
        host_composition_base.clone()
    };
    // `#[class]`'s own `inherits = ..` argument always names the base's *struct* (bare `X` for a
    // consumer-defined base, `elwindui::ui::X` for a builtin — `concrete_type_ident`'s own
    // "is_builtin" rule — or `shape_composition_base_type`'s `elwindui::core::ui::X`
    // struct path for a raw virtual-builtin shape); the macro derives the matching `XExt` supertrait
    // bound on `#target`'s own generated trait internally (docs/elwindui_spec.md 付録H.2.1a) — never
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
    let property_resync_methods: TokenStream = mark_inherent(property_resync_methods_for(
        &bind_owners,
        &plan,
        &ctx,
        from,
        table,
        true,
        is_shape_composition || is_host_composition,
    ));
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
                    Some(("", &property_name)),
                    &mut statements,
                    self_is_node,
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
            quote! { #component_property_enum::#property => { this.#method(); this.__refresh_dynamic_regions(); }, }
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

    if is_composed {
        // Every one of these is purely inherent (`resync`/`#[id(..)]` child accessors/user methods/
        // lifecycle shadow hooks) — none is part of `#target`'s own generated trait — so `mark_inherent`
        // tags each with `#[inherent]` and they all land in the single `#[elwindui::class] impl
        // #target { .. }` block below instead of needing a second, separate plain `impl` purely to
        // hold them.
        let property_resync_methods: TokenStream = mark_inherent(property_resync_methods_for(
            &bind_owners,
            &plan,
            &ctx,
            from,
            table,
            false,
            is_shape_composition || is_host_composition,
        ));
        let component_property_resync_methods = mark_inherent(component_property_resync_methods);
        let own_computed_recompute_methods = mark_inherent(own_computed_recompute_methods);

        let resync_method = mark_inherent(quote! {
            fn resync(&self) {
                #resync_stmts
            }
        });
        let root_embed_method = mark_inherent(root_embed_method);
        let named_accessors = mark_inherent(named_accessors);
        let methods = mark_inherent(methods);
        let shadow_hooks = mark_inherent(shadow_hooks);
        let on_unmount_method = on_unmount_method.map(mark_inherent);
        quote! {
            #[allow(non_camel_case_types)]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum #component_property_enum {
                #(#component_property_variants),*
            }

            #[elwindui::class(inherits = #inherits_path)]
            pub struct #target {
                #(#plain_required_names: #plain_required_types,)*
                #mutable_required_field_decls
                #own_default_field_decls
                #own_computed_field_decls
                #deferred_own_field_decls
                #struct_fields
                __property_changed_subscriptions: std::cell::RefCell<Vec<elwindui::core::reactive::Subscription>>,
                __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<std::cell::RefCell<Box<dyn Fn(#component_property_enum)>>>)>>>,
                // Erased to `dyn Any` (not e.g. `dyn elwindui::core::ui::UIElementExt`) so this same
                // field shape works uniformly whether `#target`'s own chain reaches `UIElementExt`
                // (shape/template composition) or not (host composition, `inherits Window` — `Window`
                // never implements `UIElementExt` at all). `#[class]`'s own `{ClassName}Ext: ..`
                // supertrait chain always transitively reaches `AsAny: Any`, so `__self_weak` (see
                // `construct`, below) always coerces into this regardless of which chain it's in.
                __self_weak: std::cell::RefCell<std::rc::Weak<dyn std::any::Any>>,
            }

            #[elwindui::class]
            impl #target {
                fn construct(#(#ctor_param_names: #ctor_param_types),*) -> Self {
                    let __self_weak_erased: std::rc::Weak<dyn std::any::Any> = __self_weak.clone();
                    #construct_stmts
                    Self { #(#plain_required_names,)* #mutable_required_field_inits #own_default_field_inits #own_computed_field_inits #deferred_field_inits #field_inits __property_changed_subscriptions: std::cell::RefCell::new(Vec::new()), __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())), __self_weak: std::cell::RefCell::new(__self_weak_erased) }
                }

                // Runs automatically, exactly once, right after `#[class]`'s auto-generated `new()`
                // completes its `Rc::new_cyclic` (parent-pointer wiring, event wiring, the initial
                // `resync()`, lifecycle hooks — see `ContentControlImpl`'s own `on_constructed` doc
                // comment in `elwindui-core` for the same shape). `new()` itself is never hand-written
                // here — `#[class]` derives it from `construct` above.
                //
                // `wiring_stmts`/`component_self_subscription`/`subscribe_stmts`/`on_mount_stmt` were
                // all originally generated against a local `this: Rc<Self>` bound right after
                // `Rc::new` — several of them `move` a cloned `this`/`Rc::downgrade(&this)` into a
                // callback that outlives this function call, which a bare `&self` can't provide.
                // `on_constructed` reconstructs the same `this: Rc<Self>` binding here from the
                // `__self_weak` field `construct` populated above (guaranteed to upgrade successfully
                // — `on_constructed` only ever runs once the enclosing `Rc` exists) so none of that
                // generation logic needs to change.
                fn on_constructed(&self) {
                    let this: std::rc::Rc<#target> = self
                        .__self_weak
                        .borrow()
                        .upgrade()
                        .expect("on_constructed: object must already be Rc-constructed")
                        .downcast::<#target>()
                        .expect("on_constructed: owner must be this component");
                    #content_attach_stmt
                    #wiring_stmts
                    this.__refresh_dynamic_regions();
                    // Most widgets already read live model state at construction time, so this is a
                    // no-op for them. A widget whose own state only ever appears in `resync()` (e.g.
                    // a dynamic list, like `TabView`'s tabs) needs this call so state populated
                    // before construction (as `main.rs` does, calling `new_tab_execute()` first)
                    // appears immediately rather than waiting for the first unrelated user
                    // interaction.
                    this.resync();
                    #component_self_subscription
                    #subscribe_stmts
                    #on_mount_stmt
                }

                #own_class_methods
                #component_property_api
                #resync_method
                #property_resync_methods
                #component_property_resync_methods
                #own_computed_recompute_methods
                #dynamic_region_refresh_method
                #root_embed_method
                #named_accessors
                #methods
                #shadow_hooks
                #on_unmount_method
            }
        }
    } else {
        quote! {
            #[allow(non_camel_case_types)]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum #component_property_enum {
                #(#component_property_variants),*
            }

            impl #struct_ident {
                pub fn new(#(#ctor_param_names: #ctor_param_types),*) -> std::rc::Rc<Self> {
                    #content_capture_stmt
                    #construct_stmts
                    let this = std::rc::Rc::new(Self { #(#plain_required_names,)* #mutable_required_field_inits #own_default_field_inits #own_computed_field_inits #deferred_field_inits #field_inits __property_changed_subscriptions: std::cell::RefCell::new(Vec::new()), __property_changed_handlers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())) });
                    #content_attach_stmt
                    #wiring_stmts
                    this.resync();
                    this.__refresh_dynamic_regions();
                    #component_self_subscription
                    #subscribe_stmts
                    #on_mount_stmt
                    this
                }

                fn resync(&self) {
                    #resync_stmts
                }

                #property_resync_methods
                #component_property_resync_methods
                #own_computed_recompute_methods
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
                #deferred_own_field_decls
                #struct_fields
                __property_changed_subscriptions: std::cell::RefCell<Vec<elwindui::core::reactive::Subscription>>,
                __property_changed_handlers: std::rc::Rc<std::cell::RefCell<Vec<(std::rc::Rc<std::cell::Cell<bool>>, std::rc::Rc<std::cell::RefCell<Box<dyn Fn(#component_property_enum)>>>)>>>,
            }
        }
    }
}

struct ViewCtx {
    binds: HashMap<String, (String, String)>,
    /// Set while evaluating a `ViewExpr::Closure` body (`key`/`render_label`/`render_content`) to
    /// the closure's own declared parameter name (e.g. `"doc"`), so a bare reference to it emits
    /// the plain local variable that name is aliased to, rather than going through
    /// `resolve_bind`/`emit_path_get`'s `vm`-field machinery. `None` everywhere else.
    closure_param: Option<String>,
    /// This component's own `#[param]`-shaped fields (no initializer — the same set `generate_view`
    /// turns into `new`'s positional arguments / raw struct fields, see `param_names`), mapped to
    /// each field's own declared type string. A bare 1-segment reference to one of these (e.g.
    /// `RoundedPanel`'s own `label` used as `TextBlock { text: label }`, not `vm.something`) is the
    /// field/constructor-parameter itself, not an owner to call a getter on — checked *after*
    /// `binds` in `emit_expr`, since a bind-sugar field (`content: String = bind!(doc.content,
    /// TwoWay)`) is also technically one of this component's own fields but must still resolve
    /// through `doc.content()`, not a raw access. The type string additionally lets
    /// `emit_virtual_construction`'s `get_attr`/`get_attr_string` recognize an already-`Option<T>`
    /// own field forwarded as-is, so it isn't double-wrapped in another `Some(..)`.
    own_fields: std::collections::HashMap<String, String>,
    /// The subset of `own_fields` that's Cell/RefCell-backed (`generate_view`'s
    /// `mutable_required_names` — a required, non-`#[param]` own field, still needing to be read
    /// through its Cell/RefCell in `WithSelf` mode instead of the bare `self.<name>` every other
    /// own field uses). Empty at `Construction` time's own use (`emit_expr`'s `EmitMode::
    /// Construction` reads the raw constructor-argument local instead, always bare regardless).
    mutable_own_fields: HashSet<String>,
}

impl ViewCtx {
    fn with_closure_param(&self, param: &str) -> ViewCtx {
        ViewCtx {
            binds: self.binds.clone(),
            closure_param: Some(param.to_string()),
            own_fields: self.own_fields.clone(),
            mutable_own_fields: self.mutable_own_fields.clone(),
        }
    }
}

/// One element flattened out of the tree, in construction order (children before parents).
struct PlannedNode {
    binding: syn::Ident,
    type_path: String,
    attributes: Vec<(String, ViewExpr)>,
    /// Bindings of the element's *bare* nested children (`Type { ... }` written directly inside
    /// `{}`, not as `name: value`). Used to fill a resolved shape's `children`-named `#[param]`
    /// (an implicit list) or, absent one, the single field named by the component's own
    /// `#[content(field_name)]` (docs/elwindui_spec.md 付録E — e.g. `MenuBarItem`'s one nested
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
}

/// Internal planning marker for a transparent dynamic child range. It never names a generated
/// Rust type or a runtime element: the generated component owns a `DynamicChildSlot` field and
/// writes that range straight into its parent's declared `#[content]` collection.
const DYNAMIC_CHILD_SLOT_MARKER: &str = "__dynamic_child_slot";

#[allow(dead_code)]
enum DynamicPlan {
    If {
        condition: ViewExpr,
        then_bindings: Vec<(syn::Ident, String)>,
        else_bindings: Vec<(syn::Ident, String)>,
    },
    Match {
        value: ViewExpr,
        arms: Vec<(syn::Pat, Vec<(syn::Ident, String)>)>,
    },
    For {
        collection: ViewExpr,
        renderer: TokenStream,
        item_type: String,
        rc_identity: bool,
    },
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
    let mut child_bindings = Vec::new();
    for child in &node.children {
        match child {
            ChildEntry::Literal(elem) => {
                child_bindings.push(plan_element(elem, ctx, from, table, out, false, lets))
            }
            ChildEntry::Ref(name) => {
                let resolved = lets.get(name).unwrap_or_else(|| {
                    panic!("`{name}` does not refer to an earlier `let` binding in this view")
                });
                child_bindings.push(resolved.clone());
            }
            ChildEntry::If { .. } | ChildEntry::Match { .. } | ChildEntry::For { .. } => {
                child_bindings.push(plan_dynamic_entry(
                    child,
                    &node.type_path,
                    ctx,
                    from,
                    table,
                    out,
                    lets,
                ));
            }
        }
    }

    let mut element_attr_bindings = HashMap::new();
    for (name, expr) in &node.attributes {
        if let ViewExpr::Element(elem) = expr {
            element_attr_bindings.insert(
                name.clone(),
                plan_element(elem, ctx, from, table, out, false, lets),
            );
        }
    }

    let attributes = node.attributes.clone();
    let binding = format_ident!("__{}_{}", node.type_path.to_lowercase(), out.len());
    // A virtual builtin (`VerticalLayout`/`HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape`)
    // has a real `elwindui_core::ui` struct with real `set_*` setters (`TextBlockImpl::set_text`
    // etc.) just like any hand-written native or composed builtin — it's stored under the exact
    // same rule as everything else, so its attributes get resynced too (`emit_wiring`/
    // `emit_resync` already handle any `stored` node uniformly via their `if !node.stored {
    // return; }` guard — no changes needed there).
    let stored = is_root || !attributes.is_empty();

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
    });
    (binding, node.type_path.clone())
}

fn emit_for_renderer(
    binding: &str,
    body: &[ChildEntry],
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    item_trait: &syn::Ident,
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
    let subscriptions = subscribe_to_item_changes
        .then(|| emit_for_item_subscriptions(&plan, binding, &closure_ctx, from, table))
        .unwrap_or_default();
    let children = roots.iter().map(|(binding, ty)| {
        dynamic_child_binding(quote! { #binding }, ty, item_trait, from, table)
    });
    quote! {
        |#param_ident: &_| {
            #construct
            let mut __dynamic_item_subscriptions = Vec::new();
            #subscriptions
            elwindui::core::ui::DynamicChild::with_children(
                vec![#(#children),*],
                __dynamic_item_subscriptions,
            )
        }
    }
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
        for (name, expr) in &node.attributes {
            if name.starts_with("on_")
                || !info.field_types.contains_key(name)
                || matches!(expr, ViewExpr::Element(_) | ViewExpr::Closure { .. })
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
        ViewExpr::Element(_) | ViewExpr::Closure { .. } => false,
    }
}

/// Phase 2 (docs/elwindui_spec.md 付録H.2.1a): whether `info`'s own `#[content(...)]` field
/// (`children` if unnamed) is list-shaped (`Vec<...>`/`ListExt<...>`/`UIElementCollection`) rather
/// than scalar (e.g. `ContentControl`/`Window`'s `content: Rc<dyn UIElement>`) — mirrors
/// `validate.rs`'s `check_dynamic_child_hosts`'s own `is_collection` check exactly, reused here to
/// decide which of the two dynamic-region refresh shapes applies (`DynamicChildSlot`/`ListExt` vs a
/// plain `set_<field>(..)` swap — see `dynamic_region_refresh_method`'s own call site).
fn content_field_is_list(info: &TypeInfo) -> bool {
    let field = info.content_field.as_deref().unwrap_or("children");
    info.field_types.get(field).is_some_and(|ty| {
        ty.contains("UIElementCollection")
            || ty.trim_start().starts_with("Vec<")
            || ty.contains("ListExt<")
    })
}

/// Resolves the trait-object element type of a parent's declared content collection. This is driven
/// by the resolved `#[content]` field rather than a widget-name branch: `Vec<TabViewItem>` becomes
/// `TabViewItemExt`, while layout `Vec<Rc<dyn UIElement>>` becomes `UIElementExt`.
fn dynamic_collection_item_trait(
    parent: &PlannedNode,
    from: &Module,
    table: &SymbolTable,
) -> syn::Ident {
    let info = table
        .resolve(from, &parent.type_path)
        .unwrap_or_else(|| panic!("unknown dynamic-child parent `{}`", parent.type_path));
    let field = info.content_field.as_deref().unwrap_or("children");
    let ty = info.field_types.get(field).unwrap_or_else(|| {
        panic!(
            "`{}` has no content collection field `{field}`",
            parent.type_path
        )
    });
    if ty.contains("dyn UIElement") || ty.contains("UIElementCollection") {
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
/// Two independent ways to prove that: the collection's own declared type textually says
/// `Vec<Rc<T>>` (checked here directly), or the loop body hands the item to some child element's
/// `#[bindable]` field (`for_body_binds_item_to_a_bindable_field`, below) — the latter deliberately
/// never resolves the *item*'s own type (e.g. `DocumentViewModel`) at all, only the *receiving
/// component*'s (e.g. `DocumentView`), for the same reason `#[bindable]` itself exists: a
/// `#[elwindui::viewmodel]` type is commonly declared in a plain `.rs` file (or a sibling
/// `#[elwindui::component]` proc-macro invocation) that the `for` loop's own file/module never has
/// a `use` for and was never going to need one, since it only ever references the item through the
/// loop variable — so a resolve-by-name check against *that* type is fragile in a way a check
/// against the always-in-scope receiving component type isn't (see
/// `docs/agents/winui3_current_state.md`'s "Root cause of 'text not reflected after Open'" for the
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
    let [owner, field] = path.as_slice() else {
        return false;
    };
    let Some(owner_type) = ctx.own_fields.get(owner) else {
        return false;
    };
    let Some(owner_info) = table.resolve(from, strip_rc_wrapper(owner_type)) else {
        return false;
    };
    let Some(collection_type) = owner_info.value_field_types.get(field) else {
        return false;
    };
    let compact = collection_type
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    compact.starts_with("Vec<Rc<")
        || compact.starts_with("Vec<std::rc::Rc<")
        || compact.starts_with("Vec<rc::Rc<")
}

/// Whether `binding` (the `for`-loop's own bare loop variable) is passed, anywhere in `body`
/// (recursing into nested elements and `if`/`match` branches, but not into a nested `for`'s own
/// body — a shadowing or unrelated inner loop variable can't carry the outer one), as the exact
/// value of some attribute whose receiving element's resolved type declares that field
/// `#[bindable]`. See `collection_uses_rc_identity`'s doc comment for why this is checked against
/// the *receiving* component's type rather than the item's own.
fn for_body_binds_item_to_a_bindable_field(
    body: &[ChildEntry],
    binding: &str,
    from: &Module,
    table: &SymbolTable,
) -> bool {
    body.iter().any(|entry| match entry {
        ChildEntry::Literal(element) => {
            let bound_here = table
                .resolve(from, &element.type_path)
                .is_some_and(|info| {
                    element.attributes.iter().any(|(name, value)| {
                        matches!(value, ViewExpr::Path(path) if path.len() == 1 && path[0] == binding)
                            && info.bindable_fields.contains(name)
                    })
                });
            bound_here || for_body_binds_item_to_a_bindable_field(&element.children, binding, from, table)
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

fn dynamic_child_binding(
    binding: TokenStream,
    child_type: &str,
    item_trait: &syn::Ident,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    if item_trait == &format_ident!("UIElementExt") {
        return into_node_if_needed(binding, child_type, from, table);
    }
    quote! {
        {
            let __child: std::rc::Rc<dyn elwindui::core::ui::#item_trait> = #binding;
            __child
        }
    }
}

/// Phase 2: the construction-time value for a scalar `#[content(...)]` field whose sole bare child
/// is a dynamic (`if`/`match`) region — `marker_binding` names that region's own
/// `DYNAMIC_CHILD_SLOT_MARKER` `PlannedNode`, found in `plan`. Deliberately picks the *first*
/// branch (`If`'s `then`, `Match`'s first arm) completely unconditionally, without evaluating the
/// region's own condition/value at all: `new()` already calls `__refresh_dynamic_regions()`
/// immediately after construction, before `resync()` and before returning `Rc<Self>` to the caller
/// (mirroring how a scalar-unrelated, list-based dynamic region starts genuinely empty at
/// construction and is only ever populated for the first time by that same initial refresh call) —
/// so whichever branch is picked here is corrected to the real one synchronously, before anything
/// outside this function ever observes it. `for` can't reach here (Phase 2's validation rejects it
/// under a scalar field — see `validate.rs`'s `check_dynamic_child_hosts`), so only `If`/`Match`
/// are handled.
fn initial_dynamic_content_value(
    plan: &[PlannedNode],
    marker_binding: &syn::Ident,
    inner_ty: &str,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let node = plan
        .iter()
        .find(|n| &n.binding == marker_binding)
        .expect("dynamic marker must be in plan");
    let (child, child_ty) = match node
        .dynamic
        .as_ref()
        .expect("marker binding must be a dynamic node")
    {
        DynamicPlan::If { then_bindings, .. } => &then_bindings[0],
        DynamicPlan::Match { arms, .. } => &arms[0].1[0],
        DynamicPlan::For { .. } => {
            panic!("a `for` region cannot be the sole content of a scalar content field")
        }
    };
    if inner_ty.contains("dyn UIElement") {
        into_node_if_needed(quote! { #child }, child_ty, from, table)
    } else {
        into_any_view_if_needed(quote! { #child }, inner_ty)
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
            .flat_map(|(_, children)| children.iter())
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
            .map(|(_, children)| children.as_slice())
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
fn slot_span(plan: &[PlannedNode], node_binding: &syn::Ident) -> TokenStream {
    let node = plan
        .iter()
        .find(|n| &n.binding == node_binding)
        .expect("dynamic node must be in plan");
    let slot = dynamic_slot_ident(node_binding);
    let own = quote! { self.#slot.len() };
    let nested: Vec<TokenStream> = node
        .dynamic
        .as_ref()
        .map(|d| {
            direct_nested_marker_bindings(d)
                .into_iter()
                .map(|b| slot_span(plan, b))
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
) -> TokenStream {
    let preceding = siblings
        .iter()
        .take_while(|(binding, _)| binding != target)
        .map(|(binding, ty)| {
            if ty == DYNAMIC_CHILD_SLOT_MARKER {
                slot_span(plan, binding)
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
fn dynamic_region_start(plan: &[PlannedNode], target: &syn::Ident) -> TokenStream {
    if let Some(parent) = plan.iter().find(|candidate| {
        candidate
            .child_bindings
            .iter()
            .any(|(child, _)| child == target)
    }) {
        return preceding_span(plan, &parent.child_bindings, target);
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
    let local = preceding_span(plan, branch, target);
    let outer_start = dynamic_region_start(plan, &enclosing.binding);
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
) -> TokenStream {
    let slot = dynamic_slot_ident(&node.binding);
    let start = dynamic_region_start(plan, &node.binding);
    let mut out = quote! {
        self.#slot.replace_children(#host, #start, Vec::new());
    };
    if let Some(dynamic) = &node.dynamic {
        for nested_binding in direct_nested_marker_bindings(dynamic) {
            let nested_node = plan
                .iter()
                .find(|n| &n.binding == nested_binding)
                .expect("nested marker must be in plan");
            out.extend(emit_clear_dynamic_node(plan, nested_node, host));
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
    item_ext: &syn::Ident,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let slot = dynamic_slot_ident(&node.binding);
    let start = dynamic_region_start(plan, &node.binding);
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
            let collection = emit_expr(collection, ctx, &EmitMode::WithSelf(quote! { self }));
            if *rc_identity {
                quote! {
                    self.#slot.replace_rc_items(#host, #start, &(#collection), #renderer);
                }
            } else {
                quote! {
                    self.#slot.replace_items(#host, #start, #collection, #renderer);
                }
            }
        }
        DynamicPlan::If {
            condition,
            then_bindings,
            else_bindings,
        } => {
            let condition = emit_expr(condition, ctx, &EmitMode::WithSelf(quote! { self }));
            let (then_leaves, then_nested) = partition_branch_bindings(then_bindings);
            let (else_leaves, else_nested) = partition_branch_bindings(else_bindings);
            let then_children = then_leaves.iter().map(|(child, ty)| {
                dynamic_child_binding(quote! { self.#child.clone() }, ty, item_ext, from, table)
            });
            let else_children = else_leaves.iter().map(|(child, ty)| {
                dynamic_child_binding(quote! { self.#child.clone() }, ty, item_ext, from, table)
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
                        emit_clear_dynamic_node(plan, n, host)
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
                    self.#slot.replace_children(#host, #start, vec![#(#then_children),*]);
                    #refresh_then
                } else {
                    #clear_then
                    self.#slot.replace_children(#host, #start, vec![#(#else_children),*]);
                    #refresh_else
                }
            }
        }
        DynamicPlan::Match { value, arms } => {
            let value = emit_expr(value, ctx, &EmitMode::WithSelf(quote! { self }));
            // Each arm clears every *other* arm's own nested markers before repopulating its own —
            // never its own (unlike `If`'s fixed two-way "clear the other side" split, a `match`
            // has no single "other" side, so which markers count as "other" depends on which arm
            // ends up selected, hence computed per arm below). Clearing only the other arms (never
            // the one actually selected) is what lets a nested `for` inside the currently-active
            // arm keep reusing its previously-constructed items by `Rc` identity across refreshes —
            // clearing it too would reset that identity cache for no reason every single time.
            let arm_stmts = arms.iter().enumerate().map(|(i, (pattern, children))| {
                let (leaves, nested) = partition_branch_bindings(children);
                let leaf_children = leaves.iter().map(|(child, ty)| {
                    dynamic_child_binding(quote! { self.#child.clone() }, ty, item_ext, from, table)
                });
                let clear_other_arms: TokenStream = arms
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .flat_map(|(_, (_, other_children))| {
                        partition_branch_bindings(other_children).1
                    })
                    .map(|b| {
                        let n = plan.iter().find(|n| &n.binding == b).expect("in plan");
                        emit_clear_dynamic_node(plan, n, host)
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
                        self.#slot.replace_children(#host, #start, vec![#(#leaf_children),*]);
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
    setter: &syn::Ident,
    item_ext: &syn::Ident,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    match node
        .dynamic
        .as_ref()
        .expect("only called for a dynamic node")
    {
        DynamicPlan::For { .. } => {
            panic!("a `for` region cannot be the sole content of a scalar content field")
        }
        DynamicPlan::If {
            condition,
            then_bindings,
            else_bindings,
        } => {
            let condition = emit_expr(condition, ctx, &EmitMode::WithSelf(quote! { self }));
            let then_value = emit_scalar_branch_value(
                plan,
                &then_bindings[0],
                owner_binding,
                setter,
                item_ext,
                ctx,
                from,
                table,
            );
            let else_value = emit_scalar_branch_value(
                plan,
                &else_bindings[0],
                owner_binding,
                setter,
                item_ext,
                ctx,
                from,
                table,
            );
            quote! {
                if #condition { #then_value } else { #else_value }
            }
        }
        DynamicPlan::Match { value, arms } => {
            let value = emit_expr(value, ctx, &EmitMode::WithSelf(quote! { self }));
            let arm_stmts = arms.iter().map(|(pattern, children)| {
                let arm_value = emit_scalar_branch_value(
                    plan,
                    &children[0],
                    owner_binding,
                    setter,
                    item_ext,
                    ctx,
                    from,
                    table,
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
/// leaf child (emits the actual `self.#owner_binding.#setter(..)` call) or, when the branch is
/// itself a nested dynamic marker (Phase 1), a further recursive dispatch that bottoms out at
/// exactly one such call regardless of nesting depth.
fn emit_scalar_branch_value(
    plan: &[PlannedNode],
    entry: &(syn::Ident, String),
    owner_binding: &syn::Ident,
    setter: &syn::Ident,
    item_ext: &syn::Ident,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
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
            item_ext,
            ctx,
            from,
            table,
        );
    }
    let value = dynamic_child_binding(quote! { self.#binding.clone() }, ty, item_ext, from, table);
    quote! { self.#owner_binding.#setter(#value); }
}

fn plan_child_entry(
    entry: &ChildEntry,
    parent_type_path: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    lets: &HashMap<String, (syn::Ident, String)>,
) -> (syn::Ident, String) {
    match entry {
        ChildEntry::Literal(element) => {
            let resolved = plan_element(element, ctx, from, table, out, false, lets);
            out.last_mut()
                .expect("plan_element pushed the child root")
                .stored = true;
            resolved
        }
        ChildEntry::Ref(name) => lets.get(name).cloned().unwrap_or_else(|| {
            panic!("`{name}` does not refer to an earlier `let` binding in this view")
        }),
        ChildEntry::If { .. } | ChildEntry::Match { .. } | ChildEntry::For { .. } => {
            plan_dynamic_entry(entry, parent_type_path, ctx, from, table, out, lets)
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
fn plan_dynamic_entry(
    entry: &ChildEntry,
    parent_type_path: &str,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut Vec<PlannedNode>,
    lets: &HashMap<String, (syn::Ident, String)>,
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
            let then_bindings = then_branch
                .iter()
                .map(|e| plan_child_entry(e, parent_type_path, ctx, from, table, out, lets))
                .collect();
            let else_bindings = else_branch
                .iter()
                .map(|e| plan_child_entry(e, parent_type_path, ctx, from, table, out, lets))
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
                dynamic: Some(DynamicPlan::If {
                    condition: condition.clone(),
                    then_bindings,
                    else_bindings,
                }),
            });
            (binding, DYNAMIC_CHILD_SLOT_MARKER.to_string())
        }
        ChildEntry::Match { value, arms } => {
            let arms = arms
                .iter()
                .map(|arm| {
                    let pattern =
                        syn::parse::Parser::parse_str(syn::Pat::parse_single, &arm.pattern)
                            .unwrap_or_else(|error| {
                                panic!("invalid match pattern `{}`: {error}", arm.pattern)
                            });
                    let children = arm
                        .body
                        .iter()
                        .map(|e| plan_child_entry(e, parent_type_path, ctx, from, table, out, lets))
                        .collect();
                    (pattern, children)
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
            });
            (binding, DYNAMIC_CHILD_SLOT_MARKER.to_string())
        }
        ChildEntry::For {
            binding,
            collection,
            body,
        } => {
            let item_type = match body.first() {
                Some(ChildEntry::Literal(element)) => element.type_path.clone(),
                _ => {
                    panic!("a `for` body currently requires one or more literal element templates")
                }
            };
            if !body
                .iter()
                .all(|entry| matches!(entry, ChildEntry::Literal(_)))
            {
                panic!("a `for` body currently requires literal element templates");
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
            };
            let item_trait = dynamic_collection_item_trait(&parent, from, table);
            let rc_identity = collection_uses_rc_identity(collection, body, binding, ctx, from, table);
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
                    item_type,
                    rc_identity,
                }),
            });
            (node_binding, DYNAMIC_CHILD_SLOT_MARKER.to_string())
        }
    }
}

fn find_attr<'a>(node: &'a PlannedNode, name: &str) -> Option<&'a ViewExpr> {
    node.attributes
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
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
        let ty_str = table
            .resolve(from, owner)
            .and_then(|info| info.attached_field_types.get(field))
            .unwrap_or_else(|| panic!("`{owner}::{field}` is not a known `#[attached]` field (should have been caught by validation)"));
        let ty: syn::Type = syn::parse_str(ty_str)
            .unwrap_or_else(|e| panic!("invalid attached field type `{ty_str}`: {e}"));
        let value_ts = emit_expr(value, ctx, mode);
        out.extend(
            quote! { #binding.as_ui_element().set_attached::<#ty>(#owner, #field, #value_ts); },
        );
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
    // freshly `syn::parse_str`-parsed `.elwind` text expression never has — the underlying literal
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
    Some(inner.rsplit("::").next().unwrap_or(inner).trim().to_string())
}

fn into_node_if_needed(
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
    if is_native_control_leaf {
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

/// Whether `info` names a hand-written native type with no generated Rust of its own
/// (`is_native && !has_view` — `Button`/`TextArea`/`TabView`/`TabViewItem` via `inherits
/// NativeControl`, and `Window`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem` via `#[native]`
/// directly). These are the only components whose own `Type::new(..)` is hand-written Rust rather
/// than `generate_view`-produced — `emit_construction` uses this to decide between the
/// zero-argument-constructor-plus-setters convention (`build_component_setters`, docs/
/// elwindui_spec.md 付録H.2.1a's post-construction setter convention extended to every builtin
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
/// (docs/elwindui_spec.md 付録H.2.1a) rather than as a wrapper-only inherent method, so the trait
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
///   - the param named by the component's own `#[content(field_name)]` (docs/elwindui_spec.md 付録E,
///     `TypeInfo::content_field`) with no matching attribute binds the element's single bare nested
///     child (`MenuBarItem`'s single nested `Menu`, bound to its `#[content(submenu)]` field);
///   - anything else is an ordinary `emit_expr` value.
fn emit_construction(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    out: &mut TokenStream,
    plan: &[PlannedNode],
) {
    if table
        .resolve(from, &node.type_path)
        .is_some_and(|i| i.is_virtual_builtin)
    {
        emit_virtual_construction(node, ctx, from, table, out);
        return;
    }

    let binding = &node.binding;
    let info = table.resolve(from, &node.type_path).unwrap_or_else(|| {
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
        // `has_view`/plain-component construction (docs/elwindui_spec.md 付録H.2.1a's
        // post-construction setter convention): `build_component_args` omits this
        // target's own deferred `Option<T>` fields (`is_deferred_field`) from the positional list —
        // `build_component_optional_setters` supplies the matching trailing `.set_<field>(value)`
        // calls for whichever of them this use site actually gives a value.
        let args = build_component_args(node, ctx, from, table, info, plan);
        let optional_setters = build_component_optional_setters(node, ctx, from, table, info);
        out.extend(quote! {
            // A deferred field inherited from `UIElement` itself (`margin`/`width`/`height`/... —
            // `resolve_effective_fields`'s own doc comment) is set through `UIElementExt`, a shared
            // trait method rather than an inherent one — needs this in scope wherever
            // `optional_setters` (below) calls one. Harmless when unused (every other deferred
            // field's own setter is inherent), same as `builtin_trait_use`'s own unconditional
            // `#[allow(unused_imports)]`.
            #[allow(unused_imports)]
            use elwindui::core::ui::UIElementExt as _;
            let #binding = #type_ident::new(#(#args),*);
            #(#optional_setters)*
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
    // own a real `base` (a backend-owned `NativeControlImpl`) field (docs/elwindui_spec.md
    // 付録H.2.1a) — this use site's margin/attached properties are applied to it right
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
///   docs/elwindui_spec.md §4's param/prop split — `generate_view` keeps it a positional `new(..)`
///   argument *and* gives it a resync-triggering setter. Gated on `!info.is_builtin`: this rule
///   only holds for a genuinely `generate_view`-generated user component — `elwindui-codegen`'s own
///   embedded `builtins.elwind` also declares a `view` for `Rectangle`/`Ellipse`/`ContentControl`
///   (`has_view: true` too), but purely for symbol-table/validation purposes (docs/
///   elwindui_spec.md 付録H.2.1a) — their real implementation is hand-written directly in
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
/// a `#[computed]`/`bind!`-initialized field (never independently settable from outside), or an
/// action. Such a field has an initializer, so `param_fields` (only ever "every no-initializer
/// field") never includes it, and unlike an `Option<..>`-typed deferred field
/// (`is_deferred_field`) it previously had **no** way to be overridden from a use site at all —
/// its own declared default was the only value it could ever have, even though `generate_view`
/// already gives it a real `set_<name>` (the same one `#[computed]`'s own recompute cascade and a
/// same-component bare-identifier reference both call). `build_component_optional_setters` (below)
/// closes that gap: a use site providing an explicit attribute for one of these now gets a real
/// post-construction `set_<name>(value)` call, exactly like a deferred `Option<..>` field already
/// does. Gated on `!info.is_builtin` for the same reason `is_settable_field` is — this crate's own
/// embedded `builtins.elwind` never declares a defaulted `Prop` field today (only `#[attached]`
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
/// generated `new(..)`/`create_<snake case>(..)` (docs/elwindui_spec.md 付録H.2.1a) expects, in
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
) -> Vec<TokenStream> {
    // A bare nested child element (no `name:` attribute) only ever has somewhere to go if this
    // component declares a `children`-named param (a list, consumed in full below) or a
    // `#[content(field_name)]` (a single slot, consumed further down) — anything else, with no
    // declared destination at all, is a codegen-time authoring mistake, not a silently-guessed
    // field declaration order.
    let has_children_field = info.param_fields.iter().any(|(name, _)| name == "children");
    if !has_children_field && info.content_field.is_none() && !node.child_bindings.is_empty() {
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
        if name == "children" {
            let wants_node = ty.contains("dyn UIElement");
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
                if inner_ty.contains("dyn UIElement") {
                    into_node_if_needed(quote! { #nested_binding }, nested_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
                }
            }
            Some(ViewExpr::Closure { params, body }) => {
                emit_closure_value(params, body, ctx, from, table)
            }
            Some(other) => {
                if let Some(coerced) = coerce_color_literal(inner_ty, other) {
                    coerced
                } else {
                    let value = emit_expr(other, ctx, &EmitMode::Construction);
                    // A `String`-shaped param takes `&str` in every *hand-written* builtin (matching
                    // the shape declaration's `String`/`Option<String>` — see this crate's own
                    // `src/builtins.elwind`), so the value is wrapped in `&(..)` here regardless of
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
                    } else if inner_ty.contains("dyn UIElement") {
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
                    initial_dynamic_content_value(plan, child, inner_ty, from, table)
                } else if inner_ty.contains("dyn UIElement") {
                    into_node_if_needed(quote! { #child }, child_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #child }, inner_ty)
                }
            }
            None => panic!("`{}` requires attribute `{name}`", node.type_path),
        };
        args.push(if is_option {
            quote! { Some(#value) }
        } else {
            value
        });
    }
    args
}

/// The post-construction-setter analog of `build_component_args` — used by `emit_construction`'s
/// `is_hand_written_native` branch instead of positional constructor args (docs/elwindui_spec.md
/// 付録H.2.1a's post-construction setter convention, extended to every builtin's own declared
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
    if !has_children_field && info.content_field.is_none() && !node.child_bindings.is_empty() {
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
        // `docs/elwindui_spec.md` §3 (`#[content(field_name)]`'s own paragraph): bare nested
        // children bind to *some* field either via an explicit `#[content(field_name)]`, or — the
        // spec's documented fallback — a plain field literally named `children` with a list type.
        // Which of the two *emission* shapes applies (bulk `set_<field>(vec![...])` vs a
        // `.{field}().add(child)` loop against a live accessor) is derived purely from the
        // destination field's own declared type, not from which of the two mechanisms named it —
        // `Vec<T>` (e.g. `TabView`'s `children`) uses the former; `ListExt<T>` (e.g. `Menu`/
        // `MenuBar`'s `#[content(items)]` `items: ListExt<MenuItem>`, docs/elwindui_builtins_spec.md
        // 付録M) uses the latter, mirroring `Layout`/`Control`'s own `.children().add(..)`
        // convention for virtual builtins (`build_virtual_value`) one level up.
        if (name == "children" || is_this_field_content) && ty.trim_start().starts_with("Vec<") {
            let wants_node = ty.contains("dyn UIElement");
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
        let value = match attr {
            Some(ViewExpr::Element(_)) => {
                let (nested_binding, nested_ty) = node
                    .element_attr_bindings
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
                if inner_ty.contains("dyn UIElement") {
                    into_node_if_needed(quote! { #nested_binding }, nested_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
                }
            }
            Some(ViewExpr::Closure { params, body }) => {
                emit_closure_value(params, body, ctx, from, table)
            }
            Some(other) => {
                let value = emit_expr(other, ctx, &EmitMode::Construction);
                if inner_ty == "String" {
                    quote! { &(#value) }
                } else if inner_ty.contains("dyn UIElement") {
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
                    initial_dynamic_content_value(plan, child, inner_ty, from, table)
                } else if inner_ty.contains("dyn UIElement") {
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
        let value = match find_attr(node, name) {
            Some(ViewExpr::Element(_)) => {
                let (nested_binding, nested_ty) = node
                    .element_attr_bindings
                    .get(name)
                    .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
                if inner_ty.contains("dyn UIElement") {
                    into_node_if_needed(quote! { #nested_binding }, nested_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
                }
            }
            Some(ViewExpr::Closure { params, body }) => {
                emit_closure_value(params, body, ctx, from, table)
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
fn build_component_value(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    plan: &[PlannedNode],
) -> TokenStream {
    let info = table.resolve(from, &node.type_path).unwrap_or_else(|| {
        panic!(
            "unknown or out-of-scope element `{}` — is a `use` for it missing?",
            node.type_path
        )
    });
    let construct_path = composed_construct_path(&node.type_path, info.is_builtin);
    if info.is_builtin && node.type_path == "ContentControl" {
        return quote! { #construct_path() };
    }
    let args = build_component_args(node, ctx, from, table, info, plan);
    quote! { #construct_path(#(#args),*) }
}

/// Emits post-construction `set_attached::<T>(..)` calls (docs/elwindui_spec.md 付録H.2.1a) for
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
/// element can catch a routed `on_click`" common attribute (docs/elwindui_spec.md 4章) — used by
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
/// of `Ctrl`/`Shift`/`Alt`/`Meta` (docs/elwindui_gui_framework_design.md §8.1's platform-neutral
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
/// docs/elwindui_gui_framework_design.md §8.1's platform remap ("macOS向けビルドでは`Ctrl`が自動的に
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
/// existing Cargo-feature-flag-driven backend selection (`docs/elwindui_implementation_status.md`'s
/// noted stand-in for the not-yet-implemented `target::backend()`), not a `match` over some runtime
/// backend enum. A backend with no applicable chord at all (`resolve_shortcut_chord` returning
/// `None`) is silently skipped — `validate::validate_shortcut_fields` warns about that case ahead of
/// time so it's never a silent surprise.
fn emit_shortcut_registration(
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
    let value = build_virtual_value(node, ctx, from, table);
    let info = table.resolve(from, &node.type_path);
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
/// argument — docs/elwindui_spec.md 付録H.2.1a's post-construction setter convention, extended to
/// every builtin property) followed by whichever `set_<field>(..)` calls this use site's own
/// attributes supply, as a single block expression evaluating to the fully-configured value — the
/// value `emit_virtual_construction` normally stores directly, but which a
/// `component X inherits Y` shape-composition root (docs/elwindui_spec.md 付録H.2.1a) needs
/// unwrapped so it can be embedded directly as `X`'s own `base` field instead of erased into
/// `Rc<dyn UIElement>` (see `generate_view`'s `is_shape_composition` branch).
fn build_virtual_value(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
) -> TokenStream {
    let info = table
        .resolve(from, &node.type_path)
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
    for (name, ty) in &info.param_fields {
        let setter = format_ident!("set_{name}");
        let is_content = info.content_field.as_deref() == Some(name.as_str());
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
                quote! { for __child in vec![ #(#children),* ] { __v.children().add(__child); } },
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
        let value = if let Some(coerced) = coerce_color_literal(inner_ty, expr) {
            if is_option {
                quote! { Some(#coerced) }
            } else {
                coerced
            }
        } else {
            let value = emit_expr(expr, ctx, &EmitMode::Construction);
            if is_option && inner_ty == "String" {
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
            } else if inner_ty.contains("dyn UIElement") {
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
        if common_field_names.contains(name.as_str()) {
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

    quote! {
        {
            #type_trait_use
            #ui_element_trait_use
            #ui_element_ext_use
            let __v = elwindui::core::ui::#type_ident::new();
            #setters
            __v
        }
    }
}

/// The concrete Rust struct to construct/store for a resolved component named `type_path` — plain
/// `format_ident!("{type_path}")` (docs/elwindui_spec.md 付録H.2.1a: every `#[class]`-managed
/// struct, composed or not, compiles under exactly its own bare DSL name now), qualified with
/// `elwindui::ui::` when `info` says it's a builtin (a consumer-defined component has no such fixed
/// path, so it stays bare, resolved via the existing flat crate-root convention instead).
fn concrete_type_ident(type_path: &str, info: Option<&TypeInfo>) -> TokenStream {
    // Every `#[class]`-managed struct — composed, hand-written-native, virtual-builtin, or plain —
    // now compiles under exactly its own bare DSL name; only the *trait* alongside it (auto-derived
    // by `#[elwindui::class]`) is `{type_path}Ext`, never something callers here need to know about.
    let ident = format_ident!("{}", type_path);
    // A builtin (`is_builtin`) always lives at the fixed `elwindui::ui::..` path (see `TypeInfo::
    // is_builtin`'s own doc comment) — qualifying it there means callers never need a bare `use` for
    // it. A consumer-defined component has no such fixed path (codegen doesn't know where the
    // consumer's build.rs/proc-macro path puts it), so it stays bare, resolved via the existing
    // flat crate-root convention instead.
    if info.is_some_and(|i| i.is_builtin) {
        quote! { elwindui::ui::#ident }
    } else {
        quote! { #ident }
    }
}

/// `"ContentControl"` -> `ContentControl::construct` — the bare-value associated function a
/// composed component's own struct pairs with (docs/elwindui_spec.md 付録H.2.1a), mirroring
/// `elwindui::core::ui`'s `Control::construct`/`Shape::construct`/etc. `#[class]`-generated
/// convention. `is_builtin` mirrors `concrete_type_ident`'s own rule (`TypeInfo::is_builtin`'s doc
/// comment): a builtin's struct always lives at the fixed `elwindui::ui::X` path, but a
/// consumer-defined composed component's struct has no such fixed path (it's generated wherever the
/// consumer put its own source), so it stays bare.
fn composed_construct_path(name: &str, is_builtin: bool) -> TokenStream {
    let ident = format_ident!("{}", name);
    if is_builtin {
        quote! { elwindui::ui::#ident::construct }
    } else {
        quote! { #ident::construct }
    }
}

/// Resolves a virtual builtin's core struct path for shape composition.
fn shape_composition_base_type(base: &str) -> TokenStream {
    let ident = format_ident!("{base}");
    quote! { elwindui::core::ui::#ident }
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
    let info = table.resolve(from, &node.type_path);
    // A shape/host-composition root (`generate_view`'s `is_shape_composition`/`is_host_composition`
    // — `node.binding == root_binding`) has no separately-stored `self.#binding` field of its own:
    // it's moved into `self.base` at construction, and `self`/`this` itself *is* the tree node (see
    // that code's own doc comment). Every `let widget = this.#binding.clone();` below needs `this`
    // itself in that case instead.
    let widget_binding = if self_is_node {
        quote! { this.clone() }
    } else {
        quote! { this.#binding.clone() }
    };
    // Only inject the trait `use` when this node actually has something to wire up below — an
    // unconditional injection here left an always-unused import on any stored node with no `on_*`/
    // `#[two_way]` attribute at all (every branch of the loop below that actually emits tokens is
    // mirrored by one of these two conditions).
    let needs_wiring = node.attributes.iter().any(|(name, expr)| {
        name.starts_with("on_")
            || (info.is_some_and(|i| i.two_way_fields.contains(name))
                && matches!(expr, ViewExpr::Path(_)))
    });
    if !needs_wiring {
        return;
    }
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
    for (name, expr) in &node.attributes {
        if let Some(_event) = name.strip_prefix("on_") {
            let setter = format_ident!("set_{name}");
            // `#[routed]` (docs/elwindui_spec.md 4章): registered on the widget's own storage
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
                // `#[shortcut(...)]` (docs/elwindui_gui_framework_design.md §8.1) — a per-usage-
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
                            this.__refresh_dynamic_regions();
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
                        let this = std::rc::Rc::clone(&this);
                        widget.#setter(Box::new(move |#(#param_decls),*| {
                            #call;
                        }));
                    }
                });
            }
            continue;
        }

        let is_two_way = info.is_some_and(|i| i.two_way_fields.contains(name));
        if is_two_way {
            if let ViewExpr::Path(path) = expr {
                let bound = resolve_bind(path, &ctx.binds);
                let setter = emit_setter(&bound, &self_mode);
                let change_setter = format_ident!("set_on_{name}_change");
                out.extend(quote! {
                    {
                        let widget = #widget_binding;
                        let this = std::rc::Rc::clone(&this);
                        widget.#change_setter(Box::new(move |new_value| {
                            #setter(new_value);
                            // The model setter synchronously emits PropertyChanged. Its owning
                            // view subscription applies the model→widget update; forcing a second
                            // blanket resync here resets native editing state on AppKit.
                        }));
                    }
                });
            }
        }
    }
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

/// Rewrites bare references to a resolvable owner (a bind-sugar name, or one of this component's
/// own fields — e.g. `vm`) inside an `on_*` event handler's closure body into the same
/// `self.vm.field()`/`self.vm` forms every other DSL attribute value resolves to — the closure's
/// own bound parameters (`closure_params`, e.g. `index`) are left untouched as genuine locals.
/// Shared by [`rewrite_view_closure_expr`]/[`rewrite_view_closure_block`] since a `syn::Expr` and a
/// `syn::Block` both just need the same `syn::visit_mut::VisitMut` walk applied at a different
/// entry point.
struct ViewClosureRewriter<'a> {
    closure_params: &'a [String],
    ctx: &'a ViewCtx,
    mode: &'a EmitMode,
}

impl<'a> ViewClosureRewriter<'a> {
    fn resolved_owner(&self, name: &str) -> Option<TokenStream> {
        if self.closure_params.iter().any(|p| p == name) {
            return None;
        }
        if self.ctx.own_fields.contains_key(name) && !self.ctx.binds.contains_key(name) {
            return Some(self.mode.owner_tokens(name));
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
        if self.closure_params.iter().any(|p| p == name) {
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
}

impl<'a> VisitMut for ViewClosureRewriter<'a> {
    fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
        // `x = <rhs>` where `x` is a bare 1-segment reference to one of this component's own
        // mutable fields (`#[prop] is_checked: bool` mutated as `is_checked = !is_checked`) has no
        // real lvalue to assign into — the field's actual storage is `Cell`/`RefCell`-backed, only
        // reachable through its generated `set_<name>` setter. Rewritten to a setter call before
        // the generic `Expr::Path` handling below ever sees the (otherwise ordinary-looking) left-
        // hand side. Any other assignment (a genuine local variable, `+=`-style compound assignment
        // which `syn` represents as `Expr::Binary` and never reaches here, ...) falls through to the
        // default recursive visit at the bottom, unchanged.
        if let syn::Expr::Assign(assign) = node {
            if let syn::Expr::Path(p) = assign.left.as_ref() {
                if let Some(ident) = p.path.get_ident() {
                    let name = ident.to_string();
                    let is_closure_param = self.closure_params.iter().any(|p| p == &name);
                    if !is_closure_param && self.ctx.mutable_own_fields.contains(&name) {
                        if let EmitMode::WithSelf(self_tok) = self.mode {
                            self.visit_expr_mut(&mut assign.right);
                            let setter = format_ident!("set_{}", name);
                            let rhs = &assign.right;
                            *node = syn::parse_quote! { #self_tok.#setter(#rhs) };
                            return;
                        }
                    }
                }
            }
        }
        if let syn::Expr::Path(p) = node {
            let segments: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if let [only] = segments.as_slice() {
                if self.closure_params.iter().any(|p| p == only) {
                    return;
                }
                let resolved_bind = resolve_bind(std::slice::from_ref(only), &self.ctx.binds);
                if let [owner, field] = resolved_bind.as_slice() {
                    let base = self.mode.owner_tokens(owner);
                    let getter = format_ident!("{}", field);
                    *node = syn::parse_quote! { #base.#getter() };
                    return;
                }
                if let Some(value) = self.resolved_mutable_field_read(only) {
                    *node = syn::parse_quote! { #value };
                    return;
                }
                if let Some(base) = self.resolved_owner(only) {
                    *node = syn::parse_quote! { #base };
                }
                return;
            }
            if let [owner, field] = segments.as_slice() {
                if let Some(base) = self.resolved_owner(owner) {
                    let getter = format_ident!("{}", field);
                    *node = syn::parse_quote! { #base.#getter() };
                    return;
                }
            }
        }
        syn::visit_mut::visit_expr_mut(self, node);
    }
}

fn rewrite_view_closure_expr(
    mut expr: syn::Expr,
    closure_params: &[String],
    ctx: &ViewCtx,
    mode: &EmitMode,
) -> TokenStream {
    ViewClosureRewriter {
        closure_params,
        ctx,
        mode,
    }
    .visit_expr_mut(&mut expr);
    quote! { #expr }
}

fn rewrite_view_closure_block(
    mut block: syn::Block,
    closure_params: &[String],
    ctx: &ViewCtx,
    mode: &EmitMode,
) -> TokenStream {
    ViewClosureRewriter {
        closure_params,
        ctx,
        mode,
    }
    .visit_block_mut(&mut block);
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
fn collect_view_expr_owner_properties(
    expr: &ViewExpr,
    ctx: &ViewCtx,
    owner: &str,
    out: &mut std::collections::BTreeSet<String>,
) {
    match expr {
        ViewExpr::Path(path) => {
            let path = resolve_bind(path, &ctx.binds);
            if let [path_owner, path_property, ..] = path.as_slice() {
                if path_owner == owner {
                    out.insert(path_property.clone());
                }
            }
        }
        ViewExpr::TFluent(_, args) => {
            for (_, value) in args {
                collect_view_expr_owner_properties(value, ctx, owner, out);
            }
        }
        ViewExpr::Expr(expr) => {
            struct Collector<'a> {
                owner: &'a str,
                out: &'a mut std::collections::BTreeSet<String>,
            }
            impl<'ast> Visit<'ast> for Collector<'_> {
                fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                    let segments: Vec<_> = node.path.segments.iter().collect();
                    if segments.len() >= 2 && segments[0].ident == self.owner {
                        self.out.insert(segments[1].ident.to_string());
                    }
                    syn::visit::visit_expr_path(self, node);
                }
            }
            let mut collector = Collector { owner, out };
            collector.visit_expr(expr);
        }
        ViewExpr::Element(_) | ViewExpr::Closure { .. } => {}
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
    bind_owners
        .iter()
        .map(|owner_ident| {
            let owner_name = owner_ident.to_string();
            let mut properties: std::collections::BTreeSet<String> = Default::default();
            for node in plan {
                for (_, expr) in &node.attributes {
                    collect_view_expr_owner_properties(expr, ctx, &owner_name, &mut properties);
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
                            Some((&owner_name, property_name)),
                            &mut statements,
                            self_is_node,
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
    match expr {
        ViewExpr::Path(path) => {
            let path = resolve_bind(path, &ctx.binds);
            if owner.is_empty() {
                matches!(path.as_slice(), [path_property] if path_property == property)
            } else {
                matches!(path.as_slice(), [path_owner, path_property, ..] if path_owner == owner && path_property == property)
            }
        }
        ViewExpr::TFluent(_, args) => args
            .iter()
            .any(|(_, value)| view_expr_depends_on(value, ctx, owner, property)),
        ViewExpr::Expr(expr) => {
            struct Collector<'a> {
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
                    {
                        self.found = true;
                    }
                    syn::visit::visit_expr_path(self, node);
                }

                fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
                    // `t!` and user macros hide arbitrary expressions in token trees. They need a
                    // dedicated parser before they can participate in an exact dependency set.
                    self.opaque_macro = true;
                    syn::visit::visit_expr_macro(self, node);
                }
            }
            let mut collector = Collector {
                owner,
                property,
                found: false,
                opaque_macro: false,
            };
            collector.visit_expr(expr);
            collector.found || collector.opaque_macro
        }
        ViewExpr::Element(_) | ViewExpr::Closure { .. } => false,
    }
}

fn emit_resync(
    node: &PlannedNode,
    ctx: &ViewCtx,
    from: &Module,
    table: &SymbolTable,
    filter: Option<(&str, &str)>,
    out: &mut TokenStream,
    self_is_node: bool,
) {
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { self });
    let info = table.resolve(from, &node.type_path);
    // See `emit_wiring`'s matching `widget_binding`/`self_is_node` doc comment — a shape/host-
    // composition root has no separately-stored `self.#binding` field; `self` itself already *is*
    // the tree node.
    let receiver = if self_is_node {
        quote! { self }
    } else {
        quote! { self.#binding }
    };
    // `resync()` is its own function, a separate lexical scope from `new()` — the `use` already
    // injected alongside construction (`emit_construction`'s `builtin_trait_use`, or
    // `build_virtual_value`'s own inline copy for a virtual builtin) doesn't carry over here, so
    // any hand-written native or virtual builtin whose setters are shared-trait-only needs its own
    // copy of the same import for this function's own `self.#binding.#setter(..)` calls below.
    out.extend(builtin_trait_use(&node.type_path, info));
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
    for (name, expr) in &node.attributes {
        if info.is_some_and(|i| !i.field_types.contains_key(name)) {
            continue;
        }
        if name.starts_with("on_") {
            continue;
        }
        if matches!(expr, ViewExpr::Element(_) | ViewExpr::Closure { .. }) {
            continue;
        }
        if let Some((owner, property)) = filter {
            if !view_expr_depends_on(expr, ctx, owner, property) {
                continue;
            }
        }
        // `#[onetime]` fields (`Window`'s own `left`/`top`/`width`/`height`,
        // docs/elwindui_builtins_spec.md 付録F.1) are one-time initial-placement/size setters,
        // applied once at construction (`build_component_setters`) — never re-pushed here.
        // Re-applying them on every resync() would fight the OS window manager, snapping a
        // user-dragged/resized window back to its originally-declared value the next time
        // *anything else* triggers resync() (e.g. `TabView`'s `on_select` wiring). The live native
        // frame is available separately via `Window`'s own `left()`/`top()`/`width()`/`height()`
        // getters for whoever wants current state. Declarative (`info.onetime_fields`, from this
        // field's own `#[onetime]` attribute in `builtins.elwind`) rather than a hardcoded
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
        } else if field_ty.is_some_and(|ty| strip_option(ty).0.contains("dyn UIElement")) {
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
/// `builtins.elwind`) — no per-widget-type or per-field-name table to maintain: any current or
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

/// Resolves the DSL's bare-field bind sugar: `content` (a `component` field defined as
/// `bind!(vm.content, ...)`) becomes `["vm", "content"]`. Paths that don't match a known bind
/// (e.g. `vm.window_title`, already fully qualified) pass through unchanged.
fn resolve_bind(path: &[String], binds: &HashMap<String, (String, String)>) -> Vec<String> {
    if path.len() == 1 {
        if let Some((owner, target)) = binds.get(&path[0]) {
            return vec![owner.clone(), target.clone()];
        }
    }
    path.to_vec()
}

fn emit_expr(expr: &ViewExpr, ctx: &ViewCtx, mode: &EmitMode) -> TokenStream {
    match expr {
        ViewExpr::Expr(e) => quote! { #e },
        ViewExpr::Path(path) => {
            let path: &[String] = path.as_slice();
            // A bare reference to the closure's own bound parameter (e.g. `doc` in
            // `render_content: |doc| DocumentView { doc: doc }`) passes the value straight
            // through — it isn't a `vm`-style field with a generated getter, so it must be
            // handled before `resolve_bind`/`emit_path_get` (which has no 1-segment path shape).
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
                // `resolve_bind`/`emit_path_get`'s `vm.something`-shaped 2-segment machinery. Only
                // reached when `only` isn't a bind-sugar name (`ctx.binds` doesn't contain it —
                // `resolve_bind` below would otherwise rewrite it), since a bind-sugar field
                // (`content: String = bind!(doc.content, TwoWay)`) is also one of `own_fields` but
                // must still resolve through its bound owner instead of a raw field access.
                if ctx.own_fields.contains_key(only) && !ctx.binds.contains_key(only) {
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
            }
            let resolved = resolve_bind(path, &ctx.binds);
            emit_path_get(&resolved, mode)
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
    }
}

/// A resolved `["vm", "content"]`-style path -> `vm.content()` (construction) /
/// `self.vm.content()` (with self). A viewmodel action (`vm.save`) resolves through this exact
/// same 2-segment shape — there is no separate `Command`-wrapper indirection to fold in.
fn emit_path_get(path: &[String], mode: &EmitMode) -> TokenStream {
    match path {
        [owner, field] => {
            let base = mode.owner_tokens(owner);
            let getter = format_ident!("{}", field);
            quote! { #base.#getter() }
        }
        other => panic!(
            "unsupported path shape after bind resolution: `{}`",
            other.join(".")
        ),
    }
}

fn emit_setter(path: &[String], mode: &EmitMode) -> TokenStream {
    let [owner, field] = path else {
        panic!(
            "expected a 2-segment path after bind resolution, got `{}`",
            path.join(".")
        );
    };
    let base = mode.owner_tokens(owner);
    let setter = format_ident!("set_{}", field);
    quote! { #base.#setter }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_module;

    /// Builtins (`Window`/`VerticalLayout`/`TextArea`/etc.) only resolve when their shape modules
    /// (`crate::builtin_modules`) are part of the symbol table — `compile_dir`/`generate_from_source`
    /// do this automatically, but a test building its own table directly needs to opt in explicitly.
    fn build_symbol_table_with_builtins(modules: &[Module]) -> SymbolTable {
        let all: Vec<Module> = modules
            .iter()
            .cloned()
            .chain(crate::builtin_modules())
            .collect();
        build_symbol_table(&all)
    }

    #[test]
    fn embedded_attribute_is_the_builtin_boundary_within_builtin_module() {
        let mut module = parse_module(
            r#"
                #[embedded]
                component EmbeddedShape { }

                component OrdinaryComponent { }
            "#,
        )
        .unwrap();
        // `Module::is_builtin` only authorizes `#[embedded]`; it must not by itself turn every
        // declaration in the source into a builtin.
        module.is_builtin = true;

        let table = build_symbol_table(&[module.clone()]);
        assert!(table.resolve(&module, "EmbeddedShape").unwrap().is_builtin);
        assert!(
            !table
                .resolve(&module, "OrdinaryComponent")
                .unwrap()
                .is_builtin
        );
    }

    /// Actions can't be declared in `.elwind`-native `viewmodel` text (only `#[observable]`/
    /// `#[computed]` can); a viewmodel with actions is always built via the Rust-native
    /// `attr_frontend` frontend (`mod { struct .. impl .. }`) instead, same as the real
    /// `#[elwindui::viewmodel]` macro — see `attr_frontend::viewmodel_def_from_item_mod`. `path:
    /// Vec::new()` matches `.elwind`'s own crate-root placement (`parse_module`'s modules are also
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

    const WINDOW_SRC: &str = r#"
use crate::NotepadViewModel;

component NotepadWindow {
    #[bindable]
    vm: NotepadViewModel,

    content: String = bind!(vm.content, TwoWay),
}

view NotepadWindow {
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

            TextArea { text: content }

            HorizontalLayout {
                TextBlock { text: t!("notepad-status-chars", count: vm.char_count) }
            }
        }
    }
}
"#;

    fn assert_valid_rust(label: &str, ts: &TokenStream) {
        if let Err(e) = syn::parse2::<syn::File>(ts.clone()) {
            panic!("{label} did not generate valid Rust: {e}\n---\n{ts}");
        }
    }

    #[test]
    fn generates_dynamic_if_region_that_reads_the_current_property() {
        let module = parse_module(
            r#"
                viewmodel DynamicViewModel {
                    #[observable]
                    show: bool = true,
                }

                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }

                view DynamicHost {
                    VerticalLayout {
                        if vm.show {
                            TextBlock { text: "shown" }
                        } else {
                            TextBlock { text: "hidden" }
                        }
                    }
                }
            "#,
        )
        .expect("dynamic if source should parse");
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
        let module = parse_module(
            r#"
                viewmodel DynamicViewModel {
                    #[observable]
                    is_zero: bool = true,
                    #[observable]
                    is_one: bool = false,
                }

                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }

                view DynamicHost {
                    VerticalLayout {
                        if vm.is_zero {
                            TextBlock { text: "zero" }
                        } else if vm.is_one {
                            TextBlock { text: "one" }
                        } else {
                            TextBlock { text: "many" }
                        }
                    }
                }
            "#,
        )
        .expect("else-if source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("else_if_chain", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
    }

    /// A `for` nested inside an `if`'s then-branch: the outer `if` toggles between the `for` region
    /// and a static fallback, so the nested `for`'s own `DynamicChildSlot` must be forced empty
    /// (`replace_children` with an empty `vec`) whenever the `if` picks the static branch instead.
    #[test]
    fn generates_nested_for_inside_if_then_branch() {
        let module = parse_module(
            r#"
                viewmodel Item { }
                viewmodel DynamicViewModel {
                    #[observable]
                    show_list: bool = true,
                    #[observable]
                    items: Vec<std::rc::Rc<Item>> = Vec::new(),
                }
                component ItemView {
                    #[param]
                    item: std::rc::Rc<Item>,
                }
                view ItemView { TextBlock { text: "item" } }
                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }
                view DynamicHost {
                    VerticalLayout {
                        if vm.show_list {
                            for item in vm.items { ItemView { item: item } }
                        } else {
                            TextBlock { text: "empty" }
                        }
                    }
                }
            "#,
        )
        .expect("nested for-in-if source should parse");
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
        let module = parse_module(
            r#"
                enum Status { Ready, Busy }
                viewmodel DynamicViewModel {
                    #[observable]
                    status: Status = Status::Ready,
                    #[observable]
                    urgent: bool = false,
                }
                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }
                view DynamicHost {
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
                }
            "#,
        )
        .expect("nested if-in-match source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("nested_if_in_match", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
    }

    /// Phase 2 (docs/elwindui_spec.md 付録H.2.1a): a scalar `#[content(...)]` field
    /// (`ContentControl`'s `content: Rc<dyn UIElement>`) can host an `if`/`match` dynamic region —
    /// combined with Phase 0's implicit-composition sugar, this is what used to be called "root
    /// self-dynamism": `component X inherits ContentControl { view X { if .. { A } else { B } } }`
    /// with no wrapper element written at all. Swapping must go through `set_content`, never
    /// `DynamicChildSlot` (there is nowhere to keep a list position for a single-value field).
    #[test]
    fn generates_scalar_content_dynamic_region_via_content_control() {
        let module = parse_module(
            r#"
                viewmodel DynamicViewModel {
                    #[observable]
                    show_a: bool = true,
                }

                component DynamicHost inherits ContentControl {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }

                view DynamicHost {
                    if vm.show_a {
                        TextBlock { text: "a" }
                    } else {
                        TextBlock { text: "b" }
                    }
                }
            "#,
        )
        .expect("scalar dynamic content source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::builtin_modules())
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
    /// scalar `#[content(...)]` field (`codegen.rs`'s `content_field_is_list`/
    /// `emit_scalar_dynamic_node_refresh` don't special-case either type by name).
    #[test]
    fn generates_scalar_content_dynamic_region_via_window_host_composition() {
        let module = parse_module(
            r#"
                viewmodel DynamicViewModel {
                    #[observable]
                    show_a: bool = true,
                }

                component DynamicHost inherits Window {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }

                view DynamicHost {
                    title: "Dynamic"
                    if vm.show_a {
                        TextBlock { text: "a" }
                    } else {
                        TextBlock { text: "b" }
                    }
                }
            "#,
        )
        .expect("scalar dynamic content (host composition) source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let all_modules: Vec<_> = std::iter::once(module.clone())
            .chain(crate::builtin_modules())
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
        let module = parse_module(
            r#"
                enum Status { Ready, Busy }
                viewmodel DynamicViewModel {
                    #[observable]
                    status: Status = Status::Ready,
                }
                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }
                view DynamicHost {
                    VerticalLayout {
                        match vm.status {
                            Status::Ready => { TextBlock { text: "ready" } }
                            Status::Busy => { TextBlock { text: "busy" } }
                        }
                    }
                }
            "#,
        )
        .expect("dynamic match source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("dynamic_match", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("fn __refresh_dynamic_regions"));
    }

    #[test]
    fn generates_dynamic_for_region_with_an_item_local_template() {
        let module = parse_module(
            r#"
                viewmodel Item { }
                viewmodel DynamicViewModel {
                    #[observable]
                    items: Vec<std::rc::Rc<Item>> = Vec::new(),
                }
                component ItemView {
                    #[param]
                    item: std::rc::Rc<Item>,
                }
                view ItemView { TextBlock { text: "item" } }
                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }
                view DynamicHost {
                    VerticalLayout {
                        for item in vm.items { ItemView { item: item } }
                    }
                }
            "#,
        )
        .expect("dynamic for source should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("dynamic_for", &generated);
        let rendered = generated.to_string();
        assert!(rendered.contains("replace_rc_items"));
        assert!(rendered.contains("item . clone"));
    }

    #[test]
    fn rebuilds_only_the_for_slot_for_non_rc_items() {
        let module = parse_module(
            r#"
                viewmodel DynamicViewModel {
                    #[observable]
                    items: Vec<String> = Vec::new(),
                }
                component ItemView {
                    #[param]
                    item: String,
                }
                view ItemView { TextBlock { text: item } }
                component DynamicHost {
                    #[param]
                    #[inject]
                    vm: DynamicViewModel,
                }
                view DynamicHost {
                    VerticalLayout {
                        for item in vm.items { ItemView { item: item } }
                    }
                }
            "#,
        )
        .expect("plain dynamic for source should parse");
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
        let window_module = parse_module(WINDOW_SRC).unwrap();
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
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
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
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = parse_module(window_src).unwrap();
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
    /// (docs/elwindui_gui_framework_design.md §5.10) must be wired with the payload type each
    /// field itself declares (`fn(elwindui_core::input::PointerEventArgs)`/`TappedEventArgs`/...) —
    /// derived purely from `TypeInfo::field_types` via `callback_param_types`, never a hardcoded
    /// event-name/type table in `elwindui-codegen` itself (the codegen design doc's own no-
    /// hardcoding rule). Exercised on a plain virtual builtin (`VerticalLayout`), not `Button`.
    #[test]
    fn routed_pointer_event_derives_its_payload_type_from_the_field_declaration() {
        let window_src = r#"
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: vm.window_title
        VerticalLayout {
            on_tapped: |e| { vm.save(); }
        }
    }
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = parse_module(window_src).unwrap();
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
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: vm.window_title
        VerticalLayout {
            on_pointer_entered: |e| { vm.save(); }
            on_pointer_wheel_changed: |e| { vm.save(); }
        }
    }
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = parse_module(window_src).unwrap();
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
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: vm.window_title
        VerticalLayout {
            on_tapped: || vm.save()
        }
    }
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = parse_module(window_src).unwrap();
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
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: vm.window_title
        TabView {
            selected_index: 0
            on_select: || vm.save
        }
    }
}
"#;
        let viewmodel_module = notepad_viewmodel_module();
        let window_module = parse_module(window_src).unwrap();
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
        let document_module = parse_module(
            r#"
viewmodel Document {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",
}
"#,
        )
        .expect("document viewmodel should parse");
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
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
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
            selected_index: vm.active_tab
            on_select: |index| vm.select_tab(index)
            on_new_tab: vm.new_tab
        }
    }
}
"#;
        let window_module = parse_module(window_src).expect("window should parse");
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
viewmodel Document {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",
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
use crate::Document;

component DocumentView {
    #[param]
    #[inject]
    doc: std::rc::Rc<Document>,

    content: String = bind!(doc.content, TwoWay),
}

view DocumentView {
    VerticalLayout {
        TextArea { text: content }
    }
}
"#;
        let window_src = r#"
use crate::NotepadViewModel;
use crate::DocumentView;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
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
            selected_index: vm.active_tab
            on_select: |index| vm.select_tab(index)
            on_new_tab: vm.new_tab
        }
    }
}
"#;
        let document_module = parse_module(viewmodel_src).expect("viewmodel should parse");
        let document_view_module =
            parse_module(document_view_src).expect("document view should parse");
        let window_module = parse_module(window_src).expect("window should parse");
        let modules = [
            document_module.clone(),
            notepad_viewmodel_module.clone(),
            document_view_module.clone(),
            window_module.clone(),
        ];
        let all_modules: Vec<_> = modules
            .iter()
            .cloned()
            .chain(crate::builtin_modules())
            .collect();
        let table = build_symbol_table(&all_modules);

        assert_eq!(crate::validate::validate(&all_modules), Ok(()));

        let document_view_code = generate_module(&document_view_module, &table);
        assert_valid_rust("document_view", &document_view_code);
        let document_view_str = document_view_code.to_string();
        assert!(document_view_str.contains("fn new (doc : std :: rc :: Rc < Document >)"));
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

    /// Unlike `viewmodel_module_from_rust` (used by other tests), registers the viewmodel module at
    /// `path: vec![mod_name]` — matching what `attr_frontend::viewmodel_defs_from_rs_file` (the real
    /// `compile_dir_with_extra_viewmodels` production path) actually does, not the shared test
    /// helper's simplified `path: []`. Needed by `for_loop_identity_survives_when_element_type_isnt_
    /// used_by_the_for_loops_own_file` below to reproduce the real bug: with `path: []` (same as
    /// every `.elwind`-parsed module), the element type would be trivially visible to *any* module
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
    /// (`notepad_window.elwind`) over a `#[elwindui::viewmodel]`-declared `Vec<DocumentViewModel>`
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
        // Mirrors `document_view.elwind`: `#[bindable]` (not `#[param] #[inject]`), and `use`s
        // `Document` since it names the type directly in its own field declaration.
        let document_view_src = r#"
use crate::document_view_model::Document;

component DocumentView {
    #[bindable]
    doc: std::rc::Rc<Document>,

    content: String = bind!(doc.content, TwoWay),
}

view DocumentView {
    VerticalLayout {
        TextArea { text: content }
    }
}
"#;
        // Mirrors `notepad_window.elwind`: `use`s `NotepadViewModel` and `DocumentView`, but never
        // `Document` — `doc` is only ever referenced through the `for` loop's own binding.
        let window_src = r#"
use crate::notepad_view_model::NotepadViewModel;
use crate::DocumentView;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: t!("notepad-window-title")

        TabView {
            for doc in vm.documents {
                TabViewItem {
                    header: doc.file_name
                    DocumentView { doc: doc }
                }
            }
            selected_index: vm.active_tab
            on_select: |index| vm.select_tab(index)
            on_new_tab: vm.new_tab
        }
    }
}
"#;
        let document_view_module =
            parse_module(document_view_src).expect("document view should parse");
        let window_module = parse_module(window_src).expect("window should parse");
        let modules = [
            notepad_viewmodel_module.clone(),
            document_module.clone(),
            document_view_module.clone(),
            window_module.clone(),
        ];
        let all_modules: Vec<_> = modules
            .iter()
            .cloned()
            .chain(crate::builtin_modules())
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
        let src = r#"
viewmodel Greeter {
    #[observable]
    name: String = String::new(),
}

component Greeting {
    #[param]
    #[inject]
    greeter: Greeter,
}

view Greeting {
    TextBlock { text: greeter.name }
}
"#;
        let module = parse_module(src).expect("should parse");
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
        let src = r#"
viewmodel Document {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = String::new(),
}

component DocumentView {
    #[param]
    #[inject]
    doc: Document,
}

view DocumentView {
    VerticalLayout {
        TextArea { text: doc.content }
        TextBlock { margin: 4.0, text: doc.file_name }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("property_update_common_attributes", &generated);

        let generated = generated.to_string();
        // `margin` is set at construction and by the initial resync. Neither `content` nor
        // `file_name` notification may relayout this unrelated common UIElement property.
        assert_eq!(generated.matches("set_margin").count(), 2, "{generated}");
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
    /// rounded_panel.elwind`) — `fill`/`stroke` are `Brush`-typed (painter design doc §18's
    /// `Option<String>` → `Option<Brush>` migration), so a hex string literal must be validated
    /// and converted to `Brush::Solid(Color::rgba(..))` at codegen time (`coerce_color_literal`)
    /// rather than spliced through unchanged.
    #[test]
    fn rectangle_fill_hex_literal_is_coerced_to_a_brush() {
        let src = r##"
component Foo {
}

view Foo {
    Rectangle {
        fill: "#3a3a3c"
        corner_radius: 8.0
    }
}
"##;
        let module = parse_module(src).expect("should parse");
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
component Foo {
}

view Foo {
    Rectangle {
        fill: "#zzzzzz"
    }
}
"##;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let _ = generate_module(&module, &table);
    }

    /// `ContentControl inherits Control` (docs/elwindui_builtins_spec.md 付録F.10) — the
    /// `#[param] content` field is forwarded as a bare child into `Control`'s own children via the
    /// `PASSTHROUGH_NODE`-tagged `lets_map` seeding in `generate_view`, and every `#[param]` field
    /// (not just `#[id(...)]` lets) gets a generated named accessor.
    #[test]
    fn generates_valid_rust_for_content_control() {
        let src = r#"
component Foo {
}

view Foo {
    ContentControl {
        padding: 8.0
        TextBlock { text: "hi" }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("content_control", &generated);

        let generated_str = generated.to_string();
        // `ContentControl` is composed (docs/elwindui_spec.md 付録H.2.1a) — its real struct is
        // always its own bare name (`ContentControlExt` is its auto-derived trait), so `Foo`'s own
        // generated code, resolving `ContentControl` as a child element, must construct that
        // concrete type (`emit_construction`'s `concrete_type_ident`).
        assert!(
            generated_str.contains("ContentControl :: new"),
            "{generated_str}"
        );

        // `ContentControl`'s own generated code (produced when `builtin_modules()` is fed through
        // `generate_module` directly, mirroring how a real consumer's own `.elwind` component
        // would be generated) forwards `content` into `Control`'s children and exposes both
        // `#[param]` fields as public accessors. `builtins.elwind` bundles every builtin into one
        // module, so only `ContentControl`'s own `Item::Component`/`Item::View` pair is kept —
        // `generate_module` would otherwise also try (and fail) to generate every shape-only
        // builtin sharing that module (mirroring `compile_dir_impl`'s own filtering in `lib.rs`).
        let builtins_module = crate::builtin_modules()
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
        // `content`/`padding` are `#[class]`-managed own (untagged) methods now (docs/
        // elwindui_spec.md 付録H.2.1a) — the macro derives the matching trait declaration/impl from
        // these at expansion time, invisible in these pre-expansion generated tokens.
        assert!(
            content_control_str
                .contains("fn content (& self) -> std :: rc :: Rc < dyn UIElement >")
        );
        assert!(content_control_str.contains("fn padding (& self) -> Option < f32 >"));
        // Real struct is always the bare `ContentControl` name itself — the *source* `#[class]` is
        // written against that same bare name (docs/elwindui_spec.md 付録H.2.1a); the macro derives
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
component Foo {
}

view Foo {
    Button {
        TextBlock { text: "not a valid Button child" }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
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
component Foo {
}

view Foo {
    MenuBarItem {
        text: "File"
        Menu { }
        Menu { }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        generate_module(&module, &table);
    }

    /// A component inheriting a logical, `has_view`-having base (`ContentControl`) with *no* `view`
    /// of its own — WinUI3-style template inheritance (`resolve_view_for`): the generated code is a
    /// full `generate_view`-style struct+impl (not `generate_component`'s plain struct), targeting
    /// the derived component's own name, with `ContentControl`'s inherited template underneath.
    #[test]
    fn generates_valid_rust_for_template_inheritance_with_no_own_view() {
        let src = r#"
component LabeledPanel inherits ContentControl {
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("labeled_panel_template_inheritance", &generated);

        let generated_str = generated.to_string();
        // The compiled struct is always the bare `LabeledPanel` name itself — the *source* `#[class]`
        // is written against that same bare name (docs/elwindui_spec.md 付録H.2.1a) — same reasoning
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
        let src = r#"
component Base {
    #[virtual]
    fn label(&self) -> String {
        "base".to_string()
    }
}

view Base {
    on_mount {
        println!("base mounted");
    }
    VerticalLayout { }
}

component Derived inherits Base {
    #[override]
    fn label(&self) -> String {
        format!("{}!", base::label())
    }
}

view Derived {
    on_mount {
        base::on_mount();
        println!("derived mounted");
    }
    VerticalLayout { }
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("method_override_and_on_mount", &generated);

        let generated_str = generated.to_string();
        assert!(generated_str.contains("fn __base_label"), "{generated_str}");
        assert!(
            generated_str.contains("fn __base_on_mount"),
            "{generated_str}"
        );
        assert!(
            generated_str.contains("this . __base_on_mount"),
            "{generated_str}"
        );
    }

    /// `Grid` (§3) + attached properties (`Grid::row`/`Grid::column`, §3) end to end: a `view`
    /// using `Grid` with `rows`/`columns` array-literal params and attached setters on its children
    /// must generate valid Rust, constructing `elwindui::core::ui::Grid` directly (a virtual
    /// builtin, like `Control`/`Shape`) with each virtual child's own `grid_cell` populated.
    #[test]
    fn generates_valid_rust_for_grid_with_attached_properties() {
        let src = r##"
component Foo {
}

view Foo {
    Grid {
        rows: [elwindui::core::layout::GridLength::Auto, elwindui::core::layout::GridLength::Star(1.0)]
        columns: [elwindui::core::layout::GridLength::Fixed(120.0), elwindui::core::layout::GridLength::Star(1.0)]
        TextBlock { text: "Header", Grid::row: 0, Grid::column: 0 }
        Shape { fill: "#000000", Grid::row: 1, Grid::column: 1 }
    }
}
"##;
        let module = parse_module(src).expect("should parse");
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

    /// Verifies the attached-property behavior specified in docs/elwindui_spec.md §3:
    /// a `has_view`/plain user-defined `component`+`view` pair (non-native-rooted, so it has a real
    /// `into_node()`) used as a `Grid` child must still have its `Grid::row`/`Grid::column` reach
    /// that child's own view-root `UIElementImpl`, not be silently dropped.
    #[test]
    fn generates_valid_rust_for_grid_child_that_is_a_user_component() {
        let src = r#"
component Cell {
}

view Cell {
    TextBlock { text: "x" }
}

component Foo {
}

view Foo {
    Grid {
        rows: [elwindui::core::layout::GridLength::Auto]
        columns: [elwindui::core::layout::GridLength::Auto]
        Cell { Grid::row: 1, Grid::column: 2 }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
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
