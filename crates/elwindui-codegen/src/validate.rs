//! A narrow slice of the ~24 static verification rules in docs/elwindui_spec.md §14 — only the
//! ones reachable by the constructs the notepad example actually uses. See
//! docs/elwindui_gui_framework_design.md §10 for the full rule list.

use crate::ast::{ElementNode, FieldDef, FieldKind, Initializer, Item, Module, ViewExpr};
use crate::codegen::{self, SymbolTable};
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
                        check_vm_references(&view.root, module, &c.name, &vm_fields, &table, &mut errors);
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
        if !known_type_names.contains(f.ty.as_str()) {
            continue;
        }
        if table.resolve(from, &f.ty).is_some() {
            vm_fields.insert(f.name.as_str(), f.ty.as_str());
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
        check_vm_references(child, from, component_name, vm_fields, table, errors);
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

    match table.resolve(from, &root_field.ty) {
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
}
