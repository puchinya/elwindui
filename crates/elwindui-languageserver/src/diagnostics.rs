//! Adapter between `elwindui_codegen::{parser, validate}` and the LSP's `Diagnostic` type — kept
//! separate from the protocol plumbing (`lib.rs`) so it's testable without a real
//! `lsp_server::Connection`. See docs/elwindui_tool_languageserver_design.md §3.1.

use elwindui_codegen::ast::Module;
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Lists every `.elwind` file directly inside `dir` (sorted for deterministic output) and reads
/// each one's source text. Shared by `diagnostics_for_dir` (which additionally needs per-file parse
/// errors) and `parse_dir_modules` (which just wants the parsed `Module`s, e.g. for `completion.rs`).
fn elwind_file_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "elwind"))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();

    entries
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            (path, text)
        })
        .collect()
}

/// Parses every `.elwind` file in `dir` — the same unit `elwindui_codegen::compile_dir` processes,
/// since cross-file `bind!`/`vm.field`/`use` references need every file in the directory visible at
/// once for `codegen::build_symbol_table` to resolve them. Files that fail to parse are silently
/// skipped (best-effort: a syntax error in one sibling file shouldn't block completion/lookups in
/// another) — callers that need to report parse errors themselves (`diagnostics_for_dir`) don't use
/// this and parse each file themselves instead.
pub fn parse_dir_modules(dir: impl AsRef<Path>) -> Vec<(PathBuf, Module)> {
    elwind_file_sources(dir.as_ref())
        .into_iter()
        .filter_map(|(path, text)| {
            elwindui_codegen::parser::parse_module(&text)
                .ok()
                .map(|module| (path, module))
        })
        .collect()
}

/// Parses and validates every `.elwind` file in `dir` — the same unit `elwindui_codegen::
/// compile_dir` processes, since cross-file `bind!`/`vm.field` references need every file in the
/// directory visible at once — and returns each file's diagnostics (files with no problems still
/// get an entry, mapped to an empty `Vec`, so a caller can clear stale diagnostics on re-check).
///
/// Diagnostic positions are approximate in this first pass: parse errors carry a real line number
/// (`parser.rs`'s own error messages already include one), but `validate::validate`'s errors are
/// bare, file-agnostic strings — there's no span info in the AST yet (`ast.rs`'s `FieldDef`/
/// `ElementNode` don't record source locations at all), so a validate error is heuristically
/// attributed to whichever file's text contains the leading identifier from its own message, and
/// always placed at line 0, column 0. Precise positions need span-tracking threaded through
/// `ast.rs`/`parser.rs` — tracked as a separate follow-up, not attempted here.
pub fn diagnostics_for_dir(dir: impl AsRef<Path>) -> HashMap<PathBuf, Vec<Diagnostic>> {
    let dir = dir.as_ref();
    let mut out: HashMap<PathBuf, Vec<Diagnostic>> = HashMap::new();

    let sources = elwind_file_sources(dir);
    for (path, _) in &sources {
        out.entry(path.clone()).or_default();
    }

    let mut modules = Vec::new();
    let mut had_parse_error = false;
    for (path, text) in &sources {
        match elwindui_codegen::parser::parse_module(text) {
            Ok(module) => modules.push(module),
            Err(e) => {
                had_parse_error = true;
                out.entry(path.clone())
                    .or_default()
                    .push(parse_error_diagnostic(&e));
            }
        }
    }

    if had_parse_error {
        // `validate` needs a complete, well-formed `Module` list to build its symbol table
        // meaningfully — skip cross-file validation until every file at least parses.
        return out;
    }

    if let Err(errors) = elwindui_codegen::validate::validate(&modules) {
        for message in errors {
            let path = best_file_for_message(&sources, &message);
            out.entry(path)
                .or_default()
                .push(validate_error_diagnostic(&message));
        }
    }

    out
}

fn parse_error_diagnostic(message: &str) -> Diagnostic {
    // parser.rs's `err()` formats as "parse error at line N: msg" (1-indexed; LSP lines are 0-indexed).
    let line = message
        .strip_prefix("parse error at line ")
        .and_then(|rest| rest.split(':').next())
        .and_then(|n| n.trim().parse::<u32>().ok())
        .map(|n| n.saturating_sub(1))
        .unwrap_or(0);
    Diagnostic {
        range: point(line),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("elwindui".to_string()),
        message: message.to_string(),
        ..Default::default()
    }
}

fn validate_error_diagnostic(message: &str) -> Diagnostic {
    Diagnostic {
        range: point(0),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("elwindui".to_string()),
        message: message.to_string(),
        ..Default::default()
    }
}

fn point(line: u32) -> Range {
    Range {
        start: Position { line, character: 0 },
        end: Position { line, character: 0 },
    }
}

/// `validate::validate`'s errors are file-agnostic bare strings (e.g. "NotepadWindow: `vm.saev...`
/// ..."); this guesses which file to attribute one to by checking which file's raw source text
/// contains the message's leading identifier. Falls back to the first file if nothing matches —
/// good enough for "there's a problem somewhere in this directory", not precise attribution.
fn best_file_for_message(sources: &[(PathBuf, String)], message: &str) -> PathBuf {
    let leading_ident: String = message
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if !leading_ident.is_empty() {
        for (path, text) in sources {
            if text.contains(&leading_ident) {
                return path.clone();
            }
        }
    }
    sources.first().map(|(p, _)| p.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_dir(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "elwindui_lsp_diag_test_{}_{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, contents) in files {
            std::fs::write(dir.join(name), contents).unwrap();
        }
        dir
    }

    #[test]
    fn clean_directory_has_no_diagnostics() {
        let viewmodel_src = r#"
viewmodel Vm {
    #[observable]
    content: String = String::new(),
}
"#;
        let window_src = r#"
component Window1 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window1 { Window { TextArea { text: vm.content } } }
"#;
        let dir = write_dir(&[("vm.elwind", viewmodel_src), ("window.elwind", window_src)]);
        let diags = diagnostics_for_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(diags.len(), 2);
        assert!(
            diags.values().all(|v| v.is_empty()),
            "expected no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn parse_error_is_reported_with_a_line_number() {
        let dir = write_dir(&[("broken.elwind", "viewmodel { #[observable] }")]);
        let diags = diagnostics_for_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();

        let broken = diags
            .iter()
            .find(|(p, _)| p.ends_with("broken.elwind"))
            .unwrap()
            .1;
        assert!(!broken.is_empty(), "expected a parse-error diagnostic");
    }

    #[test]
    fn vm_reference_error_is_attributed_to_the_view_file() {
        let viewmodel_src = "viewmodel Vm { #[observable] content: String = String::new(), }";
        let window_src = r#"
component Window2 {
    #[param]
    #[inject]
    vm: Vm,
}
view Window2 { Window { Text { text: vm.no_such_field } } }
"#;
        let dir = write_dir(&[("vm.elwind", viewmodel_src), ("window.elwind", window_src)]);
        let diags = diagnostics_for_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();

        let window_diags = diags
            .iter()
            .find(|(p, _)| p.ends_with("window.elwind"))
            .unwrap()
            .1;
        assert!(
            window_diags
                .iter()
                .any(|d| d.message.contains("no_such_field")),
            "expected window.elwind's diagnostics to mention the bad reference, got: {diags:?}"
        );
    }
}
