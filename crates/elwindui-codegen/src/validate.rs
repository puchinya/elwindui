//! A narrow slice of the ~24 static verification rules in docs/specs/dsl_spec.md §13 — only the
//! ones reachable by the constructs the notepad example actually uses. See
//! docs/design/gui_framework_design.md §10 for the full rule list.

use crate::ast::{
    AssignmentKind, Attr, ChildEntry, ClosureBody, ComponentDef, ElementNode, FieldDef, FieldKind,
    Item, Module, ViewExpr,
};
use crate::codegen::{self, SymbolTable, strip_rc_wrapper};
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;

pub fn validate(modules: &[Module]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let enum_variants: HashMap<String, HashSet<String>> = modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Enum(def) => Some((
                def.name.clone(),
                def.variants.iter().cloned().collect::<HashSet<_>>(),
            )),
            _ => None,
        })
        .collect();

    // The same real-path-aware resolver `codegen.rs` uses for code generation, reused here so
    // `vm.field` and other qualified paths are checked against exactly what's actually in
    // scope for the referencing module (locally defined, or brought in via `use` — §12) rather
    // than against every `component`/`viewmodel` in the whole compilation unit regardless of
    // whether it was ever imported.
    let table = codegen::build_symbol_table(modules);

    // Bare names of every `component`/`viewmodel` anywhere in the compilation unit (including ones
    // from `compile_dir_with_extra_viewmodels`'s Rust-side `extra_modules` — see
    // `attr_frontend::viewmodel_defs_from_rs_file`). Used only to tell whether a field's type is
    // *meant* to reference one of them (as opposed to a plain `String`/`i32`/etc. — see
    // `find_vm_fields`) before checking whether that reference actually resolves through `table`;
    // a name that looks like a reference but doesn't resolve is reported as an unresolved
    // reference (missing `use`), matching Rust's own "cannot find type" behavior.
    let known_type_names: HashSet<&str> = modules
        .iter()
        .flat_map(|m| &m.items)
        .filter_map(|item| match item {
            Item::Component(c) => Some(c.name.as_str()),
            Item::ViewModel(v) => Some(v.name.as_str()),
            Item::Enum(_) | Item::View(_) => None,
        })
        .collect();

    for module in modules {
        for item in &module.items {
            match item {
                Item::Component(c) => {
                    // `#[embedded]` (docs/specs/dsl_spec.md 付録A) claims this component is one of
                    // this crate's own builtin shape declarations — reject it on anything parsed
                    // from a consumer's own source directory (`Module::is_builtin`, set only by
                    // `builtin_modules()`).
                    if c.embedded && !module.is_builtin {
                        errors.push(format!(
                            "{}: #[embedded] can only be used on a component from elwindui-codegen's own \
                             BUILTIN_SHAPE_SOURCE, not a consumer's own source",
                            c.name
                        ));
                    }

                    // `#[text_style]` (docs/specs/dsl_spec.md 付録A, `ComponentDef::text_style`'s
                    // own doc comment) injects `TEXT_STYLE_FIELDS` (font_family/font_size/
                    // font_weight/font_style/font_stretch/character_spacing/foreground) — real
                    // Rust `TextStyleStorage` backing only exists on `elwindui-core`'s own hand-
                    // written classes (`Control`/`TextBlock`/each backend's `NativeControl`), so
                    // (mirroring `#[embedded]`/`#[native]` above) it only makes sense on this
                    // crate's own builtin shape declarations.
                    if c.text_style {
                        if !module.is_builtin {
                            errors.push(format!(
                                "{}: #[text_style] can only be used on a component from elwindui-codegen's \
                                 own BUILTIN_SHAPE_SOURCE, not a consumer's own source",
                                c.name
                            ));
                        }
                        // The injected fields are prepended ahead of the component's own
                        // hand-written ones (`parser.rs`'s `component` branch), so a same-named own
                        // field appears *twice* in `c.fields` instead of being silently shadowed —
                        // reported here rather than left for a confusing duplicate-setter error
                        // downstream.
                        let mut seen: HashSet<&str> = HashSet::new();
                        for f in &c.fields {
                            if crate::text_style::is_text_style_field_name(&f.name)
                                && !seen.insert(f.name.as_str())
                            {
                                errors.push(format!(
                                    "{}: #[text_style] already declares `{}` — remove this component's own \
                                     field with the same name",
                                    c.name, f.name
                                ));
                            }
                        }
                    }

                    // `#[native]` (docs/specs/dsl_spec.md 付録A, `ComponentDef::native`'s doc
                    // comment) marks a base-less, `view`-less leaf whose real implementation is
                    // hand-written per backend crate — `Window` is the motivating case (WinUI3's
                    // `Window` has no meaningful `Control`-family ancestor, unlike `Button`/
                    // `TextArea`/... which share `inherits NativeControl`). All three misuses below
                    // mirror the reasoning `#[embedded]` already applies, plus the two invariants
                    // `resolve_is_native`'s `#[native]` fallback assumes (no `base`, no own `view`).
                    if c.native {
                        if !module.is_builtin {
                            errors.push(format!(
                                "{}: #[native] can only be used on a component from elwindui-codegen's own \
                                 BUILTIN_SHAPE_SOURCE, not a consumer's own source",
                                c.name
                            ));
                        }
                        if c.base.is_some() {
                            errors.push(format!(
                                "{}: #[native] components must have no `inherits` base — it marks a leaf \
                                 with no meaningful inheritance ancestor at all (e.g. WinUI3's `Window : \
                                 Object`); use `inherits NativeControl` instead if `{}` does share a real \
                                 native-leaf family",
                                c.name, c.name
                            ));
                        }
                        let has_own_view = module
                            .items
                            .iter()
                            .any(|item| matches!(item, Item::View(v) if v.target == c.name));
                        if has_own_view {
                            errors.push(format!(
                                "{}: #[native] components must have no `view` of its own — each backend \
                                 crate hand-writes the real Rust implementation directly",
                                c.name
                            ));
                        }
                    }

                    // `#[content(field_name)]` (docs/specs/dsl_spec.md 付録A, WinUI3's
                    // `ContentPropertyAttribute` equivalent, `ComponentDef::content_field`'s doc
                    // comment) must actually name one of this component's own effective fields
                    // (`codegen::resolve_effective_fields` — includes inherited ones, matching how
                    // `build_component_args` looks the name up against `info.param_fields`, itself
                    // built from the same effective list) — a typo'd name would otherwise silently
                    // mean "no field ever claims a bare nested child", caught only at codegen time
                    // (or not at all, if the component happens to never receive one).
                    if let Some(name) = &c.content_field {
                        let effective_fields =
                            codegen::resolve_effective_fields(module, c, modules);
                        if !effective_fields.iter().any(|f| &f.name == name) {
                            errors.push(format!(
                                "{}: #[content({name})] names a field that doesn't exist on `{}`",
                                c.name, c.name
                            ));
                        }
                    }

                    for f in &c.fields {
                        if f.kind == FieldKind::State && f.initializer.is_none() {
                            errors.push(format!(
                                "{}.{}: #[state] field needs `default = expr`",
                                c.name, f.name
                            ));
                        }
                        if f.kind == FieldKind::State && !f.attrs.is_empty() {
                            errors.push(format!(
                                "{}.{}: #[state] cannot be combined with other field attributes",
                                c.name, f.name
                            ));
                        }
                        // `#[attached]` (§3) declares a property other elements set on
                        // *themselves* via `Owner::field: value` — it needs a default value for
                        // whichever of them never set it explicitly (see `check_attached_properties`).
                        if f.kind == FieldKind::Attached && f.initializer.is_none() {
                            errors.push(format!(
                                "{}.{}: #[attached] field needs a default value (e.g. `= 0`)",
                                c.name, f.name
                            ));
                        }
                        // `#[bindable]` (`ast::Attr::Bindable`'s own doc comment) wires an
                        // auto-refreshing `PropertyChanged` subscription via
                        // `elwindui::core::reactive::ObservableExt`, whose generated call
                        // dereferences the field through an `Rc` (every `viewmodel` is always
                        // `Rc`-allocated) — this only checks the field's *spelled* type looks
                        // `Rc`-wrapped; it can't check that the type actually implements
                        // `ObservableExt` (that's real `rustc` type-checking on the generated
                        // code, not something elwindui-codegen's own static analysis can see).
                        if f.attrs.iter().any(|a| matches!(a, Attr::Bindable))
                            && strip_rc_wrapper(&f.ty) == f.ty.trim()
                        {
                            errors.push(format!(
                                "{}.{}: #[bindable] field must be `Rc<..>`-wrapped, found `{}`",
                                c.name, f.name, f.ty
                            ));
                        }
                        // `#[bindable]` is meant exclusively for viewmodel injection (§7.2 — "the
                        // standard form for viewmodel injection, unify the whole project around
                        // this shape"). When the field's (Rc-stripped) type happens to be
                        // resolvable from this module — same-directory DSL modules and
                        // `compile_dir_with_extra_viewmodels`'s Rust-side viewmodels both commonly
                        // are — check it actually names a `viewmodel`, catching a `#[bindable]`
                        // mistakenly put on a plain `component` field early. When it's *not*
                        // resolvable (e.g. a `#[elwindui::component]`+`view!{..}` proc-macro
                        // referencing a viewmodel from a separate, unrelated macro expansion),
                        // this can't be checked at all — trust the marker, the same way
                        // `collection_uses_rc_identity` (`codegen.rs`) does for `for`-loop identity.
                        if f.attrs.iter().any(|a| matches!(a, Attr::Bindable)) {
                            let inner = strip_rc_wrapper(&f.ty);
                            if let Some(info) = table.resolve(module, inner) {
                                if !info.is_viewmodel {
                                    errors.push(format!(
                                        "{}.{}: #[bindable] field's type `{}` isn't a `viewmodel` — \
                                         #[bindable] is only for injecting a viewmodel",
                                        c.name, f.name, inner
                                    ));
                                }
                            }
                        }
                    }

                    if let Some(base) = &c.base {
                        validate_inherits(module, c, base, modules, &table, &mut errors);
                        validate_field_overrides(module, c, base, &table, &mut errors);
                    }

                    // `vm.field` / `vm.command.execute()` / `vm.command.can_execute` references
                    // inside this component's `view { ... }` tree, checked against whichever
                    // `#[param]` field's type names a component/viewmodel that's actually in scope
                    // (see `find_vm_fields`). Only applies if a matching `Item::View` exists in this
                    // same `modules` slice — nothing to walk otherwise.
                    if let Some(view) =
                        modules
                            .iter()
                            .flat_map(|m| &m.items)
                            .find_map(|item| match item {
                                Item::View(v) if v.target == c.name => Some(v),
                                _ => None,
                            })
                    {
                        let vm_fields = find_vm_fields(
                            module,
                            &c.name,
                            &c.fields,
                            &table,
                            &known_type_names,
                            &mut errors,
                        );
                        for let_binding in &view.lets {
                            check_vm_references(
                                &let_binding.element,
                                module,
                                &c.name,
                                &vm_fields,
                                &table,
                                None,
                                &mut errors,
                            );
                            check_dynamic_child_hosts(
                                &let_binding.element,
                                module,
                                &c.name,
                                &table,
                                &mut errors,
                            );
                            check_attached_properties(
                                &let_binding.element,
                                module,
                                &c.name,
                                &table,
                                &mut errors,
                            );
                            check_shortcut_attrs(
                                &let_binding.element,
                                module,
                                &c.name,
                                &table,
                                &mut errors,
                            );
                            check_binding_assignments(
                                &let_binding.element,
                                module,
                                c,
                                &table,
                                false,
                                &mut errors,
                            );
                        }
                        // Phase 0 (docs/design/gui_framework_design.md §5.1): `view.root` is now a bare
                        // `ast::ViewBody` — resolve it to the concrete `ElementNode` every other
                        // check below still expects, exactly the way `codegen::generate_view` does
                        // (a composable `base` implicitly wraps the whole body; otherwise the body
                        // must reduce to exactly one literal child).
                        let is_composed = table.resolve(module, &c.name).is_some_and(|info| {
                            info.composed_shape.is_some() || info.host_composition_base.is_some()
                        });
                        match codegen::resolve_view_root_element(
                            &view.root,
                            c.base.as_deref(),
                            is_composed,
                        ) {
                            Some(resolved_root) => {
                                check_vm_references(
                                    &resolved_root,
                                    module,
                                    &c.name,
                                    &vm_fields,
                                    &table,
                                    c.base.as_deref(),
                                    &mut errors,
                                );
                                check_dynamic_child_hosts(
                                    &resolved_root,
                                    module,
                                    &c.name,
                                    &table,
                                    &mut errors,
                                );
                                check_attached_properties(
                                    &resolved_root,
                                    module,
                                    &c.name,
                                    &table,
                                    &mut errors,
                                );
                                check_shortcut_attrs(
                                    &resolved_root,
                                    module,
                                    &c.name,
                                    &table,
                                    &mut errors,
                                );
                                check_binding_assignments(
                                    &resolved_root,
                                    module,
                                    c,
                                    &table,
                                    false,
                                    &mut errors,
                                );
                                check_match_exhaustiveness(
                                    &resolved_root,
                                    c,
                                    &vm_fields,
                                    module,
                                    &table,
                                    &enum_variants,
                                    &mut errors,
                                );
                            }
                            None => errors.push(format!(
                                "{}: view root must be exactly one element unless it inherits a \
                                 composable base",
                                c.name
                            )),
                        }
                    }
                }
                Item::ViewModel(viewmodel) => {
                    // Rule 19 (viewmodel must not reference view/builtin elements) holds by
                    // construction: `ViewModelDef` has no `view` body in this AST.
                    for field in &viewmodel.fields {
                        if field.kind == FieldKind::State {
                            errors.push(format!(
                                "{}.{}: #[state] is only allowed on a component",
                                viewmodel.name, field.name
                            ));
                        }
                    }
                }
                Item::Enum(_) | Item::View(_) => {}
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check_binding_assignments(
    node: &ElementNode,
    from: &Module,
    component: &ComponentDef,
    table: &SymbolTable,
    inside_for: bool,
    errors: &mut Vec<String>,
) {
    let target_info = table.resolve(from, &node.type_path);
    for attribute in &node.attributes {
        if attribute.kind != AssignmentKind::Once
            && let Some(macro_name) = unsupported_dependency_macro(&attribute.value)
        {
            errors.push(format!(
                "{}: {}:{}: cannot safely analyze dependencies inside `{macro_name}!`; wrap the whole RHS in once!(...) to evaluate it only during initialization",
                component.name, attribute.span.line, attribute.span.column
            ));
        }
        if attribute.kind == AssignmentKind::TwoWay {
            let location = format!("{}:{}", attribute.span.line, attribute.span.column);
            if inside_for {
                errors.push(format!(
                    "{}: {location}: two-way binding in a `for` item template is not implemented",
                    component.name
                ));
            }
            match target_info {
                Some(info) if info.two_way_fields.contains(&attribute.name) => {}
                Some(_) => errors.push(format!(
                    "{}: {location}: `{}.{}` does not support #[two_way]",
                    component.name, node.type_path, attribute.name
                )),
                None => {}
            }

            match &attribute.value {
                ViewExpr::Path(path) if path.len() == 1 => {
                    let source = &path[0];
                    match component.fields.iter().find(|field| &field.name == source) {
                        Some(field) if matches!(field.kind, FieldKind::Prop | FieldKind::State) => {}
                        Some(field) => errors.push(format!(
                            "{}: {location}: `{source}` is {:?}; a two-way source must be a mutable #[prop] or #[state] field",
                            component.name, field.kind
                        )),
                        None => errors.push(format!(
                            "{}: {location}: unknown two-way source `{source}`",
                            component.name
                        )),
                    }
                }
                ViewExpr::Path(path) if path.len() == 2 => {
                    let owner = &path[0];
                    let property = &path[1];
                    match component.fields.iter().find(|field| &field.name == owner) {
                        Some(field)
                            if field.attrs.iter().any(|attr| matches!(attr, Attr::Bindable)) =>
                        {
                            let owner_ty = strip_rc_wrapper(&field.ty);
                            match table.resolve(from, owner_ty) {
                                Some(info) if info.fields.contains_key(property) => {}
                                Some(_) => errors.push(format!(
                                    "{}: {location}: bindable owner `{owner}` has no property `{property}`",
                                    component.name
                                )),
                                None => errors.push(format!(
                                    "{}: {location}: cannot resolve bindable owner type `{owner_ty}`",
                                    component.name
                                )),
                            }
                        }
                        Some(_) => errors.push(format!(
                            "{}: {location}: `{owner}` is not a direct #[bindable] owner",
                            component.name
                        )),
                        None => errors.push(format!(
                            "{}: {location}: unknown bindable owner `{owner}`",
                            component.name
                        )),
                    }
                }
                ViewExpr::Path(path) => errors.push(format!(
                    "{}: {location}: unsupported two-way path `{}`; use a component field or direct bindable owner.field",
                    component.name,
                    path.join(".")
                )),
                _ => errors.push(format!(
                    "{}: {location}: two-way RHS must be a writable component field or direct bindable owner.field",
                    component.name
                )),
            }
        }
        check_binding_assignments_in_expr(
            &attribute.value,
            from,
            component,
            table,
            inside_for,
            errors,
        );
    }
    for child in &node.children {
        match child {
            ChildEntry::Literal(element) => {
                check_binding_assignments(element, from, component, table, inside_for, errors)
            }
            ChildEntry::Ref(_) => {}
            ChildEntry::If {
                then_branch,
                else_branch,
                ..
            } => {
                for child in then_branch.iter().chain(else_branch) {
                    check_binding_assignment_child(
                        child, from, component, table, inside_for, errors,
                    );
                }
            }
            ChildEntry::Match { arms, .. } => {
                for child in arms.iter().flat_map(|arm| &arm.body) {
                    check_binding_assignment_child(
                        child, from, component, table, inside_for, errors,
                    );
                }
            }
            ChildEntry::For { body, .. } => {
                for child in body {
                    check_binding_assignment_child(child, from, component, table, true, errors);
                }
            }
        }
    }
}

fn unsupported_dependency_macro(expr: &ViewExpr) -> Option<String> {
    match expr {
        ViewExpr::TFluent(_, args) => args
            .iter()
            .find_map(|(_, value)| unsupported_dependency_macro(value)),
        ViewExpr::Expr(expr) => {
            struct Collector {
                unsupported: Option<String>,
            }
            impl<'ast> Visit<'ast> for Collector {
                fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
                    let name = node
                        .mac
                        .path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string())
                        .unwrap_or_default();
                    if matches!(name.as_str(), "format" | "format_args" | "vec" | "theme") {
                        use syn::parse::Parser as _;
                        if let Ok(arguments) = syn::punctuated::Punctuated::<
                            syn::Expr,
                            syn::Token![,],
                        >::parse_terminated
                            .parse2(node.mac.tokens.clone())
                        {
                            for argument in &arguments {
                                self.visit_expr(argument);
                            }
                            return;
                        }
                    }
                    self.unsupported.get_or_insert(name);
                }
            }
            let mut collector = Collector { unsupported: None };
            collector.visit_expr(expr);
            collector.unsupported
        }
        ViewExpr::Element(element) => element
            .attributes
            .iter()
            .find_map(|attribute| unsupported_dependency_macro(&attribute.value)),
        ViewExpr::Closure {
            body: ClosureBody::Element(element),
            ..
        } => element
            .attributes
            .iter()
            .find_map(|attribute| unsupported_dependency_macro(&attribute.value)),
        ViewExpr::Path(_) | ViewExpr::Closure { .. } => None,
    }
}

fn check_binding_assignment_child(
    child: &ChildEntry,
    from: &Module,
    component: &ComponentDef,
    table: &SymbolTable,
    inside_for: bool,
    errors: &mut Vec<String>,
) {
    if let ChildEntry::Literal(element) = child {
        check_binding_assignments(element, from, component, table, inside_for, errors);
    } else {
        let wrapper = ElementNode {
            type_path: String::new(),
            attributes: Vec::new(),
            attached: Vec::new(),
            attribute_shortcuts: Vec::new(),
            children: vec![child.clone()],
        };
        check_binding_assignments(&wrapper, from, component, table, inside_for, errors);
    }
}

fn check_binding_assignments_in_expr(
    expr: &ViewExpr,
    from: &Module,
    component: &ComponentDef,
    table: &SymbolTable,
    inside_for: bool,
    errors: &mut Vec<String>,
) {
    match expr {
        ViewExpr::Element(element) => {
            check_binding_assignments(element, from, component, table, inside_for, errors)
        }
        ViewExpr::Closure {
            body: ClosureBody::Element(element),
            ..
        } => check_binding_assignments(element, from, component, table, inside_for, errors),
        _ => {}
    }
}

fn check_match_exhaustiveness(
    node: &ElementNode,
    component: &ComponentDef,
    vm_fields: &HashMap<&str, &str>,
    module: &Module,
    table: &SymbolTable,
    enum_variants: &HashMap<String, HashSet<String>>,
    errors: &mut Vec<String>,
) {
    for child in &node.children {
        check_match_in_child(
            child,
            component,
            vm_fields,
            module,
            table,
            enum_variants,
            errors,
        );
    }
}

fn check_match_in_child(
    child: &ChildEntry,
    component: &ComponentDef,
    vm_fields: &HashMap<&str, &str>,
    module: &Module,
    table: &SymbolTable,
    enum_variants: &HashMap<String, HashSet<String>>,
    errors: &mut Vec<String>,
) {
    match child {
        ChildEntry::Literal(node) => check_match_exhaustiveness(
            node,
            component,
            vm_fields,
            module,
            table,
            enum_variants,
            errors,
        ),
        ChildEntry::Ref(_) => {}
        ChildEntry::If {
            then_branch,
            else_branch,
            ..
        } => {
            for child in then_branch.iter().chain(else_branch) {
                check_match_in_child(
                    child,
                    component,
                    vm_fields,
                    module,
                    table,
                    enum_variants,
                    errors,
                );
            }
        }
        ChildEntry::For { body, .. } => {
            for child in body {
                check_match_in_child(
                    child,
                    component,
                    vm_fields,
                    module,
                    table,
                    enum_variants,
                    errors,
                );
            }
        }
        ChildEntry::Match { value, arms } => {
            let enum_name = match value {
                ViewExpr::Path(path) if path.len() == 1 => component
                    .fields
                    .iter()
                    .find(|field| field.name == path[0])
                    .map(|field| strip_rc_wrapper(&field.ty).to_string()),
                ViewExpr::Path(path) if path.len() == 2 => vm_fields
                    .get(path[0].as_str())
                    .and_then(|vm_ty| table.resolve(module, vm_ty))
                    .and_then(|info| info.field_types.get(&path[1]))
                    .map(|ty| strip_rc_wrapper(ty).to_string()),
                _ => None,
            };
            if let Some(enum_name) = enum_name.and_then(|name| {
                enum_variants
                    .get(name.rsplit("::").next().unwrap_or(&name))
                    .cloned()
            }) {
                let wildcard = arms.iter().any(|arm| arm.pattern.trim() == "_");
                // `arm.pattern`'s source text comes either straight from DSL text (no
                // extra whitespace around `::`) or, for a `view!`-macro-sourced `component`
                // (`component_frontend.rs`), from `proc_macro2::TokenStream::to_string()`, which
                // always re-serializes a qualified path with spaces around `::` (`"Orientation ::
                // Vertical"`) — `.map(str::trim)` on the split segment (not just the whole pattern
                // up front) keeps this variant-name extraction correct in both cases.
                let covered: HashSet<String> = arms
                    .iter()
                    .filter_map(|arm| arm.pattern.rsplit("::").next().map(str::trim))
                    .filter(|variant| *variant != "_")
                    .map(str::to_string)
                    .collect();
                if !wildcard {
                    let mut missing: Vec<_> = enum_name.difference(&covered).cloned().collect();
                    missing.sort();
                    if !missing.is_empty() {
                        errors.push(format!(
                            "{}: match is not exhaustive; missing {}",
                            component.name,
                            missing.join(", ")
                        ));
                    }
                }
            }
            for arm in arms {
                for child in &arm.body {
                    check_match_in_child(
                        child,
                        component,
                        vm_fields,
                        module,
                        table,
                        enum_variants,
                        errors,
                    );
                }
            }
        }
    }
}

/// A component's `#[param]` fields whose type names a `component`/`viewmodel` that's actually in
/// scope from `from` (there's no `#[param]`/injection marker left on `FieldDef` by the time it
/// reaches here, so "names a known type" is the signal used instead; a plain `String`/`i32`/etc.
/// field never matches since those never appear in `known_type_names`). A field whose type *looks*
/// like a component/viewmodel reference (i.e. is defined somewhere in the compilation unit) but
/// isn't resolvable from `from` — not defined locally and not brought in by any `use` — is reported
/// as an unresolved reference rather than silently skipped, matching Rust's own "cannot find type
/// in this scope" (missing `use`) behavior; §12.
fn find_vm_fields<'a>(
    from: &Module,
    owner_name: &str,
    fields: &'a [FieldDef],
    table: &SymbolTable,
    known_type_names: &HashSet<&str>,
    errors: &mut Vec<String>,
) -> HashMap<&'a str, &'a str> {
    let mut vm_fields = HashMap::new();
    for f in fields {
        let ty = strip_rc_wrapper(&f.ty);
        if !known_type_names.contains(ty) {
            continue;
        }
        if table.resolve(from, ty).is_some() {
            vm_fields.insert(f.name.as_str(), ty);
        } else {
            errors.push(format!(
                "{owner_name}.{}: type `{}` is not in scope here — add a `use` for it (or define it in this file)",
                f.name, f.ty
            ));
        }
    }
    vm_fields
}

/// Walks a `view { ... }` element tree checking every attribute expression's `vm.xxx` references
/// (see `check_vm_expr`) against `table`, resolved from `from`'s scope, recursing into children.
/// Also rejects `node` itself naming an `#[abstract]` component (docs/specs/dsl_spec.md 付録A) —
/// except when `node` is *this* call's own `exempt_root_type` (only ever set by the top-level
/// `view.root` call in `validate`'s main loop, to exactly the enclosing component's own `base`):
/// shape/host composition (`Rectangle inherits Shape`, `NotepadWindow inherits Window`) legitimately
/// constructs an otherwise-abstract base as its own view's literal root — `validate_inherits`
/// already enforces that the root must match `base` exactly, so this exemption only ever fires for
/// that one, already-validated case. Recursive children are never exempted (`None` is passed down),
/// so `Shape { .. }` written anywhere *else* in a view (a nested child, a let-binding, an attribute
/// value) is still rejected.
fn check_vm_references(
    node: &ElementNode,
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    exempt_root_type: Option<&str>,
    errors: &mut Vec<String>,
) {
    if exempt_root_type != Some(node.type_path.as_str()) {
        check_not_abstract(node, from, component_name, table, errors);
    }
    for attribute in &node.attributes {
        check_vm_expr(
            &attribute.value,
            from,
            component_name,
            vm_fields,
            table,
            errors,
        );
    }
    for child in &node.children {
        check_child_vm_references(child, from, component_name, vm_fields, table, errors);
    }
}

fn check_child_vm_references(
    child: &ChildEntry,
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    match child {
        ChildEntry::Literal(element) => check_vm_references(
            element,
            from,
            component_name,
            vm_fields,
            table,
            None,
            errors,
        ),
        ChildEntry::Ref(_) => {}
        ChildEntry::If {
            condition,
            then_branch,
            else_branch,
        } => {
            check_vm_expr(condition, from, component_name, vm_fields, table, errors);
            for child in then_branch.iter().chain(else_branch) {
                check_child_vm_references(child, from, component_name, vm_fields, table, errors);
            }
        }
        ChildEntry::Match { value, arms } => {
            check_vm_expr(value, from, component_name, vm_fields, table, errors);
            for arm in arms {
                for child in &arm.body {
                    check_child_vm_references(
                        child,
                        from,
                        component_name,
                        vm_fields,
                        table,
                        errors,
                    );
                }
            }
        }
        ChildEntry::For {
            collection, body, ..
        } => {
            check_vm_expr(collection, from, component_name, vm_fields, table, errors);
            for child in body {
                check_child_vm_references(child, from, component_name, vm_fields, table, errors);
            }
        }
    }
}

/// `#[abstract]` (docs/specs/dsl_spec.md 付録A): a pure category tag (`UIElement`/`NativeControl`/
/// `Layout`/`Shape`) cannot be instantiated directly — only named as an
/// `inherits` base, or (for a shape-composition base) as a component's own view root (see
/// `check_vm_references`'s `exempt_root_type`). An unresolvable `node.type_path` is left to
/// `check_element_value`'s own "unknown component" error, not reported again here.
fn check_not_abstract(
    node: &ElementNode,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    if table
        .resolve(from, &node.type_path)
        .is_some_and(|info| info.is_abstract)
    {
        errors.push(format!(
            "{component_name}: `{}` is #[abstract] and cannot be instantiated directly — use a concrete subtype instead",
            node.type_path
        ));
    }
}

fn check_vm_expr(
    expr: &ViewExpr,
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    match expr {
        ViewExpr::Path(path) => match path.as_slice() {
            [vm_name, field] => {
                if let Some(&ty) = vm_fields.get(vm_name.as_str()) {
                    let has_field = table
                        .resolve(from, ty)
                        .is_some_and(|info| info.fields.contains_key(field.as_str()));
                    if !has_field {
                        errors.push(format!(
                            "{component_name}: `{vm_name}.{field}` — `{ty}` has no field `{field}`"
                        ));
                    }
                }
            }
            _ => {}
        },
        ViewExpr::Expr(expr) => check_static_view_expr(expr, component_name, errors),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_vm_expr(arg, from, component_name, vm_fields, table, errors);
            }
        }
        ViewExpr::Closure { params, body } => match body {
            ClosureBody::Expr(inner) => check_closure_expr_body(
                inner,
                params,
                from,
                component_name,
                vm_fields,
                table,
                errors,
            ),
            ClosureBody::Element(elem) => check_element_value(
                elem,
                params.first().map(String::as_str),
                from,
                component_name,
                vm_fields,
                table,
                errors,
            ),
            // A block-bodied closure (`on_*` event handlers) is ordinary Rust, not the DSL's own
            // path grammar — left unvalidated here, same as `ViewExpr::Expr`'s raw `syn::Expr`
            // above (`codegen::rewrite_view_closure_block` still resolves `vm.foo` references
            // correctly at codegen time; only this static pre-check is skipped).
            ClosureBody::Block(_) => {}
        },
        ViewExpr::Element(elem) => {
            check_element_value(elem, None, from, component_name, vm_fields, table, errors)
        }
    }
}

/// View attributes must remain statically inspectable.  This deliberately accepts the concrete
/// value forms the DSL already parses (literals, arrays, enum paths and enum constructors), while
/// refusing arbitrary Rust code whose dependencies cannot be subscribed to safely.
fn check_static_view_expr(expr: &syn::Expr, component_name: &str, errors: &mut Vec<String>) {
    fn allowed(expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Lit(_) | syn::Expr::Path(_) => true,
            syn::Expr::Array(array) => array.elems.iter().all(allowed),
            syn::Expr::Tuple(tuple) => tuple.elems.iter().all(allowed),
            syn::Expr::Paren(paren) => allowed(&paren.expr),
            syn::Expr::Group(group) => allowed(&group.expr),
            syn::Expr::Unary(unary) => allowed(&unary.expr),
            syn::Expr::Binary(binary) => allowed(&binary.left) && allowed(&binary.right),
            syn::Expr::Cast(cast) => allowed(&cast.expr),
            syn::Expr::Macro(expression)
                if expression
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "theme") =>
            {
                syn::parse2::<syn::Path>(expression.mac.tokens.clone()).is_ok()
            }
            syn::Expr::Macro(expression)
                if expression.mac.path.segments.last().is_some_and(|segment| {
                    matches!(
                        segment.ident.to_string().as_str(),
                        "format" | "format_args" | "vec"
                    )
                }) =>
            {
                use syn::parse::Parser as _;
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(expression.mac.tokens.clone())
                    .is_ok_and(|arguments| arguments.iter().all(allowed))
            }
            // Optional shape fields in the builtin declarations use this pure normalization
            // before constructing their enum value.
            syn::Expr::MethodCall(call) if call.method == "unwrap_or" => {
                allowed(&call.receiver) && call.args.iter().all(allowed)
            }
            syn::Expr::Struct(value) => {
                value.fields.iter().all(|field| allowed(&field.expr))
                    && value.rest.as_deref().is_none_or(allowed)
            }
            // Tuple enum variants, e.g. `GridLength::Star(1.0)`, are values rather than opaque
            // function calls. Requiring an UpperCamelCase final segment avoids accepting `foo()`.
            syn::Expr::Call(call) => match call.func.as_ref() {
                syn::Expr::Path(path) => {
                    path.path.segments.last().is_some_and(|segment| {
                        segment
                            .ident
                            .to_string()
                            .chars()
                            .next()
                            .is_some_and(char::is_uppercase)
                    }) && call.args.iter().all(allowed)
                }
                _ => false,
            },
            _ => false,
        }
    }

    if !allowed(expr) {
        errors.push(format!(
            "{component_name}: view expression `{}` is not statically analyzable — use #[computed], an explicit prop, or split it into literal/enum/path expressions",
            quote::ToTokens::to_token_stream(expr)
        ));
    }
}

/// Checks a closure body (`for item in …`'s item-local expression): a reference is
/// valid if its first segment is the closure's own bound parameter (the parameter isn't a `vm_fields`-tracked
/// component/viewmodel) or a recognized `vm`-style field (checked the normal way via
/// `check_vm_expr`). Anything else is an error — see `emit_expr` in `codegen.rs` for why an
/// outer-component reference from inside a closure body would otherwise silently resolve to a
/// bogus bare identifier instead of failing to compile.
fn check_closure_expr_body(
    expr: &ViewExpr,
    params: &[String],
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    let first_segment = match expr {
        ViewExpr::Path(path) => path.first(),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_closure_expr_body(
                    arg,
                    params,
                    from,
                    component_name,
                    vm_fields,
                    table,
                    errors,
                );
            }
            return;
        }
        // A raw `syn::Expr` (e.g. `std::rc::Rc::as_ptr(doc) as usize`, or an `on_*` handler's
        // `vm.close_tab(index)`) isn't inspected further, matching how ordinary (non-closure)
        // `Expr` values are already left unvalidated above.
        ViewExpr::Expr(_) => return,
        // The parser never produces a closure directly nested inside another closure's expression
        // body, nor a bare element there (an element-valued closure body is always
        // `ClosureBody::Element`, handled separately by `check_vm_expr`'s own `Closure` arm).
        ViewExpr::Closure { .. } | ViewExpr::Element(_) => return,
    };
    match first_segment {
        Some(first) if params.iter().any(|p| p == first) => {}
        Some(first) if vm_fields.contains_key(first.as_str()) => {
            check_vm_expr(expr, from, component_name, vm_fields, table, errors);
        }
        Some(first) => errors.push(format!(
            "{component_name}: closure body references `{first}`, which is neither one of the closure's own parameters (`{}`) nor a recognized field",
            params.join(", ")
        )),
        None => {}
    }
}

/// Dynamic child ranges are meaningful only for an ordered content collection. A scalar
/// `#[content(content)]` / `#[content(submenu)]` cannot host `if`/`match`/`for`, because there is
/// no insertion position or retained child range to reconcile.
fn check_dynamic_child_hosts(
    node: &ElementNode,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    if node.children.iter().any(|child| {
        matches!(
            child,
            ChildEntry::If { .. } | ChildEntry::Match { .. } | ChildEntry::For { .. }
        )
    }) {
        if let Some(info) = table.resolve(from, &node.type_path) {
            let field = info.content_field.as_deref().unwrap_or("children");
            let is_collection = info.field_types.get(field).is_some_and(|ty| {
                ty.contains("UIElementCollection")
                    || ty.trim_start().starts_with("Vec<")
                    || ty.contains("ListExt<")
            });
            // Phase 2 (docs/design/gui_framework_design.md §5.1): a *scalar* `#[content(...)]` field (e.g.
            // `ContentControl`/`Window`'s `content: Rc<dyn UIElement>`) can also host `if`/`match`
            // dynamic children now — not `for` (a variable-length list can never fit one slot), and
            // only if every branch, recursively, resolves to exactly one element (no `for`
            // anywhere inside it either, and no branch with zero or multiple children) — the
            // dynamic analogue of the existing "single content field can only bind one bare child"
            // rule (`codegen.rs`'s `panics_on_multiple_bare_children_for_a_single_content_field`).
            if !is_collection && !dynamic_children_reduce_to_one_element(&node.children) {
                errors.push(format!(
                    "{component_name}: dynamic child control flow under `{}` — `#[content({field})]` has scalar type `{}`, so every branch must resolve to exactly one element (`for` and multiple children per branch aren't allowed here; a collection-typed content field allows both, see `#[content({field})]`'s own type)",
                    node.type_path,
                    info.field_types.get(field).map(String::as_str).unwrap_or("<missing>")
                ));
            }
        }
    }
    for child in &node.children {
        check_dynamic_child_host_in_child(child, from, component_name, table, errors);
    }
}

/// Recurses into a single child entry for `check_dynamic_child_hosts` — shared by `If`'s
/// then/else branches, `Match`'s arms, and `For`'s body, so a nested `if`/`match`/`for` (Phase 1)
/// is walked into just like a top-level one instead of being silently skipped. The "does *this*
/// element's own `#[content(...)]` support dynamic children" check itself only ever needs to fire
/// once per real element (`check_dynamic_child_hosts`'s own body, above) — a region nested inside
/// another dynamic region still targets that same outer element, not a new one, so this function
/// only needs to keep walking down to find further literal elements with dynamic children of their
/// own to check.
fn check_dynamic_child_host_in_child(
    child: &ChildEntry,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    match child {
        ChildEntry::Literal(element) => {
            check_dynamic_child_hosts(element, from, component_name, table, errors)
        }
        ChildEntry::Ref(_) => {}
        ChildEntry::If {
            then_branch,
            else_branch,
            ..
        } => {
            for child in then_branch.iter().chain(else_branch) {
                check_dynamic_child_host_in_child(child, from, component_name, table, errors);
            }
        }
        ChildEntry::Match { arms, .. } => {
            for arm in arms {
                for child in &arm.body {
                    check_dynamic_child_host_in_child(child, from, component_name, table, errors);
                }
            }
        }
        ChildEntry::For { body, .. } => {
            for child in body {
                check_dynamic_child_host_in_child(child, from, component_name, table, errors);
            }
        }
    }
}

/// Phase 2: whether `children` (a dynamic branch's body — `If`'s then/else, one `Match` arm's
/// body) resolves to *exactly one element* — the shape a scalar `#[content(...)]` field's dynamic
/// region must have in every branch, recursively. `For` never qualifies (a variable-length list
/// can never be exactly one element); a nested `If`/`Match` qualifies only if *all* of its own
/// branches do too.
fn dynamic_children_reduce_to_one_element(children: &[ChildEntry]) -> bool {
    let [only] = children else {
        return false;
    };
    match only {
        ChildEntry::Literal(_) | ChildEntry::Ref(_) => true,
        ChildEntry::If {
            then_branch,
            else_branch,
            ..
        } => {
            dynamic_children_reduce_to_one_element(then_branch)
                && dynamic_children_reduce_to_one_element(else_branch)
        }
        ChildEntry::Match { arms, .. } => arms
            .iter()
            .all(|arm| dynamic_children_reduce_to_one_element(&arm.body)),
        ChildEntry::For { .. } => false,
    }
}

/// Checks every `Owner::field: value` attached-property setter (§3) on `node` and its descendants:
/// `Owner` must resolve to a known component/builtin, and that component must declare `field` as
/// an `#[attached]`-kind field. Deliberately does *not* check whether `node` is actually a
/// descendant of an `Owner` element anywhere in the tree — like WPF's own attached properties, one
/// set on an element that never ends up under a matching container is simply inert at runtime, not
/// a static error (see `ElementNode::attached`'s doc comment).
fn check_attached_properties(
    node: &ElementNode,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    for (owner, field, _value) in &node.attached {
        match table.resolve(from, owner) {
            Some(info) if info.fields.get(field.as_str()) == Some(&FieldKind::Attached) => {}
            Some(_) => errors.push(format!(
                "{component_name}: `{owner}::{field}` — `{owner}` has no #[attached] property named `{field}`"
            )),
            // External (no local `TypeInfo`) — same tradeoff as `check_element_value`'s own `None`
            // arm: only legitimate on the proc-macro path (`from.allows_external_builtins`), where
            // this can't be checked without a shape table at all; a genuinely wrong `Owner::field`
            // still fails to compile, just later, via `@attached_set`'s own generated dispatch.
            None if from.allows_external_builtins => {}
            None => errors.push(format!(
                "{component_name}: `{owner}::{field}` — `{owner}` is not a known component/builtin (missing `use`?)"
            )),
        }
    }
    for child in &node.children {
        if let ChildEntry::Literal(elem) = child {
            check_attached_properties(elem, from, component_name, table, errors);
        }
    }
    for attribute in &node.attributes {
        check_attached_properties_in_expr(&attribute.value, from, component_name, table, errors);
    }
}

/// `#[shortcut(...)]` (docs/design/gui_framework_design.md §8.1) only means anything on an
/// attribute that's actually `#[routed]` on this element's resolved type (same reasoning as
/// `on_click`/`on_key_down` themselves being callback-shaped, not arbitrary data) — checked here,
/// against the concrete usage site, rather than in `parse_field_def`'s per-declaration checks: a
/// shortcut is inherently a per-instance annotation (see `ast::ElementNode::attribute_shortcuts`'s
/// own doc comment for why it can't live on the field's shared `#[class]` declaration).
/// Also checks every declared key spec parses (`codegen::parse_shortcut_spec` — the same parser
/// `codegen::emit_shortcut_chord_expr` uses, so a spec that passes here is guaranteed not to panic
/// during code generation).
fn check_shortcut_attrs(
    node: &ElementNode,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    for (name, chords, _scope) in &node.attribute_shortcuts {
        match table.resolve(from, &node.type_path) {
            Some(info) if info.routed_fields.contains(name) => {}
            Some(_) => errors.push(format!(
                "{component_name}: #[shortcut(...)] on `{}.{name}` — `{name}` is not a #[routed] attribute",
                node.type_path
            )),
            // External (no local `TypeInfo`, e.g. a builtin declared entirely in `elwindui-core`):
            // this validation genuinely cannot check "is `{name}` really `#[routed]`" without a
            // shape table — `emit_wiring`'s own external-path codegen (`build_props_macro`'s doc
            // comment on why `@set` accepts a bare callable either way) already defers exactly this
            // question to the generated `@set`/`@routed` dispatch, so there is nothing left for this
            // *earlier* validation pass to usefully reject here. A genuinely wrong assumption still
            // fails to compile, just later and with a less specific message — the same tradeoff
            // every other builtin-shape check already makes once a builtin's shape lives only in its
            // own `#[prop]` declarations.
            None => {}
        }
        for (backend, spec) in chords {
            if let Err(e) = codegen::parse_shortcut_spec(spec) {
                let backend_note = backend
                    .as_deref()
                    .map(|b| format!(" ({b})"))
                    .unwrap_or_default();
                errors.push(format!(
                    "{component_name}: #[shortcut] key spec `{spec}`{backend_note} on `{}.{name}`: {e}",
                    node.type_path
                ));
            }
        }
    }
    for child in &node.children {
        if let ChildEntry::Literal(elem) = child {
            check_shortcut_attrs(elem, from, component_name, table, errors);
        }
    }
    for attribute in &node.attributes {
        check_shortcut_attrs_in_expr(&attribute.value, from, component_name, table, errors);
    }
}

fn check_shortcut_attrs_in_expr(
    expr: &ViewExpr,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    match expr {
        ViewExpr::Element(elem) => check_shortcut_attrs(elem, from, component_name, table, errors),
        ViewExpr::Closure {
            body: ClosureBody::Element(elem),
            ..
        } => check_shortcut_attrs(elem, from, component_name, table, errors),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_shortcut_attrs_in_expr(arg, from, component_name, table, errors);
            }
        }
        ViewExpr::Path(_)
        | ViewExpr::Expr(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_) | ClosureBody::Block(_),
            ..
        } => {}
    }
}

fn check_attached_properties_in_expr(
    expr: &ViewExpr,
    from: &Module,
    component_name: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    match expr {
        ViewExpr::Element(elem) => {
            check_attached_properties(elem, from, component_name, table, errors)
        }
        ViewExpr::Closure {
            body: ClosureBody::Element(elem),
            ..
        } => check_attached_properties(elem, from, component_name, table, errors),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_attached_properties_in_expr(arg, from, component_name, table, errors);
            }
        }
        ViewExpr::Path(_)
        | ViewExpr::Expr(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_) | ClosureBody::Block(_),
            ..
        } => {}
    }
}

/// Checks a `Type { attr: value, .. }` element used as a value — either a closure body
/// (`render_content: |param| Type { .. }`, `param` is `Some`) or an ordinary named-slot attribute
/// value (`menu_bar: MenuBar { .. }`, `param` is `None`). `Type` must resolve to an in-scope
/// component, and every one of its required `#[param]`-shaped fields must be satisfiable: by a
/// matching attribute, by being `Option<..>`-typed (defaults to `None`), by a `children`-named
/// field (filled from `elem`'s bare nested children, whatever their count), or — mirroring
/// `emit_construction`'s own positional fallback (e.g. `MenuBarItem`'s single nested `Menu`) — by
/// an available bare child. Anything left over is reported here instead of `panic!`ing deep in
/// codegen.
fn check_element_value(
    elem: &ElementNode,
    param: Option<&str>,
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    match table.resolve(from, &elem.type_path) {
        Some(info) => {
            if info.is_abstract {
                errors.push(format!(
                    "{component_name}: `{}` is #[abstract] and cannot be instantiated directly — use a concrete subtype instead",
                    elem.type_path
                ));
                return;
            }
            let mut next_positional_child = 0usize;
            for (name, ty) in &info.param_fields {
                if name == "children" {
                    continue;
                }
                let (_, is_option) = codegen::strip_option(ty);
                let has_attr = elem
                    .attributes
                    .iter()
                    .any(|attribute| &attribute.name == name);
                if has_attr || is_option {
                    continue;
                }
                if next_positional_child < elem.children.len() {
                    next_positional_child += 1;
                    continue;
                }
                errors.push(format!(
                    "{component_name}: `{}` is missing required attribute `{name}`",
                    elem.type_path
                ));
            }
        }
        // External (no local `TypeInfo`, e.g. a builtin declared entirely in `elwindui-core`): only
        // legitimate on `from.allows_external_builtins` (a proc-macro-built module — see that
        // field's own doc comment). This validation genuinely cannot check is_abstract-ness or
        // required-attribute completeness without a shape table, the same tradeoff
        // `emit_external_construction`/`check_shortcut_attrs` already make — a genuinely wrong
        // reference still fails to compile, just later, via `elwindui::ui::{Name}::new()` itself.
        // On the DSL text path (`allows_external_builtins == false`), no such escape hatch
        // exists — every type there must resolve through `table`, so `None` still means a typo.
        None if from.allows_external_builtins => {}
        None => errors.push(format!(
            "{component_name}: `{}` is an unknown or out-of-scope component — add a `use` for it",
            elem.type_path
        )),
    }
    for attribute in &elem.attributes {
        let value = &attribute.value;
        match param {
            Some(param) => check_closure_expr_body(
                value,
                std::slice::from_ref(&param.to_string()),
                from,
                component_name,
                vm_fields,
                table,
                errors,
            ),
            None => check_vm_expr(value, from, component_name, vm_fields, table, errors),
        }
    }
    for child in &elem.children {
        if let ChildEntry::Literal(literal) = child {
            check_vm_references(
                literal,
                from,
                component_name,
                vm_fields,
                table,
                None,
                errors,
            );
        }
    }
}

/// Checks `component X inherits Base { .. }` (docs/specs/dsl_spec.md §3): `Base` must resolve, then
/// branches on what kind of base it is:
/// - `X` itself is a hand-written virtual builtin (`codegen::is_virtual_builtin` —
///   `VerticalLayout`/`HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape`): unconditionally
///   allowed regardless of `Base`'s own shape. A virtual builtin is constructed entirely by
///   `codegen::build_virtual_value`'s per-type-name `match`, never through a `view` — it's
///   structurally incapable of having one, so none of the `view`-based checks below apply (this is
///   what lets `Layout` carry a real `children: UIElementCollection` field — see that component's
///   own `#[class]` doc comment — without breaking `VerticalLayout`/`HorizontalLayout`/
///   `Grid`'s own `inherits Layout`).
/// - A pure, field-less category tag (`base_info.effective_fields.is_empty() && !has_view` — e.g.
///   `UIElement`/`NativeControl`/`TextBlock` themselves): nothing to delegate to structurally, so
///   unconditionally allowed. `NativeControl` alone additionally requires `X`'s
///   structurally-inferred `is_native` (see `codegen::build_symbol_table`'s `resolve_is_native`) —
///   `inherits NativeControl` doesn't itself *determine* nativeness, every other category tag
///   imposes no further requirement.
/// - A native-backed leaf that *does* carry real fields (`has_view == false && is_native == true`,
///   e.g. `Button`/`Window`) — falls through to the same "`X`'s own `view` root must literally
///   construct `Base`" check as the shape-composition case below (this is how a hand-written
///   native host like `Window` gets inherited — `codegen`'s `host_composition_base` resolution;
///   docs/design/gui_framework_design.md §5.1).
/// - A primitive shape family with no `view` of its own (`has_view == false`, has real fields,
///   e.g. `Control`/`Rectangle`) — unchanged from before real field inheritance: `X` must have its
///   own `view` whose root element is literally `Base` (the shape-composition use case,
///   `codegen::resolve_view_for` doesn't attempt to auto-synthesize this one). Fields are now
///   inherited automatically either way (`X` no longer needs to redeclare `Base`'s fields to
///   forward them).
/// - A logical component with its own `view` (`has_view == true`, builtin or user-defined) — `X`'s
///   `view` is now optional (omitted: inherits `Base`'s template wholesale, WinUI3-style — see
///   `codegen::resolve_view_for`); if present, no constraint on its root element (a full template
///   override, unlike the primitive-shape case above).
fn validate_inherits(
    from: &Module,
    c: &ComponentDef,
    base: &str,
    modules: &[Module],
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    let Some(base_info) = table.resolve(from, base) else {
        // External (no local `TypeInfo`, e.g. a builtin declared entirely in `elwindui-core`): the
        // deeper checks below (sealed, category-tag exemptions, "`X`'s own `view` root must
        // construct `base`") all need `base_info`, which isn't available. For the proc-macro path
        // this loses nothing real — `component_frontend.rs` already requires every
        // `#[elwindui::component]` struct to carry exactly one `body: view! { .. }` field, so the
        // "has its own view" check this function's deepest fallback performs is structurally
        // guaranteed to pass regardless of `base` — and a genuinely `#[sealed]` base still fails to
        // compile on its own, via `#[class]`'s own inherit-macro mechanism (a sealed class emits no
        // `__elwindui_inherit_*!` trio, so naming it as `inherits = ..` fails with "macro not found"
        // rather than silently succeeding). The DSL module path (where a component may
        // legitimately have no `view` of its own, relying on an inherited template) loses real
        // checking here — accepted since that whole path is being phased out (`examples/notepad` is
        // its only remaining user, migrating to `#[component]` — see
        // `docs/status/implementation_status.md`), not extended with new capability.
        return;
    };

    if base_info.sealed {
        errors.push(format!(
            "{}: inherits `{base}`, but `{base}` is #[sealed] and cannot be inherited from",
            c.name
        ));
        return;
    }

    // A hand-written virtual builtin (`TypeInfo::is_virtual_builtin` — `VerticalLayout`/
    // `HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape`) is constructed entirely by
    // `codegen::build_virtual_value`'s own field-driven construction, never through a `view` — it's
    // structurally incapable of having one (`#[embedded]` with no `Item::View`). The "`X`'s own
    // `view` root must literally construct `Base`" shape-composition contract below therefore
    // doesn't apply to it, regardless of whether `Base` (e.g. `Layout`, once it carries a real
    // `children: UIElementCollection` field) happens to have fields of its own.
    if table
        .resolve(from, &c.name)
        .is_some_and(|i| i.is_virtual_builtin)
    {
        return;
    }

    // The three root category tags of the whole class hierarchy
    // (docs/design/gui_framework_design.md §5.1) — `UIElement` (the root), and its two immediate
    // abstract branches `Layout`/`NativeControl` — are never themselves a `view`'s root anywhere
    // (structurally: nothing
    // meaningfully "is" a bare `UIElement`/`Layout`/`NativeControl`, as opposed to some concrete
    // leaf/container beneath them), so inheriting one directly requires no evidence of a `view`
    // constructing it. This is a closed, stable set by construction — there is exactly one
    // `UIElement` root and exactly two immediate category branches beneath it — unlike
    // `is_virtual_builtin`'s own set (which grows with every new concrete virtual builtin), so
    // naming them explicitly here doesn't reintroduce the kind of per-widget hardcoding this
    // module was refactored to avoid. Not just `#[abstract]` (also true of `Shape`, which *is*
    // legitimately used as a view root by `Rectangle`/`Ellipse` and so must NOT be exempted here —
    // nor "has no fields of its own", the old proxy this used before `UIElement` grew real common
    // properties like `margin`/`width`/`height`, which broke exactly this way). `NativeControl`
    // alone carries one extra obligation: the inheritor must actually resolve as `is_native` (a
    // real backend handle exists) — every other category tag imposes no further requirement on
    // its own.
    if matches!(base, "UIElement" | "Layout" | "NativeControl") && !base_info.has_view {
        if base == "NativeControl" {
            let is_native = table
                .resolve(from, &c.name)
                .is_some_and(|info| info.is_native);
            if !is_native {
                errors.push(format!(
                    "{}: inherits `NativeControl`, but its `view` root isn't itself native (or no \
                     `view` exists) — `NativeControl` is only a category tag for genuinely \
                     native-backed components",
                    c.name
                ));
            }
        }
        return;
    }

    if base_info.has_view {
        // A logical component base: `X`'s own `view`, if any, is a full template override — no
        // root-element constraint (unlike the primitive-shape case below).
        return;
    }

    // A primitive shape family (`has_view == false`, not native): `X` must have its own `view` —
    // Phase 0's implicit-composition sugar (docs/design/gui_framework_design.md §5.1) means that `view`'s
    // body is always implicitly `base`'s own attributes/children directly (no wrapper element to
    // check the shape of anymore); a virtual-builtin base has no `view` of its own to fall back to
    // as a template, so `X` must still declare one.
    let has_own_view = modules
        .iter()
        .flat_map(|m| &m.items)
        .any(|item| matches!(item, Item::View(v) if v.target == c.name));
    if !has_own_view {
        errors.push(format!(
            "{}: inherits `{base}`, but has no `view {}` — a component inheriting a shape \
             primitive with no `view` of its own must declare one composing over `{base}`",
            c.name, c.name
        ));
    }
}

/// Checks field-level `inherits` overrides (§3): a field this component redeclares that's already
/// present on `base` (its effective, recursively-flattened field list) must either match kind
/// exactly and be `#[computed]` with `#[override]` (an intentional override — codegen's
/// `resolve_effective_fields`/`resolve_effective_methods` shadow-copies `base`'s original body
/// under `__base_name`, reachable via `base::name(...)`), or not be redeclared at all (it's already
/// inherited — remove the redeclaration). Also checks `#[overrides] fn` methods the same way
/// against `base`'s effective `#[overridable]` methods.
fn validate_field_overrides(
    from: &Module,
    c: &ComponentDef,
    base: &str,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    if base == "NativeControl" {
        return;
    }
    let Some(base_info) = table.resolve(from, base) else {
        return;
    };

    for f in &c.fields {
        let Some(&base_kind) = base_info.fields.get(f.name.as_str()) else {
            continue;
        };
        let is_override = f.attrs.iter().any(|a| matches!(a, Attr::Override));
        if base_kind != f.kind {
            errors.push(format!(
                "{}.{}: redeclares a field already inherited from `{base}` with a different kind \
                 ({:?} here, {:?} in `{base}`) — an inherited field's kind can't change",
                c.name, f.name, f.kind, base_kind
            ));
        } else if f.kind != FieldKind::Computed {
            errors.push(format!(
                "{}.{}: is already inherited from `{base}` — remove the redeclaration",
                c.name, f.name
            ));
        } else if !is_override {
            errors.push(format!(
                "{}.{}: is inherited as #[computed] from `{base}` — add #[override] to intentionally override it",
                c.name, f.name
            ));
        }
    }

    let base_virtual_methods: HashMap<&str, &crate::ast::MethodDef> = base_info
        .effective_methods
        .iter()
        .filter(|m| m.is_virtual)
        .map(|m| (m.name.as_str(), m))
        .collect();
    for m in &c.methods {
        if !m.is_override {
            continue;
        }
        let Some(base_method) = base_virtual_methods.get(m.name.as_str()) else {
            errors.push(format!(
                "{}: #[overrides] fn {} has no matching #[overridable] method named `{}` on `{base}`",
                c.name, m.name, m.name
            ));
            continue;
        };
        let same_params = m.params.len() == base_method.params.len()
            && m.params
                .iter()
                .zip(base_method.params.iter())
                .all(|((_, ty), (_, base_ty))| {
                    quote::quote!(#ty).to_string() == quote::quote!(#base_ty).to_string()
                });
        let same_return = match (&m.return_ty, &base_method.return_ty) {
            (Some(ty), Some(base_ty)) => {
                quote::quote!(#ty).to_string() == quote::quote!(#base_ty).to_string()
            }
            (None, None) => true,
            _ => false,
        };
        if !same_params || !same_return {
            errors.push(format!(
                "{}: #[overrides] fn {} has a different signature than `{base}`'s #[overridable] fn {}",
                c.name, m.name, m.name
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_module;

    /// Actions can't be declared in the DSL text form's `viewmodel` (only `#[observable]`/
    /// `#[computed]` can) — a viewmodel with an action is always built via the Rust-native
    /// `attr_frontend` frontend, same as the real `#[elwindui::viewmodel]` macro. `path:
    /// Vec::new()` matches the DSL's own crate-root placement, so a plain `vm: NotepadViewModel`
    /// reference elsewhere resolves against it exactly the same way.
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

    #[test]
    fn accepts_notepad_modules() {
        let viewmodel_module = viewmodel_module_from_rust(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = String::new())]
                    content: String,
                }

                impl NotepadViewModel {
                    fn save(&self) {}
                }
            }
        "#,
        );
        let window_src = r#"
component NotepadWindow {
    #[bindable]
    vm: std::rc::Rc<NotepadViewModel>,
}

view NotepadWindow {
    Window { TextArea { text <=> vm.content } }
}
"#;
        let modules: Vec<_> = [viewmodel_module, parse_module(window_src).unwrap()]
            .into_iter()
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_bind_to_unknown_field() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window2 {
    #[bindable]
    vm: std::rc::Rc<Vm>,
}
view Window2 { Window { TextArea { text <=> vm.does_not_exist } } }
"#;
        let modules: Vec<_> = [
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ]
        .into_iter()
        .chain(crate::test_builtin_modules())
        .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("does_not_exist")));
    }

    #[test]
    fn accepts_bindable_field_whose_type_is_a_viewmodel() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window2 {
    #[bindable]
    vm: std::rc::Rc<Vm>,
}
view Window2 { Window { TextBlock { text: "x" } } }
"#;
        let modules = vec![
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ];
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `#[bindable]` (§7.2) is exclusively for viewmodel injection — a `#[bindable]` field whose
    /// type resolves to a plain `component` (not a `viewmodel`) is a mistake this can actually catch
    /// (unlike the cross-macro-boundary case, where the type isn't resolvable at all and the marker
    /// must be trusted — see the check's own doc comment in this file).
    #[test]
    fn rejects_bindable_field_whose_type_is_not_a_viewmodel() {
        let src = r#"
component NotAViewModel {
    #[param]
    label: String,
}
view NotAViewModel { TextBlock { text: label } }

component Window2 {
    #[bindable]
    thing: std::rc::Rc<NotAViewModel>,
}
view Window2 { Window { NotAViewModel { label: "x" } } }
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("isn't a `viewmodel`")),
            "errs: {errs:?}"
        );
    }

    /// `vm.documents` / `vm.save` / `vm.save_can_execute` — the shape `examples/notepad`'s
    /// `notepad_window.rs` actually uses against a `NotepadViewModel` defined elsewhere (not in
    /// this same slice of parsed modules) — must validate cleanly. An action (`save`) resolves
    /// through the exact same 2-segment `[vm_name, field]` check as any other viewmodel field —
    /// there's no separate `Command`-wrapper form to validate.
    #[test]
    fn accepts_valid_vm_field_and_command_references() {
        let viewmodel_module = viewmodel_module_from_rust(
            r#"
            mod notepad_view_model {
                struct NotepadViewModel {
                    #[observable(default = String::new())]
                    documents: String,

                    #[computed(expr = true)]
                    save_can_execute: bool,
                }

                impl NotepadViewModel {
                    fn save(&self) {}
                }
            }
        "#,
        );
        let window_src = r#"
component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,
}

view NotepadWindow {
    Window {
        title: vm.documents
        Button {
            text: t!("save-label")
            on_click: vm.save
            enabled: vm.save_can_execute
        }
    }
}
"#;
        let modules = vec![viewmodel_module, parse_module(window_src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_reference_to_unknown_vm_field() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window3 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window3 { Window { TextBlock { text: vm.no_such_field } } }
"#;
        let modules = vec![
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("no_such_field")),
            "errors: {errs:?}"
        );
    }

    /// An unknown action reference (`vm.no_such_command`, a bare path since actions no longer have
    /// a separate `.execute()` form) is rejected by the same 2-segment `[vm_name, field]` check
    /// `rejects_reference_to_unknown_vm_field` already covers for ordinary fields.
    #[test]
    fn rejects_reference_to_unknown_vm_command() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window4 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window4 { Window { Button { text: "x", on_click: vm.no_such_command } } }
"#;
        let modules = vec![
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("no_such_command")),
            "errors: {errs:?}"
        );
    }

    /// Simulates a Rust-authored viewmodel (`#[elwindui::viewmodel] mod some_vm_mod { struct Vm {..} }`,
    /// real path `["some_vm_mod"]` — see `attr_frontend.rs`/`lib.rs::compile_dir_with_extra_viewmodels`)
    /// referenced by bare name only from a window module, with no `use` bringing it into scope.
    /// Even though a type named `Vm` exists somewhere in the compilation unit, it isn't visible from
    /// the window module's own scope, so this must be a validation error — the same "cannot find type"
    /// Rust itself reports for a missing `use` (this is the exact class of bug
    /// `examples/notepad/src/ui/notepad_window.rs`'s stale `use elwindui::viewmodel::NotepadViewModel;`
    /// used to hide: that `use` didn't resolve to anything real, yet the old flat, path-blind lookup
    /// let the reference through anyway).
    #[test]
    fn rejects_reference_to_a_type_in_a_different_real_module_without_a_use() {
        let vm_module = Module {
            path: vec!["some_vm_mod".to_string()],
            uses: Vec::new(),
            items: parse_module("viewmodel Vm { #[observable] content: String = String::new(), }")
                .unwrap()
                .items,
            ..Default::default()
        };
        let window_src = r#"
component Window6 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window6 { Window { TextArea { text: vm.content } } }
"#;
        let modules = vec![vm_module, parse_module(window_src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("not in scope")),
            "errors: {errs:?}"
        );
    }

    /// The same cross-module setup as above, but with the real path actually `use`d — must resolve
    /// cleanly, exactly like real Rust once the right `use` is in place.
    #[test]
    fn accepts_reference_to_a_type_in_a_different_real_module_when_used() {
        let vm_module = Module {
            path: vec!["some_vm_mod".to_string()],
            uses: Vec::new(),
            items: parse_module("viewmodel Vm { #[observable] content: String = String::new(), }")
                .unwrap()
                .items,
            ..Default::default()
        };
        let window_src = r#"
use crate::some_vm_mod::Vm;

component Window7 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window7 { Window { TextArea { text: vm.content } } }
"#;
        let modules = vec![vm_module, parse_module(window_src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `render_content: |doc| Nonexistent { .. }` — the target must resolve via `SymbolTable`
    /// exactly like a `#[param]` field's type does (`find_vm_fields`), not `panic!` deep inside
    /// `emit_construction`'s codegen-time fallback.
    #[test]
    fn rejects_render_content_targeting_unknown_component() {
        let src = r#"
viewmodel Doc {
    #[observable]
    documents: String = String::new(),
}

component Window8 {
    #[param]
    #[inject]
    vm: Doc,
}

view Window8 {
    Window {
        TabView {
            tabs: vm.documents
            render_content: |doc| Nonexistent { x: doc }
            selected: vm.documents
        }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("Nonexistent")),
            "errors: {errs:?}"
        );
    }

    /// `render_content`'s target component must get every one of its `#[param]`-shaped fields —
    /// otherwise `emit_construction`'s generated `Target::new(...)` call is missing an argument.
    #[test]
    fn rejects_render_content_missing_required_attribute() {
        let src = r#"
viewmodel Doc {
    #[observable]
    documents: String = String::new(),
}

component DocumentView {
    #[param]
    #[inject]
    doc: Doc,
}

component Window9 {
    #[param]
    #[inject]
    vm: Doc,
}

view Window9 {
    Window {
        TabView {
            tabs: vm.documents
            render_content: |doc| DocumentView { }
            selected: vm.documents
        }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("missing required attribute") && e.contains("doc")),
            "errors: {errs:?}"
        );
    }

    /// A closure body may only reference its own bound parameter — a reference to some other,
    /// unrelated name would resolve to a bogus bare identifier under `EmitMode::Construction`
    /// rather than the enclosing component's actual field, so it must be a validation error
    /// instead of a silent miscompile (see `emit_tabview_resync`'s doc comment in `codegen.rs`).
    #[test]
    fn rejects_closure_body_referencing_unrelated_name() {
        let src = r#"
viewmodel Doc {
    #[observable]
    documents: String = String::new(),
}

component Window10 {
    #[param]
    #[inject]
    vm: Doc,
}

view Window10 {
    Window {
        TabView {
            tabs: vm.documents
            render_label: |doc| other_thing.file_name
            selected: vm.documents
        }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("other_thing")),
            "errors: {errs:?}"
        );
    }

    /// The `for` item passthrough case (`doc: doc`) must validate cleanly.
    #[test]
    fn accepts_well_formed_render_content() {
        let src = r#"
viewmodel Doc { }

viewmodel Documents {
    #[observable]
    documents: Vec<std::rc::Rc<Doc>> = Vec::new(),
}

component DocumentView {
    #[param]
    #[inject]
    doc: std::rc::Rc<Doc>,
}

component Window11 {
    #[param]
    #[inject]
    vm: Documents,
}

view Window11 {
    Window {
        TabView {
            for doc in vm.documents {
                TabViewItem { DocumentView { doc: doc } }
            }
        }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `inherits`'s shape-composition use case (docs/specs/dsl_spec.md §3, docs/design/gui_framework_design.md §5.1): a component
    /// inheriting a primitive shape family with no `view` of its own must have its own `view`, whose
    /// body is always implicitly `Shape`'s own attributes/children (Phase 0's implicit-composition
    /// sugar — no `Shape { .. }` wrapper written) — `fill` is inherited from `Rectangle`
    /// automatically, with no redeclaration needed, and `corner_style` is `RoundedPanel`'s own
    /// genuinely new field.
    #[test]
    fn accepts_component_inheriting_a_shape_primitive_via_implicit_composition() {
        let src = r#"
component RoundedPanel inherits Shape {
    #[param]
    corner_style: Option<String>,
}

view RoundedPanel {
    kind: elwindui_core::ui::ShapeKind::RoundedRect { corner_radius: 4.0 }
    fill: fill
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `#[abstract]` (docs/specs/dsl_spec.md 付録A): `Shape` is a pure category tag that `Rectangle`/
    /// `Ellipse` shape-compose over — using it directly as a view root *without* declaring
    /// `inherits Shape` is not legitimate composition, so it's rejected the same as any other bare
    /// use (unlike `accepts_component_inheriting_a_shape_primitive_via_implicit_composition`, which
    /// *does* declare `inherits Shape` and must keep working).
    #[test]
    fn rejects_abstract_component_used_as_a_bare_view_root_without_inherits() {
        let src = r#"
component Foo {
}

view Foo {
    Shape { kind: elwindui_core::ui::ShapeKind::Oval }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("Shape") && e.contains("abstract")),
            "errors: {errs:?}"
        );
    }

    /// Same rule, but for a nested (non-root) use — `NativeControl` (another `#[abstract]` category
    /// tag) written as a bare child inside an ordinary container.
    #[test]
    fn rejects_abstract_component_used_as_a_nested_child() {
        let src = r#"
component Foo {
}

view Foo {
    VerticalLayout {
        NativeControl { }
    }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("NativeControl") && e.contains("abstract")),
            "errors: {errs:?}"
        );
    }

    /// Phase 0 (docs/design/gui_framework_design.md §5.1) removed the old "own `view`'s root element must
    /// literally construct `base`" requirement entirely — a composable base's `view` body is always
    /// implicitly its own attributes/children now, so there's no longer a *root shape* for
    /// `validate::validate` to reject here. `Shape` has no `#[content(...)]` field to bind a bare
    /// `VerticalLayout {}` child to at all — a pre-existing gap unrelated to Phase 0,
    /// `generate_module` silently constructs and discards it rather than erroring (unlike a
    /// hand-written/logical component's own `build_component_args`, which does reject this same
    /// shape — see `codegen.rs`'s `panics_on_bare_child_with_no_content_field_declared`).
    #[test]
    fn accepts_inherits_regardless_of_bare_child_shape_since_composition_is_now_always_implicit() {
        let src = r#"
component RoundedPanel inherits Shape {
    #[param]
    corner_style: Option<String>,
}

view RoundedPanel {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// Phase 2 (docs/design/gui_framework_design.md §5.1): a scalar `#[content(...)]` field (`ContentControl`'s
    /// `content: Rc<dyn UIElement>`) can host `if`/`match` dynamic children now, but never `for` — a
    /// variable-length list can never fit a single-value slot.
    #[test]
    fn rejects_for_under_a_scalar_content_field() {
        let src = r#"
viewmodel DynamicViewModel {
    #[observable]
    items: Vec<String> = Vec::new(),
}

component DynamicHost inherits ContentControl {
    #[param]
    #[inject]
    vm: DynamicViewModel,
}

view DynamicHost {
    for item in vm.items { TextBlock { text: item } }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("scalar type") && e.contains("ContentControl")),
            "errors: {errs:?}"
        );
    }

    /// A scalar content field's `if` is only valid when *both* branches resolve to exactly one
    /// element — a branch with two bare children has nowhere for the second one to go.
    #[test]
    fn rejects_multiple_children_in_one_branch_under_a_scalar_content_field() {
        let src = r#"
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
        TextBlock { text: "a2" }
    } else {
        TextBlock { text: "b" }
    }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("scalar type") && e.contains("ContentControl")),
            "errors: {errs:?}"
        );
    }

    /// Redeclaring a field already inherited from a non-`NativeControl` base (without
    /// `#[computed]`+`#[override]`) is an error — real field inheritance means it's already
    /// available via `self`, so redeclaring it is either a mistake or dead weight.
    #[test]
    fn rejects_redeclaring_an_inherited_field() {
        let src = r#"
component RoundedPanel inherits Shape {
    #[param]
    fill: Option<String>,
}

view RoundedPanel {
    Shape { kind: elwindui_core::ui::ShapeKind::RoundedRect { corner_radius: 4.0 }, fill: fill }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("already inherited")),
            "errors: {errs:?}"
        );
    }

    /// An `inherits` base this compilation has no local `TypeInfo` for is no longer flagged here —
    /// deliberately, not an oversight (see `validate_inherits`'s own doc comment on its `None` arm).
    /// `elwindui-codegen` cannot tell "a builtin declared entirely in `elwindui-core`" apart from a
    /// genuine typo without a shape table, so it assumes the former, the same way every other
    /// builtin-shape decision in this codebase now does. A genuine typo (`inherits DoesNotExist`)
    /// still fails to compile — just later, at `#[class]`'s own generated-code level (`cannot find
    /// macro __elwindui_inherit_DoesNotExist`) rather than here.
    #[test]
    fn accepts_inherits_of_an_unresolved_base_deferring_to_the_generated_code() {
        let src = r#"
component Foo inherits DoesNotExist {
}

view Foo {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert!(
            validate(&modules).is_ok(),
            "an unresolved `inherits` base should no longer be rejected by this pass"
        );
    }

    /// `inherits NativeControl` is a pure category tag checked for *consistency* against the
    /// structurally-inferred `is_native` (see `codegen::build_symbol_table`'s `resolve_is_native`)
    /// — claiming it while the `view` root is actually virtual is an error.
    #[test]
    fn rejects_inherits_native_control_when_view_root_is_virtual() {
        let src = r#"
component Foo inherits NativeControl {
}

view Foo {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("NativeControl")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn accepts_inherits_native_control_when_view_root_is_native() {
        let src = r#"
component Foo inherits NativeControl {
}

view Foo {
    Window { title: "x", content: TextBlock { text: "hi" } }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// A plain `component`+`view` pair with *no* `inherits` at all is still correctly inferred as
    /// virtual when its view's root is a virtual builtin — `is_native` is structural, not merely
    /// "did the author write `inherits`" (mirrors `examples/notepad`'s real `DocumentView`).
    #[test]
    fn is_native_is_inferred_recursively_without_requiring_inherits() {
        let src = r#"
component DocumentViewLike {
}

view DocumentViewLike {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let table = codegen::build_symbol_table(&modules);
        let info = table
            .resolve(&modules[0], "DocumentViewLike")
            .expect("resolves");
        assert!(!info.is_native);

        let native_info = table.resolve(&modules[0], "Window").expect("resolves");
        assert!(native_info.is_native);

        let virtual_builtin_info = table
            .resolve(&modules[0], "VerticalLayout")
            .expect("resolves");
        assert!(!virtual_builtin_info.is_native);
    }

    /// `Window` declares `#[native]` with **no** `inherits` at all (unlike `Button`/`TextArea`/...,
    /// which reach `is_native` via `inherits NativeControl` — see `Window`'s own `#[class]` doc comment
    /// for why `Window` deliberately doesn't share that tag). `resolve_is_native`'s `#[native]`
    /// fallback must still resolve it to native.
    #[test]
    fn window_is_native_via_native_attribute_without_inherits() {
        let modules = crate::test_builtin_modules();
        let window_module = modules
            .iter()
            .find(|m| {
                m.items
                    .iter()
                    .any(|i| matches!(i, Item::Component(c) if c.name == "Window"))
            })
            .expect("Window's module");
        let Item::Component(window_def) = window_module
            .items
            .iter()
            .find(|i| matches!(i, Item::Component(c) if c.name == "Window"))
            .unwrap()
        else {
            unreachable!()
        };
        assert!(
            window_def.base.is_none(),
            "Window must have no `inherits` base"
        );
        assert!(window_def.native, "Window must be #[native]");

        let table = codegen::build_symbol_table(&modules);
        let info = table.resolve(window_module, "Window").expect("resolves");
        assert!(info.is_native);
        assert!(!info.has_view);
        assert_eq!(info.content_field.as_deref(), Some("content"));
    }

    /// `#[content(field_name)]` (WinUI3's `ContentPropertyAttribute` equivalent) must name a real
    /// field of the component it's declared on — a typo here would otherwise silently mean "no bare
    /// nested child ever binds anywhere", so it's checked statically instead of only surfacing (if
    /// at all) as a `build_component_args` codegen panic the first time someone actually nests a
    /// bare child under it.
    #[test]
    fn rejects_content_attribute_naming_an_unknown_field() {
        let src = r#"
#[content(no_such_field)]
component Foo {
    #[param]
    label: String,
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("#[content(no_such_field)]")),
            "errors: {errs:?}"
        );
    }

    /// `#[native]` requires a `base`-less declaration — `resolve_is_native`'s fallback only checks
    /// `#[native]` when there's no `inherits` base to begin with (`validate_inherits` is never even
    /// reached for a base-less component), so combining both is a static error instead of silently
    /// ignoring one.
    #[test]
    fn rejects_native_attribute_combined_with_inherits() {
        let src = r#"
#[native]
component Foo inherits NativeControl {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[native]") && e.contains("inherits")),
            "errors: {errs:?}"
        );
    }

    /// `#[native]` means "hand-written per backend crate" — a component that also writes its own
    /// `view` contradicts that (there'd be generated Rust *and* a claimed hand-written one).
    #[test]
    fn rejects_native_attribute_combined_with_own_view() {
        let src = r#"
#[native]
component Foo {
}

view Foo {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[native]") && e.contains("view")),
            "errors: {errs:?}"
        );
    }

    /// `#[native]`, like `#[embedded]`, only makes sense on one of this crate's own builtin shape
    /// components — a consumer's own source has no way to actually provide a hand-written
    /// per-backend implementation for it.
    #[test]
    fn rejects_native_attribute_outside_builtin_module() {
        let src = r#"
#[native]
component Foo {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[native]") && e.contains("BUILTIN_SHAPE_SOURCE")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_text_style_attribute_outside_builtin_module() {
        let src = r#"
#[text_style]
component Foo {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[text_style]") && e.contains("BUILTIN_SHAPE_SOURCE")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_text_style_attribute_combined_with_own_field_of_the_same_name() {
        let mut module = parse_module(
            r#"
#[text_style]
component Foo {
    font_size: Option<f32>,
}
"#,
        )
        .unwrap();
        module.is_builtin = true;
        let modules: Vec<_> = std::iter::once(module)
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[text_style]") && e.contains("font_size")),
            "errors: {errs:?}"
        );
    }

    /// A native-backed leaf (`Window`, `has_view == false && is_native == true`) has no generated
    /// Rust to inherit from — only `NativeControl` may be used as a pure category tag. `Window`
    /// (not `Button`, unlike before `#[sealed]` existed) is used here because it isn't itself
    /// `#[sealed]` — `Button` now gets rejected for that reason first instead (see
    /// `rejects_inherits_of_a_sealed_component`), which would no longer exercise this underlying
    /// native-leaf rejection path.
    #[test]
    fn rejects_inherits_of_a_native_leaf_with_no_matching_view() {
        let src = r#"
component MyWindow inherits Window {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("has no `view MyWindow`")),
            "errors: {errs:?}"
        );
    }

    /// `Window` is a hand-written native host with real fields and no `UIElement` implementation of
    /// its own ("host composition", `codegen::TypeInfo::host_composition_base`) — inheriting it is
    /// allowed exactly like inheriting a primitive shape family (`Control`/`Rectangle`): the
    /// inheritor's own `view` root must literally construct it. See `examples/notepad`'s real
    /// `NotepadWindow inherits Window`.
    #[test]
    fn accepts_inherits_of_a_native_host_with_matching_view_root() {
        let src = r#"
component MyWindow inherits Window {
}

view MyWindow {
    Window { title: "x", content: TextBlock { text: "hi" } }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `Button` is `#[sealed]` (docs/specs/dsl_spec.md 付録A) — `validate_inherits` must reject a
    /// further `inherits Button` for that reason specifically, not just the more general
    /// native-backed-leaf rejection `rejects_inherits_of_a_native_leaf` covers.
    #[test]
    fn rejects_inherits_of_a_sealed_component() {
        let src = r#"
component MyButton inherits Button {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("Button") && e.contains("sealed")),
            "errors: {errs:?}"
        );
    }

    /// A logical component (`has_view == true`, e.g. `ContentControl`) may be inherited with *no*
    /// `view` of its own at all — WinUI3-style template inheritance (`codegen::resolve_view_for`).
    #[test]
    fn accepts_inheriting_a_logical_component_with_no_own_view() {
        let src = r#"
component LabeledPanel inherits ContentControl {
    #[param]
    label: String,
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// A logical component base's `view` is a full template override when the derived writes its
    /// own — unlike the primitive-shape-family case, there's no constraint that the root element
    /// literally construct `Base`.
    #[test]
    fn accepts_full_view_override_of_a_logical_component_base() {
        let src = r#"
component LabeledPanel inherits ContentControl {
    #[param]
    label: String,
}

view LabeledPanel {
    VerticalLayout { TextBlock { text: label } }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// A redeclared `#[computed]` field matching an inherited one is an intentional override only
    /// when marked `#[override]` — otherwise it's an accidental-shadowing error.
    #[test]
    fn rejects_computed_field_override_without_override_attr() {
        let src = r#"
component Base {
    #[computed]
    label: String = "base".to_string(),
}

component Derived inherits Base {
    #[computed]
    label: String = "derived".to_string(),
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("add #[override]")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn accepts_computed_field_override_with_override_attr() {
        let src = r#"
component Base {
    #[computed]
    label: String = "base".to_string(),
}

view Base { VerticalLayout { } }

component Derived inherits Base {
    #[override]
    #[computed]
    label: String = "derived".to_string(),
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `#[override] fn` must name-match a base `#[virtual]` method with the same signature.
    #[test]
    fn rejects_override_method_with_no_matching_virtual_base_method() {
        let src = r#"
component Base {
    #[virtual]
    fn label(&self) -> String {
        "base".to_string()
    }
}

component Derived inherits Base {
    #[override]
    fn not_label(&self) -> String {
        "derived".to_string()
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("no matching #[overridable] method")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_override_method_with_mismatched_signature() {
        let src = r#"
component Base {
    #[virtual]
    fn label(&self) -> String {
        "base".to_string()
    }
}

component Derived inherits Base {
    #[override]
    fn label(&self, suffix: i32) -> String {
        format!("derived{}", suffix)
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("different signature")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn accepts_override_method_with_matching_signature() {
        let src = r#"
component Base {
    #[virtual]
    fn label(&self) -> String {
        "base".to_string()
    }
}

view Base { VerticalLayout { } }

component Derived inherits Base {
    #[override]
    fn label(&self) -> String {
        format!("{}!", base::label())
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_attached_field_without_default_value() {
        let src = r#"
component Grid {
    #[attached]
    row: i32,
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("default value")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_unknown_attached_property() {
        let src = r#"
component MyGrid {
    #[attached]
    row: i32 = 0,
}

component Foo {
}

view Foo {
    VerticalLayout {
        TextBlock { text: "hi", MyGrid::column: 1 }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("no #[attached] property named `column`")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_attached_property_on_unknown_owner() {
        let src = r#"
component Foo {
}

view Foo {
    VerticalLayout {
        TextBlock { text: "hi", NoSuchOwner::row: 1 }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("not a known component/builtin")),
            "errors: {errs:?}"
        );
    }

    /// An attached property may be set on an element that isn't actually nested under a matching
    /// owner anywhere — like WPF, this is inert at runtime, not a static error.
    #[test]
    fn accepts_attached_property_even_when_not_nested_under_its_owner() {
        let src = r#"
component MyGrid {
    #[attached]
    row: i32 = 0,
    #[attached]
    column: i32 = 0,
}

component Foo {
}

view Foo {
    VerticalLayout {
        TextBlock { text: "hi", MyGrid::row: 1, MyGrid::column: 0 }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_non_exhaustive_enum_match_in_a_view() {
        let src = r#"
enum Status { Loading, Ready }

component Screen {
    status: Status,
}

view Screen {
    VerticalLayout {
        match status {
            Status::Loading => TextBlock { text: "loading" },
        }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        let errors = validate(&modules).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("not exhaustive") && error.contains("Ready")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn rejects_shortcut_on_non_routed_attribute() {
        let src = r#"
component SaveField { }
view SaveField {
    Button {
        #[shortcut("Ctrl+S")]
        text: "Save"
    }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errors = validate(&modules).unwrap_err();
        assert!(
            errors.iter().any(|e| e.contains("#[routed]")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn rejects_invalid_shortcut_key_spec() {
        let src = r#"
component SaveField { }
view SaveField {
    Button {
        #[shortcut("Hyper+S")]
        on_click: save
    }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errors = validate(&modules).unwrap_err();
        assert!(
            errors.iter().any(|e| e.contains("Hyper")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn accepts_valid_shortcut_on_routed_attribute() {
        let src = r#"
component SaveField { }
view SaveField {
    Button {
        #[shortcut("Ctrl+S")]
        on_click: save
    }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_two_way_target_without_capability() {
        let src = r#"
component Search {
    #[state]
    query: String = String::new(),
}
view Search { TextBlock { text <=> query } }
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errors = validate(&modules).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("does not support #[two_way]")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn rejects_non_writable_two_way_rhs_and_for_item_two_way() {
        let expression_src = r#"
component Search { }
view Search { TextArea { text <=> format!("fixed") } }
"#;
        let modules: Vec<_> = std::iter::once(parse_module(expression_src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errors = validate(&modules).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("two-way RHS must be")),
            "errors: {errors:?}"
        );

        let for_src = r#"
component Search {
    #[param]
    items: Vec<String>,
}
view Search {
    VerticalLayout {
        for item in items { TextArea { text <=> item.content } }
    }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(for_src).unwrap())
            .chain(crate::test_builtin_modules())
            .collect();
        let errors = validate(&modules).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("for` item template")),
            "errors: {errors:?}"
        );
    }
}
