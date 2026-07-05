//! Standalone tokenizer for `.elwind` syntax highlighting via LSP `textDocument/semanticTokens/full`.
//!
//! Deliberately NOT built on `elwindui_codegen::parser`: that parser is lexer-free by design (see
//! its own doc comment) and throws away source positions once a token is recognized, since
//! `ast.rs` has no span fields to put them in. Retrofitting span-tracking through the whole
//! parser/AST just for coloring would be a much larger change than highlighting needs — a
//! dedicated scanner that only classifies raw lexical spans is enough.

use lsp_types::{SemanticToken, SemanticTokenType};

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
const ATTR_NAMES: &[&str] =
    &["param", "prop", "observable", "computed", "inject", "command", "length"];
// DSL macro forms recognized by `peek_keyword_bang`/direct match: `bind!`, `command!`, `t!`.
const MACRO_NAMES: &[&str] = &["bind", "command", "t"];

struct RawToken {
    line: u32,
    start: u32,
    len: u32,
    ty: u32,
}

/// Cursor over `char`s tracking (line, UTF-16 column) so token spans line up with LSP's default
/// position encoding (UTF-16 code units), not byte or `char` offsets.
struct Scanner<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: u32,
    col: u32,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Scanner { chars: src.chars().peekable(), line: 0, col: 0 }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.col = 0;
        } else {
            self.col += c.len_utf16() as u32;
        }
        Some(c)
    }
}

pub fn semantic_tokens_for_source(src: &str) -> Vec<SemanticToken> {
    let raw = tokenize(src);
    let mut out = Vec::with_capacity(raw.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for tok in raw {
        let delta_line = tok.line - prev_line;
        let delta_start = if delta_line == 0 { tok.start - prev_start } else { tok.start };
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

fn tokenize(src: &str) -> Vec<RawToken> {
    let mut sc = Scanner::new(src);
    let mut raw = Vec::new();

    while let Some(c) = sc.peek() {
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
                raw.push(RawToken { line: start_line, start: start_col, len: sc.col - start_col, ty: COMMENT });
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
            raw.push(RawToken { line: start_line, start: start_col, len: sc.col - start_col, ty: STRING });
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
            raw.push(RawToken { line: start_line, start: start_col, len: sc.col - start_col, ty: NUMBER });
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
                    raw.push(RawToken { line: attr_line, start: attr_col, len: sc.col - attr_col, ty: MACRO });
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
            raw.push(RawToken { line: start_line, start: start_col, len: sc.col - start_col, ty });
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
        semantic_tokens_for_source(src)
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

    #[test]
    fn classifies_keyword_type_and_attribute() {
        let toks = decode("component NotepadWindow {\n    #[param]\n    vm: Vm,\n}\n");
        assert_eq!(toks[0], (0, 0, "component".len() as u32, KEYWORD));
        assert_eq!(toks[1], (0, 10, "NotepadWindow".len() as u32, TYPE));
        assert_eq!(toks[2], (1, 6, "param".len() as u32, MACRO));
        assert_eq!(toks[3], (2, 4, "vm".len() as u32, VARIABLE));
        assert_eq!(toks[4], (2, 8, "Vm".len() as u32, TYPE));
    }

    #[test]
    fn classifies_string_number_comment_and_macro_bang() {
        let toks = decode("// hello\nlength: 3,\nbind!(vm.content, TwoWay)\nt!(\"key\")\n");
        assert_eq!(toks[0], (0, 0, "// hello".len() as u32, COMMENT));
        // `length` here is a bare field name, not `#[length]`, so it's just a VARIABLE identifier.
        assert_eq!(toks[1], (1, 0, "length".len() as u32, VARIABLE));
        assert_eq!(toks[2], (1, 8, "3".len() as u32, NUMBER));
        assert_eq!(toks[3], (2, 0, "bind!".len() as u32, MACRO));
        let t_bang = toks.iter().find(|t| t.3 == MACRO && t.0 == 3).unwrap();
        assert_eq!(*t_bang, (3, 0, "t!".len() as u32, MACRO));
        assert!(toks.iter().any(|t| t.3 == STRING));
    }
}
