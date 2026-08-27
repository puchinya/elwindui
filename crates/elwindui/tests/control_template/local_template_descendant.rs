#[elwindui::component(inherits crate::LocalTemplateBaseProbe)]
pub struct LocalTemplateDescendantProbe {
    #[prop(default = false)]
    pub show_alternate: bool,

    template: template_view! {
        if templated_parent.show_alternate {
            TextBlock { text: "derived alternate" }
        } else {
            TextBlock { text: "derived initial" }
        }
    },
}

#[elwindui::component]
impl LocalTemplateDescendantProbe {}
