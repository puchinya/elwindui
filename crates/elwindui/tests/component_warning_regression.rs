//! Compile-time regressions for warning-free real `#[component]` expansion.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui::component(inherits Control)]
struct MinimalComponentWarningProbe {
    template: template_view!(|this: Self| { Grid {} }),
}

#[elwindui::component]
impl MinimalComponentWarningProbe {}

#[elwindui::component(inherits Control)]
struct TemplateComponentWarningProbe {
    #[prop]
    label: String,
    template: template_view!(|this: Self| { Grid {} }),
}

#[elwindui::component]
impl TemplateComponentWarningProbe {}

#[test]
fn real_component_templates_compile_and_construct() {
    let _minimal = MinimalComponentWarningProbe::new();
    let _template = TemplateComponentWarningProbe::new();
}
