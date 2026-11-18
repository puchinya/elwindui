// PR #312 contract v3 (D1, T11): the hand-written-storage shape and the declarative shape are never
// guessed when mixed, and there is no `manual` flag.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![allow(unexpected_cfgs)]

#[elwindui::component(inherits VerticalLayout)]
#[prop(width_hint: f32)]
pub struct ClassShapeWithParam {
    #[param]
    title: String,
}

#[elwindui::component(inherits VerticalLayout)]
#[prop(width_hint: f32)]
pub struct ClassShapeWithView {
    body: view! {
        TextBlock { text: "x" }
    },
}

#[elwindui::component(trait_only)]
pub struct TraitOnlyOnStruct {}

#[elwindui::component(struct_only = elwindui::core::ui::ButtonExt)]
pub trait StructOnlyOnTrait {}

#[elwindui::component(manual, inherits VerticalLayout)]
pub struct ManualFlag {}

#[elwindui::component(inherits VerticalLayout, abi_props = elwindui::core::__elwindui_props_Button)]
pub struct AbiPropsWithoutStructOnly {
    body: view! {
        TextBlock { text: "x" }
    },
}

fn main() {}
