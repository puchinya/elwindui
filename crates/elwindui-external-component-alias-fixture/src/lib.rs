//! A second external component crate used only through a Cargo dependency alias.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use elwindui::component;

#[component(inherits Control)]
pub struct AliasedExternalProbe {
    #[param]
    label: String,
    template: template_view!(|templated_parent: Self| { TextBlock { text: label } }),
}

#[component]
impl AliasedExternalProbe {}
