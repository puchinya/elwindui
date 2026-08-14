//! `textDocument/completion` for `vm.field` — the same shape `elwindui_codegen::validate::check_vm_expr`
//! already understands. An action (e.g. `vm.save`) completes the same way as any other field —
//! there's no separate `.execute()`/`.can_execute` member form to drill into (actions can't even
//! be declared from the DSL text form's `viewmodel` at all; only the Rust-native
//! `#[elwindui::viewmodel]` frontend supports them, see `elwindui_codegen::attr_frontend`).
//!
//! `ast.rs` has no span info (see `diagnostics.rs`'s doc comment), so this doesn't know which
//! element the cursor is structurally inside. Instead it takes every `#[param]` field of every
//! `#[elwindui::component]` struct in the current file whose type resolves via
//! `codegen::SymbolTable::resolve` (declared earlier in the same file, §12) as a completion-worthy
//! "vm-like" name — accurate enough for the current codebase's one-`component`-per-file convention,
//! and a false positive here only means an unrelated field name shows up in the candidate list,
//! never a wrong resolution.
//!
//! Only one dotted-path depth is supported, matching what `check_vm_expr` validates: `vm.|`
//! (complete `vm`'s fields). Deeper paths and recursing into a field's own type (e.g. a nested
//! viewmodel) are out of scope — `TypeInfo` doesn't carry per-field type names, only kinds.

use elwindui_codegen::ast::{FieldKind, Item, Module};
use elwindui_codegen::codegen;
use lsp_types::{CompletionItem, CompletionItemKind, Position};
use std::collections::HashMap;

/// A placeholder identifier `vm.`/`vm.sa`/`vm.save.` gets replaced with before parsing (see
/// `completions_at`). Long and DSL-namespaced enough that it will never collide with a real
/// user identifier by accident.
const PLACEHOLDER_IDENT: &str = "__elwindui_completion_placeholder";

pub fn completions_at(src: &str, position: Position) -> Vec<CompletionItem> {
    let Some(offset) = utf16_position_to_byte_offset(src, position) else {
        return Vec::new();
    };
    let Some((chain_start, owner_path, filter)) = preceding_dotted_path(src, offset) else {
        return Vec::new();
    };

    // The text right at the cursor (`vm.`, `vm.sa`, `vm.save.`, ...) is, by construction, an
    // incomplete expression — a `view! { .. }` body's own DSL-text parsing requires an identifier
    // after every `.`, so the file as typed will *not* parse. Swap the in-progress chain for a
    // placeholder identifier before parsing, so the rest of the file's structure (the component's
    // field declarations, its sibling items — everything completion actually needs) still comes
    // through; only the dotted-path info already extracted above (`owner_path`/`filter`) is used
    // for resolution. Operates on the raw file text (not just the `view!` macro body), so no
    // span-mapping through `syn::Macro::tokens` is needed — the same trick the old DSL-text
    // version of this function used, just at whole-file granularity instead of whole-file
    // granularity.
    let mut patched = String::with_capacity(src.len());
    patched.push_str(&src[..chain_start]);
    patched.push_str(PLACEHOLDER_IDENT);
    patched.push_str(&src[offset..]);
    let Ok(file) = syn::parse_file(&patched) else {
        return Vec::new();
    };
    let Ok(modules) = elwindui_codegen::component_frontend::modules_from_file(&file) else {
        return Vec::new();
    };
    let table = codegen::build_symbol_table(&modules);

    // Every `#[param]` field, across every `#[elwindui::component]` struct in this file, whose type
    // actually resolves from its own module's scope — the same idea as
    // `elwindui_codegen::validate::find_vm_fields`.
    let vm_fields: HashMap<&str, &str> = modules
        .iter()
        .flat_map(|m| components_in(m).map(move |c| (m, c)))
        .flat_map(|(m, c)| c.fields.iter().map(move |f| (m, f)))
        .filter_map(|(m, f)| {
            table
                .resolve(m, &f.ty)
                .map(|_| (f.name.as_str(), f.ty.as_str()))
        })
        .collect();

    match owner_path.as_slice() {
        [vm_name] => {
            let Some(&ty) = vm_fields.get(vm_name.as_str()) else {
                return Vec::new();
            };
            // Any module in scope can resolve `ty` — they all share the same flat, crate-root-like
            // symbol table (`build_symbol_table` doesn't distinguish which module a lookup "comes
            // from" beyond `use` resolution, and none of these modules declare any `use`s).
            let Some(module) = modules.first() else {
                return Vec::new();
            };
            let Some(info) = table.resolve(module, ty) else {
                return Vec::new();
            };
            info.fields
                .iter()
                .filter(|(name, _)| name.starts_with(filter.as_str()))
                .map(|(name, kind)| field_completion_item(name, *kind))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn components_in(module: &Module) -> impl Iterator<Item = &elwindui_codegen::ast::ComponentDef> {
    module.items.iter().filter_map(|item| match item {
        Item::Component(c) => Some(c),
        _ => None,
    })
}

fn field_completion_item(name: &str, kind: FieldKind) -> CompletionItem {
    let item_kind = match kind {
        FieldKind::Action => CompletionItemKind::METHOD,
        FieldKind::Computed
        | FieldKind::AsyncComputed
        | FieldKind::Attached
        | FieldKind::Environment => CompletionItemKind::PROPERTY,
        FieldKind::Observable | FieldKind::Prop | FieldKind::Param | FieldKind::State => {
            CompletionItemKind::FIELD
        }
    };
    CompletionItem {
        label: name.to_string(),
        kind: Some(item_kind),
        ..Default::default()
    }
}

/// LSP `Position` (0-based line, UTF-16 code-unit character) -> byte offset into `src`, matching
/// how the client counts columns.
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

    const VM_SRC: &str = r#"
        #[elwindui::viewmodel]
        mod vm_mod {
            struct Vm {
                #[observable(default = String::new())]
                content: String,

                #[computed(expr = true)]
                save_can_execute: bool,
            }
        }
    "#;

    fn window_src(body_after_vm_dot: &str) -> String {
        format!(
            r#"
            #[elwindui::component(inherits Window)]
            struct WindowC {{
                #[param]
                #[inject]
                vm: Vm,
                body: view! {{ TextArea {{ text: {body_after_vm_dot} }} }},
            }}
            "#
        )
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        let mut v: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        v.sort();
        v
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
    fn completes_vm_fields_after_vm_dot() {
        let src = format!("{VM_SRC}\n{}", window_src("vm."));
        let dot_offset = src.rfind("vm.").unwrap() + "vm.".len();
        let position = byte_offset_to_position(&src, dot_offset);

        let items = completions_at(&src, position);

        assert_eq!(labels(&items), vec!["content", "save_can_execute"]);
    }

    #[test]
    fn filters_by_partial_input() {
        let src = format!("{VM_SRC}\n{}", window_src("vm.sa"));
        let offset = src.rfind("vm.sa").unwrap() + "vm.sa".len();
        let position = byte_offset_to_position(&src, offset);

        let items = completions_at(&src, position);

        assert_eq!(labels(&items), vec!["save_can_execute"]);
    }

    #[test]
    fn no_completions_after_a_field_dot() {
        // No 2-level drilling of any kind anymore (actions resolve exactly like any other field,
        // with no `.execute()`/`.can_execute` member form to complete).
        let src = format!("{VM_SRC}\n{}", window_src("vm.content."));
        let offset = src.rfind("vm.content.").unwrap() + "vm.content.".len();
        let position = byte_offset_to_position(&src, offset);

        let items = completions_at(&src, position);

        assert!(items.is_empty());
    }

    #[test]
    fn no_completions_when_the_vm_type_does_not_resolve() {
        // `vm`'s declared type `NoSuchType` isn't defined anywhere in the file, so
        // `SymbolTable::resolve` can't find it — `vm` never makes it into `vm_fields`, and no
        // completions should be offered.
        let src = r#"
            #[elwindui::component(inherits Window)]
            struct WindowD {
                #[param]
                #[inject]
                vm: NoSuchType,
                body: view! { TextArea { text: vm. } },
            }
        "#;
        let offset = src.find("vm. ").unwrap() + "vm.".len();
        let position = byte_offset_to_position(src, offset);

        let items = completions_at(src, position);

        assert!(items.is_empty());
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
