#[elwindui::component(inherits crate::LocalTemplateBaseProbe)]
pub struct LocalTemplateDescendantProbe {
    template: template_view! {
        TextBlock { text: "descendant header" }
    },
}

#[elwindui::component]
impl LocalTemplateDescendantProbe {}
