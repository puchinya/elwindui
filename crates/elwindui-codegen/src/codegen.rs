//! AST(検証済み) → backend別Rustソース。`target::backend()`の定数畳み込みは付録Dの通りCargo
//! featureでの静的分岐に落とし込み、`elwindui-core`のトレイト境界に対して書かれたコードを生成する
//! (今回はelwindui-backend-appkitのAPIを直接呼ぶ)。
//! 依存関係グラフに基づくCell/RefCellベースの更新関数生成は付録O.5に対応する。

use crate::ast::{
    Attr, ClosureBody, ComponentDef, ElementNode, EnumDef, FieldKind, Initializer, Item, Module,
    ViewDef, ViewExpr, ViewModelDef,
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
}

impl SymbolTable {
    /// Resolves `name` as seen from `from`: a type defined locally in `from` (same real path), or
    /// brought into scope by one of `from`'s `use` declarations, matched by real path exactly like
    /// Rust's own name resolution (`use`'s last path segment is the item name; the segments before
    /// it — with a leading `crate` keyword stripped, since `Module::path` never includes it — must
    /// equal some module's real path). Returns `None` if `name` isn't visible from `from` at all —
    /// an unresolved reference (e.g. a missing `use`), which callers turn into a validation error.
    pub fn resolve(&self, from: &Module, name: &str) -> Option<&TypeInfo> {
        if let Some(info) = self.types.get(&(from.path.clone(), name.to_string())) {
            return Some(info);
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
            self.types.get(&(real_prefix.to_vec(), name.to_string()))
        })
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
    for module in modules {
        for item in &module.items {
            let (name, fields, is_viewmodel) = match item {
                Item::Component(c) => (c.name.clone(), &c.fields, false),
                Item::ViewModel(v) => (v.name.clone(), &v.fields, true),
                Item::Enum(_) | Item::View(_) => continue,
            };
            let field_kinds = fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
            let binds = fields
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
            let param_fields = fields
                .iter()
                .filter(|f| f.kind == FieldKind::Param && f.initializer.is_none())
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect();
            let two_way_fields = fields
                .iter()
                .filter(|f| f.initializer.is_none() && f.attrs.iter().any(|a| matches!(a, Attr::TwoWay)))
                .map(|f| f.name.clone())
                .collect();
            let field_types = fields
                .iter()
                .filter(|f| f.initializer.is_none())
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect();
            types.insert(
                (module.path.clone(), name),
                TypeInfo { fields: field_kinds, binds, param_fields, two_way_fields, field_types, is_viewmodel },
            );
        }
    }
    SymbolTable { types }
}

pub fn generate_module(module: &Module, table: &SymbolTable) -> TokenStream {
    // A `component`/`view` pair sharing a name is generated as a single struct+impl (by
    // `generate_view`, which also owns the widget fields); a bare `component` with no matching
    // `view` falls back to `generate_component`'s plain struct+accessors.
    let view_targets: HashSet<&str> = module
        .items
        .iter()
        .filter_map(|i| match i {
            Item::View(v) => Some(v.target.as_str()),
            _ => None,
        })
        .collect();

    let components: HashMap<&str, &ComponentDef> = module
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Component(c) => Some((c.name.as_str(), c)),
            _ => None,
        })
        .collect();

    let mut out = TokenStream::new();
    for item in &module.items {
        out.extend(match item {
            Item::Enum(e) => generate_enum(e),
            Item::ViewModel(v) => generate_viewmodel(v, module, table),
            Item::Component(c) if view_targets.contains(c.name.as_str()) => TokenStream::new(),
            Item::Component(c) => generate_component(c, table),
            Item::View(v) => {
                let component = components
                    .get(v.target.as_str())
                    .unwrap_or_else(|| panic!("view `{}` has no matching `component`", v.target));
                generate_view(v, component, module, table)
            }
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
            FieldKind::Prop | FieldKind::Param => {
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
                // `#[param] #[inject]` field: supplied by the caller, stored as-is.
                struct_fields.extend(quote! { pub #field_ident: #ty, });
                ctor_params.extend(quote! { #field_ident: #ty, });
                ctor_field_inits.extend(quote! { #field_ident, });
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
    quote! {
        pub struct #struct_name {
            #struct_fields
        }

        impl #struct_name {
            pub fn new(#ctor_params) -> Self {
                Self { #ctor_field_inits }
            }

            #accessors
        }
    }
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
    // A `component`/`view` pair always shares one `.elwind` file (`generate_module`'s
    // `view_targets` check), so the target is always defined locally in `from` — no `use` needed.
    let binds = table
        .resolve(from, &target_name)
        .map(|t| t.binds.clone())
        .unwrap_or_default();
    let ctx = ViewCtx { binds, closure_param: None };

    // The component's own `#[param]`-shaped fields (no initializer) become `new`'s positional
    // arguments and private struct fields — e.g. `NotepadWindow`'s `#[param] #[inject] vm:
    // NotepadViewModel`, or `DocumentView`'s `#[param] #[inject] doc: Rc<DocumentViewModel>`.
    // Bind-sugar fields (`content: String = bind!(doc.content, TwoWay)`) need no storage of their
    // own here — `ctx.binds` already resolves them straight through wherever referenced below.
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
    plan_element(&view.root, &ctx, &mut plan, true);

    let mut struct_fields = TokenStream::new();
    let mut construct_stmts = TokenStream::new();
    let mut field_inits = TokenStream::new();
    let mut wiring_stmts = TokenStream::new();
    let mut resync_stmts = TokenStream::new();

    for node in &plan {
        emit_construction(node, &ctx, from, table, &mut construct_stmts);
        if node.stored {
            let binding = &node.binding;
            // Every resolved type (a `component`/`view` pair or a hand-written builtin in
            // `elwindui-builtins`) is constructed as `Rc<Self>` uniformly (see `emit_construction`
            // and this same convention below in `root_embed_method`), so a stored handle is always
            // just `Rc<Type>` — no backend-crate-qualified path, no per-type bookkeeping fields.
            let type_ident = format_ident!("{}", node.type_path);
            struct_fields.extend(quote! { #binding: std::rc::Rc<#type_ident>, });
            field_inits.extend(quote! { #binding: #binding.clone(), });
        }
    }
    for node in &plan {
        emit_wiring(node, &ctx, from, table, &mut wiring_stmts);
        emit_resync(node, &ctx, from, table, &mut resync_stmts);
    }

    // `plan_element` pushes children before their parent (post-order), so the root is always last.
    let root_binding = &plan.last().expect("view must have a root element").binding;

    // `show` only exists on the `Window` builtin — a component whose view root is something else
    // (e.g. `DocumentView`'s `Column`) has no top-level window to show, only a root widget to be
    // embedded by whatever contains it (a `TabView`'s `render_content`, a plain `Row`/`Column`
    // child, etc.). For those, `into_any_view` is emitted instead — a *local* inherent method (not
    // a `From`/`Into` impl: `impl From<Rc<#target>> for AnyView` would be rejected by Rust's
    // orphan rules, since `Rc` isn't "fundamental" and so `#target` nested inside it counts as
    // covered by a foreign generic — E0117) that `into_any_view_if_needed` calls from
    // `Column`/`Row`/`Window`/`TabView`'s `render_content` embedding.
    //
    // Deliberately non-blocking (unlike the old `open`/`show_and_run`): entering the platform
    // event loop is `elwindui::application::run()`'s job, called once after every top-level
    // window has been shown — see `elwindui-backend-appkit`'s `application` module.
    let root_embed_method = if view.root.type_path == "Window" {
        quote! {
            pub fn show(self: std::rc::Rc<Self>) {
                self.#root_binding.clone().show();
            }
        }
    } else {
        let root_expr = into_any_view_if_needed(quote! { self.#root_binding }, "AnyView");
        quote! {
            pub fn into_any_view(self: std::rc::Rc<Self>) -> elwindui_backend_appkit::AnyView {
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

    quote! {
        impl #target {
            pub fn new(#(#param_names: #param_types),*) -> std::rc::Rc<Self> {
                #construct_stmts
                let this = std::rc::Rc::new(Self { #(#param_names,)* #field_inits });
                #wiring_stmts
                // Most widgets already read live model state at construction time, so this is a
                // no-op for them. A widget whose own state only ever appears in `resync()` (e.g. a
                // dynamic list, like `TabView`'s tabs) needs this call so state populated before
                // construction (as `main.rs` does, calling `new_tab_execute()` first) appears
                // immediately rather than waiting for the first unrelated user interaction.
                this.resync();
                #subscribe_stmts
                this
            }

            fn resync(&self) {
                #resync_stmts
            }

            #root_embed_method
        }

        pub struct #target {
            #(#param_names: #param_types,)*
            #struct_fields
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
}

impl ViewCtx {
    fn with_closure_param(&self, param: &str) -> ViewCtx {
        ViewCtx { binds: self.binds.clone(), closure_param: Some(param.to_string()) }
    }
}

/// One element flattened out of the tree, in construction order (children before parents).
struct PlannedNode {
    binding: syn::Ident,
    type_path: String,
    attributes: Vec<(String, ViewExpr)>,
    /// Bindings of the element's *bare* nested children (`Type { ... }` written directly inside
    /// `{}`, not as `name: value`). Used to fill a resolved shape's `children`-named `#[param]`
    /// (an implicit list) or, absent one, a single required param with no matching attribute (a
    /// positional slot — e.g. `MenuBarItem`'s one nested `Menu`).
    child_bindings: Vec<syn::Ident>,
    /// Bindings of `ViewExpr::Element`-valued *attributes* (a "named single-child slot", e.g.
    /// `menu_bar: MenuBar { .. }`), keyed by attribute name — planned/constructed the same way
    /// `child_bindings` are, just addressed by name instead of position.
    element_attr_bindings: HashMap<String, syn::Ident>,
    /// Has an attribute at all (so it might need wiring/resync later), so it needs a struct field
    /// (rather than being a construction-time-only local). No per-type list to check against
    /// anymore — every resolved type is handled identically.
    stored: bool,
}

fn plan_element(node: &ElementNode, ctx: &ViewCtx, out: &mut Vec<PlannedNode>, is_root: bool) -> syn::Ident {
    let mut child_bindings = Vec::new();
    for child in &node.children {
        child_bindings.push(plan_element(child, ctx, out, false));
    }

    let mut element_attr_bindings = HashMap::new();
    for (name, expr) in &node.attributes {
        if let ViewExpr::Element(elem) = expr {
            element_attr_bindings.insert(name.clone(), plan_element(elem, ctx, out, false));
        }
    }

    let binding = format_ident!("__{}_{}", node.type_path.to_lowercase(), out.len());
    let stored = is_root || !node.attributes.is_empty();

    out.push(PlannedNode {
        binding: binding.clone(),
        type_path: node.type_path.clone(),
        attributes: node.attributes.clone(),
        child_bindings,
        element_attr_bindings,
        stored,
    });
    binding
}

fn find_attr<'a>(node: &'a PlannedNode, name: &str) -> Option<&'a ViewExpr> {
    node.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v)
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
/// (its declared type mentions `AnyView` — `Column`/`Row`'s `children: Vec<AnyView>`, `Window`'s
/// `content: AnyView`); some containers want a *concrete* child type instead (`MenuBar`'s
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
            plan_element(elem, &closure_ctx, &mut plan, true);
            let mut construct = TokenStream::new();
            for planned in &plan {
                emit_construction(planned, &closure_ctx, from, table, &mut construct);
            }
            let root_binding = &plan.last().expect("closure element body must have a root").binding;
            let converted = into_any_view_if_needed(quote! { #root_binding }, "AnyView");
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

/// The only construction mechanism left: resolve `node.type_path` via `SymbolTable` (every
/// resolved type — a plain user component, a component-with-view, or a builtin shape backed by
/// hand-written Rust in `elwindui-builtins` — is treated identically) and call `Type::new(args)`,
/// `args` built from `info.param_fields` in declaration order:
/// - a param named `children` is filled from the element's bare nested children (a `Vec`,
///   `AnyView`-converted per element only if the declared type says `AnyView`);
/// - a `ViewExpr::Element`-valued attribute (`menu_bar: MenuBar { .. }`) is filled from its own
///   already-planned/constructed binding (`element_attr_bindings`);
/// - a `ViewExpr::Closure`-valued attribute compiles to a real boxed closure (`emit_closure_value`);
/// - an `Option<..>`-typed param with no matching attribute becomes `None`;
/// - a required param with no matching attribute and no more specific rule falls back to the next
///   unclaimed bare child, positionally (`MenuBarItem`'s single nested `Menu`);
/// - anything else is an ordinary `emit_expr` value.
fn emit_construction(node: &PlannedNode, ctx: &ViewCtx, from: &Module, table: &SymbolTable, out: &mut TokenStream) {
    let binding = &node.binding;
    let info = table.resolve(from, &node.type_path).unwrap_or_else(|| {
        panic!("unknown or out-of-scope element `{}` — is a `use` for it missing?", node.type_path)
    });
    let type_ident = format_ident!("{}", node.type_path);

    let mut next_positional_child = 0usize;
    let mut args = Vec::new();
    for (name, ty) in &info.param_fields {
        if name == "children" {
            let items = node.child_bindings.iter().map(|c| into_any_view_if_needed(quote! { #c }, ty));
            args.push(quote! { vec![ #(#items),* ] });
            continue;
        }

        let (inner_ty, is_option) = strip_option(ty);
        let attr = find_attr(node, name);
        let value = match attr {
            Some(ViewExpr::Element(_)) => {
                let nested_binding = node
                    .element_attr_bindings
                    .get(name.as_str())
                    .unwrap_or_else(|| panic!("planned element binding for `{name}` must exist"));
                into_any_view_if_needed(quote! { #nested_binding }, inner_ty)
            }
            Some(ViewExpr::Closure { param, body }) => emit_closure_value(param, body, ctx, from, table),
            Some(other) => {
                let value = emit_expr(other, ctx, &EmitMode::Construction);
                // A `String`-shaped param takes `&str` in every hand-written builtin (matching
                // the shape declaration's `String`/`Option<String>` — see
                // `src/shapes/*.elwind` in `elwindui-builtins`), so the value is always wrapped in
                // `&(..)` here regardless of whether the DSL expression itself is a `&str`
                // literal or a computed `String` (e.g. `t!(...)`) — Rust's deref coercion accepts
                // either as `&str` at the call site, the same trick the old hardcoded
                // `emit_construction` arms already relied on for every builtin's string params.
                if inner_ty == "String" {
                    quote! { &(#value) }
                } else {
                    value
                }
            }
            None if is_option => {
                args.push(quote! { None });
                continue;
            }
            None if next_positional_child < node.child_bindings.len() => {
                let child = &node.child_bindings[next_positional_child];
                next_positional_child += 1;
                into_any_view_if_needed(quote! { #child }, inner_ty)
            }
            None => panic!("`{}` requires attribute `{name}`", node.type_path),
        };
        args.push(if is_option { quote! { Some(#value) } } else { value });
    }

    out.extend(quote! {
        let #binding = #type_ident::new(#(#args),*);
    });
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

    // The widget handle is cloned out to its own binding *before* `this` is cloned into the
    // closure: `this.#binding.set_on_click(Box::new(move || { ...this... }))` would try to
    // borrow `this` for the method receiver while also moving it into the same statement's
    // closure argument, which the borrow checker rejects.
    for (name, expr) in &node.attributes {
        if let Some(_event) = name.strip_prefix("on_") {
            let setter = format_ident!("set_{name}");
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

    for (name, expr) in &node.attributes {
        if name.starts_with("on_") {
            continue;
        }
        if matches!(expr, ViewExpr::Element(_) | ViewExpr::Closure { .. }) {
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
            // A bare reference to the closure's own bound parameter (e.g. `doc` in
            // `render_content: |doc| DocumentView { doc: doc }`) passes the value straight
            // through — it isn't a `vm`-style field with a generated getter, so it must be
            // handled before `resolve_bind`/`emit_path_get` (which has no 1-segment path shape).
            if let [only] = path.as_slice() {
                if ctx.closure_param.as_deref() == Some(only.as_str()) {
                    // The closure parameter itself is always a reference (`&Rc<T>`, `&_` —
                    // `emit_closure_value`'s deliberately-typed closure param), but a passthrough
                    // like `doc: doc` needs to hand an *owned* `Rc<T>` to the target constructor —
                    // `.clone()` is the cheap `Rc` refcount bump that bridges the two.
                    let ident = format_ident!("{}", only);
                    return quote! { #ident.clone() };
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

    /// Builtins (`Window`/`Column`/`TextArea`/etc.) only resolve when their shape modules
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

        Column {
            Row {
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

            Row {
                Text { text: t!("notepad-status-chars", count: vm.char_count) }
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
            tabs: vm.documents
            key: |doc| std::rc::Rc::as_ptr(doc) as usize
            render_label: |doc| doc.file_name
            render_content: |doc| TextArea { text: doc.content }
            selected: vm.active_tab
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
        assert!(window_str.contains("MenuBar :: new"));
        assert!(window_str.contains("MenuItem :: new"));
        assert!(window_str.contains("set_shortcut"));
        assert!(window_str.contains("TabView :: new"));
        // `TabView`'s per-tab chip/content materialization (`insert_tab`, `__weak_self`) is no
        // longer generated here at all — it's hand-written Rust inside `elwindui-builtins` now,
        // reached generically the same way any other resolved type's constructor is.
        assert!(!window_str.contains("insert_tab"));
        assert!(!window_str.contains("__weak_self"));
        assert!(window_str.contains("set_tabs"));
        assert!(window_str.contains("set_selected"));
    }

    #[test]
    fn generates_valid_rust_for_tabview_render_label_and_content() {
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
    Column {
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
            tabs: vm.documents
            key: |doc| std::rc::Rc::as_ptr(doc) as usize
            render_label: |doc| doc.file_name
            render_content: |doc| DocumentView { doc: doc }
            selected: vm.active_tab
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
        assert!(document_view_str.contains("fn into_any_view"));

        let window_code = generate_module(&window_module, &table);
        assert_valid_rust("tabview_render_content_window", &window_code);
        let window_str = window_code.to_string();
        assert!(window_str.contains("DocumentView :: new"));
        assert!(window_str.contains("into_any_view"));
        assert!(
            !window_str.contains("TextArea :: new (& __doc . content ())"),
            "the fixed TextArea fallback shouldn't be emitted once `render_content` is present"
        );
        // `render_label`'s body must go through the getter-call sugar (`.file_name()`), not a raw
        // field access — see the `parse_closure_expr_body` bug this test guards against.
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
    Text { text: greeter.name }
}
"#;
        let module = parse_module(src).expect("should parse");
        let table = build_symbol_table_with_builtins(std::slice::from_ref(&module));
        let generated = generate_module(&module, &table);
        assert_valid_rust("greeting_ctor", &generated);

        let s = generated.to_string();
        assert!(s.contains("fn new (greeter : Greeter)"), "expected ctor param named `greeter`, got:\n{s}");
        assert!(!s.contains("vm"), "ctor shouldn't hardcode a `vm` field name:\n{s}");
        // `Greeting`'s view root is `Text`, not `Window` — no top-level window to `show()`.
        assert!(!s.contains("fn show"));
        assert!(s.contains("fn into_any_view"));
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
}
