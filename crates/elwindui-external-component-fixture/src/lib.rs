//! A real external generated-component crate used by the facade's downstream DSL regressions.
//!
//! The fixture intentionally depends on `elwindui` under its real crate name and exports its
//! generated component types under a separate crate name. Consumer tests therefore exercise the
//! same qualified path and defining-crate props-macro boundary as an application that depends on
//! both crates.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::rc::Rc;

use elwindui::component;

/// An external component with an owned public property and inherited scalar content.
#[component(inherits ContentControl)]
pub struct ExternalProbeItem {
    #[prop(default = String::new())]
    title: String,
    #[prop(default = true)]
    closable: bool,
    template: template_view! {
        TextBlock { text: title }
    },
}

#[component]
impl ExternalProbeItem {}

/// An external component with a public content collection and a writable/two-way property.
#[component(inherits Control)]
#[content(children)]
pub struct ExternalProbeTabs {
    #[prop(default = Vec::new())]
    children: Vec<Rc<ExternalProbeItem>>,
    #[prop(default = 0)]
    #[two_way]
    selected_index: usize,
    #[state(default = None)]
    selected_index_callback: Option<Rc<dyn Fn(usize)>>,
    template: template_view! {
        TextBlock { text: "external tabs" }
    },
}

#[component]
impl ExternalProbeTabs {}

impl ExternalProbeTabs {
    /// Simulates a user-driven selection and invokes the callback installed by a two-way DSL
    /// binding.
    pub fn select_index(&self, index: usize) {
        self.set_selected_index(index);
        if let Some(callback) = self.selected_index_callback() {
            callback(index);
        }
    }

    /// Public callback surface used by the generated external shape's `@set_on_change` arm.
    pub fn set_on_selected_index_change(&self, callback: Box<dyn Fn(usize)>) {
        self.set_selected_index_callback(Some(Rc::new(callback)));
    }
}

// Generated shape macros address each generated component's extension trait through the defining
// crate's `ui` namespace. This is a normal public export of the component library, not a facade
// impersonation: the consumer still imports the real `elwindui` and this fixture independently.
pub mod ui {
    pub use super::{
        ExternalProbeItem, ExternalProbeItemExt, ExternalProbeTabs, ExternalProbeTabsExt,
    };
    pub use elwindui::ui::*;
}
