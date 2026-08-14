//! Issue #83 `ControlTemplate` example: typed Environment override, capturing factory,
//! reactive `templated_parent`, and `ContentPresenter`-based logical content.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::ui::{ContentControlExt as _, ControlTemplate, TextBlock, TextBlockExt as _};
use std::rc::Rc;

#[elwindui::environment_key(
    name = demo_panel_template,
    value = Option<ControlTemplate<DemoPanel>>,
    default = None
)]
pub struct DemoPanelTemplate;

#[elwindui::component(inherits ContentControl, template = demo_panel_template)]
struct DemoPanel {
    #[prop]
    label: String,

    body: view! {
        VerticalLayout {
            spacing: 8.0
            TextBlock { text: "Default template" font_size: 18.0 }
            TextBlock { text: label }
            ContentPresenter { }
        }
    },
}

#[elwindui::component]
impl DemoPanel {}

#[elwindui::control_template(target = DemoPanel)]
struct CompactDemoPanelTemplate {
    body: view! {
        VerticalLayout {
            spacing: 4.0
            TextBlock { text: "Environment override" font_size: 18.0 }
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
    let authored = CompactDemoPanelTemplate::template();
    let capture = Rc::new("capturing closure".to_string());
    elwindui::core::environment::application_environment().set::<DemoPanelTemplate>(Some(
        ControlTemplate::new(move |context| {
            let _captured_value = capture.clone();
            authored.__build(context)
        }),
    ));

    let window = ControlTemplateDemoWindow::new();
    window.show();
    let content = TextBlock::new();
    content.set_text("Logical content, visually hosted by ContentPresenter");
    window.panel().set_content(content);
}
