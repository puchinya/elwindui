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
    /// Whether this module came from `elwindui-codegen`'s own `BUILTIN_SHAPE_SOURCE`
    /// (`builtin_modules()`, set there) rather than a consumer's own `.elwind` directory —
    /// `validate::validate` uses this to reject a `#[embedded]` component declared outside the
    /// actual builtin sources (docs/elwindui_spec.md 付録E).
    pub is_builtin: bool,
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
/// `base` resolves to one of four cases (see `validate.rs`'s `validate_inherits` and
/// `codegen.rs`'s `resolve_effective_fields`/`resolve_view_for`):
/// - `Base` is the `NativeControl` marker: a pure category tag, checked for consistency against
///   the recursively-inferred `is_native` (see `codegen::build_symbol_table`) — no fields/methods
///   to inherit.
/// - `Base` is a `has_view == false` primitive shape (e.g. `Control`/`Rectangle`, `is_virtual_builtin`):
///   `Name` must write its own `view` whose root literally constructs `Base` (checked by
///   `validate_inherits`; there is no view-synthesis fallback for an omitted one). `Name` inherits
///   `Base`'s fields the usual bare-reference way (`resolve_effective_fields`), and — because the
///   root construction matches `Base` exactly — `codegen.rs`'s `generate_view` additionally
///   generates `Name`'s struct with a real `base: <Base>` field (`elwindui_core::ui`'s own
///   struct/`Ext`-trait/`base` convention, docs/elwindui_spec.md 付録H.2.1a) and a direct
///   `impl UIElementExt`/`impl <Base's own Ext trait>` delegating to it, instead of the generic "wrapper
///   owning a separately-`Rc`-erased root" every other `view`-having component uses. See
///   `codegen.rs`'s `generate_view` `is_shape_composition` doc comment for why this is deliberately
///   narrow (`RoundedPanel inherits Rectangle`, `ContentControl inherits Control`).
/// - `Base` has its own `view` (a logical component, builtin or user-defined) that isn't one of the
///   virtual-builtin shapes above: `Name` inherits `Base`'s fields *and* its `view` as a default
///   template — if `Name` defines its own `view`, that's a full override (no constraint on its root
///   element; see the *code*-reuse sub-case below), otherwise `Base`'s `view` is cloned with the
///   target renamed to `Name`. That template-reuse (no-own-`view`) sub-case gets real `base`
///   composition too, transitively, whenever `Base` is itself already composed (`LabeledPanel
///   inherits ContentControl`, `TypeInfo::composed_shape`/`codegen.rs`'s `resolve_composed_shape`):
///   `Name`'s struct embeds a real `base: Base` field, built by calling `Base`'s own
///   `create_<snake case>(..)` factory (which every composed component exposes, precisely so a
///   *further* derived one can call it directly — see `generate_view`'s `is_template_composition`).
///   A `Name` that instead defines its *own* `view` reusing `Base`'s *code* rather than its structure
///   (`Derived inherits Base`, both independently rooted, `#[override] fn`/`base::name(...)`) keeps
///   the original field-flattening/`__base_<name>` shadow-method mechanism unchanged — there's no
///   live `Base` instance to compose over there, only its method *bodies* to reuse (no different from
///   `super.method()` in a mainstream OOP language never needing a freestanding `super` object).
/// - `Base` is a native-backed leaf that carries real fields and has no `view` of its own but *is*
///   a hand-written Rust type (e.g. `Window`) — same contract as the shape-composition case above:
///   `Name`'s own `view` root must literally construct `Base` ("host composition",
///   `TypeInfo::host_composition_base`/`codegen.rs`'s `generate_view`), just without an
///   `impl UIElement` (`Base` doesn't implement it either).
///
/// `Name`'s own `fields`/`methods` may redeclare a same-named inherited `#[computed]` field or
/// `#[virtual]` method only when marked `#[override]` (`Attr::Override`) — see
/// `validate::validate_field_overrides`. Overriding bodies may call the base implementation via
/// `base::name(...)`, rewritten by `codegen.rs`'s `rewrite_base_calls` to a generated `__base_name`
/// method carrying the base's original body (the shape-composition case above has no `#[override]`
/// use today, but would still go through this same mechanism if it ever did).
#[derive(Debug, Clone)]
pub struct ComponentDef {
    pub name: String,
    pub base: Option<String>,
    pub fields: Vec<FieldDef>,
    pub methods: Vec<MethodDef>,
    /// `#[embedded]` (written immediately before `component`, docs/elwindui_spec.md 付録E): marks
    /// this component as one of the builtin shape declarations (`BUILTIN_SHAPE_SOURCE`) — `validate::validate`
    /// rejects it on a component whose `Module::is_builtin` is `false` (i.e. a consumer's own
    /// `.elwind` file falsely claiming to be a builtin).
    pub embedded: bool,
    /// `#[sealed]` (same position): marks this component as unable to be named as a `base` in
    /// `component X inherits Y` — `validate::validate_inherits` rejects `inherits` naming a sealed
    /// component. Used on concrete leaves that shouldn't be extended further (`Rectangle`/`Ellipse`
    /// — extend the composable `Shape` instead; `Button`/`TextArea`/`TabView`/`TabViewItem` — already
    /// implied by their native-leaf-with-no-view shape, but stated explicitly here for clarity).
    pub sealed: bool,
    /// `#[native]` (same position): marks a **base-less, `view`-less** component whose real Rust
    /// implementation is hand-written per backend crate (`elwindui-backend-appkit`/`-winui3`/...),
    /// exactly like an `inherits NativeControl` leaf (`codegen::resolve_is_native` treats either as
    /// native) — but for a leaf with no meaningful `inherits` base at all. `Window` is the
    /// motivating case: real WinUI3's `Window` derives directly from `Object`, not through the
    /// `Control` family every other native leaf (`Button`/`TextArea`/...) shares via `NativeControl`
    /// — declaring `inherits NativeControl` for it would suggest a shared ancestry that doesn't
    /// exist. `validate::validate` rejects `#[native]` combined with an explicit `base` or an own
    /// `view`, and (like `#[embedded]`) outside this crate's own `BUILTIN_SHAPE_SOURCE`.
    pub native: bool,
    /// `#[abstract]` (same position): marks this component as a pure category tag that cannot be
    /// instantiated directly in a `view` — only named as a `base` in `component X inherits Y` or
    /// (for a shape-composition base like `Shape`) as the root of another component's own `view`.
    /// `validate::check_element_value` rejects any `Type { .. }`/bare-child use site naming an
    /// `#[abstract]` component; `codegen::generate_module` skips it entirely (no `create_<snake
    /// case>(..)`/`new(..)` is ever generated for it). Used on `builtins.elwind`'s pure markers
    /// (`UIElement`/`NativeControl`/`Layout`/`Shape`) — a concrete virtual builtin meant to be used
    /// directly (`VerticalLayout`/`HorizontalLayout`/`Control`/`Grid`/`TextBlock`) does not set this.
    pub is_abstract: bool,
    /// `#[content(field_name)]` (same position, docs/elwindui_spec.md 付録E): WinUI3's
    /// `ContentPropertyAttribute` equivalent — names which of this component's own fields a bare
    /// nested child element (`Type { .. }` written directly inside `{}`, no `name:` attribute)
    /// binds to. `codegen::build_component_args` reads this (via `TypeInfo::content_field`) instead
    /// of the field-order-dependent "first still-unclaimed non-`Option` field" fallback it used
    /// before this attribute existed. `validate::validate` checks `field_name` actually names one of
    /// this component's effective fields. `None` for a component with no bare-nested-child
    /// convention at all (a bare child anywhere in its `view` usage is then a hard codegen error,
    /// see `build_component_args`'s trailing check).
    pub content_field: Option<String>,
}

/// `#[virtual] fn name(&self, params) -> RetTy { body }` / `#[override] fn name(...) { body }`.
/// Deliberately narrow — not a general Rust-method escape hatch, just enough to give components a
/// WinUI3-style overridable hook (e.g. a lifecycle hook) with a `base::name(...)` call to chain
/// into the base implementation. See docs/elwindui_spec.md §3.
#[derive(Debug, Clone)]
pub struct MethodDef {
    pub name: String,
    pub is_virtual: bool,
    pub is_override: bool,
    pub params: Vec<(String, syn::Type)>,
    pub return_ty: Option<syn::Type>,
    pub body: syn::Block,
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
    /// `#[attached]`: a WPF/WinUI3-style attached property (§3) — declares a property that any
    /// *other* element in the tree may set on itself via `Owner::field: value` (e.g. `Grid`'s
    /// `row`/`column`, settable on any child anywhere, not just `Grid`'s own direct children).
    /// Unlike every other kind, a field of this kind is *not* instance data of the component that
    /// declares it (`Grid` doesn't itself have a `row`/`column`) — it's a schema declaration only,
    /// excluded from the declaring component's own generated struct/constructor (`codegen.rs`'s
    /// `build_symbol_table` filters `param_fields`/etc. by `f.initializer.is_none()`, and this kind
    /// requires an initializer — see `validate::validate` — so it's excluded there for free).
    Attached,
}

#[derive(Debug, Clone)]
pub enum Attr {
    /// `#[inject]`: caller supplies the value at construction (used with `#[param]`). See 付録J.5.
    Inject,
    /// `#[two_way]`: marks a builtin shape's `#[param]` field as eligible for automatic two-way
    /// wiring — when an element's value for this attribute is a settable path, codegen wires a
    /// change callback back into it generically (no per-type `codegen.rs` logic needed). See
    /// this crate's own `builtins.elwind` shape declarations (e.g. `TextArea`'s `text` field).
    TwoWay,
    /// `#[length(start..=end)]` / `#[length(start..end)]`. See §7.
    Length {
        start: i64,
        end: i64,
        inclusive: bool,
    },
    /// `#[command(can_execute: expr)]` / `#[command(async)]` / `#[command(async, can_execute: expr)]`.
    /// See 付録O.3, 付録P.4.
    CommandMeta {
        is_async: bool,
        can_execute: Option<syn::Expr>,
    },
    /// `#[routed]`: marks a callback-typed field (`fn()`, `fn(usize)`, ...) as a WinUI3-style
    /// routed event — dispatched via `elwindui_core::ui::dispatch_routed` (bubbling from the
    /// element it's declared on up through ancestors' own handlers for the same field name,
    /// stopping at the first one that sets `RoutedEventArgs::handled`) instead of being called
    /// directly. Not tied to any specific field name (`on_click` is just the first user of it) —
    /// see docs/elwindui_spec.md 4章.
    Routed,
    /// `#[override]`: on a `#[computed]` field, marks an intentional override of a same-named
    /// inherited `#[computed]` field (vs. an accidental name collision, which is a validation
    /// error). Declared types must match; the base's original initializer is preserved under a
    /// generated `__base_name` accessor, reachable from the override's body via `base::name()`.
    /// See docs/elwindui_spec.md §3, `validate::validate_field_overrides`.
    Override,
    /// `#[onetime]`: marks a builtin shape's `#[param]` field as construction-time-only — applied
    /// once when the element is built, never re-applied by `resync()`. For a field whose real
    /// setter has externally-mutable, backend-owned semantics (e.g. `Window`'s `left`/`top`/
    /// `width`/`height` — the OS window manager, not the `.elwind` declaration, owns the live
    /// value once the window exists), blindly re-pushing the originally-declared value on every
    /// unrelated `resync()` would fight the user's own subsequent interaction (dragging/resizing)
    /// by snapping it back. Declarative replacement for what used to be a hardcoded
    /// `node.type_path == "Window" && matches!(name, "left" | "top" | "width" | "height")` check
    /// in `codegen.rs`'s `emit_resync`.
    Onetime,
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
    Command {
        params: Vec<(String, syn::Type)>,
        body: syn::Block,
    },
    /// Any other initializer expression (literals, `String::new()`, `content.chars().count()
    /// as i32`, `t!(...)`, ...), parsed as a real `syn::Expr`.
    Expr(syn::Expr),
}

/// `view Name { on_mount { .. } on_unmount { .. } let-bindings... ElementTree }`. See
/// docs/elwindui_spec.md §2, §13, 付録I.1.
#[derive(Debug, Clone)]
pub struct ViewDef {
    pub target: String,
    /// `on_mount { .. }`, run once right after construction (spliced into generated `new()` after
    /// `resync()`). When `Name` inherits a base with its own `view` and `Name` provides its own
    /// `view`, an `on_mount` here may call `base::on_mount()` to chain into the base's block
    /// (rewritten by `codegen.rs`'s `rewrite_base_calls`, same as `#[override]` methods). See
    /// docs/elwindui_spec.md 付録I.1/I.3 (param-immutability during `on_mount` is still enforced).
    pub on_mount: Option<syn::Block>,
    /// `on_unmount { .. }`, parsed/validated/codegen'd (as an inert `__run_on_unmount` method) but
    /// not yet wired to any runtime teardown trigger — `elwindui_core::ui` has no detach/removal
    /// hook today. See docs/elwindui_spec.md 付録I.1.
    pub on_unmount: Option<syn::Block>,
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

/// `Type { key: expr, Owner::attached_field: expr, ChildElement { ... } }`. Attribute values and
/// nested elements share the same `{}` body; the parser splits them by whether an entry looks like
/// `key: value`, `Owner::field: value` (an attached-property setter, §3), or a bare `Type { ... }`.
#[derive(Debug, Clone)]
pub struct ElementNode {
    pub type_path: String,
    pub attributes: Vec<(String, ViewExpr)>,
    /// `Grid::row: 1` etc. — `(owner type name, attached field name, value)`. `owner` need not be
    /// (and isn't checked to be) an actual ancestor of this element anywhere in the tree — like
    /// WPF's own attached properties, an unconsumed one is simply inert, not a static error. See
    /// `validate::validate` and `codegen.rs`'s `PlannedNode`/wherever a child's `UIElementBase` is
    /// constructed.
    pub attached: Vec<(String, String, ViewExpr)>,
    pub children: Vec<ChildEntry>,
}

/// A bare (non-`key:`-prefixed) entry inside an element's `{}` body — either a literal nested
/// element (`Type { .. }`, as always) or a bare identifier referring to an earlier `let` binding
/// (e.g. `Column { editor, StatusBar {} }`'s `editor`).
#[derive(Debug, Clone)]
pub enum ChildEntry {
    Literal(ElementNode),
    Ref(String),
    /// Rust-style conditional child region. Both arms contain ordinary child entries so nested
    /// control flow and literal elements share one representation.
    If {
        condition: ViewExpr,
        then_branch: Vec<ChildEntry>,
        else_branch: Vec<ChildEntry>,
    },
    /// Enum-oriented branch region. `pattern` is kept as source text until validation resolves
    /// it against the discriminant enum (or recognises `_`).
    Match {
        value: ViewExpr,
        arms: Vec<MatchArm>,
    },
    /// Repeated child region. The binding is local to `body` and never becomes a component field.
    For {
        binding: String,
        collection: ViewExpr,
        body: Vec<ChildEntry>,
    },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: String,
    pub body: Vec<ChildEntry>,
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
    /// fixed `TextArea` codegen used to hardcode.
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
