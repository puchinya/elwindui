//! AST(検証済み) → backend別Rustソース。`target::backend()`の定数畳み込みは付録Dの通りCargo
//! featureでの静的分岐に落とし込み、`elwindui-core`のトレイト境界に対して書かれたコードを生成する
//! (今回はelwindui-backend-appkitのAPIを直接呼ぶ)。
//! 依存関係グラフに基づくCell/RefCellベースの更新関数生成は付録O.5に対応する。

use crate::ast::{
    Attr, ChildEntry, ClosureBody, ComponentDef, ElementNode, EnumDef, FieldDef, FieldKind, Initializer, Item,
    MethodDef, Module, ViewDef, ViewExpr, ViewModelDef,
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
    /// `#[param]`-shaped fields (no initializer), in declaration order — the positional argument
    /// list `Target::new(...)` expects. Used to construct a nested user-defined component from an
    /// `ElementNode` (e.g. a `render_content` closure's `DocumentView { doc: doc }` body).
    pub param_fields: Vec<(String, String)>,
    /// Names of `#[param] #[two_way]` fields — a builtin shape's opt-in to automatic two-way
    /// wiring (see `emit_wiring`'s generic two-way rule). Empty for ordinary user components.
    pub two_way_fields: HashSet<String>,
    /// Names of `#[routed]` fields (docs/elwindui_spec.md 4章) — a callback's opt-in to WinUI3-
    /// style bubbling via `elwindui_core::ui::dispatch_routed` instead of being called directly.
    /// Non-empty exactly when this type needs `into_node_if_needed` to share its own
    /// `routed_handlers()` into the `NativeControl`/virtual-builtin `UIElementBase` wrapping it,
    /// rather than starting that wrapper with a fresh, empty one.
    pub routed_fields: HashSet<String>,
    /// Every field with no initializer, `#[param]` or not, mapped to its declared type — used
    /// purely for type-hint lookups (an `on_*` callback's arity, a resync setter's by-value-vs-
    /// by-reference calling convention), independent of whether the field is a constructor
    /// argument. A callback shape field (e.g. `TabView`'s `on_select: Box<dyn Fn(usize)>`) is
    /// deliberately *not* `#[param]` — it's wired post-construction via `emit_wiring`'s generic
    /// `on_*` rule, not passed to `Target::new(...)` — so it never appears in `param_fields`, but
    /// still needs its declared type visible here for the arity check.
    pub field_types: HashMap<String, String>,
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
    /// owns a real `base: elwindui_core::ui::NativeControlImpl<H>` and implements
    /// `NativeControl<H>`/`UIElement` by delegating to it (docs/elwindui_spec.md 付録H.2.1a).
    /// `emit_construction` uses this to pass a use-site `base: UIElementImpl` as this type's
    /// `Type::new(..)`'s leading argument (mirroring `emit_virtual_construction`'s own `base` — see
    /// `build_ui_element_base`), and `into_node_if_needed` uses it to skip the external
    /// `NativeControlImpl` wrapping it used to build, since the value already implements
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
    /// - Directly against a hand-written `elwindui_core::ui` primitive: this component's own
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
    /// a `view` construction of this component binds to, replacing the old field-declaration-order
    /// ("first still-unclaimed non-`Option` field") fallback. `None` for a `viewmodel` and for any
    /// component that doesn't declare `#[content(..)]`.
    pub content_field: Option<String>,
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
            let [prefix @ .., last] = u.path.as_slice() else { return None };
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
    let mut component_meta: HashMap<(Vec<String>, String), (usize, Option<String>, Option<String>, bool)> = HashMap::new();

    for (module_index, module) in modules.iter().enumerate() {
        for item in &module.items {
            let Item::Component(c) = item else { continue };
            let view_root = resolve_view_for(module, c, modules).map(|v| v.root.type_path.clone());
            component_meta.insert((module.path.clone(), c.name.clone()), (module_index, c.base.clone(), view_root, c.native));
        }

        for item in &module.items {
            match item {
                Item::Component(c) => {
                    let effective_fields = resolve_effective_fields(module, c, modules);
                    let effective_methods = resolve_effective_methods(module, c, modules);
                    let effective_view = resolve_view_for(module, c, modules);
                    let own_view = find_view(module, &c.name);
                    let field_kinds = effective_fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
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
                    // `#[param]`'s "fixed at instantiation" no longer reflects reality there), this
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
                        .filter(|f| f.initializer.is_none() && f.attrs.iter().any(|a| matches!(a, Attr::TwoWay)))
                        .map(|f| f.name.clone())
                        .collect();
                    let routed_fields = effective_fields
                        .iter()
                        .filter(|f| f.initializer.is_none() && f.attrs.iter().any(|a| matches!(a, Attr::Routed)))
                        .map(|f| f.name.clone())
                        .collect();
                    let field_types = effective_fields
                        .iter()
                        .filter(|f| f.initializer.is_none())
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
                            field_types,
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
                        .filter(|f| f.initializer.is_none() && f.attrs.iter().any(|a| matches!(a, Attr::TwoWay)))
                        .map(|f| f.name.clone())
                        .collect();
                    let routed_fields = v
                        .fields
                        .iter()
                        .filter(|f| f.initializer.is_none() && f.attrs.iter().any(|a| matches!(a, Attr::Routed)))
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
                            field_types,
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
        resolve_composed_shape(key, &component_meta, modules, &table, &mut composed_shape_memo);
    }

    let host_composition_memo: HashMap<(Vec<String>, String), Option<(String, (Vec<String>, String))>> = keys
        .iter()
        .map(|key| (key.clone(), resolve_host_composition_base(key, &component_meta, modules, &table, &memo)))
        .collect();
    let host_composition_base_keys: HashSet<(Vec<String>, String)> =
        host_composition_memo.values().filter_map(|v| v.as_ref().map(|(_, base_key)| base_key.clone())).collect();

    let mut types = table.types;
    for (key, info) in types.iter_mut() {
        info.is_native = memo.get(key).copied().unwrap_or(false);
        info.composed_shape = composed_shape_memo.get(key).cloned().flatten();
        info.host_composition_base = host_composition_memo.get(key).cloned().flatten().map(|(name, _)| name);
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
    if let Some(found) = modules.iter().filter(|m| m.path == from.path).find_map(|m| {
        m.items.iter().find_map(|i| match i {
            Item::Component(c) if c.name == name => Some((m, c)),
            _ => None,
        })
    }) {
        return Some(found);
    }
    for u in &from.uses {
        let [prefix @ .., last] = u.path.as_slice() else { continue };
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
pub(crate) fn resolve_effective_fields<'m>(from: &'m Module, c: &ComponentDef, modules: &'m [Module]) -> Vec<FieldDef> {
    let Some(base) = c.base.as_deref() else {
        return c.fields.clone();
    };
    if base == "NativeControl" {
        return c.fields.clone();
    }
    let Some((base_module, base_c)) = find_component_and_module(from, base, modules) else {
        return c.fields.clone();
    };
    let base_fields = resolve_effective_fields(base_module, base_c, modules);
    let base_fields: Vec<FieldDef> = match find_view(from, &c.name) {
        Some(view) => base_fields.into_iter().filter(|f| view_references_bare_name(view, &f.name)).collect(),
        None => base_fields,
    };
    let own_names: HashSet<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
    let mut result: Vec<FieldDef> =
        base_fields.into_iter().filter(|f| !own_names.contains(f.name.as_str())).collect();
    result.extend(c.fields.iter().cloned());
    result
}

/// Whether `view`'s element tree references `name` as a *bare* value anywhere — a 1-segment
/// `ViewExpr::Path` (`padding: padding`) or a bare `ChildEntry::Ref` (`Control { content }`) — as
/// opposed to a literal/computed value (`fill: "#3a3a3c"`) or no mention at all. See
/// `resolve_effective_fields`'s doc comment.
fn view_references_bare_name(view: &ViewDef, name: &str) -> bool {
    view.lets.iter().any(|l| element_references_bare_name(&l.element, name))
        || element_references_bare_name(&view.root, name)
}

fn element_references_bare_name(node: &ElementNode, name: &str) -> bool {
    if node.attributes.iter().any(|(_, expr)| view_expr_references_bare_name(expr, name)) {
        return true;
    }
    node.children.iter().any(|child| match child {
        ChildEntry::Literal(elem) => element_references_bare_name(elem, name),
        ChildEntry::Ref(n) => n == name,
    })
}

fn view_expr_references_bare_name(expr: &ViewExpr, name: &str) -> bool {
    match expr {
        ViewExpr::Path(path) => path.len() == 1 && path[0] == name,
        ViewExpr::Element(elem) => element_references_bare_name(elem, name),
        ViewExpr::Closure { body: ClosureBody::Element(elem), .. } => element_references_bare_name(elem, name),
        ViewExpr::TFluent(_, args) => args.iter().any(|(_, v)| view_expr_references_bare_name(v, name)),
        ViewExpr::MethodCall(..) | ViewExpr::Expr(_) | ViewExpr::Closure { body: ClosureBody::Expr(_), .. } => false,
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
    view.lets.iter().any(|l| element_references_name_anywhere(&l.element, name))
        || element_references_name_anywhere(&view.root, name)
}

fn element_references_name_anywhere(node: &ElementNode, name: &str) -> bool {
    if node.attributes.iter().any(|(_, expr)| view_expr_references_name_anywhere(expr, name)) {
        return true;
    }
    if node.attached.iter().any(|(_, _, expr)| view_expr_references_name_anywhere(expr, name)) {
        return true;
    }
    node.children.iter().any(|child| match child {
        ChildEntry::Literal(elem) => element_references_name_anywhere(elem, name),
        ChildEntry::Ref(n) => n == name,
    })
}

fn view_expr_references_name_anywhere(expr: &ViewExpr, name: &str) -> bool {
    match expr {
        ViewExpr::Path(path) => path.iter().any(|seg| seg == name),
        ViewExpr::MethodCall(path, _) => path.iter().any(|seg| seg == name),
        ViewExpr::Element(elem) => element_references_name_anywhere(elem, name),
        ViewExpr::Closure { body: ClosureBody::Element(elem), .. } => element_references_name_anywhere(elem, name),
        ViewExpr::Closure { body: ClosureBody::Expr(e), .. } => view_expr_references_name_anywhere(e, name),
        ViewExpr::TFluent(_, args) => args.iter().any(|(_, v)| view_expr_references_name_anywhere(v, name)),
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

/// Recursively flattens `c`'s effective method list: its base's own effective methods (an
/// `#[override]`n one is kept alongside under a mangled `__base_<name>` so the override's body can
/// still reach it via `base::name(...)`, rewritten by `rewrite_base_calls`), followed by `c`'s own
/// methods (an override's body rewritten the same way). See `ComponentDef`'s doc comment. Only one
/// `inherits` hop's worth of `base::` chaining is guaranteed correct — see `generate_view`'s doc
/// comment on `own_on_mount`/`own_on_unmount` for the same limitation applied to lifecycle hooks.
pub(crate) fn resolve_effective_methods<'m>(from: &'m Module, c: &ComponentDef, modules: &'m [Module]) -> Vec<MethodDef> {
    let mut result = Vec::new();
    if let Some(base) = c.base.as_deref() {
        if base != "NativeControl" {
            if let Some((base_module, base_c)) = find_component_and_module(from, base, modules) {
                let base_methods = resolve_effective_methods(base_module, base_c, modules);
                let overridden: HashSet<&str> =
                    c.methods.iter().filter(|m| m.is_override).map(|m| m.name.as_str()).collect();
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
/// an explicit hand-written `view`, unchanged from before this feature — see
/// `validate::validate_inherits`).
pub(crate) fn resolve_view_for<'m>(from: &'m Module, c: &ComponentDef, modules: &'m [Module]) -> Option<ViewDef> {
    if let Some(own) = find_view(from, &c.name) {
        return Some(own.clone());
    }
    let base = c.base.as_deref()?;
    if base == "NativeControl" {
        return None;
    }
    let (base_module, base_c) = find_component_and_module(from, base, modules)?;
    let base_view = resolve_view_for(base_module, base_c, modules)?;
    Some(ViewDef { target: c.name.clone(), ..base_view })
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
                    let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
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
                    Some(root_key) => resolve_is_native(&root_key, component_meta, modules, table, memo),
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
        let (module_index, base, view_root, _native) = component_meta.get(key)?;
        let base = base.as_deref()?;
        if base == "NativeControl" {
            return None;
        }
        let from = &modules[*module_index];
        let has_own_view = find_view(from, &key.1).is_some();

        if is_virtual_builtin(base) {
            // Direct shape composition against a hand-written `elwindui_core::ui` primitive
            // (`ContentControl inherits Control`): this component's own effective root must be
            // exactly `base` — matching `validate::validate_inherits`'s own requirement that an
            // explicit `view` is needed here.
            return (view_root.as_deref() == Some(base)).then(|| base.to_string());
        }

        let base_key = table.resolve_key(from, base)?;
        let base_composed = resolve_composed_shape(&base_key, component_meta, modules, table, memo);

        if has_own_view {
            // Direct composition against an *already-composed DSL component*, one delegation hop
            // further out (`RoundedPanel inherits ContentControl`, own `view` root literally
            // `ContentControl`) — the same shape as the virtual-builtin case above, just one level
            // up the chain. `generate_view`'s `is_shape_composition` doesn't otherwise care whether
            // `base` is a hand-written primitive or another composed DSL component, since it always
            // delegates through `self.base` regardless of that type's own nature.
            (view_root.as_deref() == Some(base)).then_some(())?;
            base_composed
        } else {
            // Template composition (`LabeledPanel inherits ContentControl`): only eligible when this
            // component writes no `view` of its own (a full override has an independent tree — see
            // `generate_view`'s own `is_shape_composition`/`is_template_composition` doc comments for
            // why that case keeps the `resolve_effective_fields`/`__base_<name>` mechanism instead),
            // and only if the base itself is already composed.
            base_composed
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
    let (module_index, base, view_root, _native) = component_meta.get(key)?;
    let base = base.as_deref()?;
    if base == "NativeControl" || is_virtual_builtin(base) {
        return None;
    }
    let from = &modules[*module_index];
    let base_key = table.resolve_key(from, base)?;
    let base_info = table.types.get(&base_key)?;
    let base_is_native = is_native_memo.get(&base_key).copied().unwrap_or(false);
    if base_is_native && !base_info.has_view && !base_info.is_native_control_leaf {
        (view_root.as_deref() == Some(base)).then(|| (base.to_string(), base_key))
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
                let info = table
                    .resolve(module, &c.name)
                    .unwrap_or_else(|| panic!("component `{}` missing from its own symbol table", c.name));
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
    matches!(ty, "i32" | "i64" | "f32" | "f64" | "bool" | "u32" | "u64" | "usize") || {
        // A bare, capitalized single-word type that isn't a known non-Copy std type is assumed to
        // be one of this file's own enums (all generated with `derive(Copy)`, see `generate_enum`).
        ty.chars().next().is_some_and(|c| c.is_uppercase()) && ty != "String" && ty != "Command"
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
    let looks_nested = inner.chars().next().is_some_and(|c| c.is_uppercase())
        && !matches!(inner, "String" | "Command");
    (known || looks_nested).then(|| inner.to_string())
}

pub fn generate_viewmodel(v: &ViewModelDef, from: &Module, table: &SymbolTable) -> TokenStream {
    let struct_name = format_ident!("{}", v.name);
    let field_names: HashSet<&str> = v.fields.iter().map(|f| f.name.as_str()).collect();

    // Every viewmodel is `Rc::new_cyclic`-constructed (see `new()` below) and carries a
    // `__self_weak: Weak<Self>` so a `#[command(async)]` body can upgrade to an owned `Rc<Self>`
    // before spawning — `elwindui_core::task::spawn_local` requires its future to be `'static`,
    // which a body referencing sibling fields through a borrowed `&self` can't satisfy (the future
    // may genuinely outlive this call, unlike the old poll-once `__elwindui_block_on_ready`). See
    // the `FieldKind::Command` `is_async` arm below and docs/elwindui_spec.md 付録P.5.

    // `#[computed]` fields and `#[command(can_execute: ...)]` both need a dependency list so that
    // each observable's setter can call exactly the recompute functions that depend on it,
    // matching 付録O.5's "具体的な更新関数を静的に生成する" (no dynamic subscriber list).
    let mut dependents_of: HashMap<String, Vec<String>> = HashMap::new();
    for f in &v.fields {
        if f.kind == FieldKind::Computed {
            if let Some(Initializer::Expr(expr)) = &f.initializer {
                for dep in referenced_fields(expr, &field_names) {
                    dependents_of.entry(dep).or_default().push(f.name.clone());
                }
            }
        }
        if f.kind == FieldKind::Command {
            if let Some(Attr::CommandMeta { can_execute: Some(expr), .. }) =
                f.attrs.iter().find(|a| matches!(a, Attr::CommandMeta { .. }))
            {
                for dep in referenced_fields(expr, &field_names) {
                    dependents_of
                        .entry(dep)
                        .or_default()
                        .push(format!("{}_can_execute", f.name));
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
                let item_ty: syn::Type = syn::parse_str(&nested_vec_item_type(&f.ty, from, table).unwrap())
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
                        quote! { self.#recompute(); }
                    })
                    .collect();

                accessors.extend(quote! {
                    pub fn #getter(&self) -> Vec<std::rc::Rc<#item_ty>> {
                        self.#field_ident.borrow().clone()
                    }
                    pub fn #pusher(&self, item: std::rc::Rc<#item_ty>) {
                        self.#field_ident.borrow_mut().push(item);
                        #(#recompute_calls)*
                        for f in self.__resync_subscribers.borrow().iter() { f(); }
                    }
                    pub fn #remover(&self, index: usize) {
                        self.#field_ident.borrow_mut().remove(index);
                        #(#recompute_calls)*
                        for f in self.__resync_subscribers.borrow().iter() { f(); }
                    }
                });
            }
            FieldKind::Observable => {
                let field_ident = format_ident!("{}", f.name);
                let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
                let init_expr = match &f.initializer {
                    Some(Initializer::Expr(e)) => {
                        rewrite_field_refs(coerce_to_owned_string(&f.ty, e.clone()), &field_names, &format_ident!("self"))
                    }
                    _ => panic!("observable field `{}` needs a plain initializer expr", f.name),
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
                        quote! { self.#recompute(); }
                    })
                    .collect();

                accessors.extend(quote! {
                    pub fn #getter(&self) -> #ty { #get_body }
                    pub fn #setter(&self, value: #ty) {
                        #set_body
                        #(#recompute_calls)*
                        for f in self.__resync_subscribers.borrow().iter() { f(); }
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
            FieldKind::Command => {
                let (is_async, can_execute_expr) = f
                    .attrs
                    .iter()
                    .find_map(|a| match a {
                        Attr::CommandMeta { is_async, can_execute } => {
                            Some((*is_async, can_execute.clone()))
                        }
                        _ => None,
                    })
                    .unwrap_or((false, None));
                let can_execute_ident = format_ident!("{}_can_execute", f.name);
                let can_execute_cache = format_ident!("{}_can_execute_cache", f.name);
                let can_execute_expr_ts = match &can_execute_expr {
                    Some(expr) => rewrite_field_refs(expr.clone(), &field_names, &format_ident!("self")),
                    None => quote! { true },
                };

                struct_fields.extend(quote! { #can_execute_cache: std::cell::Cell<bool>, });
                ctor_fields.extend(quote! { #can_execute_cache: std::cell::Cell::new(true), });

                let recompute_can_execute = format_ident!("recompute_{}_can_execute", f.name);
                accessors.extend(quote! {
                    pub fn #can_execute_ident(&self) -> bool { self.#can_execute_cache.get() }
                    fn #recompute_can_execute(&self) {
                        let value: bool = #can_execute_expr_ts;
                        self.#can_execute_cache.set(value);
                    }
                });
                recompute_calls_after_new.extend(quote! { instance.#recompute_can_execute(); });

                let Some(Initializer::Command { params, body: block }) = &f.initializer else {
                    panic!("#[command] field `{}` needs a command!(...) initializer", f.name);
                };
                let execute_ident = format_ident!("{}_execute", f.name);
                let param_decls = params.iter().map(|(name, ty)| {
                    let ident = format_ident!("{}", name);
                    quote! { #ident: #ty }
                });
                if is_async {
                    // See docs/elwindui_spec.md 付録P.4/P.5. `elwindui_core::task::spawn_local`
                    // requires a `'static` future, which a body referencing sibling fields through
                    // a borrowed `&self` can't provide (the future may genuinely outlive this
                    // call, unlike the old poll-once `__elwindui_block_on_ready`) — so the body is
                    // rewritten against an owned `__self: Rc<Self>` (upgraded from `__self_weak`)
                    // instead of `self`. `spawn_local` polls the future once immediately (covering
                    // today's modal-dialog `.await`s, which never really suspend, at no extra
                    // cost) and, if it genuinely suspends, resumes it later on this same (UI)
                    // thread via the active backend's `Dispatcher` — see `elwindui-core/src/task.rs`.
                    // `async move` (rather than plain `async`) so a parameterized command's
                    // argument is captured by value, matching 付録O.4's parameterized-command
                    // extension.
                    let self_ident = format_ident!("__self");
                    let rewritten_block = rewrite_command_body(block.clone(), &field_names, &self_ident);
                    accessors.extend(quote! {
                        pub fn #execute_ident(&self, #(#param_decls),*) {
                            let __self = self.__self_weak.upgrade().expect(
                                "elwindui: viewmodel was dropped while a #[command(async)] was still pending"
                            );
                            elwindui_core::task::spawn_local(async move #rewritten_block);
                        }
                    });
                } else {
                    let self_ident = format_ident!("self");
                    let rewritten_block = rewrite_command_body(block.clone(), &field_names, &self_ident);
                    accessors.extend(quote! {
                        pub fn #execute_ident(&self, #(#param_decls),*) #rewritten_block
                    });
                }
            }
            FieldKind::Prop | FieldKind::Param | FieldKind::Attached => {
                panic!("viewmodel field `{}` must be #[observable]/#[computed]/#[command]", f.name);
            }
        }
    }

    quote! {
        pub struct #struct_name {
            #struct_fields
            // A dynamic subscriber list, unlike `dependents_of` above: the *number* of components
            // that end up `bind!`-ing to this viewmodel instance (e.g. one `DocumentView` per open
            // notepad tab) isn't known at compile time, so it can't be resolved into a static
            // per-field recompute call the way `#[computed]`/`#[command(can_execute)]` dependents
            // are (付録O.5). See docs/elwindui_spec.md §10/付録J.3: any mutation of a field reachable
            // through `bind!` must propagate to every subscribing `prop`, not just ones reached via
            // that same component's own wired `on_*` callbacks.
            __resync_subscribers: std::cell::RefCell<Vec<Box<dyn Fn()>>>,
            // Lets an async `#[command(async)]` body upgrade to an owned `Rc<Self>` before
            // spawning (see the `FieldKind::Command` `is_async` arm) instead of capturing a
            // borrowed `&self` that can't outlive this call. Unused (and so `#[allow(dead_code)]`)
            // on a viewmodel with no async command.
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
                        __resync_subscribers: std::cell::RefCell::new(Vec::new()),
                        __self_weak: __self_weak.clone(),
                    };
                    #recompute_calls_after_new
                    instance
                })
            }

            /// Registers `f` to run after any `#[observable]` field on this instance changes.
            /// Called by a `bind!`-ing component's generated `new()` so its `resync()` re-fires
            /// whenever this viewmodel changes, regardless of which code path mutated it.
            pub fn subscribe(&self, f: impl Fn() + 'static) {
                self.__resync_subscribers.borrow_mut().push(Box::new(f));
            }

            #accessors
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
                        self.found.extend(referenced_fields(value, self.field_names));
                    }
                }
            }
            syn::visit::visit_expr_macro(self, node);
        }
    }
    let mut collector = Collector { field_names, found: Vec::new() };
    collector.visit_expr(expr);
    collector.found.sort();
    collector.found.dedup();
    collector.found
}

/// Rewrites bare identifier reads that name a sibling field (`content` inside a `#[computed]`
/// initializer) into accessor calls (`self.content()`). Does not touch assignment targets —
/// `command!` bodies use [`rewrite_command_body`] for that.
fn rewrite_field_refs(mut expr: syn::Expr, field_names: &HashSet<&str>, receiver: &syn::Ident) -> TokenStream {
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
    let mut rewriter = Rewriter { field_names, receiver };
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
fn rewrite_t_macro(expr: TokenStream, field_names: &HashSet<&str>, receiver: &syn::Ident) -> TokenStream {
    let expr: syn::Expr = syn::parse2(expr).expect("rewrite_field_refs always yields valid Expr");
    if let syn::Expr::Macro(m) = &expr {
        if m.mac.path.is_ident("t") {
            return rewrite_t_call(&m.mac.tokens, field_names, receiver);
        }
    }
    quote! { #expr }
}

/// Parses a `t!(...)` macro's raw tokens (`"key", name1: expr1, name2: expr2`) into the key and
/// its named argument expressions. Shared by [`rewrite_t_call`] (codegen) and [`referenced_fields`]
/// (dependency-graph analysis) — both need to look inside the macro's opaque token stream, since
/// `syn::visit`/`syn::visit_mut` never descend into a macro's tokens on their own.
fn parse_t_macro_tokens(tokens: &TokenStream) -> syn::Result<(syn::LitStr, Vec<(syn::Ident, syn::Expr)>)> {
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

fn rewrite_t_call(tokens: &TokenStream, field_names: &HashSet<&str>, receiver: &syn::Ident) -> TokenStream {
    // Tokens look like: "key", name1: expr1, name2: expr2
    let (key, args) = parse_t_macro_tokens(tokens)
        .expect("t!(...) arguments must be `\"key\", name: expr, ...`");
    let arg_pairs = args.iter().map(|(name, value)| {
        let name_str = name.to_string();
        let value = rewrite_field_refs(value.clone(), field_names, receiver);
        quote! { (#name_str, elwindui_i18n::FluentValue::from(#value)) }
    });
    quote! { elwindui_i18n::t(#key, &[ #(#arg_pairs),* ]) }
}

/// Rewrites a `command!(|| { ... })` body: assignments to a sibling field (`state = expr`) become
/// setter calls, bare reads of a sibling field become getter calls, and the whole thing becomes a
/// method body (`fn f(&self) { ... }`) rather than a closure. `receiver` is `self` for a plain
/// (synchronous) command, or an owned local (`__self: Rc<Self>`) for an async one — see the
/// `FieldKind::Command` `is_async` arm for why a borrowed `self` won't do there.
fn rewrite_command_body(mut block: syn::Block, field_names: &HashSet<&str>, receiver: &syn::Ident) -> TokenStream {
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
            // `t!(...)` inside a command body: `syn::visit_mut` never descends into a macro's
            // token stream, so this has to be special-cased the same way as
            // `rewrite_t_macro`/`rewrite_t_call` (used for `#[computed]` initializers).
            if let syn::Expr::Macro(m) = node {
                if m.mac.path.is_ident("t") {
                    let rewritten = rewrite_t_call(&m.mac.tokens, self.field_names, self.receiver);
                    *node = syn::parse2(rewritten).expect("rewrite_t_call always yields a valid Expr");
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
    let mut rewriter = Rewriter { field_names, receiver };
    rewriter.visit_block_mut(&mut block);
    quote! { #block }
}

fn generate_component(c: &ComponentDef, table: &SymbolTable) -> TokenStream {
    let struct_name = format_ident!("{}", c.name);
    let mut struct_fields = TokenStream::new();
    let mut ctor_params = TokenStream::new();
    let mut ctor_field_inits = TokenStream::new();
    let mut accessors = TokenStream::new();

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
                // deferred field's storage type is no longer the field's own declared type.
                let (inner_ty_str, is_option) = strip_option(&f.ty);
                if is_option {
                    let inner_ty: syn::Type = syn::parse_str(inner_ty_str).expect("field inner type must parse");
                    let cell_ty = if is_copy_type(inner_ty_str) { quote! { std::cell::Cell } } else { quote! { std::cell::RefCell } };
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
            Some(Initializer::Expr(_)) | Some(Initializer::Command { .. }) => {
                panic!("component field `{}` initializer form not supported yet", f.name);
            }
        }
    }

    let _ = table; // reserved for future cross-component validation
    let methods = emit_methods(&c.methods);
    quote! {
        pub struct #struct_name {
            #struct_fields
        }

        impl #struct_name {
            pub fn new(#ctor_params) -> Self {
                Self { #ctor_field_inits }
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
        let vis = if m.name.starts_with("__base_") { quote! {} } else { quote! { pub } };
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

fn generate_view(view: &ViewDef, component: &ComponentDef, from: &Module, table: &SymbolTable) -> TokenStream {
    let target_name = view.target.clone();
    let target = format_ident!("{}", target_name);
    let has_own_view = find_view(from, &target_name).is_some();

    // `component X inherits Y` where `Y` is a virtual-builtin shape primitive (`Control`/
    // `Rectangle`/`Ellipse`/`TextBlock`/`Grid`/`VerticalLayout`/`HorizontalLayout` —
    // `is_virtual_builtin`) and `X`'s own view root is literally a construction of `Y`
    // (`validate::validate_inherits` already enforces this) — the real, load-bearing case of
    // docs/elwindui_spec.md 付録H.2.1a's `struct XImpl { base: YImpl, .. }` composition: `X`'s
    // generated struct embeds `Y`'s real `elwindui_core::ui` `YImpl` as its own `base` field and
    // implements `UIElement` (and `Y`'s own trait) by delegating to it, instead of the ordinary
    // "wrapper owns a separately-`Rc`-erased root" shape every other `view`-having component uses
    // (see this function's tail `quote!`).
    let composed_shape = table.resolve(from, &target_name).and_then(|i| i.composed_shape.clone());
    let is_shape_composition = has_own_view && composed_shape.is_some();
    // `component X inherits Y` where `Y` is itself already composed (either directly, like
    // `ContentControl`, or transitively) and `X` writes no `view` of its own — pure template reuse
    // (`LabeledPanel inherits ContentControl`): `X`'s struct embeds `Y`'s own (unwrapped) value as
    // its `base` field, built by calling `Y`'s own `create_<snake_case Y>(..)` plain factory (see
    // `snake_case_ident`/the `param_forward_split` below) instead of `build_virtual_value`.
    // Deliberately excludes a component with its *own* `view` that merely inherits another
    // component's *code* rather than its structure (`Derived inherits Base`, both independently
    // `VerticalLayout`-rooted) — there `base::name(...)`/`base::on_mount()` only ever need to reach
    // the base's method *bodies*, not a live base *instance* (no different from `super.method()` in
    // a mainstream OOP language never requiring a separate freestanding `super` object to exist), so
    // the original `resolve_effective_fields`/`resolve_effective_methods`/`__base_<name>` mechanism
    // already does exactly the right thing and is left unchanged.
    let is_template_composition = !has_own_view && composed_shape.is_some();
    // `component X inherits Y` where `Y` is a hand-written native host with no `UIElement`
    // implementation of its own (only `Window` today) and `X`'s own view root literally constructs
    // `Y` — "host composition" (docs/elwindui_spec.md 付録H.2.1a, `TypeInfo::host_composition_base`).
    // Follows the same `base`-field/`XImpl`-rename/synthesized-trait shape as shape composition
    // below, just without an `impl UIElement` (`Y` doesn't implement it either) — see this
    // function's dedicated branch further down.
    let host_composition_base = table.resolve(from, &target_name).and_then(|i| i.host_composition_base.clone());
    let is_host_composition = host_composition_base.is_some();
    let is_composed = composed_shape.is_some() || is_host_composition;
    // A composed component's real struct is `XImpl`, not the bare DSL name `target`/`X` — that bare
    // name becomes a real `pub trait X: UIElement + ..` instead (docs/elwindui_spec.md 付録H.2.1a),
    // and Rust's shared type/trait namespace means the two can't coexist under one identifier.
    // External construction sites still write/generate `X::new(args)` unchanged — `emit_construction`
    // (via `concrete_type_ident`) is the one place that resolves `X` to this real `XImpl` struct name
    // before emitting the call, so nothing outside this function needs to know about the suffix.
    let struct_ident = if is_composed { format_ident!("{}Impl", target_name) } else { target.clone() };

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
    let own_fields: std::collections::HashMap<String, String> = component
        .fields
        .iter()
        .filter(|f| f.initializer.is_none())
        .map(|f| (f.name.clone(), f.ty.clone()))
        .collect();
    let ctx = ViewCtx { binds, closure_param: None, own_fields };

    let param_names: Vec<syn::Ident> = component
        .fields
        .iter()
        .filter(|f| f.initializer.is_none())
        .map(|f| format_ident!("{}", f.name))
        .collect();
    let param_types: Vec<syn::Type> = component
        .fields
        .iter()
        .filter(|f| f.initializer.is_none())
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
    let mut own_struct_param_names: Vec<syn::Ident> =
        if is_template_composition { param_names[base_param_count.min(param_names.len())..].to_vec() } else { param_names.clone() };
    // Assigned below once `shape_forwarded_names` is known (`is_shape_composition` narrows this
    // further still), from `own_struct_param_names`'s own final value — see there.
    let own_struct_param_types: Vec<syn::Type>;

    // `bind!(owner.field, mode)` fields whose `owner` is one of this component's own `#[param]`
    // dependencies and whose `mode` isn't `OneTime` (docs/elwindui_spec.md §10: `OneTime` captures
    // once at instantiation and stays fixed, so it has nothing to subscribe to). Deduplicated by
    // owner, since one `resync()` call already re-reads every attribute bound to that owner
    // (`emit_resync` below), not just the specific field named in the `bind!`. Only owners whose
    // type is a `viewmodel` are kept — `validate_bind_path` allows `bind!` to target a plain
    // `component` too, but only `generate_viewmodel`'s output has a `subscribe` method.
    let mut bind_owners: Vec<syn::Ident> = Vec::new();
    for f in &component.fields {
        let Some(Initializer::Bind { path, mode }) = &f.initializer else { continue };
        if mode == "OneTime" {
            continue;
        }
        let [owner, _target] = path.as_slice() else { continue };
        let Some(owner_field) = component.fields.iter().find(|of| &of.name == owner) else { continue };
        let is_viewmodel = table
            .resolve(from, strip_rc_wrapper(&owner_field.ty))
            .is_some_and(|info| info.is_viewmodel);
        if is_viewmodel && !bind_owners.iter().any(|o| o.to_string() == *owner) {
            bind_owners.push(format_ident!("{}", owner));
        }
    }

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
            lets_map.insert(field.name.clone(), (format_ident!("{}", field.name), PASSTHROUGH_NODE.to_string()));
        }
    }
    for let_binding in &view.lets {
        let resolved = plan_element(&let_binding.element, &ctx, from, table, &mut plan, let_binding.id.is_some(), &lets_map);
        if let_binding.id.is_some() {
            plan.last_mut().expect("plan_element always pushes its own node").id = let_binding.id.clone();
        }
        lets_map.insert(let_binding.name.clone(), resolved);
    }

    plan_element(&view.root, &ctx, from, table, &mut plan, true, &lets_map);

    // Host composition (`is_host_composition`'s doc comment): the root's stored field must be
    // named `base` (the same trait+Impl+base convention `is_shape_composition` follows), not the
    // generic auto-numbered binding every other stored node gets — renamed here, before anything
    // below reads `node.binding`, so the ordinary "stored field" path (`struct_fields`/
    // `field_inits`), `emit_wiring`, and `emit_resync` all naturally reference `self.base` with no
    // further special-casing (unlike shape composition, the root here is still built by ordinary
    // `emit_construction`, so there's no separate construction path to intercept — only storage).
    if is_host_composition {
        plan.last_mut().expect("plan_element always pushes a node for the root").binding = format_ident!("base");
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
    // and *must* be kept: the underlying `elwindui_core::ui` base field it forwards into is often
    // a narrower stored shape (`ShapeImpl::stroke_width`/`ControlImpl::padding` are plain `f32`, not
    // `Option<f32>` — `build_virtual_value`'s `get_attr` unwraps via `.unwrap_or(0.0)` before
    // storing), so delegating its accessor to `self.base.<name>` would return the wrong type.
    let shape_forwarded_names: HashSet<String> = if is_shape_composition {
        let root_node = plan.last().expect("plan_element always pushes a node for the root");
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
    let own_struct_param_names_set: HashSet<String> = own_struct_param_names.iter().map(|n| n.to_string()).collect();
    own_struct_param_types = param_names
        .iter()
        .zip(param_types.iter())
        .filter(|(n, _)| own_struct_param_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();

    // docs/elwindui_spec.md 付録H.2.1a's post-construction setter convention, extended from
    // builtins (付録H.2.1a's own "Phase 1") to every `view`-having component: an `Option<T>`-typed
    // own field (never a forwarded one — those are excluded from `own_struct_param_names` above
    // already, and always consumed eagerly building `base`) whose name is never referenced anywhere
    // in *this* component's own view construction (`view_references_name_anywhere` — not just a
    // *bare* forward like `ContentControl`'s `Control { padding: padding }`, but also as a
    // sub-expression identifier, e.g. `Rectangle`'s own `corner_radius.unwrap_or(0.0)`; either way
    // the value is needed before `Self` even exists, so it can't be deferred) is dropped from
    // `new(..)`'s own argument list, stored `Cell`/`RefCell`-wrapped (`is_copy_type`) defaulting to
    // `None`, and given a `set_<name>(&self, value: T)` setter. Every other own field
    // (required/non-`Option`, or referenced anywhere in this component's own view regardless of
    // `Option`-ness) keeps today's exact behavior unchanged: a `new(..)` argument, plain storage, no
    // setter — there's no sound default to defer a `Default`-less required field to, and a
    // referenced field's value is needed eagerly at construction time either way.
    let is_deferred_own_field = |name: &syn::Ident| -> bool {
        let ty_str = ctx.own_fields.get(&name.to_string()).expect("own_struct_param_names names one of ctx.own_fields' own keys");
        strip_option(ty_str).1 && !view_references_name_anywhere(view, &name.to_string())
    };
    let deferred_own_names: Vec<syn::Ident> = own_struct_param_names.iter().filter(|n| is_deferred_own_field(n)).cloned().collect();
    let deferred_own_inner_types: Vec<syn::Type> = deferred_own_names
        .iter()
        .map(|n| {
            let ty_str = ctx.own_fields.get(&n.to_string()).expect("own_struct_param_names names one of ctx.own_fields' own keys");
            syn::parse_str(strip_option(ty_str).0).expect("field inner type must parse")
        })
        .collect();
    let deferred_own_cell_types: Vec<TokenStream> = deferred_own_names
        .iter()
        .zip(deferred_own_inner_types.iter())
        .map(|(n, inner_ty)| {
            let ty_str = ctx.own_fields.get(&n.to_string()).unwrap();
            let cell_ty = if is_copy_type(strip_option(ty_str).0) { quote! { std::cell::Cell } } else { quote! { std::cell::RefCell } };
            quote! { #cell_ty<Option<#inner_ty>> }
        })
        .collect();
    let deferred_own_names_set: HashSet<String> = deferred_own_names.iter().map(|n| n.to_string()).collect();
    // The `Self { .. }`/`#struct_ident { .. }` construction shorthand (`#(#name,)*`) only works for
    // a field with a live local variable of the same name — still true for a required own field
    // (still a `new(..)` argument), but not a deferred one (no argument, no local variable at all),
    // which instead needs an explicit `#name: #cell_ty::new(None)` initializer built here once and
    // reused by both `new(..)`'s own inline construction and `create_<snake case>(..)` below.
    let required_own_names: Vec<syn::Ident> =
        own_struct_param_names.iter().filter(|n| !deferred_own_names_set.contains(&n.to_string())).cloned().collect();
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
    let ctor_param_names: Vec<syn::Ident> = param_names.iter().filter(|n| !deferred_own_names_set.contains(&n.to_string())).cloned().collect();
    let ctor_param_types: Vec<syn::Type> = param_names
        .iter()
        .zip(param_types.iter())
        .filter(|(n, _)| !deferred_own_names_set.contains(&n.to_string()))
        .map(|(_, t)| t.clone())
        .collect();

    let mut struct_fields = TokenStream::new();
    let mut construct_stmts = TokenStream::new();
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
    // regardless of `is_composed`. See `composition_impls`'s own use of these two.
    let mut trait_sigs = TokenStream::new();
    let mut trait_impl_bodies = TokenStream::new();

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
    // `shape_forwarded_names` one reads the field straight off the base's `elwindui_core::ui`
    // struct instead — those structs' non-`Copy` fields are `RefCell`-wrapped (docs/elwindui_spec.md
    // 付録H.2.1a's post-construction setter convention), so this reads `self.base.<name>.borrow()
    // .clone()`, not a plain `.clone()` (unlike a DSL-composed base's own accessor method).
    for (name, ty) in param_names.iter().zip(param_types.iter()) {
        let is_forwarded = !own_struct_param_names.contains(name);
        let is_deferred = deferred_own_names_set.contains(&name.to_string());
        let body = if is_template_composition && is_forwarded {
            quote! { self.base.#name() }
        } else if is_forwarded {
            quote! { self.base.#name.borrow().clone() }
        } else if is_deferred {
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
        // as a real trait method (signature in the trait, body in `impl #target for
        // #struct_ident` — see `composition_impls`) rather than a plain inherent one, so it's
        // reachable generically through `dyn #target`/any bound on it — not just non-composed
        // (plain) components stay purely inherent (no trait to move into).
        if is_composed {
            trait_sigs.extend(quote! { fn #name(&self) -> #ty; });
            trait_impl_bodies.extend(quote! {
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
    // (unwrapped) type, bare — not `Option<T>` — matching Phase 1's builtin setter convention
    // exactly (`build_component_setters`): an absent value simply never calls this at all, leaving
    // the field's own `None` default in place, so the setter itself never needs to accept `None`.
    for (name, inner_ty) in deferred_own_names.iter().zip(deferred_own_inner_types.iter()) {
        let set_name = format_ident!("set_{}", name);
        let ty_str = ctx.own_fields.get(&name.to_string()).unwrap();
        let set_body = if is_copy_type(strip_option(ty_str).0) {
            quote! { self.#name.set(Some(value)); }
        } else {
            quote! { *self.#name.borrow_mut() = Some(value); }
        };
        if is_composed {
            trait_sigs.extend(quote! { fn #set_name(&self, value: #inner_ty); });
            trait_impl_bodies.extend(quote! {
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

    // `is_template_composition`'s `plan`/`view` are the *base's* own (cloned, `resolve_view_for`)
    // tree, not this component's — its only real construction step is calling the base's own
    // `create_<snake case>(..)` factory (below), so none of `plan`'s nodes are constructed or wired
    // here at all.
    let root_index = plan.len() - 1;
    if !is_template_composition {
        for (i, node) in plan.iter().enumerate() {
            // The shape-composition root (see `is_shape_composition`'s doc comment) is built as a
            // plain, unwrapped `elwindui_core::ui::create_xxx(...)` value under its own
            // `node.binding` name — not `new_element`-wrapped/erased into `Rc<dyn UIElement>` like
            // every other node — so it can be moved into `Self`'s own `base` field as-is (see the
            // `struct_fields`/`field_inits` branch below and this function's tail `quote!`).
            if is_shape_composition && i == root_index {
                let binding = &node.binding;
                // The base may be a hand-written `elwindui_core::ui` primitive (`Control`/`Shape`/
                // ...) or itself a resolved DSL component (`ContentControl`, for `RoundedPanel
                // inherits ContentControl`) — either way the result is a plain, unwrapped value
                // moved into `Self`'s own `base` field as-is (see the `struct_fields`/`field_inits`
                // branch below and this function's tail `quote!`), never `new_element`-wrapped/
                // erased into `Rc<dyn UIElement>` like every other node.
                if is_virtual_builtin(&view.root.type_path) {
                    let value = build_virtual_value(node, &ctx, from, table);
                    let (base_impl_ty, _) = shape_composition_base_types(&view.root.type_path);
                    construct_stmts.extend(quote! { let #binding: #base_impl_ty = #value; });
                } else {
                    let value = build_component_value(node, &ctx, from, table);
                    let base_impl_ty = concrete_type_ident(&view.root.type_path, table.resolve(from, &view.root.type_path));
                    construct_stmts.extend(quote! { let #binding: #base_impl_ty = #value; });
                }
                continue;
            }
            emit_construction(node, &ctx, from, table, &mut construct_stmts);
            if node.stored {
                let binding = &node.binding;
                // Every resolved type (a `component`/`view` pair or a hand-written builtin in
                // an `elwindui-backend-*` crate) is constructed as `Rc<Self>` uniformly (see `emit_construction`
                // and this same convention below in `root_embed_method`), so a stored handle is always
                // just `Rc<Type>` — no backend-crate-qualified path, no per-type bookkeeping fields.
                let type_ident = concrete_type_ident(&node.type_path, table.resolve(from, &node.type_path));
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
            emit_wiring(node, &ctx, from, table, &mut wiring_stmts);
            emit_resync(node, &ctx, from, table, &mut resync_stmts);
        }
    }

    // `plan_element` pushes children before their parent (post-order), so the root is always last.
    // Irrelevant (the base's own root, not this component's) when `is_template_composition`.
    let root_binding = &plan.last().expect("view must have a root element").binding;
    let root_is_virtual_builtin = !is_template_composition && is_virtual_builtin(&view.root.type_path);

    // A hardcoded virtual builtin root (`VerticalLayout`, say — `DocumentView`'s actual root) is
    // never `stored` (see `plan_element`), so unlike every other node its value only exists as the
    // bare local `let` binding `emit_construction` produced inside `new()`'s `construct_stmts` —
    // nothing stashes it on `Self` the normal way. Stash it here as a plain `Rc` field instead —
    // `Rc<dyn UIElement>` (unlike the old `Box`) is `Clone`, so `into_node()` (below) can just
    // clone it, the same convention every other stored field already uses.
    //
    // The shape-composition case (`is_shape_composition`) stashes it differently: as a real `base`
    // field of the shape's own `elwindui_core::ui` `YImpl` type (built unwrapped, above), not a
    // type-erased `Rc<dyn UIElement>` — see this function's tail `quote!` for the accompanying
    // `impl UIElement for #target` that makes `#target` itself usable as a tree node. Template
    // composition (`is_template_composition`) is the same idea one level up: `base`'s type is the
    // immediate DSL base's own struct (not an `elwindui_core::ui` type), built by calling that
    // base's own `create_<snake case>(..)` factory directly rather than constructing anything itself.
    if is_template_composition {
        let base_name = component.base.as_deref().expect("is_template_composition implies a base");
        // `base_name` (bare) is itself a composed component, so it's a real *trait* now, not a
        // struct (see `struct_ident`'s doc comment) — the field's concrete type must be its `Impl`
        // struct, exactly like `concrete_type_ident` resolves for any other reference to it.
        let base_ty = concrete_type_ident(base_name, table.resolve(from, base_name));
        let base_create_fn = composed_create_fn_ident(base_name);
        struct_fields.extend(quote! { base: #base_ty, });
        field_inits.extend(quote! { base: #base_create_fn(#(#forward_param_names),*), });
    } else if is_shape_composition {
        let base_impl_ty = if is_virtual_builtin(&view.root.type_path) {
            shape_composition_base_types(&view.root.type_path).0
        } else {
            let ident = concrete_type_ident(&view.root.type_path, table.resolve(from, &view.root.type_path));
            quote! { #ident }
        };
        struct_fields.extend(quote! { base: #base_impl_ty, });
        field_inits.extend(quote! { base: #root_binding, });
    } else if root_is_virtual_builtin {
        struct_fields.extend(quote! {
            #root_binding: std::rc::Rc<dyn elwindui_core::ui::UIElement>,
        });
        field_inits.extend(quote! {
            #root_binding: #root_binding.clone(),
        });
    }

    // `show` only exists on the `Window` builtin — a component whose view root is something else
    // (e.g. `DocumentView`'s `VerticalLayout`) has no top-level window to show, only a root value
    // to be embedded by whatever contains it (a `TabView`'s `render_content`, a `Window`'s
    // `content`, a plain `VerticalLayout`/`Rectangle` child, etc.).
    //
    // Deliberately non-blocking (unlike the old `open`/`show_and_run`): entering the platform
    // event loop is `elwindui::application::run()`'s job, called once after every top-level
    // window has been shown — see `elwindui-backend-appkit`'s `application` module.
    //
    // Whether this component itself ends up "native" or "virtual" (from the *caller's*
    // perspective — see `into_node_if_needed`) is inherited from its own view root, computed the
    // same way `build_symbol_table`'s `resolve_is_native` does. A native root (including `Window`,
    // handled separately above) keeps the original `into_any_view` — a *local* inherent method
    // (not a `From`/`Into` impl: `impl From<Rc<#target>> for AnyView` would be rejected by Rust's
    // orphan rules, since `Rc` isn't "fundamental" and so `#target` nested inside it counts as
    // covered by a foreign generic — E0117). A virtual root gets `into_node` instead, returning
    // `Rc<dyn elwindui_core::ui::UIElement>` — either the hardcoded-builtin case handled just
    // above (a plain clone of the stored `Rc`), or (a user-defined component whose own root is
    // itself virtual — chained `inherits`) delegating to *that* root's own `into_node`/
    // `into_any_view` via `into_node_if_needed`, exactly like any other embedding site.
    let root_is_native = !is_template_composition && table.resolve(from, &view.root.type_path).is_some_and(|info| info.is_native);
    let root_embed_method = if is_template_composition || is_shape_composition {
        // `#target` implements `UIElement` itself now (see this function's tail `quote!`), so
        // `self` — not a separately-stored root field — already *is* the tree node; `Rc<Self>`
        // unsizes to `Rc<dyn UIElement>` directly.
        quote! {
            pub fn into_node(self: std::rc::Rc<Self>) -> std::rc::Rc<dyn elwindui_core::ui::UIElement> {
                self
            }
        }
    } else if is_host_composition {
        // Host composition (`is_host_composition`'s doc comment): `self.base` — not a separately
        // auto-named field — holds the native host instance now, so its own `show()` is reached
        // through that fixed name (mirroring `is_shape_composition`'s `self.base`-through-`base`
        // convention just above, minus the `UIElement` delegation Window doesn't participate in).
        quote! {
            pub fn show(self: std::rc::Rc<Self>) {
                self.base.clone().show();
            }
        }
    } else if view.root.type_path == "Window" {
        // A `Window`-rooted view with no `inherits Window` declaration (`is_host_composition` is
        // `false`) — `inherits` is opt-in for the stricter `base`-named/renamed host-composition
        // treatment above; a plain `Window` root still gets `show()` the original, structural way
        // (independent of `inherits`), through its ordinary auto-numbered stored field.
        quote! {
            pub fn show(self: std::rc::Rc<Self>) {
                self.#root_binding.clone().show();
            }
        }
    } else if root_is_native {
        let root_expr = into_any_view_if_needed(quote! { self.#root_binding }, "AnyView");
        quote! {
            pub fn into_any_view(self: std::rc::Rc<Self>) -> elwindui_backend_appkit::AnyView {
                #root_expr
            }
        }
    } else if root_is_virtual_builtin {
        quote! {
            pub fn into_node(self: std::rc::Rc<Self>) -> std::rc::Rc<dyn elwindui_core::ui::UIElement> {
                self.#root_binding.clone()
            }
        }
    } else {
        let root_expr = into_node_if_needed(quote! { self.#root_binding }, &view.root.type_path, from, table);
        quote! {
            pub fn into_node(self: std::rc::Rc<Self>) -> std::rc::Rc<dyn elwindui_core::ui::UIElement> {
                #root_expr
            }
        }
    };

    // For each `bind!` owner found above: subscribe so this component's `resync()` re-fires
    // whenever that viewmodel changes through *any* path (a sibling component's callback, an async
    // command, ...), not just this component's own wired `on_*` closures (`emit_wiring` only
    // reaches the latter). `Weak` avoids a retain cycle — this component already holds a strong
    // `Rc` to the owner via its own `#[param]` field, so the subscription closure must not hold a
    // strong `Rc` back to `this` or the pair would never be dropped.
    let subscribe_stmts: TokenStream = bind_owners
        .iter()
        .map(|owner_ident| {
            quote! {
                {
                    let weak = std::rc::Rc::downgrade(&this);
                    this.#owner_ident.subscribe(move || {
                        if let Some(this) = weak.upgrade() { this.resync(); }
                    });
                }
            }
        })
        .collect();

    // §3/付録I.1's lifecycle hooks. `on_mount` is spliced directly into `new()` (against the local
    // `this: Rc<Self>`, the same receiver `base::on_mount()` rewrites to — see below); `on_unmount`
    // is codegen'd as a real (if presently uncalled) `__run_on_unmount` method — `elwindui_core::ui`
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
    let on_mount_stmt = view.on_mount.as_ref().map(|block| rewrite_base_calls(block.clone(), &this_ident));

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

    // Meaningful whenever `#target` is composed at all (`is_shape_composition`/
    // `is_template_composition`, docs/elwindui_spec.md 付録H.2.1a): wires every child of `#target`'s
    // own `base` (delegated through `UIElement::children`, regardless of how many composition levels
    // deep `base` itself goes) to point its `parent` back at `this` — the same job
    // `elwindui_core::ui::new_element` does for every other construction site, reproduced here
    // because `this` (not `self.base` alone) is the real tree node now, and `new_element` is never
    // called on `self.base` in isolation (built as a plain, unwrapped value instead — see
    // `construct_stmts`/`field_inits` above).
    let parent_wiring_stmt = if is_shape_composition || is_template_composition {
        quote! {
            {
                use elwindui_core::ui::UIElement as _;
                let __erased: std::rc::Rc<dyn elwindui_core::ui::UIElement> = this.clone();
                for child in this.visual_children() {
                    *child.base().parent.borrow_mut() = Some(std::rc::Rc::downgrade(&__erased));
                }
            }
        }
    } else {
        TokenStream::new()
    };

    // `#struct_ident`'s own composition impls (docs/elwindui_spec.md 付録H.2.1a): the bare-named
    // `pub trait X: UIElement + <base's own trait>` (now free to exist — the real struct moved to
    // `XImpl` above, so there's no `struct X`/`trait X` namespace clash), `UIElement` itself
    // (delegating every method to `self.base`), and the ultimately-composed-over shape's own trait
    // (`Control`'s real accessors, or an empty marker for `Shape`/`Grid`/`Stack`/`TextBlock`) —
    // `composed_shape` already resolved this transitively, so `LabeledPanel` (composing over
    // `ContentControl`, which itself composes over `Control`) implements `Control` exactly like
    // `ContentControl` does, one delegation hop further out.
    let composition_impls = if let Some(shape) = &composed_shape {
        let (_, base_trait_path) = shape_composition_base_types(shape);
        let base_trait_impl = shape_composition_base_trait_impl(shape, &struct_ident);
        quote! {
            pub trait #target: elwindui_core::ui::UIElement + #base_trait_path {
                #trait_sigs
            }
            impl #target for #struct_ident {
                #trait_impl_bodies
            }

            impl elwindui_core::ui::UIElement for #struct_ident {
                fn base(&self) -> &elwindui_core::ui::UIElementImpl {
                    elwindui_core::ui::UIElement::base(&self.base)
                }
                fn visual_children(&self) -> Vec<std::rc::Rc<dyn elwindui_core::ui::UIElement>> {
                    elwindui_core::ui::UIElement::visual_children(&self.base)
                }
                fn measure_override(&self, available: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> elwindui_core::layout::Size {
                    elwindui_core::ui::UIElement::measure_override(&self.base, available, child_sizes)
                }
                fn arrange_override(&self, final_size: elwindui_core::layout::Size, child_sizes: &[elwindui_core::layout::Size]) -> Vec<elwindui_core::layout::Rect> {
                    elwindui_core::ui::UIElement::arrange_override(&self.base, final_size, child_sizes)
                }
                fn paint(&self) -> Option<elwindui_core::ui::PaintKind> {
                    elwindui_core::ui::UIElement::paint(&self.base)
                }
            }
            #base_trait_impl
        }
    } else if let Some(base_name) = &host_composition_base {
        // Host composition (`is_host_composition`'s doc comment): `base_name` (e.g. `Window`) is a
        // real trait now (per `TypeInfo::is_host_composition_base`'s paired `{Base}Impl` rename in
        // the base's own hand-written crate), so `#target: #base_trait` is a genuine supertrait
        // bound, exactly like the shape-composition case above — just no `impl UIElement`, since the
        // base doesn't implement it either (`#struct_ident`'s own `show()` stays an inherent method
        // on `impl #struct_ident` below, reached through `self.base`, not a trait method).
        let base_trait = format_ident!("{}", base_name);
        quote! {
            pub trait #target: #base_trait {
                #trait_sigs
            }
            impl #target for #struct_ident {
                #trait_impl_bodies
            }
            impl #base_trait for #struct_ident {}
        }
    } else {
        TokenStream::new()
    };

    // Whenever `#struct_ident` is composed (`is_shape_composition`/`is_template_composition`), its
    // plain (not-yet-`Rc`-wrapped) construction is split out into its own callable `pub fn
    // create_<snake case>(..)` — mirroring `elwindui_core::ui`'s `create_control`/`create_shape`/
    // etc. — instead of being inlined into `new()` only, so a *further* derived component's own
    // `is_template_composition` case (`LabeledPanel inherits ContentControl`) can call `#struct_ident`'s
    // own factory directly to build its `base` field (see this function's `field_inits` branch
    // above). An ordinary (non-composed) component keeps building `Self {..}` inline in `new()`,
    // unchanged — nothing else ever needs its bare, unwrapped value.
    let create_fn_ident = composed_create_fn_ident(&target_name);
    let create_fn = if is_composed {
        quote! {
            pub fn #create_fn_ident(#(#ctor_param_names: #ctor_param_types),*) -> #struct_ident {
                #construct_stmts
                #struct_ident { #(#required_own_names,)* #deferred_field_inits #field_inits }
            }
        }
    } else {
        TokenStream::new()
    };
    let new_construct_stmt = if is_composed {
        quote! { let this = std::rc::Rc::new(#create_fn_ident(#(#ctor_param_names),*)); }
    } else {
        quote! {
            #construct_stmts
            let this = std::rc::Rc::new(Self { #(#required_own_names,)* #deferred_field_inits #field_inits });
        }
    };

    quote! {
        #create_fn

        impl #struct_ident {
            pub fn new(#(#ctor_param_names: #ctor_param_types),*) -> std::rc::Rc<Self> {
                #new_construct_stmt
                #parent_wiring_stmt
                #wiring_stmts
                // Most widgets already read live model state at construction time, so this is a
                // no-op for them. A widget whose own state only ever appears in `resync()` (e.g. a
                // dynamic list, like `TabView`'s tabs) needs this call so state populated before
                // construction (as `main.rs` does, calling `new_tab_execute()` first) appears
                // immediately rather than waiting for the first unrelated user interaction.
                this.resync();
                #subscribe_stmts
                #on_mount_stmt
                this
            }

            fn resync(&self) {
                #resync_stmts
            }

            #root_embed_method

            #named_accessors
            #methods
            #shadow_hooks
            #on_unmount_method
        }

        pub struct #struct_ident {
            #(#required_own_names: #required_own_types,)*
            #deferred_own_field_decls
            #struct_fields
        }

        #composition_impls
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
}

impl ViewCtx {
    fn with_closure_param(&self, param: &str) -> ViewCtx {
        ViewCtx {
            binds: self.binds.clone(),
            closure_param: Some(param.to_string()),
            own_fields: self.own_fields.clone(),
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
    /// someone else's argument) whether it's already an `elwindui_core::ui::Node<AnyView>` value
    /// (a virtual builtin/component) or a real native handle needing `Node::Native(..)`/
    /// `.into_any_view()` wrapping — see `into_node_if_needed`/`into_any_view_if_needed`.
    child_bindings: Vec<(syn::Ident, String)>,
    /// `Owner::field: value` attached-property setters written directly on this element (§3) —
    /// copied verbatim from `ElementNode::attached`. Consulted only when constructing this node's
    /// own `UIElementBase` (see `grid_cell_expr`); a node with none gets `GridCell::default()`.
    attached: Vec<(String, String, ViewExpr)>,
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
            ChildEntry::Literal(elem) => child_bindings.push(plan_element(elem, ctx, from, table, out, false, lets)),
            ChildEntry::Ref(name) => {
                let resolved = lets
                    .get(name)
                    .unwrap_or_else(|| panic!("`{name}` does not refer to an earlier `let` binding in this view"));
                child_bindings.push(resolved.clone());
            }
        }
    }

    let mut element_attr_bindings = HashMap::new();
    for (name, expr) in &node.attributes {
        if let ViewExpr::Element(elem) = expr {
            element_attr_bindings.insert(name.clone(), plan_element(elem, ctx, from, table, out, false, lets));
        }
    }

    let attributes = desugar_command_attr(&node.type_path, node.attributes.clone(), from, table);
    let binding = format_ident!("__{}_{}", node.type_path.to_lowercase(), out.len());
    // A hand-written virtual builtin (`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`)
    // never gets a struct field: its `Node::Virtual` value is built once
    // inline (see `emit_construction`) and immediately moved into whichever `Vec<Node<H>>` it's a
    // child of, or (if it's the view's root) into a one-shot `RefCell` — see `generate_view`. It
    // has no native setter to wire `on_*`/resync against, so it must never be `stored` regardless
    // of `is_root`/its own attributes (which `emit_wiring`/`emit_resync` already skip via their
    // `if !node.stored { return; }` guard — no changes needed there).
    let stored = !is_virtual_builtin(&node.type_path) && (is_root || !attributes.is_empty());

    out.push(PlannedNode {
        binding: binding.clone(),
        type_path: node.type_path.clone(),
        attributes,
        attached: node.attached.clone(),
        child_bindings,
        element_attr_bindings,
        stored,
        id: None,
    });
    (binding, node.type_path.clone())
}

/// `command: <path>` sugar (docs/elwindui_spec.md 付録O.4), WinUI3's `Button.Command`-style
/// convenience: expands to the equivalent `<sole on_* field>: <path>.execute()` (+
/// `enabled: <path>.can_execute` if the shape also declares an `enabled` field) — exactly what
/// writing `on_click: vm.save.execute()` + `enabled: vm.save.can_execute` by hand already
/// generates. `command` never becomes a real `#[param]` passed to `Type::new(..)` — there's no
/// single shared `Command` Rust type to pass (付録O.5 monomorphizes each viewmodel's `Command`
/// field into its own `<field>_execute`/`<field>_can_execute` methods, never a materialized
/// `Command` value) — this is purely an attribute-level rewrite, run once during planning.
///
/// Driven entirely by the resolved shape's own declared fields (which single field name starts
/// with `on_`), not hardcoded per widget name — so this works identically for a hand-written
/// builtin (`Button`/`MenuItem`, which declare their `on_click`/`on_select` field the same way
/// `TabView` already declares `on_select`/`on_close`) or any user-defined component (native or
/// virtual) with exactly one `on_*` event of its own. A shape with zero or more than one `on_*`
/// field has no unambiguous trigger, so `command` is left untouched (inert — `emit_construction`
/// ignores attribute names with no matching declared field, same as any other unrecognized
/// attribute). Explicit `on_*`/`enabled` attributes on the same element always win — this only
/// fills in ones the caller didn't already set, so `command` and the older two-attribute style can
/// be freely mixed on the same element.
fn desugar_command_attr(
    type_path: &str,
    attributes: Vec<(String, ViewExpr)>,
    from: &Module,
    table: &SymbolTable,
) -> Vec<(String, ViewExpr)> {
    let Some(ViewExpr::Path(command_path)) = attributes.iter().find(|(name, _)| name == "command").map(|(_, v)| v) else {
        return attributes;
    };
    let Some(info) = table.resolve(from, type_path) else {
        return attributes;
    };
    let on_fields: Vec<&String> = info.fields.keys().filter(|name| name.starts_with("on_")).collect();
    let [trigger] = on_fields.as_slice() else {
        return attributes;
    };
    let trigger = (*trigger).clone();
    let command_path = command_path.clone();
    let has_enabled_field = info.field_types.contains_key("enabled");

    // `command` isn't itself a declared field on any target (see this function's doc comment) —
    // left in place, `emit_resync`'s generic "call `set_<attr>` for every non-callback attribute"
    // loop would try (and fail to find) a `set_command` method, so it must be removed once
    // desugared, not just left inert.
    let mut result: Vec<(String, ViewExpr)> = attributes.into_iter().filter(|(name, _)| name != "command").collect();
    if !result.iter().any(|(name, _)| *name == trigger) {
        result.push((trigger, ViewExpr::MethodCall(command_path.clone(), "execute".to_string())));
    }
    if has_enabled_field && !result.iter().any(|(name, _)| name == "enabled") {
        let mut can_execute_path = command_path;
        can_execute_path.push("can_execute".to_string());
        result.push(("enabled".to_string(), ViewExpr::Path(can_execute_path)));
    }
    result
}

fn find_attr<'a>(node: &'a PlannedNode, name: &str) -> Option<&'a ViewExpr> {
    node.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// Builds `node`'s own `elwindui_core::layout::GridCell { row: ..., column: ... }` from whatever
/// `Owner::row`/`Owner::column`-shaped attached setters it has (§3) — any field not set falls back
/// to `0`, matching `GridCell::default()` (and `builtin::Grid`'s own declared attached-field
/// defaults, which is why no evaluation of those defaults is ever needed here).
///
/// Deliberately keyed by field name alone (`row`/`column`), not by `(owner, field)` — this crate
/// has exactly one attached-property owner today (`Grid`), and `GridCell` is a concrete, hand-
/// curated struct (not a type-erased per-owner bag, matching `UIElementBase`'s existing evolution
/// pattern — see its own `grid_cell` field's doc comment). A future second attached-property owner
/// reusing the `row`/`column` names would collide here; a genuinely distinct future property would
/// get its own field name and its own branch, the same way `row`/`column` were added.
///
/// Scope note: only ever called from `emit_virtual_construction`, so this only takes effect for a
/// `Grid` child that is itself one of codegen's hardcoded virtual builtins (`TextBlock`/
/// `Rectangle`/`Ellipse`/`Stack`/`Control`/a nested `Grid`) — verified end-to-end by launching the
/// notepad example with a temporary `Grid` in its status bar (Fixed/Star/Fixed columns rendered
/// with correct proportional widths). A native-leaf child (`Button`/`TextArea`/..., wrapped via
/// `into_node_if_needed`) or a user-defined `component`+`view` child (e.g. `RoundedPanel`, which
/// builds its own root's `UIElementBase` inside its own generated `new()`) doesn't have this
/// specific child's `attached` list threaded to wherever *its* `UIElementBase` gets built, without
/// further plumbing; such a child's `Grid::row`/`Grid::column` setters still validate successfully
/// but are inert (default `(0, 0)`) until that's added.
fn grid_cell_expr(node: &PlannedNode, ctx: &ViewCtx, mode: &EmitMode) -> TokenStream {
    if node.attached.is_empty() {
        return quote! { elwindui_core::layout::GridCell::default() };
    }
    let mut row = quote! { 0 };
    let mut column = quote! { 0 };
    for (_owner, field, value) in &node.attached {
        let value_ts = emit_expr(value, ctx, mode);
        match field.as_str() {
            "row" => row = value_ts,
            "column" => column = value_ts,
            _ => {}
        }
    }
    quote! { elwindui_core::layout::GridCell { row: #row, column: #column } }
}

/// `Option<Foo>` -> `("Foo", true)`; anything else -> `(ty, false)` unchanged.
pub(crate) fn strip_option(ty: &str) -> (&str, bool) {
    let trimmed = ty.trim();
    match trimmed.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        Some(inner) => (inner.trim(), true),
        None => (trimmed, false),
    }
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

/// Whether `type_path` names one of the hand-written *virtual* builtins (`VerticalLayout`/
/// `HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock`/`Control`/`Grid`) — these have no backend
/// Rust struct or `Type::new(args)` constructor at all; `emit_construction` builds a `Box<dyn
/// elwindui_core::ui::UIElement>` value for them directly (see its top-of-function check).
/// `ContentControl` (docs/elwindui_builtins_spec.md 付録F.10) is deliberately *not* here — it's an
/// ordinary `component`+`view` pair whose view root literally constructs `Control` (the
/// `inherits`-based shape-composition pattern, docs/elwindui_spec.md §3, same as `RoundedPanel
/// inherits Rectangle`), so it goes through the normal `generate_view`/`emit_construction` path and
/// gets a real generated struct with real accessors.
/// See docs/elwindui_spec.md 付録H.2. `pub(crate)`: `validate.rs`'s `validate_inherits` also
/// consults this, to exempt a virtual builtin's own `inherits` declaration from the shape-
/// composition "view root must construct Base" check — see that function's doc comment.
pub(crate) fn is_virtual_builtin(type_path: &str) -> bool {
    matches!(type_path, "VerticalLayout" | "HorizontalLayout" | "TextBlock" | "Control" | "Grid" | "Shape")
}

/// Sentinel `source_type_path` passed to `into_node_if_needed` for a value that is *already* an
/// `Rc<dyn UIElement>` with no associated component type name to resolve (a `#[param]` field of
/// that type, forwarded as a bare child in the component's own `view` — e.g. `ContentControl`'s
/// `content` forwarded into `Control { content }`). `into_node_if_needed` treats it as an
/// unconditional pass-through instead of trying (and failing) to resolve it via `SymbolTable`.
const PASSTHROUGH_NODE: &str = "__passthrough_node__";

/// Converts a constructed child binding into `Rc<dyn elwindui_core::ui::UIElement>` for a slot
/// that wants one (`Window`'s `content`, `TabView`'s `item_template` return, or a virtual
/// builtin's own `children: Vec<Rc<dyn UIElement>>` — anywhere the declared type mentions `dyn
/// UIElement`, checked by the caller before calling this). Four cases, by `source_type_path`'s
/// resolved `is_native`/`is_native_control_leaf`:
/// - A hand-written virtual builtin (`is_virtual_builtin`, always `!is_native`): `base` is
///   *already* an `Rc<dyn UIElement>` local value (built by `emit_construction`'s virtual branch,
///   via `elwindui_core::ui::new_element`) — used as-is.
/// - A user-defined component whose own `view` root is virtual (`!is_native`, e.g. `DocumentView`,
///   whose root is `VerticalLayout`): its generated `into_node(self: Rc<Self>)` (see
///   `generate_view`) produces the `Rc<dyn UIElement>` value — same `.clone()` convention as
///   `into_any_view_if_needed` so the original binding stays valid for any later reference.
/// - `Button`/`TextArea`/`TabView` (`TypeInfo::is_native_control_leaf`): already implements
///   `UIElement` directly — its own `base: elwindui_core::ui::NativeControlImpl<H>` was already
///   built at construction time from *this exact use site*'s margin/alignment/data_context/
///   `routed_handlers` (see `emit_construction`'s `build_ui_element_base` argument) — so this is a
///   plain upcast, no fresh wrapper needed.
/// - Anything else native (`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`Window`, or a user component
///   whose own root is native): wrapped as a `NativeControl` via `new_element`
///   (`UIElementBase::default()` — no way to set `margin`/`alignment` on an embedding site's own
///   wrapper here — except `routed_handlers`, which is shared from the widget's own storage when it
///   has any `#[routed]` fields), reusing `into_any_view_if_needed` for the inner handle conversion.
///   In practice this branch is never actually reached for `MenuBar`-family types today (none of
///   their own child slots are `dyn UIElement`-typed), kept only for a hypothetical user component
///   whose own root resolves native without being one of the three real `is_native_control_leaf`
///   leaves above.
fn into_node_if_needed(base: TokenStream, source_type_path: &str, from: &Module, table: &SymbolTable) -> TokenStream {
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
                let __node: std::rc::Rc<dyn elwindui_core::ui::UIElement> = #base.clone();
                __node
            }
        }
    } else if is_native {
        // A `#[routed]` field (docs/elwindui_spec.md 4章) registers its handler on the widget's
        // *own* `routed_handlers()` at its own construction time (see `emit_wiring`) — long before
        // this `NativeControl` wrapper exists (tree construction is bottom-up: children first).
        // Sharing that same `Rc` here (instead of `UIElementBase::default()`'s fresh, empty one)
        // is what makes `dispatch_routed` find it later when bubbling from this node.
        let has_routed = info.is_some_and(|i| !i.routed_fields.is_empty());
        let base_expr = base.clone();
        let view = into_any_view_if_needed(base, "AnyView");
        let ui_base = if has_routed {
            quote! {
                elwindui_core::ui::UIElementImpl {
                    routed_handlers: (#base_expr).routed_handlers(),
                    ..elwindui_core::ui::UIElementImpl::default()
                }
            }
        } else {
            quote! { elwindui_core::ui::UIElementImpl::default() }
        };
        quote! {
            elwindui_core::ui::new_element(elwindui_core::ui::create_native_control(#ui_base, #view))
        }
    } else if is_virtual_builtin(source_type_path) {
        quote! { #base }
    } else {
        quote! { #base.clone().into_node() }
    }
}

/// `|param| <body>` -> `Box::new(move |param| { <body> })` — a real, ordinary Rust closure value,
/// usable as any `Box<dyn Fn(..) -> ..>`-typed constructor argument (`TabView`'s `key`/
/// `render_label`/`render_content`, or any future widget with a per-item callback param). The
/// closure's own parameter needs no type annotation — it's inferred from the constructor
/// parameter's declared `Box<dyn Fn(&Rc<T>) -> R>` type at the call site.
fn emit_closure_value(param: &str, body: &ClosureBody, ctx: &ViewCtx, from: &Module, table: &SymbolTable) -> TokenStream {
    let param_ident = format_ident!("{}", param);
    let closure_ctx = ctx.with_closure_param(param);
    let body_expr = match body {
        ClosureBody::Expr(expr) => emit_expr(expr, &closure_ctx, &EmitMode::Construction),
        ClosureBody::Element(elem) => {
            let mut plan = Vec::new();
            // No outer `let`-bound names are visible inside a template closure body — it runs in a
            // separate per-item instantiation context, not the enclosing view's own construction.
            plan_element(elem, &closure_ctx, from, table, &mut plan, true, &HashMap::new());
            let mut construct = TokenStream::new();
            for planned in &plan {
                emit_construction(planned, &closure_ctx, from, table, &mut construct);
            }
            let root = plan.last().expect("closure element body must have a root");
            // `item_template`'s declared return type is `Rc<dyn UIElement>` (`TabView` in
            // `builtins.elwind`), not a bare `AnyView` — so a per-tab body rooted in a virtual
            // builtin/component (a `VerticalLayout`, or a `DocumentView`-style user component)
            // works exactly like any other embedding slot, via the same `is_native` dispatch
            // `into_node_if_needed` uses elsewhere.
            let root_binding = &root.binding;
            let converted = into_node_if_needed(quote! { #root_binding }, &root.type_path, from, table);
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

/// A hand-written native's own DSL-attribute-driven setters (`build_component_setters`) may call
/// one of `elwindui_core::ui`'s shared property-setter traits' methods via dot-syntax — declared
/// there (docs/elwindui_spec.md 付録H.2.1a) rather than as a wrapper-only inherent method, so the
/// trait needs to be in scope wherever that dot-call happens. Emitted as an anonymous `use ... as
/// _;` (never binds a name of its own, so repeating it for multiple bindings of the same type in
/// one function is harmless) right alongside `#binding`'s own `let` in `emit_construction`, which
/// keeps it in scope for `emit_wiring`'s later calls on the same binding too (both live in the same
/// enclosing function body). Only `Button`/`TextArea`/`MenuItem`/`MenuBarItem` actually route any
/// of their own DSL properties through a shared trait method this way — `Window`/`TabView`/
/// `TabViewItem`'s own properties, and `Menu`/`MenuBar`'s `children`, are all wrapper-only inherent
/// methods (no shared trait involved), so nothing needs importing for those.
fn hand_written_native_trait_use(type_path: &str) -> TokenStream {
    match type_path {
        "Button" => quote! { use elwindui_core::ui::Button as _; },
        "TextArea" => quote! { use elwindui_core::ui::TextArea as _; },
        "MenuItem" => quote! { use elwindui_core::ui::MenuItem as _; },
        "MenuBarItem" => quote! { use elwindui_core::ui::MenuBarItem as _; },
        _ => TokenStream::new(),
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
fn emit_construction(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, out: &mut TokenStream) {
    if is_virtual_builtin(&node.type_path) {
        emit_virtual_construction(node, ctx, from, table, out);
        return;
    }

    let binding = &node.binding;
    let info = table.resolve(from, &node.type_path).unwrap_or_else(|| {
        panic!("unknown or out-of-scope element `{}` — is a `use` for it missing?", node.type_path)
    });
    let type_ident = concrete_type_ident(&node.type_path, Some(info));

    if is_hand_written_native(info) {
        let setters = build_component_setters(node, ctx, from, table, info);
        let trait_use = hand_written_native_trait_use(&node.type_path);
        out.extend(quote! {
            #trait_use
            let #binding = #type_ident::new();
            #(#setters)*
        });
    } else {
        // `has_view`/plain-component construction (docs/elwindui_spec.md 付録H.2.1a's
        // post-construction setter convention, Phase 2): `build_component_args` already omits this
        // target's own deferred `Option<T>` fields (`is_deferred_field`) from the positional list —
        // `build_component_optional_setters` supplies the matching trailing `.set_<field>(value)`
        // calls for whichever of them this use site actually gives a value.
        let args = build_component_args(node, ctx, from, table, info);
        let optional_setters = build_component_optional_setters(node, ctx, from, table, info);
        out.extend(quote! {
            let #binding = #type_ident::new(#(#args),*);
            #(#optional_setters)*
        });
    }
    // `Button`/`TextArea`/`TabView` (`inherits NativeControl`, `TypeInfo::is_native_control_leaf`)
    // own a real `base: elwindui_core::ui::NativeControlImpl<H>` field (docs/elwindui_spec.md
    // 付録H.2.1a) — this use site's margin/data_context/grid_cell are applied to it right here,
    // post-construction, exactly like `emit_virtual_construction` does for virtual builtins (see
    // `emit_common_ui_element_setters`). `MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`Window`
    // (`#[native]` directly, never entering the `UIElement` tree) don't get this at all.
    if info.is_native_control_leaf {
        let binding_ts = quote! { #binding };
        out.extend(emit_common_ui_element_setters(node, ctx, &binding_ts));
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
/// that family defers *every* field unconditionally via the separate, older
/// `build_component_setters` path, not this one.
fn is_deferred_field(info: &TypeInfo, name: &str, ty: &str) -> bool {
    if is_hand_written_native(info) || !strip_option(ty).1 {
        return false;
    }
    match &info.effective_view {
        Some(view) => !view_references_name_anywhere(view, name),
        None => true,
    }
}

/// Evaluates a resolved user-component node's own attributes into the positional argument list its
/// generated `new(..)`/`create_<snake case>(..)` (docs/elwindui_spec.md 付録H.2.1a) expects, in
/// `info.param_fields`'s declared order — shared by `emit_construction` (wraps as `Type::new(args)`)
/// and `build_component_value` (wraps as `create_<snake case>(args)`, for a shape-composition root
/// whose base is itself a DSL component rather than a hand-written `elwindui_core::ui` primitive).
/// Skips a deferred field (`is_deferred_field`) entirely — no positional slot at all, not even a
/// placeholder `None` — since that target's own `new(..)` no longer declares one; the matching
/// value (if this use site supplies one) is applied afterward instead, via
/// `build_component_optional_setters`.
fn build_component_args(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, info: &TypeInfo) -> Vec<TokenStream> {
    // A bare nested child element (no `name:` attribute) only ever has somewhere to go if this
    // component declares a `children`-named param (a list, consumed in full below) or a
    // `#[content(field_name)]` (a single slot, consumed further down) — anything else, with no
    // declared destination at all, is a codegen-time authoring mistake, not a silently-guessed
    // field-order fallback like this used to be.
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
        if is_deferred_field(info, name, ty) {
            continue;
        }
        if name == "children" {
            let wants_node = ty.contains("dyn UIElement");
            let items = node.child_bindings.iter().map(|(c, child_ty)| {
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
            Some(ViewExpr::Closure { param, body }) => emit_closure_value(param, body, ctx, from, table),
            Some(other) => {
                let value = emit_expr(other, ctx, &EmitMode::Construction);
                // A `String`-shaped param takes `&str` in every *hand-written* builtin (matching
                // the shape declaration's `String`/`Option<String>` — see this crate's own
                // `src/builtins.elwind`), so the value is wrapped in `&(..)` here regardless of
                // whether the DSL expression itself is a `&str` literal or a computed `String`
                // (e.g. `t!(...)`) — Rust's deref coercion accepts either as `&str` at the call
                // site, the same trick the old hardcoded `emit_construction` arms already relied on
                // for every builtin's string params. A `view`-having (`info.has_view`) component's
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
                } else {
                    value
                }
            }
            None if is_option => {
                args.push(quote! { None });
                continue;
            }
            None if info.content_field.as_deref() == Some(name.as_str()) && !node.child_bindings.is_empty() => {
                if node.child_bindings.len() > 1 {
                    panic!(
                        "`{}`'s `#[content({name})]` field can only bind a single nested child element, found {}",
                        node.type_path,
                        node.child_bindings.len()
                    );
                }
                let (child, child_ty) = &node.child_bindings[0];
                if inner_ty.contains("dyn UIElement") {
                    into_node_if_needed(quote! { #child }, child_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #child }, inner_ty)
                }
            }
            None => panic!("`{}` requires attribute `{name}`", node.type_path),
        };
        args.push(if is_option { quote! { Some(#value) } } else { value });
    }
    args
}

/// The post-construction-setter analog of `build_component_args` — used by `emit_construction`'s
/// `is_hand_written_native` branch instead of positional constructor args (docs/elwindui_spec.md
/// 付録H.2.1a's post-construction setter convention, extended to every builtin's own declared
/// `#[param]`s, the same way `emit_common_ui_element_setters` already applies it to
/// margin/data_context/grid_cell). Mirrors `build_component_args`'s field-by-field value
/// computation exactly (same bare-children/`ViewExpr::Element`/`ViewExpr::Closure`/
/// `#[content(field_name)]` handling), except:
/// - an absent `Option<..>`-typed attribute emits **no call at all** (the zero-argument
///   constructor's own default already applies) rather than a placeholder `None`;
/// - an `Option<..>`-typed attribute that *is* present is passed to the setter **unwrapped**
///   (its inner type), never `Some(..)`-wrapped — matching `emit_resync`'s own pre-existing
///   convention for these same hand-written setters ("the resync value itself is never
///   `Option`-wrapped, only construction-time *positional* args were" — see that function's doc
///   comment). Every hand-written builtin setter (`Button::set_enabled`, `MenuItem::set_shortcut`,
///   `TabView::set_items_source`, ...) already exists to serve `emit_resync`'s calls with this bare
///   shape, so construction must call the very same setters the very same way, not the old
///   positional-constructor's own `Some(..)`-wrapping convention (that convention lives only in
///   `build_component_args`, still used for `has_view` components' *actual* constructor
///   parameters, an entirely different call site);
/// - a `String`-shaped param still takes `&str` at the hand-written setter (unlike
///   `build_component_args`'s `has_view`-conditional `.to_string()`, which never applies here
///   since `is_hand_written_native` implies `!info.has_view`);
/// - `TabView`'s `items_source`/`header_template`/`item_template` trio (all generic over the same
///   `T`) is combined into a single `set_dynamic_source(items, header_template, item_template)`
///   call instead of three independent ones — Rust can only unify a generic method call's type
///   parameter across *that one call*'s own arguments; `header_template`/`item_template`'s closure
///   bodies (`|doc| doc.file_name()`) carry no concrete type of their own to infer `T` from in
///   isolation, so they must share a call with `items_source` (whose *value*, e.g.
///   `vm.documents()`, is concretely `Vec<Rc<Document>>`) the same way the old single positional
///   constructor let all of `Type::new(..)`'s arguments unify one shared `T`. `set_items_source`
///   itself stays a separate, single-argument method (unaffected) — `emit_resync` already calls it
///   alone (its own value is concrete, no closure involved, so no inference problem there).
fn build_component_setters(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, info: &TypeInfo) -> Vec<TokenStream> {
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
    // See this function's own doc comment's `TabView` bullet — `header_template`/`item_template`
    // are consumed together with `items_source` (whichever of the three is encountered first in
    // `info.param_fields`' declared order), so encountering either of the other two afterward must
    // be a no-op rather than emitting its own (uninferrable) call.
    let mut dynamic_source_handled = false;
    for (name, ty) in &info.param_fields {
        if (name == "header_template" || name == "item_template") && dynamic_source_handled {
            continue;
        }
        if name == "items_source" && info.param_fields.iter().any(|(n, _)| n == "header_template") {
            let Some(items) = find_attr(node, "items_source") else { continue };
            let Some(ViewExpr::Closure { param: header_param, body: header_body }) = find_attr(node, "header_template") else { continue };
            let Some(ViewExpr::Closure { param: item_param, body: item_body }) = find_attr(node, "item_template") else { continue };
            let items = emit_expr(items, ctx, &EmitMode::Construction);
            let header_template = emit_closure_value(header_param, header_body, ctx, from, table);
            let item_template = emit_closure_value(item_param, item_body, ctx, from, table);
            setters.push(quote! { #binding.set_dynamic_source(#items, #header_template, #item_template); });
            dynamic_source_handled = true;
            continue;
        }

        let setter_ident = format_ident!("set_{}", name);
        if name == "children" {
            let wants_node = ty.contains("dyn UIElement");
            let items = node.child_bindings.iter().map(|(c, child_ty)| {
                if wants_node {
                    into_node_if_needed(quote! { #c }, child_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #c }, ty)
                }
            });
            setters.push(quote! { #binding.#setter_ident(vec![ #(#items),* ]); });
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
            Some(ViewExpr::Closure { param, body }) => emit_closure_value(param, body, ctx, from, table),
            Some(other) => {
                let value = emit_expr(other, ctx, &EmitMode::Construction);
                if inner_ty == "String" {
                    quote! { &(#value) }
                } else {
                    value
                }
            }
            None if is_option => continue,
            None if info.content_field.as_deref() == Some(name.as_str()) && !node.child_bindings.is_empty() => {
                if node.child_bindings.len() > 1 {
                    panic!(
                        "`{}`'s `#[content({name})]` field can only bind a single nested child element, found {}",
                        node.type_path,
                        node.child_bindings.len()
                    );
                }
                let (child, child_ty) = &node.child_bindings[0];
                if inner_ty.contains("dyn UIElement") {
                    into_node_if_needed(quote! { #child }, child_ty, from, table)
                } else {
                    into_any_view_if_needed(quote! { #child }, inner_ty)
                }
            }
            None => panic!("`{}` requires attribute `{name}`", node.type_path),
        };
        setters.push(quote! { #binding.#setter_ident(#value); });
    }
    setters
}

/// The Phase 2 counterpart of `build_component_setters` — trailing `.set_<field>(value)` calls for
/// a `has_view`/plain component's own *deferred* `Option<T>` fields (`is_deferred_field`), used
/// alongside `build_component_args`'s now-shrunk positional list (see `emit_construction`'s
/// non-`is_hand_written_native` branch). Only ever emits a call when this use site actually
/// supplies a value for the field — an absent one leaves that field's own
/// `RefCell::new(None)`/`Cell::new(None)` default in place (`generate_view`/`generate_component`'s
/// own field-splitting doc comment).
fn build_component_optional_setters(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, info: &TypeInfo) -> Vec<TokenStream> {
    let binding = &node.binding;
    let mut setters = Vec::new();
    for (name, ty) in &info.param_fields {
        if !is_deferred_field(info, name, ty) {
            continue;
        }
        let setter_ident = format_ident!("set_{}", name);
        // `is_deferred_field` only ever returns `true` for an `Option<..>`-typed field, so
        // `inner_ty` here is always the unwrapped inner type.
        let (inner_ty, _) = strip_option(ty);
        let value = match find_attr(node, name) {
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
            Some(ViewExpr::Closure { param, body }) => emit_closure_value(param, body, ctx, from, table),
            Some(other) => {
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
            None => continue,
        };
        setters.push(quote! { #binding.#setter_ident(#value); });
    }
    setters
}

/// Builds the plain (not yet `Rc`-wrapped) `create_<snake case>(args)` call for a shape-composition
/// root whose base is a resolved DSL component (rather than a hand-written `elwindui_core::ui`
/// primitive — see `build_virtual_value` for that case) — e.g. `RoundedPanel inherits ContentControl`,
/// whose own `view` root literally constructs `ContentControl`. Mirrors `emit_construction`'s
/// `Type::new(args)` shape exactly, just calling the base's own plain factory instead (see
/// `generate_view`'s `is_shape_composition` branch).
///
/// Known gap (Phase 2, docs/elwindui_spec.md 付録H.2.1a): `build_component_args` already omits any
/// of the base's own *deferred* `Option<T>` fields from this positional call (`is_deferred_field`),
/// but unlike `emit_construction` this single-expression call site has no room to append the
/// matching trailing `.set_<field>(value)` calls (`build_component_optional_setters`) afterward —
/// so a value supplied here for such a field is currently silently dropped. Doesn't affect any
/// `#[embedded]` base today (none has a deferred field); revisit if a future one does.
fn build_component_value(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable) -> TokenStream {
    let info = table.resolve(from, &node.type_path).unwrap_or_else(|| {
        panic!("unknown or out-of-scope element `{}` — is a `use` for it missing?", node.type_path)
    });
    let create_fn = composed_create_fn_ident(&node.type_path);
    let args = build_component_args(node, ctx, from, table, info);
    quote! { #create_fn(#(#args),*) }
}

/// Emits post-construction `binding.base().set_margin(..)`/`set_data_context(..)`/`set_grid_cell(..)`
/// calls (docs/elwindui_spec.md 付録H.2.1a) for whichever of these common attributes `node` actually
/// specifies — shared by `emit_virtual_construction` (virtual builtins) and `emit_construction`'s
/// native-control-leaf branch (`Button`/`TextArea`/`TabView` — see `TypeInfo::is_native_control_leaf`).
/// `UIElementImpl`'s fields are all interior-mutable (`Cell`/`RefCell`) precisely so this can run
/// *after* `Type::new(..)` returns rather than needing every `create_xxx`/hand-written builtin
/// constructor to accept a `base: UIElementImpl` argument — a use site left with none of these
/// attributes emits nothing at all, leaving `UIElementImpl::default()` in place. Deliberately does
/// *not* handle the generic "any element can catch a routed `on_click`" attribute — see
/// `emit_generic_on_click_routing`, a separate step for exactly that.
fn emit_common_ui_element_setters(node: &PlannedNode, ctx: &ViewCtx, binding: &TokenStream) -> TokenStream {
    // Whether `expr` is a bare 1-segment reference to one of *this* component's own `#[param]`
    // fields that's already `Option<..>`-typed (e.g. `ContentControl`'s own `padding: Option<f32>`
    // forwarded as `Control { padding: padding }`) — as opposed to a plain value (a literal, a
    // required field, a `vm.field`-shaped bind path, ...) that's already the setter's own plain
    // argument type as-is.
    let is_own_option_field = |expr: &ViewExpr| match expr {
        ViewExpr::Path(segments) => match segments.as_slice() {
            [only] => ctx.own_fields.get(only).is_some_and(|ty| ty.starts_with("Option<")),
            _ => false,
        },
        _ => false,
    };
    let mut out = TokenStream::new();
    // `margin` is settable today (the view-expression parser has numeric-literal support);
    // `horizontal_alignment`/`vertical_alignment` have no enum-variant-literal syntax yet, so they
    // stay at `UIElementImpl::default()`'s `Stretch` (matching every other element's default).
    if let Some(expr) = find_attr(node, "margin") {
        let value = if is_own_option_field(expr) {
            let inner = emit_expr(expr, ctx, &EmitMode::Construction);
            quote! { (#inner).unwrap_or(0.0) }
        } else {
            emit_expr(expr, ctx, &EmitMode::Construction)
        };
        out.extend(quote! { #binding.base().set_margin(#value); });
    }
    // `data_context` (付録Y) is likewise a common attribute, settable the same way `margin` is —
    // an omitted one leaves `UIElementImpl::default()`'s `None`. The supplied expression is
    // `Rc<dyn Any>`-erased here (matching every other cross-type-parameter value in this crate,
    // e.g. `elwindui-backend-appkit`'s `tab_view` module's `erase_items`/`erase_render`) so
    // `UIElementImpl` itself stays non-generic. The expression must already evaluate to an owned
    // `Rc<T>` (matching `items_source`'s own `Vec<Rc<T>>` requirement) — the cast relies on that,
    // it doesn't wrap in a fresh `Rc`.
    if let Some(expr) = find_attr(node, "data_context") {
        let value = emit_expr(expr, ctx, &EmitMode::Construction);
        out.extend(quote! { #binding.base().set_data_context(Some((#value) as std::rc::Rc<dyn std::any::Any>)); });
    }
    if !node.attached.is_empty() {
        let grid_cell = grid_cell_expr(node, ctx, &EmitMode::Construction);
        out.extend(quote! { #binding.base().set_grid_cell(#grid_cell); });
    }
    out
}

/// Emits `binding.base().register_routed_handler::<()>("on_click", ..)` for the generic "any
/// element can catch a routed `on_click`" common attribute (docs/elwindui_spec.md 4章) — used by
/// `emit_virtual_construction` unconditionally, and by `emit_construction`'s native-control-leaf
/// branch only when the type doesn't *already* declare `on_click` as a real `#[routed]` field of
/// its own (`Button` — wired instead by `emit_wiring`'s dedicated `is_routed` branch; applying this
/// generic mechanism too would register the same callback twice).
fn emit_generic_on_click_routing(node: &PlannedNode, ctx: &ViewCtx, binding: &TokenStream) -> TokenStream {
    match find_attr(node, "on_click") {
        Some(expr) => {
            let call = emit_expr(expr, ctx, &EmitMode::Construction);
            quote! {
                #binding.base().register_routed_handler::<()>("on_click", Box::new(move |_: &(), _args: &elwindui_core::input::RoutedEventArgs| { #call; }));
            }
        }
        None => quote! {},
    }
}

/// Builds an `Rc<dyn elwindui_core::ui::UIElement>` value (via `elwindui_core::ui::new_element`)
/// for a hand-written virtual builtin (`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/
/// `TextBlock`/`Control` — see `is_virtual_builtin`) directly from its own attributes, instead of
/// calling a (nonexistent) `Type::new(args)`.
fn emit_virtual_construction(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, out: &mut TokenStream) {
    let binding = &node.binding;
    let value = build_virtual_value(node, ctx, from, table);
    out.extend(quote! {
        let #binding: std::rc::Rc<dyn elwindui_core::ui::UIElement> = elwindui_core::ui::new_element(#value);
    });
    let binding_ts = quote! { #binding };
    out.extend(emit_common_ui_element_setters(node, ctx, &binding_ts));
    out.extend(emit_generic_on_click_routing(node, ctx, &binding_ts));
}

/// Builds the plain (not yet `new_element`-wrapped) `elwindui_core::ui::create_xxx()` (empty
/// argument — docs/elwindui_spec.md 付録H.2.1a's post-construction setter convention, extended to
/// every builtin property) followed by whichever `set_<field>(..)` calls this use site's own
/// attributes supply, as a single block expression evaluating to the fully-configured value — the
/// value `emit_virtual_construction` normally wraps immediately in `new_element(..)`, but which a
/// `component X inherits Y` shape-composition root (docs/elwindui_spec.md 付録H.2.1a) needs
/// unwrapped so it can be embedded directly as `X`'s own `base` field instead of erased into
/// `Rc<dyn UIElement>` (see `generate_view`'s `is_shape_composition` branch).
fn build_virtual_value(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable) -> TokenStream {
    // Same as `build_ui_element_base`'s own `get_attr`, but for `Option<String>` shapes
    // (`Rectangle`/`Ellipse`'s `fill`/`stroke`, `TextBlock`'s `color`) — the corresponding
    // `elwindui_core::ui` fields are owned `String`s (they're stored long-term in the scene
    // tree, not just for the duration of one call), but the DSL expression supplying them may be a
    // `&'static str` literal (`fill: "#3a3a3c"`) just as easily as an already-owned `String` (a
    // `t!(...)` result) — `.to_string()` accepts either uniformly.
    let is_own_option_field = |expr: &ViewExpr| match expr {
        ViewExpr::Path(segments) => match segments.as_slice() {
            [only] => ctx.own_fields.get(only).is_some_and(|ty| ty.starts_with("Option<")),
            _ => false,
        },
        _ => false,
    };
    let get_attr = |name: &str| -> TokenStream {
        match find_attr(node, name) {
            Some(expr) if is_own_option_field(expr) => emit_expr(expr, ctx, &EmitMode::Construction),
            Some(expr) => {
                let value = emit_expr(expr, ctx, &EmitMode::Construction);
                quote! { Some(#value) }
            }
            None => quote! { None },
        }
    };
    let get_attr_string = |name: &str| -> TokenStream {
        match find_attr(node, name) {
            Some(expr) if is_own_option_field(expr) => emit_expr(expr, ctx, &EmitMode::Construction),
            Some(expr) => {
                let value = emit_expr(expr, ctx, &EmitMode::Construction);
                quote! { Some((#value).to_string()) }
            }
            None => quote! { None },
        }
    };

    let children = node
        .child_bindings
        .iter()
        .map(|(child_binding, child_ty)| into_node_if_needed(quote! { #child_binding }, child_ty, from, table));

    match node.type_path.as_str() {
        "VerticalLayout" => {
            let spacing = get_attr("spacing");
            quote! {
                {
                    use elwindui_core::ui::VerticalLayout as _;
                    let __v = elwindui_core::ui::create_vertical_layout();
                    __v.set_spacing((#spacing).unwrap_or(0.0));
                    for __c in vec![ #(#children),* ] { __v.children().add(__c); }
                    __v
                }
            }
        }
        "HorizontalLayout" => {
            let spacing = get_attr("spacing");
            quote! {
                {
                    use elwindui_core::ui::HorizontalLayout as _;
                    let __v = elwindui_core::ui::create_horizontal_layout();
                    __v.set_spacing((#spacing).unwrap_or(0.0));
                    for __c in vec![ #(#children),* ] { __v.children().add(__c); }
                    __v
                }
            }
        }
        "Shape" => {
            let kind = find_attr(node, "kind").unwrap_or_else(|| panic!("`Shape` requires attribute `kind`"));
            let kind = emit_expr(kind, ctx, &EmitMode::Construction);
            let fill = get_attr_string("fill");
            let stroke = get_attr_string("stroke");
            let stroke_width = get_attr("stroke_width");
            quote! {
                {
                    use elwindui_core::ui::Shape as _;
                    let __v = elwindui_core::ui::create_shape();
                    __v.set_kind(#kind);
                    __v.set_fill(#fill);
                    __v.set_stroke(#stroke);
                    __v.set_stroke_width((#stroke_width).unwrap_or(0.0));
                    __v
                }
            }
        }
        "TextBlock" => {
            let text = find_attr(node, "text").unwrap_or_else(|| panic!("`TextBlock` requires attribute `text`"));
            let text = emit_expr(text, ctx, &EmitMode::Construction);
            let color = get_attr_string("color");
            quote! {
                {
                    use elwindui_core::ui::TextBlock as _;
                    let __v = elwindui_core::ui::create_text_block();
                    __v.set_text((#text).to_string());
                    __v.set_color(#color);
                    __v
                }
            }
        }
        "Control" => {
            let padding = get_attr("padding");
            quote! {
                {
                    use elwindui_core::ui::Control as _;
                    let __v = elwindui_core::ui::create_control();
                    __v.set_padding((#padding).unwrap_or(0.0));
                    for __c in vec![ #(#children),* ] { __v.children.add(__c); }
                    __v
                }
            }
        }
        "Grid" => {
            let rows = find_attr(node, "rows").unwrap_or_else(|| panic!("`Grid` requires attribute `rows`"));
            let rows = emit_expr(rows, ctx, &EmitMode::Construction);
            let columns = find_attr(node, "columns").unwrap_or_else(|| panic!("`Grid` requires attribute `columns`"));
            let columns = emit_expr(columns, ctx, &EmitMode::Construction);
            quote! {
                {
                    use elwindui_core::ui::Grid as _;
                    let __v = elwindui_core::ui::create_grid();
                    __v.set_rows((#rows).to_vec());
                    __v.set_columns((#columns).to_vec());
                    for __c in vec![ #(#children),* ] { __v.children.add(__c); }
                    __v
                }
            }
        }
        other => unreachable!("is_virtual_builtin guards this match, got `{other}`"),
    }
}

/// The concrete Rust struct to construct/store for a resolved component named `type_path` — plain
/// `format_ident!("{type_path}")`, *except* when `info` says it's composed
/// (`TypeInfo::composed_shape`, docs/elwindui_spec.md 付録H.2.1a) or a hand-written native
/// (`is_hand_written_native` — `Button`/`TextArea`/`Window`/`MenuBar`/`MenuBarItem`/`Menu`/
/// `MenuItem`/`TabView`/`TabViewItem`), in which case the bare DSL name is a real `trait` there
/// (`pub trait ContentControl: UIElement + Control`, or — for the hand-written natives — one of
/// `elwindui_core::ui`'s shared property-setter traits, docs/elwindui_spec.md 付録H.2.1a), not a
/// struct — so any site that needs a concrete, `Sized`, constructible/storable type (a
/// `Type::new(..)` call, a `#[id]`-tagged accessor's `Rc<Type>` return type) must reach for
/// `format_ident!("{type_path}Impl")` instead. Every composed component's own `generate_view` call
/// names its struct this way and mints the bare-named trait alongside (see that function's
/// `is_composed`/`composition_impls`); every backend's own `builtins` module does the same by hand
/// for its hand-written natives — `X::new(args)` still works unchanged for *callers*
/// (`emit_construction` is the one place that resolves `X` to its real `XImpl` struct before
/// emitting the call), so no hand-written or `.elwind` source ever needs to know about the `Impl`
/// suffix.
fn concrete_type_ident(type_path: &str, info: Option<&TypeInfo>) -> syn::Ident {
    if info.is_some_and(|i| i.composed_shape.is_some() || i.is_host_composition_base || is_hand_written_native(i)) {
        format_ident!("{}Impl", type_path)
    } else {
        format_ident!("{}", type_path)
    }
}

/// `"ContentControl"` -> `"create_content_control"` — the free-function factory name a composed
/// component's own struct pairs with (docs/elwindui_spec.md 付録H.2.1a), mirroring
/// `elwindui_core::ui`'s `create_control`/`create_shape`/etc. naming. Only ever called with a
/// PascalCase component name, so no acronym/consecutive-uppercase handling is needed.
fn composed_create_fn_ident(name: &str) -> syn::Ident {
    let mut snake = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                snake.push('_');
            }
            snake.extend(ch.to_lowercase());
        } else {
            snake.push(ch);
        }
    }
    format_ident!("create_{}", snake)
}

/// Maps a virtual-builtin shape name (`is_virtual_builtin`) that some `component X inherits Y`
/// names as `Y` (docs/elwindui_spec.md 付録H.2.1a) to the `elwindui_core::ui` `YImpl` struct type
/// and `Y` trait path that `X`'s generated struct composes over as its own `base` field — see
/// `generate_view`'s `is_shape_composition` branch.
fn shape_composition_base_types(base: &str) -> (TokenStream, TokenStream) {
    match base {
        "Control" => (quote! { elwindui_core::ui::ControlImpl }, quote! { elwindui_core::ui::Control }),
        "Shape" => (quote! { elwindui_core::ui::ShapeImpl }, quote! { elwindui_core::ui::Shape }),
        "TextBlock" => (quote! { elwindui_core::ui::TextBlockImpl }, quote! { elwindui_core::ui::TextBlock }),
        "Grid" => (quote! { elwindui_core::ui::GridImpl }, quote! { elwindui_core::ui::Grid }),
        "VerticalLayout" => (quote! { elwindui_core::ui::VerticalLayoutImpl }, quote! { elwindui_core::ui::VerticalLayout }),
        "HorizontalLayout" => (quote! { elwindui_core::ui::HorizontalLayoutImpl }, quote! { elwindui_core::ui::HorizontalLayout }),
        other => unreachable!("shape_composition_base_types called with non-virtual-builtin `{other}`"),
    }
}

/// `#target`'s delegating impl of its shape-composition base's own trait (docs/elwindui_spec.md
/// 付録H.2.1a) — every base method forwards to `self.base`. Empty for every shape family except
/// `Control`, whose trait declares real accessor methods (`elwindui_core::ui::Control`).
fn shape_composition_base_trait_impl(base: &str, target: &syn::Ident) -> TokenStream {
    let (_, base_trait) = shape_composition_base_types(base);
    // Fully-qualified (`#base_trait::method(&self.base, ..)`), not `self.base.method(..)`: for a
    // *template*-composed `#target` (`is_template_composition`), `self.base` is itself another DSL
    // component (e.g. `LabeledPanel`'s `base: ContentControl`), which may already have its own
    // *inherent* same-named accessor/setter (`named_accessors`, unrelated to this trait — e.g. a
    // deferred `Option<f32>` field's own `set_padding`) — plain method-call syntax would resolve to
    // that inherent method instead of this trait's own (inherent methods win method resolution over
    // same-named trait methods), a type mismatch. The fully-qualified form always reaches the
    // *trait* impl (`self.base`'s own `impl #base_trait`, itself delegating one hop further),
    // correct at any composition depth.
    match base {
        "Control" => quote! {
            impl #base_trait for #target {
                fn padding(&self) -> f32 {
                    #base_trait::padding(&self.base)
                }
                fn content_horizontal_alignment(&self) -> elwindui_core::layout::HorizontalAlignment {
                    #base_trait::content_horizontal_alignment(&self.base)
                }
                fn content_vertical_alignment(&self) -> elwindui_core::layout::VerticalAlignment {
                    #base_trait::content_vertical_alignment(&self.base)
                }
                fn set_padding(&self, padding: f32) {
                    #base_trait::set_padding(&self.base, padding)
                }
                fn set_content_horizontal_alignment(&self, alignment: elwindui_core::layout::HorizontalAlignment) {
                    #base_trait::set_content_horizontal_alignment(&self.base, alignment)
                }
                fn set_content_vertical_alignment(&self, alignment: elwindui_core::layout::VerticalAlignment) {
                    #base_trait::set_content_vertical_alignment(&self.base, alignment)
                }
            }
        },
        "Shape" => quote! {
            impl #base_trait for #target {
                fn set_kind(&self, kind: elwindui_core::ui::ShapeKind) {
                    #base_trait::set_kind(&self.base, kind)
                }
                fn set_fill(&self, fill: Option<String>) {
                    #base_trait::set_fill(&self.base, fill)
                }
                fn set_stroke(&self, stroke: Option<String>) {
                    #base_trait::set_stroke(&self.base, stroke)
                }
                fn set_stroke_width(&self, stroke_width: f32) {
                    #base_trait::set_stroke_width(&self.base, stroke_width)
                }
            }
        },
        "TextBlock" => quote! {
            impl #base_trait for #target {
                fn set_text(&self, text: String) {
                    #base_trait::set_text(&self.base, text)
                }
                fn set_color(&self, color: Option<String>) {
                    #base_trait::set_color(&self.base, color)
                }
            }
        },
        "Grid" => quote! {
            impl #base_trait for #target {
                fn set_rows(&self, rows: Vec<elwindui_core::layout::GridLength>) {
                    #base_trait::set_rows(&self.base, rows)
                }
                fn set_columns(&self, columns: Vec<elwindui_core::layout::GridLength>) {
                    #base_trait::set_columns(&self.base, columns)
                }
            }
        },
        "VerticalLayout" | "HorizontalLayout" => quote! {
            impl #base_trait for #target {
                fn set_spacing(&self, spacing: f32) {
                    #base_trait::set_spacing(&self.base, spacing)
                }
            }
        },
        other => unreachable!("shape_composition_base_types called with non-virtual-builtin `{other}`"),
    }
}

/// Attaches callbacks (`on_*`) and two-way change-back wiring to widgets that were stored on
/// `self`, each capturing a fresh `Rc::clone` and calling `resync()` after mutating the model. No
/// per-type dispatch: any attribute named `on_*` is a callback (its shape's declared param type
/// decides whether the callback takes an index — see `emit_wiring`'s doc on `takes_index` below);
/// any attribute whose shape field is `#[two_way]` gets a `set_on_<attr>_change` callback wired
/// back into its bound path.
fn emit_wiring(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, out: &mut TokenStream) {
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { this });
    let info = table.resolve(from, &node.type_path);
    // `emit_wiring`'s own output lands in `NotepadWindowImpl::new()`, a *different* function from
    // wherever `emit_construction` ran (for a composed/host-composed target, that's the separate
    // `create_<snake case>(..)` free function — see `generate_view`'s `create_fn`/
    // `new_construct_stmt` split) — so the `use` injected there doesn't carry over here. See
    // `emit_resync`'s own copy of this same comment.
    out.extend(hand_written_native_trait_use(&node.type_path));

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
            // through ancestors too, rather than this being the only thing that ever runs. Zero-
            // arg only for now (`T = ()`) — see `ast::Attr::Routed`'s doc comment.
            let is_routed = info.is_some_and(|i| i.routed_fields.contains(name));
            if is_routed {
                let call = emit_expr(expr, ctx, &self_mode);
                out.extend(quote! {
                    {
                        let widget = this.#binding.clone();
                        let this = std::rc::Rc::clone(&this);
                        widget.register_routed_handler::<()>(#name, Box::new(move |_: &(), _args: &elwindui_core::input::RoutedEventArgs| {
                            #call;
                            this.resync();
                        }));
                    }
                });
                continue;
            }
            // A callback whose shape declares `Fn(usize)` (e.g. `TabView`'s per-tab `on_select`/
            // `on_close`) is a bare command path that needs an index threaded through
            // (`command_execute_call`, reused as-is from its original TabView-only use); anything
            // else (`Fn()`, e.g. `on_click`/`on_new_tab`) is an ordinary zero-arg call.
            let takes_index = info
                .and_then(|i| i.field_types.get(name))
                .is_some_and(|ty| ty.contains("usize"));
            if takes_index {
                let call = command_execute_call(node, name, ctx, &self_mode, quote! { index });
                out.extend(quote! {
                    {
                        let widget = this.#binding.clone();
                        let this = std::rc::Rc::clone(&this);
                        widget.#setter(Box::new(move |index: usize| {
                            #call;
                            this.resync();
                        }));
                    }
                });
            } else {
                let call = emit_expr(expr, ctx, &self_mode);
                out.extend(quote! {
                    {
                        let widget = this.#binding.clone();
                        let this = std::rc::Rc::clone(&this);
                        widget.#setter(Box::new(move || {
                            #call;
                            this.resync();
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
                        let widget = this.#binding.clone();
                        let this = std::rc::Rc::clone(&this);
                        widget.#change_setter(Box::new(move |new_value| {
                            #setter(new_value);
                            this.resync();
                        }));
                    }
                });
            }
        }
    }
}

/// Re-pushes every dynamic (non-callback, non-`Element`/`Closure`-valued) attribute of every
/// stored widget from current model state, calling `set_<attr>(value)` — same "blanket resync"
/// design as before (see docs/elwindui_gui_framework_design.md's "設計方針" note), just no longer
/// keyed on `node.type_path`: any resolved type works as long as it exposes a matching setter.
/// `#[two_way]` attributes (e.g. `TextArea`'s `text`) are resynced the same as any other — this
/// pushes model→widget; `emit_wiring`'s separate `set_on_<attr>_change` callback is what pushes
/// widget→model.
fn emit_resync(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, out: &mut TokenStream) {
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { self });
    let info = table.resolve(from, &node.type_path);
    // `resync()` is its own function, a separate lexical scope from `new()` — the `use` already
    // injected alongside construction (`emit_construction`'s `hand_written_native_trait_use`)
    // doesn't carry over here, so any hand-written native whose setters are shared-trait-only
    // needs its own copy of the same import for this function's own `self.#binding.#setter(..)`
    // calls below.
    out.extend(hand_written_native_trait_use(&node.type_path));

    for (name, expr) in &node.attributes {
        if name.starts_with("on_") {
            continue;
        }
        if matches!(expr, ViewExpr::Element(_) | ViewExpr::Closure { .. }) {
            continue;
        }
        // A `view`-having (`has_view`) target's own `#[param]` fields are fixed at construction —
        // `generate_view` never emits a `set_<param>` for them (unlike every hand-written builtin,
        // which by convention always defines one, even a no-op, for the "blanket resync" rule
        // above to call generically) — so resyncing one here would be calling a method that simply
        // doesn't exist (e.g. `RoundedPanel`'s `#[param] label: String`).
        if info.is_some_and(|i| i.has_view && i.param_fields.iter().any(|(n, _)| n == name)) {
            continue;
        }

        let setter = format_ident!("set_{name}");
        let value = emit_expr(expr, ctx, &self_mode);
        // The resync value itself is never `Option`-wrapped (only construction-time args are, per
        // the shape's own `Option<..>` convention for "may be absent"), so copy-ness is judged on
        // the stripped inner type — `Option<String>`'s runtime value here is a plain `String`.
        let is_copy = info
            .and_then(|i| i.field_types.get(name))
            .is_some_and(|ty| is_copy_type(strip_option(ty).0));
        if is_copy {
            out.extend(quote! { self.#binding.#setter(#value); });
        } else {
            out.extend(quote! { self.#binding.#setter(&(#value)); });
        }
    }
}

/// Resolves a `TabView` callback attribute (`on_select`/`on_close`) — a bare 2-segment path to a
/// `Command` (e.g. `vm.select_tab`, *not* a `.execute()` call, since the call itself needs to
/// happen later with a concrete per-tab index that isn't known until `emit_tabview_resync` is
/// building that specific tab's widgets) — into a call expression against that command's
/// generated `_execute` method, passing `index_arg` (usually the loop-local tab index).
fn command_execute_call(
    node: &PlannedNode,
    attr_name: &str,
    ctx: &ViewCtx,
    mode: &EmitMode,
    index_arg: TokenStream,
) -> TokenStream {
    let Some(ViewExpr::Path(path)) = find_attr(node, attr_name) else {
        panic!("TabView's `{attr_name}` must be a bare command path, e.g. `vm.select_tab`");
    };
    let resolved = resolve_bind(path, &ctx.binds);
    let (owner_path, command) = resolved.split_at(resolved.len() - 1);
    let owner = owner_path.last().cloned().unwrap_or_else(|| "vm".to_string());
    let base = mode.owner_tokens(&owner);
    let execute = format_ident!("{}_execute", command[0]);
    quote! { #base.#execute(#index_arg) }
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
            // `data_context` (WinUI3's `FrameworkElement.DataContext`, 付録Y — lowercased to match
            // this DSL's snake_case attribute naming, e.g. the `data_context:` attribute itself)
            // is sugar for the enclosing `header_template`/`item_template` closure's own bound
            // parameter — substituted for it before any other resolution, so `data_context.field`
            // behaves exactly like writing the closure's real parameter name (`doc.field`) already
            // does. Outside such a closure (`ctx.closure_param` is `None`) it's left alone and
            // falls through to ordinary path resolution, which fails to resolve it (by design —
            // this sugar is scoped to template closures only, see
            // docs/elwindui_builtins_spec.md 付録Y).
            let substituted_path;
            let path: &[String] = if path.first().map(String::as_str) == Some("data_context") {
                if let Some(param) = &ctx.closure_param {
                    substituted_path = std::iter::once(param.clone()).chain(path[1..].iter().cloned()).collect::<Vec<_>>();
                    &substituted_path
                } else {
                    path.as_slice()
                }
            } else {
                path.as_slice()
            };
            // A bare reference to the closure's own bound parameter (e.g. `doc` in
            // `item_template: |doc| DocumentView { doc: doc }`) passes the value straight
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
                    return mode.owner_tokens(only);
                }
            }
            let resolved = resolve_bind(path, &ctx.binds);
            emit_path_get(&resolved, mode)
        }
        ViewExpr::MethodCall(path, method) => {
            let resolved = resolve_bind(path, &ctx.binds);
            let (owner_path, command) = resolved.split_at(resolved.len() - 1);
            let owner = owner_path.last().cloned().unwrap_or_else(|| "vm".to_string());
            let base = mode.owner_tokens(&owner);
            let call = format_ident!("{}_{}", command[0], method);
            quote! { #base.#call() }
        }
        ViewExpr::TFluent(key, args) => {
            let arg_pairs = args.iter().map(|(name, value)| {
                let value_tokens = emit_expr(value, ctx, mode);
                quote! { (#name, elwindui_i18n::FluentValue::from(#value_tokens)) }
            });
            quote! { elwindui_i18n::t(#key, &[ #(#arg_pairs),* ]) }
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
/// `self.vm.content()` (with self). `["vm", "save", "can_execute"]` (付録O.3's `vm.save.can_execute`)
/// is the one 3-segment shape: the middle segment names a `#[command]` field, so it's folded
/// together with `can_execute` into a single `save_can_execute()` accessor.
fn emit_path_get(path: &[String], mode: &EmitMode) -> TokenStream {
    match path {
        [owner, field] => {
            let base = mode.owner_tokens(owner);
            let getter = format_ident!("{}", field);
            quote! { #base.#getter() }
        }
        [owner, command, suffix] if suffix == "can_execute" => {
            let base = mode.owner_tokens(owner);
            let getter = format_ident!("{}_can_execute", command);
            quote! { #base.#getter() }
        }
        other => panic!("unsupported path shape after bind resolution: `{}`", other.join(".")),
    }
}

fn emit_setter(path: &[String], mode: &EmitMode) -> TokenStream {
    let [owner, field] = path else {
        panic!("expected a 2-segment path after bind resolution, got `{}`", path.join("."));
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
        let all: Vec<Module> = modules.iter().cloned().chain(crate::builtin_modules()).collect();
        build_symbol_table(&all)
    }

    const VIEWMODEL_SRC: &str = r#"
enum SaveState { Unsaved, Saving, Saved }

viewmodel NotepadViewModel {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",

    #[observable]
    state: SaveState = SaveState::Unsaved,

    #[computed]
    char_count: i32 = content.chars().count() as i32,

    #[computed]
    window_title: String = t!("notepad-window-title", file_name: file_name),

    #[command(can_execute: state != SaveState::Saving)]
    save: Command = command!(|| {
        state = SaveState::Saving;
        document::save(&content);
        state = SaveState::Saved;
    }),

    #[command]
    open: Command = command!(|| {
        content = document::open_dialog();
        state = SaveState::Unsaved;
    }),
}
"#;

    const WINDOW_SRC: &str = r#"
use crate::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
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
                    on_click: vm.save.execute()
                    enabled: vm.save.can_execute
                }
                Button {
                    text: t!("notepad-menu-open")
                    on_click: vm.open.execute()
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
    fn generates_valid_rust_for_notepad() {
        let viewmodel_module = parse_module(VIEWMODEL_SRC).unwrap();
        let window_module = parse_module(WINDOW_SRC).unwrap();
        let table = build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let viewmodel_code = generate_module(&viewmodel_module, &table);
        assert_valid_rust("notepad_viewmodel", &viewmodel_code);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("notepad_window", &window_code);

        let window_str = window_code.to_string();
        assert!(window_str.contains("struct NotepadWindow"));
        assert!(window_str.contains("fn resync"));
        assert!(window_str.contains("save_execute"));
        assert!(window_str.contains("save_can_execute"));
    }

    #[test]
    fn command_attr_desugars_to_execute_wiring_and_enabled_resync() {
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

        HorizontalLayout {
            Button {
                text: t!("notepad-menu-save")
                command: vm.save
            }
        }
    }
}
"#;
        let viewmodel_module = parse_module(VIEWMODEL_SRC).unwrap();
        let window_module = parse_module(window_src).unwrap();
        let table = build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("command_attr_window", &window_code);

        let window_str = window_code.to_string();
        // `command: vm.save` must desugar to exactly what `on_click: vm.save.execute()` +
        // `enabled: vm.save.can_execute` generate by hand (see `desugar_command_attr`). `on_click`
        // is `#[routed]` (`Button` in `builtins.elwind`), so it's wired via
        // `register_routed_handler`, not `set_on_click` directly — see `emit_wiring`'s `is_routed`
        // branch.
        assert!(window_str.contains("register_routed_handler"));
        assert!(window_str.contains("save_execute"));
        assert!(window_str.contains("save_can_execute"));
    }

    #[test]
    fn command_attr_does_not_override_an_explicit_on_click() {
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

        HorizontalLayout {
            Button {
                text: t!("notepad-menu-save")
                command: vm.save
                on_click: vm.open.execute()
            }
        }
    }
}
"#;
        let viewmodel_module = parse_module(VIEWMODEL_SRC).unwrap();
        let window_module = parse_module(window_src).unwrap();
        let table = build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("command_attr_explicit_on_click_window", &window_code);

        let window_str = window_code.to_string();
        // The explicit `on_click` wins — `command`'s own execute-wiring is not also emitted, so
        // `save_execute` never appears (only `open_execute`, from the explicit `on_click`), but
        // `command`'s `enabled` wiring (no explicit `enabled` given) still comes through.
        assert!(window_str.contains("open_execute"));
        assert!(!window_str.contains("save_execute"));
        assert!(window_str.contains("save_can_execute"));
    }

    #[test]
    fn generates_valid_rust_for_menubar_and_tabview() {
        let viewmodel_src = r#"
viewmodel Document {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",
}

viewmodel NotepadViewModel {
    #[observable]
    documents: Vec<Document> = Vec::new(),

    #[observable]
    active_tab: usize = 0,

    #[command]
    new_tab: Command = command!(|| {
        documents.push(std::rc::Rc::new(Document::new()));
        active_tab = documents.len() - 1;
    }),

    #[command]
    close_tab: Command = command!(|index: usize| {
        documents.remove(index);
    }),

    #[command]
    select_tab: Command = command!(|index: usize| {
        active_tab = index;
    }),
}
"#;
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
                    MenuItem { text: t!("menu-new"), shortcut: "n", on_select: vm.new_tab.execute() }
                }
            }
        }

        content: TabView {
            items_source: vm.documents
            header_template: |doc| doc.file_name
            item_template: |doc| TextArea { text: doc.content }
            selected_index: vm.active_tab
            on_select: vm.select_tab
            on_close: vm.close_tab
            on_new_tab: vm.new_tab.execute()
            closable: true
        }
    }
}
"#;
        let viewmodel_module = parse_module(viewmodel_src).expect("viewmodel should parse");
        let window_module = parse_module(window_src).expect("window should parse");
        let table = build_symbol_table_with_builtins(&[viewmodel_module.clone(), window_module.clone()]);

        let viewmodel_code = generate_module(&viewmodel_module, &table);
        assert_valid_rust("menubar_tabview_viewmodel", &viewmodel_code);
        let viewmodel_str = viewmodel_code.to_string();
        assert!(viewmodel_str.contains("documents_push"));
        assert!(viewmodel_str.contains("documents_remove"));
        assert!(viewmodel_str.contains("Rc < Document >"));
        assert!(viewmodel_str.contains("fn close_tab_execute (& self , index : usize)"));

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("menubar_tabview_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains("MenuBarImpl :: new"));
        assert!(window_str.contains("MenuItemImpl :: new"));
        assert!(window_str.contains("set_shortcut"));
        assert!(window_str.contains("TabViewImpl :: new"));
        // `TabView`'s per-tab chip/content materialization (`insert_tab`, `__weak_self`) is no
        // longer generated here at all — it's hand-written Rust inside the corresponding
        // `elwindui-backend-*` crate now, reached generically the same way any other resolved
        // type's constructor is.
        assert!(!window_str.contains("insert_tab"));
        assert!(!window_str.contains("__weak_self"));
        assert!(window_str.contains("set_items_source"));
        assert!(window_str.contains("set_selected_index"));
    }

    #[test]
    fn generates_valid_rust_for_tabview_header_template_and_item_template() {
        let viewmodel_src = r#"
viewmodel Document {
    #[observable]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",
}

viewmodel NotepadViewModel {
    #[observable]
    documents: Vec<Document> = Vec::new(),

    #[observable]
    active_tab: usize = 0,

    #[command]
    new_tab: Command = command!(|| {
        documents.push(std::rc::Rc::new(Document::new()));
        active_tab = documents.len() - 1;
    }),

    #[command]
    close_tab: Command = command!(|index: usize| {
        documents.remove(index);
    }),

    #[command]
    select_tab: Command = command!(|index: usize| {
        active_tab = index;
    }),
}
"#;
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
            items_source: vm.documents
            header_template: |doc| doc.file_name
            item_template: |doc| DocumentView { doc: doc }
            selected_index: vm.active_tab
            on_select: vm.select_tab
            on_close: vm.close_tab
            on_new_tab: vm.new_tab.execute()
            closable: true
        }
    }
}
"#;
        let viewmodel_module = parse_module(viewmodel_src).expect("viewmodel should parse");
        let document_view_module = parse_module(document_view_src).expect("document view should parse");
        let window_module = parse_module(window_src).expect("window should parse");
        let modules = [viewmodel_module.clone(), document_view_module.clone(), window_module.clone()];
        let all_modules: Vec<_> = modules.iter().cloned().chain(crate::builtin_modules()).collect();
        let table = build_symbol_table(&all_modules);

        assert_eq!(crate::validate::validate(&all_modules), Ok(()));

        let document_view_code = generate_module(&document_view_module, &table);
        assert_valid_rust("document_view", &document_view_code);
        let document_view_str = document_view_code.to_string();
        assert!(document_view_str.contains("fn new (doc : std :: rc :: Rc < Document >)"));
        assert!(!document_view_str.contains("fn show"), "DocumentView's root isn't `Window` — `show()` shouldn't be generated");
        // `VerticalLayout` is a hand-written *virtual* builtin (no backend struct — see
        // `is_virtual_builtin`), so `DocumentView`'s root is virtual too (recursively inferred,
        // `build_symbol_table`'s `resolve_is_native`) and it generates `into_node`, not the old
        // `into_any_view`.
        assert!(document_view_str.contains("fn into_node"), "document_view_str: {document_view_str}");

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("tabview_render_content_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains("DocumentView :: new"));
        // `TabView` is a native-control leaf (`TypeInfo::is_native_control_leaf`) — it already
        // implements `UIElement` on its own construction-time `base`, so embedding it as `Window`'s
        // `content` is a plain upcast, not the old `.into_any_view()`-based wrapping.
        // `DocumentView` itself is virtual, so `render_content`'s body calls `.into_node()` on it
        // instead.
        assert!(!window_str.contains("into_any_view"), "window_str: {window_str}");
        assert!(window_str.contains(". into_node ()"), "window_str: {window_str}");
        assert!(
            !window_str.contains("TextArea :: new (& __doc . content ())"),
            "the fixed TextArea fallback shouldn't be emitted once `item_template` is present"
        );
        // `header_template`'s body must go through the getter-call sugar (`.file_name()`), not a
        // raw field access — see the `parse_closure_expr_body` bug this test guards against.
        assert!(window_str.contains("doc . file_name ()"));
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
        assert!(s.contains("fn new (greeter : Greeter)"), "expected ctor param named `greeter`, got:\n{s}");
        assert!(!s.contains("vm"), "ctor shouldn't hardcode a `vm` field name:\n{s}");
        // `Greeting`'s view root is `TextBlock`, not `Window` — no top-level window to `show()`.
        assert!(!s.contains("fn show"));
        assert!(s.contains("fn into_node"));
    }

    #[test]
    fn generates_valid_rust_for_async_command_with_nested_t_macro() {
        let src = r#"
viewmodel FileViewModel {
    #[observable]
    content: String = String::new(),

    #[observable]
    status: String = String::new(),

    #[command(async)]
    open: Command = command!(async || {
        if let Some(path) = platform::file_dialog::open().await {
            content = std::fs::read_to_string(&path).unwrap_or_default();
            status = t!("opened-status", name: content);
        }
    }),
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("async_command", &generated);

        let generated_str = generated.to_string();
        assert!(generated_str.contains("elwindui_core :: task :: spawn_local"));
        assert!(
            generated_str.contains("__self . content ()"),
            "t!(...) args inside an async command body must resolve through `__self`, not a \
             borrowed `self` that can't outlive the call:\n{generated_str}"
        );
        assert!(generated_str.contains("async"));
        assert!(generated_str.contains("elwindui_i18n :: t"));
        assert!(!generated_str.contains("t !"), "t!(...) should have been rewritten, not left as a macro call");
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
        // `ContentControlImpl` (the bare `ContentControl` is now a real trait), so `Foo`'s own
        // generated code, resolving `ContentControl` as a child element, must construct that
        // concrete type (`emit_construction`'s `concrete_type_ident`).
        assert!(generated_str.contains("ContentControlImpl :: new"), "{generated_str}");

        // `ContentControl`'s own generated code (produced when `builtin_modules()` is fed through
        // `generate_module` directly, mirroring how a real consumer's own `.elwind` component
        // would be generated) forwards `content` into `Control`'s children and exposes both
        // `#[param]` fields as public accessors. `builtins.elwind` bundles every builtin into one
        // module, so only `ContentControl`'s own `Item::Component`/`Item::View` pair is kept —
        // `generate_module` would otherwise also try (and fail) to generate every shape-only
        // builtin sharing that module (mirroring `compile_dir_impl`'s own filtering in `lib.rs`).
        let builtins_module = crate::builtin_modules()
            .into_iter()
            .find(|m| m.items.iter().any(|i| matches!(i, Item::Component(c) if c.name == "ContentControl")))
            .expect("ContentControl should be a registered builtin");
        let content_control_module = Module {
            items: builtins_module
                .items
                .iter()
                .filter(|i| matches!(i, Item::Component(c) if c.name == "ContentControl") || matches!(i, Item::View(v) if v.target == "ContentControl"))
                .cloned()
                .collect(),
            ..builtins_module
        };
        let content_control_code = generate_module(&content_control_module, &table);
        assert_valid_rust("content_control_impl", &content_control_code);
        let content_control_str = content_control_code.to_string();
        assert!(content_control_str.contains("elwindui_core :: ui :: create_control"));
        // `content`/`padding` are now trait methods (Phase E, docs/elwindui_spec.md 付録H.2.1a) —
        // no `pub` (trait methods have no visibility modifier of their own), declared in the trait
        // and implemented in `impl ContentControl for ContentControlImpl` below.
        assert!(content_control_str.contains("fn content (& self) -> std :: rc :: Rc < dyn UIElement >"));
        assert!(content_control_str.contains("fn padding (& self) -> Option < f32 >"));
        // Real struct is `ContentControlImpl`; the bare `ContentControl` is a real trait instead
        // (docs/elwindui_spec.md 付録H.2.1a) — no `struct`/`trait` namespace clash since they're
        // different identifiers.
        assert!(content_control_str.contains("struct ContentControlImpl"), "{content_control_str}");
        assert!(content_control_str.contains("pub trait ContentControl"), "{content_control_str}");
        assert!(content_control_str.contains("impl ContentControl for ContentControlImpl"), "{content_control_str}");
    }

    /// A bare nested child element with nowhere to go (no `children` field, no
    /// `#[content(field_name)]` on the component being constructed — `Button` has neither) is a hard
    /// codegen-time error: `build_component_args` no longer falls back to guessing "the first
    /// still-unclaimed required field" by declaration order the way it used to before
    /// `#[content(..)]` existed.
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
        // Real struct is `LabeledPanelImpl`; the bare `LabeledPanel` is a real trait instead
        // (docs/elwindui_spec.md 付録H.2.1a) — same reasoning as `ContentControl`/`ContentControlImpl`.
        assert!(generated_str.contains("struct LabeledPanelImpl"), "{generated_str}");
        assert!(generated_str.contains("pub trait LabeledPanel"), "{generated_str}");
        // Real base composition one level deeper than `ContentControl` itself: `LabeledPanelImpl`
        // embeds a real `base: ContentControlImpl` (built by calling `ContentControl`'s own
        // `create_content_control(..)` factory), not a copy of `Control`'s construction —
        // `elwindui_core::ui::create_control` only ever appears in `ContentControl`'s *own*
        // generated code (not exercised by this test, which only generates `LabeledPanel`).
        assert!(generated_str.contains("base : ContentControlImpl"), "{generated_str}");
        assert!(generated_str.contains("create_content_control"), "{generated_str}");
        assert!(generated_str.contains("impl elwindui_core :: ui :: UIElement for LabeledPanelImpl"), "{generated_str}");
        assert!(generated_str.contains("impl elwindui_core :: ui :: Control for LabeledPanelImpl"), "{generated_str}");
        assert!(generated_str.contains("impl LabeledPanel for LabeledPanelImpl"), "{generated_str}");
        // No `pub` — `content` is now a trait method (Phase E, docs/elwindui_spec.md 付録H.2.1a),
        // not a plain inherent one.
        assert!(
            generated_str.contains("fn content (& self) -> std :: rc :: Rc < dyn UIElement > { self . base . content () }"),
            "{generated_str}"
        );
    }

    /// `#[override] fn` + `base::name(...)` (§3): the derived's override calls into a
    /// `__base_<name>`-shadowed copy of the base's original body, and `on_mount { base::on_mount(); }`
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
        assert!(generated_str.contains("fn __base_on_mount"), "{generated_str}");
        assert!(generated_str.contains("this . __base_on_mount"), "{generated_str}");
    }

    /// `Grid` (§3) + attached properties (`Grid::row`/`Grid::column`, §3) end to end: a `view`
    /// using `Grid` with `rows`/`columns` array-literal params and attached setters on its children
    /// must generate valid Rust, constructing `elwindui_core::ui::Grid` directly (a virtual
    /// builtin, like `Control`/`Shape`) with each virtual child's own `grid_cell` populated.
    #[test]
    fn generates_valid_rust_for_grid_with_attached_properties() {
        let src = r#"
component Foo {
}

view Foo {
    Grid {
        rows: [elwindui_core::layout::GridLength::Auto, elwindui_core::layout::GridLength::Star(1.0)]
        columns: [elwindui_core::layout::GridLength::Fixed(120.0), elwindui_core::layout::GridLength::Star(1.0)]
        TextBlock { text: "Header", Grid::row: 0, Grid::column: 0 }
        Shape { kind: elwindui_core::ui::ShapeKind::RoundedRect { corner_radius: 4.0 }, fill: "black", Grid::row: 1, Grid::column: 1 }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(&[module.clone()]);
        let generated = generate_module(&module, &table);
        assert_valid_rust("grid_with_attached_properties", &generated);

        let generated_str = generated.to_string();
        assert!(generated_str.contains("elwindui_core :: ui :: create_grid"), "{generated_str}");
        assert!(generated_str.contains("GridLength :: Auto"), "{generated_str}");
        assert!(generated_str.contains("GridLength :: Fixed (120.0)"), "{generated_str}");
        assert!(generated_str.contains("GridCell { row : 0 , column : 0 }"), "{generated_str}");
        assert!(generated_str.contains("GridCell { row : 1 , column : 1 }"), "{generated_str}");
    }
}
