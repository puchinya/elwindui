pub mod ast;
pub mod attr_frontend;
pub mod codegen;
pub mod component_frontend;
pub mod environment_frontend;
pub mod parser;
#[cfg(test)]
mod testdata;
mod text_style;
#[doc(hidden)]
pub use text_style::TEXT_STYLE_FIELDS;
pub mod theme_frontend;
pub mod validate;

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
pub fn generate_component_from_item_struct(
    base: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<proc_macro2::TokenStream, String> {
    generate_component_from_item_struct_with_template(base, None, item_struct)
}

/// Generates a component whose `body: view!` is replaceable by a typed Environment template.
pub fn generate_component_from_item_struct_with_template(
    base: Option<String>,
    template: Option<String>,
    item_struct: &syn::ItemStruct,
) -> Result<proc_macro2::TokenStream, String> {
    // Shape errors (a malformed `view!`, a bad field attribute, ...) are reported here, against the
    // struct that actually contains them, rather than being deferred to the `impl` half.
    let (component_def, view_def) =
        component_frontend::component_and_view_from_item_struct(base.clone(), item_struct)?;
    let name = component_def.name.clone();
    if let Some(template_name) = &template {
        let is_control = component_def
            .base
            .as_deref()
            .map(|base| {
                same_crate_control_target(base)
                    .or_else(|| component_def.base_path.is_none().then_some(false))
            })
            .unwrap_or(Some(false));
        if is_control == Some(false) {
            return Err(format!(
                "`{name}`: template-enabled components must inherit Control; NativeControl and non-Control components are not supported"
            ));
        }
        if view_def.is_none() {
            return Err(format!(
                "`{name}`: `template = {template_name}` requires a `body: view! {{ .. }}` default template"
            ));
        }
        validate_replaceable_template_view(view_def.as_ref().unwrap())?;
        match component_frontend::lookup_same_crate_environment_key(template_name) {
            None => {
                return Err(format!(
                    "`{name}`: template Environment Key `{template_name}` is not registered; declare it with #[elwindui::environment_key] before the component"
                ));
            }
            Some((_, value_type)) if !is_control_template_key_value(&value_type, &name) => {
                return Err(format!(
                    "`{name}`: template Environment Key `{template_name}` must have Value = Option<ControlTemplate<{name}>>, found `{value_type}`"
                ));
            }
            Some(_) => {}
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
    validate::validate(&all_modules).map_err(|errors| errors.join("\n"))?;
    component_frontend::register_same_crate_component_with_template(
        &name,
        base.as_deref(),
        template.as_deref(),
        item_struct,
    );
    // Emits nothing on purpose: the paired `#[elwindui::component] impl Name { .. }` generates the
    // whole type. This mirrors `#[elwindui_macros::class]` exactly — there too the `struct` half
    // only stashes what the `impl` half needs (`store_class_args`/`load_class_args`), and the
    // `impl` half is what emits the trait, the trait impl and `new()`. Components need the same
    // split because a `#[overridable]`/`#[overrides]` method body has nowhere to live on a bare
    // `struct`, and the generated type can only be emitted once — so it has to be emitted by
    // whichever half comes last, which is the `impl`.
    Ok(proc_macro2::TokenStream::new())
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
pub fn generate_component_from_item_impl(
    item_impl: &syn::ItemImpl,
) -> Result<proc_macro2::TokenStream, String> {
    let (name, methods) = component_frontend::methods_from_item_impl(item_impl)?;
    let Some((mut component_def, view_def)) = component_frontend::registered_component_parts(&name)
    else {
        return Err(format!(
            "{name}: no `#[elwindui::component] struct {name} {{ .. }}` was expanded before this \
             `impl` block — declare the struct first"
        ));
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
    validate::validate(&all_modules).map_err(|errors| errors.join("\n"))?;
    // Issue #162 §4.6: lower every `ViewExpr::DeferredView` reachable from `name`'s own `view`
    // into a synthetic hidden Component/View pair *after* validation (which still needs to see
    // the original, unlowered `DeferredView` nodes in `name`'s own enclosing lexical scope) and
    // *before* `build_symbol_table` (which needs to see the newly synthesized hidden components).
    lower_deferred_views_in_module(&mut module, &name);
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
            return Err(format!(
                "{name}: inherits `{base}`, but `{base}` is a user-defined component — write a \
                 full crate-root-qualified path instead of a bare name (e.g. `inherits \
                 crate::ui::{base}`). Also make sure the module exposing `{base}` re-exports it \
                 with a glob (`pub use some_module::*;`), not a named list — #[class] generates a \
                 companion `__elwindui_macros_of_{base}` alongside `{base}` itself that a named \
                 re-export would strand (docs/specs/dsl_spec.md §3)."
            ));
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
fn lower_deferred_views_in_module(module: &mut ast::Module, outer_component_name: &str) {
    let Some(view) = module.items.iter_mut().find_map(|item| match item {
        ast::Item::View(v) if v.target == outer_component_name => Some(v),
        _ => None,
    }) else {
        return;
    };
    let mut ordinal = 0usize;
    let mut new_items = Vec::new();
    lower_deferred_views_in_view(view, outer_component_name, &mut ordinal, &mut new_items);
    module.items.extend(new_items);
}

fn lower_deferred_views_in_view(
    view: &mut ast::ViewDef,
    owner_type_name: &str,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for l in &mut view.lets {
        lower_deferred_views_in_element(&mut l.element, owner_type_name, ordinal, new_items);
    }
    lower_deferred_views_in_body(&mut view.root, owner_type_name, ordinal, new_items);
}

fn lower_deferred_views_in_body(
    body: &mut ast::ViewBody,
    owner_type_name: &str,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for attribute in &mut body.attributes {
        lower_deferred_views_in_expr(&mut attribute.value, owner_type_name, ordinal, new_items);
    }
    for (_, _, expr) in &mut body.attached {
        lower_deferred_views_in_expr(expr, owner_type_name, ordinal, new_items);
    }
    for child in &mut body.children {
        lower_deferred_views_in_child(child, owner_type_name, ordinal, new_items);
    }
}

fn lower_deferred_views_in_element(
    elem: &mut ast::ElementNode,
    owner_type_name: &str,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for attribute in &mut elem.attributes {
        lower_deferred_views_in_expr(&mut attribute.value, owner_type_name, ordinal, new_items);
    }
    for (_, _, expr) in &mut elem.attached {
        lower_deferred_views_in_expr(expr, owner_type_name, ordinal, new_items);
    }
    for child in &mut elem.children {
        lower_deferred_views_in_child(child, owner_type_name, ordinal, new_items);
    }
}

fn lower_deferred_views_in_child(
    child: &mut ast::ChildEntry,
    owner_type_name: &str,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    match child {
        ast::ChildEntry::Literal(elem) => {
            lower_deferred_views_in_element(elem, owner_type_name, ordinal, new_items)
        }
        ast::ChildEntry::Ref(_) => {}
        ast::ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            lower_deferred_views_in_expr(condition, owner_type_name, ordinal, new_items);
            for c in then_branch.iter_mut().chain(else_branch.iter_mut()) {
                lower_deferred_views_in_child(c, owner_type_name, ordinal, new_items);
            }
        }
        ast::ChildEntry::Match { value, arms } => {
            lower_deferred_views_in_expr(value, owner_type_name, ordinal, new_items);
            for arm in arms {
                for c in &mut arm.body {
                    lower_deferred_views_in_child(c, owner_type_name, ordinal, new_items);
                }
            }
        }
        ast::ChildEntry::For {
            collection, body, ..
        } => {
            lower_deferred_views_in_expr(collection, owner_type_name, ordinal, new_items);
            for c in body {
                lower_deferred_views_in_child(c, owner_type_name, ordinal, new_items);
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
/// from within another `context_popup`'s own content, at arbitrary depth).
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
                ordinal,
                new_items,
            );
            let (hidden_component, hidden_view) =
                component_frontend::hidden_view_template_component(
                    &hidden_name,
                    owner_type_name,
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
        ast::ViewExpr::Element(elem) => {
            lower_deferred_views_in_element(elem, owner_type_name, ordinal, new_items)
        }
        ast::ViewExpr::Closure { body, .. } => match body {
            ast::ClosureBody::Element(elem) => {
                lower_deferred_views_in_element(elem, owner_type_name, ordinal, new_items)
            }
            ast::ClosureBody::Expr(inner) => {
                lower_deferred_views_in_expr(inner, owner_type_name, ordinal, new_items)
            }
            // A raw `syn::Block` (`on_*` handler body) has no reachable `ast::ViewExpr` of its own
            // to recurse into — `view!` only ever appears at a DSL attribute-value position, which
            // a `syn::Block` doesn't parse through this AST at all.
            ast::ClosureBody::Block(_) => {}
        },
        ast::ViewExpr::TFluent(_, args) => {
            for (_, v) in args {
                lower_deferred_views_in_expr(v, owner_type_name, ordinal, new_items);
            }
        }
        ast::ViewExpr::Path(_) | ast::ViewExpr::Expr(_) => {}
    }
}

fn lower_deferred_views_in_element_lets_and_body(
    lets: &mut [ast::LetBinding],
    root: &mut ast::ViewBody,
    owner_type_name: &str,
    ordinal: &mut usize,
    new_items: &mut Vec<ast::Item>,
) {
    for l in lets.iter_mut() {
        lower_deferred_views_in_element(&mut l.element, owner_type_name, ordinal, new_items);
    }
    lower_deferred_views_in_body(root, owner_type_name, ordinal, new_items);
}

/// Generates the private component instance and typed factory for
/// `#[elwindui::control_template(target = Target)] struct Name { body: view! { .. } }`.
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
        return Err("expected a struct with exactly `body: view! { .. }`".to_string());
    };
    let mut fields_iter = fields.named.iter();
    let Some(body) = fields_iter.next() else {
        return Err("expected `body: view! { .. }`".to_string());
    };
    if fields_iter.next().is_some()
        || body.ident.as_ref().is_none_or(|ident| ident != "body")
        || !matches!(
            &body.ty,
            syn::Type::Macro(mac)
                if mac.mac.path.segments.last().is_some_and(|segment| segment.ident == "view")
        )
    {
        return Err("expected exactly one field: `body: view! { .. }`".to_string());
    }

    let (_, authored_view) = component_frontend::component_and_view_from_item_struct(
        Some("Control".to_string()),
        item_struct,
    )?;
    validate_replaceable_template_view(
        authored_view
            .as_ref()
            .ok_or_else(|| "expected `body: view! { .. }`".to_string())?,
    )?;

    let name = &item_struct.ident;
    let hidden_name = quote::format_ident!("__ElwinduiControlTemplateInstanceFor{}", name);
    let body_ty = &body.ty;
    let hidden_struct: syn::ItemStruct = syn::parse_quote! {
        struct #hidden_name {
            #[param]
            templated_parent: std::rc::Weak<#target>,
            body: #body_ty,
        }
    };
    // `ContentControl` gives the private instance a single, ordinary content slot for the authored
    // root. The instance itself is the template root stored by the target; its content remains an
    // implementation detail and is unrelated to the target's logical content/presenter channel.
    generate_component_from_item_struct(Some("ContentControl".to_string()), &hidden_struct)?;
    let hidden_impl: syn::ItemImpl = syn::parse_quote! { impl #hidden_name {} };
    let hidden_generated = generate_component_from_item_impl(&hidden_impl)?;

    let attrs = &item_struct.attrs;
    let vis = &item_struct.vis;
    Ok(quote::quote! {
        #hidden_generated

        #(#attrs)*
        #vis struct #name;

        impl #name {
            pub fn template() -> elwindui::core::ui::ControlTemplate<#target> {
                elwindui::core::ui::ControlTemplate::new(|context| {
                    let instance = #hidden_name::__new_unmounted(std::rc::Rc::downgrade(&context.control));
                    instance.mount(context.environment);
                    instance.into_node()
                })
            }
        }
    })
}

fn same_crate_control_target(name: &str) -> Option<bool> {
    fn visit(name: &str, visited: &mut std::collections::HashSet<String>) -> Option<bool> {
        match name {
            "Control" | "ContentControl" => return Some(true),
            "UIElement" | "Layout" | "Shape" | "NativeControl" | "Window" => {
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

fn is_control_template_key_value(value_type: &str, target_name: &str) -> bool {
    fn last_path_ident(ty: &syn::Type) -> Option<&syn::PathSegment> {
        let syn::Type::Path(path) = ty else {
            return None;
        };
        path.path.segments.last()
    }

    let Ok(value_type) = syn::parse_str::<syn::Type>(value_type) else {
        return false;
    };
    let Some(option) = last_path_ident(&value_type) else {
        return false;
    };
    if option.ident != "Option" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(option_args) = &option.arguments else {
        return false;
    };
    let [syn::GenericArgument::Type(template_type)] =
        option_args.args.iter().collect::<Vec<_>>().as_slice()
    else {
        return false;
    };
    let Some(template) = last_path_ident(template_type) else {
        return false;
    };
    if template.ident != "ControlTemplate" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(template_args) = &template.arguments else {
        return false;
    };
    let [syn::GenericArgument::Type(target_type)] =
        template_args.args.iter().collect::<Vec<_>>().as_slice()
    else {
        return false;
    };
    last_path_ident(target_type).is_some_and(|target| target.ident == target_name)
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
    fn authoring_generates_a_typed_factory_and_weak_templated_parent() {
        let generated = author(
            r#"
            struct CodegenControlTemplateValidA {
                body: view! { TextBlock { text: "ok" } },
            }
            "#,
        )
        .expect("valid template should generate");
        assert!(generated.contains("ControlTemplate < Control >"));
        assert!(generated.contains("Weak < Control >"));
        assert!(generated.contains("templated_parent"));
    }

    #[test]
    fn authoring_rejects_ids_multiple_presenters_and_dynamic_presenters() {
        let id = author(
            r#"
            struct CodegenControlTemplateIdB {
                body: view! {
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
                body: view! {
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
                body: view! {
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
    fn template_enabled_default_body_rejects_id() {
        component_frontend::register_same_crate_environment_key(
            "codegen_control_template_key_c",
            "CodegenControlTemplateKeyC",
            "Option<ControlTemplate<CodegenControlTemplatePanelC>>",
        )
        .unwrap();
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct CodegenControlTemplatePanelC {
                body: view! {
                    #[id("part")]
                    let part = TextBlock { text: "x" };
                    part
                },
            }
            "#,
        )
        .unwrap();
        let error = generate_component_from_item_struct_with_template(
            Some("ContentControl".to_string()),
            Some("codegen_control_template_key_c".to_string()),
            &item,
        )
        .expect_err("default template ids must be rejected");
        assert!(error.contains("#[id"), "error: {error}");
    }

    #[test]
    fn template_environment_key_value_must_match_the_component() {
        component_frontend::register_same_crate_environment_key(
            "codegen_control_template_key_mismatch_e",
            "CodegenControlTemplateKeyMismatchE",
            "Option<ControlTemplate<AnotherPanel>>",
        )
        .unwrap();
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct CodegenControlTemplatePanelE {
                body: view! { TextBlock { text: "x" } },
            }
            "#,
        )
        .unwrap();
        let error = generate_component_from_item_struct_with_template(
            Some("ContentControl".to_string()),
            Some("codegen_control_template_key_mismatch_e".to_string()),
            &item,
        )
        .expect_err("mismatched template key target must be rejected");
        assert!(
            error.contains("Option<ControlTemplate<CodegenControlTemplatePanelE>>"),
            "error: {error}"
        );
    }

    #[test]
    fn same_crate_non_control_and_native_control_targets_are_rejected_early() {
        let target: syn::ItemStruct = syn::parse_str(
            r#"
            struct CodegenControlTemplateNotControlD {
                body: view! { VerticalLayout {} },
            }
            "#,
        )
        .unwrap();
        generate_component_from_item_struct(Some("VerticalLayout".to_string()), &target).unwrap();

        let template = r#"
            struct CodegenControlTemplateInvalidTargetD {
                body: view! { TextBlock { text: "x" } },
            }
        "#;
        let non_control = author_for("CodegenControlTemplateNotControlD", template)
            .expect_err("same-crate non-Control target must be rejected");
        assert!(non_control.contains("not a Control-derived"));

        let native = author_for("NativeControl", template)
            .expect_err("NativeControl target must be rejected");
        assert!(native.contains("NativeControl"));

        component_frontend::register_same_crate_environment_key(
            "codegen_control_template_key_d",
            "CodegenControlTemplateKeyD",
            "Option<ControlTemplate<CodegenControlTemplateNotControlD>>",
        )
        .unwrap();
        let component_error = generate_component_from_item_struct_with_template(
            Some("VerticalLayout".to_string()),
            Some("codegen_control_template_key_d".to_string()),
            &target,
        )
        .expect_err("template-enabled non-Control component must be rejected");
        assert!(component_error.contains("must inherit Control"));
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
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("non-exhaustive match should be rejected");
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
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("a typo'd vm field reference should be rejected");
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
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("#[bindable] on a non-viewmodel type should be rejected");
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
        let err = generate_component_from_item_struct(None, &item_struct)
            .expect_err("#[async_computed] on a component prop should be rejected (rule 20)");
        assert!(err.contains("#[async_computed]"), "error: {err}");
        assert!(err.contains("viewmodel/store"), "error: {err}");
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

    fn methods(src: &str) -> Result<String, String> {
        let item_impl: syn::ItemImpl = syn::parse_str(src).expect("impl should parse");
        generate_component_from_item_impl(&item_impl).map(|t| t.to_string())
    }

    /// The `struct` half emits nothing at all now — every token comes from the `impl` half.
    #[test]
    fn the_struct_half_emits_nothing() {
        let item_struct: syn::ItemStruct =
            syn::parse_str(r#"struct MiSilent {}"#).expect("struct should parse");
        let out = generate_component_from_item_struct(None, &item_struct)
            .expect("struct half should succeed");
        assert!(
            out.is_empty(),
            "struct half should emit nothing, got: {out}"
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
        .expect("impl half should generate");
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
        .expect("derived impl should generate");
        assert!(
            out.contains("fn __base_label"),
            "base body should be kept as a private shadow: {out}"
        );
        assert!(
            out.contains("self . __base_label ()") || out.contains("self.__base_label()"),
            "`base::label()` should be rewritten onto the shadow: {out}"
        );
    }

    #[test]
    fn overrides_without_a_matching_overridable_is_rejected() {
        declare(None, r#"struct MiNoHook {}"#);
        declare(
            Some("crate::MiNoHook"),
            r#"struct MiNoHookChild { body: view! { MiNoHook { } }, }"#,
        );
        let err = methods(
            r#"
            impl MiNoHookChild {
                #[overrides]
                fn missing(&self) -> String { String::new() }
            }
            "#,
        )
        .expect_err("overriding a method the base never declared should be rejected");
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
        let err = methods(
            r#"
            impl MiSigChild {
                #[overrides]
                fn label(&self, extra: i32) -> String { let _ = extra; String::new() }
            }
            "#,
        )
        .expect_err("a different signature should be rejected");
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
        let err = methods(
            r#"
            impl MiNeverDeclared {
                #[overridable]
                fn label(&self) -> String { String::new() }
            }
            "#,
        )
        .expect_err("an impl with no registered struct should be rejected");
        assert!(err.contains("declare the struct first"), "error: {err}");
    }

    #[test]
    fn a_trait_impl_is_rejected() {
        declare(None, r#"struct MiTraitImpl {}"#);
        let err = methods(r#"impl Clone for MiTraitImpl { fn clone(&self) -> Self { todo!() } }"#)
            .expect_err("a trait impl should be rejected");
        assert!(err.contains("trait impl"), "error: {err}");
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
        let err =
            generate_component_from_item_struct(Some("VerticalLayout".to_string()), &item_struct)
                .expect_err("an unresolvable #[environment(name)] should be rejected");
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

    fn build(src: &str) -> Result<String, String> {
        let item_impl: syn::ItemImpl = syn::parse_str(src).expect("impl should parse");
        generate_component_from_item_impl(&item_impl).map(|t| t.to_string())
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
            r#"struct UbBase { body: view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("base struct");
        build(r#"impl UbBase { }"#).expect("base impl");
        declare(
            Some("crate::UbBase"),
            r#"struct UbDerived { body: view! { UbBase { } }, }"#,
        )
        .expect("derived struct");
        let out = build(r#"impl UbDerived { }"#).expect("derived impl should generate");
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
            r#"struct UbBareBase { body: view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("base struct");
        build(r#"impl UbBareBase { }"#).expect("base impl");
        declare(
            Some("UbBareBase"),
            r#"struct UbBareDerived { body: view! { UbBareBase { } }, }"#,
        )
        .expect("derived struct");
        let err = build(r#"impl UbBareDerived { }"#)
            .expect_err("a bare name naming a user-defined base should be rejected");
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
            r#"struct UbBuiltinDerived { body: view! { TextBlock { text: "x" } }, }"#,
        )
        .expect("derived struct");
        let out = build(r#"impl UbBuiltinDerived { }"#).expect("derived impl should generate");
        assert!(
            out.contains("inherits = elwindui :: ui :: ContentControl"),
            "builtin base should stay fully-qualified via the existing `elwindui::ui::` rule: {out}"
        );
    }
}
