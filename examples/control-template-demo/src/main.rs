//! Issue #83 `ControlTemplate` example: typed Environment override, capturing factory,
//! reactive declared parent alias, and `ContentPresenter`-based logical content.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::{TextBlock, TextBlockExt as _, WindowExt};
use elwindui::template_view;

#[elwindui::component(inherits ContentControl)]
struct DemoPanel {
    #[prop]
    label: String,

    template: template_view!(|panel: Self| {
        VerticalLayout {
            spacing: 8.0
            TextBlock { text: "Default template" }
            TextBlock { text: label }
            ContentPresenter { }
        }
    }),
}

#[elwindui::component]
impl DemoPanel {}

fn compact_demo_panel_template(prefix: String) -> elwindui::core::ui::ControlTemplate<DemoPanel> {
    template_view!(|panel: DemoPanel| {
        VerticalLayout {
            spacing: 4.0
            TextBlock {
                text: format!("{} Environment override", prefix)
            }
            TextBlock { text: panel.label }
            ContentPresenter { }
        }
    })
}

#[elwindui::component(inherits Window)]
struct ControlTemplateDemoWindow {
    // Window::new() intentionally remains unmounted until show(), so its #[id] accessor is not
    // available while the window is being prepared. Passing this value as a Param lets the panel
    // receive its logical content before the panel's own template mount.
    #[param]
    logical_content: std::rc::Rc<TextBlock>,

    body: view! {
        #[id("panel")]
        let panel = DemoPanel {
            label: "Reactive parent alias label"
            content: logical_content
        };

        title: "elwindui ControlTemplate Demo"
        width: 520.0
        height: 260.0
        panel
    },
}

#[elwindui::component]
impl ControlTemplateDemoWindow {}

#[elwindui::main]
fn main() {
    let environment = elwindui::core::environment::application_environment();
    environment.set_control_template(Some(compact_demo_panel_template("Captured:".to_string())));

    let content = TextBlock::new();
    content.set_text("Logical content, visually hosted by ContentPresenter");
    let window = ControlTemplateDemoWindow::new(content);
    window.show();
}
