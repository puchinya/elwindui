//! Adapter between `elwindui_codegen::{component_frontend, validate}` and the LSP's `Diagnostic`
//! type — kept separate from the protocol plumbing (`lib.rs`) so it's testable without a real
//! `lsp_server::Connection`. See docs/design/tools/languageserver_design.md
//!
//! Operates on a single `.rs` file's source text (Phase 7, `docs/status/implementation_status.md`)
//! — the successor to the old directory-based model, retired along with DSL text compilation
//! itself. Each `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` item in
//! the file becomes its own `Module` (`component_frontend::modules_from_file`, the same conversion
//! real macro expansion uses), validated together. There is no cross-file resolution the way the
//! old directory scan had: the real macro-expansion path only ever sees an *earlier-declared*
//! same-crate sibling via its own process-global registry (`component_frontend::
//! same_crate_components`'s own doc comment), which is populated by real compilation this
//! language server never runs — restricting to one file's own top-level items is the accurate
//! reflection of that, not a regression `parse_dir_modules`'s removal introduces.

use elwindui_codegen::ast::Module;
use elwindui_codegen::component_frontend;
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Parses `src` as a whole Rust file and builds one `Module` per `#[elwindui::component]`/
/// `#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` item found in it — `None` if `src` doesn't even
/// parse as valid Rust syntax (a `syn::parse_file` failure; e.g. mid-edit). Shared by
/// `diagnostics_for_source` (which additionally needs the parse error itself, with real span info)
/// and `completion.rs` (which just wants the `Module`s).
pub fn modules_for_source(src: &str) -> Option<Vec<Module>> {
    let file = syn::parse_file(src).ok()?;
    component_frontend::modules_from_file(&file).ok()
}

/// Parses and validates `src` (one `.rs` file's text), returning every diagnostic found — empty if
/// clean, so a caller can always publish the result and have stale diagnostics cleared once fixed.
///
/// Position precision: a `syn::parse_file` failure carries the parser's own real line/column
/// (`proc_macro2`'s span-locations tracking) — strictly better than the DSL text form
/// parser's own hand-rolled line counting. A `component_frontend::modules_from_file` conversion
/// failure (a malformed `view!` body, a non-unit `#[elwindui::dsl_enum]` variant, ...) and every
/// `validate::validate` error stay at line 0, column 0 — neither carries span info through the AST
/// (`ast.rs`'s `FieldDef`/`ElementNode` have no source-location fields), matching the precision the
/// text-frontend-era `validate_error_diagnostic` already had. Precise positions need span-tracking
/// threaded through `ast.rs`/`parser.rs`/`component_frontend.rs` — a separate follow-up, not
/// attempted here.
pub fn diagnostics_for_source(src: &str) -> Vec<Diagnostic> {
    let file = match syn::parse_file(src) {
        Ok(file) => file,
        Err(e) => return vec![syn_error_diagnostic(&e)],
    };

    let modules = match component_frontend::modules_from_file(&file) {
        Ok(modules) => modules,
        Err(message) => return vec![point_diagnostic(&message)],
    };

    if modules.is_empty() {
        // No `#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]` item in this
        // file at all — nothing for this server to check (an ordinary Rust file, or one that
        // doesn't use any elwindui macros).
        return Vec::new();
    }

    match elwindui_codegen::validate::validate(&modules) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.iter().map(|m| point_diagnostic(m)).collect(),
    }
}

fn syn_error_diagnostic(e: &syn::Error) -> Diagnostic {
    let start = e.span().start();
    // `proc_macro2::LineColumn` is 1-indexed for `line`, 0-indexed for `column` already — LSP
    // wants both 0-indexed.
    let line = (start.line as u32).saturating_sub(1);
    let character = start.column as u32;
    Diagnostic {
        range: Range {
            start: Position { line, character },
            end: Position { line, character },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("elwindui".to_string()),
        message: e.to_string(),
        ..Default::default()
    }
}

fn point_diagnostic(message: &str) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position { line: 0, character: 0 },
            end: Position { line: 0, character: 0 },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("elwindui".to_string()),
        message: message.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_file_has_no_diagnostics() {
        let src = r#"
            #[elwindui::viewmodel]
            mod vm_mod {
                struct Vm {
                    #[observable(default = String::new())]
                    content: String,
                }
            }

            #[elwindui::component(inherits Window)]
            struct Window1 {
                #[param]
                #[inject]
                vm: Vm,
                body: view! {
                    TextArea { text: vm.content }
                },
            }
        "#;
        let diags = diagnostics_for_source(src);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn file_with_no_elwindui_items_has_no_diagnostics() {
        let src = "fn main() {}";
        assert!(diagnostics_for_source(src).is_empty());
    }

    #[test]
    fn syntax_error_is_reported_with_a_real_position() {
        let src = "struct Broken {\n    field:\n";
        let diags = diagnostics_for_source(src);
        assert!(!diags.is_empty(), "expected a syntax-error diagnostic");
        // The unclosed struct starts failing on/after line 0 (1-indexed line 1) — just confirm a
        // real (non-default) position was produced, not the exact line.
        assert!(diags[0].range.start.line > 0 || diags[0].range.start.character > 0);
    }

    #[test]
    fn vm_reference_error_is_reported() {
        let src = r#"
            #[elwindui::viewmodel]
            mod vm_mod2 {
                struct Vm2 {
                    #[observable(default = String::new())]
                    content: String,
                }
            }

            #[elwindui::component(inherits Window)]
            struct Window2 {
                #[param]
                #[inject]
                vm: Vm2,
                body: view! {
                    TextArea { text: vm.no_such_field }
                },
            }
        "#;
        let diags = diagnostics_for_source(src);
        assert!(
            diags.iter().any(|d| d.message.contains("no_such_field")),
            "expected a diagnostic mentioning the bad reference, got: {diags:?}"
        );
    }
}
