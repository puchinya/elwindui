pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod component_frontend;
pub mod environment_frontend;
pub mod parser;
mod rust_analyzer_shadow;
#[cfg(test)]
mod testdata;
mod text_style;
#[doc(hidden)]
pub use text_style::TEXT_STYLE_FIELDS;
pub mod theme_frontend;
pub mod validate;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::parse::Parser;

/// Stable compile-time token used by the generic `template_view!` property bridge.  This is a
/// code-generation key, not a runtime property lookup: the generated component implements the
/// corresponding `TemplateProperty<KEY>` instance and the standalone factory carries the same
/// literal key in its trait bound.
pub(crate) const fn template_property_key(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        index += 1;
    }
    hash
}

/// Test-only counterpart to the removed `builtin_modules()` — see `testdata`'s own doc comment.
/// `pub(crate)` (not `pub`): only `codegen.rs`/`validate.rs`/`component_frontend.rs`'s own
/// `#[cfg(test)] mod tests` blocks call this, never production code.
#[cfg(test)]
pub(crate) fn test_builtin_modules() -> Vec<ast::Module> {
    testdata::test_builtin_modules()
}

/// Test-only replacement for the old DSL text frontend's `parser::parse_module(text)` — builds one
/// `Module` from one or more new-syntax component definitions, each `(base, struct_src, impl_src)`:
/// `struct_src` is a full `struct Name { ..fields.., body: view! { .. } }` (no `#[elwindui::component]`
/// attribute — that macro attribute only ever contributes `base`/`inherits`, supplied here as the
/// first tuple element, same convention `dsl_enum_tests`/`component_impl_tests` below already use),
/// `impl_src` an optional full `impl Name { ..#[overridable]/#[overrides] fns.. }`. Reuses the exact
/// same production parsing `generate_component_from_item_struct`/`generate_component_from_item_impl`
/// call (`component_frontend::component_and_view_from_item_struct`/`methods_from_item_impl`/
/// `component_module_items`) — unlike those, doesn't touch the process-global same-crate sibling
/// registries (`component_frontend::register_same_crate_component`), so it stays a pure, side-effect-
/// free `Module` builder, exactly like `parser::parse_module` was. `is_builtin`/`allows_external_builtins`
/// both default to `false`, matching `parser::parse_module`'s own output (`Module::default()`) — an
/// unresolved reference in one of these fixtures stays a genuine test failure unless the test itself
/// also chains in `test_builtin_modules()` or another `test_module` call's items.
#[cfg(test)]
pub(crate) fn test_module(
    defs: &[(Option<&str>, &str, Option<&str>)],
) -> Result<ast::Module, String> {
    let mut items = Vec::new();
    for (base, struct_src, impl_src) in defs {
        let item_struct: syn::ItemStruct = syn::parse_str(struct_src)
            .map_err(|e| format!("test fixture struct failed to parse: {e}\n---\n{struct_src}"))?;
        let (mut component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(
                base.map(|b| b.to_string()),
                &item_struct,
            )?;
        if let Some(impl_src) = impl_src {
            let item_impl: syn::ItemImpl = syn::parse_str(impl_src)
                .map_err(|e| format!("test fixture impl failed to parse: {e}\n---\n{impl_src}"))?;
            let (_, methods) = component_frontend::methods_from_item_impl(&item_impl)?;
            component_def.methods = methods;
        }
        items.extend(component_frontend::component_module_items(
            component_def,
            view_def,
        ));
    }
    Ok(ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items,
        is_builtin: false,
        allows_external_builtins: false,
    })
}

/// Issue #146 test helper: a registry-dependent generation failure (a same-crate sibling registry
/// miss, a cross-item `validate::validate` rejection, ...) no longer returns `Err` from
/// `generate_component_from_item_struct`/`generate_component_from_item_impl` — it
/// returns `Ok` carrying a `#[cfg(not(rust_analyzer))]`-gated `compile_error!` alongside the
/// rust-analyzer shadow (`docs/design/tools/codegen_design.md` §3.2a), so a spurious same-crate
/// registry-ordering miss under rust-analyzer never blanks out that shadow. Every pre-existing test
/// asserting on one of these *registry-dependent* rejections goes through this helper instead of a
/// bare `.expect_err(..)`; an *item-local* rejection (a malformed `view!`, an untagged `impl` method,
/// ...) is unaffected and still returns a real `Err` this helper also accepts unchanged.
#[cfg(test)]
pub(crate) fn expect_generation_error(result: Result<proc_macro2::TokenStream, String>) -> String {
    match result {
        Err(error) => error,
        Ok(tokens) => {
            let s = tokens.to_string();
            assert!(
                s.contains("cfg (not (rust_analyzer))") && s.contains("compile_error !"),
                "expected either a hard Err or an Ok(..) carrying a `#[cfg(not(rust_analyzer))]`-gated \
                 compile_error! (Issue #146 dual expansion) — got: {s}"
            );
            s
        }
    }
}

/// The attribute-macro counterpart to `generate_component_from_item_struct`: takes a
/// `#[elwindui::viewmodel] mod foo { struct Foo { ... } impl Foo { ... } }` (already parsed as a `syn::ItemMod` by the
/// `elwindui-macros` proc-macro), builds the same `ViewModelDef` AST `parser.rs` would from
/// equivalent DSL text (see `attr_frontend`), and feeds it through `generate_module` (not
/// `generate_viewmodel` directly — `generate_module` is also what conditionally emits the
/// `__elwindui_block_on_ready` helper an async `#[command]` needs, and there's no reason to
/// duplicate that check here).
pub fn generate_viewmodel_from_item_mod(
    item_mod: &syn::ItemMod,
) -> Result<proc_macro2::TokenStream, String> {
    let def = attr_frontend::viewmodel_def_from_item_mod(item_mod)?;
    let name = def.name.clone();
    // A single macro invocation has no directory of sibling modules to cross-reference (`use`
    // resolution is moot with only one module), so the exact real path doesn't matter here — `[]`
    // (crate root) is as good as any.
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: vec![ast::Item::ViewModel(def)],
        ..Default::default()
    };
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_viewmodel(&name, item_mod);
    Ok(generated)
}

/// The attribute-macro counterpart to `generate_viewmodel_from_item_mod`, for
/// `#[elwindui::store] mod foo { struct Foo { ... } impl Foo { ... } }`. Same shape: no sibling
/// modules are chained in (a store never needs cross-referencing another DSL item to generate its
/// own fields — a store referencing another store does so as an ordinary Rust
/// `OtherStore::instance()` call, not DSL-resolved syntax), just `validate`/`build_symbol_table`/
/// `generate_module` over this one module, then registration for
/// `component_frontend::sibling_store_modules()` so a later same-crate component/viewmodel's
/// `TypeName.field` reference can be checked against it.
pub fn generate_store_from_item_mod(
    item_mod: &syn::ItemMod,
) -> Result<proc_macro2::TokenStream, String> {
    let def = attr_frontend::store_def_from_item_mod(item_mod)?;
    let name = def.name.clone();
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: vec![ast::Item::Store(def)],
        ..Default::default()
    };
    validate::validate(std::slice::from_ref(&module)).map_err(|errors| errors.join("\n"))?;
    let table = codegen::build_symbol_table(std::slice::from_ref(&module));
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_store(&name, item_mod);
    Ok(generated)
}

/// `#[elwindui::dsl_enum] enum Name { A, B, C }` — the opt-in that makes a plain Rust `enum`
/// visible to `validate::validate`'s `match`-exhaustiveness checking the same way the DSL's own
/// `enum Name { .. }` syntax always was (§14's user-enum rule; see
/// `component_frontend::same_crate_enums`'s own doc comment for why an opt-in is needed at all —
/// unlike a `#[elwindui::component]` struct or `#[elwindui::viewmodel]` mod, nothing about a bare
/// `enum` item marks it as DSL-relevant to any proc-macro). Transparent passthrough: the enum body
/// is emitted completely unchanged (it's real Rust, matched with real Rust `match`/`if let`) — the
/// only effect is registering it via `component_frontend::register_same_crate_enum` so a later
/// same-crate `#[elwindui::component]`'s `view!` can be checked against it. Same
/// declared-before-use ordering constraint as the component/viewmodel registries.
pub fn generate_dsl_enum_from_item_enum(
    item_enum: &syn::ItemEnum,
) -> Result<proc_macro2::TokenStream, String> {
    let name = item_enum.ident.to_string();
    // Built purely to validate the enum shape up front (bare unit variants only) — the real
    // `EnumDef` a `view!` elsewhere gets checked against is rebuilt fresh, from the registered
    // source text, by `sibling_enum_modules` itself.
    component_frontend::enum_def_from_item_enum(item_enum)?;
    component_frontend::register_same_crate_enum(&name, item_enum);
    Ok(quote::quote! { #item_enum })
}

/// The attribute-macro counterpart for `component`/`view` (the struct+`view!` frontend, successor
/// to the removed `elwindui::component!` bang macro): takes an already-parsed
/// `#[elwindui::component(inherits Base)] struct Name { ..fields.., body: view! { .. } }` (`base`
/// from the attribute's own `inherits Base` argument, `item_struct` parsed by the
/// `elwindui-macros` proc-macro) and builds the matching `ComponentDef`/`ViewDef` pair (see
/// `component_frontend`). Unlike `generate_viewmodel_from_item_mod`, this also chains in
/// `component_frontend::sibling_component_modules()`/`sibling_viewmodel_modules()`/
/// `sibling_enum_modules()`, so a `view!` can reference an *earlier* same-crate
/// `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` as a plain element
/// type, reactive field target, or `match`/`if let` subject (each attribute-macro invocation
/// otherwise only ever sees its own single annotated item — see
/// `component_frontend::same_crate_components`'s own doc comment for the full mechanism and its
/// declaration-order requirement). A `view!` body routinely references
/// `Window`/`VerticalLayout`/etc. too, but those resolve with no `Module` chained in for them at all —
/// see `testdata`'s own doc comment on why, and `codegen::emit_external_construction`.
/// Low-level expansion used by the public `template_view!` proc macro.
///
/// The standalone frontend deliberately only acquires the expected target type here.  Once that
/// type is known, the generated factory uses the same ControlTemplate lifecycle and the same
/// compile-time property/notification protocol as component and named templates; there is no
/// second runtime template representation or textual `templated_parent` detection.
pub fn generate_template_view_expression(body: &str) -> Result<TokenStream, String> {
    let (on_mount, on_unmount, on_update, lets, parsed_root) = parser::parse_view_body(body)
        .map_err(|error| format!("invalid `template_view! {{ ... }}` body: {error}"))?;
    let validation_view = ast::ViewDef {
        target: "__standalone_template_view".to_string(),
        is_template: true,
        template_instance: false,
        on_mount: on_mount.clone(),
        on_unmount: on_unmount.clone(),
        on_update: on_update.clone(),
        lets: lets.clone(),
        root: parsed_root.clone(),
        implicit_owner: None,
    };
    validate_replaceable_template_view(&validation_view)
        .map_err(|error| format!("invalid `template_view!` body: {error}"))?;

    let from = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: Vec::new(),
        is_builtin: false,
        allows_external_builtins: true,
    };
    let modules: Vec<_> = std::iter::once(from.clone())
        .chain(component_frontend::sibling_component_modules(
            "__standalone_template_view",
        ))
        .chain(component_frontend::sibling_viewmodel_modules())
        .chain(component_frontend::sibling_store_modules())
        .chain(component_frontend::sibling_enum_modules())
        .collect();
    let table = codegen::build_symbol_table(&modules);
    let compiled = compile_template_body(
        &parsed_root,
        &lets,
        on_mount.as_ref(),
        on_unmount.as_ref(),
        on_update.as_ref(),
        from,
        table,
        quote! { C },
        HashSet::new(),
    )?;
    let factory = emit_standalone_template_factory(&compiled);
    Ok(quote! {{ #factory }})
}

/// Resolves the exported property-shape macro for one template element using the same module
/// metadata boundary as normal View generation.  Builtins (and unresolved external framework
/// types) are exported by `elwindui::core`; a user component in the current crate is exported at
/// `crate::`, while a qualified external component keeps its crate prefix.  This is deliberately
/// metadata/path based rather than a list of concrete widget names.
fn template_props_macro_path(type_path: &str, info: Option<&codegen::TypeInfo>) -> TokenStream {
    let ident = type_path
        .rsplit("::")
        .next()
        .map(|name| format_ident!("__elwindui_props_{name}"))
        .expect("template element path has a type name");
    if info.is_some_and(|info| info.is_builtin) || type_path.starts_with("elwindui::") {
        return quote! { elwindui::core::#ident };
    }
    if info.is_some() || type_path.starts_with("crate::") {
        quote! { crate::#ident }
    } else if !type_path.contains("::") {
        // An unresolved bare name is the established spelling for a framework builtin imported
        // through `elwindui::ui::*`; user components with metadata are resolved above, while
        // qualified external components retain their crate prefix below.
        quote! { elwindui::core::#ident }
    } else if let Some(prefix) = type_path.split("::").next() {
        let prefix = format_ident!("{prefix}");
        quote! { #prefix::#ident }
    } else {
        quote! { crate::#ident }
    }
}

static TEMPLATE_VIEW_FACTORY_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Shared backend for the `template_view!` ControlTemplate expression frontend.
///
/// Component defaults and named templates enter the same parser/validator and the generated
/// component path uses the same typed parent, dynamic-region, ContentPresenter, and Environment
/// contracts. This backend owns the expression-form factory lowering used when the target type is
/// acquired from an expected `ControlTemplate<C>` value.
struct TemplateBackend {
    property_bounds: Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
    iterable_properties: BTreeSet<u64>,
    next_binding: usize,
    parent_ident: String,
    from: ast::Module,
    table: codegen::SymbolTable,
    target_type: TokenStream,
    /// Legacy/ordinary component-default templates may use a bare own-property reference (for
    /// example `TextBlock { text: label }`).  The semantic backend normalizes those names to the
    /// same typed `templated_parent.label` path used by explicit template expressions.  The set is
    /// supplied by the frontend that knows the concrete target; standalone/named templates leave
    /// it empty and therefore require the explicit typed parent spelling.
    bare_parent_fields: HashSet<String>,
    /// `let` bindings are constructed once in source order and then referenced by later
    /// elements.  Keeping these bindings in the semantic backend makes template_view! follow the
    /// same lexical reference rule as ordinary view! lowering instead of treating references as a
    /// standalone-only error.
    lets: std::collections::HashMap<String, (syn::Ident, String)>,
    /// Lexical bindings introduced by a dynamic `for` region.  Keeping these in the shared
    /// backend lets constructor/property expressions inside the loop use the same path lowering
    /// as ordinary template expressions (for example `Child { vm: item }`).
    loop_bindings: Vec<(String, syn::Ident)>,
    /// Whether this body contains a deferred ViewTemplate expression.  Such a body must use the
    /// typed-parent factory shell even when no ordinary property expression appears, because the
    /// deferred factory captures the parent identity at the point it is authored.
    has_deferred_views: bool,
}

fn is_environment_scope_type(type_path: &str) -> bool {
    type_path.rsplit("::").next() == Some("EnvironmentScope")
}

impl Default for TemplateBackend {
    fn default() -> Self {
        Self {
            property_bounds: Rc::new(RefCell::new(BTreeMap::new())),
            iterable_properties: BTreeSet::new(),
            next_binding: 0,
            parent_ident: "__elwindui_template_parent".to_string(),
            from: ast::Module::default(),
            table: codegen::build_symbol_table(&[]),
            target_type: quote! { C },
            bare_parent_fields: HashSet::new(),
            lets: std::collections::HashMap::new(),
            loop_bindings: Vec::new(),
            has_deferred_views: false,
        }
    }
}

impl TemplateBackend {
    fn new(
        from: ast::Module,
        table: codegen::SymbolTable,
        target_type: TokenStream,
        bare_parent_fields: HashSet<String>,
    ) -> Self {
        Self {
            property_bounds: Rc::new(RefCell::new(BTreeMap::new())),
            iterable_properties: BTreeSet::new(),
            next_binding: 0,
            parent_ident: "__elwindui_template_parent".to_string(),
            from,
            table,
            target_type,
            bare_parent_fields,
            lets: std::collections::HashMap::new(),
            loop_bindings: Vec::new(),
            has_deferred_views: false,
        }
    }

    /// Resolve a template element through the ordinary lexical symbol table first.  The
    /// expression-form `template_view!` frontend is parsed outside a user module, so its
    /// unqualified same-crate component names have no `Module::path`/`use` context; in that one
    /// frontend-only case, accept a unique user-defined metadata entry.  Builtins are deliberately
    /// excluded by `resolve_unqualified`, so this remains metadata-driven rather than a list of
    /// framework type names.
    fn resolve_info(&self, type_path: &str) -> Option<&codegen::TypeInfo> {
        self.table.resolve(&self.from, type_path).or_else(|| {
            (!type_path.contains("::"))
                .then(|| self.table.resolve_unqualified(type_path))
                .flatten()
        })
    }

    fn loop_binding(&self, name: &str) -> Option<&syn::Ident> {
        self.loop_bindings
            .iter()
            .rev()
            .find_map(|(binding, ident)| (binding == name).then_some(ident))
    }

    /// Returns the declaration-driven semantic-brush capability for one template property.
    ///
    /// Local metadata can answer this at compile time.  For a builtin or external class whose
    /// declaration is represented only by its exported props macro, keep the query in the emitted
    /// tokens so that the same declaration metadata remains authoritative across crates.
    fn semantic_brush_query(&self, type_path: &str, name: &str) -> TokenStream {
        if let Some(info) = self.resolve_info(type_path) {
            let value = info.semantic_brush_fields.contains(name);
            quote! { #value }
        } else {
            let props_macro = template_props_macro_path(type_path, None);
            let name = format_ident!("{name}");
            quote! { #props_macro!(@is_semantic_brush #name) }
        }
    }

    /// Detects whether an expression needs the typed template parent while it is re-evaluated by
    /// an Environment/theme subscription.  Parent-independent standalone factories intentionally
    /// do not bind a parent variable, so this distinction keeps the shared semantic backend valid
    /// for both factory shells without introducing a standalone-only compiler branch.
    fn expression_uses_template_parent(&self, expr: &ast::ViewExpr) -> bool {
        match expr {
            ast::ViewExpr::Path(path) => path.first().is_some_and(|name| {
                name == "templated_parent" || self.bare_parent_fields.contains(name)
            }),
            ast::ViewExpr::Expr(expression) => {
                struct ParentPathVisitor<'a> {
                    fields: &'a HashSet<String>,
                    found: bool,
                }
                impl<'ast> syn::visit::Visit<'ast> for ParentPathVisitor<'_> {
                    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                        if let Some(first) = node.path.segments.first() {
                            let name = first.ident.to_string();
                            if name == "templated_parent" || self.fields.contains(&name) {
                                self.found = true;
                            }
                        }
                        syn::visit::visit_expr_path(self, node);
                    }
                }
                let mut visitor = ParentPathVisitor {
                    fields: &self.bare_parent_fields,
                    found: false,
                };
                syn::visit::Visit::visit_expr(&mut visitor, expression);
                visitor.found
            }
            ast::ViewExpr::TFluent(_, args) => args
                .iter()
                .any(|(_, value)| self.expression_uses_template_parent(value)),
            ast::ViewExpr::Element(element) => {
                element
                    .attributes
                    .iter()
                    .any(|attribute| self.expression_uses_template_parent(&attribute.value))
                    || element
                        .attached
                        .iter()
                        .any(|(_, _, value)| self.expression_uses_template_parent(value))
            }
            ast::ViewExpr::Closure { .. } | ast::ViewExpr::DeferredView(_) => false,
        }
    }

    /// Whether an unresolved/external property can safely be repeated from the semantic-brush
    /// listener.  Builtin props metadata is available only through the exported props macro, so we
    /// cannot decide its semantic-brush bit until the generated code is compiled.  Restrict that
    /// deferred query to expressions whose repeated closure does not move an arbitrary caller
    /// capture out of an enclosing `Fn` factory: typed parent paths and qualified constant paths.
    /// Local metadata remains authoritative and does not need this conservative fallback.
    fn expression_safe_for_semantic_subscription(&self, expr: &ast::ViewExpr) -> bool {
        match expr {
            ast::ViewExpr::Path(path) => path.first().is_some_and(|first| {
                self.loop_binding(first).is_none()
                    && (first == "templated_parent"
                        || self.bare_parent_fields.contains(first)
                        || path.len() > 1)
            }),
            ast::ViewExpr::TFluent(_, args) => args
                .iter()
                .all(|(_, value)| self.expression_safe_for_semantic_subscription(value)),
            ast::ViewExpr::Element(_)
            | ast::ViewExpr::Expr(_)
            | ast::ViewExpr::Closure { .. }
            | ast::ViewExpr::DeferredView(_) => false,
        }
    }

    /// Emits the common Environment/theme subscription for a semantic-brush property.
    ///
    /// The assignment is kept inside one reusable closure rather than spelling the value
    /// expression once for construction and once for the theme callback.  This is important for
    /// arbitrary template expressions: a captured value such as `format!("{}", captured)` may be
    /// borrowed repeatedly by an `Fn` closure, but duplicating the expression into two independent
    /// `move` closures can make the compiler treat the capture as moved twice.  The reusable
    /// closure also gives default, named, and standalone templates one identical semantic-brush
    /// path.
    fn semantic_brush_subscription(
        &mut self,
        node: &syn::Ident,
        type_path: &str,
        props_macro: &TokenStream,
        name: &syn::Ident,
        expr: &ast::ViewExpr,
        value: &TokenStream,
        sink: &SubscriptionSink,
    ) -> TokenStream {
        let query = self.semantic_brush_query(type_path, &name.to_string());
        let parent_dependent = self.expression_uses_template_parent(expr);
        let node_weak = self.fresh("semantic_brush_node");
        let environment = self.fresh("semantic_brush_environment");
        let apply_environment = self.fresh("semantic_brush_apply_environment");
        let apply = self.fresh("semantic_brush_apply");
        let listener = self.fresh("semantic_brush_listener");
        let subscriptions = self.fresh("semantic_brush_subscriptions");
        let parent = format_ident!("{}", self.parent_ident);
        let (parent_capture, apply_body) = if parent_dependent {
            let parent_weak = self.fresh("semantic_brush_parent");
            let callback_node = self.fresh("semantic_brush_callback_node");
            (
                quote! { let #parent_weak = std::rc::Rc::downgrade(&#parent); },
                quote! {
                    if let (Some(#callback_node), Some(#parent)) =
                        (#node_weak.upgrade(), #parent_weak.upgrade())
                    {
                        let __environment = #apply_environment.clone();
                        #props_macro!(@set_with_environment #callback_node, #name, #value, &__environment);
                    }
                },
            )
        } else {
            let callback_node = self.fresh("semantic_brush_callback_node");
            (
                TokenStream::new(),
                quote! {
                    if let Some(#callback_node) = #node_weak.upgrade() {
                        let __environment = #apply_environment.clone();
                        #props_macro!(@set_with_environment #callback_node, #name, #value, &__environment);
                    }
                },
            )
        };
        let extend = match sink {
            SubscriptionSink::Shared(storage) => {
                quote! { #storage.borrow_mut().extend(#subscriptions); }
            }
            SubscriptionSink::Local(storage) => quote! { #storage.extend(#subscriptions); },
        };
        quote! {
            {
                let #node_weak = std::rc::Rc::downgrade(&#node);
                let #environment = __environment.clone();
                let #apply_environment = #environment.clone();
                #parent_capture
                let #apply: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
                    #apply_body
                });
                #apply();
                if #query {
                let #listener: std::rc::Rc<dyn Fn()> = {
                    let #apply = #apply.clone();
                    std::rc::Rc::new(move || #apply())
                };
                let #subscriptions = elwindui::core::theme::subscribe_semantic_brushes(
                    &#environment,
                    #listener,
                );
                #extend
                }
            }
        }
    }

    fn user_component_args(
        &mut self,
        element: &ast::ElementNode,
        param_fields: &[(String, String)],
        content_field: Option<&str>,
        children: &[(TokenStream, String)],
    ) -> Result<Vec<TokenStream>, String> {
        let mut args = Vec::new();
        for (name, ty) in param_fields {
            if name == "children" {
                let values = children.iter().map(|(value, child_ty)| {
                    if ty.contains("dyn UIElement") {
                        if self
                            .resolve_info(child_ty)
                            .is_some_and(|info| !info.is_native && !info.is_virtual_builtin)
                        {
                            quote! { #value.clone().into_node() }
                        } else {
                            quote! { #value.clone() }
                        }
                    } else {
                        quote! { #value.clone() }
                    }
                });
                args.push(quote! { vec![#(#values),*] });
                continue;
            }

            let attr = element.attributes.iter().find(|attr| attr.name == *name);
            let value = if let Some(attr) = attr {
                match &attr.value {
                    ast::ViewExpr::Element(child) => {
                        let value = self.compile_element(
                            child,
                            &SubscriptionSink::Shared(quote! { __subscriptions }),
                        )?;
                        quote! { #value }
                    }
                    ast::ViewExpr::Closure { params, body } => {
                        let parent = format_ident!("{}", self.parent_ident);
                        codegen::emit_template_closure_value_for_target_with_fields(
                            params,
                            body,
                            &parent,
                            &self.property_bounds,
                            &self.from,
                            &self.table,
                            self.target_type.clone(),
                            self.bare_parent_fields.clone(),
                        )
                    }
                    expr => self.expression(expr)?,
                }
            } else if content_field == Some(name.as_str()) && children.len() == 1 {
                let (value, child_ty) = &children[0];
                if ty.contains("dyn UIElement")
                    && self
                        .resolve_info(child_ty)
                        .is_some_and(|info| !info.is_native && !info.is_virtual_builtin)
                {
                    quote! { #value.clone().into_node() }
                } else {
                    quote! { #value.clone() }
                }
            } else if ty.trim_start().starts_with("Option<") {
                quote! { None }
            } else {
                return Err(format!(
                    "template element `{}` is missing required property `{name}`",
                    element.type_path
                ));
            };
            let value = if ty.trim() == "String" {
                quote! { (#value).to_string() }
            } else if ty.trim_start().starts_with("Option<")
                && !matches!(attr.map(|a| &a.value), Some(ast::ViewExpr::Path(path)) if path.first().is_some_and(|p| p == name))
                && !(content_field == Some(name.as_str()) && children.len() == 1)
            {
                // Most generated component fields retain their declared Option shape in the
                // constructor.  A literal supplied to an optional slot is therefore wrapped at
                // this boundary; an explicit path already carrying the option is left untouched.
                quote! { Some(#value) }
            } else {
                value
            };
            args.push(value);
        }
        Ok(args)
    }

    /// Emits the write-back endpoint for a `<=>` source path.  The read side of a template
    /// binding is lowered by [`Self::expression`]; keeping the setter construction here means
    /// standalone, named, and component-default templates all use the same typed
    /// `TemplateProperty` bridge (and the same ordinary getter/setter chain for captured values).
    /// A path whose final owner is not writable is rejected by the generated property surface in
    /// exactly the same way as an ordinary View binding; returning `None` here is reserved for a
    /// malformed/non-path expression, which the shared validator reports before code generation.
    fn two_way_setter(
        &mut self,
        expr: &ast::ViewExpr,
        parent_ident: &syn::Ident,
    ) -> Result<Option<TokenStream>, String> {
        let ast::ViewExpr::Path(raw_path) = expr else {
            return Ok(None);
        };
        let path = if raw_path
            .first()
            .is_some_and(|name| self.bare_parent_fields.contains(name))
        {
            std::iter::once("templated_parent".to_string())
                .chain(raw_path.iter().cloned())
                .collect::<Vec<_>>()
        } else {
            raw_path.clone()
        };
        if path.len() < 2 {
            return Ok(None);
        }
        let setter_name = path.last().expect("two-way path has a final field");
        let setter = format_ident!("set_{setter_name}");
        if path[0] == "templated_parent" {
            if path.len() == 2 {
                let key = crate::template_property_key(&path[1]);
                self.property_bounds.borrow_mut().entry(key).or_insert(None);
                let target = &self.target_type;
                return Ok(Some(quote! {
                    <#target as elwindui::core::ui::TemplateProperty<#key>>::__template_set(
                        &*#parent_ident,
                        new_value,
                    );
                }));
            }

            let first = &path[1];
            let key = crate::template_property_key(first);
            self.property_bounds.borrow_mut().entry(key).or_insert(None);
            let target = &self.target_type;
            let mut receiver = quote! {
                <#target as elwindui::core::ui::TemplateProperty<#key>>::__template_get(&*#parent_ident)
            };
            for segment in &path[2..path.len() - 1] {
                let getter = format_ident!("{segment}");
                receiver = quote! { #receiver.#getter() };
            }
            return Ok(Some(quote! { #receiver.#setter(new_value); }));
        }

        let owner = format_ident!("{}", path[0]);
        let mut receiver = quote! { #owner };
        for segment in &path[1..path.len() - 1] {
            let getter = format_ident!("{segment}");
            receiver = quote! { #receiver.#getter() };
        }
        Ok(Some(quote! { #receiver.#setter(new_value); }))
    }
}

enum SubscriptionSink {
    Shared(TokenStream),
    Local(syn::Ident),
}

impl SubscriptionSink {
    fn push(&self, subscription: TokenStream) -> TokenStream {
        match self {
            Self::Shared(storage) => {
                quote! { #storage.borrow_mut().push(#subscription); }
            }
            Self::Local(storage) => quote! { #storage.push(#subscription); },
        }
    }
}

impl TemplateBackend {
    fn fresh(&mut self, prefix: &str) -> syn::Ident {
        let ident = format_ident!("__elwindui_{prefix}_{}", self.next_binding);
        self.next_binding += 1;
        ident
    }

    fn compile_root(&mut self, body: &ast::ViewBody) -> Result<TokenStream, String> {
        if body.children.len() != 1 {
            return Err("`template_view!` requires exactly one effective root".into());
        }
        let root = match &body.children[0] {
            ast::ChildEntry::Literal(element) if is_environment_scope_type(&element.type_path) => {
                let mut roots = self.compile_environment_scope_children(
                    element,
                    &SubscriptionSink::Shared(quote! { __subscriptions }),
                )?;
                if roots.len() != 1 {
                    return Err(
                        "an EnvironmentScope used as a template root must contain exactly one child"
                            .into(),
                    );
                }
                Ok(roots.remove(0))
            }
            ast::ChildEntry::Literal(element) => {
                // The ordinary View grammar permits properties/attached properties/shortcuts on
                // the body itself as shorthand for the sole root element.  Fold those entries
                // into a cloned root before handing it to the common element backend; rejecting
                // them here made the expression frontend a narrower, standalone-only dialect.
                let mut root = element.clone();
                root.attributes.extend(body.attributes.iter().cloned());
                root.attached.extend(body.attached.iter().cloned());
                root.attribute_shortcuts
                    .extend(body.attribute_shortcuts.iter().cloned());
                self.compile_element(&root, &SubscriptionSink::Shared(quote! { __subscriptions }))
            }
            ast::ChildEntry::If { .. } | ast::ChildEntry::Match { .. } => {
                if !body.attributes.is_empty()
                    || !body.attached.is_empty()
                    || !body.attribute_shortcuts.is_empty()
                {
                    return Err(
                        "root properties require a static element; dynamic roots cannot receive body attributes"
                            .into(),
                    );
                }
                self.compile_dynamic_root(&body.children[0])
            }
            ast::ChildEntry::For { .. } => {
                if !body.attributes.is_empty()
                    || !body.attached.is_empty()
                    || !body.attribute_shortcuts.is_empty()
                {
                    return Err(
                        "root properties require a static element; a `for` region cannot be the sole ControlTemplate root"
                            .into(),
                    );
                }
                Err("a `for` region cannot be the sole ControlTemplate root".into())
            }
            ast::ChildEntry::Ref(name) => {
                if !body.attributes.is_empty()
                    || !body.attached.is_empty()
                    || !body.attribute_shortcuts.is_empty()
                {
                    return Err(
                        "root properties require a static element; a reference cannot be the sole ControlTemplate root"
                            .into(),
                    );
                }
                let Some((binding, _)) = self.lets.get(name) else {
                    return Err(format!("template root reference `{name}` is not defined"));
                };
                Ok(quote! { #binding.clone() })
            }
        }?;
        Ok(root)
    }

    fn compile_dynamic_root(&mut self, entry: &ast::ChildEntry) -> Result<TokenStream, String> {
        let (selector, branches) = match entry {
            ast::ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            } => (
                Some(condition),
                vec![then_branch.as_slice(), else_branch.as_slice()],
            ),
            ast::ChildEntry::Match { value, arms } => {
                let branches = arms.iter().map(|arm| arm.body.as_slice()).collect();
                (Some(value), branches)
            }
            _ => return Err("expected a dynamic template root".into()),
        };
        let selector = selector.expect("dynamic root always has a selector");
        let initial_parent_ident = self.fresh("initial_parent");
        let initial_previous_parent_ident =
            std::mem::replace(&mut self.parent_ident, initial_parent_ident.to_string());
        let selector_tokens = self.expression(selector)?;
        let selector_keys = self.expression_property_keys(selector);
        let boolean_match = matches!(
            entry,
            ast::ChildEntry::Match { arms, .. }
                if arms.iter().all(|arm| matches!(arm.pattern.trim(), "true" | "false"))
        );
        if matches!(entry, ast::ChildEntry::If { .. }) || boolean_match {
            for key in &selector_keys {
                self.constrain_property(*key, quote! { bool });
            }
        }
        let branch_vec = self.fresh("branch_subscriptions");
        let branch_exprs: Vec<_> = branches
            .iter()
            .map(|branch| {
                if branch.len() != 1 {
                    return Err(
                        "every dynamic ControlTemplate root branch must contain exactly one element"
                            .to_string(),
                    );
                }
                self.compile_dynamic_branch(
                    &branch[0],
                    &SubscriptionSink::Local(branch_vec.clone()),
                )
            })
            .collect::<Result<_, _>>()?;
        self.parent_ident = initial_previous_parent_ident;
        let callback_parent_ident = self.fresh("callback_parent");
        let callback_branch_vec = self.fresh("next_branch_subscriptions");
        let previous_parent_ident =
            std::mem::replace(&mut self.parent_ident, callback_parent_ident.to_string());
        let callback_selector_tokens = self.expression(selector)?;
        let callback_branch_exprs: Vec<_> = branches
            .iter()
            .map(|branch| {
                self.compile_dynamic_branch(
                    &branch[0],
                    &SubscriptionSink::Local(callback_branch_vec.clone()),
                )
            })
            .collect::<Result<_, _>>()?;
        self.parent_ident = previous_parent_ident;
        let initial_root = self.fresh("initial_root");
        let branch_state = self.fresh("branch_state");
        let callback_branch_state = self.fresh("callback_branch_state");
        let control_weak = self.fresh("control_weak");
        let subscription_parent = self.fresh("subscription_parent");
        let callback_root = self.fresh("next_root");
        let root_parent_ident = format_ident!("{}", self.parent_ident);
        let condition = match entry {
            ast::ChildEntry::If { .. } => {
                let then_expr = &branch_exprs[0];
                let else_expr = &branch_exprs[1];
                quote! { if #selector_tokens { #then_expr } else { #else_expr } }
            }
            ast::ChildEntry::Match { arms, .. } => {
                let arms = arms
                    .iter()
                    .zip(branch_exprs.iter())
                    .map(|(arm, expr)| {
                        let pattern = syn::Pat::parse_single
                            .parse_str(&arm.pattern)
                            .map_err(|error| format!("invalid template match pattern: {error}"))?;
                        Ok::<_, String>(quote! { #pattern => #expr })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                quote! { match #selector_tokens { #(#arms),* } }
            }
            _ => unreachable!(),
        };
        let callback_condition = match entry {
            ast::ChildEntry::If { .. } => {
                let then_expr = &callback_branch_exprs[0];
                let else_expr = &callback_branch_exprs[1];
                quote! { if #callback_selector_tokens { #then_expr } else { #else_expr } }
            }
            ast::ChildEntry::Match { arms, .. } => {
                let arms = arms
                    .iter()
                    .zip(callback_branch_exprs.iter())
                    .map(|(arm, expr)| {
                        let pattern = syn::Pat::parse_single
                            .parse_str(&arm.pattern)
                            .map_err(|error| format!("invalid template match pattern: {error}"))?;
                        Ok::<_, String>(quote! { #pattern => #expr })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                quote! { match #callback_selector_tokens { #(#arms),* } }
            }
            _ => unreachable!(),
        };
        let subscription = if selector_keys.is_empty() {
            quote! { vec![elwindui::core::reactive::Subscription::new(|| {})] }
        } else {
            let subscriptions = selector_keys.iter().map(|key| {
                let property_receiver = self.template_property_receiver(*key);
                let callback_environment = self.fresh("callback_environment");
                let selector_subscription = quote! {
                    #property_receiver::__template_subscribe(
                        &*#subscription_parent,
                        move || {
                            if let Some(control) = #control_weak.upgrade() {
                                let __environment = #callback_environment.clone();
                                #callback_branch_state.borrow_mut().clear();
                                let mut #callback_branch_vec = Vec::<elwindui::core::reactive::Subscription>::new();
                                let #callback_parent_ident = control.clone();
                                let #callback_root = #callback_condition;
                                *#callback_branch_state.borrow_mut() = #callback_branch_vec;
                                control.__set_template_root(#callback_root);
                            }
                        },
                    )
                };
                quote! {{
                    let #subscription_parent = #root_parent_ident.clone();
                    let #control_weak = std::rc::Rc::downgrade(&#subscription_parent);
                    let #callback_branch_state = #branch_state.clone();
                    let #callback_environment = __environment.clone();
                    #selector_subscription
                }}
            });
            quote! { vec![#(#subscriptions),*] }
        };
        let selector_block = quote! {
            let __selector_subscriptions = #subscription;
            __subscriptions.borrow_mut().extend(__selector_subscriptions);
        };
        let initial = quote! {
            let #initial_parent_ident = #root_parent_ident.clone();
            let mut #branch_vec = Vec::<elwindui::core::reactive::Subscription>::new();
            let #initial_root = #condition;
            let #branch_state = std::rc::Rc::new(std::cell::RefCell::new(#branch_vec));
            #selector_block
            let __branch_state_for_cleanup = #branch_state.clone();
            __subscriptions.borrow_mut().push(
                elwindui::core::reactive::Subscription::new(move || {
                    __branch_state_for_cleanup.borrow_mut().clear();
                }),
            );
            #initial_root
        };
        Ok(quote! {{
            use elwindui::core::ui::ControlExt as _;
            #initial
        }})
    }

    fn compile_dynamic_branch(
        &mut self,
        entry: &ast::ChildEntry,
        sink: &SubscriptionSink,
    ) -> Result<TokenStream, String> {
        match entry {
            ast::ChildEntry::Literal(element) => {
                let value = self.compile_element(element, sink)?;
                // Dynamic regions are stored in a `DynamicChildSlot<dyn UIElementExt>`.  Coerce
                // each branch at its own expression boundary so Rust never has to infer one
                // concrete `Rc<T>` type for an `if`/`match` whose arms intentionally contain
                // different generated components.
                Ok(codegen::into_node_if_needed(
                    value,
                    &element.type_path,
                    &self.from,
                    &self.table,
                ))
            }
            ast::ChildEntry::If { .. } | ast::ChildEntry::Match { .. } => {
                Err("nested dynamic roots require a static visual parent".into())
            }
            ast::ChildEntry::For { .. } => {
                Err("a `for` region cannot be a scalar template root branch".into())
            }
            ast::ChildEntry::Ref(name) => {
                let Some((binding, type_path)) = self.lets.get(name) else {
                    return Err(format!("template branch reference `{name}` is not defined"));
                };
                Ok(codegen::into_node_if_needed(
                    quote! { #binding.clone() },
                    type_path,
                    &self.from,
                    &self.table,
                ))
            }
        }
    }

    /// Compiles the ordered contents of one dynamic branch.  A branch may be empty (the common
    /// `if condition { child }` form has no `else` arm) or contain several literal children; the
    /// collection slot replaces the whole branch range atomically.  Each entry is still lowered
    /// by the same element backend, so constructor parameters, bindings, and Environment mounting
    /// remain identical to static siblings.
    fn compile_dynamic_branch_children(
        &mut self,
        entries: &[ast::ChildEntry],
        sink: &SubscriptionSink,
        cache: Option<&syn::Ident>,
    ) -> Result<TokenStream, String> {
        let values = entries
            .iter()
            .map(|entry| self.compile_dynamic_branch(entry, sink))
            .collect::<Result<Vec<_>, _>>()?;
        let values = if cache.is_some() {
            values
                .into_iter()
                .map(|value| Self::cache_dynamic_branch_value(value, cache))
                .collect()
        } else {
            values
        };
        Ok(quote! { vec![#(#values),*] })
    }

    /// A childless literal branch has no nested dynamic/subscription state of its own.  Keep its
    /// generated control alive when the surrounding selector changes so switching back reuses the
    /// same `Rc` (the ordinary View backend's lazy-once branch behavior).  Branches with
    /// attributes or children stay on the normal rebuild path because their subscription and
    /// nested-region lifetimes are managed by the branch state below.
    fn can_cache_dynamic_branch(entry: &ast::ChildEntry) -> bool {
        matches!(
            entry,
            ast::ChildEntry::Literal(element)
                if element.children.is_empty()
                    && element.attributes.is_empty()
                    && element.attached.is_empty()
                    && element.attribute_shortcuts.is_empty()
        )
    }

    fn cache_dynamic_branch_value(value: TokenStream, cache: Option<&syn::Ident>) -> TokenStream {
        let Some(cache) = cache else {
            return value;
        };
        quote! {{
            let mut __elwindui_branch_cache = #cache.borrow_mut();
            if __elwindui_branch_cache.is_none() {
                *__elwindui_branch_cache = Some(#value);
            }
            __elwindui_branch_cache
                .as_ref()
                .expect("dynamic branch was just materialized")
                .clone()
        }}
    }

    /// Compiles an `EnvironmentScope` as a transparent sequence of visual children.  The scope
    /// itself is not a UI element: each returned child derives the current template environment,
    /// applies the scope's writable overrides, and evaluates its normal shared-backend lowering
    /// under a lexical `__environment` alias.  Keeping this expansion at the parent collection
    /// boundary preserves the existing no-extra-node semantics while still allowing a scope to
    /// contain more than one child.
    fn compile_environment_scope_children(
        &mut self,
        scope: &ast::ElementNode,
        sink: &SubscriptionSink,
    ) -> Result<Vec<TokenStream>, String> {
        let scope_environment = self.fresh("scope_environment");
        let mut sets = TokenStream::new();
        for attribute in &scope.attributes {
            let Some((key_type_name, _)) =
                component_frontend::lookup_writable_environment_key(&attribute.name)
            else {
                return Err(format!(
                    "EnvironmentScope: `{}` is not a writable environment key",
                    attribute.name
                ));
            };
            let key_type: syn::Type = syn::parse_str(&key_type_name)
                .map_err(|error| format!("invalid EnvironmentScope key type: {error}"))?;
            let value = self.expression(&attribute.value)?;
            sets.extend(quote! {
                #scope_environment.set::<#key_type>((#value).into());
            });
        }

        let mut values = Vec::new();
        for child in &scope.children {
            let value = match child {
                ast::ChildEntry::Literal(element)
                    if is_environment_scope_type(&element.type_path) =>
                {
                    self.compile_environment_scope_children(element, sink)?
                }
                ast::ChildEntry::Literal(element) => {
                    vec![self.compile_element(element, sink)?]
                }
                ast::ChildEntry::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    if then_branch.len() != 1 || else_branch.len() != 1 {
                        return Err(
                            "dynamic EnvironmentScope branches must contain exactly one child"
                                .into(),
                        );
                    }
                    let condition = self.expression(condition)?;
                    let then_value =
                        self.compile_environment_scoped_entry(&then_branch[0], sink)?;
                    let else_value =
                        self.compile_environment_scoped_entry(&else_branch[0], sink)?;
                    vec![quote! { if #condition { #then_value } else { #else_value } }]
                }
                ast::ChildEntry::Match { value, arms } => {
                    let selector = self.expression(value)?;
                    let mut arm_values = Vec::new();
                    for arm in arms {
                        if arm.body.len() != 1 {
                            return Err(
                                "dynamic EnvironmentScope match arms must contain exactly one child"
                                    .into(),
                            );
                        }
                        let pattern =
                            syn::Pat::parse_single
                                .parse_str(&arm.pattern)
                                .map_err(|error| {
                                    format!("invalid EnvironmentScope match pattern: {error}")
                                })?;
                        let branch = self.compile_environment_scoped_entry(&arm.body[0], sink)?;
                        arm_values.push(quote! { #pattern => #branch });
                    }
                    vec![quote! { match #selector { #(#arm_values),* } }]
                }
                ast::ChildEntry::For { .. } => {
                    return Err(
                        "a `for` region cannot be directly nested in an EnvironmentScope".into(),
                    );
                }
                ast::ChildEntry::Ref(name) => {
                    let Some((binding, _)) = self.lets.get(name) else {
                        return Err(format!(
                            "EnvironmentScope reference `{name}` is not defined"
                        ));
                    };
                    vec![quote! { #binding.clone() }]
                }
            };
            values.extend(value);
        }

        Ok(values
            .into_iter()
            .map(|value| {
                quote! {{
                    let #scope_environment = __environment.derive();
                    #sets
                    let __environment = #scope_environment.clone();
                    #value
                }}
            })
            .collect())
    }

    fn compile_environment_scoped_entry(
        &mut self,
        entry: &ast::ChildEntry,
        sink: &SubscriptionSink,
    ) -> Result<TokenStream, String> {
        match entry {
            ast::ChildEntry::Literal(element) if is_environment_scope_type(&element.type_path) => {
                let mut nested = self.compile_environment_scope_children(element, sink)?;
                if nested.len() != 1 {
                    return Err(
                        "nested EnvironmentScope branch must resolve to exactly one child".into(),
                    );
                }
                Ok(nested.remove(0))
            }
            ast::ChildEntry::Literal(element) => self.compile_element(element, sink),
            ast::ChildEntry::Ref(name) => {
                let Some((binding, _)) = self.lets.get(name) else {
                    return Err(format!(
                        "EnvironmentScope reference `{name}` is not defined"
                    ));
                };
                Ok(quote! { #binding.clone() })
            }
            ast::ChildEntry::If { .. } | ast::ChildEntry::Match { .. } => {
                Err("nested dynamic EnvironmentScope branch is not supported".into())
            }
            ast::ChildEntry::For { .. } => {
                Err("a `for` region cannot be an EnvironmentScope branch".into())
            }
        }
    }

    fn compile_element(
        &mut self,
        element: &ast::ElementNode,
        sink: &SubscriptionSink,
    ) -> Result<TokenStream, String> {
        let type_path: syn::Path = syn::parse_str(&element.type_path).map_err(|error| {
            format!("invalid template element `{}`: {error}", element.type_path)
        })?;
        let (props_macro_path, known_user, user_fields) = {
            let resolved_info = self.resolve_info(&element.type_path);
            let props_macro_path = template_props_macro_path(&element.type_path, resolved_info);
            let known_user = resolved_info
                .filter(|info| !info.is_virtual_builtin && !info.is_native_control_leaf)
                .map(|info| {
                    (
                        info.param_fields.clone(),
                        info.content_field.clone(),
                        info.effective_fields.clone(),
                    )
                });
            let user_fields = resolved_info
                .filter(|info| !info.is_virtual_builtin && !info.is_native_control_leaf)
                .map(|info| info.effective_fields.clone());
            (props_macro_path, known_user, user_fields)
        };
        let node = self.fresh("node");
        let all_static_children = element.children.iter().all(|child| match child {
            ast::ChildEntry::Literal(element) => !is_environment_scope_type(&element.type_path),
            ast::ChildEntry::Ref(_) => true,
            ast::ChildEntry::If { .. }
            | ast::ChildEntry::Match { .. }
            | ast::ChildEntry::For { .. } => false,
        });
        let user_element = known_user.is_some();
        let user_param_names: std::collections::HashSet<String> = known_user
            .as_ref()
            .map(|(fields, _, _)| fields.iter().map(|(name, _)| name.clone()).collect())
            .unwrap_or_default();
        let user_constructor_only_names: std::collections::HashSet<String> = known_user
            .as_ref()
            .map(|(_, _, fields)| {
                fields
                    .iter()
                    .filter(|field| field.kind == ast::FieldKind::Param)
                    .map(|field| field.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let static_children = if all_static_children {
            Some(
                element
                    .children
                    .iter()
                    .map(|child| match child {
                        ast::ChildEntry::Literal(child) => self
                            .compile_element(child, sink)
                            .map(|value| (value, child.type_path.clone())),
                        ast::ChildEntry::Ref(name) => {
                            let Some((binding, type_path)) = self.lets.get(name) else {
                                return Err(format!(
                                    "template child reference `{name}` is not defined"
                                ));
                            };
                            Ok((quote! { #binding.clone() }, type_path.clone()))
                        }
                        _ => unreachable!(),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        let construction = if let Some((param_fields, content_field, _)) = known_user.clone() {
            let children = static_children.as_deref().unwrap_or(&[]);
            let args = self.user_component_args(
                element,
                &param_fields,
                content_field.as_deref(),
                &children,
            )?;
            quote! {{
                let __node = #type_path::__new_unmounted(#(#args),*);
                // Every generated component exposes the same hidden mount helper, including
                // composed components whose public `mount` lives on their class Ext trait.
                // Calling the helper keeps construction independent of whether that trait is
                // imported in the template's source module.
                __node.__mount(__environment.clone());
                __node
            }}
        } else {
            quote! { #type_path::new() }
        };
        let template_presenter_bind = if element
            .type_path
            .rsplit("::")
            .next()
            .is_some_and(|name| name == "ContentPresenter")
        {
            let parent = format_ident!("{}", self.parent_ident);
            quote! {
                elwindui::core::ui::ContentPresenter::__bind_templated_parent(
                    &#node,
                    &#parent,
                );
            }
        } else {
            TokenStream::new()
        };
        let mut statements = TokenStream::new();
        for attribute in &element.attributes {
            let name = format_ident!("{}", attribute.name);
            let direct_field_ty = user_fields
                .as_ref()
                .and_then(|fields| {
                    fields.iter().find(|field| {
                        field.name == attribute.name
                            && matches!(field.kind, ast::FieldKind::Prop | ast::FieldKind::State)
                            && !field.name.starts_with("on_")
                    })
                })
                .map(|field| field.ty.clone());
            let value = if attribute.name.starts_with("on_") {
                match &attribute.value {
                    ast::ViewExpr::Closure { params, body } => {
                        let parent = format_ident!("{}", self.parent_ident);
                        let closure_parent = self.fresh("event_parent");
                        let body = codegen::emit_template_event_closure_body_for_target_with_fields(
                            body,
                            params,
                            &closure_parent,
                            &self.property_bounds,
                            self.target_type.clone(),
                            self.bare_parent_fields.clone(),
                        );
                        let params = params.iter().map(|param| format_ident!("{param}"));
                        quote! {{
                            let #closure_parent = #parent.clone();
                            move |#(#params),*| { #body }
                        }}
                    }
                    other => self.expression(other)?,
                }
            } else {
                self.expression(&attribute.value)?
            };
            // Deferred `view!` attributes are assignment targets rather than a special
            // standalone-only value shape.  Let the destination's exported property metadata
            // perform the same `ViewTemplate`/`Option<ViewTemplate>` coercion used by ordinary
            // generated views.  The shared backend intentionally does this at the property
            // boundary because an expression itself has no field-type context.
            let value = if matches!(&attribute.value, ast::ViewExpr::DeferredView(_)) {
                quote! {
                    elwindui::core::ui::__coerce_deferred_view_assignment_target::<
                        #props_macro_path!(@field_type #name)
                    >(#value)
                }
            } else {
                value
            };
            let constructor_only =
                user_element && user_constructor_only_names.contains(&attribute.name);
            // Semantic-brush handling is a property concern, never an event-handler concern.  An
            // `on_*` closure can legitimately capture arbitrary external values; duplicating it
            // into a theme listener would create a second move closure and can make valid `Fn`
            // handlers fail to compile.  Constructor-only inputs likewise have no mutable setter
            // to resynchronize after construction.
            let semantic_brush = !attribute.name.starts_with("on_")
                && !constructor_only
                && self
                    .resolve_info(&element.type_path)
                    .map(|info| info.semantic_brush_fields.contains(&attribute.name))
                    .unwrap_or_else(|| {
                        self.expression_safe_for_semantic_subscription(&attribute.value)
                    });
            if semantic_brush {
                statements.extend(self.semantic_brush_subscription(
                    &node,
                    &element.type_path,
                    &props_macro_path,
                    &name,
                    &attribute.value,
                    &value,
                    sink,
                ));
            } else if !user_element
                || !user_param_names.contains(&attribute.name)
                || !constructor_only
            {
                if let Some(field_ty) = direct_field_ty.as_deref() {
                    let setter = format_ident!("set_{}", attribute.name);
                    let value = if field_ty.trim() == "String" {
                        quote! { (#value).to_string() }
                    } else {
                        value.clone()
                    };
                    statements.extend(quote! {
                        #node.#setter(#value);
                    });
                } else {
                    statements.extend(quote! {
                        #props_macro_path!(@set_with_environment #node, #name, #value, &__environment);
                    });
                }
            }
            // Fixed `#[param]` constructor inputs are consumed only while the nested component is
            // created.  They are not mutable property destinations, so a parent-dependent
            // expression supplied for one must not produce a resync subscription (nor ask the
            // props macro for a field type that intentionally has no setter).  Required mutable
            // props/state fields are constructor inputs too, but remain on the shared
            // setter/subscription path so a template binding can keep them synchronized after
            // construction.
            if user_element && user_constructor_only_names.contains(&attribute.name) {
                continue;
            }

            // `<=>` has two halves.  The initial model-to-widget assignment above and the
            // dependency subscription below are shared with ordinary one-way attributes; this
            // callback is the generic widget-to-source half.  The target props macro owns the
            // concrete `set_on_<field>_change` shape, while `two_way_setter` emits only the
            // source-side endpoint (typed `TemplateProperty` for the template parent, ordinary
            // getter/setter calls for an external capture).
            if attribute.kind == ast::AssignmentKind::TwoWay {
                let parent = format_ident!("{}", self.parent_ident);
                let callback_parent = self.fresh("two_way_parent");
                let Some(setter) = self.two_way_setter(&attribute.value, &callback_parent)? else {
                    return Err(format!(
                        "two-way template binding for `{}` requires a writable path",
                        attribute.name
                    ));
                };
                // A source owned by a loop binding (or another captured value) does not need the
                // template parent at all.  Avoid emitting an otherwise-unused `parent.clone()`:
                // the generated handler is nested inside the loop renderer, so that reference
                // would force the renderer to capture the enclosing component Rc and make the
                // callback non-`'static` when the renderer is reused by `DynamicChildSlot`.
                let parent_binding = match &attribute.value {
                    ast::ViewExpr::Path(path)
                        if path.first().is_some_and(|name| {
                            name == "templated_parent" || self.bare_parent_fields.contains(name)
                        }) =>
                    {
                        quote! { let #callback_parent = #parent.clone(); }
                    }
                    _ => TokenStream::new(),
                };
                statements.extend(quote! {
                    {
                        #parent_binding
                        #props_macro_path!(@set_on_change #name, #node, Box::new(move |new_value| {
                            #setter
                        }));
                    }
                });
            }
            let keys = self.expression_property_keys(&attribute.value);
            for key in keys {
                let node_weak = self.fresh("node_weak");
                let control_weak = self.fresh("control_weak");
                let subscription_parent = self.fresh("subscription_parent");
                let parent_ident = format_ident!("{}", self.parent_ident);
                let callback_parent_ident = parent_ident.clone();
                let expected = user_fields
                    .as_ref()
                    .and_then(|fields| {
                        fields
                            .iter()
                            .find(|field| field.name == attribute.name)
                            .map(|field| {
                                syn::parse_str::<syn::Type>(&field.ty)
                                    .expect("template field type must parse")
                            })
                    })
                    .map(|field_ty| quote! { #field_ty })
                    .unwrap_or_else(|| quote! { #props_macro_path!(@field_type #name) });
                self.constrain_property(key, expected);
                let resync_value = self.expression(&attribute.value)?;
                let resync_value = if direct_field_ty
                    .as_deref()
                    .is_some_and(|ty| ty.trim() == "String")
                {
                    quote! { (#resync_value).to_string() }
                } else {
                    resync_value
                };
                let normal_resync_set = if direct_field_ty.is_some() {
                    let setter = format_ident!("set_{}", attribute.name);
                    quote! { node.#setter(#resync_value); }
                } else {
                    quote! { #props_macro_path!(@set node, #name, #resync_value); }
                };
                let semantic_brush = self
                    .resolve_info(&element.type_path)
                    .map(|info| info.semantic_brush_fields.contains(&attribute.name))
                    .unwrap_or(true);
                let semantic_environment = self.fresh("semantic_resync_environment");
                let semantic_query = self.semantic_brush_query(&element.type_path, &attribute.name);
                let semantic_resync_set = quote! {
                    let __environment = #semantic_environment.clone();
                    #props_macro_path!(@set_with_environment
                        node,
                        #name,
                        #resync_value,
                        &__environment
                    );
                };
                let resync_set = if !semantic_brush {
                    normal_resync_set
                } else if self
                    .resolve_info(&element.type_path)
                    .is_some_and(|info| info.semantic_brush_fields.contains(&attribute.name))
                {
                    semantic_resync_set
                } else {
                    quote! {
                        if #semantic_query {
                            #semantic_resync_set
                        } else {
                            #normal_resync_set
                        }
                    }
                };
                let property_receiver = self.template_property_receiver(key);
                // Keep the environment binding available even when local metadata proves this
                // property is non-semantic.  The emitted `if #semantic_query` branch is still
                // type-checked by Rust (and by exported props macros), so its semantic setter
                // tokens must not refer to a conditionally absent local.
                let semantic_environment_binding =
                    quote! { let #semantic_environment = __environment.clone(); };
                let subscription = quote! {
                    {
                        let #node_weak = std::rc::Rc::downgrade(&#node);
                        let #subscription_parent = #parent_ident.clone();
                        let #control_weak = std::rc::Rc::downgrade(&#subscription_parent);
                        #semantic_environment_binding
                        #property_receiver::__template_subscribe(
                            &*#subscription_parent,
                            move || {
                                if let (Some(node), Some(control)) = (#node_weak.upgrade(), #control_weak.upgrade()) {
                                    let #callback_parent_ident = control;
                                    if #semantic_brush {
                                        let __environment = #semantic_environment.clone();
                                        #resync_set
                                    } else {
                                        #resync_set
                                    }
                                }
                            },
                        )
                    }
                };
                statements.extend(sink.push(subscription));
            }
        }

        // Attached properties are part of the shared View grammar as well.  Their declared value
        // type is owned by the attached-property owner's exported shape macro, so this path works
        // for builtins and user-defined owners without a frontend-specific type list.
        if !element.attached.is_empty() {
            let mut attached = TokenStream::new();
            for (owner, field, value) in &element.attached {
                let owner_info = self.resolve_info(owner);
                let props = template_props_macro_path(owner, owner_info);
                let field_ident = format_ident!("{field}");
                let value = self.expression(value)?;
                attached.extend(quote! {
                    #props!(@attached_set #field_ident, #node, #value);
                });
            }
            statements.extend(quote! {
                {
                    use elwindui::core::ui::UIElementExt as _;
                    #attached
                }
            });
        }

        // Keyboard shortcut attributes use the same backend registration helper as ordinary
        // `view!` generation.  Keeping this at the shared element boundary makes shortcut
        // declarations available in default, named, and standalone templates without a
        // standalone-only parser or backend branch.
        for (name, chords, scope) in &element.attribute_shortcuts {
            let binding = quote! { #node };
            let registration = codegen::emit_shortcut_registration(name, chords, *scope, &binding);
            statements.extend(quote! {
                {
                    use elwindui::core::ui::UIElementExt as _;
                    #registration
                }
            });
        }

        if element.children.is_empty() {
            return Ok(quote! {{
                let #node = #construction;
                #statements
                #template_presenter_bind
                #node
            }});
        }
        if all_static_children {
            let children = if let Some(children) = static_children.as_ref() {
                children
                    .iter()
                    .map(|(value, _)| value.clone())
                    .collect::<Vec<_>>()
            } else {
                element
                    .children
                    .iter()
                    .map(|child| match child {
                        ast::ChildEntry::Literal(child) => self.compile_element(child, sink),
                        ast::ChildEntry::Ref(name) => {
                            let Some((binding, _)) = self.lets.get(name) else {
                                return Err(format!(
                                    "template child reference `{name}` is not defined"
                                ));
                            };
                            Ok(quote! { #binding.clone() })
                        }
                        _ => unreachable!(),
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            // A generated component receives literal children through its constructor/content
            // property, exactly like the ordinary View compiler.  Builtin/external elements do
            // not have a local constructor shape, so their exported props macro owns attachment.
            // Never attach the same child a second time after passing it to a user component.
            let attach = if user_element {
                TokenStream::new()
            } else {
                quote! {
                    #props_macro_path!(@children #node, [#(#children),*]);
                }
            };
            return Ok(quote! {{
                let #node = #construction;
                #statements
                #template_presenter_bind
                #attach
                #node
            }});
        }

        // Dynamic nested regions use the same collection operation as generated view code.  The
        // host is intentionally not classified by a frontend-specific type-name table: the
        // generated `LayoutExt::children` call is the generic capability boundary.  Builtin and
        // user-defined Layout-derived controls therefore follow exactly the same path, while a
        // non-layout host fails through the normal trait/type diagnostics.
        let host = self.fresh("host");
        let mut child_statements = TokenStream::new();
        let mut index_offset = 0usize;
        for (source_index, child) in element.children.iter().enumerate() {
            let index = source_index + index_offset;
            match child {
                ast::ChildEntry::Literal(child) if is_environment_scope_type(&child.type_path) => {
                    let values = self.compile_environment_scope_children(child, sink)?;
                    for (offset, value) in values.iter().enumerate() {
                        let insert_index = index + offset;
                        child_statements.extend(quote! { #host.insert(#insert_index, #value); });
                    }
                    index_offset = index_offset.saturating_add(values.len().saturating_sub(1));
                }
                ast::ChildEntry::Literal(child) => {
                    let value = self.compile_element(child, sink)?;
                    child_statements.extend(quote! { #host.insert(#index, #value); });
                }
                ast::ChildEntry::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    let slot = self.fresh("slot");
                    let branch_state = self.fresh("branch_state");
                    let branch_vec = self.fresh("branch_subscriptions");
                    let then_cache = (then_branch.len() == 1
                        && Self::can_cache_dynamic_branch(&then_branch[0]))
                    .then(|| self.fresh("then_branch_cache"));
                    let else_cache = (else_branch.len() == 1
                        && Self::can_cache_dynamic_branch(&else_branch[0]))
                    .then(|| self.fresh("else_branch_cache"));
                    let initial_parent_ident = format_ident!("{}", self.parent_ident);
                    let then_value = self.compile_dynamic_branch_children(
                        then_branch,
                        &SubscriptionSink::Local(branch_vec.clone()),
                        then_cache.as_ref(),
                    )?;
                    let else_value = self.compile_dynamic_branch_children(
                        else_branch,
                        &SubscriptionSink::Local(branch_vec.clone()),
                        else_cache.as_ref(),
                    )?;
                    let condition_value = self.expression(condition)?;
                    let keys = self.expression_property_keys(condition);
                    for key in &keys {
                        self.constrain_property(*key, quote! { bool });
                    }
                    let callback_parent_ident = self.fresh("callback_parent");
                    let callback_branch_vec = self.fresh("next_branch_subscriptions");
                    let previous_parent_ident = std::mem::replace(
                        &mut self.parent_ident,
                        callback_parent_ident.to_string(),
                    );
                    let callback_condition_value = self.expression(condition)?;
                    let callback_then_value = self.compile_dynamic_branch_children(
                        then_branch,
                        &SubscriptionSink::Local(callback_branch_vec.clone()),
                        then_cache.as_ref(),
                    )?;
                    let callback_else_value = self.compile_dynamic_branch_children(
                        else_branch,
                        &SubscriptionSink::Local(callback_branch_vec.clone()),
                        else_cache.as_ref(),
                    )?;
                    self.parent_ident = previous_parent_ident;
                    let host_owner = self.fresh("host_owner");
                    let branch_cache_declarations: TokenStream = [
                        then_cache.as_ref(),
                        else_cache.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .map(|cache| {
                        quote! {
                            let #cache = std::rc::Rc::new(
                                std::cell::RefCell::new(
                                    None::<std::rc::Rc<dyn elwindui::core::ui::UIElementExt>>
                                )
                            );
                        }
                    })
                    .collect();
                    let replacement: TokenStream = keys
                        .iter()
                        .map(|key| {
                            let weak_control = self.fresh("control_weak");
                            let callback_state = self.fresh("callback_branch_state");
                            let callback_slot = self.fresh("callback_slot");
                            let callback_host_owner = self.fresh("callback_host_owner");
                            let next_value = self.fresh("next_value");
                            let callback_environment = self.fresh("callback_environment");
                            let property_receiver = self.template_property_receiver(*key);
                            let sub = quote! {
                                {
                                    let #weak_control = std::rc::Rc::downgrade(&#initial_parent_ident);
                                    let #callback_state = #branch_state.clone();
                                    let #callback_slot = #slot.clone();
                                    let #callback_host_owner = #host_owner.clone();
                                    let #callback_environment = __environment.clone();
                                    #property_receiver::__template_subscribe(
                                        &*#initial_parent_ident,
                                        move || {
                                            if let Some(control) = #weak_control.upgrade() {
                                                let __environment = #callback_environment.clone();
                                                #callback_state.borrow_mut().clear();
                                                let mut #callback_branch_vec = Vec::<elwindui::core::reactive::Subscription>::new();
                                                let #callback_parent_ident = control;
                                                let #next_value = if #callback_condition_value {
                                                    #callback_then_value
                                                } else {
                                                    #callback_else_value
                                                };
                                                *#callback_state.borrow_mut() = #callback_branch_vec;
                                                #callback_slot.replace_children(
                                                    elwindui::core::ui::LayoutExt::children(&*#callback_host_owner),
                                                    #index,
                                                    #next_value,
                                                );
                                            }
                                        },
                                    )
                                }
                            };
                            sink.push(sub)
                        })
                        .collect();
                    let initial = self.fresh("initial_value");
                    child_statements.extend(quote! {
                        let #host_owner = #node.clone();
                        let #slot = std::rc::Rc::new(elwindui::core::ui::DynamicChildSlot::<dyn elwindui::core::ui::UIElementExt>::default());
                        #branch_cache_declarations
                        let mut #branch_vec = Vec::<elwindui::core::reactive::Subscription>::new();
                        let #initial = if #condition_value { #then_value } else { #else_value };
                        #slot.replace_children(#host, #index, #initial);
                        let #branch_state = std::rc::Rc::new(std::cell::RefCell::new(#branch_vec));
                        #replacement
                    });
                }
                ast::ChildEntry::Match { value, arms } => {
                    let slot = self.fresh("slot");
                    let branch_state = self.fresh("branch_state");
                    let branch_vec = self.fresh("branch_subscriptions");
                    let arm_caches: Vec<Option<syn::Ident>> = arms
                        .iter()
                        .map(|arm| {
                            (arm.body.len() == 1 && Self::can_cache_dynamic_branch(&arm.body[0]))
                                .then(|| self.fresh("match_branch_cache"))
                        })
                        .collect();
                    let initial_parent_ident = format_ident!("{}", self.parent_ident);
                    let initial_branches: Vec<_> = arms
                        .iter()
                        .zip(arm_caches.iter())
                        .map(|(arm, cache)| {
                            self.compile_dynamic_branch_children(
                                &arm.body,
                                &SubscriptionSink::Local(branch_vec.clone()),
                                cache.as_ref(),
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    let selector_value = self.expression(value)?;
                    let keys = self.expression_property_keys(value);
                    if arms
                        .iter()
                        .all(|arm| matches!(arm.pattern.trim(), "true" | "false"))
                    {
                        for key in &keys {
                            self.constrain_property(*key, quote! { bool });
                        }
                    }
                    let callback_parent_ident = self.fresh("callback_parent");
                    let callback_branch_vec = self.fresh("next_branch_subscriptions");
                    let previous_parent_ident = std::mem::replace(
                        &mut self.parent_ident,
                        callback_parent_ident.to_string(),
                    );
                    let callback_branches: Vec<_> = arms
                        .iter()
                        .zip(arm_caches.iter())
                        .map(|(arm, cache)| {
                            self.compile_dynamic_branch_children(
                                &arm.body,
                                &SubscriptionSink::Local(callback_branch_vec.clone()),
                                cache.as_ref(),
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    self.parent_ident = previous_parent_ident;
                    let initial_arms = arms
                        .iter()
                        .zip(initial_branches.iter())
                        .map(|(arm, branch)| {
                            let pattern = syn::Pat::parse_single.parse_str(&arm.pattern).map_err(
                                |error| format!("invalid template match pattern: {error}"),
                            )?;
                            Ok::<_, String>(quote! { #pattern => #branch })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let callback_selector_value = {
                        let previous = std::mem::replace(
                            &mut self.parent_ident,
                            callback_parent_ident.to_string(),
                        );
                        let value = self.expression(value)?;
                        self.parent_ident = previous;
                        value
                    };
                    let callback_arms = arms
                        .iter()
                        .zip(callback_branches.iter())
                        .map(|(arm, branch)| {
                            let pattern = syn::Pat::parse_single.parse_str(&arm.pattern).map_err(
                                |error| format!("invalid template match pattern: {error}"),
                            )?;
                            Ok::<_, String>(quote! { #pattern => #branch })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let initial_value = quote! { match #selector_value { #(#initial_arms),* } };
                    let callback_value =
                        quote! { match #callback_selector_value { #(#callback_arms),* } };
                    let host_owner = self.fresh("host_owner");
                    let branch_cache_declarations: TokenStream = arm_caches
                        .iter()
                        .flatten()
                        .map(|cache| {
                            quote! {
                                let #cache = std::rc::Rc::new(
                                    std::cell::RefCell::new(
                                        None::<std::rc::Rc<dyn elwindui::core::ui::UIElementExt>>
                                    )
                                );
                            }
                        })
                        .collect();
                    let subscriptions: TokenStream = keys
                        .iter()
                        .map(|key| {
                            let weak_control = self.fresh("control_weak");
                            let callback_state = self.fresh("callback_branch_state");
                            let callback_slot = self.fresh("callback_slot");
                            let callback_host_owner = self.fresh("callback_host_owner");
                            let next_value = self.fresh("next_value");
                            let callback_environment = self.fresh("callback_environment");
                            let property_receiver = self.template_property_receiver(*key);
                            let sub = quote! {
                                {
                                    let #weak_control = std::rc::Rc::downgrade(&#initial_parent_ident);
                                    let #callback_state = #branch_state.clone();
                                    let #callback_slot = #slot.clone();
                                    let #callback_host_owner = #host_owner.clone();
                                    let #callback_environment = __environment.clone();
                                    #property_receiver::__template_subscribe(
                                        &*#initial_parent_ident,
                                        move || {
                                            if let Some(control) = #weak_control.upgrade() {
                                                let __environment = #callback_environment.clone();
                                                #callback_state.borrow_mut().clear();
                                                let mut #callback_branch_vec = Vec::<elwindui::core::reactive::Subscription>::new();
                                                let #callback_parent_ident = control;
                                                let #next_value = #callback_value;
                                                *#callback_state.borrow_mut() = #callback_branch_vec;
                                                #callback_slot.replace_children(
                                                    elwindui::core::ui::LayoutExt::children(&*#callback_host_owner),
                                                    #index,
                                                    #next_value,
                                                );
                                            }
                                        },
                                    )
                                }
                            };
                            sink.push(sub)
                        })
                        .collect();
                    child_statements.extend(quote! {
                        let #host_owner = #node.clone();
                        let #slot = std::rc::Rc::new(elwindui::core::ui::DynamicChildSlot::<dyn elwindui::core::ui::UIElementExt>::default());
                        #branch_cache_declarations
                        let mut #branch_vec = Vec::<elwindui::core::reactive::Subscription>::new();
                        let __initial_value = #initial_value;
                        #slot.replace_children(#host, #index, __initial_value);
                        let #branch_state = std::rc::Rc::new(std::cell::RefCell::new(#branch_vec));
                        #subscriptions
                    });
                }
                ast::ChildEntry::For { .. } => {
                    let ast::ChildEntry::For {
                        binding,
                        collection,
                        body,
                    } = child
                    else {
                        unreachable!();
                    };
                    if body.is_empty()
                        || body
                            .iter()
                            .any(|entry| !matches!(entry, ast::ChildEntry::Literal(_)))
                    {
                        return Err(
                            "template `for` bodies must contain one or more literal elements"
                                .into(),
                        );
                    }
                    let collection_value = self.expression(collection)?;
                    let collection_keys = self.expression_property_keys(collection);
                    self.iterable_properties
                        .extend(collection_keys.iter().copied());
                    let rc_identity = codegen::for_body_binds_item_to_a_bindable_field(
                        body,
                        binding,
                        &self.from,
                        &self.table,
                    );
                    let item_ident = format_ident!("{binding}");
                    let item_subscriptions = self.fresh("item_subscriptions");
                    self.loop_bindings
                        .push((binding.clone(), item_ident.clone()));
                    let item_children = body
                        .iter()
                        .map(|entry| {
                            self.compile_dynamic_branch(
                                entry,
                                &SubscriptionSink::Local(item_subscriptions.clone()),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    self.loop_bindings.pop();
                    let host_owner = self.fresh("host_owner");
                    let slot = self.fresh("slot");
                    let render = quote! {
                        {
                            let __render_environment = __environment.clone();
                            move |#item_ident| {
                                let __environment = __render_environment.clone();
                                // DynamicChildSlot renderers receive a borrowed item, while
                                // nested event/two-way handlers require an owned value that can
                                // outlive this invocation.  Clone the item at the shared backend
                                // boundary so both `replace_items` and identity-preserving
                                // `replace_rc_items` use the same lifetime-safe lowering.
                                let #item_ident = #item_ident.clone();
                                let mut #item_subscriptions =
                                    Vec::<elwindui::core::reactive::Subscription>::new();
                                elwindui::core::ui::DynamicChild::with_children(
                                    vec![#(#item_children),*],
                                    #item_subscriptions,
                                )
                            }
                        }
                    };
                    let initial_replace = if rc_identity {
                        quote! {
                            #slot.replace_rc_items(
                                #host,
                                #index,
                                &__initial_collection,
                                #render,
                            );
                        }
                    } else {
                        quote! {
                            #slot.replace_items(
                                #host,
                                #index,
                                __initial_collection,
                                #render,
                            );
                        }
                    };
                    let initial_parent_ident = format_ident!("{}", self.parent_ident);
                    let callback_parent_ident = self.fresh("callback_parent");
                    let previous_parent_ident = std::mem::replace(
                        &mut self.parent_ident,
                        callback_parent_ident.to_string(),
                    );
                    let callback_collection_value = self.expression(collection)?;
                    self.parent_ident = previous_parent_ident;
                    let refresh_subscriptions: TokenStream = collection_keys
                        .iter()
                        .map(|key| {
                            let weak_control = self.fresh("control_weak");
                            let callback_slot = self.fresh("callback_slot");
                            let callback_host_owner = self.fresh("callback_host_owner");
                            let callback_collection = self.fresh("callback_collection");
                            let callback_environment = self.fresh("callback_environment");
                            let property_receiver = self.template_property_receiver(*key);
                            let callback_replace = if rc_identity {
                                quote! {
                                    #callback_slot.replace_rc_items(
                                        elwindui::core::ui::LayoutExt::children(&*#callback_host_owner),
                                        #index,
                                        &(#callback_collection),
                                        #render,
                                    );
                                }
                            } else {
                                quote! {
                                    #callback_slot.replace_items(
                                        elwindui::core::ui::LayoutExt::children(&*#callback_host_owner),
                                        #index,
                                        #callback_collection,
                                        #render,
                                    );
                                }
                            };
                            let subscription = quote! {
                                {
                                    let #weak_control = std::rc::Rc::downgrade(&#initial_parent_ident);
                                    let #callback_slot = #slot.clone();
                                    let #callback_host_owner = #host_owner.clone();
                                    let #callback_environment = __environment.clone();
                                    #property_receiver::__template_subscribe(
                                        &*#initial_parent_ident,
                                        move || {
                                            if let Some(control) = #weak_control.upgrade() {
                                                let __environment = #callback_environment.clone();
                                                let #callback_parent_ident = control;
                                                let #callback_collection = #callback_collection_value;
                                                #callback_replace
                                            }
                                        },
                                    )
                                }
                            };
                            sink.push(subscription)
                        })
                        .collect();
                    child_statements.extend(quote! {
                        let #host_owner = #node.clone();
                        let #slot = std::rc::Rc::new(
                            elwindui::core::ui::DynamicChildSlot::<dyn elwindui::core::ui::UIElementExt>::default()
                        );
                        let __initial_collection = #collection_value;
                        #initial_replace
                        #refresh_subscriptions
                    });
                }
                ast::ChildEntry::Ref(name) => {
                    let Some((binding, _)) = self.lets.get(name) else {
                        return Err(format!("template child reference `{name}` is not defined"));
                    };
                    child_statements.extend(quote! {
                        #host.insert(#index, #binding.clone());
                    });
                }
            }
        }
        Ok(quote! {{
            let #node = #construction;
            #statements
            #template_presenter_bind
            let #host = elwindui::core::ui::LayoutExt::children(&*#node);
            #child_statements
            #node
        }})
    }

    fn expression(&mut self, expr: &ast::ViewExpr) -> Result<TokenStream, String> {
        match expr {
            ast::ViewExpr::Path(path) if !path.is_empty() => {
                if let Some(binding) = self.loop_binding(&path[0]).cloned() {
                    let mut value = quote! { #binding };
                    if path.len() == 1 {
                        return Ok(quote! { #value.clone() });
                    }
                    for segment in &path[1..] {
                        let getter = format_ident!("{segment}");
                        value = quote! { #value.#getter() };
                    }
                    return Ok(value);
                }
                self.expression_path(path)
            }
            ast::ViewExpr::Path(_) => unreachable!("empty template path was parsed"),
            ast::ViewExpr::TFluent(key, args) => {
                // Fluent arguments are expressions in their own right.  Lower them through
                // this backend rather than handing the whole node to the generic Rust
                // expression emitter so component-default shorthand such as
                // `t!("count", value: doc.count)` is normalized to the typed
                // `templated_parent` bridge as well.
                let args = args
                    .iter()
                    .map(|(name, value)| {
                        let value = self.expression(value)?;
                        Ok::<_, String>(quote! {
                            (#name, elwindui::i18n::FluentValue::from(#value))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(quote! { elwindui::i18n::t(#key, &[ #(#args),* ]) })
            }
            ast::ViewExpr::Expr(_) => Ok(codegen::emit_template_expression_for_target_with_fields(
                expr,
                &format_ident!("{}", self.parent_ident),
                &self.property_bounds,
                self.target_type.clone(),
                self.bare_parent_fields.clone(),
            )),
            ast::ViewExpr::Element(element) => self.compile_element(
                element,
                &SubscriptionSink::Shared(quote! {
                    __subscriptions
                }),
            ),
            ast::ViewExpr::Closure { params, body } => {
                let parent = format_ident!("{}", self.parent_ident);
                Ok(codegen::emit_template_closure_value_for_target_with_fields(
                    params,
                    body,
                    &parent,
                    &self.property_bounds,
                    &self.from,
                    &self.table,
                    self.target_type.clone(),
                    self.bare_parent_fields.clone(),
                ))
            }
            ast::ViewExpr::DeferredView(deferred) => {
                self.has_deferred_views = true;
                let compiled = compile_template_body(
                    &deferred.body.root,
                    &deferred.body.lets,
                    deferred.body.on_mount.as_ref(),
                    deferred.body.on_unmount.as_ref(),
                    deferred.body.on_update.as_ref(),
                    self.from.clone(),
                    self.table.clone(),
                    self.target_type.clone(),
                    self.bare_parent_fields.clone(),
                )?;
                let parent = format_ident!("{}", self.parent_ident);
                Ok(emit_view_template_factory(
                    &compiled,
                    self.target_type.clone(),
                    &parent,
                ))
            }
        }
    }

    fn expression_path(&mut self, path: &[String]) -> Result<TokenStream, String> {
        if self.bare_parent_fields.contains(path[0].as_str()) {
            // Component default templates historically allowed both `label` and
            // `label.text` shorthand.  Normalize the first segment through the same typed
            // `templated_parent` bridge used by explicit template expressions, preserving
            // the remaining getter path for compound values such as `doc.content`.
            let parent_path = ast::ViewExpr::Path(
                std::iter::once("templated_parent".to_string())
                    .chain(path.iter().cloned())
                    .collect(),
            );
            return Ok(codegen::emit_template_expression_for_target_with_fields(
                &parent_path,
                &format_ident!("{}", self.parent_ident),
                &self.property_bounds,
                self.target_type.clone(),
                self.bare_parent_fields.clone(),
            ));
        }
        Ok(codegen::emit_template_expression_for_target_with_fields(
            &ast::ViewExpr::Path(path.to_vec()),
            &format_ident!("{}", self.parent_ident),
            &self.property_bounds,
            self.target_type.clone(),
            self.bare_parent_fields.clone(),
        ))
    }

    fn expression_property_keys(&mut self, expr: &ast::ViewExpr) -> BTreeSet<u64> {
        let mut keys = BTreeSet::new();
        match expr {
            ast::ViewExpr::Path(path)
                if !path.is_empty() && self.bare_parent_fields.contains(path[0].as_str()) =>
            {
                keys.insert(crate::template_property_key(&path[0]));
            }
            ast::ViewExpr::Expr(expression) if !self.bare_parent_fields.is_empty() => {
                // Raw Rust expressions in a component default template may use the same bare
                // property spelling as ordinary `view!` attributes.  The shared closure rewriter
                // handles explicit `templated_parent.foo`; dependency collection here only needs
                // to add keys for bare names that belong to the concrete target.
                struct Collector<'a> {
                    fields: &'a HashSet<String>,
                    keys: &'a mut BTreeSet<u64>,
                }
                impl<'ast> syn::visit::Visit<'ast> for Collector<'_> {
                    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
                        if node.path.segments.len() == 1 {
                            let name = node.path.segments[0].ident.to_string();
                            if self.fields.contains(&name) {
                                self.keys.insert(crate::template_property_key(&name));
                            }
                        }
                        syn::visit::visit_expr_path(self, node);
                    }
                }
                let mut collector = Collector {
                    fields: &self.bare_parent_fields,
                    keys: &mut keys,
                };
                syn::visit::Visit::visit_expr(&mut collector, expression);
                codegen::collect_template_property_keys(expr, &mut keys);
            }
            ast::ViewExpr::TFluent(_, args) => {
                for (_, value) in args {
                    keys.extend(self.expression_property_keys(value));
                }
            }
            _ => codegen::collect_template_property_keys(expr, &mut keys),
        }
        keys
    }

    fn constrain_property(&mut self, key: u64, expected: TokenStream) {
        self.property_bounds
            .borrow_mut()
            .entry(key)
            .and_modify(|current| {
                if current.is_none() {
                    *current = Some(expected.clone());
                }
            })
            .or_insert(Some(expected));
    }

    fn template_property_receiver(&self, key: u64) -> TokenStream {
        let target = &self.target_type;
        quote! { <#target as elwindui::core::ui::TemplateProperty<#key>> }
    }
}

/// The semantic result of compiling a template body.  All three template frontends (a component
/// default, a named `#[control_template]`, and the expression-form `template_view!`) use this
/// representation before wrapping it in their respective factory/declaration shells.  Keeping
/// the root, lifecycle, dependency, and lexical-binding output together prevents a frontend from
/// quietly implementing a second property/dynamic/lifecycle compiler.
pub(crate) struct CompiledTemplateBody {
    root: TokenStream,
    let_statements: TokenStream,
    property_bounds: Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
    iterable_properties: BTreeSet<u64>,
    on_mount: Option<TokenStream>,
    on_unmount: Option<TokenStream>,
    on_update: Option<TokenStream>,
    lifecycle_keys: BTreeSet<u64>,
    has_deferred_views: bool,
}

/// Compiles a parsed template body through the shared semantic backend.  The caller is
/// responsible only for obtaining the target/type context and for wrapping the result in a
/// `ControlTemplate<C>` declaration.
pub(crate) fn compile_template_body(
    body: &ast::ViewBody,
    lets: &[ast::LetBinding],
    on_mount: Option<&syn::Block>,
    on_unmount: Option<&syn::Block>,
    on_update: Option<&ast::OnUpdateHook>,
    from: ast::Module,
    table: codegen::SymbolTable,
    target_type: TokenStream,
    bare_parent_fields: HashSet<String>,
) -> Result<CompiledTemplateBody, String> {
    let mut compiler = TemplateBackend::new(from.clone(), table, target_type, bare_parent_fields);
    let mut let_statements = TokenStream::new();
    for binding in lets {
        let value = compiler.compile_element(
            &binding.element,
            &SubscriptionSink::Shared(quote! { __subscriptions }),
        )?;
        let ident = format_ident!("__elwindui_let_{}", binding.name);
        compiler.lets.insert(
            binding.name.clone(),
            (ident.clone(), binding.element.type_path.clone()),
        );
        let_statements.extend(quote! {
            let #ident = #value;
        });
    }
    let root = compiler.compile_root(body)?;
    let template_parent_ident = format_ident!("__elwindui_template_parent");
    let on_mount_tokens = on_mount.map(|block| {
        codegen::emit_template_event_closure_body_for_target_with_fields(
            &ast::ClosureBody::Block(block.clone()),
            &[],
            &template_parent_ident,
            &compiler.property_bounds,
            compiler.target_type.clone(),
            compiler.bare_parent_fields.clone(),
        )
    });
    let on_unmount_tokens = on_unmount.map(|block| {
        codegen::emit_template_event_closure_body_for_target_with_fields(
            &ast::ClosureBody::Block(block.clone()),
            &[],
            &template_parent_ident,
            &compiler.property_bounds,
            compiler.target_type.clone(),
            compiler.bare_parent_fields.clone(),
        )
    });
    let on_update_tokens = on_update.map(|hook| {
        codegen::emit_template_event_closure_body_for_target_with_fields(
            &ast::ClosureBody::Block(hook.block.clone()),
            &[],
            &template_parent_ident,
            &compiler.property_bounds,
            compiler.target_type.clone(),
            compiler.bare_parent_fields.clone(),
        )
    });
    let mut lifecycle_keys = BTreeSet::new();
    if let Some(block) = on_mount {
        codegen::collect_template_rust_block_property_keys(block, &mut lifecycle_keys);
    }
    if let Some(block) = on_unmount {
        codegen::collect_template_rust_block_property_keys(block, &mut lifecycle_keys);
    }
    if let Some(hook) = on_update {
        codegen::collect_template_rust_block_property_keys(&hook.block, &mut lifecycle_keys);
        if let Some(fields) = &hook.fields {
            lifecycle_keys.extend(
                fields
                    .iter()
                    .map(|field| crate::template_property_key(field)),
            );
        }
    }
    Ok(CompiledTemplateBody {
        root,
        let_statements,
        property_bounds: compiler.property_bounds,
        iterable_properties: compiler.iterable_properties,
        on_mount: on_mount_tokens,
        on_unmount: on_unmount_tokens,
        on_update: on_update_tokens,
        lifecycle_keys,
        has_deferred_views: compiler.has_deferred_views,
    })
}

/// Wraps a compiled semantic template body in a concrete `ControlTemplate<T>` factory.  The
/// factory shell is intentionally kept separate from [`compile_template_body`]: component and
/// named-template frontends need only choose their target type and declaration shape, while
/// construction, binding, dynamic regions, lifecycle hooks, and cleanup are emitted once by the
/// shared body compiler.
pub(crate) fn emit_compiled_template_factory(
    body: &CompiledTemplateBody,
    target_type: TokenStream,
) -> TokenStream {
    // A standalone template whose body never reads a property from the typed parent can still
    // capture ordinary Rust values.  Keep that value-capturing path on `ControlTemplate`'s
    // environment-only constructor; a generic function item cannot capture the caller's locals.
    // Parent-dependent templates use the generic typed factory below, where `_` is resolved by
    // the expected `ControlTemplate<C>` type at the call site.
    if target_type.to_string() == "_"
        && body.property_bounds.borrow().is_empty()
        && body.iterable_properties.is_empty()
        && body.lifecycle_keys.is_empty()
    {
        let root = &body.root;
        let let_statements = &body.let_statements;
        let on_mount_hook = body
            .on_mount
            .clone()
            .map(|body| {
                quote! {
                    {
                        let __template_mount_environment = __environment.clone();
                        let __template_mount_subscriptions = __subscriptions.clone();
                        elwindui::core::ui::UIElementExt::add_mount_hook(
                            &*__root,
                            Box::new(move || {
                                let __environment = __template_mount_environment.clone();
                                let __subscriptions = __template_mount_subscriptions.clone();
                                #body
                            }),
                        );
                    }
                }
            })
            .unwrap_or_default();
        let on_unmount_hook = body
            .on_unmount
            .clone()
            .map(|body| {
                quote! {
                    elwindui::core::ui::UIElementExt::add_unmount_hook(
                        &*__root,
                        Box::new(move || {
                            #body
                        }),
                    );
                }
            })
            .unwrap_or_default();
        return quote! {
            elwindui::core::ui::ControlTemplate::from_environment(move |__environment| {
                use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
                use elwindui::ui::*;
                let __subscriptions = std::rc::Rc::new(std::cell::RefCell::new(
                    Vec::<elwindui::core::reactive::Subscription>::new(),
                ));
                #let_statements
                let __root = #root;
                #on_mount_hook
                #on_unmount_hook
                __root
            })
        };
    }

    let root = &body.root;
    let let_statements = &body.let_statements;
    let on_unmount = body.on_unmount.clone();
    let on_mount_hook = body
        .on_mount
        .clone()
        .map(|body| {
            let weak_parent = format_ident!("__elwindui_template_mount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            quote! {
                {
                    let #weak_parent = std::rc::Rc::downgrade(&#parent);
                    let __template_mount_environment = __environment.clone();
                    let __template_mount_subscriptions = __subscriptions.clone();
                    elwindui::core::ui::UIElementExt::add_mount_hook(
                        &*__root,
                        Box::new(move || {
                            if let Some(#parent) = #weak_parent.upgrade() {
                                let this = #parent.clone();
                                let __environment = __template_mount_environment.clone();
                                let __subscriptions = __template_mount_subscriptions.clone();
                                #body
                            }
                        }),
                    );
                }
            }
        })
        .unwrap_or_default();
    let update_subscriptions: TokenStream = if let Some(update_body) = body.on_update.clone() {
        body.lifecycle_keys
            .iter()
            .map(|key| {
                let weak_parent = format_ident!("__elwindui_template_update_weak_{key}");
                let parent = format_ident!("__elwindui_template_parent");
                quote! {
                    {
                        let #weak_parent = std::rc::Rc::downgrade(&#parent);
                        __subscriptions.borrow_mut().push(
                            <#target_type as elwindui::core::ui::TemplateProperty<#key>>::__template_subscribe(
                                &*#parent,
                                move || {
                                    if let Some(#parent) = #weak_parent.upgrade() {
                                        let this = #parent.clone();
                                        #update_body
                                    }
                                },
                            ),
                        );
                    }
                }
            })
            .collect()
    } else {
        TokenStream::new()
    };
    let on_unmount_hook = on_unmount.map(|body| {
        let weak_parent = format_ident!("__elwindui_template_unmount_weak");
        let parent = format_ident!("__elwindui_template_parent");
        quote! {
            {
                let #weak_parent = std::rc::Rc::downgrade(&#parent);
                elwindui::core::ui::UIElementExt::add_unmount_hook(
                    &*__root,
                    Box::new(move || {
                        if let Some(#parent) = #weak_parent.upgrade() {
                            let this = #parent.clone();
                            #body
                        }
                    }),
                );
            }
        }
    });
    let on_unmount_hook = on_unmount_hook.unwrap_or_default();
    quote! {
        elwindui::core::ui::ControlTemplate::<#target_type>::new(move |context| {
            use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
            use elwindui::ui::*;
            let __elwindui_template_parent = context.control.clone();
            let __environment = context.environment.clone();
            let __subscriptions = std::rc::Rc::new(std::cell::RefCell::new(
                Vec::<elwindui::core::reactive::Subscription>::new(),
            ));
            #let_statements
            let __root = #root;
            #on_mount_hook
            #update_subscriptions
            #on_unmount_hook
            let __template_subscriptions_for_cleanup = __subscriptions.clone();
            let __template_target_for_cleanup = __elwindui_template_parent.clone();
            __template_target_for_cleanup.add_unmount_hook(Box::new(move || {
                __template_subscriptions_for_cleanup.borrow_mut().clear();
            }));
            __root
        })
    }
}

/// Emits a `ViewTemplate` factory for a deferred expression nested inside a ControlTemplate.  The
/// deferred value keeps the same semantic body backend as its enclosing template; only the outer
/// lifecycle context changes from `ControlTemplateContext` to `ViewBuildContext`.  The concrete
/// typed parent is captured at expression-construction time, avoiding any downcast or erased target
/// lookup when the deferred view is later opened.
fn emit_view_template_factory(
    body: &CompiledTemplateBody,
    target_type: TokenStream,
    parent: &syn::Ident,
) -> TokenStream {
    let root = &body.root;
    let let_statements = &body.let_statements;
    let on_mount_hook = body
        .on_mount
        .clone()
        .map(|body| {
            let weak_parent = format_ident!("__elwindui_deferred_mount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            quote! {
                {
                    let #weak_parent = std::rc::Rc::downgrade(&#parent);
                    let __deferred_mount_environment = __environment.clone();
                    let __deferred_mount_subscriptions = __subscriptions.clone();
                    elwindui::core::ui::UIElementExt::add_mount_hook(
                        &*__root,
                        Box::new(move || {
                            if let Some(#parent) = #weak_parent.upgrade() {
                                let this = #parent.clone();
                                let __environment = __deferred_mount_environment.clone();
                                let __subscriptions = __deferred_mount_subscriptions.clone();
                                #body
                            }
                        }),
                    );
                }
            }
        })
        .unwrap_or_default();
    let update_subscriptions: TokenStream = if let Some(update_body) = body.on_update.clone() {
        body.lifecycle_keys
            .iter()
            .map(|key| {
                let weak_parent = format_ident!("__elwindui_deferred_update_weak_{key}");
                let parent = format_ident!("__elwindui_template_parent");
                quote! {
                    {
                        let #weak_parent = std::rc::Rc::downgrade(&#parent);
                        __subscriptions.borrow_mut().push(
                            <#target_type as elwindui::core::ui::TemplateProperty<#key>>::__template_subscribe(
                                &*#parent,
                                move || {
                                    if let Some(#parent) = #weak_parent.upgrade() {
                                        let this = #parent.clone();
                                        #update_body
                                    }
                                },
                            ),
                        );
                    }
                }
            })
            .collect()
    } else {
        TokenStream::new()
    };
    let on_unmount_hook = body
        .on_unmount
        .clone()
        .map(|body| {
            let weak_parent = format_ident!("__elwindui_deferred_unmount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            quote! {
                {
                    let #weak_parent = std::rc::Rc::downgrade(&#parent);
                    elwindui::core::ui::UIElementExt::add_unmount_hook(
                        &*__root,
                        Box::new(move || {
                            if let Some(#parent) = #weak_parent.upgrade() {
                                let this = #parent.clone();
                                #body
                            }
                        }),
                    );
                }
            }
        })
        .unwrap_or_default();
    quote! {
        {
            let __deferred_parent_weak = std::rc::Rc::downgrade(&#parent);
            elwindui::core::ui::ViewTemplate::new(move |context| {
                context.owner.upgrade()?;
                let __elwindui_template_parent = __deferred_parent_weak.upgrade()?;
                let __environment = context.environment.clone();
                let __subscriptions = std::rc::Rc::new(std::cell::RefCell::new(
                    Vec::<elwindui::core::reactive::Subscription>::new(),
                ));
                #let_statements
                let __root = #root;
                #on_mount_hook
                #update_subscriptions
                #on_unmount_hook
                let __deferred_subscriptions = __subscriptions.clone();
                elwindui::core::ui::UIElementExt::add_unmount_hook(
                    &*__root,
                    Box::new(move || {
                        __deferred_subscriptions.borrow_mut().clear();
                    }),
                );
                Some(__root)
            })
        }
    }
}

/// Emits the standalone `template_view!` factory shell.  A template that reads the typed parent
/// must be represented by a generic function item so Rust can infer `C` from the surrounding
/// `ControlTemplate<C>` expected type.  A parent-independent template instead uses the capturing
/// environment constructor, preserving ordinary Rust captures (which function items cannot
/// close over).  Both shells execute the same [`CompiledTemplateBody`] produced by the shared
/// backend above.
fn emit_standalone_template_factory(body: &CompiledTemplateBody) -> TokenStream {
    let parent_dependent = !body.property_bounds.borrow().is_empty()
        || !body.iterable_properties.is_empty()
        || !body.lifecycle_keys.is_empty()
        || body.has_deferred_views;
    let root = &body.root;
    let let_statements = &body.let_statements;

    if !parent_dependent {
        let on_mount_hook = body
            .on_mount
            .clone()
            .map(|body| {
                quote! {
                    {
                        let __template_mount_environment = __environment.clone();
                        let __template_mount_subscriptions = __subscriptions.clone();
                        elwindui::core::ui::UIElementExt::add_mount_hook(
                            &*__root,
                            Box::new(move || {
                                let __environment = __template_mount_environment.clone();
                                let __subscriptions = __template_mount_subscriptions.clone();
                                #body
                            }),
                        );
                    }
                }
            })
            .unwrap_or_default();
        let on_unmount_hook = body
            .on_unmount
            .clone()
            .map(|body| {
                quote! {
                    elwindui::core::ui::UIElementExt::add_unmount_hook(
                        &*__root,
                        Box::new(move || {
                            #body
                        }),
                    );
                }
            })
            .unwrap_or_default();
        return quote! {
            elwindui::core::ui::ControlTemplate::from_environment(move |__environment| {
                use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
                use elwindui::ui::*;
                let __subscriptions = std::rc::Rc::new(std::cell::RefCell::new(
                    Vec::<elwindui::core::reactive::Subscription>::new(),
                ));
                #let_statements
                let __root = #root;
                #on_mount_hook
                #on_unmount_hook
                __root
            })
        };
    }

    let bounds: Vec<_> = body
        .property_bounds
        .borrow()
        .iter()
        .map(|(key, expected)| match expected {
            Some(expected) => quote! {
                elwindui::core::ui::TemplateProperty<#key, Value = #expected>
            },
            None => quote! { elwindui::core::ui::TemplateProperty<#key> },
        })
        .collect();
    let iterable_bounds: Vec<_> = body
        .iterable_properties
        .iter()
        .map(|key| {
            quote! {
                <C as elwindui::core::ui::TemplateProperty<#key>>::Value: IntoIterator,
                <<C as elwindui::core::ui::TemplateProperty<#key>>::Value as IntoIterator>::Item:
                    std::fmt::Display
            }
        })
        .collect();
    let factory_ident = format_ident!(
        "__elwindui_template_factory_{}",
        TEMPLATE_VIEW_FACTORY_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let update_subscriptions: TokenStream = if let Some(update_body) = body.on_update.clone() {
        body.lifecycle_keys
            .iter()
            .map(|key| {
                let weak_parent = format_ident!("__elwindui_template_update_weak_{key}");
                let parent = format_ident!("__elwindui_template_parent");
                quote! {
                    {
                        let #weak_parent = std::rc::Rc::downgrade(&#parent);
                        __subscriptions.borrow_mut().push(
                            <C as elwindui::core::ui::TemplateProperty<#key>>::__template_subscribe(
                                &*#parent,
                                move || {
                                    if let Some(#parent) = #weak_parent.upgrade() {
                                        let this = #parent.clone();
                                        #update_body
                                    }
                                },
                            ),
                        );
                    }
                }
            })
            .collect()
    } else {
        TokenStream::new()
    };
    let on_unmount_hook = body
        .on_unmount
        .clone()
        .map(|body| {
            let weak_parent = format_ident!("__elwindui_template_unmount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            quote! {
                {
                    let #weak_parent = std::rc::Rc::downgrade(&#parent);
                    elwindui::core::ui::UIElementExt::add_unmount_hook(
                        &*__root,
                        Box::new(move || {
                            if let Some(#parent) = #weak_parent.upgrade() {
                                let this = #parent.clone();
                                #body
                            }
                        }),
                    );
                }
            }
        })
        .unwrap_or_default();
    let on_mount_hook = body
        .on_mount
        .clone()
        .map(|body| {
            let weak_parent = format_ident!("__elwindui_template_mount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            quote! {
                {
                    let #weak_parent = std::rc::Rc::downgrade(&#parent);
                    let __template_mount_environment = __environment.clone();
                    let __template_mount_subscriptions = __subscriptions.clone();
                    elwindui::core::ui::UIElementExt::add_mount_hook(
                        &*__root,
                        Box::new(move || {
                            if let Some(#parent) = #weak_parent.upgrade() {
                                let this = #parent.clone();
                                let __environment = __template_mount_environment.clone();
                                let __subscriptions = __template_mount_subscriptions.clone();
                                #body
                            }
                        }),
                    );
                }
            }
        })
        .unwrap_or_default();
    quote! {
        fn #factory_ident<C>(
            context: elwindui::core::ui::ControlTemplateContext<C>,
        ) -> std::rc::Rc<dyn elwindui::core::ui::UIElementExt>
        where
            C: elwindui::core::ui::ControlExt + 'static + #(#bounds)+*,
            #(#iterable_bounds,)*
        {
            use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
            use elwindui::ui::*;
            let __elwindui_template_parent = context.control.clone();
            let __environment = context.environment.clone();
            let __subscriptions = std::rc::Rc::new(std::cell::RefCell::new(
                Vec::<elwindui::core::reactive::Subscription>::new(),
            ));
            #let_statements
            let __root = #root;
            #on_mount_hook
            #update_subscriptions
            #on_unmount_hook
            let __template_subscriptions_for_cleanup = __subscriptions.clone();
            let __template_target_for_cleanup = __elwindui_template_parent.clone();
            __template_target_for_cleanup.add_unmount_hook(Box::new(move || {
                __template_subscriptions_for_cleanup.borrow_mut().clear();
            }));
            __root
        }

        elwindui::core::ui::ControlTemplate::new(#factory_ident::<_>)
    }
}

/// Generates a component whose `template: template_view!` field declares a typed default
/// Environment-selectable ControlTemplate.
///
/// Issue #146: splits into an item-local phase (`component_frontend::component_and_view_from_item_struct`
/// — a malformed `view!`/field attribute is a genuine mistake, reported unconditionally on both
/// rust-analyzer and real `rustc`) and a registry-dependent phase (`register_component_struct_real`
/// — template Environment Key resolution and cross-item `validate::validate`, both of which may fail
/// spuriously under rust-analyzer's own incomplete same-crate registry expansion order even when the
/// source is correctly ordered). The rust-analyzer struct shadow
/// (`rust_analyzer_shadow::build_component_struct_shadow`) is built once the item-local phase
/// succeeds, entirely independent of the registry-dependent phase's own outcome — see
/// `docs/design/tools/codegen_design.md` §3.2a.
pub fn generate_component_from_item_struct(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<proc_macro2::TokenStream, String> {
    // Shape errors (a malformed `view!`, a bad field attribute, ...) are reported here, against the
    // struct that actually contains them, rather than being deferred to the `impl` half. Item-local:
    // always an unconditional error.
    let (component_def, view_def) =
        component_frontend::component_and_view_from_item_struct(base.clone(), item_struct)?;

    let shadow = rust_analyzer_shadow::build_component_struct_shadow(
        component_def.base.as_deref(),
        item_struct,
        &component_def,
        view_def.as_ref(),
    )?;

    match register_component_struct_real(base, item_struct, component_def, view_def) {
        Ok(()) => Ok(shadow),
        Err(ComponentGenerationFailure::ItemLocal(error)) => Err(error),
        Err(ComponentGenerationFailure::RegistryDependent(error)) => {
            let gated_error = quote::quote! {
                #[cfg(not(rust_analyzer))]
                #[allow(unexpected_cfgs)]
                compile_error!(#error);
            };
            Ok(quote::quote! {
                #shadow
                #gated_error
            })
        }
        Err(ComponentGenerationFailure::Classified(diagnostics)) => {
            let error_tokens = validation_diagnostic_tokens(&diagnostics);
            Ok(quote::quote! {
                #shadow
                #error_tokens
            })
        }
    }
}

/// PR #169 review remediation (A1/A2): whether a Component generation failure is decidable from
/// the current item alone (`ItemLocal` — must stay an unconditional `Err`, visible under
/// rust-analyzer too, since it is a genuine mistake no same-crate registry state could excuse) or
/// depends on same-crate sibling/registry data (`RegistryDependent` — routed to a
/// `cfg(not(rust_analyzer))`-gated real error alongside the rust-analyzer shadow instead, since it
/// may be a spurious rust-analyzer expansion-order artifact even when the source is correctly
/// ordered). See `docs/design/tools/codegen_design.md` §3.2a and `validate::validate_classified`'s
/// own doc comment for the classification method `classify_validate_result` (below) uses for a
/// `validate::validate` failure specifically.
enum ComponentGenerationFailure {
    ItemLocal(String),
    RegistryDependent(String),
    /// PR #169 review remediation, round 2 (AD-R2-3): `validate::validate_classified` can find both
    /// `ItemLocal` and `RegistryDependent` diagnostics in the same pass — collapsing that mix into a
    /// single `ItemLocal`/`RegistryDependent` verdict (this variant's predecessor) either hides a
    /// real registry-dependent diagnostic behind an item-local one under `cargo build` (both are
    /// bundled into the same message either way, so nothing is technically lost there) or, worse,
    /// turns a genuine item-local mistake into a `cfg(not(rust_analyzer))`-gated error invisible to
    /// rust-analyzer the moment an unrelated registry-dependent diagnostic also fired. Carries every
    /// diagnostic from one `validate::validate_classified` call untouched; `validation_diagnostic_tokens`
    /// (below) routes each individually.
    Classified(Vec<validate::ValidationDiagnostic>),
}

/// Runs `validate::validate_classified` over `all_modules`, returning every diagnostic found
/// untouched (PR #169 review remediation, round 2, AD-R2-3) rather than folding them into one
/// collapsed verdict — a Component generator's own caller routes each diagnostic individually via
/// `validation_diagnostic_tokens`.
fn classify_validate_result(all_modules: &[ast::Module]) -> Result<(), ComponentGenerationFailure> {
    let diagnostics = validate::validate_classified(all_modules);
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(ComponentGenerationFailure::Classified(diagnostics))
    }
}

/// PR #169 review remediation, round 2 (AD-R2-3): renders a classified diagnostic list as generated
/// `compile_error!` tokens, one per diagnostic — an `ItemLocal` diagnostic unconditional (a genuine
/// mistake no same-crate registry state could excuse, so it must stay visible to rust-analyzer too),
/// a `RegistryDependent` one `#[cfg(not(rust_analyzer))]`-gated (it may be a spurious same-crate
/// registry-ordering artifact under rust-analyzer even when the source is correctly ordered) —
/// mirroring the single-diagnostic gating every other `ComponentGenerationFailure` call site already
/// applies inline, just for a whole list instead of one message.
fn validation_diagnostic_tokens(
    diagnostics: &[validate::ValidationDiagnostic],
) -> proc_macro2::TokenStream {
    diagnostics
        .iter()
        .map(|d| {
            let message = &d.message;
            match d.dependency {
                validate::ValidationDependency::ItemLocal => quote::quote! {
                    compile_error!(#message);
                },
                validate::ValidationDependency::RegistryDependent => quote::quote! {
                    #[cfg(not(rust_analyzer))]
                    #[allow(unexpected_cfgs)]
                    compile_error!(#message);
                },
            }
        })
        .collect()
}

/// The registry-dependent half of component generation (Issue #146):
/// typed template-target validation, cross-item `validate::validate` (chains in every same-crate
/// sibling registry), and — only on success — registration into the same-crate Component registry.
/// Emits no tokens of its own: the struct half always emits nothing (see this function's own tail
/// doc comment on why the `impl` half emits the whole type) — the caller's own `shadow` is the only
/// token output for a `struct` half either way.
///
/// PR #169 review remediation (A1): despite this function's own name, not every failure inside it
/// is actually registry-dependent — `view_def.is_none()` and `validate_replaceable_template_view`'s
/// own structural checks are decidable
/// from the struct alone, so those stay `ComponentGenerationFailure::ItemLocal`. Only a same-crate
/// registry lookup (`same_crate_control_target`, `lookup_same_crate_environment_key`, or
/// `validate::validate`'s own registry-dependent diagnostics) is `RegistryDependent`.
fn register_component_struct_real(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
    component_def: ast::ComponentDef,
    view_def: Option<ast::ViewDef>,
) -> Result<(), ComponentGenerationFailure> {
    let name = component_def.name.clone();
    if view_def.as_ref().is_some_and(|view| view.is_template) {
        // PR #169 review remediation, round 2 (AD-R2-2): whether this is a Control-target mistake
        // is only genuinely registry-dependent when the base is a same-crate *user* Component
        // (`ControlTargetKnowledge::NeedsSameCrateRegistry`) — `same_crate_control_target` itself
        // already resolves the fixed builtin category-tag set (`Control`/`ContentControl`/
        // `UIElement`/`Layout`/`Shape`/`NativeControl`/`Window`) without touching the registry at
        // all, and a base-less template-enabled Component is a mistake decidable from the current
        // item alone. Only the `NeedsSameCrateRegistry` branch below may fail for a reason that
        // could be a spurious rust-analyzer expansion-order artifact.
        match control_target_knowledge(component_def.base.as_deref()) {
            ControlTargetKnowledge::KnownNonControl => {
                return Err(ComponentGenerationFailure::ItemLocal(format!(
                    "`{name}`: template-enabled components must inherit Control; NativeControl and non-Control components are not supported"
                )));
            }
            ControlTargetKnowledge::KnownControl => {}
            ControlTargetKnowledge::NeedsSameCrateRegistry => {
                let base = component_def
                    .base
                    .as_deref()
                    .expect("NeedsSameCrateRegistry is only returned when a base is present");
                let is_control = same_crate_control_target(base)
                    .or_else(|| component_def.base_path.is_none().then_some(false));
                if is_control == Some(false) {
                    return Err(ComponentGenerationFailure::RegistryDependent(format!(
                        "`{name}`: template-enabled components must inherit Control; NativeControl and non-Control components are not supported"
                    )));
                }
            }
        }
        validate_replaceable_template_view(view_def.as_ref().unwrap())
            .map_err(ComponentGenerationFailure::ItemLocal)?;
    } else if view_def
        .as_ref()
        .is_some_and(|view| !view.template_instance)
        && view_def.is_some()
    {
        let is_control = match control_target_knowledge(component_def.base.as_deref()) {
            ControlTargetKnowledge::KnownControl => Some(true),
            ControlTargetKnowledge::KnownNonControl => Some(false),
            ControlTargetKnowledge::NeedsSameCrateRegistry => component_def
                .base
                .as_deref()
                .and_then(same_crate_control_target),
        };
        if is_control == Some(true) {
            return Err(ComponentGenerationFailure::ItemLocal(format!(
                "`{name}`: Control-derived components must declare visual chrome with `template: template_view! {{ ... }}`; `body: view! {{ ... }}` is ordinary component composition"
            )));
        }
    }
    // Validate here as well as in the `impl` half. Everything except `#[overrides]`-vs-base method
    // checking is already decidable from the struct alone (a non-exhaustive `match`, a typo'd
    // `vm.field`, a `#[bindable]` field whose type isn't a viewmodel, ...), and a diagnostic is far
    // more useful pointing at the struct that contains the mistake than at the `impl` below it.
    let module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: component_frontend::component_module_items(component_def, view_def),
        allows_external_builtins: true,
        ..Default::default()
    };
    let all_modules: Vec<_> = std::iter::once(module)
        .chain(component_frontend::sibling_component_modules(&name))
        .chain(component_frontend::sibling_viewmodel_modules())
        .chain(component_frontend::sibling_store_modules())
        .chain(component_frontend::sibling_enum_modules())
        .collect();
    classify_validate_result(&all_modules)?;
    component_frontend::register_same_crate_component(&name, base.as_deref(), item_struct);
    // The struct half emits nothing on purpose: the paired `#[elwindui::component] impl Name { .. }`
    // generates the whole type. This mirrors `#[elwindui_macros::class]` exactly — there too the
    // `struct` half only stashes what the `impl` half needs (`store_class_args`/`load_class_args`),
    // and the `impl` half is what emits the trait, the trait impl and `new()`. Components need the
    // same split because a `#[overridable]`/`#[overrides]` method body has nowhere to live on a bare
    // `struct`, and the generated type can only be emitted once — so it has to be emitted by
    // whichever half comes last, which is the `impl`.
    Ok(())
}

/// The generating half of the pair: `#[elwindui::component] impl Name { .. }`, whose `struct`
/// counterpart has already registered itself (see `generate_component_from_item_struct`). This is
/// what actually emits the component — the struct, its `#[elwindui_macros::class]` declaration and
/// paired class `impl`, `new()`, the accessors, the resync machinery, and §3's
/// `#[overridable]`/`#[overrides]` methods (see `component_frontend::methods_from_item_impl` for
/// the accepted method shape, and `docs/specs/dsl_spec.md` §3).
///
/// The `impl` block is required even when it declares no methods: it is the only place the type can
/// be emitted from, because a `#[overrides]` body can only be known once the `impl` is in hand and
/// the generated type must be emitted exactly once. Mirrors `#[elwindui_macros::class]`'s own
/// `struct`-stores/`impl`-emits split.
///
/// An `#[overrides]` method also gets its base's original body kept as a private `__base_<name>`
/// shadow (`codegen::resolve_effective_methods`), since `base::<name>(..)` in the override body is
/// rewritten to `self.__base_<name>(..)`.
///
/// Issue #146: splits into an item-local phase (`component_frontend::methods_from_item_impl` — a
/// malformed method signature/tag is a genuine mistake, reported unconditionally on both
/// rust-analyzer and real `rustc`) and a registry-dependent phase (`generate_component_impl_real` —
/// the paired struct's own same-crate registry lookup, cross-item `validate::validate`, and the rest
/// of real generation, all of which may fail spuriously under rust-analyzer's own incomplete
/// same-crate registry expansion order even when the source is correctly ordered — including the
/// exact "no struct was expanded before this impl block" ghost diagnostic this Issue tracks). The
/// rust-analyzer impl method shadow (`rust_analyzer_shadow::build_component_impl_shadow`) is built
/// once the item-local phase succeeds, entirely independent of the registry-dependent phase's own
/// outcome — see `docs/design/tools/codegen_design.md` §3.2a. A registry-dependent failure keeps its
/// exact existing diagnostic text, just gated to `cfg(not(rust_analyzer))` so a real ordering mistake
/// stays a real `cargo build`/`cargo check` error.
pub fn generate_component_from_item_impl(
    item_impl: &syn::ItemImpl,
) -> Result<proc_macro2::TokenStream, String> {
    // Item-local: a malformed method tag/signature is a genuine mistake, reported unconditionally —
    // `build_component_impl_shadow` itself calls `component_frontend::methods_from_item_impl` first
    // and propagates any such error via `?` below, so no separate check is needed here.
    let shadow = rust_analyzer_shadow::build_component_impl_shadow(item_impl)?;

    match generate_component_impl_real(item_impl) {
        Ok(real) => {
            let gated_real = rust_analyzer_shadow::gate_real_items_for_rustc(real)?;
            Ok(quote::quote! {
                #gated_real
                #shadow
            })
        }
        Err(ComponentGenerationFailure::ItemLocal(error)) => Err(error),
        Err(ComponentGenerationFailure::RegistryDependent(error)) => {
            let gated_error = quote::quote! {
                #[cfg(not(rust_analyzer))]
                #[allow(unexpected_cfgs)]
                compile_error!(#error);
            };
            Ok(quote::quote! {
                #shadow
                #gated_error
            })
        }
        Err(ComponentGenerationFailure::Classified(diagnostics)) => {
            let error_tokens = validation_diagnostic_tokens(&diagnostics);
            Ok(quote::quote! {
                #shadow
                #error_tokens
            })
        }
    }
}

/// The registry-dependent half of `generate_component_from_item_impl` (Issue #146) — everything the
/// original (pre-#146) function body did, unchanged, from the paired struct's own same-crate registry
/// lookup through real code generation and registration. Returns the real generated tokens
/// ungated; the caller gates them to `cfg(not(rust_analyzer))`.
///
/// PR #169 review remediation (A1): the struct-registry lookup miss, `validate::validate`'s
/// registry-dependent diagnostics, and the base-qualified-path check (needs `table.resolve`, which
/// sees sibling modules) are all genuinely `RegistryDependent`. `methods_from_item_impl`'s own
/// parse (item-local) already succeeded before this function is ever reached —
/// `build_component_impl_shadow` (called by the caller before this) runs that exact same parse and
/// would have propagated any item-local error via `?` first.
fn generate_component_impl_real(
    item_impl: &syn::ItemImpl,
) -> Result<proc_macro2::TokenStream, ComponentGenerationFailure> {
    let (name, methods) = component_frontend::methods_from_item_impl(item_impl)
        .expect("item-local method parse already validated by build_component_impl_shadow");
    let Some((mut component_def, view_def)) = component_frontend::registered_component_parts(&name)
    else {
        return Err(ComponentGenerationFailure::RegistryDependent(format!(
            "{name}: no `#[elwindui::component] struct {name} {{ .. }}` was expanded before this \
             `impl` block — declare the struct first"
        )));
    };
    component_def.methods = methods;
    let base = component_def.base.clone();
    let base_path = component_def.base_path.clone();
    let mut module = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: component_frontend::component_module_items(component_def, view_def),
        allows_external_builtins: true,
        ..Default::default()
    };
    let all_modules: Vec<_> = std::iter::once(module.clone())
        .chain(component_frontend::sibling_component_modules(&name))
        .chain(component_frontend::sibling_viewmodel_modules())
        .chain(component_frontend::sibling_store_modules())
        .chain(component_frontend::sibling_enum_modules())
        .collect();
    classify_validate_result(&all_modules)?;
    // PR #165 final rereview remediation, A2: the implicit-owner field-readable/writable schema
    // for `name` must be derived from a symbol table built over the *unlowered* `all_modules` —
    // built here, before lowering, specifically so `codegen::implicit_owner_schema` can resolve
    // `name`'s own effective (inherited-included) fields. Lowering itself still runs before the
    // *final* symbol table below (which must see the newly synthesized hidden components).
    let pre_lowering_table = codegen::build_symbol_table(&all_modules);
    let implicit_owner_schema = codegen::implicit_owner_schema(&pre_lowering_table, &module, &name);
    // Issue #162 §4.6: lower every `ViewExpr::DeferredView` reachable from `name`'s own `view`
    // into a synthetic hidden Component/View pair *after* validation (which still needs to see
    // the original, unlowered `DeferredView` nodes in `name`'s own enclosing lexical scope) and
    // *before* `build_symbol_table` (which needs to see the newly synthesized hidden components).
    lower_deferred_views_in_module(&mut module, &name, &implicit_owner_schema);
    let all_modules: Vec<_> = std::iter::once(module.clone())
        .chain(all_modules.into_iter().skip(1))
        .collect();
    let table = codegen::build_symbol_table(&all_modules);
    // A bare, non-builtin `inherits <Base>` compiles here (unlike `validate::validate_inherits`,
    // shared with the DSL text frontend, which has no qualified-path escape hatch to require at
    // all — see `ComponentDef::base_path`'s own doc comment) but is *guaranteed* to fail later,
    // confusingly, once the `#[elwindui::class(inherits = ..)]` this emits is itself expanded
    // (`elwindui_macros::class::validate_fully_qualified_path`). Catching it here, with the base's
    // own DSL name in hand, gives a much more actionable diagnostic (Refs #25).
    if let Some(base) = &base {
        let base_is_builtin = table
            .resolve(&module, base)
            .is_none_or(|info| info.is_builtin);
        if !base_is_builtin && base_path.is_none() {
            return Err(ComponentGenerationFailure::RegistryDependent(format!(
                "{name}: inherits `{base}`, but `{base}` is a user-defined component — write a \
                 full crate-root-qualified path instead of a bare name (e.g. `inherits \
                 crate::ui::{base}`). Also make sure the module exposing `{base}` re-exports it \
                 with a glob (`pub use some_module::*;`), not a named list — #[class] generates a \
                 companion `__elwindui_macros_of_{base}` alongside `{base}` itself that a named \
                 re-export would strand (docs/specs/dsl_spec.md §3)."
            )));
        }
    }
    let generated = codegen::generate_module(&module, &table);
    component_frontend::register_same_crate_component_methods(&name, item_impl);
    Ok(generated)
}

/// Issue #162 §3.5/§4.6: finds `outer_component_name`'s own `Item::View` in `module` (if any) and
/// lowers every `ViewExpr::DeferredView` reachable from it, appending one synthetic hidden
/// `Item::Component`/`Item::View` pair per deferred view found (`component_frontend::
/// hidden_view_template_component`) directly into `module.items`. A no-op when `module` has no
/// matching view (a `view`-less component can't contain a `context_popup: view! { .. }` at all).
///
/// `implicit_owner_schema` is `codegen::implicit_owner_schema`'s output for `outer_component_name`,
/// computed by the caller *before* this runs (PR #165 final rereview remediation, A2) — every
/// `DeferredView` reachable from this one call, at every nesting depth, receives the identical
/// schema (see `lower_deferred_views_in_expr`'s own doc comment for why it must never be
/// recomputed per nesting level).
pub(crate) fn lower_deferred_views_in_module(
    module: &mut ast::Module,
    outer_component_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
) {
    let Some(view) = module.items.iter_mut().find_map(|item| match item {
        ast::Item::View(v) if v.target == outer_component_name => Some(v),
        _ => None,
    }) else {
        return;
    };
    let mut ordinal = 0usize;
    let mut new_items = Vec::new();
    lower_deferred_views_in_view(
        view,
        outer_component_name,
        implicit_owner_schema,
        &mut ordinal,
        &mut new_items,
    );
    module.items.extend(new_items);
}

fn lower_deferred_views_in_view(
    view: &mut ast::ViewDef,
    owner_type_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for l in &mut view.lets {
        lower_deferred_views_in_element(
            &mut l.element,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
    lower_deferred_views_in_body(
        &mut view.root,
        owner_type_name,
        implicit_owner_schema,
        ordinal,
        new_items,
    );
}

fn lower_deferred_views_in_body(
    body: &mut ast::ViewBody,
    owner_type_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for attribute in &mut body.attributes {
        lower_deferred_views_in_expr(
            &mut attribute.value,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
    for (_, _, expr) in &mut body.attached {
        lower_deferred_views_in_expr(
            expr,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
    for child in &mut body.children {
        lower_deferred_views_in_child(
            child,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
}

fn lower_deferred_views_in_element(
    elem: &mut ast::ElementNode,
    owner_type_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for attribute in &mut elem.attributes {
        lower_deferred_views_in_expr(
            &mut attribute.value,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
    for (_, _, expr) in &mut elem.attached {
        lower_deferred_views_in_expr(
            expr,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
    for child in &mut elem.children {
        lower_deferred_views_in_child(
            child,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
}

fn lower_deferred_views_in_child(
    child: &mut ast::ChildEntry,
    owner_type_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    match child {
        ast::ChildEntry::Literal(elem) => lower_deferred_views_in_element(
            elem,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        ),
        ast::ChildEntry::Ref(_) => {}
        ast::ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            lower_deferred_views_in_expr(
                condition,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            );
            for c in then_branch.iter_mut().chain(else_branch.iter_mut()) {
                lower_deferred_views_in_child(
                    c,
                    owner_type_name,
                    implicit_owner_schema,
                    ordinal,
                    new_items,
                );
            }
        }
        ast::ChildEntry::Match { value, arms } => {
            lower_deferred_views_in_expr(
                value,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            );
            for arm in arms {
                for c in &mut arm.body {
                    lower_deferred_views_in_child(
                        c,
                        owner_type_name,
                        implicit_owner_schema,
                        ordinal,
                        new_items,
                    );
                }
            }
        }
        ast::ChildEntry::For {
            collection, body, ..
        } => {
            lower_deferred_views_in_expr(
                collection,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            );
            for c in body {
                lower_deferred_views_in_child(
                    c,
                    owner_type_name,
                    implicit_owner_schema,
                    ordinal,
                    new_items,
                );
            }
        }
    }
}

/// The one variant-specific step: everything else in this file's lowering walker only exists to
/// *reach* every `ViewExpr::DeferredView` anywhere in `owner_type_name`'s own view tree — this is
/// where one gets turned into a hidden Component/View pair (Issue #162 §3.5). Recurses into the
/// deferred body's own content *after* assigning this one's ordinal/name, using the **original**
/// `owner_type_name` — never the hidden component name just generated for *this* level — as the
/// lexical owner for any further-nested `view! { .. }` found inside it (a `context_popup` opened
/// from within another `context_popup`'s own content, at arbitrary depth). `implicit_owner_schema`
/// is likewise the *same* schema at every nesting depth (PR #165 final rereview remediation, A2) —
/// never recomputed from a nested level's own (synthetic, effectively field-less) hidden Component.
///
/// PR #165 review remediation, A3: an earlier revision passed the just-assigned `hidden_name`
/// here instead, which changed source lexical-scoping semantics — a doubly-nested deferred view's
/// bare names would resolve against the *synthetic* outer hidden Component instead of the real
/// source Component the DSL author actually wrote both `view! { .. }` blocks inside of. Lowering
/// is a compiler-internal transformation invisible to the DSL author's own mental model: from
/// their perspective every `context_popup: view! { .. }` — no matter how many levels deep inside
/// another one — is still textually nested inside the *same* outer `view! { .. }` macro
/// invocation the enclosing Component declared, exactly like an `on_click` closure's own bare
/// names already resolve against the outer Component regardless of closure nesting depth. Every
/// `DeferredView` reachable from one `lower_deferred_views_in_module` call therefore keeps the
/// *same* `owner_type_name` — the original source Component — no matter its nesting depth; only
/// the generated hidden component's own *name* (`hidden_name`) changes per level.
fn lower_deferred_views_in_expr(
    expr: &mut ast::ViewExpr,
    owner_type_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    match expr {
        ast::ViewExpr::DeferredView(deferred) => {
            *ordinal += 1;
            let hidden_name =
                format!("__ElwinduiViewTemplateInstanceFor{owner_type_name}_{ordinal}");
            lower_deferred_views_in_element_lets_and_body(
                &mut deferred.body.lets,
                &mut deferred.body.root,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            );
            let (hidden_component, hidden_view) =
                component_frontend::hidden_view_template_component(
                    &hidden_name,
                    owner_type_name,
                    implicit_owner_schema,
                    &deferred.body,
                );
            new_items.push(ast::Item::Component(hidden_component));
            new_items.push(ast::Item::View(hidden_view));
            deferred.hidden_component = Some(hidden_name);
            // A3: the true source lexical owner, not whatever component's generated code this
            // factory expression ends up emitted inside of (see `DeferredViewExpr::lexical_owner`'s
            // own doc comment).
            deferred.lexical_owner = Some(owner_type_name.to_string());
        }
        ast::ViewExpr::Element(elem) => lower_deferred_views_in_element(
            elem,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        ),
        ast::ViewExpr::Closure { body, .. } => match body {
            ast::ClosureBody::Element(elem) => lower_deferred_views_in_element(
                elem,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            ),
            ast::ClosureBody::Expr(inner) => lower_deferred_views_in_expr(
                inner,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            ),
            // A raw `syn::Block` (`on_*` handler body) has no reachable `ast::ViewExpr` of its own
            // to recurse into — `view!` only ever appears at a DSL attribute-value position, which
            // a `syn::Block` doesn't parse through this AST at all.
            ast::ClosureBody::Block(_) => {}
        },
        ast::ViewExpr::TFluent(_, args) => {
            for (_, v) in args {
                lower_deferred_views_in_expr(
                    v,
                    owner_type_name,
                    implicit_owner_schema,
                    ordinal,
                    new_items,
                );
            }
        }
        ast::ViewExpr::Path(_) | ast::ViewExpr::Expr(_) => {}
    }
}

fn lower_deferred_views_in_element_lets_and_body(
    lets: &mut [ast::LetBinding],
    root: &mut ast::ViewBody,
    owner_type_name: &str,
    implicit_owner_schema: &ast::ImplicitOwnerDef,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for l in lets.iter_mut() {
        lower_deferred_views_in_element(
            &mut l.element,
            owner_type_name,
            implicit_owner_schema,
            ordinal,
            new_items,
        );
    }
    lower_deferred_views_in_body(
        root,
        owner_type_name,
        implicit_owner_schema,
        ordinal,
        new_items,
    );
}

/// Generates the private component instance and typed factory for
/// `#[elwindui::control_template(target = Target)] struct Name { template: template_view! { .. } }`.
pub fn generate_control_template_from_item_struct(
    target: &syn::Path,
    item_struct: &syn::ItemStruct,
) -> Result<proc_macro2::TokenStream, String> {
    let target_name = target
        .segments
        .last()
        .expect("a syn::Path always has at least one segment")
        .ident
        .to_string();
    if same_crate_control_target(&target_name) == Some(false) {
        return Err(format!(
            "target `{target_name}` is not a Control-derived component; NativeControl and non-Control targets cannot be templated"
        ));
    }
    if !item_struct.generics.params.is_empty() {
        return Err("ControlTemplate declarations cannot be generic".to_string());
    }
    let syn::Fields::Named(fields) = &item_struct.fields else {
        return Err("expected a struct with exactly `template: template_view! { .. }`".to_string());
    };
    let mut fields_iter = fields.named.iter();
    let Some(body) = fields_iter.next() else {
        return Err("expected `template: template_view! { .. }`".to_string());
    };
    if fields_iter.next().is_some()
        || body.ident.as_ref().is_none_or(|ident| ident != "template")
        || !matches!(
            &body.ty,
            syn::Type::Macro(mac)
                if mac.mac.path.segments.last().is_some_and(|segment| segment.ident == "template_view")
        )
    {
        return Err("expected exactly one field: `template: template_view! { .. }`".to_string());
    }

    let (_, authored_view) = component_frontend::component_and_view_from_item_struct(
        Some("Control".to_string()),
        item_struct,
    )?;
    validate_replaceable_template_view(
        authored_view
            .as_ref()
            .ok_or_else(|| "expected `template: template_view! { .. }`".to_string())?,
    )?;

    let name = &item_struct.ident;
    // Named templates use the same semantic backend as component defaults and the expression-form
    // `template_view!` frontend.  The public declaration remains a zero-sized marker; its factory
    // is built directly for the requested target type rather than through a hidden component.
    let authored_view = authored_view
        .as_ref()
        .expect("validated control template must have an authored template view");
    let from = ast::Module {
        path: Vec::new(),
        uses: Vec::new(),
        items: Vec::new(),
        is_builtin: false,
        allows_external_builtins: true,
    };
    let modules: Vec<_> = std::iter::once(from.clone())
        .chain(component_frontend::sibling_component_modules(
            &name.to_string(),
        ))
        .chain(component_frontend::sibling_viewmodel_modules())
        .chain(component_frontend::sibling_store_modules())
        .chain(component_frontend::sibling_enum_modules())
        .collect();
    let table = codegen::build_symbol_table(&modules);
    let compiled = compile_template_body(
        &authored_view.root,
        &authored_view.lets,
        authored_view.on_mount.as_ref(),
        authored_view.on_unmount.as_ref(),
        authored_view.on_update.as_ref(),
        from,
        table,
        quote! { #target },
        HashSet::new(),
    )?;
    let template_factory = emit_compiled_template_factory(&compiled, quote! { #target });

    let attrs = &item_struct.attrs;
    let vis = &item_struct.vis;
    // Keep the rust-analyzer split for the public marker/signature. The real declaration invokes the
    // shared factory directly; no hidden runtime component or standalone-only owner is emitted.
    let real = rust_analyzer_shadow::gate_real_items_for_rustc(quote::quote! {
        #(#attrs)*
        #vis struct #name;

        impl #name {
            pub fn template() -> elwindui::core::ui::ControlTemplate<#target> {
                #template_factory
            }
        }
    })?;
    let shadow = rust_analyzer_shadow::build_control_template_shadow(item_struct, target)?;

    Ok(quote::quote! {
        #real
        #shadow
    })
}

/// PR #169 review remediation, round 2 (AD-R2-2): whether a template-enabled Component's `inherits`
/// base is decidable as Control-derived (or not) purely from the current item plus the fixed set of
/// builtin category-tag names `same_crate_control_target` itself already resolves without any
/// same-crate registry lookup, or whether deciding requires resolving a same-crate user-defined base
/// through the Component registry (`same_crate_control_target`'s own recursive
/// `registered_component_parts` walk).
enum ControlTargetKnowledge {
    KnownControl,
    KnownNonControl,
    NeedsSameCrateRegistry,
}

/// Classifies `base` (a template-enabled Component's own `inherits` target, if any) into
/// [`ControlTargetKnowledge`]. A base-less Component and every fixed builtin category-tag name
/// (`Control`/`ContentControl`/`UIElement`/`Layout`/`Shape`/`NativeControl`/`Window`) are decidable
/// from the current item alone; only a name outside that fixed set — necessarily a same-crate user
/// Component, since a builtin ancestor is always one of these tags — needs the registry.
fn control_target_knowledge(base: Option<&str>) -> ControlTargetKnowledge {
    match base {
        None => ControlTargetKnowledge::KnownNonControl,
        Some("Control") | Some("ContentControl") => ControlTargetKnowledge::KnownControl,
        Some("UIElement")
        | Some("Layout")
        | Some("Shape")
        | Some("NativeControl")
        | Some("Window") => ControlTargetKnowledge::KnownNonControl,
        Some(_) => ControlTargetKnowledge::NeedsSameCrateRegistry,
    }
}

#[cfg(test)]
mod control_target_knowledge_tests {
    use super::*;

    /// PR #169 review remediation, round 2, T-R2-4 (AD-R2-2): a fixed builtin category-tag name
    /// known to be non-Control (`NativeControl`, e.g.) is decidable from the current item alone —
    /// no same-crate registry lookup needed — so `register_component_struct_real` must reject it
    /// with an unconditional `ItemLocal` failure, never a registry-dependent one.
    #[test]
    fn t_r2_4_known_non_control_builtin_base_needs_no_registry() {
        assert!(matches!(
            control_target_knowledge(Some("NativeControl")),
            ControlTargetKnowledge::KnownNonControl
        ));
        assert!(matches!(
            control_target_knowledge(Some("Control")),
            ControlTargetKnowledge::KnownControl
        ));
        assert!(matches!(
            control_target_knowledge(None),
            ControlTargetKnowledge::KnownNonControl
        ));
    }

    /// PR #169 review remediation, round 2, T-R2-5 (AD-R2-2): a same-crate user-defined base name
    /// (outside the fixed builtin category-tag set) can only be resolved as Control-derived or not
    /// through `same_crate_control_target`'s own registry walk — `control_target_knowledge` must
    /// route it to `NeedsSameCrateRegistry` rather than guessing.
    #[test]
    fn t_r2_5_same_crate_user_base_needs_registry() {
        assert!(matches!(
            control_target_knowledge(Some("SomeUserDefinedComponent")),
            ControlTargetKnowledge::NeedsSameCrateRegistry
        ));
    }
}

fn same_crate_control_target(name: &str) -> Option<bool> {
    fn visit(name: &str, visited: &mut std::collections::HashSet<String>) -> Option<bool> {
        match name {
            "Control" | "ContentControl" => return Some(true),
            "UIElement" | "Layout" | "Shape" | "NativeControl" | "Window" | "VerticalLayout"
            | "HorizontalLayout" | "Grid" | "TextBlock" | "Rectangle" | "Ellipse" => {
                return Some(false);
            }
            _ => {}
        }
        if !visited.insert(name.to_string()) {
            return None;
        }
        let (component, _) = component_frontend::registered_component_parts(name)?;
        let base = component.base.as_deref()?;
        visit(base, visited).or_else(|| component.base_path.is_none().then_some(false))
    }

    visit(name, &mut std::collections::HashSet::new())
}

fn validate_replaceable_template_view(view: &ast::ViewDef) -> Result<(), String> {
    if view.lets.iter().any(|binding| binding.id.is_some()) {
        return Err("#[id(...)] is not supported inside a replaceable ControlTemplate".to_string());
    }

    fn is_presenter(type_path: &str) -> bool {
        type_path.rsplit("::").next() == Some("ContentPresenter")
    }

    fn visit_expr(
        expr: &ast::ViewExpr,
        dynamic: bool,
        presenters: &mut usize,
    ) -> Result<(), String> {
        match expr {
            ast::ViewExpr::Element(element) => visit_element(element, dynamic, presenters),
            ast::ViewExpr::Closure { body, .. } => match body {
                ast::ClosureBody::Element(element) => visit_element(element, dynamic, presenters),
                ast::ClosureBody::Expr(expr) => visit_expr(expr, dynamic, presenters),
                ast::ClosureBody::Block(_) => Ok(()),
            },
            ast::ViewExpr::TFluent(_, args) => {
                for (_, expr) in args {
                    visit_expr(expr, dynamic, presenters)?;
                }
                Ok(())
            }
            // A deferred view is its own independent nested scope (lowered to its own hidden
            // Component, Issue #162) — its `ContentPresenter`/`#[id]` usage is no more this
            // `ControlTemplate`'s concern than an ordinary nested Component's own view would be.
            ast::ViewExpr::Path(_) | ast::ViewExpr::Expr(_) | ast::ViewExpr::DeferredView(_) => {
                Ok(())
            }
        }
    }

    fn visit_element(
        element: &ast::ElementNode,
        dynamic: bool,
        presenters: &mut usize,
    ) -> Result<(), String> {
        if is_presenter(&element.type_path) {
            if dynamic {
                return Err(
                    "ContentPresenter is not supported inside a dynamic template region"
                        .to_string(),
                );
            }
            *presenters += 1;
            if *presenters > 1 {
                return Err(
                    "a ControlTemplate may contain at most one ContentPresenter".to_string()
                );
            }
        }
        for attribute in &element.attributes {
            visit_expr(&attribute.value, dynamic, presenters)?;
        }
        for child in &element.children {
            visit_child(child, dynamic, presenters)?;
        }
        Ok(())
    }

    fn visit_child(
        child: &ast::ChildEntry,
        dynamic: bool,
        presenters: &mut usize,
    ) -> Result<(), String> {
        match child {
            ast::ChildEntry::Literal(element) => visit_element(element, dynamic, presenters),
            ast::ChildEntry::Ref(_) => Ok(()),
            ast::ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            } => {
                visit_expr(condition, dynamic, presenters)?;
                for child in then_branch.iter().chain(else_branch) {
                    visit_child(child, true, presenters)?;
                }
                Ok(())
            }
            ast::ChildEntry::Match { value, arms } => {
                visit_expr(value, dynamic, presenters)?;
                for arm in arms {
                    for child in &arm.body {
                        visit_child(child, true, presenters)?;
                    }
                }
                Ok(())
            }
            ast::ChildEntry::For {
                collection, body, ..
            } => {
                visit_expr(collection, dynamic, presenters)?;
                for child in body {
                    visit_child(child, true, presenters)?;
                }
                Ok(())
            }
        }
    }

    let mut presenters = 0;
    for binding in &view.lets {
        visit_element(&binding.element, false, &mut presenters)?;
    }
    for child in &view.root.children {
        visit_child(child, false, &mut presenters)?;
    }
    Ok(())
}

#[cfg(test)]
mod template_view_expression_tests {
    use super::*;

    #[test]
    fn standalone_expression_reuses_control_template_presenter_validation() {
        let error = generate_template_view_expression(
            r#"
                VerticalLayout {
                    ContentPresenter {}
                    ContentPresenter {}
                }
            "#,
        )
        .expect_err("a template cannot contain multiple ContentPresenter nodes");
        assert!(
            error.contains("multiple") || error.contains("ContentPresenter"),
            "{error}"
        );

        let error = generate_template_view_expression(
            r#"
                VerticalLayout {
                    if show_content {
                        ContentPresenter {}
                    }
                }
            "#,
        )
        .expect_err("a dynamic ContentPresenter is not supported");
        assert!(
            error.contains("dynamic") || error.contains("ContentPresenter"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod control_template_tests {
    use super::*;

    fn author(src: &str) -> Result<String, String> {
        author_for("Control", src)
    }

    fn author_for(target: &str, src: &str) -> Result<String, String> {
        let item: syn::ItemStruct = syn::parse_str(src).expect("template struct should parse");
        let target: syn::Path = syn::parse_str(target).unwrap();
        generate_control_template_from_item_struct(&target, &item).map(|tokens| tokens.to_string())
    }

    #[test]
    fn authoring_generates_a_typed_factory_without_hidden_instance() {
        let generated = author(
            r#"
            struct CodegenControlTemplateValidA {
                template: template_view! { TextBlock { text: "ok" } },
            }
            "#,
        )
        .expect("valid template should generate");
        assert!(generated.contains("ControlTemplate < Control >"));
        assert!(generated.contains("ControlTemplate :: < Control > :: new"));
        assert!(!generated.contains("Weak < Control >"));
        assert!(!generated.contains("__ElwinduiControlTemplateInstanceFor"));
    }

    /// The named-template frontend keeps separate rustc/rust-analyzer declarations, while its
    /// runtime branch directly invokes the shared typed template factory.  No hidden component
    /// instance is synthesized for named templates.
    #[test]
    fn t12_output_contains_real_and_shadow_branches_with_no_discarded_hidden_shadow() {
        let generated = author(
            r#"
            struct CodegenControlTemplateT12 {
                template: template_view! { TextBlock { text: "ok" } },
            }
            "#,
        )
        .expect("valid template should generate");

        // Real: gated, with the real shared-factory body.
        assert!(
            generated.contains("cfg (not (rust_analyzer))"),
            "{generated}"
        );
        assert!(
            generated.contains("struct CodegenControlTemplateT12 ;"),
            "{generated}"
        );
        assert!(
            generated.contains("ControlTemplate :: < Control > :: new")
                && !generated.contains("__new_unmounted")
                && !generated.contains("into_node"),
            "the real template() body must invoke the shared factory directly: {generated}"
        );

        // Shadow: gated, signature-only (no hidden-instance construction).
        assert!(generated.contains("cfg (rust_analyzer)"), "{generated}");
        assert!(
            generated.contains(
                "fn template () -> elwindui :: core :: ui :: ControlTemplate < Control > { unreachable ! () }"
            ),
            "the shadow template() must be signature-only, never calling the real hidden-instance \
             construction: {generated}"
        );

        // No hidden Component is part of the unified named-template output.
        assert!(
            !generated.contains("__ElwinduiControlTemplateInstanceFor"),
            "named templates must not synthesize a hidden Component: {generated}"
        );
    }

    #[test]
    fn authoring_rejects_ids_multiple_presenters_and_dynamic_presenters() {
        let id = author(
            r#"
            struct CodegenControlTemplateIdB {
                template: template_view! {
                    #[id("part")]
                    let part = TextBlock { text: "x" };
                    part
                },
            }
            "#,
        )
        .expect_err("replaceable template ids must be rejected");
        assert!(id.contains("#[id"), "error: {id}");

        let multiple = author(
            r#"
            struct CodegenControlTemplateMultipleB {
                template: template_view! {
                    VerticalLayout { ContentPresenter {} ContentPresenter {} }
                },
            }
            "#,
        )
        .expect_err("multiple presenters must be rejected");
        assert!(
            multiple.contains("at most one ContentPresenter"),
            "error: {multiple}"
        );

        let dynamic = author(
            r#"
            struct CodegenControlTemplateDynamicB {
                template: template_view! {
                    VerticalLayout { if true { ContentPresenter {} } }
                },
            }
            "#,
        )
        .expect_err("dynamic presenters must be rejected");
        assert!(
            dynamic.contains("dynamic template region"),
            "error: {dynamic}"
        );
    }

    #[test]
    fn same_crate_non_control_and_native_control_targets_are_rejected_early() {
        let template = r#"
            struct CodegenControlTemplateInvalidTargetD {
                template: template_view! { TextBlock { text: "x" } },
            }
        "#;
        let non_control = author_for("VerticalLayout", template)
            .expect_err("same-crate non-Control target must be rejected");
        assert!(non_control.contains("not a Control-derived"));

        let native = author_for("NativeControl", template)
            .expect_err("NativeControl target must be rejected");
        assert!(native.contains("NativeControl"));
    }
}

/// Phase 4 (`docs/status/implementation_status.md`): exercises `#[elwindui::dsl_enum]` end to
/// end through the exact same path `generate_component_from_item_struct` uses in production
/// (register the enum via `generate_dsl_enum_from_item_enum`, then chain it in via
/// `component_frontend::sibling_enum_modules()` while building a sibling component) — confirming a
/// `match` in a Rust-macro-path `view!` gets real exhaustiveness checking against a same-crate
/// `#[elwindui::dsl_enum]`, the gap `component_frontend::same_crate_enums`'s own doc comment
/// describes. Names are unique per test (`DslEnumTest*`) — `same_crate_enums`/
/// `same_crate_components` are process-global statics keyed by `compiling_crate_key()`, which is
/// constant across every test in this one crate's test binary, so two tests reusing the same enum/
/// component name would leak state into each other (same constraint `component_frontend.rs`'s own
/// tests already live with).
#[cfg(test)]
mod dsl_enum_tests {
    use super::*;

    fn register_enum(src: &str) {
        let item_enum: syn::ItemEnum = syn::parse_str(src).expect("enum should parse");
        generate_dsl_enum_from_item_enum(&item_enum).expect("dsl_enum generation should succeed");
    }

    #[test]
    fn exhaustive_match_against_sibling_dsl_enum_validates() {
        register_enum("enum DslEnumTestStatusA { Loading, Ready }");
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct DslEnumTestScreenA {
                #[prop]
                status: DslEnumTestStatusA,
                body: view! {
                    match status {
                        DslEnumTestStatusA::Loading => TextBlock { text: "loading" },
                        DslEnumTestStatusA::Ready => TextBlock { text: "ready" },
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let result =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct);
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[test]
    fn non_exhaustive_match_against_sibling_dsl_enum_is_rejected() {
        register_enum("enum DslEnumTestStatusB { Loading, Ready }");
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct DslEnumTestScreenB {
                #[prop]
                status: DslEnumTestStatusB,
                body: view! {
                    VerticalLayout {
                        match status {
                            DslEnumTestStatusB::Loading => TextBlock { text: "loading" },
                        }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = expect_generation_error(generate_component_from_item_struct(None, &item_struct));
        assert!(
            err.contains("not exhaustive") && err.contains("Ready"),
            "error should name the missing variant: {err}"
        );
    }
}

/// Phase 4: confirms the same-crate viewmodel registry (`component_frontend::same_crate_viewmodels`,
/// populated by `generate_viewmodel_from_item_mod`) actually catches, on the Rust-macro path, the
/// two mistakes its own doc comment claims it fixes — a typo'd `vm.field` reference and a
/// `#[bindable]` field whose type isn't a `viewmodel` at all — mirroring `validate.rs`'s own
/// `rejects_reference_to_unknown_vm_field`/`rejects_bindable_field_whose_type_is_not_a_viewmodel`
/// tests for the DSL-text frontend. Names are unique per test for the same reason
/// `dsl_enum_tests` uses unique names (shared process-global registries).
#[cfg(test)]
mod viewmodel_registry_tests {
    use super::*;

    fn register_viewmodel(src: &str) {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("mod should parse");
        generate_viewmodel_from_item_mod(&item_mod).expect("viewmodel generation should succeed");
    }

    #[test]
    fn typo_vm_field_reference_is_rejected_on_macro_path() {
        register_viewmodel(
            r#"
            mod vm_typo_a_mod {
                struct VmTypoA {
                    #[observable(default = String::new())]
                    content: String,
                }
            }
            "#,
        );
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct ScreenTypoA {
                #[param]
                #[inject]
                vm: VmTypoA,
                body: view! {
                    VerticalLayout {
                        TextBlock { text: vm.no_such_field }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = expect_generation_error(generate_component_from_item_struct(None, &item_struct));
        assert!(err.contains("no_such_field"), "error: {err}");
    }

    #[test]
    fn bindable_field_on_non_viewmodel_type_is_rejected_on_macro_path() {
        // A plain sibling component (not a viewmodel), registered exactly like any other
        // #[elwindui::component] would be.
        let not_vm_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct NotAViewModelB {
                #[param]
                label: String,
                body: view! { TextBlock { text: label } },
            }
            "#,
        )
        .unwrap();
        generate_component_from_item_struct(Some("VerticalLayout".to_string()), &not_vm_struct)
            .expect("sibling component should build");

        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct WindowBindableB {
                #[bindable]
                thing: std::rc::Rc<NotAViewModelB>,
                body: view! {
                    Window { TextBlock { text: "x" } }
                },
            }
            "#,
        )
        .unwrap();
        let err = expect_generation_error(generate_component_from_item_struct(None, &item_struct));
        assert!(err.contains("isn't a `viewmodel`"), "error: {err}");
    }
}

/// `#[elwindui::store] mod foo { .. }` (Issue #82) — the macro-path counterpart to
/// `viewmodel_registry_tests` above, covering `store`'s own AST/codegen shape and the rule-20
/// (`#[async_computed]` attachment surface) validation it shares with `viewmodel`.
#[cfg(test)]
mod store_registry_tests {
    use super::*;

    #[test]
    fn store_with_observable_computed_and_async_computed_generates_a_singleton_accessor() {
        let item_mod: syn::ItemMod = syn::parse_str(
            r#"
            mod counter_store_gen_test_mod {
                struct CounterStoreGenTest {
                    #[observable(default = 0i32)]
                    count: i32,

                    #[computed(expr = count * 2)]
                    doubled: i32,

                    #[async_computed(expr = fetch(count))]
                    remote: i64,
                }
            }
            "#,
        )
        .expect("mod should parse");
        let generated = generate_store_from_item_mod(&item_mod)
            .expect("store generation should succeed")
            .to_string();

        // Delegates field codegen to `generate_viewmodel` unchanged — same accessor shapes a
        // viewmodel would get.
        assert!(generated.contains("fn count"), "generated: {generated}");
        assert!(generated.contains("fn set_count"), "generated: {generated}");
        assert!(generated.contains("fn doubled"), "generated: {generated}");
        // `#[async_computed]`'s getter returns the `AsyncComputed<T>` wrapper, not bare `T`.
        assert!(
            generated.contains("AsyncComputed < i64 >") || generated.contains("AsyncComputed<i64>"),
            "generated: {generated}"
        );
        assert!(
            generated.contains("__spawn_recompute_remote"),
            "generated: {generated}"
        );
        // The singleton wrapper: a generated `EnvironmentKey` and `instance()`.
        assert!(
            generated.contains("EnvironmentKey") && generated.contains("fn instance"),
            "generated: {generated}"
        );
    }

    /// Asserts `result` carries `message` as an *unconditional* `compile_error!` — never gated
    /// behind `#[cfg(not(rust_analyzer))]` (PR #169 review remediation, round 2, AD-R2-3: an
    /// `ItemLocal` diagnostic from `classify_validate_result`/`validate::validate_classified` is
    /// routed through `ComponentGenerationFailure::Classified` + `validation_diagnostic_tokens` into
    /// an unconditional generated `compile_error!` alongside the rust-analyzer shadow, not a hard
    /// proc-macro `Err` — a real mistake stays a real `cargo build`/`cargo check` error either way,
    /// but this form also keeps the shadow available to other same-crate consumers of the type under
    /// rust-analyzer, matching every other item-local check's own inline gating already does for a
    /// single diagnostic). A bare `Err` (an item-local mistake caught before `classify_validate_result`
    /// is ever reached, e.g. a `component_frontend` parse failure) is also accepted — the two forms
    /// have the same end-user-visible effect under `cargo build`.
    fn expect_unconditional_item_local_error(
        result: Result<proc_macro2::TokenStream, String>,
        message_substrings: &[&str],
    ) -> String {
        match result {
            Err(error) => {
                for substring in message_substrings {
                    assert!(error.contains(substring), "error: {error}");
                }
                error
            }
            Ok(tokens) => {
                let s = tokens.to_string();
                assert!(
                    s.contains("compile_error !") && !s.contains("cfg (not (rust_analyzer))"),
                    "expected an unconditional (non-gated) compile_error!, not one gated behind \
                     `#[cfg(not(rust_analyzer))]`: {s}"
                );
                for substring in message_substrings {
                    assert!(s.contains(substring), "generated: {s}");
                }
                s
            }
        }
    }

    #[test]
    fn async_computed_on_a_plain_component_prop_is_rejected() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct AsyncComputedMisuseComponent {
                #[async_computed(expr = fetch())]
                remote: i32,
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        expect_unconditional_item_local_error(
            generate_component_from_item_struct(None, &item_struct),
            &["#[async_computed]", "viewmodel/store"],
        );
    }

    /// PR #169 review remediation, T1 (round 1); updated round 2 (AD-R2-3): `#[async_computed]` on a
    /// plain Component field is decidable from the field's own `FieldKind` alone — no sibling module
    /// is ever consulted to decide it — so `validate::validate_classified` tags it `ItemLocal`. Round
    /// 2's `ComponentGenerationFailure::Classified` routes an `ItemLocal` diagnostic to an
    /// *unconditional* generated `compile_error!` (visible under both rust-analyzer and real
    /// `cargo build`) rather than a hard proc-macro `Err` — see `expect_unconditional_item_local_error`'s
    /// own doc comment. `inherits VerticalLayout` here (unlike the sibling test above) isolates this
    /// from the *also* item-local base-less-with-view rule, so this test exercises
    /// `#[async_computed]` alone.
    #[test]
    fn t1_async_computed_on_component_field_is_an_unconditional_item_local_error() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct T1AsyncComputedMisuse {
                #[async_computed(expr = fetch())]
                remote: i32,
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        expect_unconditional_item_local_error(
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct),
            &["#[async_computed]", "viewmodel/store"],
        );
    }

    /// PR #169 review remediation, T2 (round 1); updated round 2 (AD-R2-3, see T1's own doc comment
    /// for the shape of the change): `#[bindable]` on a field whose declared type is not
    /// `Rc<..>`-wrapped is decidable from the field's own declared type text alone — no sibling
    /// module lookup involved (contrast `bindable_field_on_non_viewmodel_type_is_rejected_on_macro_path`
    /// above, which genuinely needs `table.resolve` to know whether the *pointee* type is a
    /// viewmodel, and stays registry-dependent) — so it is `ItemLocal`, surfaced as an unconditional
    /// `compile_error!`.
    #[test]
    fn t2_non_rc_bindable_field_is_an_unconditional_item_local_error() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct T2NonRcBindable {
                #[bindable]
                vm: SomeViewModelNotWrappedInRc,
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        expect_unconditional_item_local_error(
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct),
            &["#[bindable]", "Rc<..>"],
        );
    }

    /// PR #169 review remediation, T3 (round 1); updated round 2 (AD-R2-3, see T1's own doc comment
    /// for the shape of the change): a Component with its own `body: view! { .. }` but no
    /// `inherits <Base>` is decidable from the struct alone — no sibling module lookup involved — so
    /// it is `ItemLocal`, surfaced as an unconditional `compile_error!`.
    #[test]
    fn t3_base_less_component_with_own_view_is_an_unconditional_item_local_error() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct T3BaseLessWithView {
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        expect_unconditional_item_local_error(
            generate_component_from_item_struct(None, &item_struct),
            &["must declare", "inherits"],
        );
    }
}

/// `#[elwindui::component] impl Name { .. }` — §3's `#[overridable]`/`#[overrides]` method
/// inheritance on the Rust-macro path (`generate_component_methods_from_item_impl`). The
/// downstream half (`codegen::resolve_effective_methods`'s `__base_<name>` shadows,
/// `rewrite_base_calls`, `validate`'s signature check) is already covered by `parser.rs`/
/// `validate.rs`'s own tests against the DSL text form; these cover the macro front door and the
/// struct-before-impl pairing it depends on.
///
/// Component names are unique per test for the same reason `dsl_enum_tests` does it — the
/// same-crate registries are process-global statics shared by every test in this binary.
#[cfg(test)]
mod component_impl_tests {
    use super::*;

    fn declare(base: Option<&str>, src: &str) {
        let item_struct: syn::ItemStruct = syn::parse_str(src).expect("struct should parse");
        generate_component_from_item_struct(base.map(str::to_string), &item_struct)
            .expect("struct half should generate");
    }

    fn methods(src: &str) -> Result<proc_macro2::TokenStream, String> {
        let item_impl: syn::ItemImpl = syn::parse_str(src).expect("impl should parse");
        generate_component_from_item_impl(&item_impl)
    }

    /// The `struct` half emits no *real* item at all now — every real token comes from the `impl`
    /// half. Issue #146: it does now unconditionally emit a `cfg(rust_analyzer)`-only shadow, so this
    /// checks for the absence of an unconditional/`cfg(not(rust_analyzer))` real item instead of bare
    /// emptiness.
    #[test]
    fn the_struct_half_emits_nothing() {
        let item_struct: syn::ItemStruct =
            syn::parse_str(r#"struct MiSilent {}"#).expect("struct should parse");
        let out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should succeed")
            .to_string();
        assert!(
            out.contains("cfg (rust_analyzer)"),
            "struct half should still emit its rust-analyzer shadow: {out}"
        );
        assert!(
            !out.contains("cfg (not (rust_analyzer))"),
            "struct half should emit no real (unconditional) item: {out}"
        );
    }

    #[test]
    fn overridable_method_is_emitted_as_a_public_inherent_method() {
        declare(None, r#"struct MiBase {}"#);
        let out = methods(
            r#"
            impl MiBase {
                #[overridable]
                fn label(&self) -> String { "base".to_string() }
            }
            "#,
        )
        .expect("impl half should generate")
        .to_string();
        assert!(
            out.contains("struct MiBase"),
            "the impl half emits the whole type: {out}"
        );
        assert!(out.contains("pub fn label"), "should emit `label`: {out}");
    }

    #[test]
    fn overrides_gets_a_base_shadow_and_a_rewritten_base_call() {
        declare(None, r#"struct MiSuper {}"#);
        methods(
            r#"
            impl MiSuper {
                #[overridable]
                fn label(&self) -> String { "super".to_string() }
            }
            "#,
        )
        .expect("base impl should generate");
        declare(
            Some("crate::MiSuper"),
            r#"struct MiDerived { body: view! { MiSuper { } }, }"#,
        );
        let out = methods(
            r#"
            impl MiDerived {
                #[overrides]
                fn label(&self) -> String { format!("{}!", base::label()) }
            }
            "#,
        )
        .expect("derived impl should generate")
        .to_string();
        assert!(
            out.contains("fn __base_label"),
            "base body should be kept as a private shadow: {out}"
        );
        assert!(
            out.contains("self . __base_label ()") || out.contains("self.__base_label()"),
            "`base::label()` should be rewritten onto the shadow: {out}"
        );
        assert_eq!(
            out.matches("fn __base_label").count(),
            1,
            "component-to-component inheritance must not emit duplicate base shadows: {out}"
        );
    }

    #[test]
    fn overrides_without_a_matching_overridable_is_rejected() {
        declare(None, r#"struct MiNoHook {}"#);
        declare(
            Some("crate::MiNoHook"),
            r#"struct MiNoHookChild { body: view! { MiNoHook { } }, }"#,
        );
        let err = expect_generation_error(methods(
            r#"
            impl MiNoHookChild {
                #[overrides]
                fn missing(&self) -> String { String::new() }
            }
            "#,
        ));
        assert!(err.contains("no matching"), "error: {err}");
    }

    #[test]
    fn signature_mismatch_against_the_base_is_rejected() {
        declare(None, r#"struct MiSigBase {}"#);
        methods(
            r#"
            impl MiSigBase {
                #[overridable]
                fn label(&self) -> String { String::new() }
            }
            "#,
        )
        .expect("base impl should generate");
        declare(
            Some("crate::MiSigBase"),
            r#"struct MiSigChild { body: view! { MiSigBase { } }, }"#,
        );
        let err = expect_generation_error(methods(
            r#"
            impl MiSigChild {
                #[overrides]
                fn label(&self, extra: i32) -> String { let _ = extra; String::new() }
            }
            "#,
        ));
        assert!(err.contains("different signature"), "error: {err}");
    }

    #[test]
    fn an_untagged_fn_is_rejected() {
        declare(None, r#"struct MiUntagged {}"#);
        let err = methods(r#"impl MiUntagged { fn helper(&self) -> String { String::new() } }"#)
            .expect_err("an untagged fn should be rejected");
        assert!(
            err.contains("#[overridable]") && err.contains("#[overrides]"),
            "error should name both tags: {err}"
        );
    }

    #[test]
    fn an_impl_before_its_struct_is_rejected() {
        let err = expect_generation_error(methods(
            r#"
            impl MiNeverDeclared {
                #[overridable]
                fn label(&self) -> String { String::new() }
            }
            "#,
        ));
        assert!(err.contains("declare the struct first"), "error: {err}");
    }

    #[test]
    fn a_trait_impl_is_rejected() {
        declare(None, r#"struct MiTraitImpl {}"#);
        let err = methods(r#"impl Clone for MiTraitImpl { fn clone(&self) -> Self { todo!() } }"#)
            .expect_err("a trait impl should be rejected");
        assert!(err.contains("trait impl"), "error: {err}");
    }

    /// PR #169 review remediation, T10: the real `impl` half (`codegen::generate_view`'s own
    /// `not(rust_analyzer)`-gated output, via `generate_component_from_item_impl`) and the
    /// rust-analyzer Component struct shadow (its `rust_analyzer`-gated output) must agree on which
    /// own `Option<T>` fields are required constructor parameters vs. deferred setter-only fields —
    /// because both now consult the same `component_frontend::component_public_shape` classification
    /// (AD-R4 of the PR #169 review contract), not two independently hand-written copies of it. Mirrors
    /// `examples/graphics-demo`'s own `GraphicsDemoWindow` shape (an ordinary unannotated required
    /// field plus a referenced-vs-unreferenced own `Option<T>` pair).
    #[test]
    fn t10_real_and_shadow_agree_on_own_field_constructor_deferred_classification() {
        let struct_item: syn::ItemStruct = syn::parse_str(
            r#"
            struct T10ShapeSharing {
                required_prop: i32,
                referenced_padding: Option<f32>,
                unreferenced_padding: Option<f32>,
                body: view! {
                    VerticalLayout {
                        Rectangle { corner_radius: referenced_padding }
                        TextBlock { text: "x" }
                    }
                },
            }
            "#,
        )
        .expect("struct should parse");
        // The struct half's own return value carries the rust-analyzer struct shadow
        // (`new(..)`/getters/setters) — captured directly here rather than through the shared
        // `declare` helper (used by every other test in this module), which discards it.
        let struct_out =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &struct_item)
                .expect("struct half should generate")
                .to_string();
        let impl_out = methods(r#"impl T10ShapeSharing {}"#)
            .expect("impl half should generate")
            .to_string();
        let out = format!("{struct_out} {impl_out}");

        // Real generation names this an ancestor-composed `construct(..)` (this component inherits
        // `VerticalLayout`, a shape-composition base); the shadow always names it `new(..)` — a
        // pre-existing, unrelated naming/formatting difference (the shadow also always trails every
        // parameter with a comma, real generation doesn't) this test doesn't care about. Both must
        // still agree on *which* fields appear as parameters at all.
        assert!(
            out.contains(
                "fn construct (required_prop : i32 , referenced_padding : Option < f32 >)"
            ),
            "real construct(..) must take required_prop and the referenced own Option<T> field, in \
             field order, with unreferenced_padding excluded: {out}"
        );
        assert!(
            out.contains(
                "pub fn new (required_prop : i32 , referenced_padding : Option < f32 > ,) -> std :: rc :: Rc < Self >"
            ),
            "shadow new(..) must classify the same two fields as required, in the same order: {out}"
        );

        // Both branches expose a setter for the deferred field, and — since this Component has a
        // `view!` and `referenced_padding` is a plain `prop` (runtime-mutable by definition even
        // though also required at construction time, `codegen::generate_view`'s own
        // `mutable_required_names`) — for the required-but-referenced field too.
        assert!(out.contains("fn set_unreferenced_padding"), "{out}");
        assert!(out.contains("fn set_referenced_padding"), "{out}");
    }

    /// PR #169 review remediation, round 2, T-R2-6 (AD-R2-5): a has-view Component's own
    /// `on_*`-named no-initializer field (a `#[routed]`-style callback, wired through event
    /// handling — `codegen::generate_view`'s own `param_names` exclusion is purely name-based, not
    /// attribute-based) must not become a positional constructor parameter, in either the real
    /// generator or the rust-analyzer shadow — both consult `component_public_shape`'s own
    /// exclusion (AD-R2-5), so this test fails if either independently reintroduces it.
    #[test]
    fn t_r2_6_own_on_prefixed_field_excluded_from_real_and_shadow_constructor() {
        let struct_item: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR26OnFieldExclusion {
                on_custom: fn(),
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        let struct_out =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &struct_item)
                .expect("struct half should generate")
                .to_string();
        let impl_out = methods(r#"impl TR26OnFieldExclusion {}"#)
            .expect("impl half should generate")
            .to_string();
        let out = format!("{struct_out} {impl_out}");

        assert!(
            !out.contains("on_custom : fn ()") || !out.contains("fn new"),
            "sanity: on_custom must not appear as a typed struct/shadow field storage entry: {out}"
        );
        // Neither the real `construct(..)`/`new(..)` positional list nor the shadow's own `new(..)`
        // may take `on_custom` as a parameter.
        let real_ctor = out
            .split("fn construct (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        assert!(
            !real_ctor.contains("on_custom"),
            "real constructor must not include the on_*-prefixed field: {real_ctor}"
        );
        let shadow_ctor = out
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("shadow constructor signature should be present");
        assert!(
            !shadow_ctor.contains("on_custom"),
            "shadow constructor must not include the on_*-prefixed field: {shadow_ctor}"
        );
    }

    /// PR #169 review remediation, round 2, T-R2-3 (AD-R2-3), end-to-end: a Component whose struct
    /// half triggers both an `ItemLocal` diagnostic (`#[async_computed]` misuse) and a
    /// `RegistryDependent` one (`#[content(name)]` naming an unknown field) in the same
    /// `validate::validate_classified` pass must surface *both* in the same generated output — the
    /// item-local one as an unconditional `compile_error!`, the registry-dependent one gated behind
    /// `#[cfg(not(rust_analyzer))]` — alongside the still-present rust-analyzer shadow. This fails on
    /// the pre-AD-R2-3 `classify_validate_result`, which collapsed both into a single `ItemLocal`
    /// verdict and returned a bare `Err`, discarding the shadow entirely.
    #[test]
    fn t_r2_3_end_to_end_mixed_diagnostics_both_gated_correctly_alongside_shadow() {
        // PR #169 review remediation, round 3 (A1/AD-R3-1, T-R3-3): a base-less `#[content(..)]`
        // typo is now correctly `ItemLocal` (see `t_r3_1_*` in `validate.rs`), so this end-to-end
        // mix needs a genuine same-crate-base-dependent `#[content(..)]` miss for its
        // registry-dependent half — declared and registered first, exactly like every other
        // cross-invocation same-crate registry test in this module.
        declare(
            None,
            r#"
            struct TR23EndToEndMixedBase {
                #[param]
                unrelated_field: String,
            }
            "#,
        );
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            #[content(missing_on_base)]
            struct TR23EndToEndMixed {
                #[async_computed(expr = fetch())]
                value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let generated = generate_component_from_item_struct(
            Some("TR23EndToEndMixedBase".to_string()),
            &item_struct,
        )
        .expect("a Classified failure must still be Ok(shadow + routed compile_error!s)")
        .to_string();

        // The shadow must still be present (not discarded the way a bare `Err` would).
        assert!(
            generated.contains("cfg (rust_analyzer)")
                && generated.contains("struct TR23EndToEndMixed"),
            "generated: {generated}"
        );

        // The item-local diagnostic (`#[async_computed]` misuse) is unconditional.
        let async_computed_error = generated
            .split("compile_error !")
            .find(|segment| segment.contains("async_computed"))
            .expect("async_computed diagnostic should be present");
        assert!(
            !async_computed_error.contains("cfg (not (rust_analyzer))"),
            "item-local diagnostic must not be gated: {generated}"
        );

        // The registry-dependent diagnostic (`#[content(..)]` naming a field missing from the
        // same-crate base) is gated.
        assert!(
            generated.contains("cfg (not (rust_analyzer))")
                && generated.contains("missing_on_base"),
            "registry-dependent diagnostic must be gated behind cfg(not(rust_analyzer)): {generated}"
        );
    }

    /// PR #169 review remediation, round 3, T-R3-5 (A2/AD-R3-2/AD-R3-3/AD-R3-4): a derived
    /// Component's inherited (not its own) field must keep being forwarded into the real generated
    /// constructor exactly as before, even though `component_public_shape` is now given only the
    /// derived's own literal fields (`source_component`) rather than the effective/flattened set —
    /// proving the source/effective split preserves inherited-field forwarding rather than silently
    /// dropping it (`codegen.rs`'s own `is_param_eligible` fallback for non-own fields).
    #[test]
    fn t_r3_5_generate_view_preserves_inherited_field_forwarding_with_source_local_shape() {
        declare(
            Some("VerticalLayout"),
            r#"
            struct TR35Base {
                #[param]
                base_value: i32,
                body: view! { TextBlock { text: "base" } },
            }
            "#,
        );
        methods(r#"impl TR35Base {}"#).expect("base impl half should generate");

        let derived_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR35Derived {
                #[param]
                own_value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        generate_component_from_item_struct(Some("crate::TR35Base".to_string()), &derived_struct)
            .expect("derived struct half should generate");
        let derived_impl: syn::ItemImpl =
            syn::parse_str("impl TR35Derived {}").expect("impl should parse");
        let generated = generate_component_from_item_impl(&derived_impl)
            .expect("derived impl half should generate")
            .to_string();

        let real_ctor = generated
            .split("fn construct (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        assert!(
            real_ctor.contains("base_value"),
            "the inherited field must still be forwarded into the real constructor: {real_ctor}"
        );
        assert!(
            real_ctor.contains("own_value"),
            "the derived's own field must still be a real constructor parameter: {real_ctor}"
        );
    }

    /// PR #169 review remediation, round 3, T-R3-6 (AD-R3-5): a required (referenced-by-view), own,
    /// unannotated `prop` field's setter comes from `own_shape.writable_fields` — both the real
    /// generated setter and the rust-analyzer shadow's own setter must still exist, sourced from the
    /// same shape instance `generate_view` now builds from `source_component`.
    #[test]
    fn t_r3_6_required_writable_prop_comes_from_shape() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR36RequiredWritableProp {
                value: i32,
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        let struct_out =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct)
                .expect("struct half should generate")
                .to_string();
        let impl_out = methods(r#"impl TR36RequiredWritableProp {}"#)
            .expect("impl half should generate")
            .to_string();
        let out = format!("{struct_out} {impl_out}");

        let real_ctor = out
            .split("fn construct (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        assert!(real_ctor.contains("value"), "real ctor: {real_ctor}");
        assert!(
            out.contains("fn set_value"),
            "real generation must expose a setter for a required, referenced, unannotated prop: {out}"
        );
        let shadow_ctor = out
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("shadow constructor signature should be present");
        assert!(shadow_ctor.contains("value"), "shadow ctor: {shadow_ctor}");
    }

    /// PR #169 review remediation, round 3, T-R3-7: a required `#[param]` field is immutable once
    /// constructed — no setter, in either real generation or the rust-analyzer shadow — since
    /// `component_public_shape` only pushes a required field's setter into `writable_fields` for
    /// `FieldKind::Prop`, never `FieldKind::Param`.
    #[test]
    fn t_r3_7_required_param_is_not_writable() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR37RequiredParamNotWritable {
                #[param]
                value: i32,
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .expect("struct should parse");
        let struct_out =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct)
                .expect("struct half should generate")
                .to_string();
        let impl_out = methods(r#"impl TR37RequiredParamNotWritable {}"#)
            .expect("impl half should generate")
            .to_string();
        let out = format!("{struct_out} {impl_out}");

        assert!(
            out.contains("fn value"),
            "a #[param] field still has a getter: {out}"
        );
        assert!(
            !out.contains("fn set_value"),
            "a required #[param] field must have no setter anywhere in real or shadow output: {out}"
        );
    }

    /// PR #169 review remediation, round 3, T-R3-8 (AD-R3-6): a view-less Component's real
    /// (`generate_component`) and rust-analyzer-shadow public surfaces must agree exactly on
    /// constructor params, getter names, and setter names for every own field kind — required
    /// unannotated `prop`, deferred `Option<T>` `prop`, defaulted `prop`, and `#[param]`.
    #[test]
    fn t_r3_8_view_less_accessor_parity_across_field_kinds() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR38ViewLessParity {
                required_prop: i32,
                deferred_prop: Option<i32>,
                #[prop(default = 1)]
                defaulted_prop: i32,
                #[param]
                required_param: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let struct_out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let impl_out = methods(r#"impl TR38ViewLessParity {}"#)
            .expect("impl half should generate")
            .to_string();
        let generated = format!("{struct_out} {impl_out}");

        let real_ctor = generated
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        let shadow_ctor = generated
            .split("pub fn new (")
            .nth(2)
            .and_then(|s| s.split(')').next())
            .expect("shadow constructor signature should be present");
        for ctor in [real_ctor, shadow_ctor] {
            assert!(ctor.contains("required_prop"), "ctor: {ctor}");
            assert!(ctor.contains("required_param"), "ctor: {ctor}");
            assert!(!ctor.contains("deferred_prop"), "ctor: {ctor}");
            assert!(!ctor.contains("defaulted_prop"), "ctor: {ctor}");
        }
        // Every field has a getter; only the deferred/defaulted own fields have setters (a
        // view-less required field — Prop or Param — never gets a setter, matching real
        // generation's own long-standing view-less-path behavior).
        for name in [
            "required_prop",
            "deferred_prop",
            "defaulted_prop",
            "required_param",
        ] {
            assert!(
                generated.contains(&format!("fn {name}")),
                "missing getter for {name}: {generated}"
            );
        }
        assert!(generated.contains("fn set_deferred_prop"), "{generated}");
        assert!(generated.contains("fn set_defaulted_prop"), "{generated}");
        assert!(!generated.contains("fn set_required_prop"), "{generated}");
        assert!(!generated.contains("fn set_required_param"), "{generated}");
        // Both real and shadow are view-less, so both return bare `Self`.
        assert!(
            generated.matches("-> Self").count() >= 2,
            "both real and shadow constructors should return Self: {generated}"
        );
    }

    /// PR #169 review remediation, round 4, T-R4-1 (AD-R4-1/AD-R4-3/AD-R4-4): a view-less
    /// component's required (non-deferred, no-initializer) own field's constructor/getter
    /// membership comes from `component_public_shape`'s own `constructor_params`/`readable_fields`
    /// — checked directly against the same shape instance real generation consumes, not just
    /// indirectly through the generated output.
    #[test]
    fn t_r4_1_view_less_required_prop_consumes_constructor_and_readable_shape() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR41Required {
                value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        let shape = component_frontend::component_public_shape(&component_def, view_def.as_ref());
        assert!(
            shape.constructor_params.iter().any(|(n, _)| n == "value"),
            "{:?}",
            shape.constructor_params
        );
        assert!(
            shape.readable_fields.iter().any(|(n, _, _)| n == "value"),
            "{:?}",
            shape.readable_fields
        );
        assert!(
            !shape.writable_fields.iter().any(|(n, _, _)| n == "value"),
            "{:?}",
            shape.writable_fields
        );

        let struct_out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let impl_out = methods(r#"impl TR41Required {}"#)
            .expect("impl half should generate")
            .to_string();
        let generated = format!("{struct_out} {impl_out}");
        let real_ctor = generated
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        assert!(real_ctor.contains("value"), "{real_ctor}");
        assert!(generated.contains("pub fn value"), "{generated}");
        assert!(!generated.contains("fn set_value"), "{generated}");
    }

    /// PR #169 review remediation, round 4, T-R4-2 (AD-R4-1/AD-R4-6/AD-R4-5): a view-less
    /// component's deferred `Option<T>` own field is excluded from `constructor_params`, present in
    /// `deferred_option_fields`/`readable_fields`/`writable_fields`, and its real generated setter
    /// takes the inner `T` — never `Option<T>`.
    #[test]
    fn t_r4_2_view_less_deferred_option_consumes_all_relevant_shape_surfaces() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR42Deferred {
                value: Option<String>,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        let shape = component_frontend::component_public_shape(&component_def, view_def.as_ref());
        assert!(
            !shape.constructor_params.iter().any(|(n, _)| n == "value"),
            "{:?}",
            shape.constructor_params
        );
        assert!(
            shape
                .deferred_option_fields
                .iter()
                .any(|(n, declared_ty, inner_ty)| n == "value"
                    && declared_ty == "Option<String>"
                    && inner_ty == "String"),
            "{:?}",
            shape.deferred_option_fields
        );
        assert!(
            shape
                .readable_fields
                .iter()
                .any(|(n, ty, _)| n == "value" && ty == "Option<String>"),
            "{:?}",
            shape.readable_fields
        );
        assert!(
            shape
                .writable_fields
                .iter()
                .any(|(n, ty, _)| n == "value" && ty == "String"),
            "{:?}",
            shape.writable_fields
        );

        let generated = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let real_ctor = generated
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        assert!(!real_ctor.contains("value"), "{real_ctor}");
        assert!(
            generated.contains("fn value (& self) -> Option < String >"),
            "{generated}"
        );
        let setter = generated
            .split("fn set_value (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("setter signature should be present");
        assert!(
            setter.contains("value : String") && !setter.contains("Option"),
            "setter parameter must be the inner T, not Option<T>: {setter}"
        );
    }

    /// PR #169 review remediation, round 4, T-R4-3 (AD-R4-4): a `#[state(default = ..)]` own
    /// field's getter/setter visibility comes from `ShadowVisibility::Private` (the shape), not an
    /// independent `FieldKind::State` check — both accessors must be non-`pub`.
    #[test]
    fn t_r4_3_defaulted_state_visibility_follows_shape() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR43State {
                #[state(default = 0)]
                state_value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let generated = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        assert!(
            generated.contains("fn state_value") && !generated.contains("pub fn state_value"),
            "{generated}"
        );
        assert!(
            generated.contains("fn set_state_value")
                && !generated.contains("pub fn set_state_value"),
            "{generated}"
        );
    }

    /// PR #169 review remediation, round 4, T-R4-4 (AD-R4-4): a `#[prop(default = ..)]` own field
    /// stays public read-write, and its recompute/property-change notification behavior (an
    /// implementation detail `ComponentPublicShape` has no concept of) is unaffected.
    #[test]
    fn t_r4_4_defaulted_prop_remains_public_read_write() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR44DefaultedProp {
                #[prop(default = 0)]
                value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let struct_out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let impl_out = methods(r#"impl TR44DefaultedProp {}"#)
            .expect("impl half should generate")
            .to_string();
        let generated = format!("{struct_out} {impl_out}");
        assert!(generated.contains("pub fn value"), "{generated}");
        assert!(generated.contains("pub fn set_value"), "{generated}");
        assert!(
            generated.contains("on_property_changed"),
            "property-change notification must still fire: {generated}"
        );
    }

    /// PR #169 review remediation, round 4, T-R4-5 (AD-R4-1/AD-R4-3/AD-R4-5): a `#[param]` own
    /// field is a constructor argument with a public getter and, per the shape, definitively no
    /// setter — matching the RA shadow's own (already shape-driven) surface.
    #[test]
    fn t_r4_5_param_remains_constructor_and_getter_only() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR45Param {
                #[param]
                value: String,
            }
            "#,
        )
        .expect("struct should parse");
        let struct_out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let impl_out = methods(r#"impl TR45Param {}"#)
            .expect("impl half should generate")
            .to_string();
        let generated = format!("{struct_out} {impl_out}");
        let real_ctor = generated
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("real constructor signature should be present");
        let shadow_ctor = generated
            .split("pub fn new (")
            .nth(2)
            .and_then(|s| s.split(')').next())
            .expect("shadow constructor signature should be present");
        assert!(real_ctor.contains("value"), "{real_ctor}");
        assert!(shadow_ctor.contains("value"), "{shadow_ctor}");
        assert!(generated.contains("pub fn value"), "{generated}");
        assert!(!generated.contains("fn set_value"), "{generated}");
    }

    /// PR #169 review remediation, round 5, T-R5-1 (AD-R5-1/AD-R5-3/AD-R5-4): a deferred own
    /// field's setter parameter type must come from `writable_fields`'s own type entry (the inner
    /// `T`), not merely happen to equal it via `strip_option`/`deferred_option_fields` — checked
    /// directly against the shape instance, and against the real generated setter signature.
    #[test]
    fn t_r5_1_deferred_setter_parameter_comes_from_writable_shape_entry() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR51Deferred {
                value: Option<String>,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        let shape = component_frontend::component_public_shape(&component_def, view_def.as_ref());
        assert!(
            shape
                .readable_fields
                .iter()
                .any(|(n, ty, _)| n == "value" && ty == "Option<String>"),
            "{:?}",
            shape.readable_fields
        );
        assert!(
            shape
                .writable_fields
                .iter()
                .any(|(n, ty, _)| n == "value" && ty == "String"),
            "{:?}",
            shape.writable_fields
        );

        let generated = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let setter = generated
            .split("fn set_value (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("setter signature should be present");
        assert_eq!(
            setter.trim(),
            "& self , value : String",
            "setter parameter must come from writable_fields's own String entry, not Option<String>: {setter}"
        );
    }

    /// PR #169 review remediation, round 5, T-R5-2 (AD-R5-1/AD-R5-3/AD-R5-5): a defaulted `prop`
    /// field's setter parameter type comes from `writable_fields`, not `#ty` directly.
    #[test]
    fn t_r5_2_defaulted_prop_setter_uses_writable_type() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR52DefaultedProp {
                #[prop(default = String::new())]
                value: String,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        let shape = component_frontend::component_public_shape(&component_def, view_def.as_ref());
        assert!(
            shape
                .writable_fields
                .iter()
                .any(|(n, ty, _)| n == "value" && ty == "String"),
            "{:?}",
            shape.writable_fields
        );

        let struct_out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let impl_out = methods(r#"impl TR52DefaultedProp {}"#)
            .expect("impl half should generate")
            .to_string();
        let generated = format!("{struct_out} {impl_out}");
        let setter = generated
            .split("fn set_value (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("setter signature should be present");
        assert!(
            setter.contains("value : String"),
            "setter parameter must come from writable_fields: {setter}"
        );
    }

    /// PR #169 review remediation, round 5, T-R5-3 (AD-R5-1/AD-R5-3/AD-R5-5): a `#[state(default =
    /// ..)]` field's setter parameter type comes from `writable_fields`, and its visibility (also
    /// from `writable_fields`'s `ShadowVisibility`) stays private.
    #[test]
    fn t_r5_3_state_setter_uses_writable_type_and_private_visibility() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR53State {
                #[state(default = 0)]
                value: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        let shape = component_frontend::component_public_shape(&component_def, view_def.as_ref());
        assert!(
            shape
                .writable_fields
                .iter()
                .any(|(n, ty, visibility)| n == "value"
                    && ty == "i32"
                    && matches!(visibility, component_frontend::ShadowVisibility::Private)),
            "{:?}",
            shape.writable_fields
        );

        let generated = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        assert!(
            generated.contains("fn set_value (& self , value : i32)")
                && !generated.contains("pub fn set_value"),
            "{generated}"
        );
    }

    /// PR #169 review remediation, round 5, T-R5-4 (AD-R5-1): every legal own writable field
    /// category's real setter name/type/visibility matches `shape.writable_fields` exactly —
    /// deferred `prop`, deferred `#[param]` (a same-crate `Option<T>` `#[param]` field is deferred
    /// the same way an unannotated `prop` is — `component_public_shape`'s own deferral branch does
    /// not distinguish `FieldKind::Param`/`Prop` for the deferred case), defaulted `prop`, and
    /// defaulted `#[state]`.
    #[test]
    fn t_r5_4_all_own_writable_categories_match_shape_exactly() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR54AllWritable {
                deferred_prop: Option<i32>,
                #[param]
                deferred_param: Option<String>,
                #[prop(default = 1)]
                defaulted_prop: i32,
                #[state(default = String::new())]
                defaulted_state: String,
            }
            "#,
        )
        .expect("struct should parse");
        let (component_def, view_def) =
            component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        let shape = component_frontend::component_public_shape(&component_def, view_def.as_ref());

        let struct_out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should generate")
            .to_string();
        let impl_out = methods(r#"impl TR54AllWritable {}"#)
            .expect("impl half should generate")
            .to_string();
        let generated = format!("{struct_out} {impl_out}");

        let expected: &[(&str, &str, component_frontend::ShadowVisibility)] = &[
            (
                "deferred_prop",
                "i32",
                component_frontend::ShadowVisibility::Public,
            ),
            (
                "deferred_param",
                "String",
                component_frontend::ShadowVisibility::Public,
            ),
            (
                "defaulted_prop",
                "i32",
                component_frontend::ShadowVisibility::Public,
            ),
            (
                "defaulted_state",
                "String",
                component_frontend::ShadowVisibility::Private,
            ),
        ];
        for (name, ty, visibility) in expected {
            assert!(
                shape
                    .writable_fields
                    .iter()
                    .any(|(n, t, v)| n == name && t == ty && v == visibility),
                "shape missing/mismatched entry for {name}: {:?}",
                shape.writable_fields
            );
            let setter = generated
                .split(&format!("fn set_{name} ("))
                .nth(1)
                .and_then(|s| s.split(')').next())
                .unwrap_or_else(|| panic!("setter for {name} should be present: {generated}"));
            assert!(
                setter.contains(&format!("value : {ty}")),
                "setter for {name} must use the shape's own type: {setter}"
            );
            let is_pub = generated.contains(&format!("pub fn set_{name}"));
            let expected_pub = matches!(visibility, component_frontend::ShadowVisibility::Public);
            assert_eq!(
                is_pub, expected_pub,
                "setter visibility for {name} must match the shape: {generated}"
            );
        }
    }
}

/// Issue #84: exercises `#[elwindui::environment_key]` + `#[environment(name)]` end to end through
/// the exact same path production code uses — register the Key via
/// `environment_frontend::generate_environment_key_from_item_struct`, then build a sibling
/// component declaring `#[environment(name)]`, confirming both the registry lookup
/// (`component_frontend::lookup_same_crate_environment_key`, `validate.rs` rule 34) and the
/// generated construction/subscription code (`codegen.rs`'s `own_environment_*` machinery)
/// succeed. Names are unique per test for the same reason `dsl_enum_tests` uses unique names
/// (`component_frontend`'s same-crate registries are process-global statics shared by every test
/// in this binary).
#[cfg(test)]
mod environment_key_tests {
    use super::*;

    fn register_environment_key(src: &str, args: &str) {
        let item_struct: syn::ItemStruct = syn::parse_str(src).expect("struct should parse");
        let args: proc_macro2::TokenStream = args.parse().expect("args should parse");
        environment_frontend::generate_environment_key_from_item_struct(args, &item_struct)
            .expect("environment key generation should succeed");
    }

    #[test]
    fn component_field_resolves_registered_key_and_generates_construction_and_subscription() {
        register_environment_key(
            "pub struct EnvKeyTestLocaleA;",
            "name = env_key_test_locale_a, value = String, default = String::from(\"en-US\")",
        );
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct EnvKeyTestScreenA {
                #[environment(env_key_test_locale_a)]
                locale: String,
                body: view! {
                    TextBlock { text: locale }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let out =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct);
        assert!(out.is_ok(), "expected success, got: {:?}", out.err());
        let item_impl: syn::ItemImpl =
            syn::parse_str("impl EnvKeyTestScreenA {}").expect("impl should parse");
        let generated = generate_component_from_item_impl(&item_impl)
            .expect("impl half should generate")
            .to_string();
        assert!(
            generated.contains("application_environment ()"),
            "mount() should bridge with application_environment(), not an ambient read (CI-6 of #80): {generated}"
        );
        assert!(
            generated.contains("__mount_environment . get ()"),
            "the environment field should resolve from __mount_environment, populated by mount() (CI-5 of #80), not a second independent ambient read: {generated}"
        );
        assert!(
            generated.contains(". get :: < EnvKeyTestLocaleA > ()"),
            "should call get::<KeyType>(): {generated}"
        );
        assert!(
            generated.contains(". subscribe :: < EnvKeyTestLocaleA > ("),
            "should subscribe to the Key's cell for live updates: {generated}"
        );
        assert!(
            generated.contains("__refresh_dynamic_regions"),
            "the subscription callback should refresh dynamic regions on change: {generated}"
        );
        // Never a constructor argument (docs/specs/dsl_spec.md §4).
        assert!(
            !generated.contains("pub fn new (locale"),
            "an #[environment(..)] field must not become a new() parameter: {generated}"
        );
    }

    #[test]
    fn unregistered_key_name_is_rejected() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct EnvKeyTestScreenB {
                #[environment(env_key_test_never_registered)]
                locale: String,
                body: view! {
                    TextBlock { text: locale }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err = expect_generation_error(generate_component_from_item_struct(
            Some("VerticalLayout".to_string()),
            &item_struct,
        ));
        assert!(
            err.contains("env_key_test_never_registered") && err.contains("isn't declared"),
            "error: {err}"
        );
    }

    #[test]
    fn combining_environment_with_param_is_rejected() {
        register_environment_key(
            "pub struct EnvKeyTestLocaleC;",
            "name = env_key_test_locale_c, value = String, default = String::from(\"en-US\")",
        );
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct EnvKeyTestScreenC {
                #[param]
                #[environment(env_key_test_locale_c)]
                locale: String,
                body: view! {
                    TextBlock { text: locale }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let err =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct)
                .expect_err("#[environment] combined with #[param] should be rejected");
        assert!(err.contains("cannot be combined"), "error: {err}");
    }

    /// Issue #129: `#[environment(some_crate::name)]` must bypass the same-crate registry
    /// entirely — `some_crate_env_key_test_never_registered_here` is deliberately never
    /// registered by this test file's own `register_environment_key`, proving rule 34's
    /// same-crate check (exercised by `unregistered_key_name_is_rejected` above) does not fire
    /// for the qualified form. The real cross-crate integration coverage (an actual second crate,
    /// compiled and run for real) lives in `crates/elwindui/tests/environment_field_cross_crate.rs`.
    #[test]
    fn qualified_cross_crate_key_bypasses_the_same_crate_registry() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct EnvKeyTestScreenD {
                #[environment(some_crate::some_crate_env_key_test_never_registered_here)]
                locale: String,
                body: view! {
                    TextBlock { text: locale }
                },
            }
            "#,
        )
        .expect("struct should parse");
        let out =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct);
        assert!(
            out.is_ok(),
            "a qualified cross-crate #[environment(..)] must not be rejected by the same-crate \
             registry check: {:?}",
            out.err()
        );
        let item_impl: syn::ItemImpl =
            syn::parse_str("impl EnvKeyTestScreenD {}").expect("impl should parse");
        let generated = generate_component_from_item_impl(&item_impl)
            .expect("impl half should generate")
            .to_string();
        // Not a direct absolute-path macro call (`some_crate :: __elwindui_environment_key_..! ()`
        // spliced straight into `.get::<..>()`) — that form trips rustc's deny-by-default
        // `macro_expanded_macro_exports_accessed_by_absolute_paths` lint for a macro-expanded
        // `macro_export` macro referenced from other macro-expanded code (confirmed by an isolated
        // multi-crate repro; see `environment_key_type`'s own doc comment). Instead: a `use`-import
        // of the bare macro name, a local `type` alias invoking it, then the alias used bare.
        assert!(
            generated.contains(
                "use some_crate :: __elwindui_environment_key_some_crate_env_key_test_never_registered_here ;"
            ),
            "should use-import the declaring crate's exported macro by bare name: {generated}"
        );
        assert!(
            generated.contains(
                "type __ElwindEnvKeyAlias_locale = __elwindui_environment_key_some_crate_env_key_test_never_registered_here ! () ;"
            ),
            "should alias a bare (unqualified) invocation of the imported macro to a local type: {generated}"
        );
        assert!(
            generated.contains(". get :: < __ElwindEnvKeyAlias_locale > ()"),
            "should use the local alias type, not the macro call directly, at the use site: {generated}"
        );
    }
}

/// Reproduction scaffolding for `Derived inherits <user component>` (Refs #23's investigation,
/// Refs #25's fix). Unlike `component_impl_tests` above (which only asserts `Ok(())`/error
/// *strings*, discarding the generated token text), these tests inspect the actual generated
/// `#[elwindui::class(inherits = ..)]` argument — that blind spot is exactly what let #25's bug
/// (a bare, unqualified name for a user-defined base, silently rejected only once real code tried
/// to compile it) slip past this same test module for a full release cycle.
#[cfg(test)]
mod user_base_inherits_tests {
    use super::*;

    fn declare(base: Option<&str>, src: &str) -> Result<(), String> {
        let item_struct: syn::ItemStruct = syn::parse_str(src).expect("struct should parse");
        generate_component_from_item_struct(base.map(str::to_string), &item_struct).map(|_| ())
    }

    fn build(src: &str) -> Result<proc_macro2::TokenStream, String> {
        let item_impl: syn::ItemImpl = syn::parse_str(src).expect("impl should parse");
        generate_component_from_item_impl(&item_impl)
    }

    /// `inherits crate::UbBase` (a fully crate-root-qualified path, as `#25`'s fix now requires
    /// for a user-defined base): `UbDerived` must build, and — the actual regression check — its
    /// generated `#[elwindui::class(inherits = ..)]` argument must carry that same qualified path
    /// verbatim, not the bare `UbBase` `codegen::base_trait_path` used to emit before the fix
    /// (which `elwindui_macros::class::validate_fully_qualified_path` would reject the moment this
    /// token stream was ever fed through the real `#[class]` proc macro).
    #[test]
    fn derived_from_a_user_component_builds_with_a_qualified_path() {
        declare(
            Some("ContentControl"),
            r#"struct UbBase { template: template_view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("base struct");
        build(r#"impl UbBase { }"#).expect("base impl");
        declare(
            Some("crate::UbBase"),
            r#"struct UbDerived { template: template_view! { UbBase { } }, }"#,
        )
        .expect("derived struct");
        let out = build(r#"impl UbDerived { }"#)
            .expect("derived impl should generate")
            .to_string();
        assert!(
            out.contains("inherits = crate :: UbBase"),
            "expected the qualified path to survive into `#[elwindui::class(inherits = ..)]` \
             verbatim: {out}"
        );
    }

    /// The exact failure Issue #25 reported: a *bare* name for a user-defined base. Must now be
    /// rejected up front, with a diagnostic that names the actual problem (a missing qualified
    /// path) — not left to surface later as `elwindui_macros::class`'s own internal-sounding
    /// `__elwindui_inherit_*!` error once the generated (broken) code is actually compiled.
    #[test]
    fn derived_from_a_user_component_with_a_bare_base_name_is_rejected() {
        declare(
            Some("ContentControl"),
            r#"struct UbBareBase { template: template_view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("base struct");
        build(r#"impl UbBareBase { }"#).expect("base impl");
        declare(
            Some("UbBareBase"),
            r#"struct UbBareDerived { template: template_view! { UbBareBase { } }, }"#,
        )
        .expect("derived struct");
        let err = expect_generation_error(build(r#"impl UbBareDerived { }"#));
        assert!(
            err.contains("crate::ui::UbBareBase") || err.contains("crate::UbBareBase"),
            "error should suggest a qualified path: {err}"
        );
    }

    /// Regression guard: a *builtin* base must keep emitting its old, bare-but-fully-resolvable
    /// `elwindui::ui::X` form — #25's fix only changes user-defined bases, never builtin ones.
    #[test]
    fn derived_from_a_builtin_base_still_emits_the_builtin_path() {
        declare(
            Some("ContentControl"),
            r#"struct UbBuiltinDerived { template: template_view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("derived struct");
        let out = build(r#"impl UbBuiltinDerived { }"#)
            .expect("derived impl should generate")
            .to_string();
        assert!(
            out.contains("inherits = elwindui :: ui :: ContentControl"),
            "builtin base should stay fully-qualified via the existing `elwindui::ui::` rule: {out}"
        );
    }
}
