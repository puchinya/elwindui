#[elwindui::component(inherits ContentControl)]
pub struct LocalTemplateBaseProbe {
    template: template_view! {
        TextBlock { text: "base header" }
    },
}

#[elwindui::component]
impl LocalTemplateBaseProbe {}
