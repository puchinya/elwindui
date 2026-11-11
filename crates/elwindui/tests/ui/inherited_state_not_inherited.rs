// Issue #166: an ancestor's `#[state]` is private to its declaring Component and is not inherited.
// A derived Component's own view that names it — from a DSL value position, from raw Rust, and as
// an assignment target — must not resolve it as an inherited field; rustc reports the bare name as
// an unknown value.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
// trybuild compiles fixtures outside the workspace lint table, which is where the generated
// cfg(rust_analyzer) gate is registered for check-cfg; keep the snapshot to the diagnostic under test.
#![allow(unexpected_cfgs)]

#[elwindui::component(inherits VerticalLayout)]
struct StateOwner {
    #[prop(default = String::from("visible"))]
    label: String,
    #[state(default = 7)]
    secret: i32,

    body: view! {
        TextBlock { text: format!("{label}:{secret}") }
    },
}

#[elwindui::component]
impl StateOwner {}

#[elwindui::component(inherits crate::StateOwner)]
struct StateReader {
    body: view! {
        TextBlock {
            text: format!("{secret}"),
            on_tapped: |_event| {
                secret = 1;
            },
        }
    },
}

#[elwindui::component]
impl StateReader {}

fn main() {}
