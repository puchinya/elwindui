//! `textDocument/semanticTokens/full` — scoped to just each `view! { .. }` macro body's worth of
//! text (Issue #14's "未解決の論点", resolved in favor of this option): a `.rs` file already gets
//! real Rust syntax highlighting from rust-analyzer everywhere else, so providing tokens for the
//! whole file would double-color ordinary Rust and could conflict with rust-analyzer's own semantic
//! tokens. `view! { .. }`'s own contents are the one part of the file rust-analyzer can't highlight
//! meaningfully — `view!` is never a real macro (see `component_frontend.rs`'s own doc comment), so
//! it shows as an unexpanded, undecorated token stream to any ordinary Rust-aware tool.
//!
//! The scanner (`Scanner`/`tokenize`/`RawToken`, classification logic and the `KEYWORDS`/
//! `ATTR_NAMES`/`MACRO_NAMES` tables) is the pre-Phase-7 text-form tokenizer, otherwise
//! unchanged (see git history at `b648618^` for the original whole-file version) — deliberately not
//! built on `elwindui_codegen::parser` (span-free by design, see that module's own doc comment), so
//! a dedicated lexical scanner is the only way to recover per-character positions at all. What's new
//! here is scoping: `tokenize` now takes `ranges` (each `view! { .. }`'s exact byte span in the
//! *original* source, one `Range` per field) and only classifies characters that fall inside one —
//! everywhere else is walked (to keep the running line/column count correct for whatever follows)
//! but never classified, so nothing outside a `view!` body ever gets a token.
//!
//! Locating those ranges needs real source positions, which `syn::parse_file`'s AST doesn't carry by
//! default — enabled here via `proc-macro2`'s `span-locations` feature (`Cargo.toml`), which gives
//! accurate `Span::start()` locations even outside a real proc-macro invocation (this crate is an
//! ordinary binary, never itself expanding as a proc-macro). The final token's end is derived from
//! that start location plus its source spelling because rust-analyzer's proc-macro model exposes
//! the start position but not the range helpers.

use lsp_types::{SemanticToken, SemanticTokenType};
use std::ops::Range;

pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,  // 0
    SemanticTokenType::TYPE,     // 1
    SemanticTokenType::STRING,   // 2
    SemanticTokenType::NUMBER,   // 3
    SemanticTokenType::COMMENT,  // 4
    SemanticTokenType::MACRO,    // 5
    SemanticTokenType::VARIABLE, // 6
];

const KEYWORD: u32 = 0;
const TYPE: u32 = 1;
const STRING: u32 = 2;
const NUMBER: u32 = 3;
const COMMENT: u32 = 4;
const MACRO: u32 = 5;
const VARIABLE: u32 = 6;

// Structural keywords `parser.rs` actually recognizes via `eat_keyword` (§1-15).
const KEYWORDS: &[&str] = &["use", "enum", "component", "viewmodel", "view", "async"];
// `#[name(...)]` attribute names `parse_field_def` recognizes (kind markers + `inject`/`length`).
const ATTR_NAMES: &[&str] = &[
    "param",
    "prop",
    "state",
    "observable",
    "computed",
    "inject",
    "bindable",
    "two_way",
    "length",
];
// DSL and dependency-analyzable expression macros accepted inside `view!` values.
const MACRO_NAMES: &[&str] = &["once", "t", "format", "format_args", "vec"];

/// Finds every `view! { .. }` field's exact byte range in `src` and returns semantic tokens for
/// their contents only — see this module's own doc comment. Returns an empty `Vec` (rather than
/// erroring) for a file that doesn't parse or has no `view!` fields at all; a broken file already
/// gets a real diagnostic from `diagnostics.rs`, and semantic tokens degrading to "none" rather than
/// erroring matches how an LSP client expects this request to behave.
pub fn semantic_tokens_for_file(src: &str) -> Vec<SemanticToken> {
    let Ok(file) = syn::parse_file(src) else {
        return Vec::new();
    };
    let ranges = view_body_ranges(src, &file);
    encode(tokenize(src, &ranges))
}

/// Every top-level struct field typed `view! { .. }`, matching `component_frontend.rs`'s own
/// `is_view_macro_field` check — mirrors that module's flat, non-recursive-into-`mod` walk of
/// `file.items` (a `view!` field only ever appears on a top-level `#[elwindui::component] struct`,
/// same convention `component_frontend::modules_from_file` already relies on).
fn view_body_ranges(src: &str, file: &syn::File) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item_struct) => Some(item_struct),
            _ => None,
        })
        .filter_map(|item_struct| match &item_struct.fields {
            syn::Fields::Named(named) => Some(named),
            _ => None,
        })
        .flat_map(|named| named.named.iter())
        .filter_map(|field| match &field.ty {
            syn::Type::Macro(tm) if tm.mac.path.is_ident("view") => {
                token_stream_byte_range(src, &tm.mac.tokens)
            }
            _ => None,
        })
        .collect();
    ranges.sort_by_key(|r| r.start);
    ranges
}

/// The byte range spanning every token in `tokens` (a `view!` macro's own content, i.e. between —
/// not including — its own `{`/`}` delimiters), from the first token's start to the last token's
/// end. `None` for an empty `view! {}` (nothing to highlight). `proc_macro2::Span` exposes the
/// starting line/column on rust-analyzer's analysis model even when its range methods are
/// unavailable, so derive the final token's byte end from its source position and token text.
fn token_stream_byte_range(src: &str, tokens: &proc_macro2::TokenStream) -> Option<Range<usize>> {
    let mut iter = tokens.clone().into_iter();
    let first = iter.next()?;
    let start = line_column_byte_offset(src, first.span().start());
    let last = iter.last().unwrap_or_else(|| first.clone());
    let last_start = token_start_byte_offset(src, &last);
    let end = last_start
        .saturating_add(last.to_string().len())
        .min(src.len());
    Some(start..end)
}

fn token_start_byte_offset(src: &str, token: &proc_macro2::TokenTree) -> usize {
    let location = match token {
        proc_macro2::TokenTree::Group(group) => group.span().start(),
        proc_macro2::TokenTree::Ident(ident) => ident.span().start(),
        proc_macro2::TokenTree::Punct(punct) => punct.span().start(),
        proc_macro2::TokenTree::Literal(literal) => literal.span().start(),
    };
    line_column_byte_offset(src, location)
}

fn line_column_byte_offset(src: &str, location: proc_macro2::LineColumn) -> usize {
    let line_index = location.line.saturating_sub(1) as usize;
    let line_start = src
        .split_inclusive('\n')
        .take(line_index)
        .map(str::len)
        .sum::<usize>();
    line_start.saturating_add(location.column).min(src.len())
}

fn in_ranges(ranges: &[Range<usize>], pos: usize) -> bool {
    ranges.iter().any(|r| r.contains(&pos))
}

struct RawToken {
    line: u32,
    start: u32,
    len: u32,
    ty: u32,
}

/// Cursor over `char`s tracking (line, UTF-16 column, byte offset) so token spans line up with
/// LSP's default position encoding (UTF-16 code units) while still being comparable against
/// `ranges`' byte offsets (converted from `proc_macro2::Span` line/column locations).
struct Scanner<'a> {
    iter: std::iter::Peekable<std::str::CharIndices<'a>>,
    line: u32,
    col: u32,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Scanner {
            iter: src.char_indices().peekable(),
            line: 0,
            col: 0,
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.iter.peek().map(|&(_, c)| c)
    }

    fn peek_pos(&mut self) -> Option<usize> {
        self.iter.peek().map(|&(i, _)| i)
    }

    fn bump(&mut self) -> Option<char> {
        let (_, c) = self.iter.next()?;
        if c == '\n' {
            self.line += 1;
            self.col = 0;
        } else {
            self.col += c.len_utf16() as u32;
        }
        Some(c)
    }
}

fn encode(raw: Vec<RawToken>) -> Vec<SemanticToken> {
    let mut out = Vec::with_capacity(raw.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for tok in raw {
        let delta_line = tok.line - prev_line;
        let delta_start = if delta_line == 0 {
            tok.start - prev_start
        } else {
            tok.start
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: tok.len,
            token_type: tok.ty,
            token_modifiers_bitset: 0,
        });
        prev_line = tok.line;
        prev_start = tok.start;
    }
    out
}

fn tokenize(src: &str, ranges: &[Range<usize>]) -> Vec<RawToken> {
    let mut sc = Scanner::new(src);
    let mut raw = Vec::new();

    while let Some(c) = sc.peek() {
        let pos = sc.peek_pos().expect("just peeked Some(c)");
        if !in_ranges(ranges, pos) {
            sc.bump();
            continue;
        }

        if c.is_whitespace() {
            sc.bump();
            continue;
        }

        let start_line = sc.line;
        let start_col = sc.col;

        if c == '/' {
            sc.bump();
            if sc.peek() == Some('/') {
                while let Some(c2) = sc.peek() {
                    if c2 == '\n' {
                        break;
                    }
                    sc.bump();
                }
                raw.push(RawToken {
                    line: start_line,
                    start: start_col,
                    len: sc.col - start_col,
                    ty: COMMENT,
                });
            }
            continue;
        }

        if c == '"' {
            sc.bump();
            while let Some(c2) = sc.bump() {
                if c2 == '\\' {
                    sc.bump();
                    continue;
                }
                if c2 == '"' {
                    break;
                }
            }
            raw.push(RawToken {
                line: start_line,
                start: start_col,
                len: sc.col - start_col,
                ty: STRING,
            });
            continue;
        }

        if c.is_ascii_digit() {
            while let Some(c2) = sc.peek() {
                if c2.is_ascii_alphanumeric() || c2 == '.' || c2 == '_' {
                    sc.bump();
                } else {
                    break;
                }
            }
            raw.push(RawToken {
                line: start_line,
                start: start_col,
                len: sc.col - start_col,
                ty: NUMBER,
            });
            continue;
        }

        if c == '#' {
            sc.bump();
            if sc.peek() == Some('[') {
                sc.bump();
                while sc.peek().is_some_and(|c| c.is_whitespace()) {
                    sc.bump();
                }
                let attr_line = sc.line;
                let attr_col = sc.col;
                let mut name = String::new();
                while let Some(c2) = sc.peek() {
                    if c2.is_alphanumeric() || c2 == '_' {
                        name.push(c2);
                        sc.bump();
                    } else {
                        break;
                    }
                }
                if ATTR_NAMES.contains(&name.as_str()) {
                    raw.push(RawToken {
                        line: attr_line,
                        start: attr_col,
                        len: sc.col - attr_col,
                        ty: MACRO,
                    });
                }
            }
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let mut ident = String::new();
            while let Some(c2) = sc.peek() {
                if c2.is_alphanumeric() || c2 == '_' {
                    ident.push(c2);
                    sc.bump();
                } else {
                    break;
                }
            }
            let ty = if KEYWORDS.contains(&ident.as_str()) {
                KEYWORD
            } else if MACRO_NAMES.contains(&ident.as_str()) && sc.peek() == Some('!') {
                sc.bump();
                MACRO
            } else if ident.chars().next().is_some_and(|c| c.is_uppercase()) {
                TYPE
            } else {
                VARIABLE
            };
            raw.push(RawToken {
                line: start_line,
                start: start_col,
                len: sc.col - start_col,
                ty,
            });
            continue;
        }

        sc.bump();
    }

    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(src: &str) -> Vec<(u32, u32, u32, u32)> {
        let mut line = 0u32;
        let mut start = 0u32;
        semantic_tokens_for_file(src)
            .into_iter()
            .map(|t| {
                line += t.delta_line;
                if t.delta_line != 0 {
                    start = t.delta_start;
                } else {
                    start += t.delta_start;
                }
                (line, start, t.length, t.token_type)
            })
            .collect()
    }

    /// A struct field with an ordinary Rust type never contributes any token — only a `view! { .. }`
    /// field's own contents do. Confirms the whole point of the scoping: `#[param]`/`vm: Vm`/etc.
    /// (real Rust, already colored by rust-analyzer) produce nothing here.
    #[test]
    fn only_the_view_macro_body_is_tokenized() {
        let src = r#"
#[elwindui::component(inherits Window)]
struct NotepadWindow {
    #[param]
    vm: Vm,
    body: view! { TextBlock { text: "hi" } },
}
"#;
        let toks = decode(src);
        // Nothing from `#[elwindui::component(inherits Window)]`, `struct NotepadWindow`,
        // `#[param]`, or `vm: Vm` — only `TextBlock`/`text`/`"hi"` from inside `view! { .. }`.
        assert_eq!(toks.len(), 3, "{toks:?}");
        let types: Vec<u32> = toks.iter().map(|t| t.3).collect();
        assert_eq!(types, vec![TYPE, VARIABLE, STRING]);
    }

    #[test]
    fn classifies_string_number_and_element_type_inside_view() {
        let src = "struct Foo {\n    body: view! {\n        Rectangle { width: 3.0, fill: \"#000\" }\n    },\n}\n";
        let toks = decode(src);
        let types: Vec<u32> = toks.iter().map(|t| t.3).collect();
        assert_eq!(
            types,
            vec![TYPE, VARIABLE, NUMBER, VARIABLE, STRING],
            "{toks:?}"
        );
    }

    #[test]
    fn recognizes_once_macro_calls_inside_view() {
        let src = r#"
struct Foo {
    body: view! { TextBlock { text: once!(format!("{}", vm.content)) } },
}
"#;
        let toks = decode(src);
        assert!(
            toks.iter()
                .any(|t| t.3 == MACRO && t.2 == "once!".len() as u32),
            "{toks:?}"
        );
    }

    /// Two components in one file, each with their own `view!` field — both get tokenized, nothing
    /// from the plain-Rust text between/around them does (mirrors a real multi-component `.rs` file
    /// like `examples/notepad/src/ui/notepad_window.rs`).
    #[test]
    fn tokenizes_every_view_field_across_multiple_components_in_one_file() {
        let src = r#"
struct A {
    body: view! { TextBlock { text: "a" } },
}

struct B {
    body: view! { TextBlock { text: "b" } },
}
"#;
        let toks = decode(src);
        let strings: Vec<u32> = toks.iter().filter(|t| t.3 == STRING).map(|t| t.2).collect();
        assert_eq!(strings.len(), 2, "{toks:?}");
    }

    #[test]
    fn a_component_with_no_view_field_produces_no_tokens() {
        let src = "struct A {\n    #[param]\n    label: String,\n}\n";
        assert!(decode(src).is_empty());
    }

    #[test]
    fn an_unparseable_file_produces_no_tokens_rather_than_panicking() {
        let src = "struct A {\n    field:\n";
        assert!(decode(src).is_empty());
    }
}
