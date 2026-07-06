/// A single `.elwind` file (or an equivalent synthetic module built from a plain `.rs` file's
/// `#[elwindui::viewmodel] mod foo { .. }`, see `attr_frontend.rs`). See docs/elwindui_spec.md §12
/// (`use`), §1-15 core language, 付録B.1 (how `path` maps to a real Rust module path).
#[derive(Debug, Clone, Default)]
pub struct Module {
    /// This module's real, crate-relative path segments — `[]` for a `.elwind` file compiled by
    /// `compile_dir` (which lands flat at the crate root via `include!`, 付録B.1) or for a
    /// standalone proc-macro invocation; `["notepad_view_model"]` for Rust source's
    /// `mod notepad_view_model { .. }`. `use` declarations (§12) are resolved against these paths
    /// exactly like Rust's own name resolution — see `codegen::build_symbol_table`/`validate::validate`.
    pub path: Vec<String>,
    pub uses: Vec<UseDecl>,
    pub items: Vec<Item>,
}

/// `use components::card::Card;` / `use a::b::{C, D};` (§12). Only the flat form is needed for
/// notepad; the brace-group form can be added when a `.elwind` file actually uses it.
#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Component(ComponentDef),
    ViewModel(ViewModelDef),
    Enum(EnumDef),
    View(ViewDef),
}

/// `component Name inherits Base { fields }`. See docs/elwindui_spec.md §3, 付録H.2.
///
/// `base` does *not* merge `Base`'s fields into this component's own (see `validate.rs`'s
/// `inherits` checks) — it's a structural contract, not field inheritance: either `Base` is the
/// `NativeComponent` marker (a pure category tag, checked for consistency against the
/// recursively-inferred `is_native`, see `codegen::build_symbol_table`), or the paired `view`'s
/// root element must literally construct `Base` (e.g. `RoundedPanel inherits Rectangle`).
#[derive(Debug, Clone)]
pub struct ComponentDef {
    pub name: String,
    pub base: Option<String>,
    pub fields: Vec<FieldDef>,
}

/// `viewmodel Name { fields }`, reusing the same field syntax as `component`/`store`.
/// See docs/elwindui_spec.md 付録O.2.
#[derive(Debug, Clone)]
pub struct ViewModelDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

/// See docs/elwindui_spec.md §8.
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Default: runtime-mutable. See §4.
    Prop,
    /// `#[param]`: fixed at instantiation. See §4.
    Param,
    /// `#[observable]`: `viewmodel`/`store` runtime-mutable field. See 付録O.2.
    Observable,
    /// `#[computed]`: read-only, recomputed from its dependencies. See §4, 付録O.5.
    Computed,
    /// `#[command(...)]`, backed by `command!(...)`. See 付録O.3.
    Command,
}

#[derive(Debug, Clone)]
pub enum Attr {
    /// `#[inject]`: caller supplies the value at construction (used with `#[param]`). See 付録J.5.
    Inject,
    /// `#[two_way]`: marks a builtin shape's `#[param]` field as eligible for automatic two-way
    /// wiring — when an element's value for this attribute is a settable path, codegen wires a
    /// change callback back into it generically (no per-type `codegen.rs` logic needed). See
    /// `crates/elwindui-builtins`'s shape declarations (e.g. `TextArea`'s `text` field).
    TwoWay,
    /// `#[length(start..=end)]` / `#[length(start..end)]`. See §7.
    Length { start: i64, end: i64, inclusive: bool },
    /// `#[command(can_execute: expr)]` / `#[command(async)]` / `#[command(async, can_execute: expr)]`.
    /// See 付録O.3, 付録P.4.
    CommandMeta { is_async: bool, can_execute: Option<syn::Expr> },
}

/// A `component`/`viewmodel` field. See docs/elwindui_spec.md §3, 付録O.2.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub ty: String,
    pub kind: FieldKind,
    pub attrs: Vec<Attr>,
    pub initializer: Option<Initializer>,
}

/// How a field's initializer expression was recognized. Only `bind!`/`command!` are given their
/// own DSL-level macro syntax (§10, 付録O.3); everything else is an arbitrary Rust expression and
/// is parsed for real via `syn` rather than kept as opaque text.
#[derive(Debug, Clone)]
pub enum Initializer {
    /// `bind!(vm.content, TwoWay)`. See §10.
    Bind { path: Vec<String>, mode: String },
    /// `command!(|| { ... })` / `command!(|index: usize| { ... })`. `params` is empty for the
    /// common zero-arg case; a parameterized command (needed so e.g. `TabView`'s per-tab
    /// close/select callbacks can pass an index through to a `Command`) generates
    /// `pub fn X_execute(&self, index: usize)` instead of the zero-arg form. See 付録O.3.
    Command { params: Vec<(String, syn::Type)>, body: syn::Block },
    /// Any other initializer expression (literals, `String::new()`, `content.chars().count()
    /// as i32`, `t!(...)`, ...), parsed as a real `syn::Expr`.
    Expr(syn::Expr),
}

/// `view Name { let-bindings... ElementTree }`. See docs/elwindui_spec.md §2, §13.
#[derive(Debug, Clone)]
pub struct ViewDef {
    pub target: String,
    /// Zero or more `#[id("...")]? let name = Element { .. };` statements, in source order,
    /// preceding `root`. Each introduces a name referenceable later (as a bare `ChildEntry::Ref`)
    /// within `root` or a later `let`'s own element.
    pub lets: Vec<LetBinding>,
    pub root: ElementNode,
}

/// `#[id("editor")] let editor = TextArea { text: content };` — see docs/elwindui_spec.md §13's
/// "特定要素への名前付きアクセス". `id`, when present, becomes a generated named accessor method
/// (`self.editor()`) returning that binding's concrete Rust type (`codegen.rs`'s
/// `emit_named_accessors`) — not a runtime string-keyed lookup (`#[id(...)]` names are always
/// known at compile time, so a monomorphized accessor is strictly sufficient and matches this
/// project's avoid-type-erasure/avoid-dyn-dispatch convention, 付録O.5).
#[derive(Debug, Clone)]
pub struct LetBinding {
    pub id: Option<String>,
    pub name: String,
    pub element: ElementNode,
}

/// `Type { key: expr, ChildElement { ... } }`. Attribute values and nested elements share the
/// same `{}` body; the parser splits them by whether an entry looks like `key: value` or a bare
/// `Type { ... }`.
#[derive(Debug, Clone)]
pub struct ElementNode {
    pub type_path: String,
    pub attributes: Vec<(String, ViewExpr)>,
    pub children: Vec<ChildEntry>,
}

/// A bare (non-`key:`-prefixed) entry inside an element's `{}` body — either a literal nested
/// element (`Type { .. }`, as always) or a bare identifier referring to an earlier `let` binding
/// (e.g. `Column { editor, StatusBar {} }`'s `editor`).
#[derive(Debug, Clone)]
pub enum ChildEntry {
    Literal(ElementNode),
    Ref(String),
}

/// Expressions that can appear as an element attribute value. `t!` is recognized directly by the
/// parser (its `name: expr` argument form isn't valid standalone Rust); everything else that
/// isn't one of the DSL's own field-path sugars falls back to a real `syn::Expr`.
#[derive(Debug, Clone)]
pub enum ViewExpr {
    /// A dotted field path, e.g. `content` -> `["content"]`, `vm.window_title` ->
    /// `["vm", "window_title"]`.
    Path(Vec<String>),
    /// `vm.save.execute()`: `(["vm", "save"], "execute")`. See 付録O.4.
    MethodCall(Vec<String>, String),
    /// `t!("key", name: expr, ...)`. See §11.
    TFluent(String, Vec<(String, ViewExpr)>),
    /// Any other expression (string/number literals, etc.), parsed via `syn`.
    Expr(syn::Expr),
    /// `|doc| <body>` — a single untyped bound parameter (no destructuring, no type annotation)
    /// used by `TabView`'s `header_template`/`item_template` attributes (付録Y) so a tab's
    /// per-item header/content can be an arbitrary expression or nested `view`, rather than the
    /// fixed `TextArea` codegen used to hardcode. Also implicitly aliased as `data_context` inside
    /// the closure body (`emit_expr`'s `data_context` substitution).
    Closure { param: String, body: ClosureBody },
    /// `menu_bar: MenuBar { .. }` — a nested element used as an ordinary (non-closure) attribute
    /// value, for a builtin shape's "named single-child slot" (e.g. `Window`'s `menu_bar`/
    /// `content` params instead of positional/type-based child detection). Same shape as
    /// `ClosureBody::Element`, just not behind a `|param|`.
    Element(Box<ElementNode>),
}

/// The body of a `ViewExpr::Closure`. `key`/`render_label` return a plain expression;
/// `render_content` returns a `view` (an element construction), so the two need different shapes
/// rather than forcing `render_content`'s `Type { ... }` through `ViewExpr`.
#[derive(Debug, Clone)]
pub enum ClosureBody {
    /// `|doc| doc.file_name`, `|doc| std::rc::Rc::as_ptr(doc) as usize`.
    Expr(Box<ViewExpr>),
    /// `|doc| DocumentView { doc: doc }`.
    Element(Box<ElementNode>),
}
