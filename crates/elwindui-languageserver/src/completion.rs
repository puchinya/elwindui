//! `textDocument/completion` for `vm.field` / `vm.command.execute()` / `vm.command.can_execute`
//! (付録O.4) — the same shapes `elwindui_codegen::validate::check_vm_expr` already understands.
//!
//! `ast.rs` has no span info (see `diagnostics.rs`'s doc comment), so this doesn't know which
//! element the cursor is structurally inside. Instead it takes every `#[param]` field of every
//! `component` in the current file whose type resolves via `codegen::SymbolTable::resolve` (locally
//! defined or brought into scope by a `use`, §12) as a completion-worthy "vm-like" name — accurate
//! enough for the current codebase's one-`component`-per-file convention, and a false positive here
//! only means an unrelated field name shows up in the candidate list, never a wrong resolution.
//!
//! Only two dotted-path depths are supported, matching what `check_vm_expr` validates:
//! `vm.|` (complete `vm`'s fields/commands) and `vm.save.|` (complete `execute`/`can_execute`, only
//! if `save` is a `#[command]` field). Deeper paths and recursing into a field's own type (e.g. a
//! nested viewmodel) are out of scope — `TypeInfo` doesn't carry per-field type names, only kinds.

use elwindui_codegen::ast::{FieldKind, Item, Module};
use elwindui_codegen::codegen;
use lsp_types::{CompletionItem, CompletionItemKind, Position};
use std::collections::HashMap;
use std::path::Path;

/// A placeholder identifier `vm.`/`vm.sa`/`vm.save.` gets replaced with before parsing (see
/// `completions_at`). Long and DSL-namespaced enough that it will never collide with a real
/// user identifier by accident.
const PLACEHOLDER_IDENT: &str = "__elwindui_completion_placeholder";

pub fn completions_at(
    dir: impl AsRef<Path>,
    current_file: impl AsRef<Path>,
    src: &str,
    position: Position,
) -> Vec<CompletionItem> {
    let current_file = current_file.as_ref();
    let Some(offset) = utf16_position_to_byte_offset(src, position) else {
        return Vec::new();
    };
    let Some((chain_start, owner_path, filter)) = preceding_dotted_path(src, offset) else {
        return Vec::new();
    };

    // The text right at the cursor (`vm.`, `vm.sa`, `vm.save.`, ...) is, by construction, an
    // incomplete expression — `parser.rs`'s `parse_view_expr` requires an identifier after every
    // `.`, so the file as typed will *not* parse. Swap the in-progress chain for a placeholder
    // identifier before parsing, so the rest of the file's structure (the `component`'s field
    // declarations, its `use`s — everything completion actually needs) still comes through; only
    // the dotted-path info already extracted above (`owner_path`/`filter`) is used for resolution.
    let mut patched = String::with_capacity(src.len());
    patched.push_str(&src[..chain_start]);
    patched.push_str(PLACEHOLDER_IDENT);
    patched.push_str(&src[offset..]);
    let Ok(current_module) = elwindui_codegen::parser::parse_module(&patched) else {
        return Vec::new();
    };

    let mut modules: Vec<Module> = crate::diagnostics::parse_dir_modules(dir)
        .into_iter()
        .filter(|(path, _)| path != current_file)
        .map(|(_, m)| m)
        .collect();
    modules.push(current_module);
    let current_module = modules.last().expect("just pushed");
    let table = codegen::build_symbol_table(&modules);

    // Every `#[param]` field, across every `component` in this file, whose type actually resolves
    // from this module's scope — the same idea as `elwindui_codegen::validate::find_vm_fields`.
    let vm_fields: HashMap<&str, &str> = current_module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Component(c) => Some(c),
            _ => None,
        })
        .flat_map(|c| &c.fields)
        .filter_map(|f| {
            table
                .resolve(current_module, &f.ty)
                .map(|_| (f.name.as_str(), f.ty.as_str()))
        })
        .collect();

    match owner_path.as_slice() {
        [vm_name] => {
            let Some(&ty) = vm_fields.get(vm_name.as_str()) else {
                return Vec::new();
            };
            let Some(info) = table.resolve(current_module, ty) else {
                return Vec::new();
            };
            info.fields
                .iter()
                .filter(|(name, _)| name.starts_with(filter.as_str()))
                .map(|(name, kind)| field_completion_item(name, *kind))
                .collect()
        }
        [vm_name, command_name] => {
            let Some(&ty) = vm_fields.get(vm_name.as_str()) else {
                return Vec::new();
            };
            let Some(info) = table.resolve(current_module, ty) else {
                return Vec::new();
            };
            if info.fields.get(command_name.as_str()) != Some(&FieldKind::Command) {
                return Vec::new();
            }
            ["execute", "can_execute"]
                .into_iter()
                .filter(|name| name.starts_with(filter.as_str()))
                .map(command_member_completion_item)
                .collect()
        }
        _ => Vec::new(),
    }
}

fn field_completion_item(name: &str, kind: FieldKind) -> CompletionItem {
    let item_kind = match kind {
        FieldKind::Command => CompletionItemKind::METHOD,
        FieldKind::Computed | FieldKind::Attached => CompletionItemKind::PROPERTY,
        FieldKind::Observable | FieldKind::Prop | FieldKind::Param => CompletionItemKind::FIELD,
    };
    CompletionItem {
        label: name.to_string(),
        kind: Some(item_kind),
        ..Default::default()
    }
}

fn command_member_completion_item(name: &str) -> CompletionItem {
    let (kind, insert_text) = if name == "execute" {
        (CompletionItemKind::METHOD, Some("execute()".to_string()))
    } else {
        (CompletionItemKind::PROPERTY, None)
    };
    CompletionItem {
        label: name.to_string(),
        kind: Some(kind),
        insert_text,
        ..Default::default()
    }
}

/// LSP `Position` (0-based line, UTF-16 code-unit character) -> byte offset into `src`, matching
/// `semantic_tokens.rs::Scanner`'s UTF-16 column tracking (`char::len_utf16`) so the two stay
/// consistent with how the client counts columns.
fn utf16_position_to_byte_offset(src: &str, pos: Position) -> Option<usize> {
    let mut line = 0u32;
    let mut col = 0u32;
    for (byte_idx, ch) in src.char_indices() {
        if line == pos.line && col == pos.character {
            return Some(byte_idx);
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u32;
        }
    }
    (line == pos.line && col == pos.character).then_some(src.len())
}

/// From `src[..offset]` (the text immediately before the cursor), extracts the dotted identifier
/// chain being typed — e.g. `"...vm."` -> `(start, ["vm"], "")`, `"...vm.sa"` -> `(start, ["vm"],
/// "sa")`, `"...vm.save."` -> `(start, ["vm", "save"], "")` — as (byte offset the chain starts at,
/// owner path, filter prefix for the last, possibly-partial segment). `None` if there's no dotted
/// chain at all right before the cursor (nothing to offer member completions for).
fn preceding_dotted_path(src: &str, offset: usize) -> Option<(usize, Vec<String>, String)> {
    let prefix = src.get(..offset)?;
    let start = prefix
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .map(|i| {
            i + prefix[i..]
                .chars()
                .next()
                .expect("rfind match is a valid char boundary")
                .len_utf8()
        })
        .unwrap_or(0);
    let chain = &prefix[start..];
    if chain.is_empty() {
        return None;
    }

    let mut segments: Vec<String> = chain.split('.').map(str::to_string).collect();
    if segments.len() < 2 {
        // No `.` typed yet — nothing to complete an owner's members against.
        return None;
    }
    let filter = segments
        .pop()
        .expect("split always yields at least one segment");
    Some((start, segments, filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_dir(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "elwindui_lsp_completion_test_{}_{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).unwrap();
        }
        dir
    }

    const VM_SRC: &str = r#"
viewmodel Vm {
    #[observable]
    content: String = String::new(),

    #[command(can_execute: true)]
    save: Command = command!(|| {}),
}
"#;

    fn window_src(body_after_vm_dot: &str) -> String {
        format!(
            r#"
component Window {{
    #[param]
    #[inject]
    vm: Vm,
}}
view Window {{ Window {{ Text {{ text: {body_after_vm_dot} }} }} }}
"#
        )
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        let mut v: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        v.sort();
        v
    }

    #[test]
    fn completes_vm_fields_and_commands_after_vm_dot() {
        let src = window_src("vm.");
        let dir = write_dir(&[("vm.elwind", VM_SRC), ("window.elwind", &src)]);
        let window_path = dir.join("window.elwind");
        let dot_offset = src.find("vm.").unwrap() + "vm.".len();
        let position = byte_offset_to_position(&src, dot_offset);

        let items = completions_at(&dir, &window_path, &src, position);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(labels(&items), vec!["content", "save"]);
    }

    #[test]
    fn filters_by_partial_input() {
        let src = window_src("vm.sa");
        let dir = write_dir(&[("vm.elwind", VM_SRC), ("window.elwind", &src)]);
        let window_path = dir.join("window.elwind");
        let offset = src.find("vm.sa").unwrap() + "vm.sa".len();
        let position = byte_offset_to_position(&src, offset);

        let items = completions_at(&dir, &window_path, &src, position);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(labels(&items), vec!["save"]);
    }

    #[test]
    fn completes_execute_and_can_execute_after_a_command_field() {
        let src = window_src("vm.save.");
        let dir = write_dir(&[("vm.elwind", VM_SRC), ("window.elwind", &src)]);
        let window_path = dir.join("window.elwind");
        let offset = src.find("vm.save.").unwrap() + "vm.save.".len();
        let position = byte_offset_to_position(&src, offset);

        let items = completions_at(&dir, &window_path, &src, position);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(labels(&items), vec!["can_execute", "execute"]);
    }

    #[test]
    fn no_completions_after_a_non_command_field() {
        let src = window_src("vm.content.");
        let dir = write_dir(&[("vm.elwind", VM_SRC), ("window.elwind", &src)]);
        let window_path = dir.join("window.elwind");
        let offset = src.find("vm.content.").unwrap() + "vm.content.".len();
        let position = byte_offset_to_position(&src, offset);

        let items = completions_at(&dir, &window_path, &src, position);
        std::fs::remove_dir_all(&dir).ok();

        assert!(items.is_empty());
    }

    #[test]
    fn no_completions_when_the_vm_type_does_not_resolve() {
        // `vm`'s declared type `NoSuchType` isn't defined anywhere in the compilation unit, so
        // `SymbolTable::resolve` can't find it — `vm` never makes it into `vm_fields`, and no
        // completions should be offered.
        let src = r#"
component Window {
    #[param]
    #[inject]
    vm: NoSuchType,
}
view Window { Window { Text { text: vm. } } }
"#;
        let dir = write_dir(&[("window.elwind", src)]);
        let window_path = dir.join("window.elwind");
        let offset = src.find("vm.").unwrap() + "vm.".len();
        let position = byte_offset_to_position(src, offset);

        let items = completions_at(&dir, &window_path, src, position);
        std::fs::remove_dir_all(&dir).ok();

        assert!(items.is_empty());
    }

    fn byte_offset_to_position(src: &str, offset: usize) -> Position {
        let mut line = 0u32;
        let mut col = 0u32;
        for (idx, ch) in src.char_indices() {
            if idx == offset {
                return Position {
                    line,
                    character: col,
                };
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += ch.len_utf16() as u32;
            }
        }
        Position {
            line,
            character: col,
        }
    }

    #[test]
    fn utf16_position_to_byte_offset_handles_multibyte_lines() {
        let src = "あvm.\n";
        // "あ" is 1 UTF-16 unit, 3 UTF-8 bytes; "vm." starts right after it.
        let offset = utf16_position_to_byte_offset(
            src,
            Position {
                line: 0,
                character: 4,
            },
        );
        assert_eq!(offset, Some("あvm.".len()));
    }
}
