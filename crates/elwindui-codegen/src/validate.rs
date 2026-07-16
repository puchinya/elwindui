//! A narrow slice of the ~24 static verification rules in docs/elwindui_spec.md §14 — only the
//! ones reachable by the constructs the notepad example actually uses. See
//! docs/elwindui_gui_framework_design.md §10 for the full rule list.

use crate::ast::{
    Attr, ChildEntry, ClosureBody, ComponentDef, ElementNode, FieldDef, FieldKind, Initializer,
    Item, Module, ViewExpr,
};
use crate::codegen::{self, SymbolTable, strip_rc_wrapper};
use std::collections::{HashMap, HashSet};

pub fn validate(modules: &[Module]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // The same real-path-aware resolver `codegen.rs` uses for code generation, reused here so
    // `vm.field` / `bind!(vm.content, ..)` / etc. are checked against exactly what's actually in
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
                    // `#[embedded]` (docs/elwindui_spec.md 付録E) claims this component is one of
                    // this crate's own builtin shape declarations — reject it on anything parsed
                    // from a consumer's own `.elwind` directory (`Module::is_builtin`, set only by
                    // `builtin_modules()`).
                    if c.embedded && !module.is_builtin {
                        errors.push(format!(
                            "{}: #[embedded] can only be used on a component from elwindui-codegen's own \
                             BUILTIN_SHAPE_SOURCE, not a consumer's own `.elwind` file",
                            c.name
                        ));
                    }

                    // `#[native]` (docs/elwindui_spec.md 付録E, `ComponentDef::native`'s doc
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
                                 BUILTIN_SHAPE_SOURCE, not a consumer's own `.elwind` file",
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

                    // `#[content(field_name)]` (docs/elwindui_spec.md 付録E, WinUI3's
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
                        // Rule 18: `#[command]` field type must be `Command`.
                        if f.kind == FieldKind::Command && f.ty != "Command" {
                            errors.push(format!(
                                "{}.{}: #[command] field must have type `Command`, found `{}`",
                                c.name, f.name, f.ty
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
                        if let Some(Initializer::Bind { path, .. }) = &f.initializer {
                            validate_bind_path(
                                module,
                                &c.name,
                                &f.name,
                                path,
                                &c.fields,
                                &table,
                                &mut errors,
                            );
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
                            check_tab_view_mode(&let_binding.element, &c.name, &mut errors);
                            check_attached_properties(
                                &let_binding.element,
                                module,
                                &c.name,
                                &table,
                                &mut errors,
                            );
                        }
                        check_vm_references(
                            &view.root,
                            module,
                            &c.name,
                            &vm_fields,
                            &table,
                            c.base.as_deref(),
                            &mut errors,
                        );
                        check_tab_view_mode(&view.root, &c.name, &mut errors);
                        check_attached_properties(&view.root, module, &c.name, &table, &mut errors);
                    }
                }
                Item::ViewModel(v) => {
                    for f in &v.fields {
                        if f.kind == FieldKind::Command && f.ty != "Command" {
                            errors.push(format!(
                                "{}.{}: #[command] field must have type `Command`, found `{}`",
                                v.name, f.name, f.ty
                            ));
                        }
                        // Rule 19 (viewmodel must not reference view/builtin elements) holds by
                        // construction: `ViewModelDef` has no `view` body in this AST.
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
/// Also rejects `node` itself naming an `#[abstract]` component (docs/elwindui_spec.md 付録E) —
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
    for (_, expr) in &node.attributes {
        check_vm_expr(expr, from, component_name, vm_fields, table, errors);
    }
    for child in &node.children {
        // A `ChildEntry::Ref` doesn't need its own recursive check here — the `let` binding it
        // refers to is itself already walked as one of `view.lets` in `validate`'s main loop.
        if let ChildEntry::Literal(elem) = child {
            check_vm_references(elem, from, component_name, vm_fields, table, None, errors);
        }
    }
}

/// `#[abstract]` (docs/elwindui_spec.md 付録E): a pure category tag (`UIElement`/`NativeControl`/
/// `Layout`/`Shape` in `builtins.elwind`) cannot be instantiated directly — only named as an
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
            // `vm.command.can_execute` (付録O.4's 3-segment special form).
            [vm_name, command, suffix] if suffix == "can_execute" => {
                if let Some(&ty) = vm_fields.get(vm_name.as_str()) {
                    let is_command = table.resolve(from, ty).is_some_and(|info| {
                        info.fields.get(command.as_str()) == Some(&FieldKind::Command)
                    });
                    if !is_command {
                        errors.push(format!(
                            "{component_name}: `{vm_name}.{command}.can_execute` — `{ty}` has no command `{command}`"
                        ));
                    }
                }
            }
            _ => {}
        },
        ViewExpr::MethodCall(path, method) if method == "execute" => {
            if let [vm_name, command] = path.as_slice() {
                if let Some(&ty) = vm_fields.get(vm_name.as_str()) {
                    let is_command = table.resolve(from, ty).is_some_and(|info| {
                        info.fields.get(command.as_str()) == Some(&FieldKind::Command)
                    });
                    if !is_command {
                        errors.push(format!(
                            "{component_name}: `{vm_name}.{command}.execute()` — `{ty}` has no command `{command}`"
                        ));
                    }
                }
            }
        }
        ViewExpr::MethodCall(..) | ViewExpr::Expr(_) => {}
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_vm_expr(arg, from, component_name, vm_fields, table, errors);
            }
        }
        ViewExpr::Closure { param, body } => match body {
            ClosureBody::Expr(inner) => check_closure_expr_body(
                inner,
                param,
                from,
                component_name,
                vm_fields,
                table,
                errors,
            ),
            ClosureBody::Element(elem) => check_element_value(
                elem,
                Some(param),
                from,
                component_name,
                vm_fields,
                table,
                errors,
            ),
        },
        ViewExpr::Element(elem) => {
            check_element_value(elem, None, from, component_name, vm_fields, table, errors)
        }
    }
}

/// Checks a closure body (`header_template`/`item_template`'s `|param| ...`): a reference is
/// valid if its first segment is the closure's own bound parameter (the parameter isn't a `vm_fields`-tracked
/// component/viewmodel) or a recognized `vm`-style field (checked the normal way via
/// `check_vm_expr`). Anything else is an error — see `emit_expr` in `codegen.rs` for why an
/// outer-component reference from inside a closure body would otherwise silently resolve to a
/// bogus bare identifier instead of failing to compile.
fn check_closure_expr_body(
    expr: &ViewExpr,
    param: &str,
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    let first_segment = match expr {
        ViewExpr::Path(path) => path.first(),
        ViewExpr::MethodCall(path, _) => path.first(),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_closure_expr_body(arg, param, from, component_name, vm_fields, table, errors);
            }
            return;
        }
        // A raw `syn::Expr` (e.g. `std::rc::Rc::as_ptr(doc) as usize`) isn't inspected further,
        // matching how ordinary (non-closure) `Expr` values are already left unvalidated above.
        ViewExpr::Expr(_) => return,
        // The parser never produces a closure directly nested inside another closure's expression
        // body, nor a bare element there (an element-valued closure body is always
        // `ClosureBody::Element`, handled separately by `check_vm_expr`'s own `Closure` arm).
        ViewExpr::Closure { .. } | ViewExpr::Element(_) => return,
    };
    match first_segment {
        Some(first) if first == param => {}
        Some(first) if vm_fields.contains_key(first.as_str()) => {
            check_vm_expr(expr, from, component_name, vm_fields, table, errors);
        }
        Some(first) => errors.push(format!(
            "{component_name}: closure body references `{first}`, which is neither the closure's own parameter `{param}` nor a recognized field — a closure may only reference its own bound parameter"
        )),
        None => {}
    }
}

/// Rule (付録Y): a `TabView` must use *exactly one* of its two mutually exclusive child-declaration
/// modes — nested `TabViewItem`s written literally in `{}` (static), or `items_source` (+
/// `header_template`/`item_template`, dynamic). Both or neither is ambiguous/incomplete and is
/// rejected here rather than left to fail confusingly deep in codegen. Walks the whole `view` tree,
/// including into element-valued attributes/closure bodies (`menu_bar: MenuBar { .. }`-style named
/// slots and `header_template`/`item_template` closures), since a `TabView` may appear nested
/// inside either.
fn check_tab_view_mode(node: &ElementNode, component_name: &str, errors: &mut Vec<String>) {
    if node.type_path == "TabView" {
        let has_children = !node.children.is_empty();
        let has_items_source = node
            .attributes
            .iter()
            .any(|(name, _)| name == "items_source");
        match (has_children, has_items_source) {
            (true, true) => errors.push(format!(
                "{component_name}: `TabView` has both nested `TabViewItem` children and `items_source` — use exactly one (static nesting or `items_source`, not both)"
            )),
            (false, false) => errors.push(format!(
                "{component_name}: `TabView` has neither nested `TabViewItem` children nor `items_source` — must use exactly one"
            )),
            _ => {}
        }
    }
    for child in &node.children {
        if let ChildEntry::Literal(elem) = child {
            check_tab_view_mode(elem, component_name, errors);
        }
    }
    for (_, expr) in &node.attributes {
        check_tab_view_mode_in_expr(expr, component_name, errors);
    }
}

fn check_tab_view_mode_in_expr(expr: &ViewExpr, component_name: &str, errors: &mut Vec<String>) {
    match expr {
        ViewExpr::Element(elem) => check_tab_view_mode(elem, component_name, errors),
        ViewExpr::Closure {
            body: ClosureBody::Element(elem),
            ..
        } => check_tab_view_mode(elem, component_name, errors),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_tab_view_mode_in_expr(arg, component_name, errors);
            }
        }
        ViewExpr::Path(_)
        | ViewExpr::MethodCall(..)
        | ViewExpr::Expr(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_),
            ..
        } => {}
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
    for (_, expr) in &node.attributes {
        check_attached_properties_in_expr(expr, from, component_name, table, errors);
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
        | ViewExpr::MethodCall(..)
        | ViewExpr::Expr(_)
        | ViewExpr::Closure {
            body: ClosureBody::Expr(_),
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
                let has_attr = elem.attributes.iter().any(|(k, _)| k == name);
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
        None => errors.push(format!(
            "{component_name}: `{}` is an unknown or out-of-scope component — add a `use` for it",
            elem.type_path
        )),
    }
    for (_, value) in &elem.attributes {
        match param {
            Some(param) => check_closure_expr_body(
                value,
                param,
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

/// Checks `bind!(vm.content, ...)`: `vm` must be a field of the enclosing `component` whose type
/// names a `component`/`viewmodel` in scope from `from`, and `content` must be one of that type's
/// fields. Generalizes rules 12/13 (written against `store` in the spec) to any bindable owner.
fn validate_bind_path(
    from: &Module,
    owner_name: &str,
    field_name: &str,
    path: &[String],
    own_fields: &[FieldDef],
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    let [root, target_field] = path else {
        errors.push(format!(
            "{owner_name}.{field_name}: bind! path must be `owner.field`, found `{}`",
            path.join(".")
        ));
        return;
    };

    let Some(root_field) = own_fields.iter().find(|f| &f.name == root) else {
        errors.push(format!(
            "{owner_name}.{field_name}: bind! refers to unknown field `{root}`"
        ));
        return;
    };

    match table.resolve(from, strip_rc_wrapper(&root_field.ty)) {
        Some(info) if info.fields.contains_key(target_field.as_str()) => {}
        Some(_) => errors.push(format!(
            "{owner_name}.{field_name}: `{}` has no field `{target_field}`",
            root_field.ty
        )),
        None => errors.push(format!(
            "{owner_name}.{field_name}: unknown type `{}` for bind! target `{root}`",
            root_field.ty
        )),
    }
}

/// Checks `component X inherits Base { .. }` (docs/elwindui_spec.md §3): `Base` must resolve, then
/// branches on what kind of base it is:
/// - `X` itself is a hand-written virtual builtin (`codegen::is_virtual_builtin` —
///   `VerticalLayout`/`HorizontalLayout`/`TextBlock`/`Control`/`Grid`/`Shape`): unconditionally
///   allowed regardless of `Base`'s own shape. A virtual builtin is constructed entirely by
///   `codegen::build_virtual_value`'s per-type-name `match`, never through a `view` — it's
///   structurally incapable of having one, so none of the `view`-based checks below apply (this is
///   what lets `Layout` carry a real `children: UIElementCollection` field — see that component's
///   own doc comment in `builtins.elwind` — without breaking `VerticalLayout`/`HorizontalLayout`/
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
///   docs/elwindui_spec.md 付録H.2.1a).
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
        errors.push(format!(
            "{}: inherits `{base}`, but `{base}` is not a known component/builtin (missing `use`?)",
            c.name
        ));
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

    // The three root category tags of the whole class hierarchy (docs/elwindui_spec.md 付録
    // H.2.1a) — `UIElement` (the root), and its two immediate abstract branches `Layout`/
    // `NativeControl` — are never themselves a `view`'s root anywhere (structurally: nothing
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

    // A primitive shape family (`has_view == false`, not native): `X` must have its own `view`
    // whose root element is literally `Base` — unchanged shape-composition contract.
    let view = modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|item| match item {
            Item::View(v) if v.target == c.name => Some(v),
            _ => None,
        });
    match view {
        None => errors.push(format!(
            "{}: inherits `{base}`, but has no `view {}` — a component inheriting a shape \
             primitive with no `view` of its own must have its view's root element construct `{base}`",
            c.name, c.name
        )),
        Some(v) if v.root.type_path != base => errors.push(format!(
            "{}: inherits `{base}`, so `view {}`'s root element must be `{base}`, found `{}`",
            c.name, c.name, v.root.type_path
        )),
        Some(_) => {}
    }
}

/// Checks field-level `inherits` overrides (§3): a field this component redeclares that's already
/// present on `base` (its effective, recursively-flattened field list) must either match kind
/// exactly and be `#[computed]` with `#[override]` (an intentional override — codegen's
/// `resolve_effective_fields`/`resolve_effective_methods` shadow-copies `base`'s original body
/// under `__base_name`, reachable via `base::name(...)`), or not be redeclared at all (it's already
/// inherited — remove the redeclaration). Also checks `#[override] fn` methods the same way against
/// `base`'s effective `#[virtual]` methods.
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
                "{}: #[override] fn {} has no matching #[virtual] method named `{}` on `{base}`",
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
                "{}: #[override] fn {} has a different signature than `{base}`'s #[virtual] fn {}",
                c.name, m.name, m.name
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_module;

    #[test]
    fn accepts_notepad_modules() {
        let viewmodel_src = r#"
enum SaveState { Unsaved, Saving, Saved }

viewmodel NotepadViewModel {
    #[observable]
    content: String = String::new(),

    #[command(can_execute: true)]
    save: Command = command!(|| {}),
}
"#;
        let window_src = r#"
component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    content: String = bind!(vm.content, TwoWay),
}

view NotepadWindow {
    Window { TextArea { text: content } }
}
"#;
        let modules = vec![
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ];
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_bind_to_unknown_field() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window2 {
    #[param]
    #[inject]
    vm: Vm,

    missing: String = bind!(vm.does_not_exist, TwoWay),
}
view Window2 { Window { TextArea { text: missing } } }
"#;
        let modules = vec![
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ];
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("does_not_exist")));
    }

    /// `vm.documents` / `vm.save.execute()` / `vm.save.can_execute` — the shape
    /// `examples/notepad`'s `notepad_window.elwind` actually uses against a `NotepadViewModel`
    /// defined elsewhere (not in this same slice of parsed modules) — must validate cleanly.
    #[test]
    fn accepts_valid_vm_field_and_command_references() {
        let viewmodel_src = r#"
viewmodel NotepadViewModel {
    #[observable]
    documents: String = String::new(),

    #[command(can_execute: true)]
    save: Command = command!(|| {}),
}
"#;
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
            on_click: vm.save.execute()
            enabled: vm.save.can_execute
        }
    }
}
"#;
        let modules = vec![
            parse_module(viewmodel_src).unwrap(),
            parse_module(window_src).unwrap(),
        ];
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

    #[test]
    fn rejects_reference_to_unknown_vm_command() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window4 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window4 { Window { Button { text: "x", on_click: vm.no_such_command.execute() } } }
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

    #[test]
    fn rejects_reference_to_unknown_vm_can_execute() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window5 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window5 { Window { Button { text: "x", enabled: vm.no_such_command.can_execute } } }
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
    /// referenced by bare name only from a `.elwind` window file, with no `use` bringing it into scope.
    /// Even though a type named `Vm` exists somewhere in the compilation unit, it isn't visible from
    /// the window module's own scope, so this must be a validation error — the same "cannot find type"
    /// Rust itself reports for a missing `use` (this is the exact class of bug
    /// `examples/notepad/src/ui/notepad_window.elwind`'s stale `use elwindui::viewmodel::NotepadViewModel;`
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

    /// The passthrough case (`doc: doc`) and a well-formed `item_template` must validate cleanly.
    #[test]
    fn accepts_well_formed_render_content() {
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

component Window11 {
    #[param]
    #[inject]
    vm: Doc,
}

view Window11 {
    Window {
        TabView {
            items_source: vm.documents
            header_template: |doc| doc.file_name
            item_template: |doc| DocumentView { doc: doc }
            selected_index: vm.documents
        }
    }
}
"#;
        let modules = vec![parse_module(src).unwrap()];
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `inherits`'s shape-composition use case (docs/elwindui_spec.md §3): a component inheriting a
    /// primitive shape family with no `view` of its own must have its own `view`'s root element
    /// literally construct that base — `fill` is inherited from `Rectangle` automatically, with no
    /// redeclaration needed, and `corner_style` is `RoundedPanel`'s own genuinely new field.
    #[test]
    fn accepts_component_inheriting_a_shape_primitive_with_matching_view_root() {
        let src = r#"
component RoundedPanel inherits Shape {
    #[param]
    corner_style: Option<String>,
}

view RoundedPanel {
    Shape { kind: elwindui_core::ui::ShapeKind::RoundedRect { corner_radius: 4.0 }, fill: fill }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `#[abstract]` (docs/elwindui_spec.md 付録E): `Shape` is a pure category tag that `Rectangle`/
    /// `Ellipse` shape-compose over — using it directly as a view root *without* declaring
    /// `inherits Shape` is not legitimate composition, so it's rejected the same as any other bare
    /// use (unlike `accepts_component_inheriting_a_shape_primitive_with_matching_view_root`, which
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("NativeControl") && e.contains("abstract")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_inherits_when_view_root_does_not_match_base() {
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
            .chain(crate::builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("must be `Shape`")),
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
            .chain(crate::builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("already inherited")),
            "errors: {errs:?}"
        );
    }

    #[test]
    fn rejects_inherits_of_unknown_base() {
        let src = r#"
component Foo inherits DoesNotExist {
}

view Foo {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("not a known component/builtin")),
            "errors: {errs:?}"
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
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
    /// which reach `is_native` via `inherits NativeControl` — see `window.elwind`'s own doc comment
    /// for why `Window` deliberately doesn't share that tag). `resolve_is_native`'s `#[native]`
    /// fallback must still resolve it to native.
    #[test]
    fn window_is_native_via_native_attribute_without_inherits() {
        let modules = crate::builtin_modules();
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[native]") && e.contains("view")),
            "errors: {errs:?}"
        );
    }

    /// `#[native]`, like `#[embedded]`, only makes sense on one of this crate's own builtin shape
    /// components — a consumer's own `.elwind` file has no way to actually provide a hand-written
    /// per-backend implementation for it.
    #[test]
    fn rejects_native_attribute_outside_builtin_module() {
        let src = r#"
#[native]
component Foo {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::builtin_modules())
            .collect();
        let errs = validate(&modules).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("#[native]") && e.contains("BUILTIN_SHAPE_SOURCE")),
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
            .collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    /// `Button` is `#[sealed]` (docs/elwindui_spec.md 付録E) — `validate_inherits` must reject a
    /// further `inherits Button` for that reason specifically, not just the more general
    /// native-backed-leaf rejection `rejects_inherits_of_a_native_leaf` covers.
    #[test]
    fn rejects_inherits_of_a_sealed_component() {
        let src = r#"
component MyButton inherits Button {
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap())
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
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
            .chain(crate::builtin_modules())
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
                .any(|e| e.contains("no matching #[virtual] method")),
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
}
