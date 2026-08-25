#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::core::environment::EnvironmentContext;
use elwindui::core::ui::ControlTemplate;
use elwindui::{component, template_view};

#[component(inherits Control)]
struct TemplateProbe {
    #[prop]
    label: String,
    template: template_view! {
        TextBlock { text: templated_parent.label }
    },
}

#[component]
impl TemplateProbe {}

#[test]
fn typed_template_view_can_be_passed_to_environment() {
    let environment = EnvironmentContext::root();
    let captured = String::from("captured");
    let _: ControlTemplate<TemplateProbe> = template_view! {
        TextBlock { text: captured }
    };
    environment.set_control_template::<TemplateProbe>(Some(template_view! {
        TextBlock { text: "override" }
    }));
    let _ = ControlTemplate::<TemplateProbe>::new(|context| {
        let _ = context.control.label();
        elwindui::core::ui::TextBlock::new()
    });
}
