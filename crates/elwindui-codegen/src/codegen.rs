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
            Item::ViewModel(v) => generate_viewmodel(v),
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
fn block_on_ready_helper() -> TokenStream {
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

fn generate_viewmodel(v: &ViewModelDef) -> TokenStream {
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

                let Some(Initializer::Command(block)) = &f.initializer else {
                    panic!("#[command] field `{}` needs a command!(...) initializer", f.name);
                };
                let execute_ident = format_ident!("{}_execute", f.name);
                let rewritten_block = rewrite_command_body(block.clone(), &field_names);
                if is_async {
                    // See docs/elwindui_spec.md 付録P.4. `block_on_ready` (emitted once per file,
                    // see `emit_block_on_ready_helper`) only actually supports futures that
                    // resolve on their first poll (e.g. a modal file dialog's `.await`, which
                    // never really suspends) — see that helper's doc comment for what a genuine
                    // non-blocking `#[command(async)]` still needs.
                    accessors.extend(quote! {
                        pub fn #execute_ident(&self) {
                            __elwindui_block_on_ready(async #rewritten_block);
                        }
                    });
                } else {
                    accessors.extend(quote! {
                        pub fn #execute_ident(&self) #rewritten_block
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
                    for (_, value) in &args {
                        self.visit_expr(value);
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
            Some(Initializer::Expr(_)) | Some(Initializer::Command(_)) => {
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
    }
    for node in &plan {
        emit_wiring(node, &ctx, &mut wiring_stmts);
        emit_resync(node, &ctx, &mut resync_stmts);
    }

    // `plan_element` pushes children before their parent (post-order), so the root is always last.
    let root_binding = &plan.last().expect("view must have a root element").binding;

    quote! {
        impl #target {
            pub fn new(vm: NotepadViewModel) -> std::rc::Rc<Self> {
                #construct_stmts
                let this = std::rc::Rc::new(Self { vm, #field_inits });
                #wiring_stmts
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
    for child in &node.children {
        child_bindings.push(plan_element(child, ctx, out, false));
    }

    let binding = format_ident!("__{}_{}", node.type_path.to_lowercase(), out.len());
    // Every interactive/dynamic leaf is stored on `self` so callbacks and `resync()` can reach it
    // later; `Row`/`Column` are pure containers and never need to be revisited after construction.
    let stored = is_root || matches!(node.type_path.as_str(), "TextArea" | "Button" | "Text");

    out.push(PlannedNode {
        binding: binding.clone(),
        type_path: node.type_path.clone(),
        attributes: node.attributes.clone(),
        child_bindings,
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
            let child = &children[0];
            out.extend(quote! {
                let #binding = elwindui_backend_appkit::Window::new(&(#title));
                #binding.set_content(#child.clone().into());
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
        _ => {}
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
