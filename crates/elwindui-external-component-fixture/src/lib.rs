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
    template: template_view!(|templated_parent: Self| { TextBlock { text: title } }),
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
    #[computed(expr = children.len())]
    child_count: usize,
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: format!("{}", child_count),
        }
    }),
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

/// An external component whose public shape deliberately exercises each generated property
/// category used by downstream DSL construction. Its `deferred` field is an unreferenced
/// `Option<String>` Prop with full Option storage/getter/setter semantics.
#[component(inherits Control)]
pub struct ExternalShapeProbe {
    #[prop(default = 0)]
    count: usize,
    #[prop(default = None)]
    optional: Option<String>,
    #[prop]
    deferred: Option<String>,
    #[computed(expr = count)]
    computed_value: usize,
    #[state(default = String::from("private"))]
    private_state: String,
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: format!("{}", computed_value),
        }
    }),
}

#[component]
impl ExternalShapeProbe {}

/// An external component whose constructor contains required values in declaration order while
/// still exposing a required mutable `Prop` after construction.
#[component(inherits Control)]
pub struct RequiredExternalCard {
    #[param]
    title: String,
    #[param]
    count: usize,
    #[param]
    optional: Option<String>,
    #[param(default = 5)]
    fixed: usize,
    #[param(default = None)]
    defaulted_optional: Option<String>,
    #[prop(default = String::from("none"))]
    optional_fallback: String,
    #[prop]
    mutable_label: String,
    template: template_view!(|templated_parent: Self| {
        VerticalLayout {
            TextBlock { text: title }
            match optional {
                None => {
                    TextBlock { text: "optional absent" }
                }
                Some(_) => {
                    TextBlock { text: "optional present" }
                }
            }
            TextBlock { text: mutable_label }
        }
    }),
}

#[component]
impl RequiredExternalCard {}

/// An external component whose bare Vec content is a normal mutable Prop. Bare content is supplied
/// through the constructor ABI's pre-mount initialization protocol, but it is not a constructor
/// parameter (the public contract keeps Prop-backed content out of the fixed constructor).
#[component(inherits Control)]
#[content(children)]
pub struct RequiredExternalTabs {
    #[prop]
    children: Vec<Rc<ExternalProbeItem>>,
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "required external tabs",
        }
    }),
}

#[component]
impl RequiredExternalTabs {}

/// A generated Vec-backed content owner used by the inherited-content capability probe.
#[component(inherits Control)]
#[content(children)]
pub struct BaseExternalTabs {
    #[prop(default = Vec::new())]
    children: Vec<Rc<ExternalProbeItem>>,
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "base external tabs",
        }
    }),
}

#[component]
impl BaseExternalTabs {}

/// Inherits the generated Vec content shape without redeclaring `children`.
#[component(inherits crate::BaseExternalTabs)]
pub struct DerivedExternalTabs {
    template: template_view!(|templated_parent: Self| {
        TextBlock {
            text: "derived external tabs",
        }
    }),
}

#[component]
impl DerivedExternalTabs {}

/// A nested-module component used to prove that the exported props macro stays at the defining
/// crate root while construction and extension-trait paths retain the authored module path.
pub mod nested {
    use super::*;

    #[component(inherits Control)]
    pub struct NestedExternalProbe {
        #[param]
        label: String,
        template: template_view!(|templated_parent: Self| { TextBlock { text: label } }),
    }

    #[component]
    impl NestedExternalProbe {}
}
