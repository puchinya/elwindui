//! Hand-written lexer-free recursive-descent parser for a `view! { .. }` macro body's own inner
//! syntax (element trees, control flow, closures, attached-property setters, ...). Field/attribute-
//! value expressions that aren't one of the DSL's own macro forms (`once!`, `command!`, `t!`) are
//! handed off to `syn` for real parsing. `component`/`viewmodel`/`enum`/`use` items are current-
//! syntax Rust items instead (`#[elwindui::component]` struct+impl, `#[elwindui::viewmodel]` mod,
//! `#[elwindui::dsl_enum]` enum, ordinary `use`), parsed by `component_frontend.rs`/`attr_frontend.rs`
//! via `syn` — this module only ever parses what a `view! { .. }` macro invocation's tokens contain.
//! See docs/specs/dsl_spec.md §1-14.

use crate::ast::*;

/// Parses the content that would appear inside `view Name { <this> }` — on_mount/on_unmount
/// blocks, `let`-bindings, then the root body — from a standalone string with no enclosing
/// `view Name { .. }` of its own (no target name, no wrapping braces). Used by
/// `component_frontend.rs` to parse a `view! { .. }`-typed struct field's macro tokens, which
/// arrive as exactly this content (`syn::Macro::tokens` excludes the delimiters). Appends a
/// synthetic trailing `}` since `parse_element_body`'s own loop always terminates by consuming one.
#[allow(clippy::type_complexity)]
pub fn parse_view_body(
    src: &str,
) -> Result<
    (
        Option<syn::Block>,
        Option<syn::Block>,
        Option<OnUpdateHook>,
        Vec<LetBinding>,
        ViewBody,
    ),
    String,
> {
    Parser::new(&format!("{src}\n}}")).parse_view_body_tail()
}

/// Parses a single field/attribute initializer expression from standalone text — the
/// `default = ...`/`expr = ...` right-hand side of a `#[prop(default = ...)]`/`#[computed(expr =
/// ...)]`-style field attribute (`attr_frontend::fields_from_item_struct`).
pub fn parse_initializer(src: &str) -> Result<Initializer, String> {
    // `src` here is a whole, self-contained attribute-token string
    // (`parse_name_value_tokens`'s `tokens.to_string()`) with nothing after it — so a bare
    // literal/expr default (`#[prop(default = 50)]`) would hit EOF before any terminator and fail
    // the plain-expression fallback's `take_balanced_until` lookup. Appending a synthetic
    // terminator gives that fallback a trailing character to find.
    Parser::new(&format!("{src}}}")).parse_initializer()
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser { src, pos: 0 }
    }

    /// Parses the comma-separated contents of `#[shortcut(...)]`, *not* including the surrounding
    /// parens (the caller already consumed the opening one and consumes the closing one). Each
    /// entry is either a bare string literal (`"Ctrl+S"`, a chord for every backend with no more
    /// specific entry), `scope: local`/`scope: global` (sets `ShortcutScope`, default `Global`), or
    /// `backend_name: "chord"` (a per-backend override, e.g. `winui3: "Ctrl+S"`) — see
    /// `ast::ElementNode::attribute_shortcuts`'s own doc comment. Called from
    /// `parse_element_body`'s attribute-prefix handling, *not* from any field-declaration parser —
    /// a shortcut is a per-usage-site annotation, not part of any field's own declaration.
    fn parse_shortcut_attr(
        &mut self,
    ) -> Result<(Vec<(Option<String>, String)>, ShortcutScope), String> {
        let mut chords = Vec::new();
        let mut scope = ShortcutScope::Global;
        loop {
            self.skip_trivia();
            match self.peek_char() {
                Some(')') | None => break,
                Some('"') => {
                    let literal = self.take_string_literal()?;
                    chords.push((None, literal.trim_matches('"').to_string()));
                }
                _ => {
                    let key = self.parse_ident()?;
                    self.skip_trivia();
                    self.expect_char(':')?;
                    self.skip_trivia();
                    if key == "scope" {
                        let value = self.parse_ident()?;
                        scope = match value.as_str() {
                            "global" => ShortcutScope::Global,
                            "local" => ShortcutScope::Local,
                            other => {
                                return Err(self.err(&format!(
                                    "unknown #[shortcut] scope `{other}` (expected `global` or `local`)"
                                )));
                            }
                        };
                    } else {
                        let literal = self.take_string_literal()?;
                        chords.push((Some(key), literal.trim_matches('"').to_string()));
                    }
                }
            }
            self.skip_trivia();
            if !self.eat_char(',') {
                break;
            }
        }
        if chords.is_empty() {
            return Err(self.err("#[shortcut(...)] needs at least one key combination"));
        }
        Ok((chords, scope))
    }

    fn parse_initializer(&mut self) -> Result<Initializer, String> {
        if self.eat_keyword_bang("bind") {
            return Err(self.err(
                "bind!(...) was removed; use normal `property: expression`, `once!(...)`, or `property <=> writable_target` in a view",
            ));
        }

        let expr_src = self.take_balanced_until(&[',', '}'])?;
        let expr = syn::parse_str::<syn::Expr>(expr_src.trim())
            .map_err(|e| format!("invalid initializer expr `{}`: {e}", expr_src.trim()))?;
        Ok(Initializer::Expr(expr))
    }

    /// The part of a `view! { <this> }` macro body — on_mount/on_unmount blocks, `let`-bindings,
    /// then the root body (attributes/attached/children). Wrapped by `parse_view_body` (above),
    /// which parses this same content standalone, with no `target`/wrapping braces of its own — see
    /// that function's doc comment.
    #[allow(clippy::type_complexity)]
    fn parse_view_body_tail(
        &mut self,
    ) -> Result<
        (
            Option<syn::Block>,
            Option<syn::Block>,
            Option<OnUpdateHook>,
            Vec<LetBinding>,
            ViewBody,
        ),
        String,
    > {
        let mut on_mount = None;
        let mut on_unmount = None;
        let mut on_update = None;
        loop {
            self.skip_trivia();
            if self.eat_keyword("on_mount") {
                self.skip_trivia();
                self.eat_char(':'); // docs/design/runtime/ui_tree_design.md's `on_mount: { .. }` — the `:` is optional sugar.
                self.skip_trivia();
                let block_src = self.take_block_src()?;
                on_mount = Some(
                    syn::parse_str::<syn::Block>(&block_src)
                        .map_err(|e| format!("invalid on_mount body: {e}"))?,
                );
            } else if self.eat_keyword("on_unmount") {
                self.skip_trivia();
                self.eat_char(':');
                self.skip_trivia();
                let block_src = self.take_block_src()?;
                on_unmount = Some(
                    syn::parse_str::<syn::Block>(&block_src)
                        .map_err(|e| format!("invalid on_unmount body: {e}"))?,
                );
            } else if self.eat_keyword("on_update") {
                self.skip_trivia();
                // Optional `(field, ...)` — bare `on_update { .. }`/`on_update: { .. }` watches any
                // `#[prop]`/`#[computed]`/`#[state]`/`#[environment(name)]` change instead (dsl_spec.md
                // §3).
                let fields = if self.eat_char('(') {
                    let mut names = Vec::new();
                    loop {
                        self.skip_trivia();
                        if self.peek_char() == Some(')') {
                            break;
                        }
                        names.push(self.parse_ident()?);
                        self.skip_trivia();
                        if !self.eat_char(',') {
                            break;
                        }
                    }
                    self.skip_trivia();
                    self.expect_char(')')?;
                    if names.is_empty() {
                        return Err(
                            self.err("on_update(...) needs at least one field name, or omit the parens for `on_update { .. }`")
                        );
                    }
                    Some(names)
                } else {
                    None
                };
                self.skip_trivia();
                self.eat_char(':');
                self.skip_trivia();
                let block_src = self.take_block_src()?;
                let block = syn::parse_str::<syn::Block>(&block_src)
                    .map_err(|e| format!("invalid on_update body: {e}"))?;
                if on_update.is_some() {
                    return Err(self.err("only one on_update block is supported per view"));
                }
                on_update = Some(OnUpdateHook { fields, block });
            } else {
                break;
            }
        }

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
                continue;
            }
            if id.is_some() {
                return Err(
                    self.err("#[id(\"...\")] must be immediately followed by a `let` binding")
                );
            }
            self.pos = checkpoint;
            break;
        }

        self.skip_trivia();
        let (attributes, attached, attribute_shortcuts, children) = self.parse_element_body()?;
        // `parse_element_body` already consumed the view's own closing `}` (mirroring
        // `parse_element_node`, which consumes `Type { ... }`'s own closing `}` the same way).
        Ok((
            on_mount,
            on_unmount,
            on_update,
            lets,
            ViewBody {
                attributes,
                attached,
                attribute_shortcuts,
                children,
            },
        ))
    }

    fn parse_element_node(&mut self) -> Result<ElementNode, String> {
        let type_path = self.parse_type_path()?;
        self.skip_trivia();
        self.expect_char('{')?;
        let (attributes, attached, attribute_shortcuts, children) = self.parse_element_body()?;

        Ok(ElementNode {
            type_path,
            attributes,
            attached,
            attribute_shortcuts,
            children,
        })
    }

    /// The part of an element's `{ ... }` body that follows the opening `{` — attribute/attached-
    /// property lines and bare/control-flow child entries, up to (and consuming) the matching `}`.
    /// Shared between `parse_element_node` (called after its own `type_path {`) and
    /// `parse_view_body_tail` (called after a `view! { .. }` macro's own leading on_mount/on_unmount/
    /// `let`-bindings, which — unlike an element — name no type of their own; see `ast::ViewBody`).
    #[allow(clippy::type_complexity)]
    fn parse_element_body(
        &mut self,
    ) -> Result<
        (
            Vec<ViewAttribute>,
            Vec<(String, String, ViewExpr)>,
            Vec<(String, Vec<(Option<String>, String)>, ShortcutScope)>,
            Vec<ChildEntry>,
        ),
        String,
    > {
        let mut attributes = Vec::new();
        let mut attached = Vec::new();
        let mut attribute_shortcuts = Vec::new();
        let mut children = Vec::new();

        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            if self.peek_keyword("if") || self.peek_keyword("match") || self.peek_keyword("for") {
                children.push(self.parse_control_child()?);
                self.skip_trivia();
                self.eat_char(',');
                continue;
            }
            // `#[shortcut(...)]` (docs/design/runtime/input_focus_design.md) — the only
            // attribute-prefix syntax an element body supports today (unlike `#[id("...")]`, which
            // only ever precedes a `let` binding, never an ordinary attribute line — see
            // `parse_view_body_tail`). Must be immediately followed by a plain `ident: value`
            // attribute line; it annotates that specific attribute's value, not a child or attached
            // property.
            let mut pending_shortcut = None;
            if self.peek_char() == Some('#') {
                self.eat_char('#');
                self.expect_char('[')?;
                let attr_name = self.parse_ident()?;
                if attr_name != "shortcut" {
                    return Err(self.err(&format!(
                        "unknown element attribute #[{attr_name}] (only #[shortcut(...)] is supported here)"
                    )));
                }
                self.expect_char('(')?;
                let (chords, scope) = self.parse_shortcut_attr()?;
                self.expect_char(')')?;
                self.expect_char(']')?;
                self.skip_trivia();
                pending_shortcut = Some((chords, scope));
            }
            if pending_shortcut.is_none() && self.looks_like_element_path() {
                children.push(ChildEntry::Literal(self.parse_element_node()?));
                self.skip_trivia();
                self.eat_char(',');
                continue;
            }
            let ident_start = self.pos;
            let ident = self.parse_ident()?;
            self.skip_trivia();
            if self.eat_str("::") {
                if pending_shortcut.is_some() {
                    return Err(self.err(
                        "#[shortcut(...)] must be immediately followed by a plain `attribute: value` line, not an attached property",
                    ));
                }
                // `Owner::field: value` — an attached-property setter (§3), checked *before* the
                // single-`:` attribute case below (`eat_str("::")` only matches the literal 2-char
                // sequence, so plain `ident:` is unaffected).
                self.skip_trivia();
                let field = self.parse_ident()?;
                self.skip_trivia();
                self.expect_char(':')?;
                self.skip_trivia();
                let value = self.parse_view_expr()?;
                attached.push((ident, field, value));
            } else if self.eat_str("<=>") {
                if pending_shortcut.is_some() {
                    return Err(self.err(
                        "#[shortcut(...)] can annotate only an `attribute: event` assignment, not `<=>`",
                    ));
                }
                self.skip_trivia();
                let value = self.parse_view_expr()?;
                attributes.push(ViewAttribute {
                    name: ident,
                    value,
                    kind: AssignmentKind::TwoWay,
                    span: self.source_span(ident_start, self.pos),
                });
            } else if self.eat_char(':') {
                self.skip_trivia();
                let (kind, value) = if self.eat_keyword_bang("once") {
                    self.expect_char('(')?;
                    let source = self.take_balanced_until(&[')'])?;
                    self.expect_char(')')?;
                    let mut parser = Parser::new(source.trim());
                    let value = parser.parse_view_expr()?;
                    parser.skip_trivia();
                    if !parser.at_eof() {
                        return Err(self.err("invalid once!(...) expression"));
                    }
                    (AssignmentKind::Once, value)
                } else {
                    (AssignmentKind::Normal, self.parse_view_expr()?)
                };
                if let Some((chords, scope)) = pending_shortcut {
                    attribute_shortcuts.push((ident.clone(), chords, scope));
                }
                attributes.push(ViewAttribute {
                    name: ident,
                    value,
                    kind,
                    span: self.source_span(ident_start, self.pos),
                });
            } else if self.peek_char() == Some('{') {
                if pending_shortcut.is_some() {
                    return Err(self.err(
                        "#[shortcut(...)] must be immediately followed by a plain `attribute: value` line, not a child element",
                    ));
                }
                // bare `Type { ... }`: this is a nested child element, `ident` was its type name.
                self.pos = ident_start;
                children.push(ChildEntry::Literal(self.parse_element_node()?));
            } else {
                if pending_shortcut.is_some() {
                    return Err(self.err(
                        "#[shortcut(...)] must be immediately followed by a plain `attribute: value` line, not a `let` reference",
                    ));
                }
                // bare identifier with neither `:` nor `{` following: a reference to an earlier
                // `#[id(...)]? let <ident> = ...;` binding (see `parse_view_body_tail`), e.g.
                // `Column { editor, StatusBar {} }`'s `editor`.
                children.push(ChildEntry::Ref(ident));
            }
            self.skip_trivia();
            self.eat_char(',');
        }

        Ok((attributes, attached, attribute_shortcuts, children))
    }

    fn source_span(&self, start: usize, end: usize) -> SourceSpan {
        let prefix = &self.src[..start.min(self.src.len())];
        let line = prefix.matches('\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let column = self.src[line_start..start]
            .chars()
            .count()
            .saturating_add(1);
        SourceSpan {
            start,
            end,
            line,
            column,
        }
    }

    fn parse_control_child(&mut self) -> Result<ChildEntry, String> {
        if self.eat_keyword("if") {
            let condition = self.parse_control_expr_until('{')?;
            let then_branch = self.parse_child_block()?;
            self.skip_trivia();
            let else_branch = if self.eat_keyword("else") {
                self.skip_trivia();
                if self.peek_keyword("if") {
                    vec![self.parse_control_child()?]
                } else {
                    self.parse_child_block()?
                }
            } else {
                Vec::new()
            };
            return Ok(ChildEntry::If {
                condition,
                then_branch,
                else_branch,
            });
        }
        if self.eat_keyword("for") {
            self.skip_trivia();
            let binding = self.parse_ident()?;
            self.skip_trivia();
            if !self.eat_keyword("in") {
                return Err(self.err("expected `in` in for child"));
            }
            let collection = self.parse_control_expr_until('{')?;
            let body = self.parse_child_block()?;
            return Ok(ChildEntry::For {
                binding,
                collection,
                body,
            });
        }
        if !self.eat_keyword("match") {
            return Err(self.err("expected control-flow child"));
        }
        let value = self.parse_control_expr_until('{')?;
        self.expect_char('{')?;
        let mut arms = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            let pattern = self.take_balanced_until(&['='])?.trim().to_string();
            self.expect_char('=')?;
            self.expect_char('>')?;
            self.skip_trivia();
            let body = if self.peek_char() == Some('{') {
                self.parse_child_block()?
            } else {
                vec![ChildEntry::Literal(self.parse_element_node()?)]
            };
            arms.push(MatchArm { pattern, body });
            self.skip_trivia();
            self.eat_char(',');
        }
        Ok(ChildEntry::Match { value, arms })
    }

    fn parse_child_block(&mut self) -> Result<Vec<ChildEntry>, String> {
        self.expect_char('{')?;
        let mut entries = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat_char('}') {
                break;
            }
            if self.peek_keyword("if") || self.peek_keyword("match") || self.peek_keyword("for") {
                entries.push(self.parse_control_child()?);
            } else {
                entries.push(ChildEntry::Literal(self.parse_element_node()?));
            }
            self.skip_trivia();
            self.eat_char(',');
        }
        Ok(entries)
    }

    fn parse_control_expr_until(&mut self, terminator: char) -> Result<ViewExpr, String> {
        let source = self.take_balanced_until(&[terminator])?;
        let mut parser = Parser::new(source.trim());
        let expr = parser.parse_view_expr()?;
        parser.skip_trivia();
        if !parser.at_eof() {
            return Err(self.err("invalid control-flow expression"));
        }
        Ok(expr)
    }

    fn parse_view_expr(&mut self) -> Result<ViewExpr, String> {
        self.skip_trivia();

        if self.peek_char() == Some('|') {
            return self.parse_closure();
        }

        if self.peek_char() == Some('"') {
            // A bare string literal is common enough (`text: "hello"`) to deserve its own fast
            // path, but a literal immediately followed by a method chain (`locale:
            // "ja-JP".to_string()`) is a larger expression, not just the literal — probing one
            // character ahead (after the literal, past any trivia) and rewinding to the whole
            // expression's start when a `.` follows avoids silently truncating at the closing
            // quote (CI-7 of #80 found this: `EnvironmentScope`'s override values are ordinary
            // `t!("locale", value: "ja-JP".to_string())`-style expressions and hit exactly this).
            let start = self.pos;
            self.take_string_literal()?;
            let after_literal = self.pos;
            self.skip_trivia();
            let continues_as_method_chain = self.peek_char() == Some('.');
            self.pos = start;
            if continues_as_method_chain {
                let expr_src = self.take_expr_until_line_end_or(&[',', '}'])?;
                let expr = syn::parse_str::<syn::Expr>(expr_src.trim())
                    .map_err(|e| format!("invalid expression `{}`: {e}", expr_src.trim()))?;
                return Ok(ViewExpr::Expr(expr));
            }
            self.pos = after_literal;
            let lit_src = self.src[start..after_literal].to_string();
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
            || (self.peek_char() == Some('-')
                && self.rest()[1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            let lit_src = self.take_number_literal()?;
            let expr = syn::parse_str::<syn::Expr>(&lit_src)
                .map_err(|e| format!("invalid number literal: {e}"))?;
            return Ok(ViewExpr::Expr(expr));
        }

        // `[GridLength::Auto, GridLength::Star(1.0), GridLength::Fixed(100.0)]` — an array literal
        // attribute value (`Grid`'s `rows`/`columns`, §3). Captured verbatim and handed to `syn`
        // directly (same take-then-`syn::parse_str` fallback `parse_initializer` already uses for
        // a field's default expr) rather than taught to this function's own dotted-path/`t!` sugar,
        // since a bracketed literal can't be confused with any of those. Unlike a general
        // expression (`take_expr_until_line_end_or`, relied on by `parse_closure_expr_body` and
        // needed there because an arbitrary expression has no self-delimiting end marker), an
        // array literal *is* self-delimiting — its own matching `]` unambiguously ends it — so
        // `take_bracketed_src` stops exactly there regardless of trailing separators or
        // whitespace/newlines. This matters beyond style: relying on an unnested newline to end
        // the capture (as this used to) silently breaks whenever this DSL text didn't come from a
        // real DSL module with real line breaks but from a macro's `TokenStream::to_string()`
        // (`elwindui::component!`'s removed bang-macro form, or `view!`'s tokens today) —
        // `to_string()` never preserves original source line breaks, so the "stop at newline"
        // fallback would keep consuming every subsequent attribute/child until the next stray
        // `,`/`}`, exactly the class of bug `eat_keyword_bang` fixed for `t!`/`once!`/`command!`.
        if self.peek_char() == Some('[') {
            let expr_src = self.take_bracketed_src()?;
            let expr = syn::parse_str::<syn::Expr>(expr_src.trim())
                .map_err(|e| format!("invalid array literal `{}`: {e}", expr_src.trim()))?;
            return Ok(ViewExpr::Expr(expr));
        }

        // Bare `Type { .. }` as an ordinary (non-closure) attribute value — a builtin shape's
        // "named single-child slot" (e.g. `Window`'s `menu_bar: MenuBar { .. }`), generalizing the
        // same shape `ClosureBody::Element` already uses inside `|param| Type { .. }` bodies.
        if self.looks_like_element() {
            let element = self.parse_element_node()?;
            return Ok(ViewExpr::Element(Box::new(element)));
        }

        // A `::`-qualified path (`elwindui_core::ui::ShapeKind::RoundedRect { corner_radius: .. }`,
        // an enum struct/tuple-variant construction, or any other multi-segment Rust path) — none
        // of this parser's other sugars understand `::` (the dotted-path fallback below only
        // consumes `.`-separated segments, for `vm.content`-style bind references), so hand the raw
        // text to `syn` directly instead, the same fallback the array-literal case above uses.
        // Detected via lookahead (parse one identifier, see if `::` immediately follows) so a plain
        // bind reference or bare identifier is never mistaken for one.
        if self.looks_like_qualified_path() {
            let expr_src = self.take_expr_until_line_end_or(&[',', '}'])?;
            let expr = syn::parse_str::<syn::Expr>(expr_src.trim())
                .map_err(|e| format!("invalid expression `{}`: {e}", expr_src.trim()))?;
            return Ok(ViewExpr::Expr(expr));
        }

        // `context_popup: view! { .. }` — a nested deferred view, parsed with exactly the same
        // full-body grammar (`parse_view_body_tail`) an ordinary top-level `view!` uses, not a
        // second, narrower grammar. Checked before the generic macro-call/`syn::Expr` fallbacks
        // below (`looks_like_macro_call` would otherwise swallow `view!{..}`'s tokens as an opaque
        // `syn::Expr::Macro`, losing its structure entirely) — see docs/design/runtime/
        // view_template_design.md §3, Issue #162.
        if self.eat_keyword_bang("view") {
            self.skip_trivia();
            self.expect_char('{')?;
            let (on_mount, on_unmount, on_update, lets, root) = self.parse_view_body_tail()?;
            return Ok(ViewExpr::DeferredView(Box::new(DeferredViewExpr {
                body: DeferredViewBody {
                    on_mount,
                    on_unmount,
                    on_update,
                    lets,
                    root,
                },
                hidden_component: None,
                lexical_owner: None,
            })));
        }

        if self.eat_keyword_bang("t") {
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

        if self.eat_keyword_bang("bind") {
            return Err(self.err(
                "bind!(...) was removed; use normal `property: expression`, `once!(...)`, or `property <=> writable_target`",
            ));
        }

        // Other Rust-style expression macros are self-delimiting by their token group. Preserve
        // the complete invocation as `syn::Expr::Macro`; validation decides whether its arguments
        // can be analyzed reactively or require an outer `once!(...)`.
        if self.looks_like_macro_call() {
            let start = self.pos;
            self.parse_ident()?;
            self.skip_trivia();
            self.expect_char('!')?;
            self.skip_trivia();
            let open = self
                .peek_char()
                .ok_or_else(|| self.err("expected macro delimiter"))?;
            let close = match open {
                '(' => ')',
                '[' => ']',
                '{' => '}',
                _ => return Err(self.err("expected `(`, `[`, or `{` after macro name")),
            };
            self.expect_char(open)?;
            self.take_balanced_until(&[close])?;
            self.expect_char(close)?;
            let source = &self.src[start..self.pos];
            let expr = syn::parse_str::<syn::Expr>(source)
                .map_err(|error| format!("invalid macro expression `{source}`: {error}"))?;
            return Ok(ViewExpr::Expr(expr));
        }

        // Dotted field path. `()` is no longer special-cased here — a trailing call like
        // `vm.close_tab(index)` only ever appears inside a closure body (`parse_closure_expr_body`
        // falls back to `syn::Expr` for it), never as a bare top-level attribute value.
        let mut path = vec![self.parse_ident()?];
        while self.eat_str(".") {
            path.push(self.parse_ident()?);
        }
        Ok(ViewExpr::Path(path))
    }

    /// `|| <body>` / `|index| <body>` / `|a, b| <body>` — used both by 付録Y's `key`/
    /// `render_label`/`render_content` attributes and, more generally, by any `on_*` event
    /// attribute that needs to name its callback's arguments (`codegen::emit_wiring`). Zero or
    /// more untyped bound parameters (no destructuring, no `: Type` — the real types come
    /// positionally from the target field's own `fn(T0, T1, ...)` declaration); the body is a
    /// nested element construction (`render_content: |doc| DocumentView { doc: doc }`), a brace-
    /// delimited Rust block (`on_close: |index| { vm.log(index); vm.close_tab(index) }`), or a
    /// plain expression (`key`/`render_label`/`on_select: |index| vm.select_tab(index)`).
    fn parse_closure(&mut self) -> Result<ViewExpr, String> {
        self.expect_char('|')?;
        self.skip_trivia();
        let mut params = Vec::new();
        if self.peek_char() != Some('|') {
            loop {
                params.push(self.parse_ident()?);
                self.skip_trivia();
                if !self.eat_char(',') {
                    break;
                }
                self.skip_trivia();
            }
        }
        self.skip_trivia();
        self.expect_char('|')?;
        self.skip_trivia();

        if self.peek_char() == Some('{') {
            let block_src = self.take_block_src()?;
            let block = syn::parse_str::<syn::Block>(&block_src)
                .map_err(|e| format!("invalid closure block body: {e}"))?;
            return Ok(ViewExpr::Closure {
                params,
                body: ClosureBody::Block(block),
            });
        }

        if self.looks_like_element() {
            let element = self.parse_element_node()?;
            return Ok(ViewExpr::Closure {
                params,
                body: ClosureBody::Element(Box::new(element)),
            });
        }

        let body = self.parse_closure_expr_body()?;
        Ok(ViewExpr::Closure {
            params,
            body: ClosureBody::Expr(Box::new(body)),
        })
    }

    /// Lookahead-and-rewind (same idiom `parse_element_node` uses at its attribute/child-element
    /// split) to tell a bare `Type { ... }` (an element construction) apart from a plain
    /// expression, without consuming anything.
    fn looks_like_element(&mut self) -> bool {
        let save = self.pos;
        let is_type_name = self
            .parse_ident()
            .map(|path| path.chars().next().is_some_and(|c| c.is_uppercase()))
            .unwrap_or(false);
        self.skip_trivia();
        let followed_by_brace = self.peek_char() == Some('{');
        self.pos = save;
        is_type_name && followed_by_brace
    }

    /// The element-body variant of [`looks_like_element`]. A qualified path is allowed in a child
    /// position, while `parse_view_expr` keeps the bare-name check so Rust enum/struct expressions
    /// such as `ShapeKind::RoundedRect { .. }` remain expressions rather than being mistaken for
    /// DSL elements.
    fn looks_like_element_path(&mut self) -> bool {
        let save = self.pos;
        let is_type_name = self
            .parse_type_path()
            .map(|path| {
                path.rsplit("::")
                    .next()
                    .and_then(|name| name.chars().next())
                    .is_some_and(|c| c.is_uppercase())
            })
            .unwrap_or(false);
        self.skip_trivia();
        let followed_by_brace = self.peek_char() == Some('{');
        self.pos = save;
        is_type_name && followed_by_brace
    }

    /// Parses the type-shaped path that starts a DSL element.  The parser keeps this as text in
    /// the AST because the same path is later handed to Rust code generation unchanged for a
    /// qualified external component (`some_crate::Widget`).  Attached-property syntax is still
    /// parsed separately by `parse_element_body`: it only calls `parse_ident()` for the owner and
    /// therefore continues to recognize `Grid::row: value` as an attached assignment.
    fn parse_type_path(&mut self) -> Result<String, String> {
        let mut path = self.parse_ident()?;
        loop {
            self.skip_trivia();
            if !self.eat_str("::") {
                break;
            }
            path.push_str("::");
            path.push_str(&self.parse_ident()?);
        }
        Ok(path)
    }

    fn looks_like_macro_call(&mut self) -> bool {
        let checkpoint = self.pos;
        let result = self.parse_ident().is_ok() && {
            self.skip_trivia();
            self.peek_char() == Some('!')
        };
        self.pos = checkpoint;
        result
    }

    /// Lookahead-and-rewind (same idiom as `looks_like_element`) for a `::`-qualified path value —
    /// parses one identifier and checks whether `::` immediately follows, without consuming
    /// anything. See `parse_view_expr`'s own doc comment on why this needs its own sugar.
    fn looks_like_qualified_path(&mut self) -> bool {
        let save = self.pos;
        let ok = self.parse_ident().is_ok() && {
            self.skip_trivia();
            self.eat_str("::")
        };
        self.pos = save;
        ok
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
            if after
                .chars()
                .next()
                .map(|c| !c.is_alphanumeric() && c != '_')
                .unwrap_or(true)
            {
                self.pos += kw.len();
                return true;
            }
        }
        false
    }

    fn peek_keyword(&mut self, kw: &str) -> bool {
        let checkpoint = self.pos;
        let matched = self.eat_keyword(kw);
        self.pos = checkpoint;
        matched
    }

    /// Like `eat_keyword`, but for the DSL's own `once!(..)`/`command!(..)`/`t!(..)` macro-call
    /// sugar forms: consumes `kw` followed by `!`, tolerating whitespace in between. Real rustc's
    /// `proc_macro::TokenStream::to_string()` never puts a space between an identifier and an
    /// immediately-following `!`, but rust-analyzer's own proc-macro-srv token-stream-to-text
    /// implementation does (`foo !` rather than `foo!`) — confirmed via `rust-analyzer diagnostics`
    /// producing a real `macro-error` here for a `view!`/`component!`-style macro's tokens read
    /// back out as DSL text, while `cargo build`/`cargo test` (real rustc) stayed clean. Assuming
    /// the two always agree byte-for-byte is exactly the kind of thing CLAUDE.md's "verify with
    /// rust-analyzer" step exists to catch.
    fn eat_keyword_bang(&mut self, kw: &str) -> bool {
        let checkpoint = self.pos;
        if self.eat_keyword(kw) && self.eat_char('!') {
            true
        } else {
            self.pos = checkpoint;
            false
        }
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

    /// Captures a full brace-delimited block (`{ ... }`, braces included), respecting nested
    /// braces/parens/brackets and string literals — for a method/`on_mount`/`on_unmount` body
    /// handed to `syn::parse_str::<syn::Block>` (which requires the surrounding braces).
    fn take_block_src(&mut self) -> Result<String, String> {
        self.skip_trivia();
        let start = self.pos;
        if self.peek_char() != Some('{') {
            return Err(self.err("expected `{`"));
        }
        let mut depth: i32 = 0;
        loop {
            match self.peek_char() {
                None => return Err(self.err("unexpected end of input in block")),
                Some('"') => {
                    self.take_string_literal()?;
                    continue;
                }
                Some('{') | Some('(') | Some('[') => {
                    depth += 1;
                    self.pos += 1;
                }
                Some('}') | Some(')') | Some(']') => {
                    depth -= 1;
                    self.pos += 1;
                    if depth == 0 {
                        break;
                    }
                }
                Some(c) => self.pos += c.len_utf8(),
            }
        }
        Ok(self.src[start..self.pos].to_string())
    }

    /// Captures a full bracket-delimited literal (`[ ... ]`, brackets included), respecting nested
    /// brackets/parens/braces and string literals — mirrors `take_block_src`'s `{ ... }` capture,
    /// just for `[`/`]`. Used for array-literal attribute values (`rows: [GridLength::Auto, ..]`),
    /// which — unlike a general expression — are self-delimiting by their own matching bracket, so
    /// this needs no separator/newline convention to know where the value ends.
    fn take_bracketed_src(&mut self) -> Result<String, String> {
        self.skip_trivia();
        let start = self.pos;
        if self.peek_char() != Some('[') {
            return Err(self.err("expected `[`"));
        }
        let mut depth: i32 = 0;
        loop {
            match self.peek_char() {
                None => return Err(self.err("unexpected end of input in array literal")),
                Some('"') => {
                    self.take_string_literal()?;
                    continue;
                }
                Some('[') | Some('(') | Some('{') => {
                    depth += 1;
                    self.pos += 1;
                }
                Some(']') | Some(')') | Some('}') => {
                    depth -= 1;
                    self.pos += 1;
                    if depth == 0 {
                        break;
                    }
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
    ///
    /// The newline check alone isn't enough when this text came from `view!`-macro token recovery
    /// (`TokenStream::to_string()`, `component_frontend.rs`) rather than a real DSL module —
    /// `to_string()` never preserves original source line breaks, and doesn't reliably insert space
    /// between every adjacent token pair either (`take_bracketed_src`'s own doc comment notes the
    /// same line-break gap for array literals, fixed there by that value being self-delimiting; a
    /// closure body has no such delimiter of its own). So at every unnested position this also
    /// checks whether what's ahead looks like the *next* attribute starting — a bare identifier
    /// immediately followed by `:` (not `::`), or a `#[` shortcut-attribute prefix — exactly the
    /// grammar `parse_element_node`'s own attribute loop uses to begin one, checked regardless of
    /// whether any whitespace actually separates it from the expression text so far.
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
                Some(_) if depth == 0 && self.looks_like_next_attribute_ahead() => break,
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

    /// Lookahead-and-rewind (same idiom as `looks_like_element`), called at every unnested position
    /// inside a closure body's raw-text capture: skips the trivia and checks whether what follows is
    /// the start of a new view attribute (`ident:`, not `::`) or a `#[shortcut]`-style attribute,
    /// without consuming anything either way.
    fn looks_like_next_attribute_ahead(&mut self) -> bool {
        let save = self.pos;
        self.skip_trivia();
        // Tolerate whitespace between `#` and `[`: `view!` tokens recovered via
        // `TokenStream::to_string()` (`component_frontend.rs`) always render `#[shortcut(...)]`
        // as two separate tokens with an inserted space (`# [shortcut (...)]`), never the literal
        // `#[` a hand-typed `.elwind`/DSL-text source would have — a plain `peek_str("#[")` misses
        // that spelling entirely and lets this attribute's tokens get swallowed into the previous
        // attribute's raw-text capture (`take_expr_until_line_end_or`) instead.
        let looks_like_attribute = (self.eat_char('#') && self.eat_char('[')) || {
            self.parse_ident().is_ok() && {
                self.skip_trivia();
                self.peek_char() == Some(':') && !self.peek_str("::")
            }
        };
        self.pos = save;
        looks_like_attribute
    }

    fn err(&self, msg: &str) -> String {
        let line = self.src[..self.pos].matches('\n').count() + 1;
        let pos = self.pos.min(self.src.len());
        let mut snippet_start = pos.saturating_sub(30);
        while snippet_start > 0 && !self.src.is_char_boundary(snippet_start) {
            snippet_start -= 1;
        }
        let mut snippet_end = (pos + 30).min(self.src.len());
        while snippet_end < self.src.len() && !self.src.is_char_boundary(snippet_end) {
            snippet_end += 1;
        }
        let before = &self.src[snippet_start..pos];
        let after = &self.src[pos..snippet_end];
        format!("parse error at line {line}: {msg} (near: {before:?} <|> {after:?})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dynamic_if_match_and_for_children() {
        let (_, _, _, _, root) = parse_view_body(
            r#"
                VerticalLayout {
                    if vm.visible { TextBlock { text: "yes" } } else { TextBlock { text: "no" } }
                    match vm.status {
                        Status::Ready => { TextBlock { text: "ready" } }
                        _ => { TextBlock { text: "other" } }
                    }
                    for item in vm.items { TextBlock { text: "item" } }
                }
            "#,
        )
        .expect("dynamic control-flow source should parse");
        let root = literal(&root.children[0]);
        assert_eq!(root.type_path, "VerticalLayout");
        assert!(matches!(root.children[0], ChildEntry::If { .. }));
        assert!(matches!(root.children[1], ChildEntry::Match { .. }));
        assert!(matches!(root.children[2], ChildEntry::For { .. }));
    }

    /// Issue #162 T1: a raw-`UIElement`-rooted `context_popup: view! { .. }` parses as
    /// `ViewExpr::DeferredView`, reusing `parse_view_body_tail`'s full grammar rather than a
    /// single-element-only sugar.
    #[test]
    fn parses_deferred_view_attribute_with_raw_root() {
        let (_, _, _, _, root) = parse_view_body(
            r#"
                TextBlock {
                    text: "Open popup",
                    context_popup: view! {
                        VerticalLayout {
                            TextBlock { text: "Popup" }
                        }
                    },
                }
            "#,
        )
        .expect("deferred view attribute should parse");
        let root = literal(&root.children[0]);
        let attr = root
            .attributes
            .iter()
            .find(|a| a.name == "context_popup")
            .expect("context_popup attribute should be present");
        let ViewExpr::DeferredView(deferred) = &attr.value else {
            panic!(
                "context_popup value should parse as ViewExpr::DeferredView, got {:?}",
                attr.value
            );
        };
        assert_eq!(deferred.hidden_component, None);
        assert!(deferred.body.on_mount.is_none());
        assert!(deferred.body.lets.is_empty());
        let deferred_root = literal(&deferred.body.root.children[0]);
        assert_eq!(deferred_root.type_path, "VerticalLayout");
    }

    /// Issue #162 T2: the full existing `view!` body grammar (`on_mount`/`on_unmount`/`on_update`/
    /// `#[id]` lets/`if`) is accepted inside a nested `context_popup: view! { .. }`, not just a
    /// bare element construction.
    #[test]
    fn parses_deferred_view_attribute_with_full_body() {
        let (_, _, _, _, root) = parse_view_body(
            r#"
                TextBlock {
                    context_popup: view! {
                        on_mount { record("mount"); }
                        on_unmount { record("unmount"); }
                        on_update(selected_item) { record("update"); }

                        #[id("title")]
                        let title = TextBlock { text: selected_item };

                        VerticalLayout {
                            title

                            if show_extra {
                                TextBlock { text: "extra" }
                            }
                        }
                    },
                }
            "#,
        )
        .expect("full deferred view body should parse");
        let root = literal(&root.children[0]);
        let attr = root
            .attributes
            .iter()
            .find(|a| a.name == "context_popup")
            .expect("context_popup attribute should be present");
        let ViewExpr::DeferredView(deferred) = &attr.value else {
            panic!("expected ViewExpr::DeferredView, got {:?}", attr.value);
        };
        assert!(deferred.body.on_mount.is_some());
        assert!(deferred.body.on_unmount.is_some());
        let on_update = deferred
            .body
            .on_update
            .as_ref()
            .expect("on_update should parse");
        assert_eq!(
            on_update.fields.as_deref(),
            Some(["selected_item".to_string()].as_slice())
        );
        assert_eq!(deferred.body.lets.len(), 1);
        assert_eq!(deferred.body.lets[0].id.as_deref(), Some("title"));
        assert_eq!(deferred.body.lets[0].name, "title");
        let deferred_root = literal(&deferred.body.root.children[0]);
        assert_eq!(deferred_root.type_path, "VerticalLayout");
        assert!(matches!(deferred_root.children[0], ChildEntry::Ref(ref n) if n == "title"));
        assert!(matches!(deferred_root.children[1], ChildEntry::If { .. }));
    }

    #[test]
    fn parses_notepad_viewmodel() {
        let item_enum: syn::ItemEnum =
            syn::parse_str("enum SaveState { Unsaved, Saving, Saved }").expect("enum should parse");
        let enum_def = crate::component_frontend::enum_def_from_item_enum(&item_enum)
            .expect("enum should build");
        assert_eq!(enum_def.name, "SaveState");
        assert_eq!(enum_def.variants, vec!["Unsaved", "Saving", "Saved"]);

        let item_mod: syn::ItemMod = syn::parse_str(
            r#"
            mod notepad_view_model_mod {
                struct NotepadViewModel {
                    #[observable(default = String::new())]
                    #[length(0..=100000)]
                    content: String,

                    #[observable(default = "untitled.txt")]
                    file_name: String,

                    #[observable(default = SaveState::Unsaved)]
                    state: SaveState,

                    #[computed(expr = content.chars().count() as i32)]
                    char_count: i32,

                    #[computed(expr = t!("notepad-window-title", file_name: file_name))]
                    window_title: String,

                    #[computed(expr = state != SaveState::Saving)]
                    save_can_execute: bool,
                }
            }
            "#,
        )
        .expect("mod should parse");
        let vm = crate::attr_frontend::viewmodel_def_from_item_mod(&item_mod)
            .expect("viewmodel should build");
        assert_eq!(vm.name, "NotepadViewModel");
        assert_eq!(vm.fields.len(), 6);

        assert_eq!(vm.fields[0].name, "content");
        assert_eq!(vm.fields[0].kind, FieldKind::Observable);
        assert!(matches!(
            vm.fields[0].attrs.as_slice(),
            [Attr::Length {
                start: 0,
                end: 100000,
                inclusive: true
            }]
        ));

        assert_eq!(vm.fields[3].name, "char_count");
        assert_eq!(vm.fields[3].kind, FieldKind::Computed);
        assert!(matches!(
            vm.fields[3].initializer,
            Some(Initializer::Expr(_))
        ));

        assert_eq!(vm.fields[4].name, "window_title");
        assert!(matches!(
            vm.fields[4].initializer,
            Some(Initializer::Expr(_))
        ));
    }

    #[test]
    fn parses_notepad_window() {
        // The old DSL text form's own top-level `use` declaration (§12) has no counterpart on this
        // (real, production) frontend — an ordinary Rust `use` in the surrounding source file is
        // already resolved by `rustc` itself, with no DSL-side parsing involved at all.
        let src = r#"
        struct NotepadWindow {
            #[param]
            #[inject]
            vm: NotepadViewModel,

            body: view! {
                Window {
                    title: vm.window_title

                    Column {
                        Row {
                            Button {
                                text: t!("notepad-menu-save")
                                on_click: vm.save
                                enabled: vm.save_can_execute
                            }
                            Button {
                                text: t!("notepad-menu-open")
                                on_click: vm.open
                            }
                        }

                        TextArea { text <=> vm.content }

                        Row {
                            Text { text: t!("notepad-status-chars", count: vm.char_count) }
                        }
                    }
                }
            },
        }
        "#;
        let item_struct: syn::ItemStruct = syn::parse_str(src).expect("struct should parse");
        let (component, view) =
            crate::component_frontend::component_and_view_from_item_struct(None, &item_struct)
                .expect("should build");
        assert_eq!(component.name, "NotepadWindow");
        assert_eq!(component.fields.len(), 1);
        assert_eq!(component.fields[0].name, "vm");
        assert_eq!(component.fields[0].kind, FieldKind::Param);
        assert!(component.fields[0].initializer.is_none());

        let view = view.expect("view should be present");
        assert_eq!(view.target, "NotepadWindow");
        assert_eq!(view.root.children.len(), 1);
        let root = literal(&view.root.children[0]);
        assert_eq!(root.type_path, "Window");
        assert_eq!(root.children.len(), 1);

        let column = literal(&root.children[0]);
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
            .find(|attribute| attribute.name == "on_click")
            .map(|attribute| &attribute.value)
            .unwrap();
        assert!(matches!(on_click, ViewExpr::Path(path)
            if path == &vec!["vm".to_string(), "save".to_string()]));
    }

    /// Unwraps a test fixture's `ChildEntry`, which is always a literal nested element (none of
    /// these fixtures reference a `let`-bound name).
    fn literal(entry: &ChildEntry) -> &ElementNode {
        match entry {
            ChildEntry::Literal(elem) => elem,
            ChildEntry::Ref(name) => {
                panic!("expected a literal child element, found a `let`-ref to `{name}`")
            }
            ChildEntry::If { .. } | ChildEntry::Match { .. } | ChildEntry::For { .. } => {
                panic!("expected a literal child element, found a control-flow region")
            }
        }
    }

    fn parse_closure_attr(attr_src: &str) -> ViewExpr {
        let src = format!("TabView {{ {attr_src} }}");
        let (_, _, _, _, root_body) = parse_view_body(&src).expect("should parse");
        let root = literal(&root_body.children[0]);
        let expr = root
            .attributes
            .iter()
            .find(|attribute| attribute.name == "x")
            .expect("attribute `x`")
            .value
            .clone();
        expr
    }

    #[test]
    fn parses_closure_with_dotted_path_body() {
        let expr = parse_closure_attr("x: |doc| doc.file_name");
        let ViewExpr::Closure { params, body } = expr else {
            panic!("expected closure, got {expr:?}")
        };
        assert_eq!(params, vec!["doc".to_string()]);
        let ClosureBody::Expr(inner) = body else {
            panic!("expected expr body")
        };
        assert!(
            matches!(*inner, ViewExpr::Path(p) if p == vec!["doc".to_string(), "file_name".to_string()])
        );
    }

    #[test]
    fn parses_closure_with_syn_fallback_body() {
        let expr = parse_closure_attr("x: |doc| std::rc::Rc::as_ptr(doc) as usize");
        let ViewExpr::Closure { params, body } = expr else {
            panic!("expected closure, got {expr:?}")
        };
        assert_eq!(params, vec!["doc".to_string()]);
        let ClosureBody::Expr(inner) = body else {
            panic!("expected expr body")
        };
        assert!(
            matches!(*inner, ViewExpr::Expr(_)),
            "expected a raw syn::Expr fallback, got {inner:?}"
        );
    }

    #[test]
    fn parses_closure_with_element_body() {
        let expr = parse_closure_attr("x: |doc| DocumentView { doc: doc }");
        let ViewExpr::Closure { params, body } = expr else {
            panic!("expected closure, got {expr:?}")
        };
        assert_eq!(params, vec!["doc".to_string()]);
        let ClosureBody::Element(elem) = body else {
            panic!("expected element body")
        };
        assert_eq!(elem.type_path, "DocumentView");
        assert_eq!(elem.attributes.len(), 1);
        assert_eq!(elem.attributes[0].name, "doc");
        assert!(
            matches!(&elem.attributes[0].value, ViewExpr::Path(p) if p == &vec!["doc".to_string()])
        );
    }

    #[test]
    fn parses_qualified_external_element_as_a_child() {
        let (_, _, _, _, root) =
            parse_view_body("Host { external_widgets::ExternalWidget { title: \"hello\" } }")
                .expect("qualified external child should parse");
        let host = literal(&root.children[0]);
        let ChildEntry::Literal(child) = &host.children[0] else {
            panic!("qualified child should be a literal element");
        };
        assert_eq!(child.type_path, "external_widgets::ExternalWidget");
    }

    #[test]
    fn parses_zero_param_closure() {
        let expr = parse_closure_attr("x: || vm.save");
        let ViewExpr::Closure { params, body } = expr else {
            panic!("expected closure, got {expr:?}")
        };
        assert!(params.is_empty());
        let ClosureBody::Expr(inner) = body else {
            panic!("expected expr body")
        };
        assert!(
            matches!(*inner, ViewExpr::Path(p) if p == vec!["vm".to_string(), "save".to_string()])
        );
    }

    #[test]
    fn parses_typed_attribute_assignments_and_source_spans() {
        let (_, _, _, _, root) = parse_view_body(
            r#"
TextBox {
    text: "initial"
    placeholder: once!(format!("snapshot"))
    text <=> query
}
"#,
        )
        .expect("assignment forms should parse");
        let root = literal(&root.children[0]);
        assert_eq!(root.attributes[0].kind, AssignmentKind::Normal);
        assert_eq!(root.attributes[1].kind, AssignmentKind::Once);
        assert_eq!(root.attributes[2].kind, AssignmentKind::TwoWay);
        assert_eq!(root.attributes[0].span.line, 3);
        assert_eq!(root.attributes[0].span.column, 5);
        assert!(root.attributes[0].span.end > root.attributes[0].span.start);
    }

    #[test]
    fn rejects_removed_bind_macro_in_view_attributes() {
        let error = parse_view_body("TextArea { text: bind!(vm.content, TwoWay) }")
            .expect_err("removed bind syntax must be rejected");
        assert!(error.contains("bind!(...) was removed"), "{error}");
    }

    // CI-7 of #80 residual-risk follow-up: `parse_view_expr` used to silently truncate a string
    // literal directly followed by a method chain (e.g. `"ja-JP".to_string()`) at the closing
    // quote, discovered while implementing `EnvironmentScope`'s override-value codegen (worked
    // around there via `.into()`, which sidesteps but does not fix the parser bug for DSL
    // attribute values in general — see `emit_environment_scope_construction`'s doc comment).
    #[test]
    fn a_string_literal_followed_by_a_method_chain_parses_as_one_expression() {
        let (_, _, _, _, root) = parse_view_body(
            r#"TextBlock { text: once!("hello-world".replace("-", " ").to_uppercase()) }"#,
        )
        .expect("string literal + method chain should parse as one expression");
        let root = literal(&root.children[0]);
        let ViewExpr::Expr(expr) = &root.attributes[0].value else {
            panic!(
                "expected a plain expression, got {:?}",
                root.attributes[0].value
            );
        };
        let rendered = quote::quote!(#expr).to_string();
        assert!(
            rendered.contains("replace") && rendered.contains("to_uppercase"),
            "the whole method chain must be captured, not just the leading string literal: {rendered}"
        );
    }

    #[test]
    fn a_bare_string_literal_with_no_trailing_method_chain_still_parses() {
        let (_, _, _, _, root) = parse_view_body(r#"TextBlock { text: "hello" }"#)
            .expect("a plain string literal must still parse on its own");
        let root = literal(&root.children[0]);
        let ViewExpr::Expr(expr) = &root.attributes[0].value else {
            panic!(
                "expected a plain expression, got {:?}",
                root.attributes[0].value
            );
        };
        let rendered = quote::quote!(#expr).to_string();
        assert_eq!(rendered, "\"hello\"");
    }

    #[test]
    fn parses_multi_param_closure_with_block_body() {
        let expr = parse_closure_attr("x: |a, b| { vm.log(a); vm.close_tab(b) }");
        let ViewExpr::Closure { params, body } = expr else {
            panic!("expected closure, got {expr:?}")
        };
        assert_eq!(params, vec!["a".to_string(), "b".to_string()]);
        let ClosureBody::Block(block) = body else {
            panic!("expected block body, got {body:?}")
        };
        assert_eq!(block.stmts.len(), 2);
    }

    /// Multiple closure-bearing attributes with no trailing commas, one per line — the DSL's own
    /// convention (`parse_element_node`'s `,` is optional) — must each stop at the right boundary
    /// rather than swallowing the next attribute's text. Regression test for the bug where
    /// `parse_view_expr`'s dotted-path branch silently consuming trailing trivia via its own
    /// internal `skip_trivia()` call defeated a naive "peek the next char" boundary check.
    #[test]
    fn parses_multiple_closures_without_trailing_commas() {
        let src = r#"
TabView {
    tabs: vm.documents
    key: |doc| std::rc::Rc::as_ptr(doc) as usize
    render_label: |doc| doc.file_name
    render_content: |doc| DocumentView { doc: doc }
    selected: vm.active_tab
}
"#;
        let (_, _, _, _, root_body) = parse_view_body(src).expect("should parse");
        let root = literal(&root_body.children[0]);
        let attr = |name: &str| {
            root.attributes
                .iter()
                .find(|attribute| attribute.name == name)
                .map(|attribute| attribute.value.clone())
        };

        assert!(matches!(attr("key"), Some(ViewExpr::Closure { .. })));
        assert!(matches!(
            attr("render_label"),
            Some(ViewExpr::Closure {
                body: ClosureBody::Expr(_),
                ..
            })
        ));
        assert!(matches!(
            attr("render_content"),
            Some(ViewExpr::Closure {
                body: ClosureBody::Element(_),
                ..
            })
        ));
        assert!(
            matches!(attr("selected"), Some(ViewExpr::Path(p)) if p == vec!["vm".to_string(), "active_tab".to_string()])
        );
    }

    #[test]
    fn parses_virtual_and_override_methods() {
        let module = crate::test_module(&[
            (
                None,
                r#"struct Control { #[param] padding: Option<f32>, }"#,
                Some(
                    r#"
                    impl Control {
                        #[overridable]
                        fn label(&self) -> String {
                            "control".to_string()
                        }
                    }
                    "#,
                ),
            ),
            (
                Some("Control"),
                r#"struct ContentControl { #[param] content: std::rc::Rc<dyn UIElement>, }"#,
                Some(
                    r#"
                    impl ContentControl {
                        #[overrides]
                        fn label(&self, suffix: i32) -> String {
                            format!("{}!{}", base::label(), suffix)
                        }
                    }
                    "#,
                ),
            ),
        ])
        .expect("should parse");
        let Item::Component(control) = &module.items[0] else {
            panic!("expected component")
        };
        assert_eq!(control.fields.len(), 1);
        assert_eq!(control.methods.len(), 1);
        assert_eq!(control.methods[0].name, "label");
        assert!(control.methods[0].is_virtual);
        assert!(!control.methods[0].is_override);
        assert!(control.methods[0].params.is_empty());

        let Item::Component(content_control) = &module.items[1] else {
            panic!("expected component")
        };
        assert_eq!(content_control.fields.len(), 1);
        assert_eq!(content_control.methods.len(), 1);
        assert_eq!(content_control.methods[0].name, "label");
        assert!(content_control.methods[0].is_override);
        assert_eq!(content_control.methods[0].params.len(), 1);
        assert_eq!(content_control.methods[0].params[0].0, "suffix");
    }

    #[test]
    fn parses_on_mount_and_on_unmount() {
        let src = r#"
on_mount {
    base::on_mount();
    println!("mounted");
}
on_unmount {
    println!("unmounted");
}

Text { text: "hi" }
"#;
        let (on_mount, on_unmount, _, _, root) = parse_view_body(src).expect("should parse");
        assert!(on_mount.is_some());
        assert!(on_unmount.is_some());
        assert_eq!(root.children.len(), 1);
        assert_eq!(literal(&root.children[0]).type_path, "Text");
    }

    #[test]
    fn parses_attached_property_field() {
        let item_struct: syn::ItemStruct = syn::parse_str(
            r#"
            struct Grid {
                #[attached(default = 0)]
                row: i32,
                #[attached(default = 0)]
                column: i32,
            }
            "#,
        )
        .expect("struct should parse");
        let fields =
            crate::attr_frontend::fields_from_item_struct(&item_struct, FieldKind::Prop, true)
                .expect("fields should build");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].kind, FieldKind::Attached);
        assert_eq!(fields[1].kind, FieldKind::Attached);
    }

    #[test]
    fn parses_owner_colon_colon_field_attached_setter() {
        let src = r#"
Grid {
    rows: [GridLength::Auto, GridLength::Star(1.0)]
    columns: [GridLength::Fixed(120.0), GridLength::Star(1.0)]
    TextBlock { text: "Header", Grid::row: 0, Grid::column: 0 }
    Button { text: "Click", Grid::row: 1, Grid::column: 1 }
}
"#;
        let (_, _, _, _, root_body) = parse_view_body(src).expect("should parse");
        assert_eq!(root_body.children.len(), 1);
        let root = literal(&root_body.children[0]);
        assert_eq!(root.type_path, "Grid");
        assert!(matches!(
            &root.attributes[0].value,
            ViewExpr::Expr(syn::Expr::Array(_))
        ));
        assert!(matches!(
            &root.attributes[1].value,
            ViewExpr::Expr(syn::Expr::Array(_))
        ));

        let header = literal(&root.children[0]);
        assert_eq!(header.type_path, "TextBlock");
        assert_eq!(header.attributes.len(), 1);
        assert_eq!(header.attached.len(), 2);
        assert_eq!(
            (header.attached[0].0.as_str(), header.attached[0].1.as_str()),
            ("Grid", "row")
        );
        assert_eq!(
            (header.attached[1].0.as_str(), header.attached[1].1.as_str()),
            ("Grid", "column")
        );
        assert!(matches!(
            &header.attached[0].2,
            ViewExpr::Expr(syn::Expr::Lit(_))
        ));

        let button = literal(&root.children[1]);
        assert_eq!(button.attached.len(), 2);
        assert_eq!(button.attached[0].0, "Grid");
        assert_eq!(button.attached[0].1, "row");
    }

    #[test]
    fn parses_shortcut_attr_variants() {
        let src = r#"
Button {
    #[shortcut("Ctrl+S")]
    on_click: vm.save

    #[shortcut(winui3: "Ctrl+F", appkit: "Cmd+F", scope: local)]
    on_find: vm.find
}
"#;
        let (_, _, _, _, root_body) = parse_view_body(src).expect("should parse");
        let root = literal(&root_body.children[0]);
        assert_eq!(root.type_path, "Button");
        assert_eq!(root.attribute_shortcuts.len(), 2);

        let (name, chords, scope) = &root.attribute_shortcuts[0];
        assert_eq!(name, "on_click");
        assert_eq!(chords, &[(None, "Ctrl+S".to_string())]);
        assert_eq!(*scope, ShortcutScope::Global);

        let (name, chords, scope) = &root.attribute_shortcuts[1];
        assert_eq!(name, "on_find");
        assert_eq!(
            chords,
            &[
                (Some("winui3".to_string()), "Ctrl+F".to_string()),
                (Some("appkit".to_string()), "Cmd+F".to_string()),
            ]
        );
        assert_eq!(*scope, ShortcutScope::Local);
    }

    #[test]
    fn parses_shortcut_attr_after_token_recovered_closure_value() {
        // `component_frontend.rs` recovers a `view! { .. }` field's macro tokens via
        // `proc_macro2::TokenStream::to_string()`, which always renders `#[shortcut(...)]` as two
        // separate tokens with an inserted space (`# [shortcut (...)]`) and collapses the DSL's own
        // no-comma-needed line breaks onto one line — never the literal, unspaced `#[` a hand-typed
        // source has. Reproduce that exact shape (Issue #68 bug 3) instead of hand-typed spacing.
        let src = r#"Button { on_select : | index | vm . select_tab (index) # [shortcut ("Ctrl+S")] on_click : || { save () } , }"#;
        let (_, _, _, _, root) = parse_view_body(src).expect("should parse");
        let element = literal(&root.children[0]);
        assert_eq!(element.type_path, "Button");
        assert_eq!(element.attribute_shortcuts.len(), 1);
        let (name, chords, scope) = &element.attribute_shortcuts[0];
        assert_eq!(name, "on_click");
        assert_eq!(chords, &[(None, "Ctrl+S".to_string())]);
        assert_eq!(*scope, ShortcutScope::Global);
    }
}
