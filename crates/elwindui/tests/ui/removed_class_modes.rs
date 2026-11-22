// Issue #313: `trait_only`, `struct_only`, `abi_props` and `no_ancestor_forward` were removed. Each
// is a compile error that names the mode, on both `#[component]` and `#[class]`, and a `trait`
// item is no longer a class shape.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![allow(unexpected_cfgs)]

#[elwindui::component(trait_only)]
pub struct ComponentTraitOnly {}

#[elwindui::component(struct_only = elwindui::core::ui::ButtonExt)]
pub struct ComponentStructOnly {}

#[elwindui::component(inherits VerticalLayout, abi_props = elwindui::core::__elwindui_props_Button)]
#[prop(width_hint: f32)]
pub struct ComponentAbiProps {}

#[elwindui::component(inherits VerticalLayout, no_ancestor_forward)]
#[prop(width_hint: f32)]
pub struct ComponentNoAncestorForward {}

#[elwindui::component]
pub trait ComponentTraitItem {}

#[elwindui::class(trait_only)]
pub struct ClassTraitOnly {}

#[elwindui::class(struct_only = elwindui::core::ui::ButtonExt)]
pub struct ClassStructOnly {}

#[elwindui::class(abi_props = elwindui::core::__elwindui_props_Button)]
pub struct ClassAbiProps {}

#[elwindui::class(no_ancestor_forward)]
pub struct ClassNoAncestorForward {}

#[elwindui::class]
pub trait ClassTraitItem {}

fn main() {}
