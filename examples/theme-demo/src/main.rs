//! Interactive verification for Rust-defined ElwindUI themes.
//!
//! Operating-system appearance and application variants are independent. Switching either axis
//! updates the same visual nodes, including transitions from explicit values back to backend
//! defaults.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::graphics::{Brush, Color, FontFamily, FontWeight};
use elwindui::core::theme::{ThemeController, ThemePreference, set_application_theme};
use elwindui::ui::WindowExt;

#[elwindui::theme_definition(
    extends = SystemTheme,
    variants(Default, Ocean, Solarized)
)]
struct AppTheme {
    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(5, 32, 45)),
        Solarized = Brush::Solid(Color::rgb(0, 43, 54))
    )]
    window_background: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(10, 55, 72)),
        Solarized = Brush::Solid(Color::rgb(7, 54, 66))
    )]
    layout_background: Brush,

    #[theme(default = 10.0, Ocean = 14.0, Solarized = 12.0)]
    layout_spacing: f32,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(0, 166, 200)),
        Solarized = Brush::Solid(Color::rgb(38, 139, 210))
    )]
    button_background: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(246, 252, 255)),
        Solarized = Brush::Solid(Color::rgb(253, 246, 227))
    )]
    button_foreground: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(17, 70, 88)),
        Solarized = Brush::Solid(Color::rgb(238, 232, 213))
    )]
    text_box_background: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(235, 250, 255)),
        Solarized = Brush::Solid(Color::rgb(88, 110, 117))
    )]
    text_box_foreground: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(17, 70, 88)),
        Solarized = Brush::Solid(Color::rgb(238, 232, 213))
    )]
    password_box_background: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(235, 250, 255)),
        Solarized = Brush::Solid(Color::rgb(88, 110, 117))
    )]
    password_box_foreground: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(17, 70, 88)),
        Solarized = Brush::Solid(Color::rgb(238, 232, 213))
    )]
    text_area_background: Brush,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(235, 250, 255)),
        Solarized = Brush::Solid(Color::rgb(88, 110, 117))
    )]
    text_area_foreground: Brush,

    #[theme(
        default = platform_default,
        Ocean = FontFamily::new("Segoe UI Variable, Segoe UI"),
        Solarized = FontFamily::new("Consolas, Menlo, monospace")
    )]
    text_block_font_family: FontFamily,

    #[theme(default = platform_default, Ocean = 18.0, Solarized = 17.0)]
    text_block_font_size: f32,

    #[theme(
        default = platform_default,
        Ocean = Brush::Solid(Color::rgb(223, 246, 255)),
        Solarized = Brush::Solid(Color::rgb(147, 161, 161))
    )]
    text_block_foreground: Brush,

    #[theme(
        default = Brush::Solid(Color::rgb(39, 103, 216)),
        Ocean = Brush::Solid(Color::rgb(0, 166, 200)),
        Solarized = Brush::Solid(Color::rgb(181, 137, 0))
    )]
    brand: Brush,

    #[theme(default = 10.0, Ocean = 20.0, Solarized = 4.0)]
    rectangle_corner_radius: f32,

    #[theme(
        default = Brush::Solid(Color::rgb(39, 103, 216)),
        Ocean = Brush::Solid(Color::rgb(0, 166, 200)),
        Solarized = Brush::Solid(Color::rgb(181, 137, 0))
    )]
    shape_stroke: Brush,
}

thread_local! {
    static APP_THEME: ThemeController<AppTheme> =
        AppTheme::controller(AppThemeVariant::Default);
}

fn install_theme() {
    APP_THEME.with(|theme| set_application_theme(theme.handle()));
}

fn select_variant(variant: AppThemeVariant) -> u64 {
    APP_THEME.with(|theme| {
        theme.set_variant(variant);
        theme.revision()
    })
}

fn select_preference(preference: ThemePreference) -> u64 {
    APP_THEME.with(|theme| {
        theme.set_preference(preference);
        theme.revision()
    })
}

#[elwindui::viewmodel]
mod theme_demo_view_model {
    use super::{AppThemeVariant, ThemePreference, select_preference, select_variant};

    struct ThemeDemoViewModel {
        #[observable(default = "System".to_string())]
        preference_name: String,
        #[observable(default = "Light (reported after Window sync)".to_string())]
        appearance_name: String,
        #[observable(default = "Default".to_string())]
        variant_name: String,
        #[observable(default = "0".to_string())]
        revision: String,
        #[observable(default = "Editable TextBox".to_string())]
        text_box_value: String,
        #[observable(default = "ThemeDemo42".to_string())]
        password_value: String,
        #[observable(default = "Selection, caret and focus continue to use native theme resources.".to_string())]
        text_area_value: String,
    }

    impl ThemeDemoViewModel {
        fn appearance_system(&self) {
            preference_name = "System".to_string();
            revision = select_preference(ThemePreference::System).to_string();
            appearance_name = "System / backend reported".to_string();
        }

        fn appearance_light(&self) {
            preference_name = "Light".to_string();
            revision = select_preference(ThemePreference::Light).to_string();
            appearance_name = "Light requested".to_string();
        }

        fn appearance_dark(&self) {
            preference_name = "Dark".to_string();
            revision = select_preference(ThemePreference::Dark).to_string();
            appearance_name = "Dark requested".to_string();
        }

        fn variant_default(&self) {
            variant_name = "Default / platform_default".to_string();
            revision = select_variant(AppThemeVariant::Default).to_string();
        }

        fn variant_ocean(&self) {
            variant_name = "Ocean".to_string();
            revision = select_variant(AppThemeVariant::Ocean).to_string();
        }

        fn variant_solarized(&self) {
            variant_name = "Solarized".to_string();
            revision = select_variant(AppThemeVariant::Solarized).to_string();
        }
    }
}

#[elwindui::component(inherits Window)]
struct ThemeDemoWindow {
    #[bindable]
    vm: std::rc::Rc<ThemeDemoViewModel>,

    body: view! {
        title: "elwindui Theme Demo"
        width: 1420.0
        height: 780.0
        menu_bar: MenuBar {
            MenuBarItem {
                text: "Theme"
                Menu {
                    MenuItem { text: "Default" on_select: vm.variant_default }
                    MenuItem { text: "Ocean" on_select: vm.variant_ocean }
                    MenuItem { text: "Solarized" on_select: vm.variant_solarized }
                }
            }
        }
        content: VerticalLayout {
            margin: 16.0
            spacing: theme!(AppTheme::layout_spacing)
            background: theme!(AppTheme::window_background)

            TextBlock {
                text: "ElwindUI Rust Theme Demo"
                font_size: 28.0
                font_weight: FontWeight::BOLD,
                foreground: theme!(AppTheme::brand)
            }

            HorizontalLayout {
                spacing: 8.0
                TextBlock { text: "Appearance:" font_weight: FontWeight::BOLD }
                Button { text: "System" on_click: vm.appearance_system }
                Button { text: "Light" on_click: vm.appearance_light }
                Button { text: "Dark" on_click: vm.appearance_dark }
                TextBlock { text: vm.preference_name }
            }

            HorizontalLayout {
                spacing: 8.0
                TextBlock { text: "Variant:" font_weight: FontWeight::BOLD }
                Button { text: "Default" on_click: vm.variant_default }
                Button { text: "Ocean" on_click: vm.variant_ocean }
                Button { text: "Solarized" on_click: vm.variant_solarized }
                TextBlock { text: vm.variant_name }
            }

            TextBlock { text: vm.appearance_name }
            TextBlock { text: vm.revision }

            HorizontalLayout {
                spacing: 20.0

                VerticalLayout {
                    // Wide enough for "Nested Layout: background is intentionally unset and
                    // transparent." (below) at the Ocean/Solarized variants' own larger
                    // `AppTheme::text_block_font_size` (18.0/17.0 vs the platform default this
                    // unstyled TextBlock otherwise gets) — TextBlock has no word-wrap of its own
                    // (docs/elwindui_builtins_spec.md 付録F.3), so a too-narrow column here just
                    // silently clips the sentence instead of wrapping it. Solarized's own
                    // `text_block_font_family` is an explicit monospace stack (`Consolas, Menlo,
                    // monospace`), which needs noticeably more width per character than Ocean's
                    // proportional `Segoe UI Variable` at a comparable point size — sized for that
                    // worst case, not just Ocean's.
                    width: 740.0
                    spacing: theme!(AppTheme::layout_spacing)
                    background: theme!(AppTheme::layout_background)

                    TextBlock {
                        text: "Explicitly themed Layout"
                        font_family: theme!(AppTheme::text_block_font_family)
                        font_size: theme!(AppTheme::text_block_font_size)
                        foreground: theme!(AppTheme::text_block_foreground)
                    }
                    VerticalLayout {
                        spacing: 6.0
                        TextBlock { text: "Nested Layout: background is intentionally unset and transparent." }
                        TextBlock {
                            text: "Inherited text sample — The quick brown fox 0123456789"
                            font_family: theme!(AppTheme::text_block_font_family)
                            font_size: theme!(AppTheme::text_block_font_size)
                            foreground: theme!(AppTheme::text_block_foreground)
                        }
                    }
                    Rectangle {
                        width: 460.0
                        height: 74.0
                        fill: theme!(AppTheme::brand)
                        stroke: theme!(AppTheme::shape_stroke)
                        stroke_width: 3.0
                        corner_radius: theme!(AppTheme::rectangle_corner_radius)
                    }
                    TextBlock { text: "The rectangle uses the app-owned brand token." }
                }

                VerticalLayout {
                    // See the left column's own width comment — "TabView selection and native
                    // focus remain OS-controlled." below needs the same headroom for Solarized's
                    // monospace `text_block_font_family`.
                    width: 620.0
                    spacing: 10.0

                    TextBlock { text: "Native controls" font_size: 20.0 font_weight: FontWeight::BOLD }
                    Button {
                        text: "Normal / hover / pressed"
                        background: theme!(AppTheme::button_background)
                        foreground: theme!(AppTheme::button_foreground)
                    }
                    Button {
                        text: "Disabled native state"
                        enabled: false
                        background: theme!(AppTheme::button_background)
                        foreground: theme!(AppTheme::button_foreground)
                    }
                    TextBox {
                        text: vm.text_box_value
                        placeholder: "Placeholder uses native resources"
                        background: theme!(AppTheme::text_box_background)
                        foreground: theme!(AppTheme::text_box_foreground)
                    }
                    PasswordBox {
                        password: vm.password_value
                        placeholder: "Password"
                        background: theme!(AppTheme::password_box_background)
                        foreground: theme!(AppTheme::password_box_foreground)
                    }
                    TextArea {
                        height: 115.0
                        text: vm.text_area_value
                        background: theme!(AppTheme::text_area_background)
                        foreground: theme!(AppTheme::text_area_foreground)
                    }
                    TextBlock { text: "TabView selection and native focus remain OS-controlled." }
                    TabView {
                        height: 120.0
                        selected_index: 0
                        on_select: |_| {}
                        on_new_tab: || {}
                        TabViewItem {
                            header: "Selected"
                            closable: false
                            on_close: || {}
                            content: TextBlock { text: "ThemeContext crosses the nested tab host." }
                        }
                        TabViewItem {
                            header: "Second"
                            closable: false
                            on_close: || {}
                            content: TextBlock { text: "Switch variants while this tab is selected." }
                        }
                    }
                }
            }
        }
    },
}

#[elwindui::main]
fn main() {
    install_theme();
    let vm = ThemeDemoViewModel::new();
    let window = ThemeDemoWindow::new(vm);
    window.show();
}
