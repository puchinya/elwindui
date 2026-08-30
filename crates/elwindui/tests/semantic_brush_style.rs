//! Issue #97: end-to-end coverage for semantic brush roles in Theme, EnvironmentScope, and the
//! four Brush-valued DSL properties. These tests compile the generated Rust and exercise live
//! Environment-driven re-resolution rather than checking generated token strings only.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::graphics::{Brush, Color};
use elwindui::core::theme::{BrushStyle, ResolvedValue, Theme};
use elwindui::core::ui::{ControlExt as _, LayoutExt as _, TextStyleOwner as _, UIElementExt as _};

fn brush(r: u8, g: u8, b: u8) -> Brush {
    Brush::Solid(Color::rgb(r, g, b))
}

#[elwindui::theme]
struct SemanticBrushConcreteTheme {
    #[theme(value = BrushStyle::Value(brush(10, 20, 30)))]
    primary: BrushStyle,
    #[theme(value = BrushStyle::Primary)]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::Value(brush(40, 50, 60)))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(brush(70, 80, 90)))]
    separator: BrushStyle,
}

#[elwindui::theme]
struct SemanticBrushPlatformDefaultTheme {
    #[theme(value = BrushStyle::PlatformDefault)]
    primary: BrushStyle,
    #[theme(value = BrushStyle::PlatformDefault)]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::PlatformDefault)]
    background: BrushStyle,
    #[theme(value = BrushStyle::PlatformDefault)]
    separator: BrushStyle,
}

#[elwindui::component(inherits VerticalLayout)]
struct SemanticBrushView {
    #[environment(primary)]
    primary_style: BrushStyle,

    body: view! {
        #[id("label")]
        let label = TextBlock {
            text: "semantic",
            foreground: BrushStyle::Foreground,
        };
        let shape = Rectangle {
            fill: BrushStyle::Primary,
            stroke: BrushStyle::Separator,
        };
        #[id("panel")]
        let panel = VerticalLayout {
            background: BrushStyle::Background,
            label
            shape
        };

        panel
    },
}

#[elwindui::component]
impl SemanticBrushView {}

#[test]
fn theme_roles_resolve_reactively_for_all_brush_dsl_properties() {
    let environment = elwindui::core::environment::application_environment();
    SemanticBrushConcreteTheme.apply(&environment);

    let view = SemanticBrushView::new();
    assert_eq!(
        view.primary_style().resolve(&environment),
        ResolvedValue::Value(brush(10, 20, 30))
    );
    assert_eq!(view.label().foreground(), Some(brush(10, 20, 30)));
    assert_eq!(view.panel().background(), Some(brush(40, 50, 60)));

    SemanticBrushPlatformDefaultTheme.apply(&environment);

    assert_eq!(view.label().foreground(), None);
    assert_eq!(view.panel().background(), None);
}

thread_local! {
    static NON_SEMANTIC_FILL: std::cell::RefCell<String> = const {
        std::cell::RefCell::new(String::new())
    };
}

#[elwindui::component(inherits ContentControl)]
struct NonSemanticFillLeaf {
    #[prop(default = String::from("default"))]
    fill: String,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            NON_SEMANTIC_FILL.with(|value| *value.borrow_mut() = self.fill());
        }
        on_update(fill) {
            NON_SEMANTIC_FILL.with(|value| *value.borrow_mut() = this.fill());
        }
        TextBlock { text: fill }
    }),
}

#[elwindui::component]
impl NonSemanticFillLeaf {}

#[elwindui::component(inherits ContentControl)]
struct NonSemanticFillHost {
    template: template_view!(|templated_parent: Self| {
        NonSemanticFillLeaf {
            fill: "ordinary string",
        }
    }),
}

#[elwindui::component]
impl NonSemanticFillHost {}

#[test]
fn an_unrelated_property_named_fill_is_not_treated_as_a_semantic_brush() {
    NON_SEMANTIC_FILL.with(|value| value.borrow_mut().clear());
    let host = NonSemanticFillHost::new();
    assert!(host.apply_template());
    assert_eq!(
        NON_SEMANTIC_FILL.with(|value| value.borrow().clone()),
        "ordinary string"
    );
}

thread_local! {
    static SCOPED_PRIMARY: std::cell::RefCell<BrushStyle> =
        std::cell::RefCell::new(BrushStyle::PlatformDefault);
}

#[elwindui::component(inherits ContentControl)]
struct SemanticBrushScopeChild {
    #[environment(primary)]
    primary_style: BrushStyle,

    template: template_view!(|templated_parent: Self| {
        on_mount {
            SCOPED_PRIMARY.with(|value| *value.borrow_mut() = self.primary_style());
        }
        on_update(primary_style) {
            SCOPED_PRIMARY.with(|value| *value.borrow_mut() = this.primary_style());
        }
        TextBlock {
            text: "scoped",
            foreground: BrushStyle::Primary,
        }
    }),
}

#[elwindui::component]
impl SemanticBrushScopeChild {}

#[elwindui::component(inherits VerticalLayout)]
struct SemanticBrushScopeView {
    #[prop(default = BrushStyle::PlatformDefault)]
    local_primary: BrushStyle,

    body: view! {
        EnvironmentScope {
            primary: local_primary,
            SemanticBrushScopeChild {}
        }
    },
}

#[elwindui::component]
impl SemanticBrushScopeView {}

#[test]
fn environment_scope_retains_and_replays_semantic_overrides() {
    SCOPED_PRIMARY.with(|value| *value.borrow_mut() = BrushStyle::PlatformDefault);
    let view = elwindui::new!(SemanticBrushScopeView(
        local_primary: BrushStyle::Value(brush(1, 2, 3))
    ));
    let child = elwindui::core::visual_tree::find_all::<SemanticBrushScopeChild>(view.as_ref())
        .into_iter()
        .next()
        .expect("scoped semantic brush child");
    let child = child
        .as_any()
        .downcast_ref::<SemanticBrushScopeChild>()
        .expect("scoped semantic brush child has its concrete type");
    assert!(child.apply_template());
    assert_eq!(
        SCOPED_PRIMARY.with(|value| value.borrow().clone()),
        BrushStyle::Value(brush(1, 2, 3))
    );

    view.set_local_primary(BrushStyle::Value(brush(4, 5, 6)));
    assert_eq!(
        SCOPED_PRIMARY.with(|value| value.borrow().clone()),
        BrushStyle::Value(brush(4, 5, 6))
    );
}
