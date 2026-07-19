// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

pub mod accessibility;
pub mod base;
pub mod error;
pub mod focus;
pub mod graphics;
pub mod input;
pub mod layout;
pub mod reactive;
pub mod task;
pub mod ui;
pub mod visual_tree;
