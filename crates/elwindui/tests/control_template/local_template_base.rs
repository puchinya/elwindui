#[elwindui::component(inherits ContentControl)]
pub struct LocalTemplateBaseProbe {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "base header",
        }
    }),
}

#[elwindui::component]
impl LocalTemplateBaseProbe {}
