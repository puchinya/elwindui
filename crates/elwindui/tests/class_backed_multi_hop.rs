//! PR #312 Implementation Contract v3 (T8): a same-crate class-backed Component is used from a
//! `view!` like any other element, and sits between generated levels of an inheritance chain.

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

/// Generated base.
#[elwindui::component(inherits VerticalLayout)]
struct GenBase {
    #[prop(default = false)]
    flag: bool,

    body: view! {
        TextBlock { text: format!("base {flag}") }
    },
}

#[elwindui::component]
impl GenBase {}

/// Hand-written storage over the generated base, with a property of its own.
#[elwindui::component(inherits crate::GenBase)]
#[prop(note: String)]
pub struct RawMiddle {
    note: RefCell<String>,
}

#[elwindui::component]
impl RawMiddle {
    fn set_note(&self, note: &str) {
        *self.note.borrow_mut() = note.to_string();
    }

    #[inherent]
    pub fn current_note(&self) -> String {
        self.note.borrow().clone()
    }

    fn construct() -> Self {
        Self {
            base: GenBase::construct(),
            note: RefCell::new(String::new()),
        }
    }
}

/// Generated leaf over the hand-written middle: its root sets the middle's own property, the
/// generated base's Prop and a builtin property.
#[elwindui::component(inherits crate::RawMiddle)]
struct GenLeaf {
    body: view! {
        note: "leaf note"
        flag: true
        margin: 3.0
        TextBlock { text: "leaf" }
    },
}

#[elwindui::component]
impl GenLeaf {}

#[test]
fn generated_raw_generated_chain() {
    let leaf = GenLeaf::new();
    assert_eq!(leaf.base.current_note(), "leaf note");
    assert_eq!(leaf.margin(), 3.0);
    assert_eq!(texts(&*leaf), ["base true", "leaf"]);
}

/// A same-crate class-backed Component constructed from a view.
#[elwindui::component(inherits VerticalLayout)]
struct UsesRaw {
    body: view! {
        crate::RawMiddle { note: "from view", margin: 2.0 }
    },
}

#[elwindui::component]
impl UsesRaw {}

#[test]
fn a_same_crate_class_backed_component_is_built_from_a_view() {
    let host = UsesRaw::new();
    let children = host.visual_children();
    let middle = children[0]
        .as_any()
        .downcast_ref::<RawMiddle>()
        .expect("the view builds the class-backed Component");
    assert_eq!(middle.current_note(), "from view");
    assert_eq!(middle.margin(), 2.0);
    // Outermost here, the class-backed level builds its generated base's view once attached.
    assert_eq!(texts(middle), ["base false"]);
}

/// The same chain across crates: a generated leaf here over the other crate's class-backed middle,
/// whose own generated base has a required input (auto-forwarded through the middle's published
/// `#[constructor_inputs(title)]`).
#[elwindui::component(inherits elwindui_external_component_fixture::ExternalRawMiddle)]
struct CrossCrateLeafOverRaw {
    body: view! {
        badge: "gold"
        flag: true
        margin: 4.0
        TextBlock { text: "cross leaf" }
    },
}

#[elwindui::component]
impl CrossCrateLeafOverRaw {}

#[test]
fn cross_crate_generated_raw_generated_chain() {
    let leaf = CrossCrateLeafOverRaw::new("t".to_string());
    assert_eq!(leaf.base.current_badge(), "gold");
    assert_eq!(leaf.margin(), 4.0);
    assert_eq!(texts(&*leaf), ["t:true:1", "cross leaf"]);
}
