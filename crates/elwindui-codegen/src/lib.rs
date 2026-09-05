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
use syn::visit::Visit as _;

fn append_tokens<T: quote::ToTokens>(out: &mut TokenStream, value: &T) {
    quote::ToTokens::to_tokens(value, out);
}

fn grouped_tokens(delimiter: proc_macro2::Delimiter, tokens: TokenStream) -> TokenStream {
    std::iter::once(proc_macro2::TokenTree::Group(proc_macro2::Group::new(
        delimiter, tokens,
    )))
    .collect()
}

fn generic_type(prefix: TokenStream, target_type: &TokenStream) -> TokenStream {
    let mut output = prefix;
    output.extend(quote! { < });
    append_tokens(&mut output, target_type);
    output.extend(quote! { > });
    output
}

/// Stable compile-time token used by the generic `template_view!` property bridge.  This is a
/// 64-bit FNV-1a-style hash of the field-name literal, calculated during code generation; it is a
/// code-generation key, not a runtime property lookup.  The generated component implements the
/// corresponding `TemplateProperty<KEY>` instance and the standalone factory carries the same
/// literal key in its trait bound.  There is no runtime registry or string lookup.  If two
/// distinct properties in one target collide, Rust reports the duplicate `TemplateProperty<KEY>`
/// implementation/associated-type conflict at compile time; the collision is never resolved
/// silently.
#[doc(hidden)]
pub const fn template_property_key(name: &str) -> u64 {
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
/// Low-level expansion used by the public lambda-style `template_view!` proc macro.
pub fn generate_template_view_expression(input: TokenStream) -> Result<TokenStream, String> {
    let invocation = parser::parse_template_view_invocation(input)
        .map_err(|error| format!("invalid `template_view!` header: {error}"))?;
    let target_type = match &invocation.header.target {
        ast::TemplateTarget::Concrete(target) => quote! { #target },
        ast::TemplateTarget::SelfType => {
            return Err(
                "standalone `template_view!` requires a concrete target type; `Self` is only valid in a component default template"
                    .into(),
            )
        }
    };
    let (on_mount, on_unmount, on_update, lets, parsed_root) =
        parser::parse_view_body(&invocation.body.to_string()).map_err(|error| {
            format!("invalid `template_view!(|alias: Target| {{ ... }})` body: {error}")
        })?;
    let validation_view = ast::ViewDef {
        target: "__standalone_template_view".to_string(),
        is_template: true,
        on_mount: on_mount.clone(),
        on_unmount: on_unmount.clone(),
        template_header: Some(invocation.header.clone()),
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
        target_type.clone(),
        invocation.header.parent_alias.clone(),
        HashSet::new(),
    )?;
    let factory = emit_compiled_template_factory(&compiled, target_type, true);
    Ok(quote! { { #factory } })
}

/// The semantic result of compiling a template body.  Component defaults and expression-form
/// `template_view!` values use this
/// representation before wrapping it in their respective factory/declaration shells.  Keeping
/// the root, lifecycle, dependency, and lexical-binding output together prevents a frontend from
/// quietly implementing a second property/dynamic/lifecycle compiler.
pub(crate) struct CompiledTemplateBody {
    root: TokenStream,
    let_statements: TokenStream,
    captured_names: Vec<syn::Ident>,
    refresh: TokenStream,
    property_bounds: Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
    writable_properties: BTreeSet<u64>,
    iterable_properties: BTreeSet<u64>,
    on_mount: Option<TokenStream>,
    on_unmount: Option<TokenStream>,
    on_update: Option<TokenStream>,
    lifecycle_keys: BTreeSet<u64>,
    has_deferred_views: bool,
    requires_parent: bool,
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
    parent_alias: String,
    bare_parent_fields: HashSet<String>,
) -> Result<CompiledTemplateBody, String> {
    validate_template_parent_alias_shadowing(
        body,
        lets,
        on_mount,
        on_unmount,
        on_update,
        &parent_alias,
    )?;
    let lowered = codegen::lower_template_body(
        body,
        lets,
        on_mount,
        on_unmount,
        on_update,
        &from,
        &table,
        target_type.clone(),
        parent_alias.clone(),
        bare_parent_fields.clone(),
    )?;
    let captured_names = collect_template_capture_names(
        body,
        lets,
        on_mount,
        on_unmount,
        on_update,
        &parent_alias,
        &bare_parent_fields,
    );
    let template_parent_ident = format_ident!("__elwindui_template_parent");
    let on_mount_tokens = on_mount.map(|block| {
        codegen::emit_template_event_closure_body_for_target_with_fields(
            &ast::ClosureBody::Block(block.clone()),
            &[],
            &template_parent_ident,
            &lowered.property_bounds,
            target_type.clone(),
            parent_alias.clone(),
            bare_parent_fields.clone(),
        )
    });
    let on_unmount_tokens = on_unmount.map(|block| {
        codegen::emit_template_event_closure_body_for_target_with_fields(
            &ast::ClosureBody::Block(block.clone()),
            &[],
            &template_parent_ident,
            &lowered.property_bounds,
            target_type.clone(),
            parent_alias.clone(),
            bare_parent_fields.clone(),
        )
    });
    let on_update_tokens = on_update.map(|hook| {
        codegen::emit_template_event_closure_body_for_target_with_fields(
            &ast::ClosureBody::Block(hook.block.clone()),
            &[],
            &template_parent_ident,
            &lowered.property_bounds,
            target_type.clone(),
            parent_alias.clone(),
            bare_parent_fields.clone(),
        )
    });
    let mut lifecycle_keys = BTreeSet::new();
    if let Some(block) = on_mount {
        codegen::collect_template_rust_block_property_keys(
            block,
            &parent_alias,
            &mut lifecycle_keys,
        );
    }
    if let Some(block) = on_unmount {
        codegen::collect_template_rust_block_property_keys(
            block,
            &parent_alias,
            &mut lifecycle_keys,
        );
    }
    if let Some(hook) = on_update {
        codegen::collect_template_rust_block_property_keys(
            &hook.block,
            &parent_alias,
            &mut lifecycle_keys,
        );
        if let Some(fields) = &hook.fields {
            lifecycle_keys.extend(
                fields
                    .iter()
                    .map(|field| crate::template_property_key(field)),
            );
        }
    }
    Ok(CompiledTemplateBody {
        root: lowered.root,
        let_statements: lowered.let_statements,
        captured_names,
        refresh: lowered.refresh,
        property_bounds: lowered.property_bounds,
        writable_properties: lowered.writable_properties,
        iterable_properties: lowered.iterable_properties,
        on_mount: on_mount_tokens,
        on_unmount: on_unmount_tokens,
        on_update: on_update_tokens,
        lifecycle_keys,
        has_deferred_views: lowered.has_deferred_views,
        requires_parent: lowered.requires_parent,
    })
}

/// Classifies whether a component base is known to be Control-derived without consulting the
/// same-crate component registry.
enum ControlTargetKnowledge {
    KnownControl,
    KnownNonControl,
    NeedsSameCrateRegistry,
}

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

fn same_crate_control_target(name: &str) -> Option<bool> {
    fn visit(name: &str, visited: &mut HashSet<String>) -> Option<bool> {
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

    visit(name, &mut HashSet::new())
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

/// Rejects bindings that would hide the explicitly declared template-parent alias.  This is a
/// structural reservation check: ordinary Rust expression rewriting still owns lexical scope
/// resolution, while this pass only visits binding positions and reports the public template
/// diagnostic before lowering can accidentally reinterpret the alias as a local value.
pub(crate) fn validate_template_parent_alias_shadowing(
    body: &ast::ViewBody,
    lets: &[ast::LetBinding],
    on_mount: Option<&syn::Block>,
    on_unmount: Option<&syn::Block>,
    on_update: Option<&ast::OnUpdateHook>,
    alias: &str,
) -> Result<(), String> {
    let error = || {
        format!(
            "template parent alias `{alias}` cannot be shadowed inside this template; choose a different local name"
        )
    };

    struct BindingVisitor<'a> {
        alias: &'a str,
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for BindingVisitor<'_> {
        fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
            if pattern.ident == self.alias {
                self.found = true;
            }
            syn::visit::visit_pat_ident(self, pattern);
        }
    }

    fn block_contains_binding(block: &syn::Block, alias: &str) -> bool {
        let mut visitor = BindingVisitor {
            alias,
            found: false,
        };
        syn::visit::Visit::visit_block(&mut visitor, block);
        visitor.found
    }

    fn expr_contains_binding(expr: &syn::Expr, alias: &str) -> bool {
        let mut visitor = BindingVisitor {
            alias,
            found: false,
        };
        syn::visit::Visit::visit_expr(&mut visitor, expr);
        visitor.found
    }

    fn pat_contains_binding(pattern: &syn::Pat, alias: &str) -> bool {
        let mut visitor = BindingVisitor {
            alias,
            found: false,
        };
        syn::visit::Visit::visit_pat(&mut visitor, pattern);
        visitor.found
    }

    fn pattern_contains_binding(pattern: &str, alias: &str) -> bool {
        syn::parse::Parser::parse_str(syn::Pat::parse_single, pattern)
            .map(|pattern| pat_contains_binding(&pattern, alias))
            .unwrap_or(false)
    }

    fn view_body_contains_binding(
        body: &ast::ViewBody,
        lets: &[ast::LetBinding],
        on_mount: Option<&syn::Block>,
        on_unmount: Option<&syn::Block>,
        on_update: Option<&ast::OnUpdateHook>,
        alias: &str,
    ) -> bool {
        if lets.iter().any(|binding| binding.name == alias)
            || on_mount.is_some_and(|block| block_contains_binding(block, alias))
            || on_unmount.is_some_and(|block| block_contains_binding(block, alias))
            || on_update.is_some_and(|hook| block_contains_binding(&hook.block, alias))
        {
            return true;
        }
        body.attributes
            .iter()
            .any(|attribute| view_expr_contains_binding(&attribute.value, alias))
            || body
                .attached
                .iter()
                .any(|(_, _, value)| view_expr_contains_binding(value, alias))
            || body
                .children
                .iter()
                .any(|child| child_contains_binding(child, alias))
    }

    fn view_expr_contains_binding(expr: &ast::ViewExpr, alias: &str) -> bool {
        match expr {
            ast::ViewExpr::Path(_) => false,
            ast::ViewExpr::TFluent(_, args) => args
                .iter()
                .any(|(_, value)| view_expr_contains_binding(value, alias)),
            ast::ViewExpr::Expr(expr) => expr_contains_binding(expr, alias),
            ast::ViewExpr::Closure { params, body } => {
                params.iter().any(|param| param == alias)
                    || match body {
                        ast::ClosureBody::Expr(expr) => view_expr_contains_binding(expr, alias),
                        ast::ClosureBody::Element(element) => {
                            element_contains_binding(element, alias)
                        }
                        ast::ClosureBody::Block(block) => block_contains_binding(block, alias),
                    }
            }
            ast::ViewExpr::Element(element) => element_contains_binding(element, alias),
            ast::ViewExpr::DeferredView(deferred) => view_body_contains_binding(
                &deferred.body.root,
                &deferred.body.lets,
                deferred.body.on_mount.as_ref(),
                deferred.body.on_unmount.as_ref(),
                deferred.body.on_update.as_ref(),
                alias,
            ),
        }
    }

    fn element_contains_binding(element: &ast::ElementNode, alias: &str) -> bool {
        element
            .attributes
            .iter()
            .any(|attribute| view_expr_contains_binding(&attribute.value, alias))
            || element
                .attached
                .iter()
                .any(|(_, _, value)| view_expr_contains_binding(value, alias))
            || element
                .children
                .iter()
                .any(|child| child_contains_binding(child, alias))
    }

    fn child_contains_binding(child: &ast::ChildEntry, alias: &str) -> bool {
        match child {
            ast::ChildEntry::Literal(element) => element_contains_binding(element, alias),
            ast::ChildEntry::Ref(_) => false,
            ast::ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            } => {
                view_expr_contains_binding(condition, alias)
                    || then_branch
                        .iter()
                        .chain(else_branch)
                        .any(|child| child_contains_binding(child, alias))
            }
            ast::ChildEntry::Match { value, arms } => {
                view_expr_contains_binding(value, alias)
                    || arms.iter().any(|arm| {
                        pattern_contains_binding(&arm.pattern, alias)
                            || arm
                                .body
                                .iter()
                                .any(|child| child_contains_binding(child, alias))
                    })
            }
            ast::ChildEntry::For {
                binding,
                collection,
                body,
            } => {
                binding == alias
                    || view_expr_contains_binding(collection, alias)
                    || body
                        .iter()
                        .any(|child| child_contains_binding(child, alias))
            }
        }
    }

    if view_body_contains_binding(body, lets, on_mount, on_unmount, on_update, alias) {
        Err(error())
    } else {
        Ok(())
    }
}

/// Finds ordinary Rust values referenced by a template body so the generated `Fn` factory can
/// clone them once per build before the refresh callback moves its local copy.  A
/// `ControlTemplate`/`ViewFactory` factory is callable more than once; moving a call-site capture
/// directly into the nested `move` refresh callback would otherwise make an otherwise valid
/// `Fn` factory fail with E0507 (for example, a reusable template that captures a `String`).
///
/// This deliberately scans only real Rust expressions.  DSL paths are resolved by the shared
/// template lowerer (`owner.field`, bare parent fields, and view-model paths) and therefore must
/// not be treated as ambient captures here.  Binding names are collected in a separate pass so
/// generated `let`/`for` locals and Rust closure/block locals are not cloned before they exist.
fn collect_template_capture_names(
    body: &ast::ViewBody,
    lets: &[ast::LetBinding],
    on_mount: Option<&syn::Block>,
    on_unmount: Option<&syn::Block>,
    on_update: Option<&ast::OnUpdateHook>,
    parent_alias: &str,
    bare_parent_fields: &HashSet<String>,
) -> Vec<syn::Ident> {
    fn collect_block_bindings(block: &syn::Block, bindings: &mut HashSet<String>) {
        struct BindingCollector<'a> {
            bindings: &'a mut HashSet<String>,
        }

        impl<'ast> syn::visit::Visit<'ast> for BindingCollector<'_> {
            fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
                self.bindings.insert(pattern.ident.to_string());
                syn::visit::visit_pat_ident(self, pattern);
            }
        }

        BindingCollector { bindings }.visit_block(block);
    }

    fn collect_pattern_bindings(pattern: &str, bindings: &mut HashSet<String>) {
        if let Ok(pattern) = syn::parse::Parser::parse_str(syn::Pat::parse_single, pattern) {
            struct BindingCollector<'a> {
                bindings: &'a mut HashSet<String>,
            }

            impl<'ast> syn::visit::Visit<'ast> for BindingCollector<'_> {
                fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
                    self.bindings.insert(pattern.ident.to_string());
                    syn::visit::visit_pat_ident(self, pattern);
                }
            }

            BindingCollector { bindings }.visit_pat(&pattern);
        }
    }

    fn collect_view_expr_bindings(expr: &ast::ViewExpr, bindings: &mut HashSet<String>) {
        match expr {
            ast::ViewExpr::Path(_) => {}
            ast::ViewExpr::TFluent(_, args) => {
                for (_, value) in args {
                    collect_view_expr_bindings(value, bindings);
                }
            }
            ast::ViewExpr::Expr(expr) => collect_block_or_expr_bindings(expr, bindings),
            ast::ViewExpr::Closure { params, body } => {
                bindings.extend(params.iter().cloned());
                match body {
                    ast::ClosureBody::Expr(expr) => collect_view_expr_bindings(expr, bindings),
                    ast::ClosureBody::Element(element) => {
                        collect_element_bindings(element, bindings)
                    }
                    ast::ClosureBody::Block(block) => collect_block_bindings(block, bindings),
                }
            }
            ast::ViewExpr::Element(element) => collect_element_bindings(element, bindings),
            ast::ViewExpr::DeferredView(deferred) => {
                collect_view_body_bindings(
                    &deferred.body.root,
                    &deferred.body.lets,
                    deferred.body.on_mount.as_ref(),
                    deferred.body.on_unmount.as_ref(),
                    deferred.body.on_update.as_ref(),
                    bindings,
                );
            }
        }
    }

    fn collect_block_or_expr_bindings(expr: &syn::Expr, bindings: &mut HashSet<String>) {
        struct BindingCollector<'a> {
            bindings: &'a mut HashSet<String>,
        }

        impl<'ast> syn::visit::Visit<'ast> for BindingCollector<'_> {
            fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
                self.bindings.insert(pattern.ident.to_string());
                syn::visit::visit_pat_ident(self, pattern);
            }
        }

        BindingCollector { bindings }.visit_expr(expr);
    }

    fn collect_element_bindings(element: &ast::ElementNode, bindings: &mut HashSet<String>) {
        for attribute in &element.attributes {
            collect_view_expr_bindings(&attribute.value, bindings);
        }
        for (_, _, value) in &element.attached {
            collect_view_expr_bindings(value, bindings);
        }
        for child in &element.children {
            collect_child_bindings(child, bindings);
        }
    }

    fn collect_view_body_bindings(
        body: &ast::ViewBody,
        lets: &[ast::LetBinding],
        on_mount: Option<&syn::Block>,
        on_unmount: Option<&syn::Block>,
        on_update: Option<&ast::OnUpdateHook>,
        bindings: &mut HashSet<String>,
    ) {
        bindings.extend(lets.iter().map(|binding| binding.name.clone()));
        for binding in lets {
            collect_element_bindings(&binding.element, bindings);
        }
        if let Some(block) = on_mount {
            collect_block_bindings(block, bindings);
        }
        if let Some(block) = on_unmount {
            collect_block_bindings(block, bindings);
        }
        if let Some(hook) = on_update {
            collect_block_bindings(&hook.block, bindings);
        }
        for attribute in &body.attributes {
            collect_view_expr_bindings(&attribute.value, bindings);
        }
        for (_, _, value) in &body.attached {
            collect_view_expr_bindings(value, bindings);
        }
        for child in &body.children {
            collect_child_bindings(child, bindings);
        }
    }

    fn collect_child_bindings(child: &ast::ChildEntry, bindings: &mut HashSet<String>) {
        match child {
            ast::ChildEntry::Literal(element) => collect_element_bindings(element, bindings),
            ast::ChildEntry::Ref(_) => {}
            ast::ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            } => {
                collect_view_expr_bindings(condition, bindings);
                for child in then_branch.iter().chain(else_branch) {
                    collect_child_bindings(child, bindings);
                }
            }
            ast::ChildEntry::Match { value, arms } => {
                collect_view_expr_bindings(value, bindings);
                for arm in arms {
                    collect_pattern_bindings(&arm.pattern, bindings);
                    for child in &arm.body {
                        collect_child_bindings(child, bindings);
                    }
                }
            }
            ast::ChildEntry::For {
                binding,
                collection,
                body,
            } => {
                bindings.insert(binding.clone());
                collect_view_expr_bindings(collection, bindings);
                for child in body {
                    collect_child_bindings(child, bindings);
                }
            }
        }
    }

    struct CandidateCollector<'a> {
        names: &'a mut Vec<String>,
        seen: &'a mut HashSet<String>,
    }

    impl CandidateCollector<'_> {
        fn add_name(&mut self, name: String) {
            if self.seen.insert(name.clone()) {
                self.names.push(name);
            }
        }

        fn add_path(&mut self, path: &syn::Path) {
            if let Some(ident) = path.get_ident() {
                self.add_name(ident.to_string());
            }
        }
    }

    impl<'ast> syn::visit::Visit<'ast> for CandidateCollector<'_> {
        fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
            self.add_path(&expression.path);
            syn::visit::visit_expr_path(self, expression);
        }

        fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
            // A bare function path is a callee, not a captured value.  Visit arguments explicitly
            // so `make_value(prefix)` still records `prefix`.
            for argument in &expression.args {
                self.visit_expr(argument);
            }
        }

        fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
            let macro_name = expression
                .mac
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string());
            if !matches!(
                macro_name.as_deref(),
                Some("format" | "format_args" | "vec")
            ) {
                return;
            }
            let arguments = syn::parse::Parser::parse2(
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
                expression.mac.tokens.clone(),
            );
            let Ok(arguments) = arguments else {
                return;
            };
            if matches!(macro_name.as_deref(), Some("format" | "format_args")) {
                if let Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(format),
                    ..
                })) = arguments.first()
                {
                    for name in template_format_inline_idents(&format.value()) {
                        self.add_name(name);
                    }
                }
            }
            for argument in arguments {
                if let syn::Expr::Assign(assign) = &argument {
                    self.visit_expr(&assign.right);
                } else {
                    self.visit_expr(&argument);
                }
            }
        }
    }

    fn collect_view_expr_candidates(
        expr: &ast::ViewExpr,
        names: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        match expr {
            ast::ViewExpr::Path(_) => {}
            ast::ViewExpr::TFluent(_, args) => {
                for (_, value) in args {
                    collect_view_expr_candidates(value, names, seen);
                }
            }
            ast::ViewExpr::Expr(expr) => CandidateCollector { names, seen }.visit_expr(expr),
            ast::ViewExpr::Closure { body, .. } => match body {
                ast::ClosureBody::Expr(expr) => collect_view_expr_candidates(expr, names, seen),
                ast::ClosureBody::Element(element) => {
                    collect_element_candidates(element, names, seen)
                }
                ast::ClosureBody::Block(block) => {
                    CandidateCollector { names, seen }.visit_block(block)
                }
            },
            ast::ViewExpr::Element(element) => collect_element_candidates(element, names, seen),
            ast::ViewExpr::DeferredView(deferred) => collect_view_body_candidates(
                &deferred.body.root,
                &deferred.body.lets,
                deferred.body.on_mount.as_ref(),
                deferred.body.on_unmount.as_ref(),
                deferred.body.on_update.as_ref(),
                names,
                seen,
            ),
        }
    }

    fn collect_element_candidates(
        element: &ast::ElementNode,
        names: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        for attribute in &element.attributes {
            collect_view_expr_candidates(&attribute.value, names, seen);
        }
        for (_, _, value) in &element.attached {
            collect_view_expr_candidates(value, names, seen);
        }
        for child in &element.children {
            collect_child_candidates(child, names, seen);
        }
    }

    fn collect_view_body_candidates(
        body: &ast::ViewBody,
        lets: &[ast::LetBinding],
        on_mount: Option<&syn::Block>,
        on_unmount: Option<&syn::Block>,
        on_update: Option<&ast::OnUpdateHook>,
        names: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        for binding in lets {
            collect_element_candidates(&binding.element, names, seen);
        }
        for block in [on_mount, on_unmount] {
            if let Some(block) = block {
                CandidateCollector { names, seen }.visit_block(block);
            }
        }
        if let Some(hook) = on_update {
            CandidateCollector { names, seen }.visit_block(&hook.block);
        }
        for attribute in &body.attributes {
            collect_view_expr_candidates(&attribute.value, names, seen);
        }
        for (_, _, value) in &body.attached {
            collect_view_expr_candidates(value, names, seen);
        }
        for child in &body.children {
            collect_child_candidates(child, names, seen);
        }
    }

    fn collect_child_candidates(
        child: &ast::ChildEntry,
        names: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        match child {
            ast::ChildEntry::Literal(element) => collect_element_candidates(element, names, seen),
            ast::ChildEntry::Ref(_) => {}
            ast::ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            } => {
                collect_view_expr_candidates(condition, names, seen);
                for child in then_branch.iter().chain(else_branch) {
                    collect_child_candidates(child, names, seen);
                }
            }
            ast::ChildEntry::Match { value, arms } => {
                collect_view_expr_candidates(value, names, seen);
                for arm in arms {
                    for child in &arm.body {
                        collect_child_candidates(child, names, seen);
                    }
                }
            }
            ast::ChildEntry::For {
                collection, body, ..
            } => {
                collect_view_expr_candidates(collection, names, seen);
                for child in body {
                    collect_child_candidates(child, names, seen);
                }
            }
        }
    }

    fn template_format_inline_idents(value: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut chars = value.chars().peekable();
        while let Some(character) = chars.next() {
            if character != '{' {
                if character == '}' && chars.peek() == Some(&'}') {
                    chars.next();
                }
                continue;
            }
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
            if !name.is_empty()
                && (name
                    .starts_with(|character: char| character.is_alphabetic() || character == '_'))
                && name
                    .chars()
                    .all(|character| character.is_alphanumeric() || character == '_')
            {
                names.push(name);
            }
        }
        names
    }

    let mut bindings = HashSet::new();
    collect_view_body_bindings(body, lets, on_mount, on_unmount, on_update, &mut bindings);

    let mut names = Vec::new();
    let mut seen = HashSet::new();
    collect_view_body_candidates(
        body, lets, on_mount, on_unmount, on_update, &mut names, &mut seen,
    );
    names
        .into_iter()
        .filter(|name| {
            !bindings.contains(name)
                && name != parent_alias
                && !bare_parent_fields.contains(name)
                && !matches!(
                    name.as_str(),
                    "self"
                        | "Self"
                        | "crate"
                        | "super"
                        | "this"
                        | "__environment"
                        | "__subscriptions"
                        | "__root"
                )
                // Uppercase single-segment paths conventionally denote `static`/`const` items,
                // not values owned by the caller.  In particular, cloning a `thread_local!`
                // `LocalKey` such as `TARGET_MOUNTS` is both unnecessary and invalid.
                && name.chars().any(|character| character.is_lowercase())
                && !name.starts_with("__elwindui_")
        })
        .map(|name| format_ident!("{name}"))
        .collect()
}

fn template_capture_clones(names: &[syn::Ident]) -> TokenStream {
    names
        .iter()
        .map(|name| quote! { let #name = #name.clone(); })
        .collect()
}

fn template_body_uses_ident(body: &TokenStream, wanted: &str) -> bool {
    struct IdentUse<'a> {
        wanted: &'a str,
        found: bool,
    }

    impl syn::visit::Visit<'_> for IdentUse<'_> {
        fn visit_expr_path(&mut self, expression: &syn::ExprPath) {
            if expression
                .path
                .get_ident()
                .is_some_and(|ident| ident == self.wanted)
            {
                self.found = true;
            }
            syn::visit::visit_expr_path(self, expression);
        }
    }

    let Ok(block) = syn::parse2::<syn::Block>(body.clone()) else {
        return true;
    };
    let mut visitor = IdentUse {
        wanted,
        found: false,
    };
    visitor.visit_block(&block);
    visitor.found
}

/// Emits the property-change subscriptions shared by every typed template factory.  Property
/// reads are lowered into `TemplateProperty<KEY>` accesses; the corresponding subscription merely
/// schedules the same refresh closure used by event wiring and dynamic regions.  Keeping this in
/// the factory layer means the semantic lowerer has no knowledge of how a `ControlTemplate` or a
/// deferred view owns its cleanup subscription vector.
fn emit_template_property_subscriptions(
    target_type: &TokenStream,
    property_bounds: &Rc<RefCell<BTreeMap<u64, Option<TokenStream>>>>,
) -> TokenStream {
    property_bounds
        .borrow()
        .keys()
        .copied()
        .map(|key| {
            let weak_parent = format_ident!("__elwindui_template_property_weak_{key}");
            let refresh_cell = format_ident!("__elwindui_template_property_refresh_cell_{key}");
            let mut parent_declaration = quote! { let parent: Option< };
            append_tokens(
                &mut parent_declaration,
                &generic_type(quote! { std::rc::Rc }, target_type),
            );
            parent_declaration.extend(quote! { > = #weak_parent.upgrade(); });
            let mut callback_body = TokenStream::new();
            append_tokens(&mut callback_body, &parent_declaration);
            callback_body.extend(quote! {
                if parent.is_some() {
                    if let Some(__elwindui_template_refresh_callback) =
                        #refresh_cell.borrow().as_ref().cloned()
                    {
                        __elwindui_template_refresh_callback();
                    }
                }
            });
            let mut callback = quote! { move || };
            callback.extend(grouped_tokens(proc_macro2::Delimiter::Brace, callback_body));
            let mut subscription_path = quote! { < };
            append_tokens(&mut subscription_path, target_type);
            subscription_path.extend(quote! {
                as elwindui::core::ui::TemplateProperty<#key>>::__template_subscribe
            });
            let mut subscription_arguments = quote! { &*__elwindui_template_parent, };
            append_tokens(&mut subscription_arguments, &callback);
            subscription_path.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                subscription_arguments,
            ));
            let mut push_arguments = TokenStream::new();
            append_tokens(&mut push_arguments, &subscription_path);
            let mut item_body = quote! { let #weak_parent: };
            append_tokens(
                &mut item_body,
                &generic_type(quote! { std::rc::Weak }, target_type),
            );
            item_body.extend(quote! {
                = std::rc::Rc::downgrade(&__elwindui_template_parent);
                let #refresh_cell = std::rc::Rc::clone(&__elwindui_template_refresh_cell);
                __subscriptions.borrow_mut().push
            });
            item_body.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                push_arguments,
            ));
            item_body.extend(quote! { ; });
            grouped_tokens(proc_macro2::Delimiter::Brace, item_body)
        })
        .collect()
}

/// Wraps a compiled semantic template body in a concrete `ControlTemplate<T>` factory. The
/// factory shell is intentionally kept separate from [`compile_template_body`]: the component
/// default and standalone expression frontends provide only their explicit target and declaration
/// shape, while construction, binding, dynamic regions, lifecycle hooks, and cleanup are emitted
/// once by the shared body compiler.
pub(crate) fn emit_compiled_template_factory(
    body: &CompiledTemplateBody,
    target_type: TokenStream,
    allow_environment_only: bool,
) -> TokenStream {
    // A template whose body never reads a property from the typed parent can still capture
    // ordinary Rust values. Keep that value-capturing path on `ControlTemplate<T>`'s
    // environment-only constructor; the parent-dependent path below needs the typed control
    // context for property subscriptions and resync.
    let parent_dependent = !body.property_bounds.borrow().is_empty()
        || !body.iterable_properties.is_empty()
        || !body.lifecycle_keys.is_empty()
        || !body.writable_properties.is_empty()
        || body.has_deferred_views
        || body.requires_parent;
    if allow_environment_only && !parent_dependent {
        let root = &body.root;
        let let_statements = &body.let_statements;
        let capture_clones = template_capture_clones(&body.captured_names);
        let on_mount_hook = body
            .on_mount
            .clone()
            .map(|body| {
                let uses_environment = template_body_uses_ident(&body, "__environment");
                let uses_subscriptions = template_body_uses_ident(&body, "__subscriptions");
                let mut outer_mount_state = TokenStream::new();
                if uses_environment {
                    outer_mount_state.extend(quote! {
                        let __template_mount_environment = __environment.clone();
                    });
                }
                if uses_subscriptions {
                    outer_mount_state.extend(quote! {
                        let __template_mount_subscriptions = __subscriptions.clone();
                    });
                }
                let mut callback_body = TokenStream::new();
                if uses_environment {
                    callback_body.extend(quote! {
                        let __environment = __template_mount_environment.clone();
                    });
                }
                if uses_subscriptions {
                    callback_body.extend(quote! {
                        let __subscriptions = __template_mount_subscriptions.clone();
                    });
                }
                append_tokens(&mut callback_body, &body);
                let mut closure = quote! { move || };
                closure.extend(grouped_tokens(proc_macro2::Delimiter::Brace, callback_body));
                let mut boxed_closure = quote! { Box::new };
                boxed_closure.extend(grouped_tokens(proc_macro2::Delimiter::Parenthesis, closure));
                let mut call_arguments = quote! { &*__root, };
                call_arguments.extend(boxed_closure);
                let mut mount_call = quote! { elwindui::core::ui::UIElementExt::add_mount_hook };
                mount_call.extend(grouped_tokens(
                    proc_macro2::Delimiter::Parenthesis,
                    call_arguments,
                ));
                mount_call.extend(quote! { ; });
                let mut outer_body = TokenStream::new();
                append_tokens(&mut outer_body, &outer_mount_state);
                append_tokens(&mut outer_body, &mount_call);
                grouped_tokens(proc_macro2::Delimiter::Brace, outer_body)
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
            elwindui::core::ui::ControlTemplate::<#target_type>::from_environment(move |__environment| {
                use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
                use elwindui::ui::*;
                let __subscriptions: std::rc::Rc<std::cell::RefCell<Vec<
                    elwindui::core::reactive::Subscription,
                >>> = std::rc::Rc::new(std::cell::RefCell::new(
                    Vec::<elwindui::core::reactive::Subscription>::new(),
                ));
                #capture_clones
                #let_statements
                let __root: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #root;
                #on_mount_hook
                #on_unmount_hook
                __root
            })
        };
    }

    let root = &body.root;
    let let_statements = &body.let_statements;
    let capture_clones = template_capture_clones(&body.captured_names);
    let refresh = &body.refresh;
    let property_subscriptions =
        emit_template_property_subscriptions(&target_type, &body.property_bounds);
    let on_mount_hook = body
        .on_mount
        .clone()
        .map(|_body| {
            let uses_parent = template_body_uses_ident(&_body, "this");
            let uses_environment = template_body_uses_ident(&_body, "__environment");
            let uses_subscriptions = template_body_uses_ident(&_body, "__subscriptions");
            let weak_parent = format_ident!("__elwindui_template_mount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            let mut mount_body = TokenStream::new();
            if uses_parent {
                mount_body.extend(quote! {
                    let this: std::rc::Rc<#target_type> = #parent.clone();
                });
            }
            if uses_environment {
                mount_body.extend(quote! {
                    let __environment = __template_mount_environment.clone();
                });
            }
            if uses_subscriptions {
                mount_body.extend(quote! {
                    let __subscriptions = __template_mount_subscriptions.clone();
                });
            }
            append_tokens(&mut mount_body, &_body);
            let mut outer_mount_state = TokenStream::new();
            if uses_environment {
                outer_mount_state.extend(quote! {
                    let __template_mount_environment = __environment.clone();
                });
            }
            if uses_subscriptions {
                outer_mount_state.extend(quote! {
                    let __template_mount_subscriptions = __subscriptions.clone();
                });
            }
            let mount_guard = if uses_parent {
                let mut guard = quote! { if let Some(#parent) = mount_parent };
                guard.extend(grouped_tokens(proc_macro2::Delimiter::Brace, mount_body));
                guard
            } else {
                let mut guard = quote! { if mount_parent.is_some() };
                guard.extend(grouped_tokens(proc_macro2::Delimiter::Brace, mount_body));
                guard
            };
            let mut callback_body = quote! {
                let mount_parent: Option<std::rc::Rc<#target_type>> =
                    #weak_parent.upgrade();
            };
            append_tokens(&mut callback_body, &mount_guard);
            let mut closure = quote! { move || };
            closure.extend(grouped_tokens(proc_macro2::Delimiter::Brace, callback_body));
            let mut boxed_closure = quote! { Box::new };
            boxed_closure.extend(grouped_tokens(proc_macro2::Delimiter::Parenthesis, closure));
            let mut call_arguments = quote! { &*__root, };
            call_arguments.extend(boxed_closure);
            let mut mount_call = quote! { elwindui::core::ui::UIElementExt::add_mount_hook };
            mount_call.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                call_arguments,
            ));
            mount_call.extend(quote! { ; });
            let mut outer_body = quote! {
                let #weak_parent: std::rc::Weak<#target_type> =
                    std::rc::Rc::downgrade(&#parent);
            };
            append_tokens(&mut outer_body, &outer_mount_state);
            append_tokens(&mut outer_body, &mount_call);
            grouped_tokens(proc_macro2::Delimiter::Brace, outer_body)
        })
        .unwrap_or_default();
    let update_subscriptions: TokenStream = if body.on_update.is_some() {
        let mut update_subscriptions = TokenStream::new();
        for key in &body.lifecycle_keys {
            let weak_parent = format_ident!("__elwindui_template_update_weak_{key}");
            let mut update_guard_body = quote! {
                let this: std::rc::Rc<#target_type> =
                    __elwindui_template_parent.clone();
                let _ = &this;
            };
            update_guard_body.extend(body.on_update.clone().expect("update body checked above"));
            let mut update_guard = quote! {
                if let Some(__elwindui_template_parent) = update_parent
            };
            update_guard.extend(grouped_tokens(
                proc_macro2::Delimiter::Brace,
                update_guard_body,
            ));
            let mut callback_body = quote! {
                let update_parent: Option<std::rc::Rc<#target_type>> =
                    #weak_parent.upgrade();
            };
            append_tokens(&mut callback_body, &update_guard);
            let mut callback = quote! { move || };
            callback.extend(grouped_tokens(proc_macro2::Delimiter::Brace, callback_body));
            let mut subscription_arguments = quote! { &*__elwindui_template_parent, };
            append_tokens(&mut subscription_arguments, &callback);
            let mut subscription = quote! {
                <#target_type as elwindui::core::ui::TemplateProperty<#key>>::__template_subscribe
            };
            subscription.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                subscription_arguments,
            ));
            let mut push_arguments = TokenStream::new();
            append_tokens(&mut push_arguments, &subscription);
            let mut item_body = quote! {
                let #weak_parent: std::rc::Weak<#target_type> =
                    std::rc::Rc::downgrade(&__elwindui_template_parent);
                __subscriptions.borrow_mut().push
            };
            item_body.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                push_arguments,
            ));
            item_body.extend(quote! { ; });
            update_subscriptions.extend(grouped_tokens(proc_macro2::Delimiter::Brace, item_body));
        }
        update_subscriptions
    } else {
        TokenStream::new()
    };
    let on_unmount_hook = body.on_unmount.clone().map(|body| {
        let weak_parent = format_ident!("__elwindui_template_unmount_weak");
        let parent = format_ident!("__elwindui_template_parent");
        quote! {
            {
                let #weak_parent: std::rc::Weak<#target_type> =
                    std::rc::Rc::downgrade(&#parent);
                elwindui::core::ui::UIElementExt::add_unmount_hook(
                    &*__root,
                    Box::new(move || {
                        let unmount_parent: Option<std::rc::Rc<#target_type>> =
                            #weak_parent.upgrade();
                        if let Some(#parent) = unmount_parent {
                            let this: std::rc::Rc<#target_type> = #parent.clone();
                            #body
                        }
                    }),
                );
            }
        }
    });
    let on_unmount_hook = on_unmount_hook.unwrap_or_default();
    let mut template_body = quote! {
            use elwindui::core::ui::{ControlExt as _, UIElementExt as _};
            use elwindui::ui::*;
            let __elwindui_template_parent: std::rc::Rc<#target_type> = context.control.clone();
            let __environment = context.environment.clone();
            let __subscriptions: std::rc::Rc<std::cell::RefCell<Vec<
                elwindui::core::reactive::Subscription,
            >>> = std::rc::Rc::new(std::cell::RefCell::new(
                Vec::<elwindui::core::reactive::Subscription>::new(),
            ));
            let __elwindui_template_refresh_cell: std::rc::Rc<std::cell::RefCell<
                Option<std::rc::Rc<dyn Fn()>>,
            >> = std::rc::Rc::new(
                std::cell::RefCell::new(None::<std::rc::Rc<dyn Fn()>>),
            );
            let __elwindui_template_refresh_parent: std::rc::Rc<#target_type> =
                __elwindui_template_parent.clone();
            let __elwindui_template_refresh_environment = __environment.clone();
            let __elwindui_template_refresh_cell_for_callback: std::rc::Rc<std::cell::RefCell<
                Option<std::rc::Rc<dyn Fn()>>,
            >> = std::rc::Rc::clone(&__elwindui_template_refresh_cell);
            let this: std::rc::Rc<#target_type> = __elwindui_template_parent.clone();
            let _ = &this;
            #capture_clones
            #let_statements
            let __root: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #root;
            let __elwindui_template_refresh_callback: std::rc::Rc<dyn Fn()> =
                std::rc::Rc::new(move || {
                        let __elwindui_template_parent: std::rc::Rc<#target_type> =
                            __elwindui_template_refresh_parent.clone();
                        let this: std::rc::Rc<#target_type> = __elwindui_template_parent.clone();
                        let _ = &this;
                    let __environment = __elwindui_template_refresh_environment.clone();
                    #refresh
                });
            *__elwindui_template_refresh_cell_for_callback.borrow_mut() =
                Some(__elwindui_template_refresh_callback);
    };
    template_body.extend(property_subscriptions);
    append_tokens(&mut template_body, &on_mount_hook);
    template_body.extend(update_subscriptions);
    template_body.extend(on_unmount_hook);
    template_body.extend(quote! {
            let __template_subscriptions_for_cleanup: std::rc::Rc<std::cell::RefCell<Vec<
                elwindui::core::reactive::Subscription,
            >>> = __subscriptions.clone();
            let __template_target_for_cleanup: std::rc::Rc<#target_type> =
                __elwindui_template_parent.clone();
            __template_target_for_cleanup.add_unmount_hook(Box::new(move || {
                __template_subscriptions_for_cleanup.borrow_mut().clear();
            }));
            __root
    });
    let mut factory_arguments = quote! { move |context| };
    factory_arguments.extend(std::iter::once(proc_macro2::TokenTree::Group(
        proc_macro2::Group::new(proc_macro2::Delimiter::Brace, template_body),
    )));
    let mut factory = quote! {
        elwindui::core::ui::ControlTemplate::<#target_type>::new
    };
    factory.extend(std::iter::once(proc_macro2::TokenTree::Group(
        proc_macro2::Group::new(proc_macro2::Delimiter::Parenthesis, factory_arguments),
    )));
    factory
}

/// Emits a `ViewFactory` factory for a deferred expression nested inside a ControlTemplate.  The
/// deferred value keeps the same semantic body backend as its enclosing template; only the outer
/// lifecycle context changes from `ControlTemplateContext` to `ViewBuildContext`.  The concrete
/// typed parent is captured at expression-construction time, avoiding any downcast or erased target
/// lookup when the deferred view is later opened.
fn emit_view_factory(
    body: &CompiledTemplateBody,
    target_type: TokenStream,
    parent: &syn::Ident,
) -> TokenStream {
    let root = &body.root;
    let let_statements = &body.let_statements;
    let capture_clones = template_capture_clones(&body.captured_names);
    let refresh = &body.refresh;
    let property_subscriptions =
        emit_template_property_subscriptions(&target_type, &body.property_bounds);
    let on_mount_hook = body
        .on_mount
        .clone()
        .map(|_body| {
            let uses_parent = template_body_uses_ident(&_body, "this");
            let uses_environment = template_body_uses_ident(&_body, "__environment");
            let uses_subscriptions = template_body_uses_ident(&_body, "__subscriptions");
            let weak_parent = format_ident!("__elwindui_deferred_mount_weak");
            let parent = format_ident!("__elwindui_template_parent");
            let mut mount_body = TokenStream::new();
            if uses_parent {
                mount_body.extend(quote! {
                    let this: std::rc::Rc<#target_type> = #parent.clone();
                });
            }
            if uses_environment {
                mount_body.extend(quote! {
                    let __environment = __deferred_mount_environment.clone();
                });
            }
            if uses_subscriptions {
                mount_body.extend(quote! {
                    let __subscriptions = __deferred_mount_subscriptions.clone();
                });
            }
            append_tokens(&mut mount_body, &_body);
            let mut outer_mount_state = TokenStream::new();
            if uses_environment {
                outer_mount_state.extend(quote! {
                    let __deferred_mount_environment = __environment.clone();
                });
            }
            if uses_subscriptions {
                outer_mount_state.extend(quote! {
                    let __deferred_mount_subscriptions = __subscriptions.clone();
                });
            }
            let mount_guard = if uses_parent {
                let mut guard = quote! { if let Some(#parent) = mount_parent };
                guard.extend(grouped_tokens(proc_macro2::Delimiter::Brace, mount_body));
                guard
            } else {
                let mut guard = quote! { if mount_parent.is_some() };
                guard.extend(grouped_tokens(proc_macro2::Delimiter::Brace, mount_body));
                guard
            };
            let mut callback_body = quote! {
                let mount_parent: Option<std::rc::Rc<#target_type>> =
                    #weak_parent.upgrade();
            };
            append_tokens(&mut callback_body, &mount_guard);
            let mut closure = quote! { move || };
            closure.extend(grouped_tokens(proc_macro2::Delimiter::Brace, callback_body));
            let mut boxed_closure = quote! { Box::new };
            boxed_closure.extend(grouped_tokens(proc_macro2::Delimiter::Parenthesis, closure));
            let mut call_arguments = quote! { &*__root, };
            call_arguments.extend(boxed_closure);
            let mut mount_call = quote! { elwindui::core::ui::UIElementExt::add_mount_hook };
            mount_call.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                call_arguments,
            ));
            mount_call.extend(quote! { ; });
            let mut outer_body = quote! {
                let #weak_parent: std::rc::Weak<#target_type> =
                    std::rc::Rc::downgrade(&#parent);
            };
            append_tokens(&mut outer_body, &outer_mount_state);
            append_tokens(&mut outer_body, &mount_call);
            grouped_tokens(proc_macro2::Delimiter::Brace, outer_body)
        })
        .unwrap_or_default();
    let update_subscriptions: TokenStream = if body.on_update.is_some() {
        let mut update_subscriptions = TokenStream::new();
        for key in &body.lifecycle_keys {
            let weak_parent = format_ident!("__elwindui_deferred_update_weak_{key}");
            let mut update_guard_body = quote! {
                let this: std::rc::Rc<#target_type> =
                    __elwindui_template_parent.clone();
                let _ = &this;
            };
            update_guard_body.extend(body.on_update.clone().expect("update body checked above"));
            let mut update_guard = quote! {
                if let Some(__elwindui_template_parent) = update_parent
            };
            update_guard.extend(grouped_tokens(
                proc_macro2::Delimiter::Brace,
                update_guard_body,
            ));
            let mut callback_body = quote! {
                let update_parent: Option<std::rc::Rc<#target_type>> =
                    #weak_parent.upgrade();
            };
            append_tokens(&mut callback_body, &update_guard);
            let mut callback = quote! { move || };
            callback.extend(grouped_tokens(proc_macro2::Delimiter::Brace, callback_body));
            let mut subscription_arguments = quote! { &*__elwindui_template_parent, };
            append_tokens(&mut subscription_arguments, &callback);
            let mut subscription = quote! {
                <#target_type as elwindui::core::ui::TemplateProperty<#key>>::__template_subscribe
            };
            subscription.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                subscription_arguments,
            ));
            let mut push_arguments = TokenStream::new();
            append_tokens(&mut push_arguments, &subscription);
            let mut item_body = quote! {
                let #weak_parent: std::rc::Weak<#target_type> =
                    std::rc::Rc::downgrade(&__elwindui_template_parent);
                __subscriptions.borrow_mut().push
            };
            item_body.extend(grouped_tokens(
                proc_macro2::Delimiter::Parenthesis,
                push_arguments,
            ));
            item_body.extend(quote! { ; });
            update_subscriptions.extend(grouped_tokens(proc_macro2::Delimiter::Brace, item_body));
        }
        update_subscriptions
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
                let #weak_parent: std::rc::Weak<#target_type> =
                    std::rc::Rc::downgrade(&#parent);
                    elwindui::core::ui::UIElementExt::add_unmount_hook(
                        &*__root,
                        Box::new(move || {
                            let unmount_parent: Option<std::rc::Rc<#target_type>> =
                                #weak_parent.upgrade();
                            if let Some(#parent) = unmount_parent {
                                let this: std::rc::Rc<#target_type> = #parent.clone();
                                #body
                            }
                        }),
                    );
                }
            }
        })
        .unwrap_or_default();
    let mut view_body = quote! {
                let __owner: Option<std::rc::Rc<dyn elwindui::core::ui::UIElementExt>> =
                    context.owner.upgrade();
                __owner?;
                let __deferred_parent: Option<std::rc::Rc<#target_type>> =
                    __deferred_parent_weak.upgrade();
                let __elwindui_template_parent: std::rc::Rc<#target_type> = __deferred_parent?;
                let __environment = context.environment.clone();
                let __subscriptions: std::rc::Rc<std::cell::RefCell<Vec<
                    elwindui::core::reactive::Subscription,
                >>> = std::rc::Rc::new(std::cell::RefCell::new(
                    Vec::<elwindui::core::reactive::Subscription>::new(),
                ));
                let __elwindui_template_refresh_cell: std::rc::Rc<std::cell::RefCell<
                    Option<std::rc::Rc<dyn Fn()>>,
                >> = std::rc::Rc::new(
                    std::cell::RefCell::new(None::<std::rc::Rc<dyn Fn()>>),
                );
                let __elwindui_template_refresh_parent: std::rc::Rc<#target_type> =
                    __elwindui_template_parent.clone();
                let __elwindui_template_refresh_environment = __environment.clone();
                let __elwindui_template_refresh_cell_for_callback: std::rc::Rc<
                    std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>,
                > = std::rc::Rc::clone(&__elwindui_template_refresh_cell);
                let this: std::rc::Rc<#target_type> = __elwindui_template_parent.clone();
                let _ = &this;
                #capture_clones
                #let_statements
                let __root: std::rc::Rc<dyn elwindui::core::ui::UIElementExt> = #root;
                let __elwindui_template_refresh_callback: std::rc::Rc<dyn Fn()> =
                    std::rc::Rc::new(move || {
                        let __elwindui_template_parent: std::rc::Rc<#target_type> =
                            __elwindui_template_refresh_parent.clone();
                        let this: std::rc::Rc<#target_type> = __elwindui_template_parent.clone();
                        let _ = &this;
                        let __environment = __elwindui_template_refresh_environment.clone();
                        #refresh
                    });
                *__elwindui_template_refresh_cell_for_callback.borrow_mut() =
                    Some(__elwindui_template_refresh_callback);
    };
    view_body.extend(property_subscriptions);
    append_tokens(&mut view_body, &on_mount_hook);
    view_body.extend(update_subscriptions);
    view_body.extend(on_unmount_hook);
    view_body.extend(quote! {
                let __deferred_subscriptions = __subscriptions.clone();
                elwindui::core::ui::UIElementExt::add_unmount_hook(
                    &*__root,
                    Box::new(move || {
                        __deferred_subscriptions.borrow_mut().clear();
                    }),
                );
                Some(__root)
    });
    let mut factory_arguments = quote! { move |context| };
    factory_arguments.extend(std::iter::once(proc_macro2::TokenTree::Group(
        proc_macro2::Group::new(proc_macro2::Delimiter::Brace, view_body),
    )));
    let mut factory = quote! { elwindui::core::ui::ViewFactory::new };
    factory.extend(std::iter::once(proc_macro2::TokenTree::Group(
        proc_macro2::Group::new(proc_macro2::Delimiter::Parenthesis, factory_arguments),
    )));
    let mut result = TokenStream::new();
    let mut outer_body = quote! {
        let __deferred_parent_weak: std::rc::Weak<#target_type> =
            std::rc::Rc::downgrade(&#parent);
    };
    outer_body.extend(factory);
    result.extend(std::iter::once(proc_macro2::TokenTree::Group(
        proc_macro2::Group::new(proc_macro2::Delimiter::Brace, outer_body),
    )));
    result
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
    } else if view_def.as_ref().is_some_and(|view| !view.is_template) && view_def.is_some() {
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
                "`{name}`: Control-derived components must declare visual chrome with `template: template_view!(|alias: Self| {{ ... }})`; `body: view! {{ ... }}` is ordinary component composition"
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
/// hidden_view_factory_component`) directly into `module.items`. A no-op when `module` has no
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
                format!("__ElwinduiViewFactoryInstanceFor{owner_type_name}_{ordinal}");
            lower_deferred_views_in_element_lets_and_body(
                &mut deferred.body.lets,
                &mut deferred.body.root,
                owner_type_name,
                implicit_owner_schema,
                ordinal,
                new_items,
            );
            let (hidden_component, hidden_view) = component_frontend::hidden_view_factory_component(
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

#[cfg(test)]
mod template_view_header_tests {
    use super::*;

    #[test]
    fn standalone_self_target_is_rejected_with_migration_guidance() {
        let error = generate_template_view_expression(quote! {
            |control: Self| { TextBlock {} }
        })
        .expect_err("standalone Self target should be rejected");
        assert_eq!(
            error,
            "standalone `template_view!` requires a concrete target type; `Self` is only valid in a component default template"
        );
    }

    #[test]
    fn declared_parent_alias_shadowing_uses_the_public_diagnostic() {
        let (_, _, _, lets, body) = parser::parse_view_body("for button in items { TextBlock {} }")
            .expect("template body should parse");
        let error =
            validate_template_parent_alias_shadowing(&body, &lets, None, None, None, "button")
                .expect_err("the declared alias must be reserved");
        assert_eq!(
            error,
            "template parent alias `button` cannot be shadowed inside this template; choose a different local name"
        );
    }

    #[test]
    fn unrelated_local_binding_does_not_shadow_declared_parent_alias() {
        let (_, _, _, lets, body) = parser::parse_view_body("for item in items { TextBlock {} }")
            .expect("template body should parse");
        validate_template_parent_alias_shadowing(&body, &lets, None, None, None, "button")
            .expect("an unrelated local name is allowed");
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

    /// The real `impl` half and the rust-analyzer Component struct shadow must agree on the public
    /// shape for ordinary Props, including `Option<T>` Props.  In particular, an unannotated field
    /// is a defaulted mutable Prop, regardless of whether the view references it; both generated
    /// surfaces therefore omit it from the fixed constructor and expose a full-type setter.
    #[test]
    fn ordinary_props_are_not_constructor_parameters_in_real_or_shadow_output() {
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
        // `VerticalLayout`, a shape-composition base); the shadow always names it `new(..)`. Both
        // must agree that ordinary Props are not fixed constructor inputs.
        assert!(
            out.contains("fn construct ()"),
            "real construct(..) must have no ordinary Prop parameters: {out}"
        );
        assert!(
            out.contains("pub fn new () -> std :: rc :: Rc < Self >"),
            "shadow new(..) must have no ordinary Prop parameters: {out}"
        );
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

    /// An unannotated own field is an ordinary mutable Prop.  Its default is implicit, so it is not
    /// a fixed constructor input even when the component's view references it.
    #[test]
    fn t_r3_6_ordinary_writable_prop_comes_from_shape() {
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
        assert!(!real_ctor.contains("value"), "real ctor: {real_ctor}");
        assert!(
            out.contains("fn set_value"),
            "real generation must expose a setter for an ordinary Prop: {out}"
        );
        let shadow_ctor = out
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("shadow constructor signature should be present");
        assert!(!shadow_ctor.contains("value"), "shadow ctor: {shadow_ctor}");
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

    /// A view-less Component's real (`generate_component`) and rust-analyzer-shadow public surfaces
    /// must agree on constructor params, getter names, and setter names for ordinary Props and a
    /// required `#[param]`.
    #[test]
    fn t_r3_8_view_less_accessor_parity_across_field_kinds() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR38ViewLessParity {
                required_prop: i32,
                optional_prop: Option<i32>,
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
            assert!(ctor.contains("required_param"), "ctor: {ctor}");
            assert!(!ctor.contains("required_prop"), "ctor: {ctor}");
            assert!(!ctor.contains("optional_prop"), "ctor: {ctor}");
            assert!(!ctor.contains("defaulted_prop"), "ctor: {ctor}");
        }
        // Every field has a getter; ordinary Props have public setters and the Param does not.
        for name in [
            "required_prop",
            "optional_prop",
            "defaulted_prop",
            "required_param",
        ] {
            assert!(
                generated.contains(&format!("fn {name}")),
                "missing getter for {name}: {generated}"
            );
        }
        assert!(generated.contains("fn set_required_prop"), "{generated}");
        assert!(generated.contains("fn set_optional_prop"), "{generated}");
        assert!(generated.contains("fn set_defaulted_prop"), "{generated}");
        assert!(!generated.contains("fn set_required_param"), "{generated}");
        // Both real and shadow are view-less, so both return bare `Self`.
        assert!(
            generated.matches("-> Self").count() >= 2,
            "both real and shadow constructors should return Self: {generated}"
        );
    }

    /// A view-less ordinary Prop has an implicit default, a getter, and a public setter, but is not
    /// part of the fixed constructor.  The assertion reads the same public shape consumed by real
    /// generation.
    #[test]
    fn t_r4_1_view_less_ordinary_prop_consumes_readable_and_writable_shape() {
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
            !shape.constructor_params.iter().any(|(n, _)| n == "value"),
            "{:?}",
            shape.constructor_params
        );
        assert!(
            shape.readable_fields.iter().any(|(n, _, _)| n == "value"),
            "{:?}",
            shape.readable_fields
        );
        assert!(
            shape
                .writable_fields
                .iter()
                .any(|(n, ty, _)| n == "value" && ty == "i32"),
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
        assert!(!real_ctor.contains("value"), "{real_ctor}");
        assert!(generated.contains("pub fn value"), "{generated}");
        assert!(generated.contains("pub fn set_value"), "{generated}");
    }

    /// An ordinary view-less `Option<T>` Prop is excluded from the fixed constructor while its
    /// getter and public setter both use the full declared `Option<T>` type.
    #[test]
    fn t_r4_2_view_less_option_prop_consumes_all_relevant_shape_surfaces() {
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
                .any(|(n, ty, _)| n == "value" && ty == "Option<String>"),
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
            setter.contains("value : Option < String >"),
            "setter parameter must preserve the full declared Option<T>: {setter}"
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

    /// An ordinary `Option<T>` Prop's setter parameter type comes from the writable shape entry and
    /// preserves the full declared type, not an inferred inner `T`.
    #[test]
    fn t_r5_1_option_prop_setter_parameter_comes_from_writable_shape_entry() {
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
                .any(|(n, ty, _)| n == "value" && ty == "Option<String>"),
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
            "& self , value : Option < String >",
            "setter parameter must come from writable_fields's full Option<String> entry: {setter}"
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

    /// Every legal own writable field category's real setter name/type/visibility matches
    /// `shape.writable_fields` exactly: ordinary Props (including `Option<T>`), defaulted Props,
    /// and defaulted State.  A required `#[param]` remains constructor-only.
    #[test]
    fn t_r5_4_all_own_writable_categories_match_shape_exactly() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct TR54AllWritable {
                optional_prop: Option<i32>,
                #[param]
                required_param: Option<String>,
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
                "optional_prop",
                "Option<i32>",
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
        let constructor = generated
            .split("pub fn new (")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("constructor signature should be present");
        assert!(constructor.contains("required_param"), "{constructor}");
        assert!(!constructor.contains("optional_prop"), "{constructor}");
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
            let setter_without_spaces = setter.replace(' ', "");
            let type_without_spaces = ty.replace(' ', "");
            assert!(
                setter_without_spaces.contains(&format!("value:{type_without_spaces}")),
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
            r#"struct UbBase { template: template_view!(|templated_parent: Self| { TextBlock { text: "x" } }), }"#,
        )
        .expect("base struct");
        build(r#"impl UbBase { }"#).expect("base impl");
        declare(
            Some("crate::UbBase"),
            r#"struct UbDerived { template: template_view!(|templated_parent: Self| { UbBase { } }), }"#,
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
            r#"struct UbBareBase { template: template_view!(|templated_parent: Self| { TextBlock { text: "x" } }), }"#,
        )
        .expect("base struct");
        build(r#"impl UbBareBase { }"#).expect("base impl");
        declare(
            Some("UbBareBase"),
            r#"struct UbBareDerived { template: template_view!(|templated_parent: Self| { UbBareBase { } }), }"#,
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
            r#"struct UbBuiltinDerived { template: template_view!(|templated_parent: Self| { TextBlock { text: "x" } }), }"#,
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
