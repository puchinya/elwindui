//! A narrow slice of the ~24 static verification rules in docs/elwindui_spec.md §14 — only the
//! ones reachable by the constructs the notepad example actually uses. See
//! docs/elwindui_gui_framework_design.md §10 for the full rule list.

use crate::ast::{ElementNode, FieldKind, Initializer, Item, Module, ViewExpr};
use std::collections::HashMap;

pub fn validate(modules: &[Module]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // Field tables for every `component`/`viewmodel`, keyed by type name, so `bind!(vm.content,
    // ...)` can be checked against `NotepadViewModel`'s fields even though they live in a
    // different `.elwind` file (符14 rule 12/13's `store` is generalized here to `viewmodel`) —
    // or even outside `.elwind` entirely, when the caller is `compile_dir_with_extra_viewmodels`
    // and `modules` includes synthetic `Module`s built from `#[elwindui::viewmodel]`-annotated
    // Rust source (see `attr_frontend::viewmodel_defs_from_rs_file`). `command_fields` is the same
    // idea restricted to `#[command]`-kind fields, for checking `vm.command.execute()`/
    // `vm.command.can_execute` specifically (a name that's a plain field but not a command
    // shouldn't pass a `.execute()`/`.can_execute` check).
    let mut field_tables: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut command_fields: HashMap<&str, Vec<&str>> = HashMap::new();
    for module in modules {
        for item in &module.items {
            let (name, fields) = match item {
                Item::Component(c) => (c.name.as_str(), &c.fields),
                Item::ViewModel(v) => (v.name.as_str(), &v.fields),
                Item::Enum(_) | Item::View(_) => continue,
            };
            field_tables.entry(name).or_default().extend(fields.iter().map(|f| f.name.as_str()));
            command_fields.entry(name).or_default().extend(
                fields.iter().filter(|f| f.kind == FieldKind::Command).map(|f| f.name.as_str()),
            );
        }
    }

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
                            validate_bind_path(&c.name, &f.name, path, &c.fields, &field_tables, &mut errors);
                        }
                    }

                    // `vm.field` / `vm.command.execute()` / `vm.command.can_execute` references
                    // inside this component's `view { ... }` tree, checked against whichever
                    // `#[param]` field's type names a known component/viewmodel (see
                    // `find_vm_fields`). Only applies if a matching `Item::View` exists in this
                    // same `modules` slice — nothing to walk otherwise.
                    if let Some(view) = modules.iter().flat_map(|m| &m.items).find_map(|item| match item {
                        Item::View(v) if v.target == c.name => Some(v),
                        _ => None,
                    }) {
                        let vm_fields = find_vm_fields(&c.fields, &field_tables);
                        check_vm_references(&view.root, &c.name, &vm_fields, &field_tables, &command_fields, &mut errors);
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

/// A component's `#[param]` fields whose type names a known `component`/`viewmodel` — candidates
/// for the `vm` in `vm.field`/`vm.command.execute()` (there's no `#[param]`/injection marker left
/// on `FieldDef` by the time it reaches here, so "known type" is the signal used instead; a plain
/// `String`/`i32`/etc. field never matches since those never appear as `field_tables` keys).
fn find_vm_fields<'a>(
    fields: &'a [crate::ast::FieldDef],
    field_tables: &HashMap<&str, Vec<&str>>,
) -> HashMap<&'a str, &'a str> {
    fields
        .iter()
        .filter(|f| field_tables.contains_key(f.ty.as_str()))
        .map(|f| (f.name.as_str(), f.ty.as_str()))
        .collect()
}

/// Walks a `view { ... }` element tree checking every attribute expression's `vm.xxx` references
/// (see `check_vm_expr`) against the known field/command tables, recursing into children.
fn check_vm_references(
    node: &ElementNode,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    field_tables: &HashMap<&str, Vec<&str>>,
    command_fields: &HashMap<&str, Vec<&str>>,
    errors: &mut Vec<String>,
) {
    for (_, expr) in &node.attributes {
        check_vm_expr(expr, component_name, vm_fields, field_tables, command_fields, errors);
    }
    for child in &node.children {
        check_vm_references(child, component_name, vm_fields, field_tables, command_fields, errors);
    }
}

fn check_vm_expr(
    expr: &ViewExpr,
    component_name: &str,
    vm_fields: &HashMap<&str, &str>,
    field_tables: &HashMap<&str, Vec<&str>>,
    command_fields: &HashMap<&str, Vec<&str>>,
    errors: &mut Vec<String>,
) {
    match expr {
        ViewExpr::Path(path) => match path.as_slice() {
            [vm_name, field] => {
                if let Some(&ty) = vm_fields.get(vm_name.as_str()) {
                    if !field_tables.get(ty).is_some_and(|fs| fs.contains(&field.as_str())) {
                        errors.push(format!(
                            "{component_name}: `{vm_name}.{field}` — `{ty}` has no field `{field}`"
                        ));
                    }
                }
            }
            // `vm.command.can_execute` (付録O.4's 3-segment special form).
            [vm_name, command, suffix] if suffix == "can_execute" => {
                if let Some(&ty) = vm_fields.get(vm_name.as_str()) {
                    if !command_fields.get(ty).is_some_and(|cs| cs.contains(&command.as_str())) {
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
                    if !command_fields.get(ty).is_some_and(|cs| cs.contains(&command.as_str())) {
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
                check_vm_expr(arg, component_name, vm_fields, field_tables, command_fields, errors);
            }
        }
    }
}

/// Checks `bind!(vm.content, ...)`: `vm` must be a field of the enclosing `component` whose type
/// names a known `component`/`viewmodel`, and `content` must be one of that type's fields.
/// Generalizes rules 12/13 (written against `store` in the spec) to any bindable owner.
fn validate_bind_path(
    owner_name: &str,
    field_name: &str,
    path: &[String],
    own_fields: &[crate::ast::FieldDef],
    field_tables: &HashMap<&str, Vec<&str>>,
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

    match field_tables.get(root_field.ty.as_str()) {
        Some(fields) if fields.contains(&target_field.as_str()) => {}
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
view Window3 { Window { Text { text: vm.no_such_field } } }
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
}
