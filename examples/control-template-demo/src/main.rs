//! Issue #83 `ControlTemplate` example: typed Environment override, capturing factory,
//! reactive declared parent alias, and `ContentPresenter`-based logical content.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::{ContentControlExt as _, TextBlock, TextBlockExt as _, WindowExt};
use elwindui::template_view;
use std::rc::Rc;

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

fn compact_demo_panel_template() -> elwindui::core::ui::ControlTemplate<DemoPanel> {
    template_view!(|panel: DemoPanel| {
        VerticalLayout {
            spacing: 4.0
            TextBlock { text: "Environment override" }
            TextBlock { text: panel.label }
            ContentPresenter { }
        }
    })
}

#[elwindui::component(inherits Window)]
struct ControlTemplateDemoWindow {
    body: view! {
        #[id("panel")]
        let panel = DemoPanel { label: "Reactive parent alias label" };

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
    let capture = Rc::new("capturing closure".to_string());
    let environment = elwindui::core::environment::application_environment();
    let authored = compact_demo_panel_template();
    environment.set_control_template::<DemoPanel>(Some(elwindui::core::ui::ControlTemplate::new(
        move |context| {
            let _captured_value = capture.clone();
            authored.__build(context)
        },
    )));

    let window = ControlTemplateDemoWindow::new();
    window.show();
    let content = TextBlock::new();
    content.set_text("Logical content, visually hosted by ContentPresenter");
    window.panel().set_content(content);
}
