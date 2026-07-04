//! AST(検証済み) → backend別Rustソース。`target::backend()`の定数畳み込みは付録Dの通りCargo
//! featureでの静的分岐に落とし込み、`elwindui-core`のトレイト境界に対して書かれたコードを生成する
//! (今回はelwindui-backend-appkitのAPIを直接呼ぶ)。
//! 依存関係グラフに基づくCell/RefCellベースの更新関数生成は付録O.5に対応する。

use crate::ast::{
    Attr, ComponentDef, ElementNode, EnumDef, FieldKind, Initializer, Item, Module, ViewDef,
    ViewExpr, ViewModelDef,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

/// What every `component`/`viewmodel` in the whole compilation unit looks like, so that
/// cross-file references (e.g. `notepad_window.elwind`'s `vm.window_title` referring to a
/// `#[computed]` field defined in `notepad_viewmodel.elwind`) can be resolved.
pub struct SymbolTable {
    pub types: HashMap<String, TypeInfo>,
}

pub struct TypeInfo {
    pub fields: HashMap<String, FieldKind>,
    /// `component` fields defined as `bind!(owner.target, mode)`: `field_name -> (owner, target)`.
    /// Lets the view generator resolve the DSL's bare-field sugar (`content`) straight through to
    /// the field it's actually bound to (`vm.content`) without needing `self` to exist yet.
    pub binds: HashMap<String, (String, String)>,
}

pub fn build_symbol_table(modules: &[Module]) -> SymbolTable {
    let mut types = HashMap::new();
    for module in modules {
        for item in &module.items {
            let (name, fields) = match item {
                Item::Component(c) => (c.name.clone(), &c.fields),
                Item::ViewModel(v) => (v.name.clone(), &v.fields),
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
            types.insert(name, TypeInfo { fields: field_kinds, binds });
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

    let mut out = TokenStream::new();
    for item in &module.items {
        out.extend(match item {
            Item::Enum(e) => generate_enum(e),
            Item::ViewModel(v) => generate_viewmodel(v, table),
            Item::Component(c) if view_targets.contains(c.name.as_str()) => TokenStream::new(),
            Item::Component(c) => generate_component(c, table),
            Item::View(v) => generate_view(v, table),
        });
    }

    let has_async_command = module.items.iter().any(|item| {
        let fields = match item {
            Item::ViewModel(v) => &v.fields,
            Item::Component(c) => &c.fields,
            Item::Enum(_) | Item::View(_) => return false,
        };
        fields.iter().any(|f| {
            f.attrs
                .iter()
                .any(|a| matches!(a, Attr::CommandMeta { is_async: true, .. }))
        })
    });
    if has_async_command {
        out.extend(block_on_ready_helper());
    }

    out
}

/// A future that resolves on its very first `poll` (never returns `Pending`) completes here
/// without ever needing a real waker/executor — which covers `#[command(async)]` bodies that only
/// `.await` a modal dialog (docs/elwindui_spec.md 付録T.2), since AppKit's `runModal` is itself
/// synchronous. It is **not** a general-purpose async executor: a future that returns `Pending`
/// (e.g. real non-blocking I/O, a timer) will panic here. That needs `elwindui-core`'s planned
/// `Dispatcher`/`spawn` (docs/elwindui_gui_framework_design.md §7.3), which bridges to each
/// backend's actual event loop/async runtime — not yet implemented.
pub fn block_on_ready_helper() -> TokenStream {
    quote! {
        fn __elwindui_block_on_ready<F: std::future::Future>(fut: F) -> F::Output {
            use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
            fn noop(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                RawWaker::new(std::ptr::null(), &VTABLE)
            }
            static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
            let raw_waker = RawWaker::new(std::ptr::null(), &VTABLE);
            let waker = unsafe { Waker::from_raw(raw_waker) };
            let mut cx = Context::from_waker(&waker);
            let mut fut = Box::pin(fut);
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!(
                    "elwindui: #[command(async)] future did not resolve on its first poll \
                     (a real async executor/Dispatcher is not yet implemented)"
                ),
            }
        }
    }
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
fn nested_vec_item_type(ty: &str, table: &SymbolTable) -> Option<String> {
    let inner = ty.strip_prefix("Vec<")?.strip_suffix(">")?.trim();
    // The `.elwind`/`compile_dir` path builds one `SymbolTable` spanning every file, so a lookup
    // there is exact. The attribute-macro frontend (`attr_frontend.rs`) expands each
    // `#[elwindui::viewmodel] mod { ... }` in isolation — it has no way to see a *different* mod's
    // struct, so it always calls this with an empty table and relies entirely on the heuristic
    // below, same idea as `is_copy_type`'s "capitalized and not a known scalar" guess.
    let known = table.types.contains_key(inner);
    let looks_nested = inner.chars().next().is_some_and(|c| c.is_uppercase())
        && !matches!(inner, "String" | "Command");
    (known || looks_nested).then(|| inner.to_string())
}

pub fn generate_viewmodel(v: &ViewModelDef, table: &SymbolTable) -> TokenStream {
    let struct_name = format_ident!("{}", v.name);
    let field_names: HashSet<&str> = v.fields.iter().map(|f| f.name.as_str()).collect();

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
            FieldKind::Observable if nested_vec_item_type(&f.ty, table).is_some() => {
                let field_ident = format_ident!("{}", f.name);
                let item_ty: syn::Type = syn::parse_str(&nested_vec_item_type(&f.ty, table).unwrap())
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
                    }
                    pub fn #remover(&self, index: usize) {
                        self.#field_ident.borrow_mut().remove(index);
                        #(#recompute_calls)*
                    }
                });
            }
            FieldKind::Observable => {
                let field_ident = format_ident!("{}", f.name);
                let ty: syn::Type = syn::parse_str(&f.ty).expect("field type must parse");
                let init_expr = match &f.initializer {
                    Some(Initializer::Expr(e)) => {
                        rewrite_field_refs(coerce_to_owned_string(&f.ty, e.clone()), &field_names)
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
                    rewrite_field_refs(raw_expr.clone(), &field_names),
                    &field_names,
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
                    Some(expr) => rewrite_field_refs(expr.clone(), &field_names),
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
                let rewritten_block = rewrite_command_body(block.clone(), &field_names);
                let param_decls = params.iter().map(|(name, ty)| {
                    let ident = format_ident!("{}", name);
                    quote! { #ident: #ty }
                });
                if is_async {
                    // See docs/elwindui_spec.md 付録P.4. `block_on_ready` (emitted once per file,
                    // see `emit_block_on_ready_helper`) only actually supports futures that
                    // resolve on their first poll (e.g. a modal file dialog's `.await`, which
                    // never really suspends) — see that helper's doc comment for what a genuine
                    // non-blocking `#[command(async)]` still needs. `async move` (rather than
                    // plain `async`) so a parameterized command's argument is captured by value
                    // into the block, matching 付録O.4's parameterized-command extension.
                    accessors.extend(quote! {
                        pub fn #execute_ident(&self, #(#param_decls),*) {
                            __elwindui_block_on_ready(async move #rewritten_block);
                        }
                    });
                } else {
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
        }

        impl #struct_name {
            pub fn new() -> Self {
                let instance = Self { #ctor_fields };
                #recompute_calls_after_new
                instance
            }

            #accessors
        }

        impl Default for #struct_name {
            fn default() -> Self { Self::new() }
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
fn rewrite_field_refs(mut expr: syn::Expr, field_names: &HashSet<&str>) -> TokenStream {
    struct Rewriter<'a> {
        field_names: &'a HashSet<&'a str>,
    }
    impl<'a> VisitMut for Rewriter<'a> {
        fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
            if let syn::Expr::Path(p) = node {
                if let Some(ident) = p.path.get_ident() {
                    if self.field_names.contains(ident.to_string().as_str()) {
                        let call: syn::Expr = syn::parse_quote! { self.#ident() };
                        *node = call;
                        return;
                    }
                }
            }
            syn::visit_mut::visit_expr_mut(self, node);
        }
    }
    let mut rewriter = Rewriter { field_names };
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
fn rewrite_t_macro(expr: TokenStream, field_names: &HashSet<&str>) -> TokenStream {
    let expr: syn::Expr = syn::parse2(expr).expect("rewrite_field_refs always yields valid Expr");
    if let syn::Expr::Macro(m) = &expr {
        if m.mac.path.is_ident("t") {
            return rewrite_t_call(&m.mac.tokens, field_names);
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

fn rewrite_t_call(tokens: &TokenStream, field_names: &HashSet<&str>) -> TokenStream {
    // Tokens look like: "key", name1: expr1, name2: expr2
    let (key, args) = parse_t_macro_tokens(tokens)
        .expect("t!(...) arguments must be `\"key\", name: expr, ...`");
    let arg_pairs = args.iter().map(|(name, value)| {
        let name_str = name.to_string();
        let value = rewrite_field_refs(value.clone(), field_names);
        quote! { (#name_str, elwindui_i18n::FluentValue::from(#value)) }
    });
    quote! { elwindui_i18n::t(#key, &[ #(#arg_pairs),* ]) }
}

/// Rewrites a `command!(|| { ... })` body: assignments to a sibling field (`state = expr`) become
/// setter calls, bare reads of a sibling field become getter calls, and the whole thing becomes a
/// method body (`fn f(&self) { ... }`) rather than a closure.
fn rewrite_command_body(mut block: syn::Block, field_names: &HashSet<&str>) -> TokenStream {
    struct Rewriter<'a> {
        field_names: &'a HashSet<&'a str>,
    }
    impl<'a> VisitMut for Rewriter<'a> {
        fn visit_stmt_mut(&mut self, stmt: &mut syn::Stmt) {
            syn::visit_mut::visit_stmt_mut(self, stmt);
        }

        fn visit_expr_mut(&mut self, node: &mut syn::Expr) {
            if let syn::Expr::Assign(assign) = node {
                if let syn::Expr::Path(p) = assign.left.as_ref() {
                    if let Some(ident) = p.path.get_ident() {
                        if self.field_names.contains(ident.to_string().as_str()) {
                            let setter = format_ident!("set_{}", ident);
                            let mut value = (*assign.right).clone();
                            self.visit_expr_mut(&mut value);
                            *node = syn::parse_quote! { self.#setter(#value) };
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
                            *node = syn::parse_quote! { self.#helper(#args) };
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
                    let rewritten = rewrite_t_call(&m.mac.tokens, self.field_names);
                    *node = syn::parse2(rewritten).expect("rewrite_t_call always yields a valid Expr");
                    return;
                }
            }
            if let syn::Expr::Path(p) = node {
                if let Some(ident) = p.path.get_ident() {
                    if self.field_names.contains(ident.to_string().as_str()) {
                        *node = syn::parse_quote! { self.#ident() };
                        return;
                    }
                }
            }
            syn::visit_mut::visit_expr_mut(self, node);
        }
    }
    let mut rewriter = Rewriter { field_names };
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

fn generate_view(view: &ViewDef, table: &SymbolTable) -> TokenStream {
    let target_name = view.target.clone();
    let target = format_ident!("{}", target_name);
    let binds = table
        .types
        .get(&target_name)
        .map(|t| t.binds.clone())
        .unwrap_or_default();
    let ctx = ViewCtx { binds };

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
        emit_construction(node, &ctx, &mut construct_stmts);
        if node.stored {
            let binding = &node.binding;
            let ty = node.backend_type();
            struct_fields.extend(quote! { #binding: elwindui_backend_appkit::#ty, });
            field_inits.extend(quote! { #binding: #binding.clone(), });
        }
        // 付録Y: a `TabView` needs two extra bookkeeping fields beyond the widget handle itself —
        // see `emit_tabview_resync`'s doc comment for why (chip-rebuild tracking, and remembering
        // which tab is currently materialized as the content pane so typing doesn't rebuild it).
        if node.type_path == "TabView" {
            let binding = &node.binding;
            let chips_field = format_ident!("{}_chips", binding);
            let active_field = format_ident!("{}_active", binding);
            struct_fields.extend(quote! {
                #chips_field: std::cell::RefCell<Vec<elwindui_backend_appkit::TabChip>>,
                #active_field: std::cell::RefCell<Option<usize>>,
            });
            field_inits.extend(quote! {
                #chips_field: std::cell::RefCell::new(Vec::new()),
                #active_field: std::cell::RefCell::new(None),
            });
        }
    }
    for node in &plan {
        emit_wiring(node, &ctx, &mut wiring_stmts);
        emit_resync(node, &ctx, &mut resync_stmts);
    }

    // `plan_element` pushes children before their parent (post-order), so the root is always last.
    let root_binding = &plan.last().expect("view must have a root element").binding;

    // `resync()` takes `&self`, but `emit_tabview_resync` needs an `Rc<Self>` to hand fresh
    // per-tab closures a clone to call back into (`this.resync()`) — the standard
    // weak-self-reference pattern for self-referential callback wiring. Only wired up when a
    // `TabView` is actually present, so views without one are unaffected.
    let needs_weak_self = plan.iter().any(|n| n.type_path == "TabView");
    let (weak_self_field, weak_self_init, weak_self_set) = if needs_weak_self {
        (
            quote! { __weak_self: std::cell::RefCell<std::rc::Weak<Self>>, },
            quote! { __weak_self: std::cell::RefCell::new(std::rc::Weak::new()), },
            quote! { *this.__weak_self.borrow_mut() = std::rc::Rc::downgrade(&this); },
        )
    } else {
        (TokenStream::new(), TokenStream::new(), TokenStream::new())
    };

    quote! {
        impl #target {
            pub fn new(vm: NotepadViewModel) -> std::rc::Rc<Self> {
                #construct_stmts
                let this = std::rc::Rc::new(Self { vm, #field_inits #weak_self_init });
                #weak_self_set
                #wiring_stmts
                // Most widgets already read live model state at construction time (e.g.
                // `TextArea::new(&(vm.content()))`), so this is a no-op for them. `TabView` is the
                // exception — its actual tab widgets are only ever materialized in `resync()` (see
                // `emit_tabview_resync`), so without this call a `documents` list populated before
                // the window existed (as `main.rs` does, calling `new_tab_execute()` first) would
                // never appear until the first unrelated user interaction.
                this.resync();
                this
            }

            fn resync(&self) {
                #resync_stmts
            }

            pub fn open(self: std::rc::Rc<Self>) {
                self.#root_binding.clone().show_and_run();
            }
        }

        pub struct #target {
            vm: NotepadViewModel,
            #struct_fields
            #weak_self_field
        }
    }
}

struct ViewCtx {
    binds: HashMap<String, (String, String)>,
}

/// One element flattened out of the tree, in construction order (children before parents).
struct PlannedNode {
    binding: syn::Ident,
    type_path: String,
    attributes: Vec<(String, ViewExpr)>,
    child_bindings: Vec<syn::Ident>,
    /// Parallel to `child_bindings`: each child's `type_path`. Needed by `Window`'s construction
    /// to tell an optional `MenuBar` child apart from its one required content child (children
    /// are otherwise only reachable here as opaque bindings, having already been flattened out of
    /// the original tree).
    child_types: Vec<String>,
    /// Has a callback and/or a value that can change after construction, so it needs a struct
    /// field (rather than being a construction-time-only local).
    stored: bool,
}

impl PlannedNode {
    fn backend_type(&self) -> syn::Ident {
        format_ident!("{}", self.type_path)
    }
}

fn plan_element(node: &ElementNode, ctx: &ViewCtx, out: &mut Vec<PlannedNode>, is_root: bool) -> syn::Ident {
    let mut child_bindings = Vec::new();
    let mut child_types = Vec::new();
    for child in &node.children {
        child_bindings.push(plan_element(child, ctx, out, false));
        child_types.push(child.type_path.clone());
    }

    let binding = format_ident!("__{}_{}", node.type_path.to_lowercase(), out.len());
    // Every interactive/dynamic leaf is stored on `self` so callbacks and `resync()` can reach it
    // later; `Row`/`Column`/`MenuBar`/`MenuBarItem`/`Menu` are pure containers and never need to
    // be revisited after construction. `MenuItem` needs `on_select` wiring and `enabled` resync,
    // same as `Button`. `TabView` is dynamic (tabs come and go at runtime), so it's stored too.
    let stored = is_root
        || matches!(node.type_path.as_str(), "TextArea" | "Button" | "Text" | "MenuItem" | "TabView");

    out.push(PlannedNode {
        binding: binding.clone(),
        type_path: node.type_path.clone(),
        attributes: node.attributes.clone(),
        child_bindings,
        child_types,
        stored,
    });
    binding
}

fn find_attr<'a>(node: &'a PlannedNode, name: &str) -> Option<&'a ViewExpr> {
    node.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn emit_construction(node: &PlannedNode, ctx: &ViewCtx, out: &mut TokenStream) {
    let binding = &node.binding;
    let children = &node.child_bindings;
    match node.type_path.as_str() {
        "Window" => {
            let title = emit_expr(find_attr(node, "title").expect("Window requires `title`"), ctx, &EmitMode::Construction);
            // A `MenuBar` child (付録X) is optional and, unlike the rest of `Window`'s children,
            // isn't the content view — it's told apart from the one required content child by
            // type, since `children` only gives us opaque bindings at this point.
            let menu_bar_binding = node
                .child_types
                .iter()
                .position(|t| t == "MenuBar")
                .map(|i| &children[i]);
            let content_binding = node
                .child_types
                .iter()
                .position(|t| t != "MenuBar")
                .map(|i| &children[i])
                .expect("Window requires exactly one non-MenuBar content child");
            let set_menu_bar = menu_bar_binding.map(|mb| quote! { #binding.set_menu_bar(&#mb); });
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::Window::new(&(#title));
                #binding.set_content(#content_binding.clone().into());
                #set_menu_bar
            });
        }
        "Column" => out.extend(quote! {
            let #binding = elwindui_backend_appkit::Column::new(vec![ #(#children.clone().into()),* ]);
        }),
        "Row" => out.extend(quote! {
            let #binding = elwindui_backend_appkit::Row::new(vec![ #(#children.clone().into()),* ]);
        }),
        "TextArea" => {
            let text = emit_expr(find_attr(node, "text").expect("TextArea requires `text`"), ctx, &EmitMode::Construction);
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::TextArea::new(&(#text));
            });
        }
        "Button" => {
            let text = emit_expr(find_attr(node, "text").expect("Button requires `text`"), ctx, &EmitMode::Construction);
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::Button::new(&(#text));
            });
            if let Some(enabled_expr) = find_attr(node, "enabled") {
                let enabled = emit_expr(enabled_expr, ctx, &EmitMode::Construction);
                out.extend(quote! { #binding.set_enabled(#enabled); });
            }
        }
        "Text" => {
            let text = emit_expr(find_attr(node, "text").expect("Text requires `text`"), ctx, &EmitMode::Construction);
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::Text::new(&(#text));
            });
        }
        // 付録X: static structure (a fixed set of File/Edit/... items known at compile time), so
        // `MenuBar`/`MenuBarItem`/`Menu`/`MenuItem` go through the same construction/wiring/resync
        // pipeline as any other builtin rather than needing a specialized codegen path.
        "MenuBar" => out.extend(quote! {
            let #binding = elwindui_backend_appkit::MenuBar::new(vec![ #(#children.clone()),* ]);
        }),
        "MenuBarItem" => {
            let text = emit_expr(find_attr(node, "text").expect("MenuBarItem requires `text`"), ctx, &EmitMode::Construction);
            let submenu = &children[0];
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::MenuBarItem::new(&(#text), #submenu.clone());
            });
        }
        "Menu" => out.extend(quote! {
            let #binding = elwindui_backend_appkit::Menu::new(vec![ #(#children.clone()),* ]);
        }),
        "MenuItem" => {
            let text = emit_expr(find_attr(node, "text").expect("MenuItem requires `text`"), ctx, &EmitMode::Construction);
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::MenuItem::new(&(#text));
            });
            if let Some(shortcut_expr) = find_attr(node, "shortcut") {
                let shortcut = emit_expr(shortcut_expr, ctx, &EmitMode::Construction);
                out.extend(quote! { #binding.set_shortcut(&(#shortcut)); });
            }
            if let Some(enabled_expr) = find_attr(node, "enabled") {
                let enabled = emit_expr(enabled_expr, ctx, &EmitMode::Construction);
                out.extend(quote! { #binding.set_enabled(#enabled); });
            }
        }
        // 付録Y: dynamic (tabs come and go at runtime), constructed empty here; the observable
        // `tabs` list is materialized into actual per-tab widgets in `resync()` — see `emit_resync`.
        "TabView" => out.extend(quote! {
            let #binding = elwindui_backend_appkit::TabView::new();
        }),
        other => panic!("unknown builtin element `{other}`"),
    }
}

/// Attaches callbacks (`on_click`, two-way `text` binding) to widgets that were stored on `self`,
/// each capturing a fresh `Rc::clone` and calling `resync()` after mutating the model.
fn emit_wiring(node: &PlannedNode, ctx: &ViewCtx, out: &mut TokenStream) {
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { this });

    // The widget handle is cloned out to its own binding *before* `this` is cloned into the
    // closure: `this.#binding.set_on_click(Box::new(move || { ...this... }))` would try to
    // borrow `this` for the method receiver while also moving it into the same statement's
    // closure argument, which the borrow checker rejects.
    if let Some(on_click) = find_attr(node, "on_click") {
        let call = emit_expr(on_click, ctx, &self_mode);
        out.extend(quote! {
            {
                let widget = this.#binding.clone();
                let this = std::rc::Rc::clone(&this);
                widget.set_on_click(Box::new(move || {
                    #call;
                    this.resync();
                }));
            }
        });
    }

    if node.type_path == "TextArea" {
        if let Some(ViewExpr::Path(path)) = find_attr(node, "text") {
            let bound = resolve_bind(path, &ctx.binds);
            let setter = emit_setter(&bound, &self_mode);
            out.extend(quote! {
                {
                    let widget = this.#binding.clone();
                    let this = std::rc::Rc::clone(&this);
                    widget.set_on_change(Box::new(move |new_text: String| {
                        #setter(new_text);
                        this.resync();
                    }));
                }
            });
        }
    }

    // 付録X: `MenuItem`'s dropdown-select callback — same shape as `on_click`, different backend
    // method name (`set_on_select` vs `set_on_click`).
    if node.type_path == "MenuItem" {
        if let Some(on_select) = find_attr(node, "on_select") {
            let call = emit_expr(on_select, ctx, &self_mode);
            out.extend(quote! {
                {
                    let widget = this.#binding.clone();
                    let this = std::rc::Rc::clone(&this);
                    widget.set_on_select(Box::new(move || {
                        #call;
                        this.resync();
                    }));
                }
            });
        }
    }

    // 付録Y: the "+" button's callback. Per-tab select/close callbacks are wired individually as
    // each tab widget is created/destroyed in `emit_resync`'s `TabView` diffing, not here.
    if node.type_path == "TabView" {
        if let Some(on_new_tab) = find_attr(node, "on_new_tab") {
            let call = emit_expr(on_new_tab, ctx, &self_mode);
            out.extend(quote! {
                {
                    let widget = this.#binding.clone();
                    let this = std::rc::Rc::clone(&this);
                    widget.set_on_new_tab(Box::new(move || {
                        #call;
                        this.resync();
                    }));
                }
            });
        }
    }
}

/// Re-pushes every dynamic attribute of every stored widget from current model state. See the
/// "設計方針" note in docs/elwindui_gui_framework_design.md about deferring a full diffing
/// view-binding runtime; for notepad's small widget set a blanket resync is correct and simple.
fn emit_resync(node: &PlannedNode, ctx: &ViewCtx, out: &mut TokenStream) {
    if !node.stored {
        return;
    }
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { self });
    match node.type_path.as_str() {
        "Window" => {
            if let Some(title_expr) = find_attr(node, "title") {
                let value = emit_expr(title_expr, ctx, &self_mode);
                out.extend(quote! { self.#binding.set_title(&(#value)); });
            }
        }
        "TextArea" => {
            if let Some(text_expr) = find_attr(node, "text") {
                let value = emit_expr(text_expr, ctx, &self_mode);
                out.extend(quote! { self.#binding.set_text(&(#value)); });
            }
        }
        "Button" => {
            if let Some(enabled_expr) = find_attr(node, "enabled") {
                let value = emit_expr(enabled_expr, ctx, &self_mode);
                out.extend(quote! { self.#binding.set_enabled(#value); });
            }
        }
        "Text" => {
            if let Some(text_expr) = find_attr(node, "text") {
                let value = emit_expr(text_expr, ctx, &self_mode);
                out.extend(quote! { self.#binding.set_text(&(#value)); });
            }
        }
        "MenuItem" => {
            if let Some(enabled_expr) = find_attr(node, "enabled") {
                let value = emit_expr(enabled_expr, ctx, &self_mode);
                out.extend(quote! { self.#binding.set_enabled(#value); });
            }
        }
        "TabView" => emit_tabview_resync(node, ctx, out),
        _ => {}
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y.2's scope note: this is a specialized codegen path,
/// not a generalization of `emit_resync`'s normal one-attribute-at-a-time dispatch. Every resync:
///
/// 1. Fully rebuilds the tab chip strip from the current `tabs` list (chips hold no state worth
///    preserving across a rebuild — just a title and a close button — so a full rebuild each time
///    is simpler than incrementally diffing, and cheap at notepad's scale).
/// 2. Only swaps the content pane's `TextArea` when the *selected* tab actually changed. Doing
///    this unconditionally (like the chip strip) would destroy and recreate the native text view
///    on every keystroke — since typing itself triggers a resync — losing cursor position/focus
///    each time. `self.#binding_active_tab` remembers which tab is currently materialized so a
///    same-tab resync (the common case, triggered by typing) is a no-op here.
fn emit_tabview_resync(node: &PlannedNode, ctx: &ViewCtx, out: &mut TokenStream) {
    let binding = &node.binding;
    let self_mode = EmitMode::WithSelf(quote! { self });
    let chips_field = format_ident!("{}_chips", binding);
    let active_field = format_ident!("{}_active", binding);

    let tabs = emit_expr(find_attr(node, "tabs").expect("TabView requires `tabs`"), ctx, &self_mode);
    let selected = emit_expr(find_attr(node, "selected").expect("TabView requires `selected`"), ctx, &self_mode);

    // These two calls are emitted *inside* `Box::new(move || { ... })` closures below, which need
    // to be `'static` — they must go through `this` (the upgraded `Rc<Self>` moved into the
    // closure), not `self` (a `&self` borrow tied to this `resync()` call's own stack frame).
    let this_mode = EmitMode::WithSelf(quote! { this });
    let select_execute = command_execute_call(node, "on_select", ctx, &this_mode, quote! { __index });
    let close_execute = command_execute_call(node, "on_close", ctx, &this_mode, quote! { __index });

    out.extend(quote! {
        {
            let this = self.__weak_self.borrow().upgrade().expect("elwindui: component dropped while resyncing TabView");
            let __docs = #tabs;
            let __selected: usize = #selected;

            for __chip in self.#chips_field.borrow_mut().drain(..) {
                self.#binding.remove_tab(&__chip);
            }
            let mut __new_chips = Vec::new();
            for (__index, __doc) in __docs.iter().enumerate() {
                let __label = __doc.file_name();
                // Each callback gets its own `{ }` block so `let this = ...` shadows only within
                // it — both blocks clone the same *outer* `this`, rather than the second cloning
                // (and thus depending on the lifetime of) the first's already-moved-into-a-closure
                // copy.
                let __on_select: Box<dyn Fn()> = {
                    let this = std::rc::Rc::clone(&this);
                    Box::new(move || { #select_execute; this.resync(); })
                };
                let __on_close: Box<dyn Fn()> = {
                    let this = std::rc::Rc::clone(&this);
                    Box::new(move || { #close_execute; this.resync(); })
                };
                let __chip = self.#binding.insert_tab(__index, &__label, __on_select, __on_close);
                __new_chips.push(__chip);
            }
            *self.#chips_field.borrow_mut() = __new_chips;

            let already_showing = *self.#active_field.borrow() == Some(__selected);
            if !already_showing {
                if let Some(__doc) = __docs.get(__selected) {
                    let __text_area = elwindui_backend_appkit::TextArea::new(&__doc.content());
                    let __doc_for_change = std::rc::Rc::clone(__doc);
                    let __this_for_change = std::rc::Rc::clone(&this);
                    __text_area.set_on_change(Box::new(move |__new_text: String| {
                        __doc_for_change.set_content(__new_text);
                        __this_for_change.resync();
                    }));
                    self.#binding.set_content(elwindui_backend_appkit::AnyView::TextArea(__text_area));
                }
                *self.#active_field.borrow_mut() = Some(__selected);
            }
        }
    });
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
use elwindui::viewmodel::NotepadViewModel;

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
        let table = build_symbol_table(&[viewmodel_module.clone(), window_module.clone()]);

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
use elwindui::viewmodel::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: t!("notepad-window-title")

        MenuBar {
            MenuBarItem {
                text: t!("menu-file")
                Menu {
                    MenuItem { text: t!("menu-new"), shortcut: "n", on_select: vm.new_tab.execute() }
                }
            }
        }

        TabView {
            tabs: vm.documents
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
        let table = build_symbol_table(&[viewmodel_module.clone(), window_module.clone()]);

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
        assert!(window_str.contains("insert_tab"));
        assert!(window_str.contains("__weak_self"));
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
        assert!(generated_str.contains("__elwindui_block_on_ready"));
        assert!(generated_str.contains("async"));
        assert!(generated_str.contains("elwindui_i18n :: t"));
        assert!(!generated_str.contains("t !"), "t!(...) should have been rewritten, not left as a macro call");
    }
}
