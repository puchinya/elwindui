//! Test-only stand-in for the old, workspace-wide builtin shape source and `builtin_modules()`
//! (removed — see 05d4861/29ced3d/c916322/d255f31/36292fb, `docs/status/implementation_status.md`,
//! Refs #14): every real builtin's shape now lives as `#[elwindui_macros::class]` DSL attributes on
//! its actual `elwindui-core`/`elwindui-backend-*` declaration, propagated to a consumer crate via
//! the `__elwindui_shape_*!` macro chain (`elwindui-macros::class::build_props_macro`) rather than a
//! parsed text `Module` — so production code (`generate_component_from_item_struct`,
//! `compile_dir_impl`) no longer chains a builtin `Module` in at all, relying entirely on
//! `codegen::emit_external_construction`'s "no local `TypeInfo`, construct via `elwindui::ui::{Name}`
//! and the shape macro's `@set`/`@clear`/`@children` protocol" path (validated end-to-end by
//! temporarily emptying this exact module and confirming every real example still builds and runs).
//!
//! What that production path structurally *can't* do is compiler-side validation that needs an
//! actual field list (`is_abstract`/`#[sealed]`/required-attribute completeness/`#[routed]`-ness) —
//! a real Rust `type` has no equivalent a proc-macro can read across a crate boundary, so those
//! checks silently no-op for an external reference (`check_element_value`/`check_shortcut_attrs`/
//! `validate_inherits`'s own doc comments). That's an acceptable trade for *production* code (a
//! wrong reference still fails to compile, just later, via `elwindui::ui::{Name}::new()`/its shape
//! macro directly) — but several of `codegen.rs`/`validate.rs`'s own unit tests exist specifically to
//! exercise those richer checks, and unlike production they call `validate::validate`/
//! `codegen::generate_module` directly (no later rustc pass to fall back on). This module — a private
//! copy of the old real shape source, now allowed to drift out of sync with the real builtins without
//! consequence, since nothing production-facing reads it — gives those tests real `TypeInfo` to
//! check against again, exactly as `builtin_modules()` used to for everyone.
//!
//! Unlike the module's previous incarnation (a hand-written `component`/`view` DSL text file fed
//! through `parser::parse_module`), every `ComponentDef` here is assembled directly as a Rust struct
//! literal via `builtin_component`/`builtin_view` below. Two of `ComponentDef`'s own flags —
//! `embedded`/`native` — and `Module::is_builtin` have **no current-DSL-syntax equivalent** at all:
//! real builtins are declared entirely outside this crate's AST (via `#[elwindui_macros::class]` on
//! real `elwindui-core`/backend types), so `component_frontend.rs` — the real, production frontend —
//! never recognizes `#[embedded]`/`#[native]` as attribute names and never produces a `Module` with
//! `is_builtin: true` (see `ComponentDef::embedded`'s own doc comment in `ast.rs`). Every *other*
//! part of a builtin's shape — its fields (`attr_frontend::fields_from_item_struct`, the same
//! production field parser `component_frontend.rs` itself uses) and, for the handful with their own
//! `view` (`Rectangle`/`Ellipse`/`ContentControl`), their view body (`parser::parse_view_body`, still
//! real production code for a `view! { .. }` macro's contents) — is built by parsing real current-
//! syntax fragments, not hand-rolled.

use crate::ast::*;
use crate::attr_frontend;
use crate::parser;

/// Parses `fields_src` (comma-separated `name: Type` field declarations, using the same field
/// attribute vocabulary `docs/specs/dsl_spec.md` §4 documents — `#[routed]`, `#[attached(default =
/// ..)]`, `#[two_way]`, `#[onetime]`, ...) as the named-fields body of a synthetic `struct {name}
/// {{ .. }}`, then builds this component's `ComponentDef` from it. `embedded`/`sealed`/`native`/
/// `is_abstract`/`text_style`/`content_field` have no current-syntax spelling (see this module's own
/// doc comment) — supplied directly by the caller instead of being parsed.
#[allow(clippy::too_many_arguments)]
fn builtin_component(
    name: &str,
    base: Option<&str>,
    embedded: bool,
    sealed: bool,
    native: bool,
    is_abstract: bool,
    text_style: bool,
    content_field: Option<&str>,
    fields_src: &str,
) -> ComponentDef {
    let full_src = format!("struct {name} {{ {fields_src} }}");
    let item_struct: syn::ItemStruct = syn::parse_str(&full_src).unwrap_or_else(|e| {
        panic!("failed to parse builtin `{name}` fields: {e}\n---\n{full_src}")
    });
    let mut fields = attr_frontend::fields_from_item_struct(&item_struct, FieldKind::Prop, true)
        .unwrap_or_else(|e| panic!("failed to build fields for builtin `{name}`: {e}"));
    // Injected first, ahead of the component's own hand-written fields, matching the old DSL text
    // frontend's `#[text_style]` handling (`parser.rs`'s `parse_module`) — see
    // `resolve_effective_fields`'s "first still-unclaimed field" positional fallbacks, which rely on
    // every builtin field appearing in the same relative order.
    if text_style {
        let mut injected = crate::text_style::text_style_field_defs();
        injected.append(&mut fields);
        fields = injected;
    }
    ComponentDef {
        name: name.to_string(),
        base: base.map(str::to_string),
        base_path: None,
        fields,
        methods: Vec::new(),
        embedded,
        sealed,
        native,
        is_abstract,
        text_style,
        content_field: content_field.map(str::to_string),
    }
}

/// Parses `body_src` as a `view! { .. }` macro body (the same production entry point
/// `component_frontend.rs` uses for a real `#[elwindui::component]`'s `body: view! { .. }` field) and
/// wraps the result as `target`'s `ViewDef`.
fn builtin_view(target: &str, body_src: &str) -> ViewDef {
    let (on_mount, on_unmount, on_update, lets, root) = parser::parse_view_body(body_src)
        .unwrap_or_else(|e| {
            panic!("failed to parse builtin view `{target}`: {e}\n---\n{body_src}")
        });
    ViewDef {
        target: target.to_string(),
        on_mount,
        on_unmount,
        on_update,
        lets,
        root,
        implicit_owner: None,
    }
}

/// Test-only counterpart to the removed `builtin_modules()` — see this module's own doc comment.
/// `pub(crate)`: only `codegen.rs`/`validate.rs`/`component_frontend.rs`'s own `#[cfg(test)] mod
/// tests` blocks call this, never production code.
pub(crate) fn test_builtin_modules() -> Vec<Module> {
    let items = vec![
        Item::Component(builtin_component(
            "UIElement",
            None,
            true,
            false,
            false,
            true,
            false,
            None,
            r#"
            margin: Option<f32>,
            horizontal_alignment: Option<elwindui::core::layout::HorizontalAlignment>,
            vertical_alignment: Option<elwindui::core::layout::VerticalAlignment>,
            visibility: Option<elwindui::core::layout::Visibility>,
            width: Option<f32>,
            height: Option<f32>,
            min_width: Option<f32>,
            min_height: Option<f32>,
            max_width: Option<f32>,
            max_height: Option<f32>,
            hit_test_visible: Option<bool>,
            tab_stop: Option<bool>,
            focus_order: Option<i32>,
            #[routed]
            on_key_down: fn(elwindui::core::input::KeyEventArgs),
            #[routed]
            on_key_up: fn(elwindui::core::input::KeyEventArgs),
            #[routed]
            on_text_input: fn(elwindui::core::input::TextInputEventArgs),
            #[routed]
            on_got_focus: fn(),
            #[routed]
            on_lost_focus: fn(),
            #[routed]
            on_pointer_pressed: fn(elwindui::core::input::PointerEventArgs),
            #[routed]
            on_pointer_released: fn(elwindui::core::input::PointerEventArgs),
            #[routed]
            on_pointer_moved: fn(elwindui::core::input::PointerEventArgs),
            #[routed]
            on_pointer_entered: fn(elwindui::core::input::PointerEventArgs),
            #[routed]
            on_pointer_exited: fn(elwindui::core::input::PointerEventArgs),
            #[routed]
            on_pointer_wheel_changed: fn(elwindui::core::input::PointerWheelEventArgs),
            #[routed]
            on_tapped: fn(elwindui::core::input::TappedEventArgs),
            #[routed]
            on_double_tapped: fn(elwindui::core::input::TappedEventArgs),
            #[routed]
            on_right_tapped: fn(elwindui::core::input::TappedEventArgs),
            context_menu: Option<std::rc::Rc<dyn elwindui::core::ui::MenuExt>>,
            context_menu_presentation: Option<elwindui::core::ui::ContextMenuPresentation>,
            context_popup: Option<elwindui::core::ui::ViewTemplate>,
            "#,
        )),
        Item::Component(builtin_component(
            "Layout",
            Some("UIElement"),
            true,
            false,
            false,
            true,
            false,
            None,
            r#"
            children: UIElementCollection,
            #[semantic_brush]
            background: Option<elwindui::core::graphics::Brush>,
            "#,
        )),
        Item::Component(builtin_component(
            "NativeControl",
            Some("UIElement"),
            true,
            false,
            false,
            true,
            true,
            None,
            r#"
            #[semantic_brush]
            background: Option<elwindui::core::graphics::Brush>,
            "#,
        )),
        Item::Component(builtin_component(
            "Window",
            None,
            true,
            false,
            true,
            false,
            false,
            Some("content"),
            r#"
            title: String,
            menu_bar: Option<MenuBar>,
            content: std::rc::Rc<dyn UIElement>,
            #[onetime]
            left: Option<f32>,
            #[onetime]
            top: Option<f32>,
            #[onetime]
            width: Option<f32>,
            #[onetime]
            height: Option<f32>,
            "#,
        )),
        Item::Component(builtin_component(
            "VerticalLayout",
            Some("Layout"),
            true,
            false,
            false,
            false,
            false,
            Some("children"),
            r#"
            spacing: Option<f32>,
            "#,
        )),
        Item::Component(builtin_component(
            "HorizontalLayout",
            Some("Layout"),
            true,
            false,
            false,
            false,
            false,
            Some("children"),
            r#"
            spacing: Option<f32>,
            "#,
        )),
        Item::Component(builtin_component(
            "Shape",
            Some("UIElement"),
            true,
            false,
            false,
            true,
            false,
            None,
            r#"
            #[semantic_brush]
            fill: Option<elwindui::core::graphics::Brush>,
            #[semantic_brush]
            stroke: Option<elwindui::core::graphics::Brush>,
            stroke_width: Option<f32>,
            "#,
        )),
        Item::Component(builtin_component(
            "Rectangle",
            Some("Shape"),
            true,
            true,
            false,
            false,
            false,
            None,
            r#"
            corner_radius: Option<f32>,
            "#,
        )),
        Item::View(builtin_view(
            "Rectangle",
            r#"
            fill: fill
            stroke: stroke
            stroke_width: stroke_width
            "#,
        )),
        Item::Component(builtin_component(
            "Ellipse",
            Some("Shape"),
            true,
            true,
            false,
            false,
            false,
            None,
            "",
        )),
        Item::View(builtin_view(
            "Ellipse",
            r#"
            fill: fill
            stroke: stroke
            stroke_width: stroke_width
            "#,
        )),
        Item::Component(builtin_component(
            "Control",
            Some("UIElement"),
            true,
            false,
            false,
            false,
            true,
            Some("children"),
            r#"
            children: UIElementCollection,
            padding: Option<f32>,
            "#,
        )),
        Item::Component(builtin_component(
            "ContentControl",
            Some("Control"),
            true,
            false,
            false,
            false,
            false,
            Some("content"),
            r#"
            content: std::rc::Rc<dyn UIElement>,
            "#,
        )),
        Item::View(builtin_view(
            "ContentControl",
            r#"
            padding: padding
            content
            "#,
        )),
        Item::Component(builtin_component(
            "Grid",
            Some("Layout"),
            true,
            false,
            false,
            false,
            false,
            Some("children"),
            r#"
            rows: Vec<GridLength>,
            columns: Vec<GridLength>,
            #[attached(default = 0)]
            row: i32,
            #[attached(default = 0)]
            column: i32,
            "#,
        )),
        Item::Component(builtin_component(
            "TextArea",
            Some("NativeControl"),
            true,
            true,
            false,
            false,
            false,
            None,
            r#"
            #[two_way]
            text: String,
            "#,
        )),
        Item::Component(builtin_component(
            "TextBox",
            Some("NativeControl"),
            true,
            true,
            false,
            false,
            false,
            None,
            r#"
            #[two_way]
            text: String,
            placeholder: Option<String>,
            read_only: Option<bool>,
            max_length: Option<u32>,
            text_alignment: Option<TextAlignment>,
            "#,
        )),
        Item::Component(builtin_component(
            "PasswordBox",
            Some("NativeControl"),
            true,
            true,
            false,
            false,
            false,
            None,
            r#"
            #[two_way]
            password: String,
            placeholder: Option<String>,
            max_length: Option<u32>,
            reveal_enabled: Option<bool>,
            "#,
        )),
        Item::Component(builtin_component(
            "ScrollView",
            Some("NativeControl"),
            true,
            true,
            false,
            false,
            false,
            Some("content"),
            r#"
            content: std::rc::Rc<dyn UIElement>,
            horizontal_scroll_enabled: Option<bool>,
            vertical_scroll_enabled: Option<bool>,
            "#,
        )),
        Item::Component(builtin_component(
            "Button",
            Some("NativeControl"),
            true,
            true,
            false,
            false,
            false,
            None,
            r#"
            text: String,
            enabled: Option<bool>,
            #[routed]
            on_click: fn(),
            "#,
        )),
        Item::Component(builtin_component(
            "TextBlock",
            Some("UIElement"),
            true,
            false,
            false,
            false,
            true,
            None,
            r#"
            text: String,
            text_alignment: Option<TextAlignment>,
            "#,
        )),
        Item::Component(builtin_component(
            "Image",
            Some("UIElement"),
            true,
            false,
            false,
            false,
            false,
            None,
            r#"
            source: Option<elwindui::core::graphics::ImageSource>,
            stretch: Option<elwindui::core::graphics::Stretch>,
            rasterize: Option<elwindui::core::graphics::VectorRasterizeMode>,
            "#,
        )),
        Item::Component(builtin_component(
            "MenuBar",
            None,
            true,
            false,
            true,
            false,
            false,
            Some("items"),
            r#"
            items: elwindui_core::ui::ListExt<MenuBarItem>,
            "#,
        )),
        Item::Component(builtin_component(
            "MenuBarItem",
            None,
            true,
            false,
            true,
            false,
            false,
            Some("submenu"),
            r#"
            text: String,
            submenu: Menu,
            "#,
        )),
        Item::Component(builtin_component(
            "Menu",
            None,
            true,
            false,
            true,
            false,
            false,
            Some("items"),
            r#"
            items: elwindui_core::ui::ListExt<MenuItem>,
            "#,
        )),
        Item::Component(builtin_component(
            "MenuItem",
            None,
            true,
            false,
            true,
            false,
            false,
            None,
            r#"
            text: String,
            shortcut: Option<String>,
            enabled: Option<bool>,
            on_select: fn(),
            "#,
        )),
        Item::Component(builtin_component(
            "TabView",
            Some("NativeControl"),
            true,
            true,
            false,
            false,
            false,
            Some("children"),
            r#"
            children: Vec<TabViewItem>,
            #[two_way]
            selected_index: usize,
            on_select: fn(usize),
            on_new_tab: fn(),
            "#,
        )),
        Item::Component(builtin_component(
            "TabViewItem",
            None,
            true,
            true,
            true,
            false,
            false,
            Some("content"),
            r#"
            header: String,
            content: std::rc::Rc<dyn UIElement>,
            closable: Option<bool>,
            on_close: fn(),
            "#,
        )),
    ];

    let module = Module {
        path: Vec::new(),
        uses: Vec::new(),
        items,
        is_builtin: true,
        allows_external_builtins: false,
    };
    vec![module]
}
