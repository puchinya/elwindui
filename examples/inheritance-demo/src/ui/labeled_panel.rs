// The base of this demo's `inherits` chain (Refs #25): a plain `inherits ContentControl`
// component, exactly the shape every other example already uses successfully — `ContentControl`
// is a builtin, so this half of the chain worked before #25. `loud_panel.rs`'s `LoudPanel`
// inherits *this*, a user-defined component, which is the case #25 actually fixes.
#[elwindui::component(inherits ContentControl)]
struct LabeledPanel {
    template: template_view!(|templated_parent: Self| { TextBlock { text: "panel" } }),
}

#[elwindui::component]
impl LabeledPanel {
    #[overridable]
    fn label(&self) -> String {
        "panel".to_string()
    }
}
