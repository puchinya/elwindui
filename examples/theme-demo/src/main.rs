//! Interactive verification for the Theme-as-Preset-over-Environment model (Issue #96).
//!
//! `brand`/`layout_spacing` are ordinary `#[elwindui::environment_key]` Environment values —
//! nothing here is Theme-specific. A `#[elwindui::theme]` Preset's only job is to batch
//! `EnvironmentContext::set` calls for keys declared this way (`docs/specs/theme_environment_spec.md`
//! §3/§4). "Switching theme" below is applying a different Preset instance to the same
//! `application_environment()`; there is no variant enum and no Theme-specific invalidation —
//! `ThemeDemoWindow`'s `#[environment(..)]` fields re-render through Environment's own per-key
//! reactive subscription (`docs/design/runtime/theme_environment_design.md`).
//!
//! Issue #97's Semantic Style path is demonstrated by assigning `BrushStyle` to
//! foreground/background/fill/stroke. These values re-resolve when a different Theme is applied;
//! automatic native-control appearance remains separate work tracked by #98.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::application_environment;
use elwindui::core::graphics::{Brush, Color, FontWeight};
use elwindui::core::theme::{BrushStyle, Theme};

#[elwindui::environment_key(name = layout_spacing, value = f32, default = 10.0)]
struct LayoutSpacingEnvironment;

#[elwindui::theme]
struct DefaultTheme {
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(39, 103, 216))))]
    primary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(24, 32, 44))))]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(242, 245, 249))))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(148, 163, 184))))]
    separator: BrushStyle,
    #[theme(value = 10.0)]
    layout_spacing: f32,
}

#[elwindui::theme]
struct OceanTheme {
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(0, 166, 200))))]
    primary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(7, 45, 54))))]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(224, 247, 250))))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(8, 120, 140))))]
    separator: BrushStyle,
    #[theme(value = 20.0)]
    layout_spacing: f32,
}

#[elwindui::theme]
struct SolarizedTheme {
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(181, 137, 0))))]
    primary: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(88, 110, 117))))]
    foreground: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(253, 246, 227))))]
    background: BrushStyle,
    #[theme(value = BrushStyle::Value(Brush::Solid(Color::rgb(147, 161, 161))))]
    separator: BrushStyle,
    #[theme(value = 4.0)]
    layout_spacing: f32,
}

fn select_default() {
    DefaultTheme.apply(&application_environment());
}

fn select_ocean() {
    OceanTheme.apply(&application_environment());
}

fn select_solarized() {
    SolarizedTheme.apply(&application_environment());
}

#[elwindui::viewmodel]
mod theme_demo_view_model {
    use super::{select_default, select_ocean, select_solarized};

    struct ThemeDemoViewModel {
        #[observable(default = "Default".to_string())]
        theme_name: String,
    }

    impl ThemeDemoViewModel {
        fn choose_default(&self) {
            theme_name = "Default".to_string();
            select_default();
        }
        fn choose_ocean(&self) {
            theme_name = "Ocean".to_string();
            select_ocean();
        }
        fn choose_solarized(&self) {
            theme_name = "Solarized".to_string();
            select_solarized();
        }
    }
}

#[elwindui::component(inherits Window)]
struct ThemeDemoWindow {
    #[environment(layout_spacing)]
    layout_spacing: f32,

    #[bindable]
    vm: std::rc::Rc<ThemeDemoViewModel>,

    body: view! {
        title: "elwindui Theme Demo"
        width: 640.0
        height: 420.0
        content: VerticalLayout {
            margin: 16.0
            spacing: layout_spacing
            background: BrushStyle::Background

            TextBlock {
                text: "Theme-as-Preset-over-Environment"
                font_size: 22.0
                font_weight: FontWeight::BOLD
                foreground: BrushStyle::Primary
            }
            TextBlock {
                text: "BrushStyle resolves semantic roles through the effective Environment. Theme buttons update foreground, background, fill, and stroke live."
                foreground: BrushStyle::Foreground
            }

            HorizontalLayout {
                spacing: 8.0
                TextBlock { text: "Theme:" font_weight: FontWeight::BOLD }
                Button { text: "Default" on_click: vm.choose_default }
                Button { text: "Ocean" on_click: vm.choose_ocean }
                Button { text: "Solarized" on_click: vm.choose_solarized }
                TextBlock { text: vm.theme_name foreground: BrushStyle::Foreground }
            }

            Rectangle {
                width: 200.0
                height: 80.0
                fill: BrushStyle::Primary
                stroke: BrushStyle::Separator
                corner_radius: layout_spacing
            }
        }
    },
}

#[elwindui::component]
impl ThemeDemoWindow {}

#[elwindui::main]
fn main() {
    select_default();
    let vm = ThemeDemoViewModel::new();
    let window = ThemeDemoWindow::new(vm);
    window.show();
}
