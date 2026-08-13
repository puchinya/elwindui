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
//! Native controls are intentionally absent from this demo: automatic Theme-driven native-control
//! styling was dropped in #96 (see the design doc's "Scope reduction") and is not yet restored by
//! Semantic Style (#97) / Native Style (#98).

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::application_environment;
use elwindui::core::graphics::{Brush, Color, FontWeight};
use elwindui::core::theme::Theme;
use elwindui::ui::WindowExt;

#[elwindui::environment_key(
    name = brand,
    value = Brush,
    default = Brush::Solid(Color::rgb(39, 103, 216))
)]
struct BrandEnvironment;

#[elwindui::environment_key(name = layout_spacing, value = f32, default = 10.0)]
struct LayoutSpacingEnvironment;

#[elwindui::theme]
struct DefaultTheme {
    #[theme(value = Brush::Solid(Color::rgb(39, 103, 216)))]
    brand: Brush,
    #[theme(value = 10.0)]
    layout_spacing: f32,
}

#[elwindui::theme]
struct OceanTheme {
    #[theme(value = Brush::Solid(Color::rgb(0, 166, 200)))]
    brand: Brush,
    #[theme(value = 20.0)]
    layout_spacing: f32,
}

#[elwindui::theme]
struct SolarizedTheme {
    #[theme(value = Brush::Solid(Color::rgb(181, 137, 0)))]
    brand: Brush,
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
    #[environment(brand)]
    brand: Brush,
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

            TextBlock {
                text: "Theme-as-Preset-over-Environment"
                font_size: 22.0
                font_weight: FontWeight::BOLD
            }
            TextBlock {
                text: "brand/layout_spacing below are ordinary #[environment(name)] fields. A Theme only sets the Environment values behind them — it never sets a property directly."
            }

            HorizontalLayout {
                spacing: 8.0
                TextBlock { text: "Theme:" font_weight: FontWeight::BOLD }
                Button { text: "Default" on_click: vm.choose_default }
                Button { text: "Ocean" on_click: vm.choose_ocean }
                Button { text: "Solarized" on_click: vm.choose_solarized }
                TextBlock { text: vm.theme_name }
            }

            Rectangle {
                width: 200.0
                height: 80.0
                fill: brand
                corner_radius: layout_spacing
            }
        }
    },
}

#[elwindui::component]
impl ThemeDemoWindow {}

#[elwindui::main]
fn main() {
    let vm = ThemeDemoViewModel::new();
    let window = ThemeDemoWindow::new(vm);
    window.show();
}
