//! Issue #313 review A1: `#[constructor_inputs(..)]` alone makes a class-backed Component, so it
//! must also publish the property-shape macro its constructor ABI and a generated descendant ask
//! for `@required_params` — on a root (no `inherits`), with an explicit empty
//! `#[constructor_inputs()]`, and across crates.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::RefCell;

use elwindui::core::ui::UIElementExt;

fn texts(root: &dyn UIElementExt) -> Vec<String> {
    root.visual_children()
        .iter()
        .filter_map(|child| {
            child
                .as_any()
                .downcast_ref::<elwindui::ui::TextBlock>()
                .map(|text| text.text.borrow().clone())
        })
        .collect()
}

/// A root (no `inherits`) whose only class-shape marker is `#[constructor_inputs]`.
#[elwindui::component]
#[constructor_inputs(title)]
pub struct InputsOnlyRoot {
    title: RefCell<String>,
}

#[elwindui::component]
impl InputsOnlyRoot {
    #[inherent]
    pub fn current_title(&self) -> String {
        self.title.borrow().clone()
    }

    fn construct(title: String) -> Self {
        Self {
            title: RefCell::new(title),
        }
    }
}

/// A root whose only class-shape marker is an explicit empty `#[constructor_inputs()]`.
#[elwindui::component]
#[constructor_inputs()]
pub struct EmptyInputsRoot {
    hits: RefCell<u32>,
}

#[elwindui::component]
impl EmptyInputsRoot {
    #[inherent]
    pub fn current_hits(&self) -> u32 {
        *self.hits.borrow()
    }

    fn construct() -> Self {
        Self {
            hits: RefCell::new(7),
        }
    }
}

/// Receives a shape macro's `@required_params` answer as a list of names.
macro_rules! required_names {
    ({ const $name:ident; } [$($input:ident),*]) => {
        const $name: &[&str] = &[$(stringify!($input)),*];
    };
}

// Before the review fix neither root emitted `__elwindui_props_<Name>!`, so these did not resolve.
crate::__elwindui_props_InputsOnlyRoot!(@required_params required_names, { const ROOT_INPUTS; });
crate::__elwindui_props_EmptyInputsRoot!(@required_params required_names, { const EMPTY_INPUTS; });

#[test]
fn a_constructor_inputs_only_root_publishes_its_property_shape() {
    assert_eq!(ROOT_INPUTS, ["title"]);
    assert!(EMPTY_INPUTS.is_empty());
}

#[test]
fn a_constructor_inputs_only_root_is_constructible() {
    assert_eq!(
        InputsOnlyRoot::new("hello".to_string()).current_title(),
        "hello"
    );
    assert_eq!(EmptyInputsRoot::new().current_hits(), 7);
}

/// A generated base with a required input.
#[elwindui::component(inherits VerticalLayout)]
struct TitledBase {
    #[param]
    title: String,

    body: view! {
        TextBlock { text: format!("base {title}") }
    },
}

#[elwindui::component]
impl TitledBase {}

/// A class-backed middle whose only class-shape marker is `#[constructor_inputs]`, publishing the
/// base's required input.
#[elwindui::component(inherits crate::TitledBase)]
#[constructor_inputs(title)]
pub struct InputsOnlyMiddle {
    title: RefCell<String>,
}

#[elwindui::component]
impl InputsOnlyMiddle {
    #[inherent]
    pub fn current_title(&self) -> String {
        self.title.borrow().clone()
    }

    fn construct(title: String) -> Self {
        Self {
            base: TitledBase::construct(title.clone()),
            title: RefCell::new(title),
        }
    }
}

/// A generated child that auto-forwards the middle's required input.
#[elwindui::component(inherits crate::InputsOnlyMiddle)]
struct InputsOnlyChild {
    body: view! {
        TextBlock { text: "child" }
    },
}

#[elwindui::component]
impl InputsOnlyChild {}

#[test]
fn a_generated_child_forwards_a_constructor_inputs_only_middle() {
    let child = InputsOnlyChild::new("t".to_string());
    assert_eq!(child.base.current_title(), "t");
    assert_eq!(texts(&*child), ["base t", "child"]);
}

/// The same over another crate's constructor-inputs-only class-backed middle.
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalInputsOnlyMiddle)]
struct CrossCrateInputsOnlyChild {
    body: view! {
        TextBlock { text: "cross child" }
    },
}

#[elwindui::component]
impl CrossCrateInputsOnlyChild {}

#[test]
fn a_cross_crate_generated_child_forwards_a_constructor_inputs_only_middle() {
    let child = CrossCrateInputsOnlyChild::new("x".to_string());
    assert_eq!(child.base.current_title(), "x");
    assert_eq!(texts(&*child), ["x:false:1", "cross child"]);
}
