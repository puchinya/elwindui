//! A narrow slice of the ~24 static verification rules in docs/elwindui_spec.md §14 — only the
//! ones reachable by the constructs the notepad example actually uses. See
//! docs/elwindui_gui_framework_design.md §10 for the full rule list.

use crate::ast::{FieldKind, Initializer, Item, Module};
use std::collections::HashMap;

pub fn validate(modules: &[Module]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // Field tables for every `component`/`viewmodel`, keyed by type name, so `bind!(vm.content,
    // ...)` can be checked against `NotepadViewModel`'s fields even though they live in a
    // different `.elwind` file (符14 rule 12/13's `store` is generalized here to `viewmodel`).
    let mut field_tables: HashMap<&str, Vec<&str>> = HashMap::new();
    for module in modules {
        for item in &module.items {
            let (name, fields) = match item {
                Item::Component(c) => (c.name.as_str(), &c.fields),
                Item::ViewModel(v) => (v.name.as_str(), &v.fields),
                Item::Enum(_) | Item::View(_) => continue,
            };
            let entry = field_tables.entry(name).or_default();
            entry.extend(fields.iter().map(|f| f.name.as_str()));
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
}
