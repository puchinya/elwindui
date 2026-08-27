//! Issue #83 `ControlTemplate` example: typed Environment override, capturing factory,
//! reactive `templated_parent`, and `ContentPresenter`-based logical content.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::{ContentControlExt as _, TextBlock, TextBlockExt as _, WindowExt};
use std::rc::Rc;

#[elwindui::component(inherits ContentControl)]
struct DemoPanel {
    #[prop]
    label: String,

    template: template_view! {
        VerticalLayout {
            spacing: 8.0
            TextBlock { text: "Default template" }
            TextBlock { text: label }
            ContentPresenter { }
        }
    },
}

#[elwindui::component]
impl DemoPanel {}

#[elwindui::control_template(target = DemoPanel)]
struct CompactDemoPanelTemplate {
    template: template_view! {
        VerticalLayout {
            spacing: 4.0
            TextBlock { text: "Environment override" }
            TextBlock { text: templated_parent.label }
            ContentPresenter { }
        }
    },
}

#[elwindui::component(inherits Window)]
struct ControlTemplateDemoWindow {
    body: view! {
        #[id("panel")]
        let panel = DemoPanel { label: "Reactive templated_parent.label" };

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
    let authored = CompactDemoPanelTemplate::template();
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
