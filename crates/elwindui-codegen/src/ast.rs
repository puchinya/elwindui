/// A single `.elwind` file. See docs/elwindui_spec.md §12 (`use`), §1-15 core language.
#[derive(Debug, Clone)]
pub struct Module {
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

/// `component Name { fields }`. See docs/elwindui_spec.md §3.
#[derive(Debug, Clone)]
pub struct ComponentDef {
    pub name: String,
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

/// `view Name { ElementTree }`. See docs/elwindui_spec.md §2.
#[derive(Debug, Clone)]
pub struct ViewDef {
    pub target: String,
    pub root: ElementNode,
}

/// `Type { key: expr, ChildElement { ... } }`. Attribute values and nested elements share the
/// same `{}` body; the parser splits them by whether an entry looks like `key: value` or a bare
/// `Type { ... }`.
#[derive(Debug, Clone)]
pub struct ElementNode {
    pub type_path: String,
    pub attributes: Vec<(String, ViewExpr)>,
    pub children: Vec<ElementNode>,
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
}
