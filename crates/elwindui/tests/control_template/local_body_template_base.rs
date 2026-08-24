#[elwindui::component(inherits ContentControl)]
pub struct LocalBodyTemplateBaseProbe {
    body: view! {
        TextBlock { text: "base header" }
    },
}

#[elwindui::component]
impl LocalBodyTemplateBaseProbe {}
