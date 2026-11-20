// PR #312 contract v3 (D3, T6/T7): a class whose `construct` takes an internal argument that is not
// published with `#[constructor_inputs]` (`InternalOnly` below), and an abstract class (`Layout`,
// and `NativeControl` since Issue #313 made it a Core abstract class), have no public constructor:
// their constructor ABI rejects every use.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![allow(unexpected_cfgs)]

#[elwindui::component(inherits elwindui::ui::VerticalLayout)]
#[prop(value: u32)]
pub struct InternalOnly {
    value: std::cell::Cell<u32>,
}

#[elwindui::component]
impl InternalOnly {
    fn set_value(&self, value: u32) {
        self.value.set(value);
    }
    fn construct(value: u32) -> Self {
        Self {
            base: elwindui::ui::VerticalLayout::construct(),
            value: std::cell::Cell::new(value),
        }
    }
}

fn main() {
    let _ = crate::__elwindui_ctor_InternalOnly!(@new InternalOnly, props = []);
    let _ = elwindui::__elwindui_ctor_NativeControl!(@new elwindui::ui::NativeControl, props = []);
    let _ = elwindui::__elwindui_ctor_Layout!(@new elwindui::ui::Layout, props = []);
}
