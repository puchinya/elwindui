use crate::LocalBodyTemplateBaseProbe;

#[elwindui::component(inherits crate::LocalBodyTemplateBaseProbe)]
pub struct LocalBodyTemplateDescendantProbe {
    body: view! {
        TextBlock { text: "descendant header" }
    },
}

#[elwindui::component]
impl LocalBodyTemplateDescendantProbe {}
