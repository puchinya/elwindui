//! Hand-written lexer-free recursive-descent parser for the DSL's own structural syntax
//! (`use`/`enum`/`component`/`viewmodel`/`view`). Field/attribute-value expressions that aren't
//! one of the DSL's own macro forms (`bind!`, `command!`, `t!`) are handed off to `syn` for real
//! parsing. See docs/elwindui_spec.md §1-15.

use crate::ast::*;

pub fn parse_module(src: &str) -> Result<Module, String> {
    Parser::new(src).parse_module()
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser { src, pos: 0 }
    }

    fn parse_module(&mut self) -> Result<Module, String> {
        let mut uses = Vec::new();
        let mut items = Vec::new();

        loop {
            self.skip_trivia();
            if self.at_eof() {
                break;
            }
            if self.eat_keyword("use") {
                uses.push(self.parse_use_decl()?);
            } else if self.eat_keyword("enum") {
                items.push(Item::Enum(self.parse_enum_def()?));
            } else if self.eat_keyword("component") {
                items.push(Item::Component(self.parse_fields_block(FieldKind::Prop, |name, fields| {
                    ComponentDef { name, fields }
                })?));
            } else if self.eat_keyword("viewmodel") {
                items.push(Item::ViewModel(self.parse_fields_block(FieldKind::Observable, |name, fields| {
                    ViewModelDef { name, fields }
                })?));
            } else if self.eat_keyword("view") {
                items.push(Item::View(self.parse_view_def()?));
            } else {
                return Err(self.err("expected `use`/`enum`/`component`/`viewmodel`/`view`"));
            }
        }

        Ok(Module { uses, items })
    }

    fn parse_use_decl(&mut self) -> Result<UseDecl, String> {
        let mut path = vec![self.parse_ident()?];
        while self.eat_str("::") {
            path.push(self.parse_ident()?);
        }
        self.expect_char(';')?;
        Ok(UseDecl { path })
    }

    fn parse_enum_def(&mut self) -> Result<EnumDef, String> {
        let name = self.parse_ident()?;
        self.expect_char('{')?;
        let mut variants = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            variants.push(self.parse_ident()?);
            self.skip_trivia();
            self.eat_char(',');
        }
        Ok(EnumDef { name, variants })
    }

    /// Parses `Name { field, field, ... }` for both `component` and `viewmodel` (§3, 付録O.2 share
    /// the same field grammar). `default_kind` is `Prop` for `component`, `Observable` for
    /// `viewmodel` (a field with no kind attribute defaults to its container's usual kind).
    fn parse_fields_block<T>(
        &mut self,
        default_kind: FieldKind,
        build: impl FnOnce(String, Vec<FieldDef>) -> T,
    ) -> Result<T, String> {
        let name = self.parse_ident()?;
        self.expect_char('{')?;
        let mut fields = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            fields.push(self.parse_field_def(default_kind)?);
        }
        Ok(build(name, fields))
    }

    fn parse_field_def(&mut self, default_kind: FieldKind) -> Result<FieldDef, String> {
        let mut kind = default_kind;
        let mut attrs = Vec::new();

        loop {
            self.skip_trivia();
            if !self.eat_char('#') {
                break;
            }
            self.expect_char('[')?;
            let attr_name = self.parse_ident()?;
            match attr_name.as_str() {
                "param" => kind = FieldKind::Param,
                "prop" => kind = FieldKind::Prop,
                "observable" => kind = FieldKind::Observable,
                "computed" => kind = FieldKind::Computed,
                "inject" => attrs.push(Attr::Inject),
                "command" => {
                    kind = FieldKind::Command;
                    let mut is_async = false;
                    let mut can_execute = None;
                    self.skip_trivia();
                    if self.eat_char('(') {
                        self.skip_trivia();
                        if self.eat_keyword("async") {
                            is_async = true;
                            self.skip_trivia();
                            self.eat_char(',');
                            self.skip_trivia();
                        }
                        // `can_execute: expr` (optional; `#[command(async)]` alone has none)
                        if self.peek_char() != Some(')') {
                            let arg_name = self.parse_ident()?;
                            if arg_name != "can_execute" {
                                return Err(self.err("expected `can_execute` in #[command(...)]"));
                            }
                            self.expect_char(':')?;
                            let expr_src = self.take_balanced_until(&[')'])?;
                            can_execute = Some(
                                syn::parse_str::<syn::Expr>(&expr_src)
                                    .map_err(|e| format!("invalid can_execute expr: {e}"))?,
                            );
                        }
                        self.expect_char(')')?;
                    }
                    attrs.push(Attr::CommandMeta { is_async, can_execute });
                }
                "length" => {
                    self.expect_char('(')?;
                    let range_src = self.take_balanced_until(&[')'])?;
                    let (start, end, inclusive) = parse_range(&range_src)?;
                    self.expect_char(')')?;
                    attrs.push(Attr::Length { start, end, inclusive });
                }
                other => return Err(self.err(&format!("unknown attribute #[{other}]"))),
            }
            self.expect_char(']')?;
        }

        self.skip_trivia();
        let name = self.parse_ident()?;
        self.skip_trivia();
        self.expect_char(':')?;
        let ty = self.take_balanced_until(&['=', ',', '}']).map(|s| s.trim().to_string())?;

        self.skip_trivia();
        let initializer = if self.eat_char('=') {
            self.skip_trivia();
            Some(self.parse_initializer()?)
        } else {
            None
        };

        self.skip_trivia();
        self.eat_char(',');

        Ok(FieldDef { name, ty, kind, attrs, initializer })
    }

    fn parse_initializer(&mut self) -> Result<Initializer, String> {
        if self.peek_keyword_bang("bind") {
            self.pos += "bind!".len();
            self.expect_char('(')?;
            self.skip_trivia();
            let mut path = vec![self.parse_ident()?];
            while self.eat_str(".") {
                path.push(self.parse_ident()?);
            }
            self.skip_trivia();
            self.expect_char(',')?;
            self.skip_trivia();
            let mode = self.parse_ident()?;
            self.skip_trivia();
            self.expect_char(')')?;
            return Ok(Initializer::Bind { path, mode });
        }

        if self.peek_keyword_bang("command") {
            self.pos += "command!".len();
            self.expect_char('(')?;
            let block_src = self.take_balanced_until(&[')'])?;
            self.expect_char(')')?;
            let block_src = block_src.trim();
            // `command!(async || { ... })` (付録P.4): the `async` marker itself is only tracked
            // via `#[command(async, ...)]` (see `parse_field_def`), so it's simply skipped here.
            let block_src = block_src.strip_prefix("async").map(str::trim).unwrap_or(block_src);
            // `||` (no params) or `|name: Type|` (single typed param, e.g. `close_tab`'s index).
            let block_src = block_src
                .strip_prefix("||")
                .map(|rest| (Vec::new(), rest))
                .or_else(|| {
                    let rest = block_src.strip_prefix('|')?;
                    let (param_src, rest) = rest.split_once('|')?;
                    let (name, ty_src) = param_src.split_once(':')?;
                    let ty = syn::parse_str::<syn::Type>(ty_src.trim()).ok()?;
                    Some((vec![(name.trim().to_string(), ty)], rest))
                });
            let (params, block_src) = block_src
                .ok_or_else(|| self.err("expected `||` or `|name: Type|` in command!(...)"))?;
            let block = syn::parse_str::<syn::Block>(block_src.trim())
                .map_err(|e| format!("invalid command! body: {e}"))?;
            return Ok(Initializer::Command { params, body: block });
        }

        let expr_src = self.take_balanced_until(&[',', '}'])?;
        let expr = syn::parse_str::<syn::Expr>(expr_src.trim())
            .map_err(|e| format!("invalid initializer expr `{}`: {e}", expr_src.trim()))?;
        Ok(Initializer::Expr(expr))
    }

    fn parse_view_def(&mut self) -> Result<ViewDef, String> {
        let target = self.parse_ident()?;
        self.expect_char('{')?;
        self.skip_trivia();
        let root = self.parse_element_node()?;
        self.skip_trivia();
        self.expect_char('}')?;
        Ok(ViewDef { target, root })
    }

    fn parse_element_node(&mut self) -> Result<ElementNode, String> {
        let type_path = self.parse_ident()?;
        self.skip_trivia();
        self.expect_char('{')?;

        let mut attributes = Vec::new();
        let mut children = Vec::new();

        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            let ident_start = self.pos;
            let ident = self.parse_ident()?;
            self.skip_trivia();
            if self.eat_char(':') {
                self.skip_trivia();
                let value = self.parse_view_expr()?;
                attributes.push((ident, value));
            } else {
                // bare `Type { ... }`: this is a child element, `ident` was its type name.
                self.pos = ident_start;
                children.push(self.parse_element_node()?);
            }
            self.skip_trivia();
            self.eat_char(',');
        }

        Ok(ElementNode { type_path, attributes, children })
    }

    fn parse_view_expr(&mut self) -> Result<ViewExpr, String> {
        self.skip_trivia();

        if self.peek_char() == Some('"') {
            let lit_src = self.take_string_literal()?;
            let expr = syn::parse_str::<syn::Expr>(&lit_src)
                .map_err(|e| format!("invalid string literal: {e}"))?;
            return Ok(ViewExpr::Expr(expr));
        }

        if self.peek_keyword_bang("t") {
            self.pos += "t!".len();
            self.expect_char('(')?;
            self.skip_trivia();
            let key_src = self.take_string_literal()?;
            let key = key_src.trim_matches('"').to_string();
            let mut args = Vec::new();
            loop {
                self.skip_trivia();
                if self.eat_char(')') {
                    break;
                }
                self.expect_char(',')?;
                self.skip_trivia();
                if self.peek_char() == Some(')') {
                    continue;
                }
                let arg_name = self.parse_ident()?;
                self.skip_trivia();
                self.expect_char(':')?;
                let arg_value = self.parse_view_expr()?;
                args.push((arg_name, arg_value));
            }
            return Ok(ViewExpr::TFluent(key, args));
        }

        // Dotted field path, optionally followed by `()` (a zero-arg method call).
        let mut path = vec![self.parse_ident()?];
        while self.eat_str(".") {
            path.push(self.parse_ident()?);
        }
        self.skip_trivia();
        if self.peek_str("()") {
            self.pos += 2;
            let method = path.pop().expect("path always has at least one segment");
            return Ok(ViewExpr::MethodCall(path, method));
        }
        Ok(ViewExpr::Path(path))
    }

    // --- low-level helpers ---

    fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_str(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn skip_trivia(&mut self) {
        loop {
            let rest = self.rest();
            let ws_len: usize = rest
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| c.len_utf8())
                .sum();
            self.pos += ws_len;
            if self.rest().starts_with("//") {
                let nl = self.rest().find('\n').unwrap_or(self.rest().len());
                self.pos += nl;
                continue;
            }
            break;
        }
    }

    fn eat_char(&mut self, c: char) -> bool {
        self.skip_trivia();
        if self.peek_char() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn expect_char(&mut self, c: char) -> Result<(), String> {
        if self.eat_char(c) {
            Ok(())
        } else {
            Err(self.err(&format!("expected `{c}`")))
        }
    }

    fn eat_str(&mut self, s: &str) -> bool {
        self.skip_trivia();
        if self.peek_str(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        self.skip_trivia();
        let rest = self.rest();
        if rest.starts_with(kw) {
            let after = &rest[kw.len()..];
            if after.chars().next().map(|c| !c.is_alphanumeric() && c != '_').unwrap_or(true) {
                self.pos += kw.len();
                return true;
            }
        }
        false
    }

    fn peek_keyword_bang(&mut self, kw: &str) -> bool {
        self.skip_trivia();
        let needle = format!("{kw}!");
        self.rest().starts_with(&needle)
    }

    fn parse_ident(&mut self) -> Result<String, String> {
        self.skip_trivia();
        let rest = self.rest();
        let len: usize = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .map(|c| c.len_utf8())
            .sum();
        if len == 0 {
            return Err(self.err("expected identifier"));
        }
        let ident = rest[..len].to_string();
        self.pos += len;
        Ok(ident)
    }

    fn take_string_literal(&mut self) -> Result<String, String> {
        self.skip_trivia();
        if self.peek_char() != Some('"') {
            return Err(self.err("expected string literal"));
        }
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.peek_char() {
                None => return Err(self.err("unterminated string literal")),
                Some('\\') => {
                    self.pos += 1;
                    if let Some(c) = self.peek_char() {
                        self.pos += c.len_utf8();
                    }
                }
                Some('"') => {
                    self.pos += 1;
                    break;
                }
                Some(c) => self.pos += c.len_utf8(),
            }
        }
        Ok(self.src[start..self.pos].to_string())
    }

    /// Captures raw source text up to (but not including) the first occurrence of any character
    /// in `terminators` that appears at bracket/paren/brace depth 0 and outside a string literal.
    fn take_balanced_until(&mut self, terminators: &[char]) -> Result<String, String> {
        self.skip_trivia();
        let start = self.pos;
        let mut depth: i32 = 0;
        loop {
            match self.peek_char() {
                None => return Err(self.err("unexpected end of input")),
                Some('"') => {
                    self.take_string_literal()?;
                    continue;
                }
                Some(c) if depth == 0 && terminators.contains(&c) => break,
                Some('(') | Some('[') | Some('{') => {
                    depth += 1;
                    self.pos += 1;
                }
                Some(')') | Some(']') | Some('}') => {
                    depth -= 1;
                    self.pos += 1;
                }
                Some(c) => self.pos += c.len_utf8(),
            }
        }
        Ok(self.src[start..self.pos].to_string())
    }

    fn err(&self, msg: &str) -> String {
        let line = self.src[..self.pos].matches('\n').count() + 1;
        format!("parse error at line {line}: {msg}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notepad_viewmodel() {
        let src = r#"
enum SaveState { Unsaved, Saving, Saved }

viewmodel NotepadViewModel {
    #[observable]
    #[length(0..=100000)]
    content: String = String::new(),

    #[observable]
    file_name: String = "untitled.txt",

    #[observable]
    state: SaveState = SaveState::Unsaved,

    #[computed]
    char_count: i32 = content.chars().count() as i32,

    #[computed]
    window_title: String = t!("notepad-window-title", file_name: file_name),

    #[command(can_execute: state != SaveState::Saving)]
    save: Command = command!(|| {
        state = SaveState::Saving;
        document::save(&content);
        state = SaveState::Saved;
    }),

    #[command]
    open: Command = command!(|| {
        content = document::open_dialog();
        state = SaveState::Unsaved;
    }),
}
"#;
        let module = parse_module(src).expect("should parse");
        assert_eq!(module.items.len(), 2);

        let Item::Enum(enum_def) = &module.items[0] else {
            panic!("expected enum");
        };
        assert_eq!(enum_def.name, "SaveState");
        assert_eq!(enum_def.variants, vec!["Unsaved", "Saving", "Saved"]);

        let Item::ViewModel(vm) = &module.items[1] else {
            panic!("expected viewmodel");
        };
        assert_eq!(vm.name, "NotepadViewModel");
        assert_eq!(vm.fields.len(), 7);

        assert_eq!(vm.fields[0].name, "content");
        assert_eq!(vm.fields[0].kind, FieldKind::Observable);
        assert!(matches!(
            vm.fields[0].attrs.as_slice(),
            [Attr::Length { start: 0, end: 100000, inclusive: true }]
        ));

        assert_eq!(vm.fields[3].name, "char_count");
        assert_eq!(vm.fields[3].kind, FieldKind::Computed);
        assert!(matches!(vm.fields[3].initializer, Some(Initializer::Expr(_))));

        assert_eq!(vm.fields[4].name, "window_title");
        assert!(matches!(vm.fields[4].initializer, Some(Initializer::Expr(_))));

        assert_eq!(vm.fields[5].name, "save");
        assert_eq!(vm.fields[5].kind, FieldKind::Command);
        assert!(matches!(vm.fields[5].initializer, Some(Initializer::Command { .. })));
        let has_can_execute = vm.fields[5]
            .attrs
            .iter()
            .any(|a| matches!(a, Attr::CommandMeta { can_execute: Some(_), .. }));
        assert!(has_can_execute);
    }

    #[test]
    fn parses_notepad_window() {
        let src = r#"
use elwindui::viewmodel::NotepadViewModel;

component NotepadWindow {
    #[param]
    #[inject]
    vm: NotepadViewModel,

    content: String = bind!(vm.content, TwoWay),
}

view NotepadWindow {
    Window {
        title: vm.window_title

        Column {
            Row {
                Button {
                    text: t!("notepad-menu-save")
                    on_click: vm.save.execute()
                    enabled: vm.save.can_execute
                }
                Button {
                    text: t!("notepad-menu-open")
                    on_click: vm.open.execute()
                }
            }

            TextArea { text: content }

            Row {
                Text { text: t!("notepad-status-chars", count: vm.char_count) }
            }
        }
    }
}
"#;
        let module = parse_module(src).expect("should parse");
        assert_eq!(module.uses.len(), 1);
        assert_eq!(module.uses[0].path, vec!["elwindui", "viewmodel", "NotepadViewModel"]);
        assert_eq!(module.items.len(), 2);

        let Item::Component(component) = &module.items[0] else {
            panic!("expected component");
        };
        assert_eq!(component.name, "NotepadWindow");
        assert_eq!(component.fields.len(), 2);
        assert_eq!(component.fields[0].name, "vm");
        assert_eq!(component.fields[0].kind, FieldKind::Param);
        assert!(component.fields[0].initializer.is_none());
        assert!(matches!(
            component.fields[1].initializer,
            Some(Initializer::Bind { .. })
        ));

        let Item::View(view) = &module.items[1] else {
            panic!("expected view");
        };
        assert_eq!(view.target, "NotepadWindow");
        assert_eq!(view.root.type_path, "Window");
        assert_eq!(view.root.children.len(), 1);

        let column = &view.root.children[0];
        assert_eq!(column.type_path, "Column");
        assert_eq!(column.children.len(), 3);
        assert_eq!(column.children[0].type_path, "Row");
        assert_eq!(column.children[1].type_path, "TextArea");
        assert_eq!(column.children[2].type_path, "Row");

        let save_button = &column.children[0].children[0];
        assert_eq!(save_button.type_path, "Button");
        let on_click = save_button
            .attributes
            .iter()
            .find(|(k, _)| k == "on_click")
            .map(|(_, v)| v)
            .unwrap();
        assert!(matches!(on_click, ViewExpr::MethodCall(path, method)
            if path == &vec!["vm".to_string(), "save".to_string()] && method == "execute"));
    }
}

/// Parses `start..=end` / `start..end` for `#[length(...)]`/`#[range(...)]`-style attributes.
fn parse_range(src: &str) -> Result<(i64, i64, bool), String> {
    let src = src.trim();
    if let Some((start, rest)) = src.split_once("..=") {
        let start: i64 = start.trim().parse().map_err(|_| "invalid range start".to_string())?;
        let end: i64 = rest.trim().parse().map_err(|_| "invalid range end".to_string())?;
        Ok((start, end, true))
    } else if let Some((start, rest)) = src.split_once("..") {
        let start: i64 = start.trim().parse().map_err(|_| "invalid range start".to_string())?;
        let end: i64 = rest.trim().parse().map_err(|_| "invalid range end".to_string())?;
        Ok((start, end, false))
    } else {
        Err(format!("invalid range `{src}`"))
    }
}
