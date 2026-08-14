//! Interactive visual verification harness for elwindui's seven inherited text-style properties.
//! Switch among the three profiles to update the same `TextBlock` and native controls in place:
//! `font_family`, `font_size`, `font_weight`, `font_style`, `font_stretch`,
//! `character_spacing`, and `foreground`.
//!
//! The sample intentionally uses viewmodel-backed `FontFamily` and `Brush` values, rather than
//! only literals. That makes it a standing end-to-end check for codegen's owned-value dispatch to
//! `TextStyleOwner`, as well as each backend's native style synchronization.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::graphics::{Brush, Color, FontFamily, FontStretch, FontStyle, FontWeight};

#[elwindui::viewmodel]
mod font_demo_view_model {
    use super::{Brush, Color, FontFamily, FontStretch, FontStyle, FontWeight};

    struct FontDemoViewModel {
        #[observable(default = "System".to_string())]
        profile_name: String,
        #[observable(default = "WinUI/AppKit defaults".to_string())]
        profile_summary: String,

        #[observable(default = FontFamily::system())]
        font_family: FontFamily,
        #[observable(default = 16.0f32)]
        font_size: f32,
        #[observable(default = FontWeight::NORMAL)]
        font_weight: FontWeight,
        #[observable(default = FontStyle::Normal)]
        font_style: FontStyle,
        #[observable(default = FontStretch::Normal)]
        font_stretch: FontStretch,
        #[observable(default = 0i32)]
        character_spacing: i32,
        #[observable(default = Brush::Solid(Color::rgb(30, 30, 30)))]
        foreground: Brush,

        #[observable(default = "Edit this native TextBox while switching profiles".to_string())]
        text_box_value: String,
        #[observable(default = "FontDemo42".to_string())]
        password_value: String,
        #[observable(default = "TextArea is a native multi-line control.\nSwitch profiles to verify that its text is updated too.".to_string())]
        text_area_value: String,
    }

    impl FontDemoViewModel {
        fn use_system(&self) {
            profile_name = "System".to_string();
            profile_summary = "system-ui · 16pt · Normal · black".to_string();
            font_family = FontFamily::system();
            font_size = 16.0;
            font_weight = FontWeight::NORMAL;
            font_style = FontStyle::Normal;
            font_stretch = FontStretch::Normal;
            character_spacing = 0;
            foreground = Brush::Solid(Color::rgb(30, 30, 30));
        }

        fn use_display(&self) {
            profile_name = "Display".to_string();
            profile_summary =
                "Segoe UI Variable, Segoe UI · 28pt · 650 · Italic · SemiExpanded · +80/1000em"
                    .to_string();
            font_family = FontFamily::new("Segoe UI Variable, Segoe UI");
            font_size = 28.0;
            font_weight = FontWeight(650);
            font_style = FontStyle::Italic;
            font_stretch = FontStretch::SemiExpanded;
            character_spacing = 80;
            foreground = Brush::Solid(Color::rgb(0, 102, 204));
        }

        fn use_mono(&self) {
            profile_name = "Mono".to_string();
            profile_summary =
                "Consolas, Segoe UI · 18pt · Medium · Oblique · Condensed · +35/1000em".to_string();
            font_family = FontFamily::new("Consolas, Segoe UI");
            font_size = 18.0;
            font_weight = FontWeight::MEDIUM;
            font_style = FontStyle::Oblique;
            font_stretch = FontStretch::Condensed;
            character_spacing = 35;
            foreground = Brush::Solid(Color::rgb(99, 51, 153));
        }
    }
}

#[elwindui::component(inherits Window)]
struct FontDemoWindow {
    #[bindable]
    vm: std::rc::Rc<FontDemoViewModel>,

    body: view! {
        title: "elwindui Font Demo"
        width: 980.0
        height: 690.0
        content: VerticalLayout {
            margin: 20.0
            spacing: 12.0

            TextBlock {
                text: "elwindui Font Demo"
                font_size: 28.0
                font_weight: FontWeight::BOLD
            }
            TextBlock { text: "Switch profiles to reapply all seven text-style properties to the same visual nodes." }

            HorizontalLayout {
                spacing: 8.0
                Button { text: "System" on_click: vm.use_system }
                Button { text: "Display" on_click: vm.use_display }
                Button { text: "Mono" on_click: vm.use_mono }
                TextBlock { text: vm.profile_name font_weight: FontWeight::BOLD }
            }
            TextBlock { text: vm.profile_summary }

            HorizontalLayout {
                spacing: 24.0

                VerticalLayout {
                    width: 445.0
                    spacing: 10.0

                    TextBlock { text: "TextBlock" font_size: 20.0 font_weight: FontWeight::BOLD }
                    TextBlock { text: "System baseline — The quick brown fox jumps over the lazy dog." }
                    TextBlock {
                        text: "Styled sample — The quick brown fox jumps over the lazy dog. 0123456789"
                        font_family: vm.font_family
                        font_size: vm.font_size
                        font_weight: vm.font_weight
                        font_style: vm.font_style
                        font_stretch: vm.font_stretch
                        character_spacing: vm.character_spacing
                        foreground: vm.foreground
                    }
                    TextBlock { text: "The selected font family is a comma-separated fallback list; System resets to the backend default." }
                }

                VerticalLayout {
                    width: 445.0
                    spacing: 10.0

                    TextBlock { text: "Native controls" font_size: 20.0 font_weight: FontWeight::BOLD }
                    Button {
                        text: "Native Button — same dynamic style"
                        font_family: vm.font_family
                        font_size: vm.font_size
                        font_weight: vm.font_weight
                        font_style: vm.font_style
                        font_stretch: vm.font_stretch
                        character_spacing: vm.character_spacing
                        foreground: vm.foreground
                    }
                    TextBox {
                        text <=> vm.text_box_value
                        font_family: vm.font_family
                        font_size: vm.font_size
                        font_weight: vm.font_weight
                        font_style: vm.font_style
                        font_stretch: vm.font_stretch
                        character_spacing: vm.character_spacing
                        foreground: vm.foreground
                    }
                    PasswordBox {
                        password <=> vm.password_value
                        font_family: vm.font_family
                        font_size: vm.font_size
                        font_weight: vm.font_weight
                        font_style: vm.font_style
                        font_stretch: vm.font_stretch
                        character_spacing: vm.character_spacing
                        foreground: vm.foreground
                    }
                    TextArea {
                        height: 155.0
                        text <=> vm.text_area_value
                        font_family: vm.font_family
                        font_size: vm.font_size
                        font_weight: vm.font_weight
                        font_style: vm.font_style
                        font_stretch: vm.font_stretch
                        character_spacing: vm.character_spacing
                        foreground: vm.foreground
                    }
                }
            }
        }
    },
}

#[elwindui::component]
impl FontDemoWindow {}

#[elwindui::main]
fn main() {
    let vm = FontDemoViewModel::new();
    let window = FontDemoWindow::new(vm);
    window.show();
}
