// PR #312 contract v3 (D2, T10): `construct` of a Component with hand-written storage is the
// constructor, not a method: each misuse is a precise compile error.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![allow(unexpected_cfgs, unused_variables)]

#[elwindui::component(inherits elwindui::core::ui::Layout)]
#[prop(width_hint: f32)]
pub struct WithReceiver {}
#[elwindui::component]
impl WithReceiver {
    fn construct(&self) -> Self {
        unreachable!()
    }
}

#[elwindui::component(inherits elwindui::core::ui::Layout)]
#[prop(width_hint: f32)]
pub struct AsyncConstruct {}
#[elwindui::component]
impl AsyncConstruct {
    async fn construct() -> Self {
        unreachable!()
    }
}

#[elwindui::component(inherits elwindui::core::ui::Layout)]
#[prop(width_hint: f32)]
pub struct WrongReturn {}
#[elwindui::component]
impl WrongReturn {
    fn construct() -> i32 {
        0
    }
}

#[elwindui::component(inherits elwindui::core::ui::Layout)]
#[prop(width_hint: f32)]
pub struct Duplicate {}
#[elwindui::component]
impl Duplicate {
    fn construct() -> Self {
        unreachable!()
    }
    fn construct() -> Self {
        unreachable!()
    }
}

#[elwindui::component(inherits elwindui::core::ui::Layout)]
#[prop(width_hint: f32)]
pub struct Overridden {}
#[elwindui::component]
impl Overridden {
    #[overrides]
    fn construct() -> Self {
        unreachable!()
    }
}

#[elwindui::component(inherits elwindui::core::ui::Layout)]
#[prop(width_hint: f32)]
#[constructor_inputs(first, second)]
pub struct MismatchedInputs {}
#[elwindui::component]
impl MismatchedInputs {
    fn construct(second: i32, first: i32) -> Self {
        unreachable!()
    }
}

fn main() {}
