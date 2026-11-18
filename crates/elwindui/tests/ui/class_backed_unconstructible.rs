// PR #312 contract v3 (D3, T6/T7): a class whose `construct` takes an internal argument that is not
// published with `#[constructor_inputs]` (the native handle of `NativeControl`), and an abstract
// class (`Layout`), have no public constructor: their constructor ABI rejects every use.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![allow(unexpected_cfgs)]

fn main() {
    let _ = elwindui::__elwindui_ctor_NativeControl!(@new elwindui::ui::NativeControl, props = []);
    let _ = elwindui::__elwindui_ctor_Layout!(@new elwindui::ui::Layout, props = []);
}
