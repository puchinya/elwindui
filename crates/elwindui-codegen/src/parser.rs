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
                items.push(Item::Component(self.parse_fields_block(FieldKind::Prop, |name, base, fields| {
                    ComponentDef { name, base, fields }
                })?));
            } else if self.eat_keyword("viewmodel") {
                items.push(Item::ViewModel(self.parse_fields_block(FieldKind::Observable, |name, _base, fields| {
                    ViewModelDef { name, fields }
                })?));
            } else if self.eat_keyword("view") {
                items.push(Item::View(self.parse_view_def()?));
            } else {
                return Err(self.err("expected `use`/`enum`/`component`/`viewmodel`/`view`"));
            }
        }

        // `parse_module` only ever sees source text, not a file path — real module paths (付録B.1)
        // are assigned by the caller (`compile_dir_impl`), which knows where each file actually
        // lands in the crate. Defaults to `[]` (crate root), matching `Module`'s `Default`.
        Ok(Module { path: Vec::new(), uses, items })
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

    /// Parses `Name [inherits Base] { field, field, ... }` for both `component` and `viewmodel`
    /// (§3, 付録O.2 share the same field grammar). `default_kind` is `Prop` for `component`,
    /// `Observable` for `viewmodel` (a field with no kind attribute defaults to its container's
    /// usual kind). `inherits` is only meaningful for `component` (see `ComponentDef::base`'s doc
    /// comment) — parsed here regardless since the grammar up to `{` is otherwise identical, but
    /// `viewmodel`'s `build` closure simply discards it.
    fn parse_fields_block<T>(
        &mut self,
        default_kind: FieldKind,
        build: impl FnOnce(String, Option<String>, Vec<FieldDef>) -> T,
    ) -> Result<T, String> {
        let name = self.parse_ident()?;
        self.skip_trivia();
        let base = if self.eat_keyword("inherits") {
            self.skip_trivia();
            Some(self.parse_ident()?)
        } else {
            None
        };
        self.skip_trivia();
        self.expect_char('{')?;
        let mut fields = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            fields.push(self.parse_field_def(default_kind)?);
        }
        Ok(build(name, base, fields))
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
                "two_way" => attrs.push(Attr::TwoWay),
                "routed" => attrs.push(Attr::Routed),
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

        let mut lets = Vec::new();
        loop {
            self.skip_trivia();
            let checkpoint = self.pos;
            let mut id = None;
            if self.eat_char('#') {
                self.expect_char('[')?;
                let attr_name = self.parse_ident()?;
                if attr_name != "id" {
                    return Err(self.err(&format!("unknown view-level attribute #[{attr_name}] (only #[id(\"...\")] is supported here)")));
                }
                self.expect_char('(')?;
                self.skip_trivia();
                let id_src = self.take_string_literal()?;
                id = Some(id_src.trim_matches('"').to_string());
                self.skip_trivia();
                self.expect_char(')')?;
                self.expect_char(']')?;
                self.skip_trivia();
            }
            if self.eat_keyword("let") {
                self.skip_trivia();
                let name = self.parse_ident()?;
                self.skip_trivia();
                self.expect_char('=')?;
                self.skip_trivia();
                let element = self.parse_element_node()?;
                self.skip_trivia();
                self.expect_char(';')?;
                lets.push(LetBinding { id, name, element });
            } else if id.is_some() {
                return Err(self.err("#[id(\"...\")] must be immediately followed by a `let` binding"));
            } else {
                self.pos = checkpoint;
                break;
            }
        }

        self.skip_trivia();
        let root = self.parse_element_node()?;
        self.skip_trivia();
        self.expect_char('}')?;
        Ok(ViewDef { target, lets, root })
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
            } else if self.peek_char() == Some('{') {
                // bare `Type { ... }`: this is a nested child element, `ident` was its type name.
                self.pos = ident_start;
                children.push(ChildEntry::Literal(self.parse_element_node()?));
            } else {
                // bare identifier with neither `:` nor `{` following: a reference to an earlier
                // `#[id(...)]? let <ident> = ...;` binding (see `parse_view_def`), e.g. `Column {
                // editor, StatusBar {} }`'s `editor`.
                children.push(ChildEntry::Ref(ident));
            }
            self.skip_trivia();
            self.eat_char(',');
        }

        Ok(ElementNode { type_path, attributes, children })
    }

    fn parse_view_expr(&mut self) -> Result<ViewExpr, String> {
        self.skip_trivia();

        if self.peek_char() == Some('|') {
            return self.parse_closure();
        }

        if self.peek_char() == Some('"') {
            let lit_src = self.take_string_literal()?;
            let expr = syn::parse_str::<syn::Expr>(&lit_src)
                .map_err(|e| format!("invalid string literal: {e}"))?;
            return Ok(ViewExpr::Expr(expr));
        }

        // `true`/`false` as bool literals — otherwise indistinguishable from an ordinary
        // dotted-path reference (a bare identifier) by the check below, which would silently
        // parse them as `ViewExpr::Path(["true"])` and fail (or worse, half-succeed) only once
        // something actually tries to evaluate the value, e.g. `closable: true` (付録Y).
        if self.eat_keyword("true") {
            return Ok(ViewExpr::Expr(syn::parse_quote!(true)));
        }
        if self.eat_keyword("false") {
            return Ok(ViewExpr::Expr(syn::parse_quote!(false)));
        }

        // A number literal (`8`, `8.0`, `-1.5`) — needed for `#[param]` fields like `Rectangle`'s
        // `corner_radius`/`stroke_width` or `VerticalLayout`'s `spacing`. Must be checked before
        // the dotted-path branch below (a bare identifier can't start with a digit, but without
        // this check a leading `-` would otherwise fall through and fail `parse_ident`).
        if self.peek_char().is_some_and(|c| c.is_ascii_digit())
            || (self.peek_char() == Some('-') && self.rest()[1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            let lit_src = self.take_number_literal()?;
            let expr = syn::parse_str::<syn::Expr>(&lit_src)
                .map_err(|e| format!("invalid number literal: {e}"))?;
            return Ok(ViewExpr::Expr(expr));
        }

        // Bare `Type { .. }` as an ordinary (non-closure) attribute value — a builtin shape's
        // "named single-child slot" (e.g. `Window`'s `menu_bar: MenuBar { .. }`), generalizing the
        // same shape `ClosureBody::Element` already uses inside `|param| Type { .. }` bodies.
        if self.looks_like_element() {
            let element = self.parse_element_node()?;
            return Ok(ViewExpr::Element(Box::new(element)));
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

    /// `|doc| <body>` — 付録Y's `key`/`render_label`/`render_content` attributes. Always a single
    /// untyped bound parameter (no destructuring, no `: Type`); the body is either a nested
    /// element construction (`render_content: |doc| DocumentView { doc: doc }`) or a plain
    /// expression (`key`/`render_label`).
    fn parse_closure(&mut self) -> Result<ViewExpr, String> {
        self.expect_char('|')?;
        self.skip_trivia();
        let param = self.parse_ident()?;
        self.skip_trivia();
        self.expect_char('|')?;
        self.skip_trivia();

        if self.looks_like_element() {
            let element = self.parse_element_node()?;
            return Ok(ViewExpr::Closure { param, body: ClosureBody::Element(Box::new(element)) });
        }

        let body = self.parse_closure_expr_body()?;
        Ok(ViewExpr::Closure { param, body: ClosureBody::Expr(Box::new(body)) })
    }

    /// Lookahead-and-rewind (same idiom `parse_element_node` uses at its attribute/child-element
    /// split) to tell a bare `Type { ... }` (an element construction) apart from a plain
    /// expression, without consuming anything.
    fn looks_like_element(&mut self) -> bool {
        let save = self.pos;
        let is_type_name = self
            .parse_ident()
            .map(|ident| ident.chars().next().is_some_and(|c| c.is_uppercase()))
            .unwrap_or(false);
        self.skip_trivia();
        let followed_by_brace = self.peek_char() == Some('{');
        self.pos = save;
        is_type_name && followed_by_brace
    }

    /// A closure expression body. View attributes have no required separator between them (a
    /// closure body followed directly by the next attribute on its own line, with no trailing
    /// `,`, is the DSL's own convention — see `parse_element_node`'s optional `self.eat_char(',')`)
    /// so the body's extent can't be determined by trying the DSL's own dotted-path grammar
    /// in-place and inspecting whatever character happens to follow — `parse_view_expr`'s dotted-
    /// path branch already calls `skip_trivia()` internally before returning, which would silently
    /// consume the very whitespace boundary being inspected. Instead, first capture the bounded
    /// span up to end-of-line (via `take_expr_until_line_end_or`), then try the DSL's own
    /// dotted-path/`t!` sugar on an isolated sub-parser over just that text (so `doc.file_name`
    /// still gets the "call the getter" treatment every other attribute value gets), falling back
    /// to a raw `syn::Expr` only if that grammar doesn't consume the whole span — e.g.
    /// `std::rc::Rc::as_ptr(doc) as usize` (`::` paths, casts) — same "hand off to syn" idiom
    /// `parse_initializer`'s fallback already uses.
    fn parse_closure_expr_body(&mut self) -> Result<ViewExpr, String> {
        let expr_src = self.take_expr_until_line_end_or(&[',', '}'])?;
        let trimmed = expr_src.trim();

        let mut sub_parser = Parser::new(trimmed);
        if let Ok(expr) = sub_parser.parse_view_expr() {
            sub_parser.skip_trivia();
            if sub_parser.at_eof() {
                return Ok(expr);
            }
        }

        let expr = syn::parse_str::<syn::Expr>(trimmed)
            .map_err(|e| format!("invalid closure body `{trimmed}`: {e}"))?;
        Ok(ViewExpr::Expr(expr))
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

    /// An optional leading `-`, digits, and an optional `.` followed by more digits — just enough
    /// to feed `syn` a valid Rust integer/float literal (no exponents, no suffixes like `8.0f32`,
    /// which the DSL has never needed so far).
    fn take_number_literal(&mut self) -> Result<String, String> {
        self.skip_trivia();
        let rest = self.rest();
        let mut len = 0;
        let mut chars = rest.char_indices().peekable();
        if let Some((_, '-')) = chars.peek().copied() {
            len += 1;
            chars.next();
        }
        let mut saw_digit = false;
        while let Some((_, c)) = chars.peek().copied() {
            if !c.is_ascii_digit() {
                break;
            }
            saw_digit = true;
            len += 1;
            chars.next();
        }
        if let Some((_, '.')) = chars.peek().copied() {
            len += 1;
            chars.next();
            while let Some((_, c)) = chars.peek().copied() {
                if !c.is_ascii_digit() {
                    break;
                }
                len += 1;
                chars.next();
            }
        }
        if !saw_digit {
            return Err(self.err("expected number literal"));
        }
        let lit = rest[..len].to_string();
        self.pos += len;
        Ok(lit)
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

    /// Like `take_balanced_until`, but also stops at an unnested newline — needed for a closure
    /// body's `syn::Expr` fallback, since view attributes have no required separator between them
    /// (`parse_element_node`'s trailing `,` is optional; one-attribute-per-line with no comma is
    /// the DSL's own convention), so only `,`/`}` would otherwise swallow the following attributes'
    /// text as part of the expression.
    fn take_expr_until_line_end_or(&mut self, terminators: &[char]) -> Result<String, String> {
        self.skip_trivia();
        let start = self.pos;
        let mut depth: i32 = 0;
        loop {
            match self.peek_char() {
                None => break,
                Some('"') => {
                    self.take_string_literal()?;
                    continue;
                }
                Some('\n') if depth == 0 => break,
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
use crate::notepad_view_model::NotepadViewModel;

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
        assert_eq!(module.uses[0].path, vec!["crate", "notepad_view_model", "NotepadViewModel"]);
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

        let column = literal(&view.root.children[0]);
        assert_eq!(column.type_path, "Column");
        assert_eq!(column.children.len(), 3);
        assert_eq!(literal(&column.children[0]).type_path, "Row");
        assert_eq!(literal(&column.children[1]).type_path, "TextArea");
        assert_eq!(literal(&column.children[2]).type_path, "Row");

        let save_button = literal(&literal(&column.children[0]).children[0]);
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

    /// Unwraps a test fixture's `ChildEntry`, which is always a literal nested element (none of
    /// these fixtures reference a `let`-bound name).
    fn literal(entry: &ChildEntry) -> &ElementNode {
        match entry {
            ChildEntry::Literal(elem) => elem,
            ChildEntry::Ref(name) => panic!("expected a literal child element, found a `let`-ref to `{name}`"),
        }
    }

    fn parse_closure_attr(attr_src: &str) -> ViewExpr {
        let src = format!("view V {{ TabView {{ {attr_src} }} }}");
        let module = parse_module(&src).expect("should parse");
        let Item::View(view) = &module.items[0] else { panic!("expected view") };
        let (_, expr) = view.root.attributes.iter().find(|(k, _)| k == "x").expect("attribute `x`").clone();
        expr
    }

    #[test]
    fn parses_closure_with_dotted_path_body() {
        let expr = parse_closure_attr("x: |doc| doc.file_name");
        let ViewExpr::Closure { param, body } = expr else { panic!("expected closure, got {expr:?}") };
        assert_eq!(param, "doc");
        let ClosureBody::Expr(inner) = body else { panic!("expected expr body") };
        assert!(matches!(*inner, ViewExpr::Path(p) if p == vec!["doc".to_string(), "file_name".to_string()]));
    }

    #[test]
    fn parses_closure_with_syn_fallback_body() {
        let expr = parse_closure_attr("x: |doc| std::rc::Rc::as_ptr(doc) as usize");
        let ViewExpr::Closure { param, body } = expr else { panic!("expected closure, got {expr:?}") };
        assert_eq!(param, "doc");
        let ClosureBody::Expr(inner) = body else { panic!("expected expr body") };
        assert!(matches!(*inner, ViewExpr::Expr(_)), "expected a raw syn::Expr fallback, got {inner:?}");
    }

    #[test]
    fn parses_closure_with_element_body() {
        let expr = parse_closure_attr("x: |doc| DocumentView { doc: doc }");
        let ViewExpr::Closure { param, body } = expr else { panic!("expected closure, got {expr:?}") };
        assert_eq!(param, "doc");
        let ClosureBody::Element(elem) = body else { panic!("expected element body") };
        assert_eq!(elem.type_path, "DocumentView");
        assert_eq!(elem.attributes.len(), 1);
        assert_eq!(elem.attributes[0].0, "doc");
        assert!(matches!(&elem.attributes[0].1, ViewExpr::Path(p) if p == &vec!["doc".to_string()]));
    }

    /// Multiple closure-bearing attributes with no trailing commas, one per line — the DSL's own
    /// convention (`parse_element_node`'s `,` is optional) — must each stop at the right boundary
    /// rather than swallowing the next attribute's text. Regression test for the bug where
    /// `parse_view_expr`'s dotted-path branch silently consuming trailing trivia via its own
    /// internal `skip_trivia()` call defeated a naive "peek the next char" boundary check.
    #[test]
    fn parses_multiple_closures_without_trailing_commas() {
        let src = r#"
view V {
    TabView {
        tabs: vm.documents
        key: |doc| std::rc::Rc::as_ptr(doc) as usize
        render_label: |doc| doc.file_name
        render_content: |doc| DocumentView { doc: doc }
        selected: vm.active_tab
    }
}
"#;
        let module = parse_module(src).expect("should parse");
        let Item::View(view) = &module.items[0] else { panic!("expected view") };
        let attr = |name: &str| view.root.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());

        assert!(matches!(attr("key"), Some(ViewExpr::Closure { .. })));
        assert!(matches!(attr("render_label"), Some(ViewExpr::Closure { body: ClosureBody::Expr(_), .. })));
        assert!(matches!(attr("render_content"), Some(ViewExpr::Closure { body: ClosureBody::Element(_), .. })));
        assert!(matches!(attr("selected"), Some(ViewExpr::Path(p)) if p == vec!["vm".to_string(), "active_tab".to_string()]));
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
