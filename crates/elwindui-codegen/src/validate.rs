//! A narrow slice of the ~24 static verification rules in docs/elwindui_spec.md §14 — only the
//! ones reachable by the constructs the notepad example actually uses. See
//! docs/elwindui_gui_framework_design.md §10 for the full rule list.

use crate::ast::{ChildEntry, ClosureBody, ComponentDef, ElementNode, FieldDef, FieldKind, Initializer, Item, Module, ViewExpr};
use crate::codegen::{self, strip_rc_wrapper, SymbolTable};
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
                    for f in &c.fields {
                        // Rule 18: `#[command]` field type must be `Command`.
                        if f.kind == FieldKind::Command && f.ty != "Command" {
                            errors.push(format!(
                                "{}.{}: #[command] field must have type `Command`, found `{}`",
                                c.name, f.name, f.ty
                            ));
                        }
                        if let Some(Initializer::Bind { path, .. }) = &f.initializer {
                            validate_bind_path(module, &c.name, &f.name, path, &c.fields, &table, &mut errors);
                        }
                    }

                    if let Some(base) = &c.base {
                        validate_inherits(module, c, base, modules, &table, &mut errors);
                    }

                    // `vm.field` / `vm.command.execute()` / `vm.command.can_execute` references
                    // inside this component's `view { ... }` tree, checked against whichever
                    // `#[param]` field's type names a component/viewmodel that's actually in scope
                    // (see `find_vm_fields`). Only applies if a matching `Item::View` exists in this
                    // same `modules` slice — nothing to walk otherwise.
                    if let Some(view) = modules.iter().flat_map(|m| &m.items).find_map(|item| match item {
                        Item::View(v) if v.target == c.name => Some(v),
                        _ => None,
                    }) {
                        let vm_fields =
                            find_vm_fields(module, &c.name, &c.fields, &table, &known_type_names, &mut errors);
                        for let_binding in &view.lets {
                            check_vm_references(&let_binding.element, module, &c.name, &vm_fields, &table, &mut errors);
                            check_tab_view_mode(&let_binding.element, &c.name, &mut errors);
                        }
                        check_vm_references(&view.root, module, &c.name, &vm_fields, &table, &mut errors);
                        check_tab_view_mode(&view.root, &c.name, &mut errors);
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
fn check_vm_references(
    node: &ElementNode,
    from: &Module,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    for (_, expr) in &node.attributes {
        check_vm_expr(expr, from, component_name, vm_fields, table, errors);
    }
    for child in &node.children {
        // A `ChildEntry::Ref` doesn't need its own recursive check here — the `let` binding it
        // refers to is itself already walked as one of `view.lets` in `validate`'s main loop.
        if let ChildEntry::Literal(elem) = child {
            check_vm_references(elem, from, component_name, vm_fields, table, errors);
        }
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
                    let has_field =
                        table.resolve(from, ty).is_some_and(|info| info.fields.contains_key(field.as_str()));
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
                    let is_command = table
                        .resolve(from, ty)
                        .is_some_and(|info| info.fields.get(command.as_str()) == Some(&FieldKind::Command));
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
                    let is_command = table
                        .resolve(from, ty)
                        .is_some_and(|info| info.fields.get(command.as_str()) == Some(&FieldKind::Command));
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
            ClosureBody::Expr(inner) => {
                check_closure_expr_body(inner, param, from, component_name, vm_fields, table, errors)
            }
            ClosureBody::Element(elem) => {
                check_element_value(elem, Some(param), from, component_name, vm_fields, table, errors)
            }
        },
        ViewExpr::Element(elem) => {
            check_element_value(elem, None, from, component_name, vm_fields, table, errors)
        }
    }
}

/// Checks a closure body (`header_template`/`item_template`'s `|param| ...`): a reference is
/// valid if its first segment is either the closure's own bound parameter (or its `data_context`
/// alias — nothing further to check, the parameter's type isn't a `vm_fields`-tracked
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
        // `data_context` is `emit_expr`'s sugar for the closure's own bound parameter (see
        // codegen.rs) — valid wherever `param` itself is.
        Some(first) if first == param || first == "data_context" => {}
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
        let has_items_source = node.attributes.iter().any(|(name, _)| name == "items_source");
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
        ViewExpr::Closure { body: ClosureBody::Element(elem), .. } => check_tab_view_mode(elem, component_name, errors),
        ViewExpr::TFluent(_, args) => {
            for (_, arg) in args {
                check_tab_view_mode_in_expr(arg, component_name, errors);
            }
        }
        ViewExpr::Path(_) | ViewExpr::MethodCall(..) | ViewExpr::Expr(_) | ViewExpr::Closure { body: ClosureBody::Expr(_), .. } => {}
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
            Some(param) => check_closure_expr_body(value, param, from, component_name, vm_fields, table, errors),
            None => check_vm_expr(value, from, component_name, vm_fields, table, errors),
        }
    }
    for child in &elem.children {
        if let ChildEntry::Literal(literal) = child {
            check_vm_references(literal, from, component_name, vm_fields, table, errors);
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

/// Checks `component X inherits Base { .. }` (docs/elwindui_spec.md 付録H.2): `Base` must resolve;
/// if it's the `NativeControl` marker, `X`'s structurally-inferred `is_native` (see
/// `codegen::build_symbol_table`'s `resolve_is_native`) must actually be `true` — a consistency
/// check, since `inherits` is a documentation/contract annotation here, not what *determines*
/// nativeness. Otherwise (e.g. `RoundedPanel inherits Rectangle`), `X` must have a paired `view`
/// whose root element is literally `Base` — the shape-composition use case.
fn validate_inherits(
    from: &Module,
    c: &ComponentDef,
    base: &str,
    modules: &[Module],
    table: &SymbolTable,
    errors: &mut Vec<String>,
) {
    if table.resolve(from, base).is_none() {
        errors.push(format!(
            "{}: inherits `{base}`, but `{base}` is not a known component/builtin (missing `use`?)",
            c.name
        ));
        return;
    }

    if base == "NativeControl" {
        let is_native = table.resolve(from, &c.name).is_some_and(|info| info.is_native);
        if !is_native {
            errors.push(format!(
                "{}: inherits `NativeControl`, but its `view` root isn't itself native (or no \
                 `view` exists) — `NativeControl` is only a category tag for genuinely \
                 native-backed components",
                c.name
            ));
        }
        return;
    }

    let view = modules.iter().flat_map(|m| &m.items).find_map(|item| match item {
        Item::View(v) if v.target == c.name => Some(v),
        _ => None,
    });
    match view {
        None => errors.push(format!(
            "{}: inherits `{base}`, but has no `view {}` — a component inheriting a \
             non-`NativeControl` base must have its view's root element construct `{base}`",
            c.name, c.name
        )),
        Some(v) if v.root.type_path != base => errors.push(format!(
            "{}: inherits `{base}`, so `view {}`'s root element must be `{base}`, found `{}`",
            c.name, c.name, v.root.type_path
        )),
        Some(_) => {}
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
        let modules = vec![parse_module(viewmodel_src).unwrap(), parse_module(window_src).unwrap()];
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
        let modules = vec![parse_module(viewmodel_src).unwrap(), parse_module(window_src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("no_such_field")), "errors: {errs:?}");
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
        let modules = vec![parse_module(viewmodel_src).unwrap(), parse_module(window_src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("no_such_command")), "errors: {errs:?}");
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
        let modules = vec![parse_module(viewmodel_src).unwrap(), parse_module(window_src).unwrap()];
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("no_such_command")), "errors: {errs:?}");
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
        assert!(errs.iter().any(|e| e.contains("not in scope")), "errors: {errs:?}");
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
        assert!(errs.iter().any(|e| e.contains("Nonexistent")), "errors: {errs:?}");
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
            errs.iter().any(|e| e.contains("missing required attribute") && e.contains("doc")),
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
        assert!(errs.iter().any(|e| e.contains("other_thing")), "errors: {errs:?}");
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

    /// `inherits`'s shape-composition use case (docs/elwindui_spec.md 付録H.2): a component
    /// inheriting a non-`NativeControl` base must have its `view`'s root element literally
    /// construct that base.
    #[test]
    fn accepts_component_inheriting_a_shape_primitive_with_matching_view_root() {
        let src = r#"
component RoundedPanel inherits Rectangle {
    #[param]
    fill: Option<String>,
}

view RoundedPanel {
    Rectangle { fill: fill }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap()).chain(crate::builtin_modules()).collect();
        assert_eq!(validate(&modules), Ok(()));
    }

    #[test]
    fn rejects_inherits_when_view_root_does_not_match_base() {
        let src = r#"
component RoundedPanel inherits Rectangle {
    #[param]
    fill: Option<String>,
}

view RoundedPanel {
    VerticalLayout { }
}
"#;
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap()).chain(crate::builtin_modules()).collect();
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("must be `Rectangle`")), "errors: {errs:?}");
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
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap()).chain(crate::builtin_modules()).collect();
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("not a known component/builtin")), "errors: {errs:?}");
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
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap()).chain(crate::builtin_modules()).collect();
        let errs = validate(&modules).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("NativeControl")), "errors: {errs:?}");
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
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap()).chain(crate::builtin_modules()).collect();
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
        let modules: Vec<_> = std::iter::once(parse_module(src).unwrap()).chain(crate::builtin_modules()).collect();
        let table = codegen::build_symbol_table(&modules);
        let info = table.resolve(&modules[0], "DocumentViewLike").expect("resolves");
        assert!(!info.is_native);

        let native_info = table.resolve(&modules[0], "Window").expect("resolves");
        assert!(native_info.is_native);

        let virtual_builtin_info = table.resolve(&modules[0], "VerticalLayout").expect("resolves");
        assert!(!virtual_builtin_info.is_native);
    }
}
