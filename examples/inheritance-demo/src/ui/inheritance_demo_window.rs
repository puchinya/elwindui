use crate::ui::{LabeledPanel, LoudPanel};

#[elwindui::component(inherits Window)]
struct InheritanceDemoWindow {
    body: view! {
        title: "Inheritance Demo"
        left: 200.0
        top: 200.0
        width: 480.0
        height: 260.0

        content: VerticalLayout {
            margin: 24.0
            spacing: 12.0
            TextBlock { text: "LabeledPanel (base)" }
            LabeledPanel { }
            TextBlock { text: "LoudPanel (derived, inherits crate::ui::LabeledPanel)" }
            LoudPanel { }
        }
    },
}

#[elwindui::component]
impl InheritanceDemoWindow {}
